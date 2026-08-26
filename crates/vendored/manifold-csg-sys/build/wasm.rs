use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

// ── wasm32-unknown-unknown build path ────────────────────────────────────
//
// Bare wasm — no WASI, no Emscripten. Targets the wasm-bindgen Rust web
// ecosystem (Bevy, Leptos, Yew, etc., once wasm-bindgen lands emscripten
// support; usable today by any wasm-bindgen-style consumer that doesn't
// need wasm-bindgen itself). Depends on `wasm-cxx-shim` for the C/C++
// runtime layer that wasm32-unknown-unknown is missing (no libc, no
// libcxx, no libcxxabi).
//
// The build path here is fundamentally different from the host/emscripten
// path: a different toolchain (clang via wasm-cxx-shim's toolchain file),
// different cmake flags (-fno-exceptions, -fno-rtti, -nostdlib, -nostdinc++,
// MANIFOLD_PAR=OFF, etc.), additional dependencies (Clipper2 cloned
// separately so we can patch it), additional carry-patches to manifold
// (iostream gating), and a libcxx-extras.cpp consumer-side file that
// provides the libc++ source-file symbols (shared_ptr internals, etc.)
// the shim deliberately doesn't ship.
//
// See docs/plans/wasm-unknown-unknown.md for the design and
// `crates/manifold-csg-sys/wasm32-uu/` for the vendored helper files.
//
// Reference implementation: wasm-cxx-shim's
// `test/manifold-link/CMakeLists.txt` (the same recipe, expressed as a
// build.rs instead of a cmake file).

const WASM_CXX_SHIM_GIT: &str = "https://github.com/zmerlynn/wasm-cxx-shim.git";
// v0.5.0 catches up to manifold v3.5.0: adds `assert.h` to libc and
// `<memory>` shared_ptr atomic-free-function stubs to libcxx, both
// surfaced by v3.5.0's new ExecutionContext-attached-via-shared_ptr
// model. The shim's tested-pin default is `v3.5.0`.
//
// Our host pin (MANIFOLD_VERSION) is now `v3.5.2`, two patch releases
// PAST the shim's tested pin. Build.rs passes `-DMANIFOLD_GIT_TAG=<our
// pin>` (below), so the wasm-uu lane builds v3.5.2 through the shim.
//
// This is lower-risk than the tested-pin gap suggests: the v0.5.0 helper
// ships no carry-patches for default-pin builds (it passes GIT_TAG
// straight to FetchContent, and patches only arrive via the caller's
// EXTRA_MANIFOLD_PATCHES, which we do not pass). So there is no patch
// that can fail to apply against a newer manifold. What the shim does
// supply is the libc++/libc surface manifold compiles against, so the
// residual risk is a compile break if v3.5.2 reaches for something the
// shim lacks - which the wasm-uu CI lane catches directly.
//
// If it does break, the fix is either (a) a shim release pinned past our
// version, or (b) pin this lane back here and cfg-gate any newer-only FFI
// on `not(all(target_arch = "wasm32", target_os = "unknown"))`. See
// CLAUDE.md "Versioning" / "Pin / shim follow-ups".
const WASM_CXX_SHIM_TAG: &str = "v0.5.0";

/// Diagnostic context populated up-front in `build_wasm_unknown_unknown()`,
/// passed to `bail_with_diagnostics()` so cmake/clang failures emit the
/// resolved toolchain paths the user actually needs to debug.
struct BuildContext {
    clangpp: PathBuf,
    libcxx_headers: PathBuf,
    /// Bin dirs the LLVM probe actually checked, in the order they were tried.
    /// Surfaces the discovery ladder without making the user re-derive it.
    llvm_candidates: Vec<PathBuf>,
}

/// Print a diagnostic dump and panic. Called from each cmake/clang
/// failure site in the wasm32-uu path.
///
/// We don't capture cmake's stdout/stderr (they stream live), so this
/// dump comes *after* whatever cmake/make printed. It adds context the
/// user can't see otherwise: what build.rs resolved as the LLVM
/// toolchain, the env vars that influence it, and tails of cmake's own
/// configure-time logs.
fn bail_with_diagnostics(ctx: &BuildContext, stage: &str, build_dir: &Path) -> ! {
    eprintln!("\n=== manifold-csg-sys: wasm32-unknown-unknown build failed ({stage}) ===");
    eprintln!("clang++:        {}", ctx.clangpp.display());
    eprintln!("libc++ headers: {}", ctx.libcxx_headers.display());
    eprintln!("build dir:      {}", build_dir.display());
    eprintln!("LLVM candidates probed (in order):");
    for c in &ctx.llvm_candidates {
        eprintln!("  {}", c.display());
    }
    eprintln!("environment:");
    for k in [
        "WASM_CXX_SHIM_LLVM_BIN_DIR",
        "WASM_CXX_SHIM_LIBCXX_HEADERS",
        "CC",
        "CXX",
        "CFLAGS",
        "CXXFLAGS",
        "LDFLAGS",
        "RUSTFLAGS",
    ] {
        let v = env::var(k).unwrap_or_else(|_| "<unset>".into());
        eprintln!("  {k}={v}");
    }
    // cmake's own diagnostic logs — most useful for configure-time
    // failures (try-compile, missing tools). For build-time compile
    // errors the relevant output already streamed live above; these
    // will simply be absent or stale.
    for log in ["CMakeFiles/CMakeError.log", "CMakeFiles/CMakeOutput.log"] {
        let p = build_dir.join(log);
        if let Ok(contents) = std::fs::read_to_string(&p) {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(200);
            eprintln!(
                "\n--- {} (last 200 lines) ---\n{}",
                p.display(),
                lines[start..].join("\n")
            );
        }
    }
    eprintln!(
        "\nFor a full bug report, run:\n  \
         bash crates/manifold-csg-sys/wasm32-uu/diagnose.sh > bugreport.txt 2>&1\n\
         and attach bugreport.txt to the issue.\n"
    );
    panic!("wasm32-unknown-unknown build failed at stage: {stage}");
}

/// Locate an LLVM install with wasm32 support, returning
/// (clang++ path, libc++ headers dir, candidates probed in order).
///
/// The candidates list is returned alongside the resolved paths so that
/// `bail_with_diagnostics()` can show what the probe ladder considered
/// even on a successful resolve (which still might be wrong — e.g. an
/// LLVM that's too old, or a libc++ that doesn't actually match).
///
/// Mirrors wasm-cxx-shim's toolchain-wasm32.cmake discovery ladder so we
/// pick the same LLVM the cmake build is using. Order:
///   1. `WASM_CXX_SHIM_LLVM_BIN_DIR` env var (explicit override)
///   2. Common per-platform locations:
///        - macOS: /opt/homebrew/opt/llvm[@N]/bin
///        - Linux Debian-family: /usr/lib/llvm-N/bin
///   3. PATH lookup (Apple's stock clang lacks libc++ headers, so this is
///      a last resort)
///
/// `WASM_CXX_SHIM_LIBCXX_HEADERS` env var separately overrides just the
/// header path, for cases where the user wants a non-default libc++.
fn find_llvm() -> (PathBuf, PathBuf, Vec<PathBuf>) {
    let candidates = candidate_llvm_bin_dirs();

    if let Ok(headers) = env::var("WASM_CXX_SHIM_LIBCXX_HEADERS") {
        let headers = PathBuf::from(headers);
        let clangpp = which("clang++")
            .or_else(|| which("clang"))
            .expect("clang++/clang not found on PATH");
        warn_if_system_libcxx(&headers);
        return (clangpp, headers, candidates);
    }

    for bin_dir in &candidates {
        let clangpp = bin_dir.join("clang++");
        if !clangpp.exists() {
            continue;
        }
        let llvm_root = bin_dir.parent().unwrap();
        // LLVM ships libc++ headers either at <root>/include/c++/v1 or
        // <root>/lib/c++/v1, depending on layout. Try both.
        for rel in ["include/c++/v1", "lib/c++/v1"] {
            let headers = llvm_root.join(rel);
            if headers.join("vector").exists() {
                warn_if_system_libcxx(&headers);
                return (clangpp, headers, candidates);
            }
        }
    }

    panic!(
        "Could not find an LLVM install with libc++ headers and wasm32 support.\n\
         Tried: {candidates:?}\n\
         Install via:\n\
         \x20  - macOS:  brew install llvm  (then add to PATH per brew's instructions)\n\
         \x20  - Debian: apt install clang-20 lld-20 libc++-20-dev libc++abi-20-dev\n\
         Or set WASM_CXX_SHIM_LLVM_BIN_DIR to the directory containing clang++\n\
         (and ensure ../include/c++/v1 contains libc++ headers).\n\
         See docs/plans/wasm-unknown-unknown.md."
    );
}

/// Warn if `headers` (after symlink resolution) lives under a system
/// include path. On Debian-family distros, `/usr/lib/llvm-N/include/c++/v1`
/// is sometimes a symlink to `/usr/include/c++/v1` — meaning even though
/// we pass a versioned LLVM path to `-isystem`, clang ends up reading the
/// system libc++. That's the failure mode we hit when the system libc++
/// is newer (or older) than what our vendored `__config_site` covers.
///
/// We warn rather than reject because the symlink layout sometimes works
/// fine; only flag it so the user can correlate the warning with a build
/// failure and override via `WASM_CXX_SHIM_LIBCXX_HEADERS`.
fn warn_if_system_libcxx(headers: &Path) {
    let canonical = std::fs::canonicalize(headers).unwrap_or_else(|_| headers.to_path_buf());
    if canonical.starts_with("/usr/include/") || canonical.starts_with("/usr/local/include/") {
        println!(
            "cargo:warning=manifold-csg-sys: libc++ headers at {} resolve to a system path \
             ({}). System libc++ may be incompatible with our vendored __config_site; \
             if the build fails with _LIBCPP_* config errors, set \
             WASM_CXX_SHIM_LIBCXX_HEADERS to a non-symlinked LLVM-versioned path.",
            headers.display(),
            canonical.display()
        );
    }
}

fn candidate_llvm_bin_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(d) = env::var("WASM_CXX_SHIM_LLVM_BIN_DIR") {
        out.push(PathBuf::from(d));
    }
    // Newest first; first match wins. Mirrors the toolchain file's range.
    for v in [22, 21, 20, 19, 18] {
        // macOS Homebrew (Apple Silicon and Intel)
        out.push(PathBuf::from(format!("/opt/homebrew/opt/llvm@{v}/bin")));
        out.push(PathBuf::from(format!("/usr/local/opt/llvm@{v}/bin")));
        // Debian-family
        out.push(PathBuf::from(format!("/usr/lib/llvm-{v}/bin")));
    }
    // Unversioned brew paths
    out.push(PathBuf::from("/opt/homebrew/opt/llvm/bin"));
    out.push(PathBuf::from("/usr/local/opt/llvm/bin"));
    // Last-resort PATH lookup. (Nested-if rather than let-chain so we
    // stay compatible with our 1.85 MSRV; let-chains landed in 1.88.)
    #[allow(clippy::collapsible_if)]
    if let Some(p) = which("clang++") {
        if let Some(parent) = p.parent() {
            out.push(parent.to_path_buf());
        }
    }
    out
}

fn which(name: &str) -> Option<PathBuf> {
    let output = Command::new(if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    })
    .arg(name)
    .output()
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(path.lines().next()?.trim()))
}

pub fn build_wasm_unknown_unknown() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=patches");
    println!("cargo:rerun-if-changed=wasm32-uu");

    // wasm32-unknown-unknown support is provisional: the build path carries
    // patches against upstream manifold and Clipper2, ships without an
    // exception runtime (implicit STL throws abort), disables OBJ I/O, and
    // depends on a precise LLVM toolchain. Require an explicit feature flag
    // so this is acknowledged at the consumer's Cargo.toml.
    if env::var("CARGO_FEATURE_UNSTABLE_WASM_UU").is_err() {
        panic!(
            "Building manifold-csg-sys for wasm32-unknown-unknown requires \
             the `unstable-wasm-uu` cargo feature. Add it to your dependency:\n\
             \n    \
             manifold-csg = {{ version = \"...\", features = [\"unstable-wasm-uu\"] }}\n\
             \n\
             See the README's \"Browser without Emscripten\" section for the \
             constraints (no exceptions, no OBJ I/O, requires LLVM 20+)."
        );
    }
    println!(
        "cargo:warning=manifold-csg-sys: wasm32-unknown-unknown support is \
         provisional. Patched manifold and Clipper2 (via wasm-cxx-shim helper); \
         no exception runtime; OBJ I/O disabled. See README for details."
    );
    // Env vars that influence toolchain selection. Without these,
    // Cargo treats them as untracked and may skip rerunning build.rs
    // when the user changes their LLVM install pointer.
    println!("cargo:rerun-if-env-changed=WASM_CXX_SHIM_LLVM_BIN_DIR");
    println!("cargo:rerun-if-env-changed=WASM_CXX_SHIM_LIBCXX_HEADERS");
    // Extra C/C++ flags spliced into every compile on this target
    // (manifold, Clipper2, libcxx-extras). Appended to CMake's default
    // Release flags (-O3 -DNDEBUG), so bare `-O3` stays in place and
    // users only add deltas like `-msimd128 -flto`. Empty by default.
    println!("cargo:rerun-if-env-changed=MANIFOLD_WASM_CXX_FLAGS");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wasm_dir = manifest_dir.join("wasm32-uu");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // ---- Toolchain sanity check ----------------------------------------
    //
    // Resolve LLVM up front (instead of probing for stock `clang`, which on
    // macOS is Apple's clang and lacks libc++ headers in the layout we
    // need). find_llvm() panics with a focused install hint if the lookup
    // fails, so the user sees the right error before any cmake work
    // starts.
    if Command::new("cmake").arg("--version").output().is_err() {
        panic!(
            "Building for wasm32-unknown-unknown requires cmake on PATH. \
             Install via `brew install cmake` or your distro's package manager."
        );
    }
    let (clangpp, libcxx_headers, llvm_candidates) = find_llvm();

    // User-supplied extra flags. Split on whitespace so callers can
    // pass e.g. `MANIFOLD_WASM_CXX_FLAGS="-msimd128 -flto"`.
    let extra_cxx: Vec<String> = env::var("MANIFOLD_WASM_CXX_FLAGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if !extra_cxx.is_empty() {
        println!(
            "cargo:warning=manifold-csg-sys: MANIFOLD_WASM_CXX_FLAGS={}",
            extra_cxx.join(" ")
        );
    }

    // Diagnostic context for cmake/clang failures. Populated before any
    // cmake invocation so bail_with_diagnostics can report what build.rs
    // actually resolved (vs. what cmake might have re-discovered).
    let ctx = BuildContext {
        clangpp: clangpp.clone(),
        libcxx_headers: libcxx_headers.clone(),
        llvm_candidates,
    };

    // Warn loudly if the user has the `parallel` feature on for this
    // target (it's a no-op — wasm32-unknown-unknown has no threads — but
    // a silent downgrade is worse than a noisy one). Mirrors the
    // emscripten path's behavior.
    if env::var("CARGO_FEATURE_PARALLEL").is_ok() {
        println!(
            "cargo:warning=manifold-csg-sys: 'parallel' feature is not supported on \
             wasm32-unknown-unknown; building without TBB. Disable default-features \
             or the 'parallel' feature to silence this warning."
        );
    }

    // ---- Stage 1: clone + build wasm-cxx-shim ---------------------------

    let shim_src = out_dir.join("wasm-cxx-shim-src");
    let shim_build = out_dir.join("wasm-cxx-shim-build");
    let shim_stamp = out_dir.join(".shim-version-stamp");
    let shim_old = std::fs::read_to_string(&shim_stamp).unwrap_or_default();
    if shim_old.trim() != WASM_CXX_SHIM_TAG && shim_src.exists() {
        let _ = std::fs::remove_dir_all(&shim_src);
        let _ = std::fs::remove_dir_all(&shim_build);
    }
    if !shim_src.join("CMakeLists.txt").exists() {
        // Partial clone from a previous failed run? Wipe and retry.
        if shim_src.exists() {
            let _ = std::fs::remove_dir_all(&shim_src);
        }
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--branch",
                WASM_CXX_SHIM_TAG,
                WASM_CXX_SHIM_GIT,
                shim_src.to_str().unwrap(),
            ])
            .status()
            .expect("failed to run git clone for wasm-cxx-shim");
        assert!(status.success(), "git clone wasm-cxx-shim failed");
        let _ = std::fs::write(&shim_stamp, WASM_CXX_SHIM_TAG);
    }

    let shim_toolchain = shim_src.join("cmake/toolchain-wasm32.cmake");
    assert!(
        shim_toolchain.exists(),
        "wasm-cxx-shim missing toolchain at {}",
        shim_toolchain.display()
    );

    if !shim_build.join("libc/libwasm-cxx-shim-libc.a").exists() {
        let mut cfg = Command::new("cmake");
        cfg.args([
            "-S",
            shim_src.to_str().unwrap(),
            "-B",
            shim_build.to_str().unwrap(),
            &format!("-DCMAKE_TOOLCHAIN_FILE={}", shim_toolchain.display()),
            "-DCMAKE_BUILD_TYPE=Release",
        ]);
        if !extra_cxx.is_empty() {
            let joined = extra_cxx.join(" ");
            cfg.arg(format!("-DCMAKE_C_FLAGS_INIT={joined}"));
            cfg.arg(format!("-DCMAKE_CXX_FLAGS_INIT={joined}"));
        }
        let status = cfg
            .args(crate::cmake_launcher_args())
            .status()
            .expect("failed to run cmake configure for wasm-cxx-shim");
        if !status.success() {
            bail_with_diagnostics(&ctx, "wasm-cxx-shim cmake configure", &shim_build);
        }

        let status = Command::new("cmake")
            .args([
                "--build",
                shim_build.to_str().unwrap(),
                "--config",
                "Release",
                "--parallel",
            ])
            .status()
            .expect("failed to run cmake build for wasm-cxx-shim");
        if !status.success() {
            bail_with_diagnostics(&ctx, "wasm-cxx-shim cmake build", &shim_build);
        }
    }

    // ---- Stage 2: build manifold + Clipper2 via the shim's helper -------
    //
    // The shim's `wasm_cxx_shim_add_manifold()` helper owns the high-
    // change-rate parts of the integration cocktail: FetchContent of
    // manifold + Clipper2 (with tested-pin defaults, carry-patches, and
    // manifold/Clipper2 CMake options). Our wrapper at
    // wasm32-uu/CMakeLists.txt sets up the consumer-side `-isystem`
    // chain (libc++ headers + our `__config_site` override + the
    // `<mutex>` stub) and calls the helper. See WASM_CXX_SHIM_TAG above
    // for the pinned version.

    let manifold_build = out_dir.join("manifold-build-wasm32-uu");

    let mut cfg = Command::new("cmake");
    cfg.args([
        "-S",
        wasm_dir.to_str().unwrap(),
        "-B",
        manifold_build.to_str().unwrap(),
        &format!("-DCMAKE_TOOLCHAIN_FILE={}", shim_toolchain.display()),
        "-DCMAKE_BUILD_TYPE=Release",
        &format!("-DWASM_CXX_SHIM_DIR={}", shim_src.display()),
        &format!("-DWASM32_UU_INC_DIR={}", wasm_dir.join("include").display()),
        &format!("-DLIBCXX_HEADERS={}", libcxx_headers.display()),
        // Override the shim's tested-pin default so wasm-uu builds
        // against the same manifold pin as host. Otherwise our FFI
        // declarations target the host's (newer) C API surface and
        // the wasm link fails with unresolved imports.
        &format!("-DMANIFOLD_GIT_TAG={}", crate::MANIFOLD_VERSION),
    ]);
    if !extra_cxx.is_empty() {
        let joined = extra_cxx.join(" ");
        cfg.arg(format!("-DCMAKE_C_FLAGS_INIT={joined}"));
        cfg.arg(format!("-DCMAKE_CXX_FLAGS_INIT={joined}"));
    }
    let status = cfg
        .args(crate::cmake_launcher_args())
        .status()
        .expect("failed to run cmake configure for manifold (wasm32-uu)");
    if !status.success() {
        bail_with_diagnostics(&ctx, "manifold cmake configure", &manifold_build);
    }

    let status = Command::new("cmake")
        .args([
            "--build",
            manifold_build.to_str().unwrap(),
            "--config",
            "Release",
            "--parallel",
        ])
        .status()
        .expect("failed to run cmake build for manifold (wasm32-uu)");
    if !status.success() {
        bail_with_diagnostics(&ctx, "manifold cmake build", &manifold_build);
    }

    // ---- Stage 5: compile + archive libcxx-extras.cpp -------------------
    //
    // libcxx-extras provides the libc++ source-file symbols (shared_ptr
    // internals, std::nothrow, etc.) the shim deliberately doesn't ship.
    // We compile it here and wrap the .o in a static archive so we can
    // emit it via cargo:rustc-link-lib=static and let cargo control
    // link order alongside the other archives.

    let extras_o = out_dir.join("libcxx-extras.o");
    let extras_a = out_dir.join("libcxx_extras.a");
    let extras_cpp = wasm_dir.join("libcxx-extras.cpp");

    let mut extras_cmd = Command::new(&clangpp);
    extras_cmd.args([
        "--target=wasm32-unknown-unknown",
        "-std=c++17",
        "-Os",
        "-fno-exceptions",
        "-fno-rtti",
        "-nostdlib",
        "-nostdinc++",
        "-nostdlibinc",
    ]);
    // Append MANIFOLD_WASM_CXX_FLAGS after -Os so a caller-supplied
    // -O3/-flto/-msimd128 overrides the default (clang honors the last
    // -O and -f flag on the command line).
    for f in &extra_cxx {
        extras_cmd.arg(f);
    }
    let status = extras_cmd
        .arg(format!("-isystem{}", wasm_dir.join("include").display()))
        .arg(format!("-isystem{}", libcxx_headers.display()))
        .arg(format!(
            "-isystem{}",
            shim_src.join("libm/include").display()
        ))
        .arg(format!(
            "-isystem{}",
            shim_src.join("libc/include").display()
        ))
        .arg("-c")
        .arg(&extras_cpp)
        .arg("-o")
        .arg(&extras_o)
        .status()
        .expect("failed to compile libcxx-extras.cpp");
    if !status.success() {
        bail_with_diagnostics(&ctx, "libcxx-extras compile", &out_dir);
    }

    let _ = std::fs::remove_file(&extras_a);
    // Use the llvm-ar that ships with our clang (next to it in the LLVM
    // bin/ dir). System `ar` won't produce wasm-friendly archives.
    let llvm_ar = clangpp
        .parent()
        .map(|d| d.join("llvm-ar"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("llvm-ar"));
    let status = Command::new(&llvm_ar)
        .args(["rcs"])
        .arg(&extras_a)
        .arg(&extras_o)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {} for libcxx-extras: {e}", llvm_ar.display()));
    if !status.success() {
        bail_with_diagnostics(&ctx, "libcxx-extras llvm-ar", &out_dir);
    }

    // ---- Stage 6: emit cargo metadata -----------------------------------
    //
    // Link order matters for static archives. Wasm-ld processes archives
    // left-to-right and pulls .o files as needed by previously-seen
    // undefined symbols. The order below mirrors wasm-cxx-shim's
    // test/manifold-link/CMakeLists.txt:
    //
    //   user obj/rlib (Rust crate) → libcxx_extras → manifoldc → manifold
    //     → Clipper2 → wasm-cxx-shim-libcxx → wasm-cxx-shim-libc
    //     → wasm-cxx-shim-libm

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=cxx_extras");

    // FetchContent puts artifacts under <build>/_deps/<name>-build/, with
    // manifold's libraries further nested under bindings/c and src. Walk
    // the build dir rather than hardcoding paths, so we stay robust
    // against helper/cmake layout changes.
    for libname in ["manifoldc", "manifold", "Clipper2"] {
        let dir = crate::find_lib_recursive(&manifold_build, libname).unwrap_or_else(|| {
            panic!(
                "could not find lib{libname}.a under {}. \
                 Expected at <build>/_deps/<name>-build/... per the helper's \
                 FetchContent layout — if this isn't where it landed, the \
                 wasm-cxx-shim helper or cmake's FetchContent module may have \
                 changed shape.",
                manifold_build.display()
            )
        });
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    println!("cargo:rustc-link-lib=static=manifoldc");
    println!("cargo:rustc-link-lib=static=manifold");
    println!("cargo:rustc-link-lib=static=Clipper2");

    let shim_lib_dirs = [
        shim_build.join("libcxx"),
        shim_build.join("libc"),
        shim_build.join("libm"),
    ];
    for d in &shim_lib_dirs {
        println!("cargo:rustc-link-search=native={}", d.display());
    }
    println!("cargo:rustc-link-lib=static=wasm-cxx-shim-libcxx");
    println!("cargo:rustc-link-lib=static=wasm-cxx-shim-libc");
    println!("cargo:rustc-link-lib=static=wasm-cxx-shim-libm");

    // No `cargo:rustc-link-lib=c++/stdc++` on this target — wasm-cxx-shim
    // covers the C++ runtime. (Same reason emscripten skips it: emcc's
    // libc++ is auto-linked; ours is provided by the shim.)
}
