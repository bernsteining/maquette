#import "@local/maquette-scad:0.1.0": *
#import "@preview/maquette:0.2.0": render-ply
#import "common.typ": doc, cover, contents, labelled, ref-table, demo-callout, demo-url, cfg-of, pct
#import "@local/maquette-scad:0.1.0" as scad-lib

#let scad-num(x) = str(if type(x) == float { calc.round(x, digits: 5) } else { x }).replace("\u{2212}", "-")
#let scad-val(v) = if type(v) == array { "[" + v.map(scad-val).join(", ") + "]" } else if type(v) == bool {
  if v { "true" } else { "false" }
} else if type(v) == str { "\"" + v + "\"" } else if type(v) == angle { scad-num(v.deg()) } else { scad-num(v) }
#let to-scad(n) = {
  let fa = if "fn" in n { ", $fn = " + str(n.fn) } else { "" }
  let wrap(head, child) = { let c = to-scad(child); if c == none { none } else { head + " " + c } }
  let group(head, children) = {
    let cs = children.map(to-scad)
    if none in cs { none } else { head + " { " + cs.join(" ") + " }" }
  }
  let op = n.op
  if op == "cube" { "cube(" + scad-val(n.size) + ", center = " + scad-val(n.center) + ");" }
  else if op == "sphere" { "sphere(" + scad-num(n.r) + fa + ");" }
  else if op == "cylinder" {
    let rs = if "r" in n { ", r = " + scad-num(n.r) } else { "" }
    rs += if "r1" in n { ", r1 = " + scad-num(n.r1) } else { "" }
    rs += if "r2" in n { ", r2 = " + scad-num(n.r2) } else { "" }
    "cylinder(h = " + scad-num(n.h) + rs + ", center = " + scad-val(n.center) + fa + ");"
  }
  else if op == "polyhedron" { "polyhedron(" + scad-val(n.points) + ", " + scad-val(n.faces) + ");" }
  else if op == "square" { "square(" + scad-val(n.size) + ", center = " + scad-val(n.center) + ");" }
  else if op == "circle" { "circle(" + scad-num(n.r) + fa + ");" }
  else if op == "ngon" { "circle(" + scad-num(n.r) + ", $fn = " + str(n.sides) + ");" }
  else if op == "polygon" { "polygon(" + scad-val(n.points) + if "paths" in n { ", " + scad-val(n.paths) } else { "" } + ");" }
  else if op == "star" {
    let k = 2 * n.points
    "polygon(" + scad-val(range(k).map(i => {
      let r = if calc.even(i) { n.outer } else { n.inner }
      let a = calc.pi * i / n.points
      (r * calc.cos(a), r * calc.sin(a))
    })) + ");"
  }
  else if op == "text" { "text(" + scad-val(n.text) + ", size = " + scad-num(n.size) + ");" }
  else if op == "linear_extrude" {
    let o = "height = " + scad-num(n.h) + ", center = " + scad-val(n.center)
    if "twist" in n { o += ", twist = " + scad-num(n.twist) }
    if "scale" in n { o += ", scale = " + scad-val(n.scale) }
    if "slices" in n { o += ", slices = " + str(n.slices) }
    wrap("linear_extrude(" + o + ")", n.child)
  }
  else if op == "rotate_extrude" { wrap("rotate_extrude(angle = " + scad-num(n.angle) + fa + ")", n.child) }
  else if op == "projection" { wrap("projection()", n.child) }
  else if op == "slice" { wrap("projection(cut = true) translate([0, 0, " + scad-num(-n.z) + "])", n.child) }
  else if op in ("translate", "scale", "mirror", "resize") { wrap(op + "(" + scad-val(n.v) + ")", n.child) }
  else if op == "rotate" { wrap("rotate(" + scad-val(n.deg) + ")", n.child) }
  else if op == "multmatrix" { wrap("multmatrix(" + scad-val(n.m) + ")", n.child) }
  else if op == "offset" { wrap("offset(r = " + scad-num(n.d) + ")", n.child) }
  else if op == "color" {
    wrap("color(" + scad-val(n.rgb) + if "alpha" in n { ", alpha = " + scad-num(n.alpha) } else { "" } + ")", n.child)
  }
  else if op in ("union", "difference", "intersection", "hull", "minkowski") { group(op + "()", n.children) }
  else { none }
}
#let scad-logo = read("openscad-logo.scad")
#let compile-only = ("files", "bin", "font", "fn", "smooth-normals", "width", "height", "format")
#let render-scad(model, ..args) = {
  let out = scad-lib.render-scad(model, ..args)
  let named = args.named()
  let src = if type(model) == str { model } else if type(model) == dictionary { to-scad(model) } else { none }
  if src == none or "files" in named or "bin" in named or "font" in named { return out }
  if "fn" in named { src = "$fn = " + str(named.fn) + ";\n" + src }
  let cfg = (specular: 0, cull_backface: false, scad_smooth_normals: named.at("smooth-normals", default: 30) != none)
  for (k, v) in cfg-of(args) { if k not in compile-only { cfg.insert(k, v) } }
  if src == scad-logo { link(demo-url("__scad__", cfg), out) }
  else { link(demo-url("__scad__", cfg) + "&scad_src=" + pct(src), out) }
}

#let look = (up: (0, 0, 1), azimuth: 35, elevation: 25, projection: "orthographic",
  background: none, ssao: true, zoom: 1.2)

#let scope = (
  render-ply: render-ply, render-scad: render-scad, render-scad-tree: render-scad-tree, image: image, look: look,
  scadypst: scadypst, compile-scad: compile-scad, compile-scad-tree: compile-scad-tree,
  scadypst-svg: scadypst-svg, compile-scad-svg: compile-scad-svg,
  scadypst-info: scadypst-info, compile-scad-info: compile-scad-info,
  scadypst-parts: scadypst-parts, compile-scad-parts: compile-scad-parts,
  scadypst-raycast: scadypst-raycast, compile-scad-raycast: compile-scad-raycast,
  scad-highlighting: scad-highlighting, openscad-view: openscad-view, import-mesh: import-mesh,
  cube: cube, sphere: sphere, cylinder: cylinder, polyhedron: polyhedron,
  square: square, circle: circle, ellipse: ellipse, polygon: polygon,
  ngon: ngon, star: star, rounded-square: rounded-square, scad-text: scad-text,
  linear-extrude: linear-extrude, rotate-extrude: rotate-extrude, projection: projection,
  slice: slice, trim: trim, hull-pts: hull-pts, simplify: simplify, calculate-normals: calculate-normals,
  translate: translate, rotate: rotate, scale: scale, mirror: mirror, multmatrix: multmatrix,
  resize: resize, offset: offset, color: color,
  union: union, difference: difference, intersection: intersection, hull: hull, minkowski: minkowski,
  bunny: read("/examples/data/bunny.obj"),
  font: read("/crates/maquette-scad/assets/DejaVuSans-Subset.ttf", encoding: none),
)
#let sect(body) = block(breakable: false, width: 100%, body)

#let dsl-table(rows) = table(
  columns: (auto, 1fr),
  align: (left + horizon, left + horizon),
  inset: (x: 6pt, y: 3.5pt),
  stroke: none,
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(249) },
  table.header([*Signature*], [*Description*]),
  ..rows.map(((s, d)) => (text(size: 7.5pt, raw(s)), text(size: 8.5pt, d))).flatten(),
)

#show: doc.with(name: "maquette-scad", scope: scope, stacked: true)

#cover(
  [maquette-scad],
  [Render OpenSCAD in Typst],
  box(clip: true, radius: 44pt, image("logo-scad.svg", width: 260pt)),
  (
    ("Source", "https://github.com/bernsteining/maquette"),
    ("Typst Universe", "https://typst.app/universe/package/maquette-scad"),
    ("Live demo", "https://bernsteining.github.io/maquette/?model=openscad-logo.scad"),
  ),
  toml("/crates/maquette-scad/maquette-scad/typst.toml").package.version,
)

#contents()

= Introduction

*maquette-scad* compiles OpenSCAD models and renders them in Typst documents. It runs entirely inside Typst as a WebAssembly plugin; no OpenSCAD installation is required.

#demo-callout("https://bernsteining.github.io/maquette/?model=openscad-logo.scad",
  [Edit OpenSCAD code and render options in the browser, then copy the matching Typst code.])

Models can be provided in two forms:

- *OpenSCAD source*: the contents of a `.scad` file, including the libraries it uses.
- *Typst DSL*: functions such as `cube`, `difference` and `translate`, combined with Typst expressions for procedural geometry.

`render-scad` compiles a model and renders it in a single call. Rendering is performed by maquette; camera, lighting and post-processing options are described in the #link("maquette-documentation.pdf")[maquette manual].

= Quickstart

#sect[
`render-scad` takes OpenSCAD source as its first argument, followed by render options (left). The remaining examples use a shared `look` preset: Z axis up, three-quarter orthographic view, transparent background (right).

#grid(columns: 2, gutter: 1em,
```example
#import "@preview/maquette-scad:0.1.0": render-scad

#render-scad(read("openscad-logo.scad"),
  azimuth: 219, elevation: 33, up: (0, 1, 0),
  zoom: 1.3)
```,
```example
// hl: 1-3
#let look = (up: (0, 0, 1), azimuth: 35, elevation: 25,
  projection: "orthographic", background: none,
  ssao: true, zoom: 1.2)
#render-scad(difference(cube(20, center: true),
  sphere(12.5, fn: 64)), ..look)
```,
)
]

= Config reference

#sect[
All parameters are optional. Default values are shown.

#text(size: 9pt, raw(block: true, lang: "json", "{ // ── Compile options (maquette-scad) ───────────────────────────────
  \"fn\": 32,                // Default $fn segment count; a primitive's own fn: wins
  \"smooth-normals\": 30,    // Crease angle (degrees) for smooth shading; none = faceted
  \"files\": {},             // Sources for include/use: { \"path\": text }
  \"bin\": {},               // Meshes for import(): { \"name\": bytes }
  \"font\": null,            // TTF/OTF bytes for text(); null = built-in DejaVu Sans
  // ── Render options (maquette) ─────────────────────────────────────
  // camera, azimuth, elevation, up, fov, zoom, background, color, shading,
  // lights, shadows, ssao, outline, antialias, …: see the maquette manual.
  // render-scad changes two defaults: specular: 0, cull-backface: false.
}"))
]

= Compile options

#sect[
== `fn`: curve resolution

`fn` sets the default number of segments used to approximate circles and spheres, equivalent to OpenSCAD's `$fn`. A primitive's own `fn` argument takes precedence.

#grid(columns: 3, gutter: 1em,
```example
// hl: 2
#render-scad(sphere(12), ..look,
  fn: 8)
```,
```example
// hl: 2
#render-scad(sphere(12), ..look,
  fn: 36)
```,
```example
// hl: 2
#render-scad(sphere(12), ..look,
  fn: 96)
```,
)
]

#sect[
== `smooth-normals`: shading across facets

`smooth-normals` sets the crease angle, in degrees, used for smooth shading. Edges between faces that meet at a sharper angle stay hard. The default is 30; `none` renders flat facets.

#grid(columns: 2, gutter: 1em,
```example
// hl: 3
#render-scad(difference(cube(18, center: true),
  sphere(11.5, fn: 16)), ..look,
  smooth-normals: none)
```,
```example
// hl: 3
#render-scad(difference(cube(18, center: true),
  sphere(11.5, fn: 16)), ..look,
  smooth-normals: 30)
```,
)
]

#sect[
== `bin` and `files`: meshes and libraries

`bin` provides the STL and OBJ meshes loaded by `import()`; imported meshes can be used in boolean operations (left). `files` provides the sources loaded by `include` and `use` (right). Both are dictionaries keyed by file name.

#grid(columns: 2, gutter: 1em,
```example
// hl: 3, 6
#render-scad(difference(
    scale((300, 300, 300), rotate((90, 0, 0),
      import-mesh("bunny.obj"))),
    translate((20, 0, 30), cube((40, 80, 80), center: true)),
  ),
  bin: ("bunny.obj": bunny), ..look, azimuth: 60, zoom: 0.9)
```,
```example
// hl: 3-4
#render-scad("use <shapes.scad>
  rounded(20, 4);",
  files: ("shapes.scad": "module rounded(s, r) minkowski()
    { cube(s - 2 * r, center = true); sphere(r); }"), ..look)
```,
)
]

#sect[
== `font`: text

`scad-text` generates glyph outlines from the font passed as `font`, a TTF or OTF file read as bytes. DejaVu Sans is used when no font is given. Text outlines are ordinary 2D shapes: they can be extruded, coloured and used in boolean operations.

```example
// hl: 1, 4-5, 8
#let font = read("DejaVuSans.ttf", encoding: none)
#let raised(c, pos, h, body) = color(c, translate(pos, linear-extrude(h, body)))
#render-scad(union(
  difference(color((0.14, 0.38, 0.6), linear-extrude(4, rounded-square(84, 32, 6))),
    translate((10, 5, 3), linear-extrude(2, scad-text("render OpenSCAD", size: 5)))),
  raised((0.97, 0.55, 0.15), (10, 13, 4), 3, scad-text("Typst", size: 13)),
  raised((0.98, 0.85, 0.25), (70, 18, 4), 5, star(5, 7, 3)),
), font: font, (width: 1200, height: 440), antialias: 5,
  ..look, azimuth: 215, elevation: 45, zoom: 1.7)
```
]

= Render options

#sect[
Other arguments are passed to maquette's renderer (left). The `openscad-view` preset reproduces the OpenSCAD preview (right).

#grid(columns: 2, gutter: 1em,
```example
// hl: 3-4
#render-scad(read("openscad-logo.scad"),
  azimuth: 219, elevation: 33, up: (0, 1, 0),
  background: "#202430", ssao: true,
  outline: (width: 1.5, color: "#000000"))
```,
```example
// hl: 2
#render-scad(read("openscad-logo.scad"),
  ..openscad-view,
  azimuth: 219, elevation: 33, up: (0, 1, 0))
```,
)
]

= Building with the DSL

#sect[
== Trees and Typst control flow

DSL functions return Typst values, so models can be built with Typst loops and arithmetic.

#grid(columns: 2, gutter: 1em,
```example
// hl: 1-4
#let ring = union(..for i in range(12) {
  let a = i * 30deg
  (translate((20 * calc.cos(a),
    20 * calc.sin(a), 0), sphere(3)),)
})
#render-scad(ring, ..look)
```,
```example
// hl: 1-3
#let stairs = union(..range(16).map(i =>
  rotate((0, 0, i * 24), translate(
    (4, -1.5, i * 1.6), cube((10, 3, 1))))),
  cylinder(26, r: 2))
#render-scad(stairs, ..look)
```,
)
]

#sect[
== Primitives and extrusions

Primitives, 2D shapes and extrusions follow OpenSCAD's names and parameters.

#grid(columns: 2, gutter: 1em,
```example
#render-scad(union(cube(10),
  translate((18, 5, 5), sphere(6)),
  translate((-12, 5, 0),
    cylinder(14, r1: 6, r2: 2))),
  ..look)
```,
```example
#render-scad(linear-extrude(30,
  star(6, 14, 8), twist: 90,
  slices: 40), ..look)
```,
```example
#render-scad(rotate-extrude(
  translate((12, 0), circle(5)),
  fn: 96), ..look)
```,
```example
#render-scad(projection(sphere(10)) ,
  ..look)
```,
)
]

#sect[
== Booleans and hulls

`difference` subtracts every following child from the first. `hull` computes the convex hull of its children, and `minkowski` their Minkowski sum.

#grid(columns: 2, gutter: 1em,
```example
#render-scad(difference(
  cube(16, center: true),
  sphere(10.5)), ..look)
```,
```example
#render-scad(intersection(
  cube(16, center: true),
  sphere(10.5)), ..look)
```,
```example
#render-scad(hull(cube(16, center: true),
  translate((16, 0, 0), sphere(5))),
  ..look)
```,
```example
#render-scad(minkowski(
  cube(10, center: true),
  sphere(3)), ..look)
```,
)
]

#sect[
== Colours and alpha

`color` takes RGB components in the range 0–1. An optional fourth component sets the opacity (right). Uncoloured parts use OpenSCAD's default yellow.

#grid(columns: 2, gutter: 1em,
```example
// hl: 2, 4
#render-scad(union(
  color((0.85, 0.3, 0.3), cube(16,
    center: true)),
  sphere(6),
  color((0.3, 0.65, 0.4), translate(
    (14, 0, 0), sphere(5))),
), ..look)
```,
```example
// hl: 2
#render-scad(union(
  color((0.85, 0.3, 0.3, 0.45), cube(16,
    center: true)),
  sphere(6),
  color((0.3, 0.65, 0.4), translate(
    (14, 0, 0), sphere(5))),
), ..look)
```,
)
]

#sect[
== Reference

#dsl-table((
  ("cube(size, center: false)", [Box; `size` is a number or `(x, y, z)`.]),
  ("sphere(r, fn: none)", [Sphere of radius `r`.]),
  ("cylinder(h, r:, r1:, r2:, center:, fn:)", [Cylinder, or a cone with `r1` and `r2`.]),
  ("polyhedron(points, faces)", [Raw mesh.]),
  ("square / circle / ellipse / polygon", [2D shapes; `polygon(points, paths:)` takes holes.]),
  ("ngon / star / rounded-square", [Regular polygon, star, rounded rectangle.]),
  ("scad-text(str, size: 10)", [Glyph outlines from the compile's `font`.]),
  ("import-mesh(file)", [An STL or OBJ passed in `bin`.]),
  ("linear-extrude(h, child, twist:, scale:, slices:)", [Extrude 2D to 3D, optionally twisted and tapered.]),
  ("rotate-extrude(child, angle: 360, fn:)", [Revolve a 2D profile around Z.]),
  ("projection(child) / slice(child, z:)", [3D to 2D: shadow, or cross-section at `z`.]),
  ("translate / rotate / scale / mirror / resize", [Transforms; `rotate` takes Euler degrees.]),
  ("multmatrix(m, child) / offset(d, child)", [Affine matrix; grow or shrink a 2D shape.]),
  ("union / difference / intersection", [Booleans; `difference` subtracts the rest from the first.]),
  ("hull / minkowski / hull-pts(points)", [Convex hull, Minkowski sum, hull of points.]),
  ("trim(child, normal, offset:)", [Cut with a plane.]),
  ("simplify(child, epsilon:)", [Vertex reduction (2D and 3D).]),
  ("calculate-normals(child, sharp-angle:)", [Smooth normals for one subtree.]),
  ("color(rgb, child, alpha:)", [Colour and transparency.]),
))

Importing the package with `*` shadows Typst's built-in `scale`, `rotate`, `circle`, `square`, `ellipse`, `polygon` and `color`. Each has a `scad-` prefixed alias, such as `scad-color`, for use alongside the built-ins.
]

= Compiling `.scad` sources

#sect[
`compile-scad` compiles OpenSCAD source to a PLY mesh without rendering it. The mesh can then be rendered several times with maquette's `render-ply` for the cost of a single compilation.

```example
// hl: 3-4
#import "@preview/maquette:0.2.0": render-ply

#let ply = compile-scad(
  read("scad/gear.scad"))
#grid(columns: 2, gutter: 0.5em,
  render-ply(ply, ..look),
  render-ply(ply, ..look, elevation: 80))
```
]

= Syntax highlighting

#sect[
The `scad-highlighting` show rule enables OpenSCAD syntax highlighting in `scad` code blocks.

````examplev
#show: scad-highlighting
```scad
module rod(d, h) cylinder(h, d = d, center = true);
difference() { sphere(d = 50); rod(25, 62.5); }
```
````
]

= Multi-file projects

#sect[
`render-scad-tree` compiles a project from its entry file, resolving `include` and `use` statements relative to `root`. Files are loaded through the `read` function, which must be defined in your document. The project below has three files: a main file, a plate and a bolt.

```example
// hl: 1-3
#render-scad-tree("main.scad", (width: 600, height: 400),
  root: "scad/tree/",
  read: p => read(p),
  ..look, azimuth: 30, elevation: 35)
```
]

= Inspecting geometry

#sect[
== Metrics

`scadypst-info` returns the model's bounding box, volume, surface area and mesh statistics.

```example
#let info = scadypst-info(difference(
  cube(20, center: true), sphere(12.5, fn: 64)))
#table(columns: 2,
  ..("volume", "surface_area", "num_tri", "genus")
    .map(k => (raw(k), str(calc.round(info.at(k), digits: 1))))
    .flatten())
```
]

#sect[
== Parts

`scadypst-parts` splits a model into its disconnected components and returns one mesh per component.

```example
// hl: 1
#let parts = scadypst-parts(union(..range(3).map(i =>
  translate((i * 12, 0, 0), cube(6, center: true)))))
#grid(columns: parts.len(), gutter: 0.5em,
  ..parts.map(p => render-ply(p, ..look)))
```
]

#sect[
== Ray casting

`scadypst-raycast` intersects the segment from `origin` to `end` with the model and returns the distance and position of each hit.

```example
#let hits = scadypst-raycast(union(cube(10, center: true),
  translate((0, 0, 12), sphere(5))), (0, 0, 30), (0, 0, -30))
#table(columns: 2, [*distance*], [*z*],
  ..hits.map(h => (str(calc.round(h.distance, digits: 2)),
    str(calc.round(h.position.at(2), digits: 2)))).flatten())
```
]

#sect[
== Simplifying and smoothing

`simplify` removes detail smaller than `epsilon` (left). `calculate-normals` applies smooth shading to a single subtree (right).

#grid(columns: 2, gutter: 1em,
```example
// hl: 2
#render-scad(simplify(sphere(12, fn: 64),
  epsilon: 2), ..look, smooth-normals: none)
```,
```example
// hl: 2
#render-scad(union(calculate-normals(
  sphere(9, fn: 24), sharp-angle: 30),
  translate((15, 0, 0), sphere(6, fn: 24))),
  ..look, smooth-normals: none)
```,
)
]

= 2D output as SVG

#sect[
`scadypst-svg` exports 2D shapes as SVG, which stays sharp at any scale. Combined with `slice`, it produces vector cross-sections of solids.

#grid(columns: 2, gutter: 1em,
```example
#image(scadypst-svg(difference(
  union(circle(10, fn: 64),
    translate((14, 0), square(20,
      center: true))),
  circle(4, fn: 48))), width: 90%)
```,
```example
// hl: 1
#image(scadypst-svg(slice(
  sphere(10, fn: 64), z: 4)),
  width: 60%)
```,
)
]

= A whole machine

#sect[
The #link("https://github.com/carlosgs/Cyclone-PCB-Factory", text(fill: blue)[Cyclone PCB Factory]) is an open-source, 3D-printable CNC mill. Its source tree contains 66 `.scad` files, including the MCAD, obiscad and Write libraries. `render-scad-tree` compiles the complete assembly in about two minutes; a low `fn` reduces compilation time.

```example
// hl: 1-3
#render-scad-tree("Cyclone.scad",
  root: "scad/cyclone/", read: p => read(p), fn: 8,
  (width: 1000, height: 900), ..look, azimuth: 215, zoom: 1.3)
```
]

= Credits

#sect[
#let src(url, body) = link(url, text(fill: blue, body))
#ref-table(([Model], [Authors], [Licence]), (
  (src("https://github.com/carlosgs/Cyclone-PCB-Factory")[Cyclone PCB Factory], [Carlosgs and contributors],
    src("https://creativecommons.org/licenses/by-sa/4.0/")[CC BY-SA 4.0]),
  (src("https://github.com/openscad/MCAD")[MCAD], [OpenSCAD contributors],
    src("https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html")[LGPL 2.1]),
  (src("https://github.com/Obijuan/obiscad")[obiscad], [Juan González-Gómez (Obijuan)],
    src("https://www.gnu.org/licenses/gpl-3.0.html")[GPL]),
  (src("https://www.thingiverse.com/thing:16193")[Write.scad], [HarlanDMii],
    src("https://creativecommons.org/licenses/by/3.0/")[CC BY 3.0]),
  (src("https://graphics.stanford.edu/data/3Dscanrep/")[Stanford Bunny], [Stanford Computer Graphics Laboratory], [Free for research]),
))
]
