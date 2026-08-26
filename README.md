# Maquette

[![Live demo](https://img.shields.io/badge/demo-live-4f46e5)](https://bernsteining.github.io/maquette/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

**Maquette is a family of Typst plugins for embedding 3D renders directly in your documents.** Change a parameter, recompile the `.typ`, and the render lands in your PDF — no external tools, no manual re-exports, no separate asset pipeline.

**[Try it live →](https://bernsteining.github.io/maquette/)** — a browser demo runs the exact same WebAssembly the plugins ship. Drag to orbit, tweak every setting, copy the generated Typst source. The demo runs the wasm through a browser JIT rather than Typst's interpreter, so it iterates ~10× faster than a document rebuild — the fastest way to dial in a shot before pasting the code into your file.

## The workspace

| Package | Kind | What it renders |
|---|---|---|
| **[maquette](maquette/README.md)** | Typst plugin | STL / OBJ / PLY, with a full CAD-style shading + material feature set |
| **[maquette-gltf](crates/maquette-gltf/README.md)** | Typst plugin | glTF 2.0 (`.glb` / `.gltf`) with PBR, IBL, KHR_materials_* extensions, Draco, quantization |
| **[maquette-scad](crates/maquette-scad/README.md)** | Typst plugin | OpenSCAD source text → in-browser mesh compilation → any maquette renderer |
| **[maquette-core](crates/maquette-core/README.md)** | Rust rlib | Shared render primitives (rasterizer, SSAA/SSAO/FXAA, shadow maps, IBL, HDR + texture decode) |

The three Typst plugins each compile to a single WebAssembly module and pull their render primitives from `maquette-core`. They ship independently on Typst Universe — you pick the one(s) matching your input format.

## Which one do I need?

- **Have `.stl` / `.obj` / `.ply` files, want CAD-style illustrations?** → [maquette](maquette/README.md). Multi-view grids, cross-sections, silhouettes, exploded views, cel/gooch/wireframe shading, per-group appearance overrides.
- **Have a `.glb` / `.gltf` artist-authored asset with PBR materials, textures, animations, or IBL?** → [maquette-gltf](crates/maquette-gltf/README.md).
- **Want to write geometry in code (parametric CAD in Typst)?** → [maquette-scad](crates/maquette-scad/README.md). Compiles OpenSCAD source right in the browser (via the Manifold CSG kernel), hands the mesh to either of the two rendering plugins above.

## Repo layout

```
.
├── README.md                       ← you are here
├── Cargo.toml                      ← workspace root + `maquette` (STL/OBJ/PLY plugin)
├── src/                            ← maquette's Rust sources
├── maquette/                       ← maquette Typst package (wrapper + wasm + docs)
│   └── README.md                   ← STL/OBJ/PLY plugin docs
├── crates/
│   ├── maquette-core/              ← shared render primitives (rlib)
│   │   └── README.md
│   ├── maquette-gltf/              ← glTF 2.0 plugin (cdylib)
│   │   ├── README.md               ← usage + supported/unsupported extensions
│   │   └── maquette-gltf/          ← Typst package (wrapper + wasm + typst.toml)
│   └── maquette-scad/              ← OpenSCAD plugin (cdylib)
│       ├── README.md
│       ├── FIDELITY.md             ← OpenSCAD language coverage tracker
│       └── maquette-scad.typ
├── docs/                           ← browser demo (GitHub Pages)
├── examples/                       ← sample assets + Typst source
├── harness/                        ← wasmi-based CLI for running the wasm off-Typst
└── Makefile                        ← build every plugin + demo assets
```

## Building

```sh
make build       # compile maquette (STL/OBJ/PLY) wasm, install into your Typst packages dir
make gltf-build  # same for maquette-gltf
make scad-build  # same for maquette-scad
make demo        # assemble the browser demo — three wasm modules + assets in docs/
make doc         # compile examples/documentation.pdf (a full walkthrough)
```

Requires `cargo`, the `wasm32-unknown-unknown` target, and `wasm-opt` from [binaryen](https://github.com/WebAssembly/binaryen).

## License

MIT.
