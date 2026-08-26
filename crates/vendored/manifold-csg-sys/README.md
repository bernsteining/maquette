# manifold-csg

[![crates.io](https://img.shields.io/crates/v/manifold-csg.svg)](https://crates.io/crates/manifold-csg)
[![docs.rs](https://docs.rs/manifold-csg/badge.svg)](https://docs.rs/manifold-csg)
[![CI](https://github.com/zmerlynn/manifold-csg/actions/workflows/ci.yml/badge.svg)](https://github.com/zmerlynn/manifold-csg/actions/workflows/ci.yml)

Safe Rust bindings to the [manifold3d](https://github.com/elalish/manifold)
geometry kernel for constructive solid geometry (CSG).

manifold3d is a fast, robust C++ library for boolean operations on 3D triangle
meshes. These bindings make its capabilities accessible from Rust with minimal
overhead and without requiring users to manage C pointers or memory. See the
[upstream documentation](https://elalish.github.io/manifold/docs/html/) for
details on the underlying algorithms and behavior.

## Contents

- [What's included](#whats-included)
- [Design choices](#design-choices)
- [Quick start](#quick-start)
- [Crates](#crates)
- [Build requirements](#build-requirements)
  - [Offline / pre-built manifold](#offline--pre-built-manifold)
  - [Browser / WebAssembly (`wasm32-unknown-emscripten`)](#browser--webassembly-wasm32-unknown-emscripten)
  - [Browser without Emscripten (`wasm32-unknown-unknown`)](#browser-without-emscripten-wasm32-unknown-unknown)
- [Feature flags](#feature-flags)
- [Documentation](#documentation)
- [License](#license)

## What's included

**`manifold-csg-sys`** provides raw FFI bindings to the manifold3d C API. If
you need direct C-level control, it's there.

**`manifold-csg`** wraps the most commonly needed operations in safe Rust:

- **3D solids** ([`Manifold`](crates/manifold-csg/src/manifold.rs)) — primitives
  (cube, sphere, cylinder, tetrahedron), boolean operations (union, difference,
  intersection), transforms, convex hull, decomposition, Minkowski sum/difference,
  mesh refinement, smoothing, SDF level sets, warp deformation, and OBJ I/O
- **2D regions** ([`CrossSection`](crates/manifold-csg/src/cross_section.rs)) —
  primitives (square, circle, polygons), boolean operations, Clipper2-based
  geometric offset, convex hull, transforms, warp, and simplification
- **Mesh data** ([`MeshGL64`](crates/manifold-csg/src/mesh.rs) /
  [`MeshGL`](crates/manifold-csg/src/mesh.rs)) — f64 and f32 mesh types for
  getting data in and out
- **Triangulation** ([`triangulate_polygons`](crates/manifold-csg/src/triangulation.rs))
  — constrained Delaunay triangulation of 2D polygons
- **2D-to-3D** — extrude (with optional twist and scale) and revolve cross-sections
  into solids; slice solids back to cross-sections

See [API_COVERAGE.md](API_COVERAGE.md) for a full table mapping every C API function
to its safe wrapper (or noting where one doesn't exist yet).

## Design choices

- **f64 by default.** Mesh I/O uses `MeshGL64` so you don't lose precision
  through f32 round-trips. f32 paths (`from_mesh_f32`, `to_mesh_f32`, `MeshGL`)
  are available when you need them.
- **`Send` + `Sync`.** All types can be moved across threads and shared for
  concurrent reads.
- **Automatic memory management.** All C handles are freed via `Drop`. No manual
  cleanup needed.
- **Operator overloads.** `&a + &b` (union), `&a - &b` (difference), `&a ^ &b`
  (intersection) work on both `Manifold` and `CrossSection`.
- **Callback-based APIs wrapped safely.** `warp`, `set_properties`, `from_sdf`,
  and OBJ I/O all accept closures with `catch_unwind` to prevent panics from
  unwinding through C stack frames.
- **C API parity.** Parameter order and names follow the C API so users
  transitioning from C/C++ find things where they expect.

## Quick start

```rust
use manifold_csg::{Manifold, CrossSection, JoinType};

// 3D: drill a cylindrical hole through a cube
let cube = Manifold::cube(20.0, 20.0, 20.0, true);
let hole = Manifold::cylinder(30.0, 5.0, 5.0, 32, false);
let result = &cube - &hole;
assert!(result.volume() < cube.volume());

// 2D -> 3D: offset a rectangle and extrude it
let section = CrossSection::square(10.0, 10.0, true);
let expanded = section.offset(2.0, JoinType::Round, 2.0, 16);
let solid = expanded.extrude(20.0);
```

See the [`examples/`](crates/manifold-csg/examples/) directory for more
complete, runnable examples.

## Crates

| Crate | Description |
|-------|-------------|
| [`manifold-csg`](crates/manifold-csg/) | Safe Rust wrapper (start here) |
| [`manifold-csg-sys`](crates/manifold-csg-sys/) | Raw FFI bindings to the full C API |

## Build requirements

- Rust 1.85+
- git, cmake, a C++ compiler
- First build clones manifold3d and compiles it from source; subsequent builds
  use the cached copy. Internet access is required for the initial clone.

```sh
cargo build           # builds both crates
cargo test --features nalgebra   # runs the test suite
```

### Offline / pre-built manifold

For sandboxed or airgapped builders (Nix, restricted CI) that can't run the
initial clone, set `MANIFOLD_CSG_LIB_DIR` to a directory holding a pre-built
manifold install (`libmanifoldc` + `libmanifold`, e.g. `${pkgs.manifold}/lib`).
This skips the clone *and* the C++ compile entirely. Set
`MANIFOLD_CSG_LIB_KIND=static` if the install is static (default is `dylib`).
The pre-built manifold must match the pinned upstream version and be built with
the C bindings + cross-section. For the default `dylib` kind, the directory must
also be on the runtime library search path (rpath / `LD_LIBRARY_PATH`); under
Nix, `buildInputs = [ manifold ]` arranges this for you.

Packaging a crate that depends on this one, with Nix:

```nix
buildRustPackage {
  # ...
  buildInputs = [ pkgs.manifold ];               # rpath; NEEDED pulls clipper2 + tbb
  MANIFOLD_CSG_LIB_DIR = "${pkgs.manifold}/lib"; # skips the clone and cmake
}
```

nixpkgs' `manifold` tracks the upstream tag, and this crate pins the same
version, so the two line up without extra work. If you pin an older nixpkgs,
check that its `manifold` still matches: `build.rs` validates that the
libraries are present but cannot check their version.

A repo-root `flake.nix` wires the same thing up as a devShell for working on
this repo: `nix develop` drops you into a shell that links nixpkgs' prebuilt
`manifold` via `MANIFOLD_CSG_LIB_DIR`, so `cargo build` / `cargo test` skip
the manifold3d clone and C++ compile entirely.

Tested on Linux, macOS, Windows, `wasm32-unknown-emscripten`, and
`wasm32-unknown-unknown` (see below).

### Browser / WebAssembly (`wasm32-unknown-emscripten`)

`manifold-csg` builds and runs in the browser via Emscripten. The C++ kernel
is compiled with `emcmake`/`emmake`; Rust links against it via emcc.

```sh
brew install emscripten   # or install the raw emsdk and source emsdk_env.sh
rustup target add wasm32-unknown-emscripten
cargo build --target wasm32-unknown-emscripten -p manifold-csg --no-default-features
```

The integration test suite runs under Node (the workspace `.cargo/config.toml`
sets the runner, so no env var needed):

```sh
cargo test --target wasm32-unknown-emscripten -p manifold-csg --no-default-features --tests
```

Notes:

- The `parallel` feature is silently disabled on emscripten — TBB requires
  pthread support which in turn needs the host page to provide `SharedArrayBuffer`
  via COOP/COEP HTTP headers. Use `--no-default-features` to opt out cleanly.
- Tests using `std::thread::spawn` are marked `#[ignore]` on this target for
  the same reason.
- For `wasm32-unknown-unknown` support (the wasm-bindgen-compatible
  target), see the next section.
- End-user crates that produce their own `bin`/`cdylib` from a wasm build
  need to forward the same emscripten link flags. Add a one-line `build.rs`
  that re-emits `DEP_MANIFOLD_LINK_ARGS`, or set them via `.cargo/config.toml`.
- Existing examples build for wasm too — e.g.
  `cargo build --example basics --target wasm32-unknown-emscripten -p manifold-csg --no-default-features`
  produces a `.wasm` + `.js` shim you can run with `node target/wasm32-unknown-emscripten/debug/examples/basics.js`.

### Browser without Emscripten (`wasm32-unknown-unknown`)

`manifold-csg` also builds for the bare-wasm target — the same one
`wasm-bindgen` consumers (Bevy, Leptos, Yew, etc.) target. The C++
runtime gap (`wasm32-unknown-unknown` ships no libc, no libc++, no
libc++abi) is filled by [`wasm-cxx-shim`](https://github.com/zmerlynn/wasm-cxx-shim),
which is cloned and built automatically by `build.rs`.

**This target is provisional.** The build carries patches against upstream
manifold and Clipper2, ships without an exception runtime (implicit STL
throws abort), and disables OBJ I/O. To acknowledge these constraints,
the `unstable-wasm-uu` cargo feature is required for any build targeting
`wasm32-unknown-unknown`. Without it, `build.rs` aborts with an instructive
error.

```toml
manifold-csg = { version = "...", default-features = false, features = ["unstable-wasm-uu"] }
```

Requirements:

- A wasm-capable LLVM 20+ install:
  - macOS: `brew install llvm` (then add `/opt/homebrew/opt/llvm@20/bin` to PATH)
  - Debian: `apt install clang-20 lld-20 libc++-20-dev libc++abi-20-dev`
- Rust target: `rustup target add wasm32-unknown-unknown`
- If LLVM is in a non-standard location, set
  `WASM_CXX_SHIM_LLVM_BIN_DIR=/path/to/llvm/bin` in your environment.

```sh
cargo build --target wasm32-unknown-unknown -p manifold-csg \
    --no-default-features --features unstable-wasm-uu
```

A runnable smoke example exercises a real CSG operation under Node:

```sh
cargo build --example wasm32_uu_smoke --target wasm32-unknown-unknown \
    -p manifold-csg --no-default-features --features unstable-wasm-uu
node crates/manifold-csg/wasm32-uu-runner/run.mjs \
    target/wasm32-unknown-unknown/debug/examples/wasm32_uu_smoke.wasm
# wasm32-uu-smoke: smoke_run() = 36 (triangle count, > 0) ✓
```

For an interactive end-to-end demo see the **[boolean playground](https://zmerlynn.github.io/manifold-csg/)** —
a browser app that wires `manifold-csg` into [three.js](https://threejs.org/) via a
small C ABI and lets you union/diff/intersect cubes/spheres/cylinders with
transform gizmos. Source under
[`crates/manifold-csg-playground/`](crates/manifold-csg-playground/) (auto-deployed
to GitHub Pages).

Notes:

- First build is slow (clones manifold + Clipper2 + wasm-cxx-shim, builds
  three cmake projects in cross-compile). Subsequent builds use the cached
  artifacts.
- The `parallel` feature is unavailable on this target (no threads in
  default `wasm32-unknown-unknown`).
- OBJ I/O is unavailable on this target (`Manifold::from_obj`/`to_obj` and
  `MeshGL64::from_obj`/`to_obj` are `#[cfg]`-elided). Manifold's iostream-
  based OBJ paths depend on libc++ machinery the freestanding wasm build
  excludes; use the binary `MeshGL64` API to round-trip mesh data instead.
- Exceptions abort. Compiled with `-fno-exceptions`; implicit STL throws
  (`bad_alloc`, etc.) become unrecoverable wasm traps.
- See [`docs/plans/wasm-unknown-unknown.md`](docs/plans/wasm-unknown-unknown.md)
  for the design background and production-readiness checklist.
- If the build fails, run `bash crates/manifold-csg-sys/wasm32-uu/diagnose.sh > bugreport.txt 2>&1` and attach the output to your issue — it captures the LLVM probe ladder, env vars, and cached build state.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `parallel` | yes | Enables TBB-based parallelism for boolean operations |
| `nalgebra` | no | Adds convenience methods that accept `nalgebra::Matrix3`, `Vector3`, `Point3` |
| `unstable-wasm-uu` | no | **Required** when targeting `wasm32-unknown-unknown`. Acknowledges that target's provisional status (carry-patches, no exceptions, no OBJ I/O). Has no effect on other targets. |

## Documentation

- **[API_COVERAGE.md](API_COVERAGE.md)** — maps every manifold3d C function to
  its safe wrapper, with source links
- **[docs.rs](https://docs.rs/manifold-csg)** — generated API docs
- **[examples/](crates/manifold-csg/examples/)** — runnable code examples
- **[Upstream docs](https://elalish.github.io/manifold/docs/html/)** — manifold3d
  C++ API documentation (helpful for understanding parameter semantics)
- **Migration guides** — one per breaking-change version pair, each structured for AI-assisted migration:
  - [from 0.0.6](https://github.com/zmerlynn/manifold-csg/blob/main/MIGRATION_FROM_0.0.6.md) — original `manifold3d` crate line (pre-transfer to this project).
  - [0.1.8 → 0.2.0](https://github.com/zmerlynn/manifold-csg/blob/main/MIGRATION_0.1.8_TO_0.2.0.md) — `ExecutionContext` attachment API reshape.
  - [0.2.0 → 0.3.0](https://github.com/zmerlynn/manifold-csg/blob/main/MIGRATION_0.2.0_TO_0.3.0.md) — more idiomatic `Result` status/mesh APIs and upstream-aligned defaults.

  These guides live at the repo root, so they are not part of the published crate tarball; the links above resolve on GitHub. A condensed [CHANGELOG.md](https://github.com/zmerlynn/manifold-csg/blob/main/crates/manifold-csg/CHANGELOG.md) ships inside the crate and flags behavioral (non-compile-breaking) changes.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
