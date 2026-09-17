# maquette-gltf

[![Typst Universe](https://img.shields.io/badge/Typst%20Universe-maquette--gltf-239dad)](https://typst.app/universe/package/maquette-gltf)
[![Live demo](https://img.shields.io/badge/demo-live-4f46e5)](https://bernsteining.github.io/maquette/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

Render **glTF 2.0** assets (`.glb` / `.gltf`) directly inside your Typst documents with `maquette-gltf`. 

**[Try it live →](https://bernsteining.github.io/maquette/?a=tokyo.glb)** load a glTF, orbit, scrub the animation, tweak every setting, and copy the generated Typst snippet.

Part of the [maquette](https://github.com/bernsteining/maquette) family (sibling to the STL/OBJ/PLY and OpenSCAD plugins), sharing the same render core.

## Usage

The same [Little Tokyo](https://sketchfab.com/3d-models/little-tokyo-diorama-6072d34e02454743a4a0f4d219c2c62c) diorama, twice: `time` scrubs the animation clip, and `camera` / `center` move the eye.

<table>
<tr>
<td>

```typst
#import "@preview/maquette-gltf:0.1.0": render-gltf

#let tokyo = read("tokyo.glb", encoding: none)

#render-gltf(tokyo, (
  camera: (-71.915, 159.083, 1026.801),
  center: (-86, 5, -25),
  up: (0, 0.989, -0.145),
  auto_center: false,
  auto_fit: false,
  background: "#182028",
  shadows: true,
  time: 5.4,
))
```

</td>
<td><a href="https://bernsteining.github.io/maquette/?model=tokyo.glb&camera=%5B-71.915%2C159.083%2C1026.801%5D&center=%5B-86%2C5%2C-25%5D&up=%5B0%2C0.989%2C-0.145%5D&auto_center=false&auto_fit=false&background=%22%23182028%22&shadows=true&time=5.4" title="Open in the live demo"><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/gltf-tokyo.png?v=2" width="340" alt="Little Tokyo diorama, front view" /></a></td>
</tr>
<tr>
<td>

```typst
// Same scene, camera and aim moved.

#render-gltf(tokyo, (
  camera: (-404.494, 348.337, 960.992),
  center: (-86, 5, -25),
  up: (0.052, 0.948, -0.314),
  auto_center: false,
  auto_fit: false,
  background: "#182028",
  shadows: true,
  time: 0.55,
))
```

</td>
<td><a href="https://bernsteining.github.io/maquette/?model=tokyo.glb&camera=%5B-404.494%2C348.337%2C960.992%5D&center=%5B-86%2C5%2C-25%5D&up=%5B0.052%2C0.948%2C-0.314%5D&auto_center=false&auto_fit=false&background=%22%23182028%22&shadows=true&time=0.55" title="Open in the live demo"><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/gltf-tokyo-camera.png?v=2" width="340" alt="Little Tokyo diorama from a high three-quarter angle" /></a></td>
</tr>
</table>

`get-gltf-info(read("tokyo.glb", encoding: none))` returns the triangle count, bounding box and `max_animation_time`: handy for framing the camera or bounding an animation slider.

## Documentation

[docs/maquette-gltf-documentation.pdf](https://github.com/bernsteining/maquette/blob/master/docs/maquette-gltf-documentation.pdf) walks through the config (camera, IBL, shadows, ground plane, tone mapping, SSAO/FXAA/SSAA) and the full list of supported extensions and texture formats.

## How it's built

maquette-gltf is a single pure-Rust crate compiled to `wasm32-unknown-unknown` ~ 1.6MB. The renderer itself lives in [`maquette-core`](https://github.com/bernsteining/maquette) and is written from scratch (matrix math, triangle rasterizer, Cook-Torrance PBR, image-based lighting, shadow maps, SSAA/SSAO/FXAA, HDR loader) with no third-party rendering dependencies. 

What it does piggyback on is parsing and decoding:

| Crate | What it does |
|---|---|
| [`gltf`](https://github.com/gltf-rs/gltf) | glTF 2.0 parsing plus 20+ KHR/EXT extensions (PBR variants, punctual lights, transmission, clearcoat, sheen, animation pointer, …). Pinned to a git rev because the crates.io release predates clearcoat/sheen. |
| [`meshopt-rs`](https://crates.io/crates/meshopt-rs) | `EXT_meshopt_compression` decoding |
| [`draco-oxide-decoder`](https://crates.io/crates/draco-oxide-decoder) | `KHR_draco_mesh_compression` decoding (decode-only, so the encoder never links) |
| [`mikktspace`](https://crates.io/crates/mikktspace) | consistent per-vertex tangents when an asset omits the `TANGENT` attribute |
| [`zune-png`](https://crates.io/crates/zune-png), [`zune-jpeg`](https://crates.io/crates/zune-jpeg), [`image-webp`](https://crates.io/crates/image-webp) | texture decoding (PNG / JPEG / `EXT_texture_webp`), shared with the OBJ/MTL path through `maquette-core` |
| [`wasm-minimal-protocol`](https://github.com/astrale-sharp/wasm-minimal-protocol) | the Typst plugin calling convention |
| [`serde_json`](https://crates.io/crates/serde_json) | parsing the render config |

Every dependency is pure Rust and wasm-friendly, so the build is a plain `cargo build --target wasm32-unknown-unknown` with no C toolchain (unlike maquette-scad, which links Manifold's C++ kernel).

## Building from source

```sh
make gltf-build   # cargo build → wasm-opt -O3 → install into the local Typst package dir
```
