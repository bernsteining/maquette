// dotSCAD "torus knot dragon" (low-poly) — JustinSDK/dotSCAD/examples/dragon,
// compiled by maquette-scad (Manifold kernel). Exercises bezier curves,
// path_extrude/sweep, matrix algebra and a heavy polyhedron load. Source + .ply
// are gitignored; reproduce with: git clone dotSCAD /tmp/dotscad &&
// cargo run --release --example dragon.
#import "../../scad/maquette-scad.typ": *
#import "../../maquette/maquette.typ": render-ply
#set page(width: 1400pt, height: auto, margin: 20pt, fill: rgb("#e9e9ec"))
#set text(font: "DejaVu Sans", size: 20pt)
= dotSCAD — torus-knot dragon (low-poly)
#text(size: 14pt)[Compiled by maquette-scad (Manifold kernel) · bezier + path_extrude + matrix algebra · watertight.]
#let d = read("dragon.ply", encoding: none)
#let cfg = openscad-view + (width: 2000, height: 2000, antialias: 2, zoom: 1.3, up: (0,0,1))
#grid(columns: 2, gutter: 10pt,
  render-ply(d, cfg + (azimuth: 35, elevation: 25), width: 100%),
  render-ply(d, cfg + (azimuth: 125, elevation: 20), width: 100%))
