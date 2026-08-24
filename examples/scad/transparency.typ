#import "../../scad/maquette-scad.typ": *
#import "../../maquette/maquette.typ": render-ply

// Transparency, two ways:
//   1. PER-PART, from the MESH — `color(.., alpha: a)` / `color([r,g,b,a])` /
//      the `%` modifier emit a per-face PLY alpha channel that maquette blends.
//      Only the translucent part is see-through; other parts stay solid.
//   2. WHOLE-MODEL, from the RENDER — pass `opacity:` to `render-ply`; the
//      entire model goes translucent (like an x-ray).

#set page(width: 1180pt, height: auto, margin: 24pt, fill: white)
#set text(font: "DejaVu Sans", size: 13pt)

#let cell(title, body) = stack(spacing: 8pt, body, align(center, text(weight: "bold", title)))

// 1. Per-part: opaque core inside a translucent hollow shell.
#let core-in-shell = openscad(union(
  color((0.90, 0.30, 0.30), sphere(6, fn: 32)),
  color((0.35, 0.55, 0.95), difference(cube(22, center: true), cube(17, center: true)), alpha: 0.28),
))

// 2. The `%` ghost modifier in real .scad (translucent gray preview).
#let ghost = openscad-text("
  $fn = 40;
  difference() { cube(20, center=true); cylinder(h=30, r=6, center=true); }
  %cylinder(h=26, r=6, center=true);
")

// 3. Whole-model opacity via the render call (config-driven x-ray).
#let csg = openscad(difference(
  intersection(cube(30, center: true), sphere(20, fn: 40)),
  cylinder(40, r: 10, center: true, fn: 40),
  rotate((90, 0, 0), cylinder(40, r: 10, center: true, fn: 40)),
  rotate((0, 90, 0), cylinder(40, r: 10, center: true, fn: 40)),
))

#grid(
  columns: (1fr, 1fr, 1fr), column-gutter: 16pt,
  cell([per-part `color(alpha)` \ (mesh PLY alpha)],
       render-ply(core-in-shell, width: 320pt, azimuth: 30, elevation: 22)),
  cell([`%` ghost modifier \ (real .scad)],
       render-ply(ghost, width: 320pt, up: (0,0,1), azimuth: 30, elevation: 28)),
  cell([whole-model `opacity: 0.35` \ (render setting)],
       render-ply(csg, width: 320pt, azimuth: 35, elevation: 25, opacity: 0.35)),
)
