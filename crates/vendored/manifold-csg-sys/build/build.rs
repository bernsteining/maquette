//! cmake-configure and build the (already-cloned, already-patched)
//! manifold source tree, for host and emscripten targets.
//!
//! Picks up `parallel` (TBB), `MANIFOLD_CROSS_SECTION`, and standard
//! manifold options. On emscripten, swaps the cmake driver to
//! `emcmake`/`emmake` and sets the appropriate flags
//! (`-fwasm-exceptions`, `MANIFOLD_PAR=OFF`). Emits the
//! `cargo:rustc-link-lib=static` directives at the end, with library
//! paths discovered via [`super::find_lib_recursive`] so we don't have to
//! hardcode cmake's internal layout.
//!
//! Does NOT cover the wasm32-unknown-unknown path; that lives in
//! `super::wasm`, which uses the shim's CMake helper instead of driving
//! manifold's own cmake directly.

use std::{env, path::Path, process::Command};

use super::{cmake_launcher_args, find_lib_recursive, is_apple};

pub fn build(
    is_emscripten: bool,
    manifold_src: &Path,
    build_dir: &Path,
    target: &str,
    target_env: &str,
    target_vendor: &str,
) {
    // Configure with cmake.
    let mut parallel = env::var("CARGO_FEATURE_PARALLEL").is_ok();

    // Threading on emscripten requires SharedArrayBuffer + COOP/COEP HTTP
    // headers from the hosting page, which is too much friction to require
    // by default. Force MANIFOLD_PAR=OFF and warn if the user explicitly
    // asked for it.
    if is_emscripten && parallel {
        println!(
            "cargo:warning=manifold-csg-sys: 'parallel' feature is not supported on \
             {target}; building without TBB. Disable default-features or the \
             'parallel' feature to silence this warning."
        );
        parallel = false;
    }

    let mut cmake_args = vec![
        "-S".to_string(),
        manifold_src.to_str().unwrap().to_string(),
        "-B".to_string(),
        build_dir.to_str().unwrap().to_string(),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        "-DMANIFOLD_TEST=OFF".to_string(),
        "-DMANIFOLD_PYBIND=OFF".to_string(),
        "-DMANIFOLD_JSBIND=OFF".to_string(),
        "-DMANIFOLD_CBIND=ON".to_string(),
        "-DMANIFOLD_CROSS_SECTION=ON".to_string(),
        "-DMANIFOLD_USE_BUILTIN_CLIPPER2=ON".to_string(),
        "-DBUILD_SHARED_LIBS=OFF".to_string(),
        "-DCMAKE_POSITION_INDEPENDENT_CODE=ON".to_string(),
    ];

    if parallel {
        cmake_args.push("-DMANIFOLD_PAR=ON".to_string());
        cmake_args.push("-DMANIFOLD_USE_BUILTIN_TBB=ON".to_string());
    } else {
        cmake_args.push("-DMANIFOLD_PAR=OFF".to_string());
    }

    if is_emscripten {
        // Compile manifold's C++ with native wasm exception handling. Manifold's
        // C wrapper translates internal C++ exceptions into status codes; without
        // this flag those throws would trap-and-abort the wasm module on invalid
        // input. Must match the link-time -fwasm-exceptions emitted below.
        cmake_args.push("-DCMAKE_CXX_FLAGS=-fwasm-exceptions".to_string());
    }

    // Route C/C++ compiles through sccache when available so rebuilds after
    // `cargo clean` reuse object files rather than recompiling manifold from
    // scratch. No-op when sccache isn't installed.
    cmake_args.extend(cmake_launcher_args());

    // emcmake / emmake wrap cmake invocations to inject Emscripten's toolchain
    // file and substitute em++/emcc as the C++/C compiler.
    let make_cmake_cmd = |em_wrapper: &str| -> Command {
        if is_emscripten {
            let mut c = Command::new(em_wrapper);
            c.arg("cmake");
            c
        } else {
            Command::new("cmake")
        }
    };

    let cmake_output = make_cmake_cmd("emcmake")
        .args(&cmake_args)
        .output()
        .expect("failed to run cmake configure");
    if !cmake_output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&cmake_output.stderr));
        panic!("cmake configure failed");
    }

    // Build both manifold and manifoldc.
    let build_output = make_cmake_cmd("emmake")
        .args([
            "--build",
            build_dir.to_str().unwrap(),
            "--config",
            "Release",
            "--parallel",
        ])
        .output()
        .expect("failed to run cmake build");
    if !build_output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&build_output.stderr));
        panic!("cmake build failed");
    }

    // Find where libraries were placed and add search paths.
    //
    // On Unix, cmake puts libraries directly in the build subdirectories.
    // On MSVC, cmake's multi-config generator puts them in Release/ subdirs.
    // We use find_lib_recursive to handle both layouts reliably.
    let required_libs = ["manifoldc", "manifold", "Clipper2"];
    for lib_name in &required_libs {
        if let Some(lib_dir) = find_lib_recursive(build_dir, lib_name) {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
        }
    }

    // Link order matters: manifoldc depends on manifold, which depends on Clipper2 and TBB.
    println!("cargo:rustc-link-lib=static=manifoldc");
    println!("cargo:rustc-link-lib=static=manifold");
    println!("cargo:rustc-link-lib=static=Clipper2");

    // TBB (builtin, for parallel CSG operations — only when "parallel" feature is enabled).
    // On different platforms, the TBB static library has different names:
    // Unix: libtbb.a, MSVC: tbb.lib or tbb12.lib or tbb12_static.lib
    if parallel {
        let tbb_names = ["tbb", "tbb12", "tbb12_static"];
        let mut found_tbb = false;
        for name in &tbb_names {
            if let Some(tbb_dir) = find_lib_recursive(build_dir, name) {
                println!("cargo:rustc-link-search=native={}", tbb_dir.display());
                println!("cargo:rustc-link-lib=static={name}");
                found_tbb = true;
                break;
            }
        }
        if !found_tbb {
            // Fall back to letting the linker find it by the default name.
            println!("cargo:rustc-link-lib=static=tbb");
        }
    }

    // C++ standard library. Read target via env, not cfg! — cfg! evaluates at
    // build-script-host compile time, which silently lies under cross-compile.
    //
    // - MSVC links the C++ runtime automatically — no explicit link needed.
    // - Emscripten's emcc auto-links libc++ during the final wasm link.
    // - Apple SDKs (macOS, iOS, tvOS, watchOS, visionOS) only ship libc++;
    //   there is no libstdc++ to link, so `stdc++` fails at link time.
    if !is_emscripten && target_env != "msvc" {
        if is_apple(target_vendor) {
            println!("cargo:rustc-link-lib=c++");
        } else {
            println!("cargo:rustc-link-lib=stdc++");
        }
    }

    // Emscripten link flags. These need to reach the final rustc → emcc link
    // step in any binary/test/cdylib that depends on us, not just cmake's own
    // link step (which is a no-op here since BUILD_SHARED_LIBS=OFF).
    //
    // Plain `cargo:rustc-link-arg=` from a sys crate's build script does NOT
    // propagate to downstream link invocations — only `rustc-link-lib` and
    // `rustc-link-search` do. The proper sys-crate idiom is to expose the
    // flags as `links` metadata (here as DEP_MANIFOLD_LINK_ARGS, since this
    // crate has `links = "manifold"`), and have the safe wrapper crate's
    // build.rs read DEP_MANIFOLD_LINK_ARGS and re-emit `rustc-link-arg=` from
    // there. End-user binaries then need a similar build.rs (or a
    // `.cargo/config.toml` entry) to forward through to their own link.
    //
    // Documented in docs/plans/wasm-emscripten.md.
    if is_emscripten {
        let link_args: &[&str] = &[
            // Native wasm exception handling — must match -fwasm-exceptions
            // passed to the C++ compile above.
            "-fwasm-exceptions",
            // Allow the wasm linear memory to grow at runtime; the default
            // 16 MiB heap will OOM on the first non-trivial mesh.
            "-sALLOW_MEMORY_GROWTH=1",
            // Cap memory at the wasm32 ceiling (4 GiB) rather than the smaller
            // default, so growth doesn't trap on large boolean operations.
            "-sMAXIMUM_MEMORY=4294967296",
            // Bump the stack from emcc's small default (~5 MB). Manifold's
            // recursive CSG / topology routines hit stack overflow under the
            // default. Mirrors upstream's emscripten cmake configuration
            // (which uses 30 MB; round to 32 MiB).
            "-sSTACK_SIZE=33554432",
            // emcc requires INITIAL_MEMORY > STACK_SIZE, and the default
            // (16 MiB) is smaller than our stack. Bump to 64 MiB to give
            // headroom for stack + initial heap.
            "-sINITIAL_MEMORY=67108864",
        ];
        // Space-separated; downstream parses on whitespace. No flag here may
        // contain a literal space — if you ever need that (e.g. paths with
        // spaces in them), change to a different separator like ';' and update
        // crates/manifold-csg/build.rs to match.
        println!("cargo:link_args={}", link_args.join(" "));
    }
}
