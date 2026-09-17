# maquette-scad

[![Typst Universe](https://img.shields.io/badge/Typst%20Universe-maquette--scad-239dad)](https://typst.app/universe/package/maquette-scad)
[![Live demo](https://img.shields.io/badge/demo-live-4f46e5)](https://bernsteining.github.io/maquette/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

Render OpenSCAD files in Typst with `maquette-scad`. 

Load `.scad` files, or use the `scadyst` DSL to render them with [maquette](https://github.com/bernsteining/maquette). 

**[Try it live →](https://bernsteining.github.io/maquette/?a=__scad__)**  edit SCAD source, orbit the result, tweak the render, and copy the generated Typst code snippet.

## Usage

Three ways in: a **`.scad` file** (the homepage example), the Typst **DSL**, and a whole **multi-file project** compiled by walking its `use`/`include` graph. All use the one-call `render-scad` / `render-scad-tree`, which compile and render in a single step with no separate `maquette` import.

<table>
<tr><th align="left">Code</th><th>Render</th></tr>
<tr><th colspan="2" align="left">1. OpenSCAD file</th></tr>
<tr>
<td>

```scad
// example.scad
$fn = 100;
module rod(d, h) cylinder(h, d = d, center = true);
hole = 25;
len = 62.5;
difference() {
  sphere(d = 50);
  rod(hole, len);
  rotate([90, 0, 0]) rod(hole, len);
}
color([0.5, 0.3, 0.1, 0.6])
rotate([0, 90, 0]) rod(hole, len);
```

```typst
#import "@preview/maquette-scad:0.1.0": render-scad

#render-scad(
  read("example.scad"),
  azimuth: 219,
  elevation: 33,
  up: (0, 1, 0),
  fov: 40,
  zoom: 1.2,
  background: none,
)
```

</td>
<td><a href="https://bernsteining.github.io/maquette/?model=openscad-logo.scad&azimuth=219&elevation=33&up=%5B0%2C1%2C0%5D&fov=40&zoom=1.2&color=%22%23f9d72c%22&specular=0&cull_backface=false&background=%22none%22" title="Open in the live demo"><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/scad-logo.png?v=3" width="300" alt="OpenSCAD logo compiled from source" /></a></td>
</tr>
<tr><th colspan="2" align="left">2. DSL</th></tr>
<tr>
<td>

```typst
#import "@preview/maquette-scad:0.1.0": *

#let bore = cylinder(30, r: 4.5, center: true, fn: 48)
#render-scad(
  color((0.16, 0.62, 0.71), difference(
    cube(20, center: true),
    sphere(12, fn: 64),
    ..((0, 0, 0), (90, 0, 0), (0, 90, 0)).map(r => rotate(r, bore)),
  )),
  smooth-normals: 25,
  azimuth: 32,
  elevation: 24,
  up: (0, 1, 0),
  specular: 0.35,
  background: none,
)
```

</td>
<td><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/scad-dsl.png" width="300" alt="CSG cube built with the Typst DSL" /></td>
</tr>
<tr><th colspan="2" align="left">3. Multiple OpenSCAD files</th></tr>
<tr>
<td>

```typst
#import "@preview/maquette-scad:0.1.0": *

#render-scad-tree(
  "Cyclone.scad",
  root: "cyclone-src/",
  read: p => read(p),
  ..openscad-view,
  azimuth: 35,
  elevation: 20,
  up: (0, 0, 1),
  background: none,
)
```

</td>
<td><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/docs/cyclone-example.png" width="300" alt="Cyclone-PCB-Factory CNC mill, a full multi-file OpenSCAD project" /></td>
</tr>
</table>

The Cyclone example is the [Cyclone-PCB-Factory](https://github.com/carlosgs/Cyclone-PCB-Factory).

## Rendering & options

`render-scad` and `render-scad-tree` apply the web-demo defaults (matte, two-sided, smooth) and forward every other argument to maquette's `render-ply`, so camera, lights, shading, specular, SSAO, shadows and colour maps all work (see the [maquette README](https://github.com/bernsteining/maquette/blob/master/crates/maquette/maquette/README.md)). One exception: `color:`/`materials:` are ignored — an OpenSCAD model carries its own per-face colours (`color()`, or the default yellow), so recolour with `color()` in the SCAD/DSL.

Shading is a free choice: the default is **smooth and matte**; `shading: "gooch"` gives a warm/cool **technical-diagram** style; spreading the `openscad-view` preset mimics OpenSCAD's own **flat-shaded** preview.

For full control, `scadypst`, `compile-scad` and `compile-scad-tree` return the PLY bytes directly, to hand to `render-ply` yourself.

## Documentation

[docs/maquette-scad-documentation.pdf](https://github.com/bernsteining/maquette/blob/master/docs/maquette-scad-documentation.pdf) has examples and the language walkthrough; [`FIDELITY.md`](FIDELITY.md) tracks exact OpenSCAD language coverage 

## How it's built

- **[Manifold](https://github.com/elalish/manifold)** (via the [`manifold-csg`](https://crates.io/crates/manifold-csg) crate) — the CSG kernel. It guarantees watertight, correctly-triangulated boolean output, so cuts and overlaps never leave open faces. On wasm its C++ is linked in-crate (through `wasm-cxx-shim`, built `-fno-exceptions`), keeping the module self-contained with zero host imports.
- **[`openscad-rs`](https://crates.io/crates/openscad-rs)** — parses real `.scad` source; the language evaluator (variables, `module`/`function`, `for`, list comprehensions, `$fn`, `use`/`include`) is written in-crate on top of it.
- **[`ttf-parser`](https://crates.io/crates/ttf-parser)** — glyph outlines for OpenSCAD `text()`; a subset DejaVu Sans ships as the default font, or pass your own.
- **[`rpds`](https://crates.io/crates/rpds)** + **[`archery`](https://crates.io/crates/archery)** + **[`rustc-hash`](https://crates.io/crates/rustc-hash)** — immutable scope maps with a fast (Fx) hasher for the evaluator.
- **[`wasm-minimal-protocol`](https://github.com/astrale-sharp/wasm-minimal-protocol)** the Typst plugin ABI.

A compile produces triangulated PLY bytes with per-face colours from any `color()` blocks which you hand to maquette's `render-ply`.

## Building from source

```sh
make scad-build   # cargo build → wasm-opt -O3 → install into the local Typst package dir
```

The wasm links Manifold's C++ CSG kernel in-crate — zero host imports, runs under Typst's `wasmi` interpreter.
