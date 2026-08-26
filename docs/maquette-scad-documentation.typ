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
  render-ply: render-ply, show-part: show-part,
  scadypst: scadypst, compile-scad: compile-scad,
  cube: cube, sphere: sphere, cylinder: cylinder, polyhedron: polyhedron,
  square: square, circle: circle, ellipse: ellipse, polygon: polygon,
  ngon: ngon, star: star, rounded-square: rounded-square,
  linear-extrude: linear-extrude, rotate-extrude: rotate-extrude, projection: projection,
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

*maquette-scad* takes an OpenSCAD-flavored expression (or a `.scad` source string), compiles it to a watertight mesh via the *Manifold* CSG kernel, and hands the result to `maquette` for rendering — all inside Typst's wasm sandbox. No external OpenSCAD install, no shell-out, no intermediate `.stl` on disk.

Two entry points:
- *Typst DSL* — write geometry as Typst expressions using `cube`, `sphere`, `translate`, `difference` etc. Terse, composable, uses Typst's own `for` / `range` / `calc` for procedural placement.
- *`.scad` ingestion* — hand in the text of an existing `.scad` file (`read("part.scad")`). Full OpenSCAD language surface: `function`, `module`, `for`, list comprehensions, `include` / `use`, `$fn` / `$fa` / `$fs`.

Both entry points return a `bytes` blob of PLY that plugs straight into `maquette`'s `render-ply`. The CSG kernel guarantees watertight boolean output — none of the "20% of faces open on any non-trivial cut" issue you get from BSP-based kernels.

See `crates/maquette-scad/FIDELITY.md` for the exhaustive OpenSCAD language coverage tracker.

#pagebreak(weak: true)

= Quickstart

```example
// cols: 2 1
#let part = scadypst(
  difference(
    cube(20, center: true),
    sphere(12, fn: 48),
  )
)
#show-part(part)
```

Two things to notice:
- `cube` / `sphere` / `difference` build an expression tree of dicts — nothing runs yet.
- `scadypst(tree)` sends the tree to the wasm compiler, which returns PLY bytes.
- `show-part` in this doc is a thin wrapper around `#render-ply(bytes, ..config)` — you'd inline the render config yourself in real use.

#pagebreak(weak: true)

= Primitives

== 3D

```example
#show-part(scadypst(cube(20, center: true)))
```

```example
#show-part(scadypst(sphere(15, fn: 48)))
```

```example
#show-part(scadypst(cylinder(30, r: 10, center: true, fn: 48)))
```

Cone / frustum: pass `r1` and `r2` instead of `r`.

```example
#show-part(scadypst(cylinder(30, r1: 15, r2: 5, center: true, fn: 48)))
```

== 2D → 3D

`square` / `circle` / `polygon` on their own return a 2D shape. `linear-extrude` and `rotate-extrude` lift them into 3D.

```example
#show-part(scadypst(linear-extrude(20, star(5, 15, 6))))
```

`linear-extrude` supports a twist (degrees over the full height) and a taper `scale`:

```example
#show-part(scadypst(linear-extrude(30, square(15, center: true), twist: 90, scale: 0.5)))
```

`rotate-extrude` revolves a 2D profile (living in the +x half-plane) around the z-axis. Classic torus:

```example
#show-part(scadypst(rotate-extrude(translate((15, 0, 0), circle(5, fn: 32)), fn: 64)))
```

#pagebreak(weak: true)

= Boolean operations

`union`, `difference`, `intersection` — variadic, take any number of children. `difference` is first-minus-the-rest.

```example
#show-part(scadypst(
  difference(
    cube(20, center: true),
    sphere(13, fn: 48),
  )
))
```

```example
#show-part(scadypst(
  intersection(
    cube(20, center: true),
    sphere(13, fn: 48),
  )
))
```

```example
#show-part(scadypst(
  union(
    cube(20, center: true),
    translate((0, 0, 15), sphere(8, fn: 32)),
  )
))
```

`hull` (convex hull of the union) and `minkowski` (Minkowski sum) are 3D-only.

```example
#show-part(scadypst(
  hull(
    translate((-15, 0, 0), sphere(6, fn: 24)),
    translate(( 15, 0, 0), sphere(6, fn: 24)),
  )
))
```

#pagebreak(weak: true)

= Transforms

`translate` / `rotate` / `scale` / `mirror` / `resize` all take a child at the end.

```example
#show-part(scadypst(
  union(
    cube(10, center: true),
    translate((15, 0, 0), rotate((0, 45, 0), cube(10, center: true))),
    translate((30, 0, 0), scale((1.5, 0.5, 1), cube(10, center: true))),
  )
))
```

`mirror` reflects across a plane whose normal is the vector you pass.

```example
#show-part(scadypst(
  union(
    translate((-8, 0, 0), cylinder(20, r1: 8, r2: 2, fn: 32)),
    mirror((1, 0, 0), translate((-8, 0, 0), cylinder(20, r1: 8, r2: 2, fn: 32))),
  )
))
```

`color` propagates through boolean ops; the emitted PLY carries per-face RGB, which `render-ply` picks up automatically:

```example
#show-part(
  scadypst(union(
    color((0.9, 0.3, 0.3), cube(10, center: true)),
    color((0.3, 0.7, 0.4), translate((15, 0, 0), sphere(6, fn: 32))),
  )),
  color: none,
)
```

#pagebreak(weak: true)

= Procedural placement (Typst `for`)

The DSL sits in Typst. You don't need OpenSCAD's `for()` — use Typst's own iteration.

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

Nested loops for a grid:

```example
// cols: 2 1
#let grid = union(..for x in range(-2, 3) {
  for y in range(-2, 3) {
    (translate(
      (x * 8, y * 8, 0),
      cube(6, center: true),
    ),)
  }
})
#show-part(scadypst(grid))
```

#pagebreak(weak: true)

= 2D primitives

For extrusion sources. Render one by extruding it a tiny amount so we can see the outline:

```example
#show-part(scadypst(linear-extrude(1, star(6, 15, 6))))
```

```example
#show-part(scadypst(linear-extrude(1, ngon(6, 10))))
```

```example
#show-part(scadypst(linear-extrude(1, rounded-square(20, 12, 3, fn: 24))))
```

`polygon` — arbitrary 2D shapes from `(x, y)` point lists. `paths` (optional) is a list of index-rings: first ring is the outer boundary, rest are holes.

```example
#show-part(scadypst(linear-extrude(1,
  polygon(((0, 0), (10, 0), (10, 10), (5, 15), (0, 10))),
)))
```

= Offset

`offset(d, shape2d)` grows (`d > 0`) or shrinks (`d < 0`) a 2D shape by `d` units. Useful for creating outlines / clearance.

```example
#show-part(scadypst(linear-extrude(1,
  offset(2, square(10, center: true)),
)))
```

#pagebreak(weak: true)

= Compiling an existing `.scad` file

`compile-scad(source)` takes real OpenSCAD source (parsed via `openscad-rs`, evaluated in-crate). Full language coverage per `FIDELITY.md`.

```typ
#import "@preview/maquette-scad:0.1.0": compile-scad
#import "@preview/maquette:0.1.3": render-ply

#let part = compile-scad(read("part.scad"))
#render-ply(part)
```

For `.scad` files that `use` or `include` other `.scad` files, or that import an STL/OBJ mesh, supply the sidecar bytes through the `files` / `bin` / `font` params:

```typ
#compile-scad(read("main.scad"),
  files: (
    "utils.scad": read("utils.scad"),
    "gears.scad": read("gears.scad"),
  ),
  bin: (
    "mount.stl": read("mount.stl", encoding: none),
  ),
  font: read("logo.ttf", encoding: none),
  fn: 48,
)
```

The `fn` argument sets a default `$fn` for the compile (overridable per-primitive via `fn:` on `cube` / `sphere` / etc.). `trace: some-path-string` dumps the evaluator trace to the given path — useful when a nested `for` or `module` isn't producing what you expected.

#pagebreak(weak: true)

= Language coverage

The `.scad` evaluator is complete for everything a typical file uses. TL;DR:

*Supported (✅)* — comments, numbers, bool, string, `undef`, vectors, ranges, variable assignment (last-wins scope), `let()` expression, member access, indexing, all standard operators, `$fn` / `$t` / `$preview` / `$children`, all 2D + 3D primitives, all transforms (`translate`, `rotate`, `scale`, `mirror`, `resize`, `multmatrix`, `color`, `offset`), all booleans (`union`, `difference`, `intersection`), extrusions (`linear_extrude` with twist / scale / slices, `rotate_extrude`), `hull`, `minkowski`, `projection`, list comprehensions, `for`, `if`, `intersection_for`, `function`, `module`, `render`, string ops, math functions, `include` + `use`, `color()` with per-face RGB propagation.

*Partial (🟡)* — `$fa` / `$fs` are defined so libraries reading them work, but our primitive builder tessellates from `$fn` (or `fn:` on the primitive), not adaptively.

*Not supported (❌)* — `import()` of external STL/OBJ/DXF (wasm sandbox has no filesystem — pass bytes via `bin:` instead), `surface()` heightmap import, `$vpr` / `$vpt` / `$vpd` / `$vpf` viewport variables (n/a — no viewport at eval time), font-dependent text glyphs beyond basic Latin-1 skeletons.

See `crates/maquette-scad/FIDELITY.md` for the exhaustive per-feature tracker.

#pagebreak(weak: true)

= Design notes

*Why Manifold?* — the previous kernel we used (`csgrs` 0.20.1, BSP-based) left about 20% of faces open on any non-trivial boolean (bores, notches, partial overlaps). Manifold guarantees watertight, correctly-triangulated output. Every mesh this crate emits has zero open edges, verified on the full test corpus.

*Why compile in-crate?* — a `.scad`-then-shell-out design would need a filesystem, a working OpenSCAD binary on the host, and IPC across a process boundary. The wasm plugin has none of that. Ingesting `.scad` text ourselves keeps the whole compile deterministic under Typst's sandbox — same input, same PDF, cross-platform.

*Two wasm modules.* — `maquette-scad.wasm` (this crate) compiles geometry to PLY. `maquette.wasm` (the sibling plugin) renders the PLY. They only exchange the PLY blob; you can also send `scadypst(...)`'s output to `maquette-gltf` via a `.gltf` wrapper if you want PBR shading on procedural geometry.
