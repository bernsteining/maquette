#import "../../scad/maquette-scad.typ": *
#import "../../maquette/maquette.typ": render-ply

// A gallery of procedurally-built models, each compiled through the maquette-scad
// plugin (OpenSCAD-flavored DSL -> csgrs CSG -> PLY) and rendered by maquette.

#set page(width: 1180pt, height: auto, margin: 24pt, fill: white)
#set text(font: "DejaVu Sans", size: 13pt)

// ---- models ----

// 1. The iconic OpenSCAD CSG example: (cube ∩ sphere) − three axis cylinders.
#let csg = color((0.30, 0.55, 0.85), difference(
  intersection(cube(30, center: true), sphere(20, fn: 44)),
  cylinder(40, r: 10, center: true, fn: 44),
  rotate((90, 0, 0), cylinder(40, r: 10, center: true, fn: 44)),
  rotate((0, 90, 0), cylinder(40, r: 10, center: true, fn: 44)),
))

// 2. Rounded box: minkowski sum of a cube and a small sphere.
#let rounded = color((0.85, 0.55, 0.30),
  minkowski(cube(18, center: true), sphere(4, fn: 14)))

// 3. Organic blob: convex hull of four spheres.
#let blob = color((0.55, 0.80, 0.45), hull(
  translate((-10, 0, 0), sphere(5, fn: 20)),
  translate((10, 0, 0), sphere(4, fn: 20)),
  translate((0, 12, 2), sphere(6, fn: 20)),
  translate((2, 2, 12), sphere(3, fn: 20)),
))

// 4. Vase: revolve a silhouette polygon around the axis (rotate_extrude).
// Profile endpoints sit on the axis (x=0); the plugin auto-nudges on-axis
// vertices off the axis so csgrs' revolve doesn't panic.
#let vase = color((0.80, 0.35, 0.55), rotate-extrude(
  polygon(((0, 0), (9, 0), (7, 5), (10, 14), (6, 24), (8, 30), (0, 30))),
  fn: 60,
))

// 5. Torus: revolve an off-axis circle.
#let torus = color((0.60, 0.45, 0.85),
  rotate-extrude(translate((11, 0), circle(3.2, fn: 24)), fn: 60))

// 6. Star prism: linear-extrude of a 2D star.
#let starprism = color((0.90, 0.75, 0.25),
  linear-extrude(8, star(7, 14, 6), center: true))

// 7. Pyramid: raw polyhedron (square base + apex).
#let pyramid = color((0.50, 0.70, 0.80), polyhedron(
  ((-8, -8, 0), (8, -8, 0), (8, 8, 0), (-8, 8, 0), (0, 0, 16)),
  ((3, 2, 1, 0), (0, 1, 4), (1, 2, 4), (2, 3, 4), (3, 0, 4)),
))

// 8. Procedural bolt circle: a disk with a ring of posts, placed via Typst's own
//    for-loop + calc (the "language" layer OpenSCAD would need $for/modules for).
#let bolts = {
  let n = 8
  let posts = ()
  for i in range(n) {
    let a = 360deg / n * i
    posts.push(translate((16 * calc.cos(a), 16 * calc.sin(a), 0), cylinder(11, r: 2.6, fn: 18)))
  }
  color((0.70, 0.70, 0.72), union(cylinder(3, r: 22, fn: 56), ..posts))
}

// ---- render helper ----
#let cell(title, model, up: (0, 0, 1), az: 35, el: 25, fnn: 32) = {
  stack(spacing: 8pt,
    render-ply(scadypst(model, fn: fnn), width: 260pt, up: up, azimuth: az, elevation: el),
    align(center, text(weight: "bold", title)),
  )
}

#grid(
  columns: (1fr, 1fr, 1fr, 1fr),
  row-gutter: 24pt, column-gutter: 12pt,
  cell("intersection − cylinders", csg),
  cell("minkowski (rounded box)", rounded),
  cell("hull of spheres", blob),
  cell("rotate_extrude (vase)", vase, up: (0, 1, 0), el: 12),
  cell("rotate_extrude (torus)", torus, up: (0, 1, 0), el: 35),
  cell("linear_extrude (star)", starprism),
  cell("polyhedron (pyramid)", pyramid),
  cell("procedural (Typst loop)", bolts, el: 40),
)
