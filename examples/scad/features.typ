// ── maquette-scad — OpenSCAD feature tour (bilingual) ────────────────────────
// Each feature is shown in BOTH input languages, labelled by zebraw:
//   • OpenSCAD text  → compiled with `compile-scad`
//   • Typst DSL      → composed with helper functions + `scadypst`
// The render beside them is produced from the OpenSCAD source; the Typst DSL
// block is the exact equivalent (same result).
//
// NOTE ON IMPORTS: the snippets show `@preview/...` imports as you'd write them
// once the packages are on Typst Universe. This document itself imports by local
// path (below) so it compiles here today (maquette-scad isn't published yet).

#import "@preview/zebraw:0.5.5": zebraw
#import "../../scad/maquette-scad.typ": *          // scadypst, compile-scad, scad-highlighting, cube, …
#import "../../maquette/maquette.typ": render-ply

#show: scad-highlighting
#set page(width: 940pt, height: 1200pt, margin: 32pt, fill: white)
#set text(font: "DejaVu Sans", size: 12pt)
#set heading(numbering: none)

#let os-block(src) = zebraw(lang: [OpenSCAD], numbering: false, raw(src, lang: "scad", block: true))
#let dsl-block(src) = zebraw(lang: [Typst DSL], numbering: false, raw(src, lang: "typc", block: true))

#let feat(title, os, dsl, up: (0, 0, 1), az: 30, el: 25, bin: (:), render: true) = block(
  breakable: false,
  above: 18pt,
  {
    text(weight: "bold", size: 13pt, title)
    v(3pt)
    grid(
      columns: (1fr, 300pt),
      column-gutter: 22pt,
      align: (top, horizon),
      stack(spacing: 7pt, os-block(os), dsl-block(dsl)),
      if render {
        render-ply(compile-scad(os, bin: bin), width: 290pt, up: up, azimuth: az, elevation: el, ..openscad-view)
      } else {
        align(center + horizon, text(fill: luma(140), style: "italic")[needs a font — \ see caption])
      },
    )
  },
)

= maquette-scad — OpenSCAD feature tour

Two ways in, one compiler. Every feature below appears as *OpenSCAD source* and
as the equivalent *Typst DSL*; both compile through the same pipeline to the mesh
shown on the right.

== Setup — importing the packages
As published on Typst Universe you'd import them like any package. Pick a
front-end: `compile-scad` for real `.scad` source, or the DSL helpers + `scadypst`.

#zebraw(lang: [Typst], numbering: false, raw(
"// A) run real OpenSCAD source
#import \"@preview/maquette:0.1.3\": render-ply
#import \"@preview/maquette-scad:0.1.0\": compile-scad

#render-ply(compile-scad(read(\"part.scad\")))", lang: "typ", block: true))

#zebraw(lang: [Typst], numbering: false, raw(
"// B) compose with the Typst DSL
#import \"@preview/maquette:0.1.3\": render-ply
#import \"@preview/maquette-scad:0.1.0\": *

#render-ply(scadypst(difference(
  cube(20, center: true),
  sphere(12, fn: 48),
)))", lang: "typ", block: true))

= Primitives & shapes

#feat("3D primitives — cube · sphere · cone",
"$fn = 48;
cube([20, 12, 8], center = true);
translate([26, 0, 0]) sphere(8);
translate([-26, 0, 0]) cylinder(h = 16, r1 = 8, r2 = 2);",
"union(
  cube((20, 12, 8), center: true),
  translate((26, 0, 0), sphere(8, fn: 48)),
  translate((-26, 0, 0), cylinder(16, r1: 8, r2: 2, fn: 48)),
)", el: 22)

#feat("2D shapes, extruded",
"$fn = 48;
linear_extrude(4) square([16, 10], center = true);
translate([26, 0, 0]) linear_extrude(4) circle(7);",
"union(
  linear-extrude(4, square((16, 10), center: true)),
  translate((26, 0, 0), linear-extrude(4, circle(7, fn: 48))),
)", el: 55)

= Booleans & combinations

#feat("union · difference · intersection",
"$fn = 40;
difference() {
  intersection() { cube(16, center = true); sphere(10); }
  cylinder(h = 40, r = 4, center = true);
}",
"difference(
  intersection(cube(16, center: true), sphere(10, fn: 40)),
  cylinder(40, r: 4, center: true, fn: 40),
)")

#feat("hull",
"$fn = 24;
hull() {
  translate([-10, 0, 0]) sphere(3);
  translate([10, 0, 0]) cylinder(h = 2, r = 5);
}",
"hull(
  translate((-10, 0, 0), sphere(3, fn: 24)),
  translate((10, 0, 0), cylinder(2, r: 5, fn: 24)),
)")

#feat("minkowski (rounded box)",
"$fn = 16;
minkowski() { cube([16, 10, 6], center = true); sphere(2.5); }",
"minkowski(
  cube((16, 10, 6), center: true),
  sphere(2.5, fn: 16),
)")

= Transforms

#feat("translate · rotate · scale · color",
"color([0.3, 0.6, 0.9])
  rotate([0, 0, 25]) scale([1, 1, 2])
    cube([12, 12, 6], center = true);",
"color((0.3, 0.6, 0.9),
  rotate((0, 0, 25), scale((1, 1, 2),
    cube((12, 12, 6), center: true))))")

= Extrusion

#feat("linear_extrude with twist",
"$fn = 48;
linear_extrude(height = 30, twist = 180, slices = 40)
  square([12, 12], center = true);",
"linear-extrude(30, twist: 180, slices: 40,
  square((12, 12), center: true))", el: 20)

#feat("rotate_extrude (revolution)",
"$fn = 60;
rotate_extrude() translate([11, 0]) circle(3.5);",
"rotate-extrude(
  translate((11, 0), circle(3.5, fn: 24)), fn: 60)", up: (0, 1, 0), el: 18)

#feat("projection (3D → 2D)",
"$fn = 32;
projection()
  rotate([0, 0, 30]) union() {
    cube([22, 8, 8], center = true);
    cube([8, 22, 8], center = true);
  }",
"projection(rotate((0, 0, 30), union(
  cube((22, 8, 8), center: true),
  cube((8, 22, 8), center: true))))", el: 70)

#feat("offset (grow a 2D shape)",
"$fn = 48;
linear_extrude(4) offset(r = 3) square([14, 9], center = true);",
"linear-extrude(4,
  offset(3, square((14, 9), center: true)))")

= Control flow — where the languages differ most

#feat("for — OpenSCAD's for vs Typst's own",
"$fn = 20;
for (i = [0:5])
  rotate([0, 0, i * 60])
    translate([13, 0, 0]) cylinder(h = 8, r = 2);
cylinder(h = 3, r = 15, $fn = 48);",
"// the DSL uses Typst's for/range/map
union(
  ..range(6).map(i => rotate((0, 0, i * 60),
    translate((13, 0, 0), cylinder(8, r: 2, fn: 20)))),
  cylinder(3, r: 15, fn: 48),
)", el: 40)

#feat("module with children() vs a Typst function",
"module ring(n = 8, r = 12) {
  for (i = [0:n-1]) rotate([0, 0, i * 360 / n])
    translate([r, 0, 0]) children();
}
ring(8) cylinder(h = 10, r = 2.5, $fn = 16);",
"// children() -> a function taking the child node
let ring(n, r, child) = union(
  ..range(n).map(i => rotate((0, 0, i * 360 / n),
    translate((r, 0, 0), child))))
ring(8, 12, cylinder(10, r: 2.5, fn: 16))", el: 32)

#feat("list comprehension vs Typst map + calc",
"$fn = 64;
n = 120;
points = [ for (i = [0:n-1])
  let (a = i*360/n, r = 12 + 3*cos(6*a))
    [r*cos(a), r*sin(a)] ];
linear_extrude(5) polygon(points);",
"let pts = range(120).map(i => {
  let a = i * 360deg / 120
  let r = 12 + 3 * calc.cos(6 * a)
  (r * calc.cos(a), r * calc.sin(a))
})
linear-extrude(5, polygon(pts))", el: 60)

#feat("recursion — module vs function",
"module tower(n, s) {
  if (n > 0) {
    cube([s, s, 4], center = true);
    translate([0, 0, 4]) rotate([0, 0, 25])
      tower(n - 1, s * 0.8);
  }
}
tower(7, 14);",
"let tower(n, s) = if n <= 0 { union() } else {
  union(cube((s, s, 4), center: true),
    translate((0, 0, 4), rotate((0, 0, 25),
      tower(n - 1, s * 0.8))))
}
tower(7, 14)", el: 18)

= Data & external assets

#feat("polygon with holes",
"linear_extrude(4) polygon(
  points = [[0,0],[22,0],[22,22],[0,22], [7,7],[15,7],[11,16]],
  paths  = [[0,1,2,3], [4,5,6]]);",
"linear-extrude(4, polygon(
  ((0,0),(22,0),(22,22),(0,22), (7,7),(15,7),(11,16)),
  paths: ((0,1,2,3), (4,5,6))))", el: 60)

#feat("import an external mesh",
"difference() {
  scale([12, 12, 12]) import(\"cube.stl\");
  translate([6, 6, -2]) cylinder(h = 20, r = 3.5, $fn = 40);
}",
"difference(
  scale((12,12,12), import-mesh(\"cube.stl\")),
  translate((6,6,-2), cylinder(20, r: 3.5, fn: 40)),
)
// bytes: scadypst(node, bin: (\"cube.stl\": read(..)))",
bin: ("cube.stl": read("../data/cube.stl", encoding: none)), el: 26)

#feat("text (needs a font)",
"linear_extrude(3) text(\"maquette\", size = 10);",
"linear-extrude(3, scad-text(\"maquette\", size: 10))
// font: scadypst(node, font: read(\"X.ttf\", encoding: none))",
render: false)

The `text` result isn't rendered here because it needs a font — supply one with
`compile-scad(src, font: read(\"Font.ttf\", encoding: none))` (or `scadypst(node,
font: ..)` for the DSL).
