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

*maquette-gltf* renders `.glb` and `.gltf` assets as images inside a Typst document. Two entry points:

- *`render-gltf(model, ..options)`* — the render call. Returns a Typst `image`.
- *`get-gltf-info(model, ..options)`* — returns a metadata dict (bbox, triangle count, animation length) without rendering.

Camera framing, background, shadows, ground plane, anti-aliasing and tone mapping are inherited unchanged from #link("maquette-documentation.pdf")[`maquette`]. This document only covers what `render-gltf` adds on top of that base — see the maquette manual for the shared options.

#pagebreak(weak: true)

= Where to find sample glTF assets

The examples below use models from Khronos's sample set (Damaged Helmet, Fox, CesiumMan, ToyCar). Any `.glb` or `.gltf` should work — compression, quantization, and animations are handled transparently.

- #link("https://github.com/KhronosGroup/glTF-Sample-Assets")[KhronosGroup/glTF-Sample-Assets] — the reference sample set. Every extension has a dedicated test asset.
- #link("https://polyhaven.com/models")[Poly Haven Models] — CC0, glTF-native, authored PBR materials.
- #link("https://sketchfab.com/3d-models?features=downloadable")[Sketchfab] (downloadable filter) — largest general library.

For the IBL section, HDR environment maps come from #link("https://polyhaven.com/hdris")[Poly Haven HDRIs] (CC0). Bundle one with your `.typ` and pass the bytes as `ibl: (hdr: ...)`.

= Quickstart

Two input shapes, one entry point.

*Self-contained `.glb` or fully-embedded `.gltf`.* Pass the bytes. This covers most Sketchfab downloads, Damaged Helmet, boombox, and so on.

#{
  align(center)[
    #raw(block: true, lang: "typ", "#import \"@preview/maquette-gltf:0.1.0\": render-gltf\n\n#render-gltf(read(\"helmet.glb\", encoding: none))")
    #v(0.5em)
    #render-gltf(helmet, read: R, width: 40%)
  ]
}

*Split `.gltf`.* The JSON references external `.bin` and textures by relative URI. Pass the path string and an inline `read:` lambda; the wrapper walks the JSON, resolves every URI through your lambda, and packs the results into a sidecar bundle for the plugin.

#{
  align(center)[
    #raw(block: true, lang: "typ", "#render-gltf(\"Duck.gltf\", read: p => read(p, encoding: none))")
    #v(0.5em)
    #render-gltf(duck-split, read: R, width: 30%)
  ]
}

The `read:` argument must be an inline lambda, not a bare `read` reference. Typst resolves `read()` paths against the file the call textually lives in; a bare reference from the package binds to the package's own directory. `p => read(p, ...)` gives the wrapper a filesystem handle rooted at your `.typ`.

#pagebreak(weak: true)

= Shared rendering config

`camera`, `center`, `up`, `fov`, `azimuth`, `elevation`, `background`, `shadows`, `ground`, `ssao`, `antialias`, and `tone_mapping` all work exactly as they do on `render-obj` / `render-stl` / `render-ply`. See the #link("maquette-documentation.pdf")[maquette manual] for the full option surface.

One glTF-only option: *`camera_auto_use`*. If the asset ships an authored camera, `render-gltf` uses it by default. Pass `camera_auto_use: false` to force your own framing arguments instead.

```example
#render-gltf(helmet, read: R,
  camera_auto_use: false,
  camera: (0, 0, 3), up: (0, 1, 0), fov: 60,
)
```

#pagebreak(weak: true)

= Image-Based Lighting

`ibl:` enables environment lighting. Without an `hdr` payload it uses a procedural sky/ground gradient.

== Procedural IBL

```example
#render-gltf(helmet, read: R,
  camera: (2.5, 1.5, 2.5), up: (0, 1, 0), fov: 40,
  background: "#181820",
  ibl: (intensity: 1.2),
)
```

== HDR environment

Pass Radiance `.hdr` file bytes as `ibl.hdr`.

```typ
#render-gltf(helmet, read: R,
  ibl: (intensity: 1.0, hdr: read("studio.hdr", encoding: none)),
)
```

== IBL rotation

`rotation` (radians) spins the environment around the up axis. Use it to align the environment's light direction to your camera composition.

```example
#render-gltf(helmet, read: R,
  camera: (2.5, 1.5, 2.5), up: (0, 1, 0), fov: 40,
  background: "#181820",
  ibl: (intensity: 1.2, rotation: 1.2),
)
```

#pagebreak(weak: true)

= Animations

`time:` (seconds) picks a sample point along the asset's animation. `get-gltf-info` reports `max_animation_time` so you can bound a scrub slider to the clip's real length.

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

When the asset ships multiple clips (idle / walk / run / …), pick one with `animation_index`.

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

`material_variant:` (0-indexed) picks one of the alternate material sets an asset may ship.

```example
#render-gltf(toycar,
  camera: (0.4, 0.3, 0.55), center: (0, 0.05, 0), up: (0, 1, 0), fov: 30,
  background: "#181820",
  ibl: (intensity: 1.4),
  material_variant: 0,
)
```

= Diffuse transmission

Assets using `KHR_materials_diffuse_transmission` render with the extension's back-lit contribution.

```example
#render-gltf(dt-test,
  camera: (1.5, 0.5, 1.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#222",
  ibl: (intensity: 1.0),
  width: 90%,
)
```

#pagebreak(weak: true)

= Compressed and quantized assets

These are decoded transparently — no options to set. Renders identically to an uncompressed version of the same asset.

```example
#render-gltf(box-draco, read: R,
  camera: (2, 1, 2), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#333",
  width: 30%,
)
```

#pagebreak(weak: true)

= Texture formats

PNG, JPEG, and WebP textures are decoded natively. KTX2 (Basis Universal) and AVIF fall back to a plain white texture — geometry renders correctly, but the material's albedo is lost. If your asset targets KTX2, pre-transcode with #link("https://gltf-transform.dev")[`gltf-transform`] to WebP or PNG.

```example
#render-gltf(duck-ktx, read: R,
  camera: (2.5, 1.5, 2.5), center: (0, 0, 0), up: (0, 1, 0), fov: 40,
  background: "#333",
  width: 40%,
)
```

#pagebreak(weak: true)

= Metadata

`get-gltf-info` returns a dict with the asset's bounding box, triangle count, and animation length — enough to drive an auto-frame calculation or a scrub slider without going through the render path.

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

= Asset compatibility

If your asset renders in a mainstream glTF viewer, it should render here. Textures use PNG / JPEG / WebP; KTX2 and AVIF fall back to a white texture (see the previous section). See `crates/maquette-gltf/README.md` for the full per-feature compliance matrix.
