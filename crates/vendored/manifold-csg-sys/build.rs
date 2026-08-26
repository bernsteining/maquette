#[path = "build/build.rs"]
mod build;
#[path = "build/fetch.rs"]
mod fetch;
#[path = "build/patch.rs"]
mod patch;
#[path = "build/wasm.rs"]
mod wasm;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Pinned upstream manifold version. Accepts a tag (e.g., "v3.5.0"),
/// branch, or commit SHA. Prefer SHAs for reproducibility (and
/// post-release fixes), tags for tagged releases.
///
/// Used by both the host build (which clones this directly) and the
/// wasm-uu build (which passes it through to the shim's
/// `wasm_cxx_shim_add_manifold()` helper as `MANIFOLD_GIT_TAG`, so both
/// paths build against the same C API surface).
pub(crate) const MANIFOLD_VERSION: &str = "v3.5.2";

/// Detect `sccache` on PATH and return cmake args that route the C/C++
/// compiler through it as a launcher. Returns empty if sccache isn't
/// installed, or if `MANIFOLD_CSG_NO_SCCACHE` is set in the environment.
///
/// sccache caches compiler outputs keyed by input hash, so the cmake
/// build re-uses .o files across `cargo clean` / target-dir wipes (which
/// otherwise discard everything under OUT_DIR). The first build is no
/// faster, but every rebuild that hits the cache skips the C++ compile
/// entirely. See issue #45.
///
/// We rerun-if-env-changed on the opt-out var and on `RUSTC_WRAPPER`
/// (which often holds sccache for the Rust side) so users get consistent
/// invalidation when they flip sccache on or off.
fn cmake_launcher_args() -> Vec<String> {
    // Memoize across the multiple call sites so the rerun-if-env-changed
    // directives and the user-facing `cargo:warning` fire exactly once
    // per build-script run, even on the wasm32-uu path that invokes cmake
    // configure twice (shim build + manifold wrapper build).
    static ARGS: OnceLock<Vec<String>> = OnceLock::new();
    ARGS.get_or_init(|| {
        println!("cargo:rerun-if-env-changed=MANIFOLD_CSG_NO_SCCACHE");
        println!("cargo:rerun-if-env-changed=RUSTC_WRAPPER");
        if env::var("MANIFOLD_CSG_NO_SCCACHE").is_ok() {
            return Vec::new();
        }
        // `output()` only succeeds if sccache is actually invocable. Avoids
        // false positives from stale shims.
        let available = Command::new("sccache")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !available {
            return Vec::new();
        }
        println!(
            "cargo:warning=manifold-csg-sys: routing C/C++ compiles through sccache. \
             Set MANIFOLD_CSG_NO_SCCACHE=1 to disable."
        );
        vec![
            "-DCMAKE_C_COMPILER_LAUNCHER=sccache".to_string(),
            "-DCMAKE_CXX_COMPILER_LAUNCHER=sccache".to_string(),
        ]
    })
    .clone()
}

/// Whether the target is an Apple platform, which ships libc++ (not
/// libstdc++) and uses the `.dylib` shared-library extension. Keyed on the
/// target *vendor* (`CARGO_CFG_TARGET_VENDOR`), which is `apple` for every
/// Apple triple - macOS, iOS, tvOS, watchOS, visionOS - and nothing else.
/// Using the vendor covers current and future Apple targets without an
/// OS allow-list to keep in sync.
pub(crate) fn is_apple(target_vendor: &str) -> bool {
    target_vendor == "apple"
}

/// Link against an externally-provided manifold install instead of
/// cloning and compiling it. Driven by `MANIFOLD_CSG_LIB_DIR` (a
/// directory containing `libmanifoldc` + `libmanifold`, e.g. a nixpkgs
/// `manifold` output's `lib/`). Returns `true` if the override was active
/// and link directives were emitted, `false` if the var was unset (caller
/// falls through to the clone+cmake path).
///
/// This is the fully-offline path: no network, no C++ compile. Because we
/// have no build-time header dependency (the FFI is hand-written `extern`
/// declarations, not bindgen), only the libraries are needed.
///
/// Link kind defaults to `dylib` (matching how distros, nixpkgs, and the
/// upstream flake build manifold - `BUILD_SHARED_LIBS=ON`). A shared
/// `libmanifold` records its own `Clipper2`/`tbb`/`stdc++` dependencies as
/// `NEEDED`, so we don't re-link them. Set `MANIFOLD_CSG_LIB_KIND=static`
/// for a static external build; then we additionally link `Clipper2` (and
/// `tbb` under the `parallel` feature), since a static archive carries no
/// transitive deps.
///
/// Caller must restrict this to host targets - the wasm/emscripten lanes
/// have their own build paths and are bypassed before this is consulted.
fn try_external_lib(target_env: &str, target_os: &str, target_vendor: &str) -> bool {
    // `env::var` returns Ok("") for a set-but-empty var, so a blank or
    // unset-via-expansion `MANIFOLD_CSG_LIB_DIR=$UNSET cargo build` would
    // otherwise hijack the build and emit an empty link-search. Treat
    // trimmed-blank as unset and fall through to the source build.
    let lib_dir = match env::var("MANIFOLD_CSG_LIB_DIR") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            // Surface a likely mistake: KIND only takes effect alongside DIR.
            if env::var("MANIFOLD_CSG_LIB_KIND").is_ok_and(|v| !v.trim().is_empty()) {
                println!(
                    "cargo:warning=manifold-csg-sys: MANIFOLD_CSG_LIB_KIND is set but \
                     MANIFOLD_CSG_LIB_DIR is not - ignoring it and building from source."
                );
            }
            return false;
        }
    };
    let kind = env::var("MANIFOLD_CSG_LIB_KIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "dylib".to_string());
    assert!(
        kind == "dylib" || kind == "static",
        "MANIFOLD_CSG_LIB_KIND must be 'dylib' or 'static', got {kind:?}"
    );

    // Fail loudly here rather than deferring a typo'd path or a dir missing
    // the libs to an opaque downstream linker error. We check the flat dir
    // for the exact filenames the linker will resolve for `-l{name}`.
    let lib_path = Path::new(&lib_dir);
    assert!(
        lib_path.is_dir(),
        "MANIFOLD_CSG_LIB_DIR={lib_dir:?} is not a directory"
    );
    // For static we also emit `-lClipper2` below, so verify it too - else a
    // dir with the manifold archives but no Clipper2 passes this check and
    // then dies with exactly the opaque linker error we're guarding against.
    // (The from-source path checks Clipper2 for the same reason.) `tbb` is
    // handled separately below: its presence is best-effort, not required.
    let mut required = vec!["manifoldc", "manifold"];
    if kind == "static" {
        required.push("Clipper2");
    }
    for name in required {
        assert!(
            external_lib_present(lib_path, name, &kind, target_env, target_os, target_vendor),
            "MANIFOLD_CSG_LIB_DIR={lib_dir:?} has no {kind} '{name}' library (looked for \
             {names:?} directly in that dir); check the path and MANIFOLD_CSG_LIB_KIND",
            names = external_lib_filenames(name, &kind, target_env, target_os, target_vendor),
        );
    }

    println!("cargo:rustc-link-search=native={lib_dir}");
    println!("cargo:rustc-link-lib={kind}=manifoldc");
    println!("cargo:rustc-link-lib={kind}=manifold");
    if kind == "static" {
        // Static archives carry no transitive deps - link them ourselves.
        // Mirrors the static link set from `super::build`.
        println!("cargo:rustc-link-lib=static=Clipper2");
        if env::var("CARGO_FEATURE_PARALLEL").is_ok() {
            // TBB's static archive is named differently across platforms
            // (libtbb.a, tbb12.lib, tbb12_static.lib). Probe the same flat
            // dir the linker actually searches (we emit a single
            // link-search=native above), so the name we pick is one that
            // links - keeping the probe consistent with the emit.
            match ["tbb", "tbb12", "tbb12_static"].into_iter().find(|n| {
                external_lib_present(lib_path, n, "static", target_env, target_os, target_vendor)
            }) {
                Some(tbb_name) => println!("cargo:rustc-link-lib=static={tbb_name}"),
                None => {
                    // Don't hard-fail: a static manifold may fold TBB in or be
                    // built without it. Warn (so a later `-ltbb` failure is
                    // explained) and emit the default name as a best effort.
                    println!(
                        "cargo:warning=manifold-csg-sys: parallel feature is on with \
                         MANIFOLD_CSG_LIB_KIND=static, but no tbb/tbb12/tbb12_static archive \
                         was found in {lib_dir}; linking will use -ltbb and may fail. If your \
                         static manifold bundles TBB or was built without it, ignore this."
                    );
                    println!("cargo:rustc-link-lib=static=tbb");
                }
            }
        }
    }

    // C++ standard library, same logic as the from-source build. MSVC links
    // it automatically; Apple platforms use libc++; everything else libstdc++.
    if target_env != "msvc" {
        if is_apple(target_vendor) {
            println!("cargo:rustc-link-lib=c++");
        } else {
            println!("cargo:rustc-link-lib=stdc++");
        }
    }

    println!(
        "cargo:warning=manifold-csg-sys: linking external manifold from \
         MANIFOLD_CSG_LIB_DIR={lib_dir} (kind={kind}); skipping clone + cmake. \
         You are responsible for matching the pinned upstream version (see \
         MANIFOLD_VERSION) and build flags (CROSS_SECTION, C bindings)."
    );
    if kind == "dylib" {
        println!(
            "cargo:warning=manifold-csg-sys: kind=dylib links at build time only; the lib \
             dir must also be on the runtime search path (rpath or LD_LIBRARY_PATH) or the \
             final binary fails to load libmanifold at startup. Nix buildInputs sets rpath \
             automatically."
        );
    }
    true
}

/// Candidate filenames the platform linker resolves for `-l{name}` of the
/// given `kind`. A file matching one of these in the lib dir is one that
/// actually links, so it doubles as the existence-check target set.
fn external_lib_filenames(
    name: &str,
    kind: &str,
    target_env: &str,
    target_os: &str,
    target_vendor: &str,
) -> Vec<String> {
    if kind == "static" {
        if target_env == "msvc" {
            vec![format!("{name}.lib")]
        } else {
            vec![format!("lib{name}.a")]
        }
    } else if target_env == "msvc" {
        // The DLL's import library is what the linker consumes.
        vec![format!("{name}.lib"), format!("{name}.dll.lib")]
    } else if target_os == "windows" {
        // windows-gnu (mingw): `-l{name}` resolves to a DLL import lib, not a
        // `.so`. Cover the mingw import-lib name and the plainer fallbacks.
        vec![
            format!("lib{name}.dll.a"),
            format!("lib{name}.a"),
            format!("{name}.lib"),
        ]
    } else if is_apple(target_vendor) {
        vec![format!("lib{name}.dylib")]
    } else {
        vec![format!("lib{name}.so")]
    }
}

/// Whether a `{kind}` library `name` is present directly in `dir` (flat,
/// matching the single `link-search=native` we emit). `exists()` follows
/// symlinks, so a versioned `.so` reached via its dev symlink counts.
fn external_lib_present(
    dir: &Path,
    name: &str,
    kind: &str,
    target_env: &str,
    target_os: &str,
    target_vendor: &str,
) -> bool {
    external_lib_filenames(name, kind, target_env, target_os, target_vendor)
        .iter()
        .any(|f| dir.join(f).exists())
}

/// Recursively search for a static library under `dir`.
///
/// Searches for `lib{name}.a` (Unix) and `{name}.lib` (MSVC).
fn find_lib_recursive(dir: &Path, name: &str) -> Option<PathBuf> {
    let unix_target = format!("lib{name}.a");
    let msvc_target = format!("{name}.lib");
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_lib_recursive(&path, name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .is_some_and(|f| f == unix_target.as_str() || f == msvc_target.as_str())
        {
            return path.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn main() {
    // docs.rs builds with --network=none, so we can't clone manifold3d.
    // The FFI declarations are just extern signatures — skip the C build
    // entirely and let rustdoc generate docs from the Rust source alone.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    // Read target info from cargo (build-script-host cfg! is wrong for cross-compiling).
    let target = env::var("TARGET").unwrap_or_default();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_vendor = env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();
    let is_emscripten = target_os == "emscripten";
    // wasm32-unknown-unknown — bare wasm without WASI or Emscripten. Browser
    // target for wasm-bindgen consumers. Has its own dedicated build path
    // that consumes wasm-cxx-shim for the C/C++ runtime; the rest of this
    // function (clone manifold, host cmake, etc.) is bypassed.
    let is_wasm_unknown_unknown =
        target_arch == "wasm32" && target_os == "unknown" && target_env.is_empty();

    if is_wasm_unknown_unknown {
        wasm::build_wasm_unknown_unknown();
        return;
    }

    // Prevent unnecessary build script re-execution. Emitted before any
    // early return below so the rerun rules apply on every host code path
    // (including the MANIFOLD_CSG_LIB_DIR override) - emitting any
    // rerun-if-* directive opts out of cargo's default "rerun on package
    // change", so these must be unconditional.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=patches");

    // Offline / bring-your-own build override (host only). Lets consumers
    // who can't run the build-script's `git clone` (sandboxed builders like
    // Nix, airgapped CI) link a pre-built manifold install via
    // `MANIFOLD_CSG_LIB_DIR`, skipping clone AND cmake entirely. Fully
    // offline, no C++ compile. See issue #49.
    println!("cargo:rerun-if-env-changed=MANIFOLD_CSG_LIB_DIR");
    println!("cargo:rerun-if-env-changed=MANIFOLD_CSG_LIB_KIND");
    if !is_emscripten && try_external_lib(&target_env, &target_os, &target_vendor) {
        return;
    }

    if is_emscripten {
        // emcmake/emmake wrap cmake to inject the Emscripten toolchain. They
        // come from the Emscripten SDK (`brew install emscripten` or the raw
        // emsdk install path; either way the binaries need to be on PATH).
        if Command::new("emcmake").output().is_err() {
            panic!(
                "Building for {target} requires the Emscripten SDK on PATH \
                 (emcmake, emmake, emcc). Install via `brew install emscripten`, \
                 or run `source emsdk_env.sh` from a raw emsdk checkout. \
                 See docs/plans/wasm-emscripten.md."
            );
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let patches_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("patches");
    let manifold_src = out_dir.join("manifold-src");
    let build_dir = out_dir.join("build");
    fetch::fetch(&out_dir, &manifold_src, &build_dir, &patches_dir);
    build::build(
        is_emscripten,
        &manifold_src,
        &build_dir,
        &target,
        &target_env,
        &target_vendor,
    )
}
