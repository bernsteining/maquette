#import "@local/maquette-gltf:0.1.0": render-gltf, get-gltf-info
#import "common.typ": *

#let fox   = read("/examples/data/gltf/fox.glb", encoding: none)
#let tokyo = read("/examples/data/gltf/tokyo.glb", encoding: none)
#let (r-gltf, i-gltf) = (render-gltf, get-gltf-info)
#let render-gltf = wrap-render(r-gltf, paths: ("gltf/fox/Fox.gltf": "fox.glb"))
#let get-gltf-info = wrap-info(i-gltf)
#let fox = dm("fox.glb", fox)
#let tokyo = dm("tokyo.glb", tokyo)
#let studio = (background: none, ibl: (intensity: 1.2), ssao: true, zoom: 1.5)
#let side = (width: 600, height: 300, azimuth: 90, elevation: 8)

#let scope = (render-gltf: render-gltf, get-gltf-info: get-gltf-info, studio: studio, side: side, fox: fox, tokyo: tokyo)
#let sect(body) = block(breakable: false, width: 100%, body)

#show: doc.with(name: "maquette-gltf", scope: scope, stacked: true)

#cover(
  [maquette-gltf],
  [Render glTF 2.0 assets in Typst],
  box(clip: true, radius: 44pt, image("logo-gltf.svg", width: 260pt)),
  (
    ("Source", "https://github.com/bernsteining/maquette"),
    ("Typst Universe", "https://typst.app/universe/package/maquette-gltf"),
    ("Live demo", "https://bernsteining.github.io/maquette/?model=tokyo.glb"),
  ),
  toml("/crates/maquette-gltf/maquette-gltf/typst.toml").package.version,
)

#contents()

= Introduction

*maquette-gltf* renders glTF 2.0 files (`.glb`, `.gltf`) as images in your Typst document, with nothing to install.

#demo-callout("https://bernsteining.github.io/maquette/?model=tokyo.glb",
  [Load a glTF, orbit it, play its animation, adjust every option in this manual, and copy the Typst code.])

It provides two functions:

- *`render-gltf(model, ..options)`* renders the model as an image.
- *`get-gltf-info(model)`* returns its size, triangle count and animation length.

The examples use two models, the #link("https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox")[Fox] and the #link("https://www.artstation.com/artwork/1AGwX")[Littlest Tokyo]; credits are at the end.

= Quickstart

#sect[
Read the model as bytes and pass it to `render-gltf` (left). The examples below reuse two settings: `studio`, a lighting look that fills the frame (right), and `side`, a wide side view for the Fox (below).

#grid(columns: 2, gutter: 1em,
```example
#import "@preview/maquette-gltf:0.1.0": render-gltf

#let tokyo = read("tokyo.glb", encoding: none)
#render-gltf(tokyo)
```,
```example
// hl: 1-2
#let studio = (background: none, ssao: true,
  ibl: (intensity: 1.2), zoom: 1.5)
#render-gltf(tokyo, ..studio)
```,
)
]

#sect[
```example
// hl: 1-2
#let side = (width: 600, height: 300,
  azimuth: 90, elevation: 8)
#render-gltf(fox, side, ..studio,
  zoom: 2.3, animation_index: 2)
```
]

= Config reference

#sect[
All optional; defaults shown.

#text(size: 8.5pt, raw(block: true, lang: "json", "{ // ── Viewport ──────────────────────────────────────────────────────
  \"width\": 500,                    // Output width in pixels
  \"height\": 500,                   // Output height in pixels
  \"background\": \"#f0f0f0\",         // Hex colour; none, \"none\" or \"\" = transparent
  // ── Camera ────────────────────────────────────────────────────────
  \"camera\": null,                  // Eye position (x, y, z); null = orbit around the model
  \"center\": [0, 0, 0],             // Look-at target (overridden by auto_center)
  \"up\": [0, 1, 0],                 // Up direction (glTF is Y-up)
  \"azimuth\": 30,                   // Orbit horizontal angle, degrees
  \"elevation\": 20,                 // Orbit vertical angle, degrees
  \"distance\": null,                // Orbit distance; null = fit the model
  \"zoom\": 1.0,                     // Move closer than the automatic framing
  \"fov\": 45,                       // Vertical field of view, degrees
  \"auto_center\": true,             // Aim at the scene's bounding-box centre
  \"auto_fit\": true,                // Frame the scene to fill the image
  \"camera_auto_use\": true,         // Use the asset's first camera when it has one
  \"camera_index\": null,            // Use an authored camera by index
  \"camera_name\": null,             // Use an authored camera by name
  \"scene_index\": null,             // Scene to render; null = the default scene
  // ── Lighting ──────────────────────────────────────────────────────
  \"light_dir\": [1, 2, 3],          // Key light direction when the asset has no lights
  \"ambient\": 0.2,                  // Constant ambient (0-1); unused with ibl
  \"ibl\": null,                     // true, or {intensity, sky, ground, rotation, hdr}
  \"shadows\": null,                 // true, or {resolution, softness, light_size, bias, normal_bias, slope_bias}
  \"ground\": null,                  // true, or {color, roughness, size_scale, y}
  \"cull_backface\": true,           // Skip back faces of single-sided materials
  // ── Post-processing ───────────────────────────────────────────────
  \"ssao\": null,                    // true, or {samples, radius, bias, strength, space}
  \"antialias\": 4,                  // 0 off, 1 FXAA, 2-3 SSAA x2/x4, 4-5 FXAA + SSAA x2/x4
  \"tone_mapping\": \"\",              // \"\", \"reinhard\" or \"aces\"
  \"exposure\": 1.0,                 // Light multiplier before tone mapping
  // ── Animation and materials ───────────────────────────────────────
  \"time\": 0.0,                     // Animation time, seconds
  \"animation_index\": null,         // Animation to use (from 0); null = all together
  \"material_variant\": 0,           // KHR_materials_variants index
  // ── Textures ──────────────────────────────────────────────────────
  \"texture_max_size\": null,        // Downsample textures to this side, pixels
  \"no_textures\": false             // Skip textures (fast drafts)
}"))
]

= Loading assets

#sect[
- *`.glb` files*: read with `encoding: none` and pass the bytes, as above.
- *Split `.gltf` files*: pass the path and a `read:` function, which loads the `.bin` and texture files next to it.

```example
// hl: 1-2
#render-gltf("gltf/fox/Fox.gltf",
  read: p => read(p, encoding: none),
  side, ..studio, zoom: 2.3)
```
]

= Camera

#sect[
== Orbit

`azimuth` turns the camera around the model and `elevation` raises it, in degrees.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  azimuth: 120, elevation: 15)
```,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  azimuth: -40, elevation: 50)
```,
)
]

#sect[
== Zoom and lens

`zoom` moves closer than the automatic framing. `fov` sets the lens angle in degrees: narrow flattens perspective, wide exaggerates it.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  zoom: 2.6)
```,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  fov: 100, zoom: 2.2)
```,
)
]

#sect[
== Explicit camera

`camera`, `center` and `up` place the camera in scene coordinates; turn off `auto_center` and `auto_fit` to use them as given. Models with their own cameras use the first one; pick another with `camera_index`, or ignore them with `camera_auto_use: false`.

```example
// hl: 2-5
#render-gltf(tokyo, (width: 600, height: 280), ..studio,
  camera: (-60, 120, 640),
  center: (-86, 5, -25),
  up: (0, 0.989, -0.145),
  auto_center: false, auto_fit: false)
```
]

= Lighting

#sect[
== Key light and ambient

Without `ibl`, `light_dir` points the main light and `ambient` adds a flat fill from 0 to 1.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, zoom: 1.5,
  light_dir: (-3, 2, 1), ambient: 0.1)
```,
```example
// hl: 2
#render-gltf(tokyo, zoom: 1.5,
  light_dir: (3, 1, 2), ambient: 0.6)
```,
)
]

#sect[
== Image-based lighting

`ibl` lights the model from a surrounding environment, which suits physically based materials.

- `intensity`, `sky` and `ground` colours, and `rotation` (radians) tune the built-in sky.
- `hdr: read("studio.hdr", encoding: none)` uses a real environment map, such as the free ones on #link("https://polyhaven.com/hdris")[Poly Haven].

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  ibl: (intensity: 0.7))
```,
```example
// hl: 2-3
#render-gltf(tokyo, ..studio,
  ibl: (sky: "#ffb070", ground: "#203050",
    rotation: 2, intensity: 1.4))
```,
)
]

= Shadows and ground

#sect[
- *`shadows`*: `true`, or set `softness`, `resolution` (default 512) and `light_size`.
- *`ground`*: a floor that catches them; set its `color`, `roughness` and `size_scale`.

```example
// hl: 3-4
#render-gltf(fox, side, ..studio, zoom: 2,
  elevation: 30, azimuth: 70,
  shadows: (softness: 3),
  ground: (color: "#c9ced8", size_scale: 8))
```
]

= Post-processing

#sect[
== Ambient occlusion

`ssao` darkens creases and contact areas, adding depth; `strength` sets how dark.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  ssao: false)
```,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  ssao: (strength: 2.5))
```,
)
]

#sect[
== Tone mapping and exposure

Bright light clips to white. `tone_mapping` (`"reinhard"` or `"aces"`) compresses it smoothly; `exposure` brightens or darkens.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  ibl: (intensity: 3), tone_mapping: "")
```,
```example
// hl: 2-3
#render-gltf(tokyo, ..studio,
  ibl: (intensity: 3), tone_mapping: "aces",
  exposure: 1.2)
```,
)
]

#sect[
== Anti-aliasing

`antialias` smooths edges, from 0 (off) to 5 (best); the default is 4.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(fox, side, ..studio,
  antialias: 0, zoom: 5)
```,
```example
// hl: 2
#render-gltf(fox, side, ..studio,
  antialias: 5, zoom: 5)
```,
)
]

= Animation

#sect[
A glTF file can store animations: the Fox's file holds three, the Fox looking around, walking and running. A render is a still image, so two options choose what it shows:

- *`animation_index`*: which animation to use, counted from 0. Without it, all of the file's animations are applied together.
- *`time`*: which moment of that animation to show, in seconds from its start.

```example
// hl: 2
#render-gltf(fox, side, ..studio, zoom: 2.3,
  animation_index: 1, time: 0.4)
```

Stepping `time` gives the frames of the run:

```examplev
// hl: 3
#grid(columns: 2, gutter: 0.6em, ..range(4).map(i =>
  render-gltf(fox, side, ..studio, zoom: 2.3,
    animation_index: 2, time: i * 0.18)))
```
]

= Textures

#sect[
For faster drafts, `texture_max_size` shrinks textures and `no_textures: true` skips them.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  texture_max_size: 16)
```,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  no_textures: true)
```,
)
]

= Output

#sect[
`width` and `height` in a dictionary set the image size in pixels; as named arguments they set its size on the page. `background` sets a colour, or `none` for transparent.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-gltf(tokyo, ..studio, zoom: 2.4,
  (width: 600, height: 300), width: 100%)
```,
```example
// hl: 2
#render-gltf(tokyo, ..studio,
  background: "#16181f")
```,
)
]

= Metadata

#sect[
`get-gltf-info` returns the model's metrics without rendering it.

```example
#let info = get-gltf-info(fox)
#let fmt(v) = if type(v) == array {
  v.map(fmt).join(", ")
} else { str(calc.round(v, digits: 1)) }
#table(columns: 2, ..info.pairs()
  .map(((k, v)) => (raw(k), fmt(v)))
  .flatten())
```
]

= Model credits

#sect[
#let src(url, body) = link(url, text(fill: blue, body))
#ref-table(([Model], [Authors], [Licence]), (
  (src("https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox")[Fox],
    [PixelMannen (model), tomkranis (rigging and animation), AsoboStudio and scurest (glTF)],
    [#src("https://creativecommons.org/publicdomain/zero/1.0/")[CC0 1.0], #src("https://creativecommons.org/licenses/by/4.0/")[CC BY 4.0]]),
  (src("https://www.artstation.com/artwork/1AGwX")[Littlest Tokyo], [Glen Fox],
    src("https://creativecommons.org/licenses/by/4.0/")[CC BY 4.0]),
))
]
