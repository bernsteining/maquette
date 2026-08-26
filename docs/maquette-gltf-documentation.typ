// Uses the local-namespace install (set up by `make gltf-build`) so the
// wrapper's `plugin("maquette-gltf.wasm")` finds the freshly-built wasm
// next to it. Falls back to the published `@preview/maquette-gltf:0.1.0`
// once released.
#import "@local/maquette-gltf:0.1.0": render-gltf, get-gltf-info

#import "@preview/zebraw:0.6.1": *

#set page(margin: 1.5em, footer: context grid(
  columns: (1fr, 1fr),
  align(left, text(size: 7.5pt, fill: luma(120))[maquette-gltf Documentation]),
  align(right, text(size: 7.5pt, fill: luma(120), counter(page).display())),
))
#set par(justify: true)
#show: zebraw.with(lang: false, numbering: false)

// The plugin wrapper needs a `read:` lambda for split-glTF paths (Typst
// packages can't reach the caller's project directly — see the wrapper
// header). Bind it once here and reuse across every example.
#let R = p => read(p, encoding: none)

// Model bindings. `.glb` files are self-contained → pre-read as bytes at doc
// scope (no `read:` lambda needed downstream). Split `.gltf` files reference
// external `.bin`/textures by URI, so those stay as path strings and every
// call site pairs them with `read: R`.
#let helmet     = read("/examples/data/gltf/helmet.blg", encoding: none)
#let boombox    = read("/examples/data/gltf/boombox.glb", encoding: none)
#let fox        = read("/examples/data/gltf/fox.glb", encoding: none)
#let toycar     = read("/examples/data/gltf/toycar.glb", encoding: none)
#let cesiumman  = read("/examples/data/gltf/cesiumman.glb", encoding: none)
#let dt-test    = read("/examples/data/gltf/DiffuseTransmissionTest/DiffuseTransmissionTest.glb", encoding: none)
#let duck-split = "/examples/data/gltf/Duck/Duck.gltf"
#let duck-quant = "/examples/data/gltf/Duck-quantized/Duck.gltf"
#let duck-webp  = "/examples/data/gltf/Duck-webp/Duck.gltf"
#let duck-ktx   = "/examples/data/gltf/Duck-ktx2/Duck.gltf"
#let box-draco  = "/examples/data/gltf/Box-draco/Box.gltf"

// Every example block is `#render-gltf(...)` — pass models by path (with `read: R`
// for splits) or by bytes for `.glb`. Scope used by the `example` show rule below.
#let doc-scope = (
  render-gltf: render-gltf, get-gltf-info: get-gltf-info, R: R,
  helmet: helmet, boombox: boombox, fox: fox, toycar: toycar, cesiumman: cesiumman,
  duck-split: duck-split, duck-quant: duck-quant, duck-webp: duck-webp, duck-ktx: duck-ktx,
  box-draco: box-draco, dt-test: dt-test,
)

// Strip the imports + doc-side `let`s from example source so the eval'd version
// picks up bindings from `doc-scope` (same trick as maquette-documentation.typ).
#let filter-eval(text) = text.split("\n").filter(l =>
  not l.starts-with("#import") and not l.starts-with("#let R ")
).join("\n")

// Highlight-line + column-ratio parser, same protocol as the main doc:
//   // hl: 3, 5-7   → highlight lines 3, 5, 6, 7
//   // cols: 2 1    → code:render column widths (default 1 1)
#let parse-hl(text) = {
  let lines = text.split("\n")
  let hl = ()
  let cols = (1, 1)
  let code = ()
  for line in lines {
    let t = line.trim()
    if t.starts-with("// hl:") {
      for part in t.slice(6).trim().split(",") {
        let p = part.trim()
        if p.contains("-") {
          let b = p.split("-")
          for n in range(int(b.at(0)), int(b.at(1)) + 1) { hl.push(n) }
        } else if p != "" { hl.push(int(p)) }
      }
    } else if t.starts-with("// cols:") {
      let s = t.slice(8).trim().split(" ")
      cols = (int(s.at(0)), int(s.at(1)))
    } else { code.push(line) }
  }
  (code.join("\n"), hl, cols)
}

#show raw.where(lang: "example"): it => {
  let (code, hl, cols) = parse-hl(it.text)
  let eval-text = filter-eval(code)
  grid(columns: (cols.at(0) * 1fr, cols.at(1) * 1fr), gutter: 1em,
    zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code)),
    align(center + horizon, eval(eval-text, mode: "markup", scope: doc-scope)),
  )
}

#show raw.where(lang: "examplev"): it => {
  let (code, hl, ..) = parse-hl(it.text)
  let eval-text = filter-eval(code)
  zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code))
  align(center, eval(eval-text, mode: "markup", scope: doc-scope))
}

#v(1fr)
#align(center)[
  #text(font: "Libertinus Serif", size: 32pt, weight: "bold")[maquette-gltf]
  #v(0.3em)
  #text(size: 14pt, fill: gray)[Render glTF 2.0 assets in Typst]
  #v(1.5em)
  #text(size: 12pt, blue)[
    #link("https://github.com/bernsteining/maquette")[github.com/bernsteining/maquette] · #link("https://bernsteining.github.io/maquette")[bernsteining.github.io/maquette]
  ]
  #render-gltf(helmet, read: R, width: 55%, camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
    background: "#181820",
    ground: (color: "#282838", size_scale: 3.0, roughness: 0.9),
    ssao: (samples: 16, radius: 0.4, strength: 1.0),
  )
  #v(0.4em)
  #text(size: 10pt, fill: luma(150))[Version #toml("/crates/maquette-gltf/maquette-gltf/typst.toml").package.version #h(0.4em)·#h(0.4em) #datetime.today().display("[month repr:long] [day], [year]")]
]
#v(1fr)

#pagebreak(weak: true)

#{
  align(center, text(size: 20pt, weight: "bold", tracking: 2pt)[CONTENTS])
  v(1em)
  set text(size: 9pt)
  set outline.entry(fill: repeat(text(fill: luma(180))[.#h(2pt)]))
  show outline.entry.where(level: 1): it => {
    v(0.4em); strong(it)
  }
  columns(2, gutter: 2em, outline(indent: 1.2em))
}

#pagebreak(weak: true)

= Introduction

*maquette-gltf* extends #link("maquette-documentation.pdf")[`maquette`] with glTF 2.0 support: `.glb` or `.gltf` in, rendered frame out. The plugin adds a PBR pipeline (Cook-Torrance GGX + Karis split-sum IBL + WBOIT translucency) on top of maquette's shared rasterizer, plus glTF-specific machinery for authored cameras/lights/materials/animations and the ecosystem's compression + texture extensions.

This document only covers what maquette-gltf *adds*. The shared rendering knobs — camera framing, background, shadows, ground plane, SSAO/FXAA/SSAA, tone mapping — are already documented in maquette's manual and behave identically here. `render-gltf` accepts the same option dicts.

#pagebreak(weak: true)

= Quickstart

Two input shapes, both handled transparently.

*`.glb` or fully-embedded `.gltf`* — pass the bytes directly. This is the common case (Damaged Helmet, boombox, most Sketchfab downloads).

#{
  align(center)[
    #raw(block: true, lang: "typ", "#import \"@preview/maquette-gltf:0.1.0\": render-gltf\n\n#render-gltf(read(\"helmet.glb\", encoding: none))")
    #v(0.5em)
    #render-gltf(helmet, read: R, width: 40%)
  ]
}

*Split `.gltf`* — the `.gltf` JSON references external `.bin` + textures by relative URI. Pass the *path string* plus an inline `read:` lambda; the wrapper walks the JSON, discovers every URI, reads each one through your lambda, packs them into a sidecar bundle for the plugin. You write zero filenames.

#{
  align(center)[
    #raw(block: true, lang: "typ", "#render-gltf(\"Duck.gltf\", read: p => read(p, encoding: none))")
    #v(0.5em)
    #render-gltf(duck-split, read: R, width: 30%)
  ]
}

The `read:` lambda has to be an *inline lambda*, not a bare `read` reference. Typst resolves `read()` paths against the source file the call is textually in — a bare reference stays bound to the package's own path context. Wrapping in `p => read(p, ...)` gives the wrapper a filesystem handle scoped to your `.typ`.

#pagebreak(weak: true)

= Shared rendering config

Camera framing (`camera` / `center` / `up` / `fov` / `azimuth` / `elevation`), `background`, `shadows`, `ground`, `ssao`, `antialias`, `tone_mapping` — all inherited from #link("maquette-documentation.pdf")[maquette] and behave the same on `render-gltf`. Don't repeat the tour here; refer to that manual.

One glTF-only knob to note: *`camera_auto_use: false`*. If the loaded asset ships an authored camera, `render-gltf` uses it by default (glTF viewer convention). Set this to `false` to force your Cartesian/spherical arguments to win instead.

```example
#render-gltf(helmet, read: R,
  camera_auto_use: false,
  camera: (0, 0, 3), up: (0, 1, 0), fov: 60,
)
```

#pagebreak(weak: true)

= Image-Based Lighting

The dominant visual lever for PBR: an environment map lights the whole scene. Enabling `ibl` without an `hdr` bytes payload falls back to a procedural hemispheric sky.

== Procedural IBL

```example
#render-gltf(helmet, read: R,
  camera: (2.5, 1.5, 2.5), up: (0, 1, 0), fov: 40,
  background: "#181820",
  ibl: (intensity: 1.2),
)
```

== HDR environment

Pass Radiance `.hdr` bytes as `ibl.hdr`. The prefilter builds a diffuse irradiance map + a specular mip chain up-front (once per HDR, cached).

```typ
#render-gltf(helmet, read: R,
  ibl: (intensity: 1.0, hdr: read("studio.hdr", encoding: none)),
)
```

== IBL rotation

The `rotation` field spins the environment around the up axis (radians). Useful for aligning studio HDRs to your camera composition.

```example
#render-gltf(helmet, read: R,
  camera: (2.5, 1.5, 2.5), up: (0, 1, 0), fov: 40,
  background: "#181820",
  ibl: (intensity: 1.2, rotation: 1.2),
)
```

#pagebreak(weak: true)

= Animations

Assets with animation channels get replayed at `time` seconds. `get-gltf-info` returns `max_animation_time` so you can build a scrub slider bounded to the actual clip length.

```example
// cols: 1 1
#render-gltf(fox,
  camera: (120, 90, 180), center: (0, 40, 0), up: (0, 1, 0), fov: 40,
  background: "#1a1a22",
  ibl: (intensity: 1.3),
  time: 0.5,
  width: 45%,
)
```

When the asset ships multiple clips (idle / walk / run / ...), pick one with `animation_index`. `None` (the default) plays every clip stacked — last-write-wins per node channel — which is almost never what you want for multi-clip assets.

```example
#render-gltf(cesiumman,
  camera: (2.5, 1, 2.5), center: (0, 0.9, 0), up: (0, 1, 0), fov: 30,
  background: "#1a1a22",
  ibl: (intensity: 1.3),
  animation_index: 0,
  time: 0.3,
  width: 40%,
)
```

#pagebreak(weak: true)

= Material variants

`KHR_materials_variants` lets an asset ship multiple material sets. Pick one with `material_variant` (0-indexed).

```example
#render-gltf(toycar,
  camera: (0.4, 0.3, 0.55), center: (0, 0.05, 0), up: (0, 1, 0), fov: 30,
  background: "#181820",
  ibl: (intensity: 1.4),
  material_variant: 0,
)
```

= Diffuse transmission

`KHR_materials_diffuse_transmission` — matte back-lit lambertian. Common on cloth, leaves, paper. The example asset is a factor × color grid; the bottom rows show the green DT color activating as the factor increases.

```example
#render-gltf(dt-test,
  camera: (1.5, 0.5, 1.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#222",
  ibl: (intensity: 1.0),
  width: 90%,
)
```

#pagebreak(weak: true)

= Compression + quantization

The plugin decodes three common wire formats up-front, so downstream traversal never has to care.

== EXT_meshopt_compression

Emitted by `gltfpack` (Meshopt's own tool). Interleaved buffer decompression, transparent.

== KHR_draco_mesh_compression

Google Draco. Decoded via `draco-oxide` (pure Rust). Up to ~20× geometry compression on typical assets, no size penalty at render time.

```example
#render-gltf(box-draco, read: R,
  camera: (2, 1, 2), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#333",
  width: 30%,
)
```

== KHR_mesh_quantization

`gltfpack` also emits quantized POSITION/NORMAL/TANGENT (i8/u8/i16/u16) paired with `KHR_texture_transform` for UV dequantization. Renders pixel-identically to the non-quantized asset — verified on the Duck below.

```example
#render-gltf(duck-quant, read: R,
  camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  width: 40%,
)
```

#pagebreak(weak: true)

= Texture formats

Beyond the base PNG + JPEG:

== EXT_texture_webp

`image-webp` (pure Rust) handles both VP8 and VP8L variants. Second-most-common texture format in production glTF (Shopify AR, IKEA, Blender's default export in 3.4+).

```example
#render-gltf(duck-webp, read: R,
  camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  width: 40%,
)
```

== Unsupported formats

`KHR_texture_basisu` (KTX2 / Basis Universal) and `EXT_texture_avif` aren't decoded — they'd pull in ~1 MB of decoder deps for formats we ship without today. Assets using them render with a *placeholder white texture* per material (geometry stays intact) rather than crashing. Below: the same Duck with KTX2 textures, silhouette-shaded because the base color reads as white:

```example
#render-gltf(duck-ktx, read: R,
  camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#333",
  width: 40%,
)
```

Workaround: pre-transcode via [`gltf-transform`](https://gltf-transform.dev) to WebP or PNG.

#pagebreak(weak: true)

= Metadata

`get-gltf-info` returns a dict — bounding box, triangle count, animation length. Useful for driving auto-framing or a scrub slider without touching the render path.

```example
// cols: 2 1
#let info = get-gltf-info(helmet, read: R)
#{
  raw(block: true, lang: "yml",
    "triangles: " + str(info.triangles) + "\n" +
    "bbox_min:  [" + info.bbox_min.map(x => str(calc.round(x, digits: 2))).join(", ") + "]\n" +
    "bbox_max:  [" + info.bbox_max.map(x => str(calc.round(x, digits: 2))).join(", ") + "]\n" +
    "center:    [" + info.center.map(x => str(calc.round(x, digits: 2))).join(", ") + "]\n" +
    "radius:    " + str(calc.round(info.radius, digits: 2)) + "\n" +
    "max_animation_time: " + str(info.max_animation_time)
  )
}
```

#pagebreak(weak: true)

= What's supported vs. not

*Core spec:* glTF 2.0 JSON + GLB (single-file and split with external `.bin`/textures), meshes (POINTS/LINES/TRIANGLES), skinning (JOINTS_0 + JOINTS_1, WEIGHTS_0 + WEIGHTS_1, up to 8 influences per vertex), morph targets, animations (TRS + morph weights, all three interpolation modes), multiple UV sets (TEXCOORD_0/1/2), vertex colors, cameras (perspective + orthographic), multiple scenes, multiple animations, textures with samplers, sparse accessors, KHR_mesh_quantization.

*Rendering:* Cook-Torrance GGX PBR, IBL from HDR (Radiance / RGBE), shadow maps (PCF + PCSS), alpha modes (OPAQUE / MASK / BLEND via WBOIT), double-sided normal flip, SSAO, FXAA, SSAA ×2/×4.

*Extensions:* `KHR_lights_punctual`, `KHR_materials_unlit`, `KHR_materials_transmission`, `KHR_materials_ior`, `KHR_materials_specular`, `KHR_materials_emissive_strength`, `KHR_materials_volume`, `KHR_materials_clearcoat`, `KHR_materials_sheen`, `KHR_materials_iridescence`, `KHR_materials_anisotropy`, `KHR_materials_dispersion`, `KHR_materials_diffuse_transmission`, `KHR_texture_transform`, `KHR_materials_pbrSpecularGlossiness`, `KHR_materials_variants`, `EXT_texture_webp`, `EXT_meshopt_compression`, `KHR_draco_mesh_compression`.

*Not supported* (assets render with graceful fallback — placeholder white texture or silent skip):

- `KHR_texture_basisu` (KTX2). Biggest gap — dominant production texture format.
- `EXT_texture_avif`. Rare in the wild.
- `KHR_animation_pointer`. Animations targeting material / light / camera properties via JSON pointer are ignored.
- TEXCOORD_N for N ≥ 3 (collapses to slot 2).
- Sparse accessors combined with quantization.

See `crates/maquette-gltf/README.md` for the full compliance matrix and per-extension notes.
