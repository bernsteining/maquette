#import "../../scad/maquette-scad.typ": openscad-text
#import "../../maquette/maquette.typ": render-ply

// Renders REAL OpenSCAD .scad source files (examples/scad/data/*.scad) through
// the maquette-scad plugin: openscad-rs parses, src/scad.rs evaluates the
// language (variables, $fn, for, if, user modules/functions, recursion, trig,
// PI), and maquette draws the resulting mesh.
//
//   flange.scad / nut.scad  — hand-written parametric parts.
//   gear.scad  — Leemon Baird's public-domain involute spur gear (thing:5505).
//   menger.scad — recursive Menger sponge (order 2), a real fractal script.

#set page(width: 820pt, height: auto, margin: 24pt, fill: white)
#set text(font: "DejaVu Sans", size: 13pt)

#let cell(title, file, ..args) = stack(spacing: 8pt,
  render-ply(openscad-text(read(file)), width: 340pt, ..args),
  align(center, text(weight: "bold", raw(title))),
)

#grid(
  columns: (1fr, 1fr), gutter: 18pt,
  cell("flange.scad", "data/flange.scad", up: (0,0,1), azimuth: 40, elevation: 32),
  cell("nut.scad", "data/nut.scad", up: (0,0,1), azimuth: 30, elevation: 55),
  cell("gear.scad (involute)", "data/gear_one.scad", up: (0,0,1), azimuth: 0, elevation: 60),
  cell("menger.scad (recursive)", "data/menger.scad", up: (0,0,1), azimuth: 25, elevation: 22),
)
