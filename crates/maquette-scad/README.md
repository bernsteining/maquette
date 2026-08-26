# maquette-scad

A [Typst](https://typst.app) plugin that lets you write **procedural CAD in Typst source** using an OpenSCAD-flavored DSL, compile it to a mesh in-browser (no OpenSCAD install), and hand the result straight to [`maquette`](../../maquette/README.md) (STL/OBJ/PLY renderer) or [`maquette-gltf`](../maquette-gltf/README.md) for rendering.

The heavy lifting is a full CSG evaluator sitting on top of the **Manifold** kernel (elalish/manifold, via the `manifold-csg` crate). Manifold guarantees watertight, correctly-triangulated boolean output — none of the "20% of faces open on any non-trivial cut" issue you get from BSP-based CSG kernels. See [`FIDELITY.md`](FIDELITY.md) for the full OpenSCAD language coverage tracker.

## Usage

Two entry points: a **Typst DSL** for terse in-document geometry, and a **`.scad` text ingestion** path for existing OpenSCAD files.

### Typst DSL

```typst
#import "@preview/maquette-scad:0.1.0": *
#import "@preview/maquette:0.1.3": render-ply

// Compose primitives + booleans + transforms directly in Typst.
#let part = scadypst(
  difference(
    cube(20, center: true),
    sphere(12, fn: 48),
  )
)

// Hand the PLY bytes to the renderer.
#render-ply(part, color: "#4488cc", shading: "gooch")
```

Primitives, transforms, and boolean operators map 1:1 to their OpenSCAD names (`cube`, `sphere`, `cylinder`, `translate`, `rotate`, `scale`, `union`, `difference`, `intersection`, `hull`, `minkowski`, `linear-extrude`, `rotate-extrude`, `polyhedron`, `polygon`, ...). Use Typst's own `for` / `range` / `calc` for procedural placement — no separate `for` loop syntax needed.

### `.scad` files

```typst
#import "@preview/maquette-scad:0.1.0": compile-scad
#import "@preview/maquette:0.1.3": render-ply

#let part = compile-scad(read("my-model.scad"))
#render-ply(part)
```

Takes real OpenSCAD source text (parsed via [`openscad-rs`](https://crates.io/crates/openscad-rs), evaluated in-crate). The full language surface — variables, `let()`, `function`, `module`, `for`, control flow, math ops, `$fn`/`$fa`/`$fs`, `include`/`use`, list comprehensions — is covered per [`FIDELITY.md`](FIDELITY.md).

## What's supported

- **CSG kernel:** Manifold (watertight guaranteed).
- **All primitives** — 2D (`square`, `circle`, `polygon`, `text`) and 3D (`cube`, `sphere`, `cylinder`, `polyhedron`).
- **All transforms** — `translate`, `rotate`, `scale`, `mirror`, `resize`, `multmatrix`, `color`, `offset`.
- **All booleans** — `union`, `difference`, `intersection`.
- **Extrusions** — `linear_extrude` (with twist/scale), `rotate_extrude`.
- **Hull + minkowski** (3D).
- **Language features** — `function`, `module`, `let()`, `for`, `if`, `intersection_for`, `render`, list comprehensions, string ops, math functions.
- **Special vars** — `$fn`, `$t`, `$preview`, `$children` (full); `$fa`, `$fs` (defined but not adaptively tessellated — see FIDELITY.md).
- **`include` / `use`** — for embedded module libraries.
- **Colored output** — `color()` blocks propagate to per-face RGB in the emitted PLY.

## What's not supported

See [`FIDELITY.md`](FIDELITY.md) for the exhaustive coverage tracker. Highlights of what's **missing** (all marked ❌ or 🟡 in FIDELITY):

- `import()` of external STL/OBJ/DXF files at compile time (wasm sandbox has no filesystem).
- `surface()` heightmap import (same reason).
- `$vpr` / `$vpt` / `$vpd` / `$vpf` viewport variables (n/a — we don't have a viewport at eval time).
- Adaptive tessellation via `$fa` / `$fs` (defined for library compatibility but our primitive builder uses `$fn` only).
- Text-shape rendering that needs a font file (basic Latin-1 skeletons only).

Anything in the [OpenSCAD cheat sheet](https://openscad.org/cheatsheet/) not marked ❌ in FIDELITY.md should evaluate correctly. Anything that emits geometry produces a watertight mesh.

## Building from source

```sh
make scad-wasm    # release wasm into crates/maquette-scad/maquette-scad.wasm
```

The wasm build links Manifold's C++ CSG kernel in-crate via [`wasm-cxx-shim`](https://crates.io/crates/wasm-cxx-shim) — the module has zero host imports and runs under Typst's `wasmi` interpreter. Requires `clang` + `cmake` for the native probe harnesses; the wasm build itself only needs `cargo` + `wasm-opt`.

The full wasm module is roughly 3 MB after `wasm-opt -O3` — mostly the Manifold kernel. Amortized: the CSG eval is compiled in the browser at demo time (JIT), where it runs an order of magnitude faster than Typst's interpreter, so live iteration on `.scad` source is snappy.

## Language coverage

Detailed status per feature: [`FIDELITY.md`](FIDELITY.md). TL;DR — everything a real-world `.scad` file typically uses is ✅. The gaps are (1) external-asset imports the wasm sandbox blocks and (2) preview-viewport variables that only matter inside OpenSCAD's own GUI.

## License

MIT.
