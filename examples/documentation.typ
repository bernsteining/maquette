#import "../maquette/maquette.typ": render-stl, render-obj, render-ply, get-stl-info, get-obj-info, get-ply-info

#import "@preview/zebraw:0.6.1": *

#set page(margin: 1.5em)
#set par(justify: true)
#show: zebraw.with(lang: false, numbering: false)

#let bunny = read("data/bunny.obj")
#let cube = read("data/cube.stl", encoding: none)
#let colored = read("data/colored_cube.stl", encoding: none)
#let obj-cube = read("data/cube.obj")
#let teapot = read("data/teapot.obj")
#let crankshaft = read("data/crankshaft.obj")
#let skull-brain = read("data/brain_skull.obj")
#let rubi_blender = read("data/rubi_blender.ply", encoding: none)
#let rubi_scan = read("data/rubi_scan.ply", encoding: none)

#let doc-scope = (
  render-stl: render-stl, render-obj: render-obj, render-ply: render-ply,
  get-stl-info: get-stl-info, get-obj-info: get-obj-info, get-ply-info: get-ply-info,
  cube: cube, colored: colored,
  obj-cube: obj-cube, teapot: teapot, crankshaft: crankshaft, bunny:bunny, skull-brain: skull-brain, rubi_blender: rubi_blender, rubi_scan: rubi_scan
)
#let filter-eval(text) = text.split("\n").filter(l =>
  not l.starts-with("#import") and not (l.starts-with("#let ") and l.contains("read("))
).join("\n")

#let parse-hl(text) = {
  let lines = text.split("\n")
  let hl = ()
  let code = ()
  for line in lines {
    if line.trim().starts-with("// hl:") {
      let spec = line.trim().slice(6).trim()
      for part in spec.split(",") {
        let p = part.trim()
        if p.contains("-") {
          let bounds = p.split("-")
          for n in range(int(bounds.at(0).trim()), int(bounds.at(1).trim()) + 1) { hl.push(n) }
        } else if p.len() > 0 { hl.push(int(p)) }
      }
    } else { code.push(line) }
  }
  (code.join("\n"), hl)
}

#show raw.where(lang: "example"): it => {
  let (code, hl) = parse-hl(it.text)
  let eval-text = filter-eval(code)
  grid(columns: (1fr, 1fr), gutter: 1em,
    zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code)),
    align(center + horizon, eval(eval-text, mode: "markup", scope: doc-scope)),
  )
}

#show raw.where(lang: "examplev"): it => {
  let (code, hl) = parse-hl(it.text)
  let eval-text = filter-eval(code)
  zebraw(lang: false, numbering: false, highlight-lines: hl, raw(block: true, lang: "typst", code))
  align(center, eval(eval-text, mode: "markup", scope: doc-scope))
}

#show raw.where(lang: "obj"): it => {
  render-obj(bytes(it.text), width: 50%)
}

#v(1fr)
#align(center)[
  #text(font: "Libertinus Serif", size: 32pt, weight: "bold")[Maquette]
  #v(0.3em)
  #text(size: 14pt, fill: gray)[Render 3D models in Typst]
  #v(1.5em)
  #text(size:12pt, blue)[#link("https://github.com/bernsteining/maquette")[github.com/bernsteining/maquette ] · #link("https://typst.app/universe/package/maquette")[typst.app/universe/package/maquette]]
  #render-obj(teapot, (
    camera: (0, 2, 5),
    up: (0, 1, 0),
    color: "#e8d0b0",
    background: "#ffffff",
    tone_mapping: "aces",
    specular: 0.2,
    width: 1000,
    height: 800,
    antialias: 4,
  ), width: 75%)
]
#v(1fr)

#pagebreak()

#{
  align(center, text(size: 20pt, weight: "bold", tracking: 2pt)[CONTENTS])
  v(1em)
  set text(size: 9pt)
  set outline.entry(fill: repeat(text(fill: luma(180))[.#h(2pt)]))
  show outline.entry.where(level: 1): it => {
    if it.element.has("label") and str(it.element.label) == "color-mapping" {
      colbreak()
    }
    v(0.4em)
    strong(it)
  }
  columns(2, gutter: 2em, outline(indent: 1.2em))
}

#pagebreak()

= Introduction

Maquette is a Typst plugin for rendering 3D models directly inside your documents. It loads STL, OBJ, and PLY files and produces publication-ready images — no external renderer, no screenshots, no manual exporting. Everything runs with WASM.

Under the hood, Maquette is a small rasterizer with a real lighting pipeline: multi-light Blinn-Phong shading, Fresnel reflections, subsurface scattering, ambient occlusion (SSAO), bloom, tone mapping, and more. Models can be rendered to PNG (rasterized, constant-size output) or SVG (scalable vector polygons). The full configuration — camera, lights, materials, post-processing — lives in your `.typ` source, so every view is reproducible and version-controllable.

= Quickstart

Import a render function, read a model file, and call it. That's it.
```example
#import "@preview/maquette:0.1.0": render-obj

#let cube = read("data/cube.obj")
#render-stl(cube)
```

The `width` and `height` arguments are forwarded to Typst's `image()` for display sizing. 

The default output is PNG; pass `format: "svg"` for vector output: 

```typst
#render-stl(cube, width: 80%, format: "svg")
```

== Inline Show Rule

With a show rule, you can write OBJ / STL / PLY geometry directly in fenced code blocks and have it rendered inline:

#raw(block: true, lang: "typst",
"#show raw.where(lang: \"obj\"): it => {\n  render-obj(bytes(it.text))\n}"
)

#let pyramid-obj = "v  0.0  1.0  0.0\nv -0.5  0.0 -0.5\nv  0.5  0.0 -0.5\nv  0.5  0.0  0.5\nv -0.5  0.0  0.5\nf 1 3 2\nf 1 4 3\nf 1 5 4\nf 1 2 5\nf 2 3 4 5"

#grid(columns: (1fr, 1fr), gutter: 1em,
  raw(block: true, lang: "typst", "```obj\n" + pyramid-obj + "\n```"),
  align(center + horizon, render-obj(bytes(pyramid-obj), width: 58%)),
)

Now we might want to customize all our rendering pipeline, so let's see how we can configure that.

#pagebreak()

= Config Reference

All parameters are optional and can be passed as a dictionary. Defaults are shown below.

#text(size: 10.05pt, raw(block: true, lang: "json", "{ // ── Camera & Viewport ─────────────────────────────────────────────
  \"camera\": [3, 3, 3],                             // Camera position in world space (Cartesian)
  \"azimuth\": null,                                 // Spherical camera: horizontal angle in degrees
  \"elevation\": null,                               // Spherical camera: vertical angle in degrees
  \"distance\": null,                                // Spherical camera: distance from center (auto)
  \"center\": [0, 0, 0],                             // Look-at target (overridden by auto_center)
  \"up\": [0, 0, 1],                                 // Up direction vector
  \"fov\": 45,                                       // Vertical FOV in degrees (perspective only)
  \"projection\": \"perspective\",                     // \"perspective\", \"orthographic\", \"isometric\" ...
  \"auto_center\": true,                             // Auto-center on model bounding box
  \"auto_fit\": true,                                // Scale model to fill viewport
  \"width\": 500,                                    // Output width in pixels
  \"height\": 500,                                   // Output height in pixels
  \"background\": \"#f0f0f0\",                         // Background color (hex), \"none\" or \"\" = transparent
  // ── Appearance ────────────────────────────────────────────────────
  \"color\": \"#4488cc\",                              // Model fill color (hex)
  \"stroke\": {\"color\": \"none\", \"width\": 0},         // Triangle edge stroke (or just \"#hex\")
  \"light_dir\": [1, 2, 3],                          // Directional light vector
  \"ambient\": 0.15,                                 // Ambient light intensity (0-1)
  \"mode\": \"solid\",                                 // \"solid\", \"wireframe\", \"solid+wireframe\", \"x-ray\"
  \"xray_opacity\": 0.1,                             // Front-face opacity for x-ray mode (0-1)
  \"cull_backface\": true,                           // Back-face culling (auto-disabled for x-ray)
  \"wireframe\": {\"color\": \"\", \"width\": 1.0},        // Wireframe edges (or just \"#hex\")
  \"smooth\": true,                                  // Gouraud smooth shading (best with PNG)
  \"specular\": 0.2,                                 // Specular highlight intensity (0-1)
  \"shininess\": 32,                                 // Specular exponent (higher = tighter)
  \"gamma_correction\": true,                        // Compute lighting in linear sRGB space
  \"fresnel\": {\"intensity\": 0.3, \"power\": 5},       // Fresnel rim lighting (or just 0.3)
  \"sss\": false,                                    // true or {intensity, power, distortion}
  \"opacity\": 1.0,                                  // Global opacity (0-1)
  \"lights\": [],                                    // Array of light definitions (see Multi-Light)
  \"tone_mapping\": {\"method\": \"\", \"exposure\": 1.0}, // HDR tone mapping (or just \"aces\")
  \"shading\": \"\",                                   // \"blinn-phong\" (default), \"gooch\", \"cel\", \"flat\", \"normal\"
  \"gooch_warm\": \"#ffcc44\",                         // Gooch warm tone color
  \"gooch_cool\": \"#4466cc\",                         // Gooch cool tone color
  \"cel_bands\": 4,                                  // Number of cel-shading bands
  \"materials\": {},                                 // OBJ material map: { \"name\": \"#hex\" }
  \"highlight\": {},                                 // OBJ group highlight: \"#hex\" or {color, specular, ...}
  // ── Annotations ─────────────────────────────────────────────────
  \"annotations\": false,                            // true or {groups, color, font_size, offset}
  // ── Color Mapping ─────────────────────────────────────────────────
  \"color_map\": \"\",                                 // \"overhang\", \"curvature\", \"scalar\", or \"\" (off)
  \"color_map_palette\": [],                         // Custom hex color gradient (curvature/scalar)
  \"scalar_function\": \"\",                           // Math expression for scalar mode: \"sqrt(x*x+y*y+z*z)\"
  \"vertex_smoothing\": 4,                           // Smooth color values across vertices (0-4)
  \"overhang_angle\": 45,                            // Overhang threshold in degrees
  // ── Outlines ──────────────────────────────────────────────────────
  \"outline\": false,                                // true or {color, width}
  // ── Effects ───────────────────────────────────────────────────────
  \"ground_shadow\": false,                          // true or {opacity, color}
  \"clip_plane\": null,                              // Clipping plane (a, b, c, d)
  \"explode\": 0,                                    // Exploded view factor
  \"point_size\": 0,                                 // Point cloud neighbor radius (0 = auto)
  \"antialias\": 1,                                  // 0: no antialias, 1:FXAA, 2:SSAA, 3-4:SSAAx2
  \"ssao\": false,                                   // true or {samples, radius, bias, strength}
  \"bloom\": false,                                  // true or {threshold, intensity, radius}
  \"glow\": false,                                   // true or {color, intensity, radius}
  \"sharpen\": false,                                // true or {strength} (default 0.5)
  // ── Multi-View ────────────────────────────────────────────────────
  \"views\": null,                                   // Named views: [\"front\", \"right\", \"top\", ...]
  \"turntable\": {\"iterations\": 0, \"elevation\": 40}, // Turntable views (or just 6)
  \"grid_labels\": true,                             // Show labels on multi-view grids
  // ── Diagnostics ───────────────────────────────────────────────────
  \"debug\": false,                                  // Overlay model metadata
  \"debug_color\": \"#cc2222\"                         // Debug text color
}"))

#pagebreak()

= Appearance

== Custom Color

Change the global color of the model by changing the `color` field.

```example
// hl: 2
#render-stl(cube, (
  color: "#c0ffee",
), width: 75%)
```

== Background Color

Set `background` to a hex color to fill the image background.

```example
// hl: 3
#render-stl(cube, (
  center: (0.5, 0.5, 0.5),
  background: "#1a1a2e",
), width: 75%)
```

Set `background: "none"` for a transparent background, which blends naturally into any Typst page.

```example
// hl: 2
#render-stl(cube, (
  background: "none",
), width: 75%)
```

Our cube view looks like a flat square because of the point of view, we might want to change it in order to change our point of view.

= Camera Position

== #link("https://en.wikipedia.org/wiki/Cartesian_coordinate_system")[Cartesian coordinates]

Change the camera position and where it points to using cartesian coordinates with `camera:(x,y,z)` and `center:(x,y,z)`. As you may have noticed with previous examples, Maquette finds the model's bounding box automatically by computing its center. So most of the time, no `center` needed.

```example
// hl: 2-3
#render-stl(cube, (
  camera: (2,3,3),
  center: (1, 1, 1),
), width: 64%)
```

== #link("https://en.wikipedia.org/wiki/Spherical_coordinate_system")[Spherical coordinates]

Instead of placing the camera with Cartesian `(x, y, z)` coordinates, you can use `azimuth` (horizontal angle) and `elevation` (vertical angle) in degrees. This makes it much easier to orbit around a model — just change the angles. When either is set, they override the `camera` field. The `distance` is auto-computed from the bounding box unless specified.

```example
// hl: 3-5
#render-obj(teapot, (
  up: (0, 1, 0),
  azimuth: 20,
  elevation: -20,
  distance: 7,
), width: 64%)
```

The `up` parameter defines which direction points "up" in the scene. The default is `(0, 0, 1)` (Z-up), which matches the convention used by most CAD software and STL files. OBJ files exported from Blender, game engines, or other Y-up tools typically need `up: (0, 1, 0)` to display correctly.

== #link("https://en.wikipedia.org/wiki/Field_of_view")[Field of View]

#grid(columns: (1fr, 1fr), column-gutter: 2em, row-gutter: 1.5em,
  [
    ```example
    // hl: 3
    #render-obj(teapot, (
      up: (0, 1, 0),
      fov: 20,
    ), width: 100%)
    ```
  ],
  [
    ```example
    // hl: 5-6
    #render-obj(teapot, (
      up: (0, 1, 0),
      azimuth: 45,
      distance: 30,
      auto_fit: false,
      fov: 10,
    ), width: 100%)
    ```
  ],
)

The `fov` parameter controls the vertical field of view angle (in degrees) for perspective projection. Lower values produce a telephoto effect, higher values create wide-angle distortion. Default is 45. By default (`auto_fit: true`), Maquette scales the model to fill the viewport; set `auto_fit: false` to use raw world-space coordinates, which lets you control framing manually with `distance` and `fov`.

#pagebreak()

= #link("https://en.wikipedia.org/wiki/3D_projection")[Projections]

Maquette supports 14 projection types. Set `projection: "name"` to switch. 

In the following examples we're using `stroke: (color, width)` to visualize triangle edges, in order to better visualize each projection's property.

#grid(columns: (1fr, 1fr), column-gutter: 2em, row-gutter: 1.5em,
  [
    == #link("https://en.wikipedia.org/wiki/Perspective_(graphical)")[Perspective] (default)
    Objects farther from the camera appear smaller, giving a natural sense of depth.
    ```example
    // hl: 3-4
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Orthographic_projection")[Orthographic]
    No perspective foreshortening: parallel lines stay parallel regardless of distance.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "orthographic",
    ))
    ```
  ],
)

#grid(columns: (1fr, 1fr), column-gutter: 2em, row-gutter: 1.5em,
  [
    == #link("https://en.wikipedia.org/wiki/Isometric_projection")[Isometric]
    Three principal axes appear equally foreshortened. Common in technical illustrations and game art.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "isometric",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Axonometric_projection#Dimetric_projection")[Dimetric]
    Two axes equally foreshortened, the third differs. Elevation ~20.7°, azimuth 45°.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "dimetric",
    ))
    ```
  ],
)

#grid(columns: (1fr, 1fr), column-gutter: 2em, row-gutter: 1.5em,
  [
    == #link("https://en.wikipedia.org/wiki/Axonometric_projection#Trimetric_projection")[Trimetric]
    All three axes have different foreshortening. Elevation ~25°, azimuth ~30°.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "trimetric",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Military_projection")[Military]
    Top-down axonometric where the plan view dominates. Elevation ~54.7°, azimuth 45°.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "military",
    ))
    ```
  ],
)

#pagebreak()

#grid(columns: (1fr, 1fr), column-gutter: 2em, row-gutter: 1.5em,
  [
    == #link("https://en.wikipedia.org/wiki/Oblique_projection#Cabinet_projection")[Cabinet]
    Oblique projection: front face at true shape, depth axis at half scale, 45° angle.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "cabinet",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Oblique_projection#Cavalier_projection")[Cavalier]
    Like cabinet, but the depth axis is drawn at full scale at a 45° angle. All dimensions are preserved equally.
    ```example
    // hl: 5
    #render-stl(cube, (
      camera: (3, 2, 2),
      stroke: (color: "#111111", width: 1.0),
      projection: "cavalier",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Fisheye_lens")[Fisheye]
    Equidistant: angular distance maps linearly to radius. Uniform distortion across the field.
    ```example
    // hl: 5
    #render-obj(teapot, (
      up: (0, 1, 0),
      elevation: 25,
      distance: 2.5,
      projection: "fisheye",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Stereographic_projection")[Stereographic]
    Conformal: preserves local shapes but enlarges the periphery. Compare the teapot's spout/handle to fisheye.
    ```example
    // hl: 5
    #render-obj(teapot, (
      up: (0, 1, 0),
      elevation: 25,
      distance: 2.5,
      projection: "stereographic",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Curvilinear_perspective")[Curvilinear]
    Perspective with barrel distortion: straight lines curve outward near the edges, simulating a wide-angle lens.
    ```example
    // hl: 5
    #render-obj(teapot, (
      up: (0, 1, 0),
      elevation: 25,
      distance: 14,
      projection: "curvilinear",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Cylindrical_perspective")[Cylindrical]
    Horizontal angles map linearly (like a panorama), vertical stays perspective. Keeps vertical lines straight.
    ```example
    // hl: 5
    #render-obj(teapot, (
      up: (0, 1, 0),
      elevation: 25,
      distance: 2.5,
      projection: "cylindrical",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Pannini_projection")[Pannini]
    Architectural photography projection: verticals stay straight, horizontals curve gracefully. A hybrid between cylindrical and stereographic.
    ```example
    // hl: 4
    #render-obj(teapot, (
      up: (0, 1, 0),
      distance: 2.5,
      projection: "pannini",
    ))
    ```
  ],
  [
    == #link("https://en.wikipedia.org/wiki/Stereographic_projection#Photography")[Tiny Planet]
    Full 360° inverse projection: objects ahead wrap to the outer edge, objects behind map to the center. Backface culling auto-disabled. Example shows Tiny Planet from inside our teapot.
    ```example
    // hl: 4
    #render-obj(teapot, (
      camera: (0, 1.7, 0),
      up: (0, 0, 1),
      projection: "tiny-planet",
    ))
    ```
  ],
)

#pagebreak()

= Shading & Lighting

== #link("https://en.wikipedia.org/wiki/Shading#Ambient_lighting")[Ambient] & Light Direction

The `ambient` parameter (0--1) controls how much light reaches surfaces regardless of their orientation. Low values create dramatic contrast; high values flatten the shading. The `light_dir` vector sets the direction light comes from.

#grid(columns: (1fr, 1fr, 1fr), gutter: 1em,
  align(center)[
    *`ambient: 0.05`*
    #render-obj(teapot, (
      up: (0, 1, 0),
      ambient: 0.05,
      width: 300,
      height: 300,
    ), width: 100%)
  ],
  align(center)[
    *`ambient: 0.3`*
    #render-obj(teapot, (
      up: (0, 1, 0),
      ambient: 0.3,
      width: 300,
      height: 300,
    ), width: 100%)
  ],
  align(center)[
    *`ambient: 0.6`*
    #render-obj(teapot, (
      up: (0, 1, 0),
      ambient: 0.6,
      width: 300,
      height: 300,
    ), width: 100%)
  ],
)

== Hemisphere Ambient

The `ambient` parameter accepts either a number (flat ambient, as before) or an object with `intensity`, `sky`, and `ground` fields. Hemisphere ambient lerps between a sky color (for upward-facing normals) and a ground color (for downward-facing normals), simulating environmental lighting without any extra cost.  

Defaults:
```typst
ambient: (intensity: 0.15, sky: "#ccd4e0", ground: "#d4ccc4")
```

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Flat ambient (default)*
    ```examplev
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.3,
      ambient: 0.3,
    ), width: 100%)```
  ],
  align(center)[
    *Hemisphere ambient*
    ```examplev
    // hl: 6
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.3,
      ambient: (intensity: 0.4, sky: "#8899cc", ground: "#443322"),
    ), width: 100%)```
  ],
)

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Silhouette")[Silhouette Outlines]

Draws bold edges where front-facing and back-facing triangles meet, producing a clean silhouette contour.

```example
// hl: 4
#render-stl(cube, (
  camera: (3, 2, 2),
  center: (0.5, 0.5, 0.5),
  outline: (color: "#000000", width: 5),
), width: 70%)
```

== Ground Shadow

A ground shadow is cast by projecting every triangle onto the ground plane along the light direction. Pass `ground_shadow: true` for defaults, or customize:
```typst
ground_shadow: (opacity: 0.3, color: "#000000")
```

```example
// hl: 4
#render-stl(cube, (
  camera: (3, 2, 2),
  light_dir: (1, -1, 3),
  ground_shadow: (opacity: 0.35, color: "#2244aa"),
), width: 70%)
```

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Gouraud_shading")[Smooth Shading]

Smooth shading is enabled by default. Vertex normals are averaged across adjacent faces and lighting is interpolated per-pixel (Gouraud shading), smoothing out the faceted appearance. Set `smooth: false` to revert to flat shading, where each triangle gets a single color based on its face normal.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Smooth (default)*
    #render-obj(teapot, (
      up: (0, 1, 0),
      width: 400,
      height: 400,
    ), width: 100%)
  ],
  align(center)[
    *Flat (`smooth: false`)*
    #render-obj(teapot, (
      up: (0, 1, 0),
      width: 400,
      height: 400,
      smooth: false,
    ), width: 100%)
  ],
)

== #link("https://en.wikipedia.org/wiki/Specular_highlight")[Specular Highlights]

Add Blinn-Phong specular highlights with the `specular` parameter (0--1). The `shininess` exponent controls how tight the highlight is: low values produce a broad, diffuse sheen; high values create a sharp, glossy spot. Specular works with both flat and smooth shading, and is most effective with PNG output.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Diffuse only*
    #render-obj(teapot, (
      up: (0, 1, 0),
      width: 400,
      height: 400,
      light_dir: (1,6,-7),
    ), width: 100%)
  ],
  align(center)[
    *Specular (`specular: 0.6, shininess: 32`)*
    #render-obj(teapot, (
      up: (0, 1, 0),
      specular: 0.6,
      light_dir: (1,6,-7),
      shininess: 32,
      width: 400,
      height: 400,
    ), width: 100%)
  ],
)

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Gamma_correction")[Gamma Correction]

By default (`gamma_correction: true`), colors are converted to linear space before shading and back to sRGB afterward. This produces physically accurate lighting: midtones brighten, dark areas gain detail, and specular highlights blend smoothly. Disabling it (`gamma_correction: false`) computes lighting directly in sRGB — faster, but produces harsher contrast and less natural results.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Without (`gamma_correction: false`)*
    #render-obj(teapot, (
      up: (0, 1, 0),
      specular: 0.6,
      shininess: 32,
      gamma_correction: false,
      width: 400,
      height: 400,
    ), width: 100%)
  ],
  align(center)[
    *With (default)*
    #render-obj(teapot, (
      up: (0, 1, 0),
      specular: 0.6,
      shininess: 32,
      width: 400,
      height: 400,
    ), width: 100%)
  ],
)

== #link("https://en.wikipedia.org/wiki/Fresnel_equations")[Fresnel] / Rim Lighting

Fresnel rim lighting brightens edges where the surface curves away from the camera, making objects stand out from the background. Control with `fresnel: (intensity, power)` or just `fresnel: 0.6` for intensity only. Higher power gives a thinner rim. Works with both flat and smooth shading.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Without fresnel*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.4,
      background: "#1a1a2e",
      width: 400,
      height: 400,
    ), width: 100%)
  ],
  align(center)[
    *With (`fresnel: 0.6`)*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.4,
      fresnel: 0.6,
      background: "#1a1a2e",
      width: 400,
      height: 400,
    ), width: 100%)
  ],
)

#pagebreak()

== Multi-Light

By default, a single white directional light is used (from `light_dir`). The `lights` array lets you define multiple lights, each with a type, direction or position, color, and intensity. When `lights` is set, it overrides `light_dir`.

Each light has `type`, `vector`, `color`, and `intensity`. Positional lights emit from a point in world space, creating distance-dependent shading.

Per-light shadows are not computed. The `ground_shadow` feature uses a single direction: the first directional light in the array, or `light_dir` as fallback.

```example
// hl: 6-10
#render-obj(teapot, (
  up: (0, 1, 0),
  ambient: 0.05,
  specular: 0.5,
  color: "#cccccc",
  lights: (
    (type: "positional", vector: (3, 3, 0), color: "#ff4444", intensity: 1.2),
    (type: "positional", vector: (-3, 2, 2), color: "#44ff44", intensity: 1.0),
    (type: "directional", vector: (0, 1, 0), color: "#4444ff", intensity: 0.5),
  ),
), width: 100%)
```

== #link("https://en.wikipedia.org/wiki/Tone_mapping")[Tone Mapping]

When multiple bright lights, strong specular, or fresnel push color values above 1.0, the default behavior hard-clips them to white — creating flat, washed-out highlights. Tone mapping compresses these HDR values gracefully, preserving detail and color in bright areas. Two operators are available: `"reinhard"` (simple, neutral) and `"aces"` (filmic, higher contrast). Use `tone_mapping: (method: "aces", exposure: 1.5)` for full control, or just `tone_mapping: "aces"` for the method alone.

#let tm-lights = (
  (type: "directional", vector: (1, 2, 1), color: "#ffaa66", intensity: 2.0),
  (type: "directional", vector: (-2, 1, -1), color: "#6699ff", intensity: 1.8),
  (type: "directional", vector: (0, -1, 2), color: "#ffffff", intensity: 0.8),
)
#let tm-base = (
  up: (0, 1, 0), azimuth: 180, distance: 0.25,
  ambient: 0.05, specular: 0.9, shininess: 16, fresnel: 0.5, color: "#ddccbb", background: "#1a1a2e",
  width: 350, height: 350, lights: tm-lights,
)

#grid(columns: (1fr, 1fr, 1fr), gutter: 0.8em,
  align(center)[
    *No tone mapping*
    #render-obj(bunny, tm-base, width: 100%)
  ],
  align(center)[
    *Reinhard*
    #render-obj(bunny, tm-base + (tone_mapping: "reinhard"), width: 100%)
  ],
  align(center)[
    *ACES*
    #render-obj(bunny, tm-base + (tone_mapping: "aces"), width: 100%)
  ],
)

#pagebreak()

== Shading Models

The `shading` parameter selects the lighting model, independently of the render mode (solid, wireframe, etc.).

#grid(columns: (1fr, 1fr), gutter: 16pt,
  [
    === #link("https://en.wikipedia.org/wiki/Blinn%E2%80%93Phong_reflection_model")[Blinn-Phong] (default)
    Photorealistic diffuse + specular.
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180, distance: 0.25,
      specular: 0.4,
    ), width: 100%)
  ],
  [
    === #link("https://en.wikipedia.org/wiki/Shading#Flat_shading")[Flat]
    Face-normal shading. Disables `smooth` unless explicitly set.
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180, distance: 0.25,
      shading: "flat",
    ), width: 100%)
  ],
  [
    === #link("https://en.wikipedia.org/wiki/Cel_shading")[Cel]
    Toon shading with discrete color bands.

    Control steps with `cel_bands`.
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180, distance: 0.25,
      specular: 0.4,
      shading: "cel", fresnel: 0.6, outline: (width: 2.0),
    ), width: 100%)
  ],
  [
    === #link("https://en.wikipedia.org/wiki/Gooch_shading")[Gooch]
    Warm-to-cool non-photorealistic shading.

    Customize with `gooch_warm` and `gooch_cool`.
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180, distance: 0.25,
      specular: 0.4,
      shading: "gooch", gooch_warm: "#ffcc44", gooch_cool: "#2255ff",
    ), width: 100%)
  ]
)

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Subsurface_scattering")[Subsurface Scattering]

Fake view-dependent subsurface scattering simulates light passing through thin geometry (wax, skin, marble, leaves). Back-lit areas glow with a color derived from the light and the model's base color. Works with any shading model.

=== Without Subsurface Scattering

```example
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  lights: (
  (type: "positional", 
   vector: (-0.1, 0.14, -0.04),
   color: "#ff0000", intensity: 3.0)),
  ), width: 100%)
```

=== Subsurface Scattering (back-lit bunny)

```example
// hl: 9
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  lights: (
  (type: "positional", 
   vector: (-0.1, 0.14, -0.04),
   color: "#ff0000", intensity: 3.0)),
  sss: (intensity: 4, power: 3.5, distortion: 0.2),
), width: 100%)
```

#pagebreak()

= Color Mapping <color-mapping>

Color mapping replaces the uniform model color with a gradient derived from geometric properties.

== Curvature Map

Colors vertices based on local surface curvature — the rate at which the surface bends. Low curvature (flat areas) maps to blue/dark colors, while high curvature (sharp edges, creases) maps to red/bright colors. Useful for quality inspection, identifying sharp features, or visualizing mesh topology.

```example
// hl: 10
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  ambient: 0.3,
  specular: 0.5,
  width: 400,
  height: 400,
  vertex_smoothing: 4,
  color_map: "curvature",
), width: 70%)
```

You might have noticed the appearance of a `vertex_smoothing` setting in the previous render. This parameter allows to smoothen the color distribution across vertices, otherwise the color looks flaky, as you can see with `vertex_smoothing: 0`:

```example
// hl: 7-8
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  ambient: 0.3,
  specular: 0.5,
  color_map: "curvature",
  vertex_smoothing: 0,
), width: 70%)
```

== Overhang Map

Faces steeper than `overhang_angle` (relative to vertical) are highlighted in red, while supported faces remain green. This directly maps to where a 3D printer would need support structures.

```example
// hl: 7-8
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  ambient: 0.3,
  specular: 0.5,
  color_map: "overhang",
  overhang_angle: 45,
), width: 70%)
```

#pagebreak()

== Scalar Function

Color vertices based on a user-defined mathematical function `f(x,y,z)`. The `scalar_function` expression is evaluated at each vertex position, producing scalar values that are automatically normalized to `[0, 1]` (min value → 0, max value → 1), then linearly interpolated through the `color_map_palette` color stops. If no palette is specified, the default blue → cyan → green → yellow → red gradient is used. Per-vertex colors are interpolated across triangle faces for smooth results.

The expression can use:
- *Variables:* `x`, `y`, `z` (vertex coordinates)
- *Constants:* `pi`, `e`, `tau`
- *Arithmetic:* `+`, `-`, `*`, `/`, `^` (power)
- *Comparison:* `<`, `>`, `<=`, `>=`, `==`, `!=` (return 0.0 or 1.0)
- *Logical:* `&&` (and), `||` (or), `!` (not)
- *Functions:*
  - Basic: `abs`, `sqrt`, `min`, `max`, `clamp`, `sign`, `pow`
  - Trigonometric: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`
  - Hyperbolic: `sinh`, `cosh`, `tanh`
  - Exponential/Logarithmic: `exp`, `ln`, `log10`, `log2`
  - Rounding: `floor`, `ceil`, `round`, `fract`
  - Graphics: `step`, `smoothstep`, `mix`, `lerp`, `length`
  - Other: `mod`

Some examples:

```example
// hl: 7-8
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  ambient: 0.3,
  specular: 0.5,
  color_map: "scalar",
  scalar_function: "smoothstep(-0.1, 0.1, x)",
), width: 80%)
```

```example
// hl: 7-14
#render-obj(bunny, (
  up: (0, 1, 0),
  azimuth: 180,
  distance: 0.25,
  ambient: 0.3,
  specular: 0.5,
  color_map: "scalar",
  scalar_function: "sin(x*60)*cos(y*60) + sin(y*60)*cos(z*60) + sin(z*60)*cos(x*60)",
  color_map_palette: (
    "#1a0533",
    "#6b2fa0",
    "#e85d75",
    "#ffcc33",
  ),
), width: 80%)
```

#pagebreak()

= File Formats & Coloring

== STL Per-face Color

Some binary STL files encode per-face colors in the attribute bytes using the RGB565 format. Maquette detects and renders these automatically — no config needed. When present, the `color` parameter is ignored in favor of the embedded colors.

```example
// hl: 1
#let colored = read("data/colored_cube.stl", encoding: none)

#render-stl(colored, (projection: "isometric"), width: 65%)
```

== PLY Format

=== Meshes

Maquette handles PLY files in ASCII and binary (little/big-endian) formats — all three are parsed automatically. PLY can store colors in its format, allowing us to color the model directly. Enjoy this beautiful PLY-colored Rubik's cube generated with Blender.

```example
// hl: 1
#let rubi_blender = read("data/rubi_blender.ply", encoding: none)

#render-ply(rubi_blender, (
  azimuth: 45,
  elevation: 25,
  distance: 10,
), width: 65%)
```

=== Point Clouds

PLY files can also contain clouds of points. 3D scanning apps usually allow to export in such a format.  Enjoy my Rubik's cube scanned with the help of my iPad's LiDAR! Point clouds rendering are configurable with `point_size`, which will define the distance for points to be considered as neighbors for our #link("https://en.wikipedia.org/wiki/K-nearest_neighbors_algorithm")[k-NN] to reconstruct the model. Auto-computed if zero.

```example
// hl: 1
#let rubi_scan = read("data/rubi_scan.ply", encoding: none)

#render-ply(rubi_scan, (
  up: (0,1,0),
  elevation: 25,
  distance: 0.8,
  auto_fit: false,
), width: 65%)
```

== OBJ Material Coloring

OBJ files can reference materials via `usemtl` directives. Provide a `materials` map to assign hex colors to each material name.

#grid(
  columns: (1fr, 2fr),
  rows: (auto, auto),
  gutter: 10pt,
  [
We list `cube.obj`'s materials as follows:

  ```sh
$ grep usemtl cube.obj
usemtl face1
usemtl face2
usemtl face3
usemtl face4
usemtl face5
usemtl face6
```
], [

  #align(center,[Then colorize material-wise:])

  ```example
// hl: 5-9
#let obj-cube = read("data/cube.obj")

#render-obj(obj-cube, (
  camera: (3,3,3),
  materials: (
    face4: "#2ecc71",
    face5: "#222222",
    face6: "#ff2222",
  )))
```
]
)

== OBJ Groups

=== Group Highlight

OBJ files with `g` or `o` directives define named groups. The `highlight` map assigns custom styling to specific groups.

Groups not listed keep their default appearance.

==== Available Attributes

#grid(columns: (59%, auto), gutter:1em, [
  When using the full object syntax, all attributes are optional:
  #text(size: 9pt, raw(block: true, lang: "json", "{
  \"highlight\": {
    \"GroupName\": \"#hexcolor\",  // Simple: just a color string
    \"AnotherGroup\": {          // Advanced: full appearance object
      \"color\": \"#ff0000\",      // Hex color (overrides global color)
      \"specular\": 0.8,         // Specular intensity 0-1 (overrides global)
      \"shininess\": 64,         // Specular exponent (overrides global)
      \"ambient\": 0.3,          // Ambient light 0-1 (overrides global)
      \"stroke\": \"#000000\",     // Triangle edge stroke color
      \"stroke_width\": 1.0,     // Triangle edge stroke width
      \"opacity\": 0.5           // Transparency 0-1 (0=invisible, 1=opaque)
    }
  }
}"))
], [We can list the groups as follows:

#text(size: 8.4pt, raw(block: true, lang: "sh", "$ grep \"g \" crankshaft.obj
g Model__Piston_F
g Model__Head
g Model__Piston_E
g Model__Head_2
g Model__Piston_D
g Model__Head_3
g Model__Piston_C
g Model__Head_4
g Model__Piston_B
g Model__Head_5
g Model__Piston_A
g Model__Head_6
g Model__Crankshaft
g Model__Camshaft"))])

Here's an example of a #link("https://www.cgtrader.com/items/124377/download-page")[crankshaft] with several parts defined by groups in the `.obj` file format:

```example
// hl: 6-10
#align(center,
render-obj(crankshaft, (
  camera: (-100, -100, 500),
  up: (0, -1, 0),
  color: "#777777",
  highlight: (
    Model__Camshaft: "#ff0000",
    Model__Crankshaft: "#00ff00",
    Model__Piston_B: (color: "#0000ff"),
  ),
), width: 87%))
```

#pagebreak()

=== Per-group appearance

Instead of a plain color, pass a dictionary with specific appearance overrides to selectively change parts appearance.

```example
// hl: 6-11
#render-obj(crankshaft, (
  camera: (-100, -100, 500),
  up: (0, -1, 0),
  color: "#777777",
  antialias: 4,
  highlight: (
    Model__Crankshaft: (color: "#cc0000", stroke: "#ffffff", stroke_width: 0.5),
    Model__Piston_A: (color: "#88ccff", opacity: 0.3),
    Model__Piston_C: (color: "#88ccff", opacity: 0.3),
    Model__Piston_D: (color: "#88ccff", opacity: 0.3),
  ),
), width: 100%)
```

=== Annotations

Annotate OBJ groups by drawing a leader line from each group's centroid to a text label.

Pass `annotations: true` to label all groups with default styling, or pass an object to customize. The `groups` field filters to specific groups; `color`, `font_size`, and `offset` control appearance.

```example
// hl: 5-10
#render-obj(crankshaft, (
  camera: (-100, -100, 500),
  up: (0, -1, 0),
  color: "#777777",
  annotations: (
    groups: ("Model__Piston_A", "Model__Piston_B", "Model__Piston_C", "Model__Piston_D", "Model__Piston_E", "Model__Piston_F"),
    color: "#bb2222",
    font_size: 12,
    offset: 45,
  ),
), width: 100%)
```

#pagebreak()

= Render Modes <render-modes>

== Solid (default)

Nothing new, since it's the default mode we saw earlier, but it allows me to introduce this beautiful low poly `brain_skull.obj`.

```example
#let skull-brain = read("data/brain_skull.obj", encoding: none)

#render-obj(skull-brain, (
  azimuth: 270,
  up: (0,1,0),
  distance: 200,
  color: "#e8e8e8",
  width: 600,
  height: 600,
), width: 100%)
```

== X-Ray Mode

(`mode: "x-ray"`) creates transparent front-facing surfaces. Back-facing surfaces remain fully opaque.

Adjust transparency with `xray_opacity` (default 0.1 opacity).

Ideal for examining joints, internal components, nested geometry, and embedded models.

Our previous skull model now unveils its inner brain!

```example
// hl: 5-10
#render-obj(skull-brain, (
  azimuth: 270,
  up: (0,1,0),
  distance: 200,
  highlight: (
    "Skull": (color: "#e8e8e8"),
    "Brain": (color: "#ee69b4"),
  ),
  xray_opacity: 0.3,
  mode: "x-ray",
), width: 100%)
```

Useful when the model has no groups to distinguish.

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Wire-frame_model")[Wireframe]

Wireframe mode draws every triangle edge without fill or back-face culling, showing the full mesh topology. Control appearance with `wireframe: (color, width)`.

It's a great occasion to showcase SVG output, no rasterization artifacts and kind-of infinite zooming allowed, thanks to vectors.

```examplev
// hl: 6-8
#render-obj(teapot, (
  up: (0, 1, 0),
  distance: 8,
  auto_fit: false,
  background: "#ffffff",
  wireframe: (color: "#cc3333", width: 0.1),
  mode: "wireframe",
), width: 55%, format: "svg")
```

== Solid + Wireframe

Combines solid shading with wireframe edges overlaid on top. Useful for visualizing mesh density and triangle distribution while still seeing the shaded surface. Configure edge appearance with `wireframe: (color, width)`.

Here ```typst antialias: 4``` should be set, wireframe's strokes benefit from antialiasing.
```example
// hl: 4-6
#render-obj(teapot, (
  up: (0, 1, 0),
  distance: 8,
  antialias:4,
  mode: "solid+wireframe",
  wireframe: (width: 0.3),
), width: 80%)
```

#pagebreak()

= Post-Processing

== #link("https://en.wikipedia.org/wiki/Spatial_anti-aliasing")[Antialiasing]

The `antialias` parameter controls supersampling for PNG output. A value of `1` (default) means no supersampling; `2` renders at 2×2 the resolution and downsamples for smoother edges; `4` gives the highest quality. Values of `3` and `4` are equivalent (both render at 4× internally).

This only affects PNG — SVG output is resolution-independent.

As a rule of thumb, FXAA does the job for most renders. Turn `antialias: 4` when using wireframe or stroke, straight lines benefit from antialiasing strongly.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *No antialiasing (`antialias: 0`)*
    #render-obj(teapot, (
      camera: (0, 2, 5),
      up: (0, 1, 0),
      specular: 0.5,
      width: 300,
      height: 300,
      antialias: 0,
    ), width: 100%)
  ],
  align(center)[
    *FXAA (`antialias: 1`, default)*
    #render-obj(teapot, (
      camera: (0, 2, 5),
      up: (0, 1, 0),
      specular: 0.5,
      width: 300,
      height: 300,
      antialias: 1,
    ), width: 100%)
  ],align(center)[
    *SSAA (`antialias: 2`)*
    #render-obj(teapot, (
      camera: (0, 2, 5),
      up: (0, 1, 0),
      specular: 0.5,
      width: 300,
      height: 300,
      antialias: 2,
    ), width: 100%)
  ],
  align(center)[
    *4× supersampling (`antialias: 4`)*
    #render-obj(teapot, (
      camera: (0, 2, 5),
      up: (0, 1, 0),
      specular: 0.5,
      width: 300,
      height: 300,
      antialias: 4,
    ), width: 100%)
  ],
)

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Ambient_occlusion")[Ambient Occlusion]

Ambient Occlusion adds realistic contact shadows (⚠️ at the cost of increased processing time) in crevices and areas where surfaces are close together, simulating how indirect light is blocked in tight spaces. SSAO computes occlusion by sampling the depth buffer after rasterization. Configure with `ssao: true` for defaults, or customize:
```typst
ssao: (samples: 16, radius: 0.5, bias: 0.025, strength: 1.0)
```

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Without SSAO*
    #render-obj(crankshaft, (
      camera: (-100, -100, 500),
      up: (0, -1, 0),
      specular: 0.5,
      color: "#777777",
      width: 400,
      height: 400,
    ), width: 95%)
  ],
  align(center)[
    *With SSAO*
    #render-obj(crankshaft, (
      camera: (-100, -100, 500),
      up: (0, -1, 0),
      ssao: (samples: 16, radius: 0.5, strength: 1),
      antialias: 4,
      color: "#777777",
    ), width: 95%)
  ],
)

== #link("https://en.wikipedia.org/wiki/Bloom_(shader_effect)")[Bloom] & Glow

Bloom makes bright areas bleed light outward. Glow creates a uniform aura around the model's silhouette. Both are PNG-only post-processing effects. Use `bloom: true` / `glow: true` for defaults, or customize:
```typst
bloom: (threshold: 0.8, intensity: 0.3, radius: 10)
glow: (color: "#ffffff", intensity: 0.5, radius: 15)
```

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Bloom*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.2,
      shininess: 64,
      fresnel: 0.5,
      bloom: (threshold: 0.2, intensity: 1.2, radius: 10),
      background: "#000000",
    ), width: 95%)
  ],
  align(center)[
    *Glow*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.2,
      glow: (color: "#00ff00", intensity: 0.5, radius: 13),
      color: "#008800",
      background: "#000000",
    ), width: 95%)
  ],
)

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Unsharp_masking")[Sharpening]

Sharpening enhances edge contrast using a 3×3 unsharp mask. Pass `sharpen: true` for default strength (0.5), or customize with `sharpen: (strength: N)`. Higher values produce a more pronounced effect.

#grid(columns: (1fr, 1fr), gutter: 1em,
  align(center)[
    *Without*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.4,
    ), width: 95%)
  ],
  align(center)[
    *Sharpen (strength: 2)*
    #render-obj(bunny, (
      up: (0, 1, 0),
      azimuth: 180,
      distance: 0.25,
      specular: 0.4,
      sharpen: (strength: 2),
    ), width: 95%)
  ],
)

= Effects

== #link("https://en.wikipedia.org/wiki/Clipping_(computer_graphics)")[Clipping Plane]

Cut away part of a model using a mathematical plane defined as $(a, b, c, d)$ where $a x + b y + c z + d >= 0$ is kept.

With `cull_backface: false`, the inner model becomes visible through the opening instead of being capped — useful for inspecting internal geometry.

```example
// hl: 9-10
#render-obj(skull-brain, (
  azimuth: 220,
  up: (0,1,0),
  highlight: (
    "Skull": (color: "#e8e8e8"),
    "Brain": (color: "#ff69b4"),
  ),
  distance: 200,
  clip_plane: (2, -1, 0, 1),
  cull_backface: false,
), width: 72%)
```

#pagebreak()

== #link("https://en.wikipedia.org/wiki/Exploded-view_drawing")[Exploded View]

Move model parts outward from the model center. Very useful to showcase different parts of a system in mechanics.

For OBJ files with `g` or `o` groups, each group is treated as a separate component.

```example
// hl: 5
#render-obj(crankshaft, (
  up: (0, -1, 0),
  camera: (-200, -200, 500),
  color: "#555555",
  explode: 0.8,
))
```

For PLY, STL files or OBJ files without groups, connected components are detected automatically using shared edges (union-find). Each component is then offset by `explode * (component_centroid - model_center)`. 

This is the case for this exploded teapot.

```example
// hl: 4
#render-obj(teapot, (
  camera: (0, 2, 5),
  up: (0, 1, 0),
  explode: 0.5,
))
```

#pagebreak()

= Multi-View

== Multi-View Grid

Render multiple named views in a single image, similar to an engineering drawing sheet. Available views are `"front"`, `"back"`, `"left"`, `"right"`, `"top"`, `"bottom"`, and `"isometric"`. The renderer arranges them in a grid and labels each cell.

```example
// hl: 2-3
#render-obj(teapot, (
  views: ("front", "right", "top", "isometric"),
  grid_labels: true,
), width: 100%)
```

== Turntable

Automatically generates a grid of views evenly spaced around the model at a fixed elevation angle. Use `turntable: (iterations: 6, elevation: 40)` or just `turntable: 6` for the number of views. View labels showing the azimuth angle are displayed by default; set `grid_labels: false` to hide them.

```example
// hl: 2-3
#render-obj(teapot, (
  turntable: (iterations: 6, elevation: 40),
  grid_labels: true,
), width: 100%)
```

#pagebreak()

= Debug

`debug: true` overlays model metadata (triangle count, bounding box, camera position) directly on its canvas.

It also renders lights as octahedrons of the color they emit, to allow placing lights seamlessly around your model.

```example
// hl: 2
#render-obj(teapot, (
  debug: true,
  lights: (
    (type: "positional", vector: (2, 4, 0), color: "#ff4444", intensity: 1.2),
    (type: "positional", vector: (-1, 2, 2), color: "#44ff44", intensity: 1.0),
    (type: "directional", vector: (0, 1, 0), color: "#4444ff", intensity: 0.5),
  ),
), width: 100%)
```

== #link("https://en.wikipedia.org/wiki/Normal_mapping")[Normal Mapping]
   Maps surface normals to RGB. Useful for debugging geometry and inspecting mesh quality.

   ```example
   // hl: 4
   #render-obj(bunny, (
     up: (0, 1, 0),
     azimuth: 180, distance: 0.25,
     shading: "normal",
   ), width: 100%)
   ```

== More functions

The `get-stl-info`, `get-obj-info`, and `get-ply-info` functions are used to output the model's metadata and are available in the plugin.

```typst
#let info = get-obj-info(teapot)
#for (key, val) in info.pairs() [*#key:* #repr(val)]
```

#pagebreak()

= Contributing

Bug reports, feature requests, issues, new feature ideas are welcome on #link("https://github.com/bernsteining/maquette")[GitHub].

= Models Credits

- #link("https://graphics.stanford.edu/courses/cs148-10-summer/as3/code/as3/teapot.obj")[Utah teapot] — Stanford
- #link("https://graphics.stanford.edu/~mdfisher/Data/Meshes/bunny.obj")[Stanford bunny] — Stanford
- #link("https://www.cgtrader.com/free-3d-models/vehicle/vehicle-part/crankshaft-with-pistons-3783b2997aa60fea365daf96a6754cf6")[Crankshaft with pistons] — CGTrader
- #link("https://sketchfab.com/3d-models/the-brain-007847f9d2b5481a882d8996c0fd1847")[Low-poly brain] — Sketchfab
- #link("https://www.printables.com/model/1047493-low-poly-skull/files")[Low-poly skull] — Printables
- Rubik's cubes: Blender generated & LiDAR scanned