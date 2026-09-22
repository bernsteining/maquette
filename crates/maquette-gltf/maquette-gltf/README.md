# maquette-gltf

[![Typst Universe](https://img.shields.io/badge/Typst%20Universe-maquette--gltf-239dad)](https://typst.app/universe/package/maquette-gltf)
[![Live demo](https://img.shields.io/badge/demo-live-4f46e5)](https://bernsteining.github.io/maquette/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

`maquette-gltf` is a [Typst](https://typst.app) plugin that renders **glTF 2.0** assets (`.gltf` / `.glb`) as raster images directly inside your documents — no screenshots, no external tools.

It reads the model, applies physically based (PBR) shading with optional image-based lighting, shadows, SSAO and tone mapping, and drops the result into your PDF at compile time. It is a companion to [`maquette`](https://typst.app/universe/package/maquette) (STL / OBJ / PLY) and [`maquette-scad`](https://typst.app/universe/package/maquette-scad) (OpenSCAD), and shares the same rendering core.

Everything runs as a single WASM plugin.

**[Try it live in your browser →](https://bernsteining.github.io/maquette/)** drag to orbit, tweak every setting, and copy the generated Typst code.

See the [documentation](https://github.com/bernsteining/maquette/blob/master/docs/maquette-gltf-documentation.pdf) for examples of every feature.

## Example

```typst
#import "@preview/maquette-gltf:0.1.0": render-gltf

// A self-contained .glb: pass its bytes directly.
#render-gltf(read("assets/helmet.glb", encoding: none),
  azimuth: 30,
  elevation: 15,
  ibl: (environment: "studio", intensity: 1.2),
  tone_mapping: "aces",
  width: 480)
```

For a split `.gltf` (with external `.bin` / textures), give the path plus a `read:` lambda and the wrapper discovers and reads the sidecars for you:

```typst
#render-gltf("assets/scene.gltf", read: p => read(p, encoding: none), azimuth: 40)
```

## Functions

### `render-gltf`

```typst
#render-gltf(model, read: none, ..config, width: auto, height: auto)
```

Renders a glTF/GLB asset to a raster image. `model` is either the **bytes** of a self-contained `.glb` (`read("m.glb", encoding: none)`) or a **path** to a `.gltf`, in which case you must also pass `read: p => read(p, encoding: none)` so the wrapper can read the external `.bin` and image sidecars. Configuration is passed as named arguments (or a single positional dictionary). Recognised keys include `camera`, `lights`, `ibl`, `shadows`, `ssao`, `fxaa`, `tone_mapping`, `background`, `ground`, `variant`, and `time` (for animated models).

### `get-gltf-info`

```typst
#let info = get-gltf-info(model, read: none)
```

Returns scene metadata (triangle count, bounding box, center, radius, `max_animation_time`) as a dictionary — handy for computing camera framing or driving an animation slider.

## Output

Raster **PNG** only: glTF's PBR materials and textures are sampled per pixel. Set `width` / `height` for resolution and `fxaa` for antialiasing.
