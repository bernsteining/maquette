# maquette-gltf

[![Typst Universe](https://img.shields.io/badge/Typst%20Universe-maquette--gltf-239dad)](https://typst.app/universe/package/maquette-gltf)
[![Live demo](https://img.shields.io/badge/demo-live-4f46e5)](https://bernsteining.github.io/maquette/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

maquette-gltf renders **glTF 2.0** assets (`.glb` / `.gltf`) directly inside your Typst documents — full Cook-Torrance PBR, image-based lighting, animation, skinning and 20+ KHR/EXT extensions — as a single WebAssembly module. 

Part of the [maquette](https://github.com/bernsteining/maquette) family (sibling to the STL/OBJ/PLY and OpenSCAD plugins), sharing the same render core.

**[Try it live →](https://bernsteining.github.io/maquette/)** — load a glTF, orbit, scrub the animation, tweak every setting, and copy the generated Typst snippet.

## Usage

The same [Little Tokyo](https://sketchfab.com/3d-models/little-tokyo-diorama-6072d34e02454743a4a0f4d219c2c62c) diorama, twice: `time` scrubs the animation clip, and `camera` / `center` move the eye.

<table>
<tr><th align="left">Code</th><th>Render</th></tr>
<tr>
<td>

```typst
#import "@preview/maquette-gltf:0.1.0": render-gltf

// `time` scrubs the glTF animation (seconds).
#render-gltf(read("tokyo.glb", encoding: none), (
  camera: (290, 464, 774),
  center: (-86, 5, -25),
  up: (0, 1, 0),
  fov: 40,
  time: 6.0,
  background: none,
))
```

</td>
<td><a href="https://bernsteining.github.io/maquette/?model=tokyo.glb&camera=%5B290%2C464%2C774%5D&center=%5B-86%2C5%2C-25%5D&up=%5B0%2C1%2C0%5D&fov=40&time=6&background=%22none%22" title="Open in the live demo"><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/gltf-tokyo.png" width="340" alt="Little Tokyo diorama, high three-quarter view" /></a></td>
</tr>
<tr>
<td>

```typst
// Same scene — move the camera and aim.
#render-gltf(read("tokyo.glb", encoding: none), (
  camera: (350, 200, 620),
  center: (-86, 5, -25),
  up: (0, 1, 0),
  fov: 45,
  time: 2.0,
  background: none,
))
```

</td>
<td><a href="https://bernsteining.github.io/maquette/?model=tokyo.glb&camera=%5B350%2C200%2C620%5D&center=%5B-86%2C5%2C-25%5D&up=%5B0%2C1%2C0%5D&fov=45&time=2&background=%22none%22" title="Open in the live demo"><img src="https://raw.githubusercontent.com/bernsteining/maquette/master/examples/readme/gltf-tokyo-camera.png" width="340" alt="Little Tokyo diorama from a lower, closer camera" /></a></td>
</tr>
</table>

`get-gltf-info(read("tokyo.glb", encoding: none))` returns the triangle count, bounding box and `max_animation_time`: handy for framing the camera or bounding an animation slider.

## Documentation

[docs/maquette-gltf-documentation.pdf](https://github.com/bernsteining/maquette/blob/master/docs/maquette-gltf-documentation.pdf) walks through the config (camera, IBL, shadows, ground plane, tone mapping, SSAO/FXAA/SSAA) and the full list of supported extensions and texture formats.

## Building from source

```sh
make gltf-build   # cargo build → wasm-opt -O3 → install into the local Typst package dir
```
