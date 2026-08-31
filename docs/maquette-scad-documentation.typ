#import "@local/maquette-scad:0.1.0": *
#import "@preview/maquette:0.1.3": render-ply

#import "@preview/zebraw:0.6.1": *

#set page(margin: 1.5em, footer: context grid(
  columns: (1fr, 1fr),
  align(left, text(size: 7.5pt, fill: luma(120))[maquette-scad Documentation]),
  align(right, text(size: 7.5pt, fill: luma(120), counter(page).display())),
))
#set par(justify: true)
#show: zebraw.with(lang: false, numbering: false)

// A neutral shading preset every example uses so shots don't fight the
// per-example composition. Accepts either raw PLY bytes (from `scadypst()`)
// or an unbuilt expression tree (dict from `cube` / `difference` / etc.) —
// the latter gets auto-compiled so example code can skip `scadypst(...)`.
#let show-part(part, ..args) = {
  let ply = if type(part) == dictionary { scadypst(part) } else { part }
  render-ply(ply,
    color: "#4488cc",
    shading: "gooch", gooch_warm: "#e8ceaa", gooch_cool: "#3a4a70",
    camera: (40, 40, 40), up: (0, 0, 1), fov: 40, zoom: 1.2,
    background: "#f7f7fa",
    antialias: 2,
    ..args,
  )
}

// Scope shared with the `example` show rule so eval() sees the SCAD DSL +
// render helper without re-importing per example.
#let doc-scope = (
  render-ply: render-ply, show-part: show-part, image: image,
  scadypst: scadypst, compile-scad: compile-scad,
  scadypst-svg: scadypst-svg, compile-scad-svg: compile-scad-svg,
  scadypst-info: scadypst-info, compile-scad-info: compile-scad-info,
  scadypst-parts: scadypst-parts, compile-scad-parts: compile-scad-parts,
  cube: cube, sphere: sphere, cylinder: cylinder, polyhedron: polyhedron,
  square: square, circle: circle, ellipse: ellipse, polygon: polygon,
  ngon: ngon, star: star, rounded-square: rounded-square,
  linear-extrude: linear-extrude, rotate-extrude: rotate-extrude, projection: projection,
  slice: slice, trim: trim, "hull-pts": hull-pts,
  translate: translate, rotate: rotate, scale: scale, mirror: mirror,
  resize: resize, offset: offset, color: color,
  union: union, difference: difference, intersection: intersection,
  hull: hull, minkowski: minkowski,
)

// Same show-rule protocol as maquette-documentation.typ:
//   // hl: 3, 5-7   → highlight lines
//   // cols: 2 1    → code:render column ratio
#let filter-eval(text) = text.split("\n").filter(l =>
  not l.starts-with("#import")
).join("\n")

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
  #text(font: "Libertinus Serif", size: 32pt, weight: "bold")[maquette-scad]
  #v(0.3em)
  #text(size: 14pt, fill: gray)[Parametric CAD in Typst, via OpenSCAD + Manifold]
  #v(1.5em)
  #text(size: 12pt, blue)[
    #link("https://github.com/bernsteining/maquette")[github.com/bernsteining/maquette] · #link("https://bernsteining.github.io/maquette")[bernsteining.github.io/maquette]
  ]
  #v(1em)
  #show-part(
    difference(
      union(
        cube(30, center: true),
        translate((0, 0, 15), sphere(15, fn: 48)),
      ),
      union(
        cylinder(50, r: 6, center: true, fn: 48),
        rotate((0, 90, 0), cylinder(50, r: 6, center: true, fn: 48)),
        rotate((90, 0, 0), cylinder(50, r: 6, center: true, fn: 48)),
      ),
    ),
    width: 55%,
    camera: (60, 60, 50), zoom: 1.1,
    shadows: (resolution: 1024, softness: 1),
    ssao: (samples: 12, radius: 0.3, strength: 0.9),
  )
  #v(0.4em)
  #text(size: 10pt, fill: luma(150))[Version 0.1.0 #h(0.4em)·#h(0.4em) #datetime.today().display("[month repr:long] [day], [year]")]
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

*maquette-scad* produces a PLY mesh (or 2D SVG) from a geometry description. Two ways to write the description:

- *`scadypst(tree)`* — build geometry from Typst-native helpers (`cube`, `sphere`, `difference`, `translate`, …) composed with Typst's own `for`, `range`, and `calc`. Returns PLY bytes.
- *`compile-scad(source)`* — pass the text of an existing `.scad` file. Returns PLY bytes.

Both entry points also have a `-svg` sibling (`scadypst-svg`, `compile-scad-svg`) for direct 2D vector output — no rasterizer in the loop.

This document is the plugin's Typst API reference: what each helper accepts, what the compile calls return, and how to route sidecar assets. Downstream rendering options (camera, lighting, shadows, tone mapping) belong to `maquette` — see its #link("maquette-documentation.pdf")[manual]. For the OpenSCAD language itself see the #link("https://openscad.org/documentation.html")[OpenSCAD Users Manual].

#pagebreak(weak: true)

= Where to find sample `.scad` files

The DSL examples in this document build their geometry inline, so no external source is needed. For the `compile-scad(read("..."))` path, `.scad` sources are easy to find:

- #link("https://github.com/openscad/openscad/tree/master/examples")[openscad/openscad `examples/`] — the set that ships with the OpenSCAD editor.
- #link("https://github.com/BelfrySCAD/BOSL2")[BOSL2] — a large utility library with hundreds of documented fragments.
- #link("https://www.thingiverse.com/tag:openscad")[Thingiverse (openscad tag)] — community archive; many entries ship the `.scad` alongside the `.stl`.

= Quickstart

Both entry points return PLY bytes; pass them to `render-ply` with the usual maquette configuration.

```typ
#import "@preview/maquette-scad:0.1.0": scadypst, cube, sphere, difference
#import "@preview/maquette:0.1.3": render-ply

#let part = scadypst(
  difference(
    cube(20, center: true),
    sphere(12, fn: 48),
  )
)
#render-ply(part, camera: (40, 40, 40), up: (0, 0, 1))
```

`compile-scad` has the same shape but takes source text:

```typ
#import "@preview/maquette-scad:0.1.0": compile-scad
#import "@preview/maquette:0.1.3": render-ply

#let part = compile-scad(read("model.scad"))
#render-ply(part)
```

Examples below use a `show-part` wrapper so the code can focus on geometry; in real documents you would inline the `render-ply(bytes, ...)` call.

#pagebreak(weak: true)

= Building geometry from Typst — `scadypst`

`scadypst(tree)` walks a tree of Typst dicts and returns PLY bytes. Each helper (`cube`, `sphere`, `translate`, …) builds a node; nothing runs until `scadypst` receives the whole tree.

Because the tree is Typst code, iteration and arithmetic come from Typst. A `..for` spread inside `union` places twelve spheres on a ring:

```example
// cols: 2 1
#let ring = union(..for i in range(12) {
  let a = i * 30
  let r = 20
  (translate(
    (calc.cos(a * 1deg) * r, calc.sin(a * 1deg) * r, 0),
    sphere(3, fn: 24),
  ),)
})
#show-part(scadypst(ring))
```

Variadic ops (`union`, `difference`, `hull`, `intersection`) accept `..items` — spread a Typst `for` block to feed a computed list.

#pagebreak(weak: true)

== DSL reference

Every helper takes Typst-native named arguments (`cube(20, center: true)`). Signatures below list positional args first, then named. `fn: N` sets the per-primitive segment count.

=== 3D primitives

- *`cube(size, center: false)`* — `size` is a number (uniform) or a 3-array `(x, y, z)`.
- *`sphere(r, fn: none)`*
- *`cylinder(h, r: none, r1: none, r2: none, center: false, fn: none)`* — pass `r` for a straight cylinder, `r1` + `r2` for a cone / frustum.
- *`polyhedron(points, faces)`* — raw mesh: `points = ((x, y, z), …)`, `faces = ((i, j, k, …), …)`.

=== 2D primitives (extrusion sources)

- *`square(size, center: false)`*, *`circle(r, fn: none)`*, *`ellipse(w, h, fn: none)`*.
- *`polygon(points, paths: none)`* — arbitrary 2D shape. `paths` is a list of index-rings into `points`: first ring is the outer boundary, rest are holes.
- *`ngon(sides, r, fn: none)`* — regular polygon.
- *`star(points, outer, inner)`* — n-pointed star.
- *`rounded-square(w, h, r, fn: none)`* — rectangle with radius-`r` corners.
- *`scad-text(str, size: 10)`* — glyph outlines from the font passed to `scadypst()`'s `font:` param.
- *`import-mesh(file)`* — reference an STL/OBJ passed via `scadypst()`'s `bin:` dict.

=== 2D → 3D lifting (and 3D → 2D)

- *`linear-extrude(h, child, center: false, twist: 0, scale: 1, slices: none)`* — extrude a 2D shape to a height `h`. `twist` is degrees over the full height; `scale` is a taper factor (0..1) or an `(sx, sy)` pair.
- *`rotate-extrude(child, angle: 360, fn: none)`* — revolve a 2D profile (living in the +x half-plane) around the z-axis.
- *`projection(child)`* — flatten a 3D solid to its 2D shadow on the Z=0 plane.

=== Transforms (2D or 3D)

- *`translate(v, child)`* — `v` is a 2- or 3-vector.
- *`rotate(deg, child)`* — Euler degrees, `(x, y, z)` or a single number (2D rotation).
- *`scale(v, child)`* — per-axis scale.
- *`mirror(v, child)`* — reflect across a plane whose normal is `v`.
- *`multmatrix(m, child)`* — 4×4 (or 4×3) affine matrix; escape hatch for anything the named transforms can't do.
- *`resize(v, child)`* — non-uniform scale to fit a target bounding box.
- *`offset(d, child)`* — grow (`d > 0`) or shrink (`d < 0`) a 2D shape by `d`.
- *`color(rgb, child, alpha: none)`* — RGB is a 3-array of 0..1 floats, or a 4-array `(r, g, b, a)`. See the Colours + alpha section below.

=== Booleans and hulls

- *`union(..items)`*, *`difference(..items)`* (first minus the rest), *`intersection(..items)`*.
- *`hull(..items)`* — convex hull of the union (3D).
- *`hull-pts(points)`* — convex hull from a raw 3D point set (a list of `(x, y, z)` triples). Complements `hull()` when you have coordinates, not geometry, to wrap.
- *`minkowski(..items)`* — Minkowski sum (3D).

All variadic — use `..` spread with a Typst `for` block to feed a computed list of children.

=== Plane operations

- *`slice(child, z: 0)`* — horizontal cross-section of a 3D solid at the given Z. Returns a 2D shape; pipe into `scadypst-svg(..)` for a vector contour, or into further 2D ops. Distinct from `projection(..)`, which unions every horizontal slice.
- *`trim(child, normal, offset: 0)`* — cut with a plane. Keeps the half where `dot(pos, normal) ≥ offset`. Cheaper and exact vs the "difference() with a giant cube" trick.

```example
// cols: 2 1
#show-part(scadypst(
  trim(sphere(10, fn: 64), (0, 0, 1))
))
```

#pagebreak(weak: true)

== Colours and alpha

Colours propagate through boolean operations. The emitted PLY carries per-vertex RGB and `render-ply` picks it up automatically. Any part without a `color()` wrapper inherits the render config's `color:` argument.

```example
#show-part(
  scadypst(union(
    color((0.9, 0.3, 0.3), cube(10, center: true)),
    color((0.3, 0.7, 0.4), translate((15, 0, 0), sphere(6, fn: 32))),
  )),
  color: none,
)
```

Alpha is supported. Two equivalent forms:

```typc
color((0.9, 0.35, 0.35, 0.5), child)              // RGBA in one 4-array
color((0.9, 0.35, 0.35), child, alpha: 0.5)       // RGB + separate alpha
```

Interior geometry shows through translucent subtrees, so a coloured cover can reveal the shape's internal features.

#pagebreak(weak: true)

== `scadypst()` — the compile call

```typc
scadypst(node, bin: (:), font: none, fn: 32)
```

- *`node`* — the tree returned by any DSL helper. Usually the top-level `union` / `difference` / a single primitive.
- *`bin`* — dict of sidecar bytes referenced by `import-mesh(file)`. Keys match the string passed to `import-mesh`, values are `read("path", encoding: none)`.
- *`font`* — TTF/OTF bytes for `scad-text(...)`. One font per compile.
- *`fn`* — default `$fn` for the whole compile. Per-primitive `fn:` overrides.

Returns the same PLY `bytes` that `compile-scad` returns — pass it to `render-ply`.

#pagebreak(weak: true)

== Name clashes with Typst built-ins

A glob-import (`#import "..": *`) shadows seven of Typst's own functions:

`scale`, `rotate`, `circle`, `square`, `ellipse`, `polygon`, `color`. (Additionally, the OpenSCAD `text` primitive is re-exported as `scad-text` to avoid shadowing `#text`.)

Two ways around it:

```typ
// Option 1 — namespace the plugin
#import "@preview/maquette-scad:0.1.0"
#let part = maquette-scad.scadypst(
  maquette-scad.difference(
    maquette-scad.cube(20, center: true),
    maquette-scad.sphere(12, fn: 48),
  )
)

// Option 2 — glob-import the plugin's `scad-*` aliases
#import "@preview/maquette-scad:0.1.0": scad-color, scad-scale, scad-rotate, scad-circle, scad-square, scad-ellipse, scad-polygon
```

Both leave Typst's own `#text` / `#circle` / … intact. The `scad-*` aliases live alongside the unprefixed names, so you can mix approaches within one document.

#pagebreak(weak: true)

= Compiling `.scad` sources — `compile-scad`

For a standalone `.scad` file, one call is enough:

```typ
#let part = compile-scad(read("part.scad"))
#render-ply(part)
```

Real-world `.scad` files rarely stand alone: they `use` / `include` a library, `import` an STL or OBJ for a fastener, or ship a font for embossed text. The wasm sandbox has no filesystem, so every sidecar is passed explicitly as bytes:

```typ
#compile-scad(read("main.scad"),
  files: (
    "utils.scad":                     read("utils.scad"),
    "gears.scad":                     read("gears.scad"),
    "MCAD/nuts_and_bolts.scad":       read("MCAD/nuts_and_bolts.scad"),
  ),
  bin: (
    "mount.stl":                      read("mount.stl", encoding: none),
  ),
  font: read("logo.ttf", encoding: none),
  fn: 48,
  trace: "trace.log",
)
```

Options:

- *`source`* (positional string) — the `.scad` text. Almost always `read("path.scad")`.
- *`files`* (dict) — resolves every `use <name>` / `include <name>` in the source. Keys are the exact names the `.scad` uses; nested paths like `MCAD/foo.scad` are keys with slashes.
- *`bin`* (dict) — sidecar mesh files referenced by `import()`. Keys match the `import` argument. Recognised formats: STL and OBJ (DXF, 3MF, and AMF are not).
- *`font`* (bytes) — a TTF or OTF file used by `text()`. One font per compile.
- *`fn`* (int) — default `$fn` for this compile. Any per-primitive `fn:` overrides it.
- *`trace`* (string) — dump the evaluator trace to this path. Handy when a nested `for` or `module` isn't producing what you expected.

#pagebreak(weak: true)

= Inspection: `scadypst-info` / `compile-scad-info`

Returns a Typst dict with the final geometry's stats without building
the PLY. Useful for laying parts out by their real size, annotating a
document with computed volumes, or failing early if a compile produced
empty geometry.

Fields: `bbox_min`, `bbox_max`, `center`, `radius`, `volume`,
`surface_area`, `num_tri`, `num_vert`, `genus`.

```example
// cols: 2 1
#let info = scadypst-info(cube(20, center: true))
#raw(block: true, lang: "yml",
  "volume:       " + str(info.volume) + " mm³\n" +
  "surface_area: " + str(info.surface_area) + " mm²\n" +
  "num_tri:      " + str(info.num_tri) + "\n" +
  "bbox_min:     " + repr(info.bbox_min) + "\n" +
  "bbox_max:     " + repr(info.bbox_max)
)
```

`compile-scad-info(src, files: (:), bin: (:), font: none, fn: 32)` is
the `.scad`-source variant with the same return shape.

#pagebreak(weak: true)

= Decomposing: `scadypst-parts` / `compile-scad-parts`

Splits the final geometry into its connected components via
`Manifold::decompose()` and returns each as its own PLY. The return
type is `array<bytes>`; each element is renderable via `render-ply` in
isolation.

Useful for laying out multiple pieces separately (laser-cut sheet
layouts, exploded-view annotations, per-part BOM tables).

```example
// cols: 2 1
#let parts = scadypst-parts(union(
  translate((0, 0, 0),  cube(4, center: true)),
  translate((10, 0, 0), cube(4, center: true)),
  translate((20, 0, 0), cube(4, center: true)),
))
Got #parts.len() parts:
#for p in parts {
  show-part(p, width: 25%)
  h(0.3em)
}
```

`compile-scad-parts(src, files: (:), bin: (:), font: none, fn: 32)` is
the `.scad`-source variant.

#pagebreak(weak: true)

= Direct 2D → SVG: `scadypst-svg` / `compile-scad-svg`

For sources that resolve to a 2D shape (`circle`, `square`, `polygon`,
`text`, hulls of 2D things, `offset`, `slice`, `projection`), the plugin
can emit an SVG document directly — no maquette in the loop, no
rasterizer. The result is resolution-independent, Inkscape-editable,
and laser-cutter-ready.

Errors if the tree resolves to a 3D solid.

```example
// cols: 2 1
#image(scadypst-svg(difference(
  union(
    circle(10, fn: 64),
    translate((14, 0), square(20, center: true)),
  ),
  circle(4, fn: 48),
)), width: 65%)
```

`slice(child, z: 0)` composes cleanly with `-svg`: 3D model → horizontal
cross-section → vector contour, in one call.

```example
// cols: 2 1
#image(scadypst-svg(slice(sphere(10, fn: 64), z: 4)), width: 60%)
```

`compile-scad-svg(src, ...)` is the `.scad`-source variant.

#pagebreak(weak: true)

= Escape hatch: precompiled `.ply`

The wasm plugin has a memory budget of about 1 GB. Very large assemblies can outgrow it. When that happens, compile once natively (e.g. `cargo run --release --example repro -p maquette-scad`) and pass the resulting `.ply` bytes straight to `render-ply`:

```typ
#import "@preview/maquette:0.1.3": render-ply

#let full = read("cyclone-full.ply", encoding: none)
#render-ply(full, camera: (400, 400, 200), up: (0, 0, 1), fov: 40)
```

`examples/scad/cyclone.typ` in the repo shows the full pattern with parameterised camera shots.

#pagebreak(weak: true)

= What `compile-scad` accepts

If your `.scad` source works in the OpenSCAD editor, it should work here — the evaluator covers everything a typical file uses. Known gaps: `surface()` (heightmap import), `import()` of DXF, `$vpr` / `$vpt` / `$vpd` / `$vpf` viewport variables, and adaptive tessellation from `$fa` / `$fs` (we use `$fn` or the per-primitive `fn:` instead). Any external STL / OBJ mesh referenced by `import()` must be routed through the `bin:` argument since the plugin has no filesystem access.

The exhaustive per-feature tracker lives at `crates/maquette-scad/FIDELITY.md`.

*Two wasm modules cooperate.* `maquette-scad.wasm` (this crate) compiles geometry to PLY. `maquette.wasm` renders it. They exchange nothing but the PLY blob, so you can also feed a `scadypst(...)` output into `maquette-gltf` (wrapped in a minimal `.gltf`) when you want PBR shading on procedural geometry.
