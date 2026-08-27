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

*maquette-scad* is a Typst plugin that turns OpenSCAD-flavored geometry into meshes for #link("maquette-documentation.pdf")[`maquette`] to render. Two entry points, both returning PLY bytes you feed to `render-ply`:

- *`scadypst(tree)`* — build geometry with Typst expressions (`cube`, `sphere`, `difference`, `translate`, …). The tree is a plain dict of dicts you compose with Typst's own `for` / `range` / `calc`.
- *`compile-scad(source, files?, bin?, font?, fn?, trace?)`* — hand in the text of an existing `.scad` file. Full OpenSCAD language surface: `function`, `module`, `for`, list comprehensions, `include` / `use`, `$fn` / `$fa` / `$fs`.

The CSG kernel is #link("https://github.com/elalish/manifold")[Manifold] — every boolean is guaranteed watertight, no BSP-style open faces on non-trivial cuts.

This doc covers the maquette-scad *API*: the Typst-side entry points, their options, and the patterns for using them. It does *not* teach OpenSCAD — for that see the #link("https://openscad.org/documentation.html")[OpenSCAD Users Manual] (language) or `crates/maquette-scad/maquette-scad/maquette-scad.typ` (per-function docstrings for every DSL helper). Once you have PLY bytes, downstream rendering (camera, lighting, materials, shadows, tone mapping) is maquette's job — see #link("maquette-documentation.pdf")[maquette's documentation]. See `crates/maquette-scad/FIDELITY.md` for the exhaustive OpenSCAD language coverage tracker.

#pagebreak(weak: true)

= Where to find sample `.scad` files

The DSL examples below run inline — no external files needed. To exercise the `.scad` ingestion path (`compile-scad(read(...))`), grab source from:

- #link("https://github.com/openscad/openscad/tree/master/examples")[openscad/openscad `examples/`] — the official example set that ships with the OpenSCAD editor.
- #link("https://github.com/BelfrySCAD/BOSL2")[BOSL2] — comprehensive OpenSCAD utility library with hundreds of documented example fragments.
- #link("https://www.thingiverse.com/tag:openscad")[Thingiverse (openscad tag)] — large community archive; many things ship the `.scad` source alongside the `.stl`.

= Quickstart

Two entry points, one plugin. Route the returned PLY to `render-ply` with whatever config you'd give any other mesh.

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

`compile-scad` is the same shape but takes source text:

```typ
#import "@preview/maquette-scad:0.1.0": compile-scad
#import "@preview/maquette:0.1.3": render-ply

#let part = compile-scad(read("model.scad"))
#render-ply(part)
```

Below, `show-part` is a thin wrapper this doc uses so examples can focus on the geometry without repeating render config. In real use you'd inline the `render-ply(bytes, ..)` call.

#pagebreak(weak: true)

= Building geometry from Typst — `scadypst`

`scadypst(tree)` compiles a tree of Typst dicts into PLY bytes. The dicts come from helpers imported from the plugin (`cube` / `sphere` / `translate` / `difference` / `hull` / `linear-extrude` / …); each helper *builds* a node in the tree, nothing runs until `scadypst` sees the whole thing.

The API mirrors OpenSCAD's own — cube, sphere, cylinder, polygon, extrusions, booleans, transforms — with Typst-native call syntax (`cube(20, center: true)` instead of `cube(20, center=true);`). For the full list see the per-function docstrings in `crates/maquette-scad/maquette-scad/maquette-scad.typ`. This doc doesn't re-document them — if you know OpenSCAD you already know the names.

The reason to reach for `scadypst` over `compile-scad(read("part.scad"))` is Typst integration: because the tree is Typst code, you get Typst's own procedural facilities for free. Here a `..for` spread inside `union` places twelve spheres on a ring:

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

The equivalent OpenSCAD `for (i = [0:11]) { ... }` works too via `compile-scad` — but a new-in-Typst design will usually reach for Typst's iteration first, since it composes cleanly with variadic ops (`union` / `difference` / `hull` / `intersection`) via the `..` spread syntax and reads with the rest of your document code.

Colours propagate through boolean ops; the emitted PLY carries per-face RGB and `render-ply` picks it up automatically:

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

= Compiling `.scad` sources — `compile-scad`

For standalone `.scad` files, `compile-scad(source_text)` is all you need:

```typ
#let part = compile-scad(read("part.scad"))
#render-ply(part)
```

Real-world `.scad` files rarely stand alone — they `use` / `include` a library, `import` an STL/OBJ for a fastener, or ship a font for embossed text. The wasm sandbox has no filesystem, so sidecars are passed explicitly as bytes:

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

- *`source`* (positional string) — the `.scad` text. `read("path.scad")` is the usual way to get it.
- *`files`* (dict, optional) — every `use <name>` / `include <name>` resolves against this dict. Keys are the exact names the `.scad` uses; nested paths (`MCAD/foo.scad`) are dict keys with slashes.
- *`bin`* (dict, optional) — sidecar mesh files referenced by `import()` in the source. Keys match the `import` argument. Decoded formats: STL and OBJ (DXF, 3MF and AMF are not).
- *`font`* (bytes, optional) — a TTF/OTF file for `text()` glyph rendering. One font per compile.
- *`fn`* (int, optional) — default `$fn` for the whole compile. Per-primitive `fn:` overrides.
- *`trace`* (string, optional) — dump the evaluator trace to this path. Useful when a nested `for` or `module` isn't producing what you expected.

#pagebreak(weak: true)

= Real-world showcase — Cyclone-PCB-Factory

For `.scad` projects big enough that the wasm plugin's ~1 GB memory budget gets in the way, precompile natively and hand `render-ply` a `.ply` blob directly. `examples/scad/cyclone.typ` in the repo shows the pattern.

The #link("https://github.com/carlosgs/Cyclone-PCB-Factory")[Cyclone PCB Factory] CNC mill — a full assembly with MCAD, obiscad, and standard_parts under `include <Cyclone.scad>` — compiles to an ~11 MB mesh. Too big for the wasm plugin, so it's compiled once by the native harness at `crates/maquette-scad/examples/repro.rs` into a `.ply`, then rendered by maquette:

```typ
#import "@preview/maquette:0.1.3": render-ply

#let full = read("cyclone-full.ply", encoding: none)
#let openscad-view = (camera: (400, 400, 200), up: (0, 0, 1), fov: 40)
#let shot = (az, el) => render-ply(full,
  openscad-view + (azimuth: az, elevation: el, antialias: 2, zoom: 1.4),
  width: 100%,
)
#shot(35, 20)
#shot(125, 28)
```

The Manifold kernel keeps the whole assembly watertight (0 open edges) — every part renders solid from every angle even where several dozen booleans stack across the MCAD library. To reproduce: obtain `Source_files/` from the upstream repo, drop it into `examples/scad/cyclone-src/`, then

```
cargo run --release --example repro -p maquette-scad
```

which produces `cyclone-full.ply` alongside the `.typ`.

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
