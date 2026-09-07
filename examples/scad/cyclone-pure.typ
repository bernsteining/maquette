// Cyclone-PCB-Factory — compiled ENTIRELY inside Typst, no native harness and
// no manual file list. compile-scad-tree() reads the entry file and walks the
// project's own use/include graph with read(), so the plugin gets every source
// automatically. Give `root` as a Typst project-root path (leading "/").
#import "../../crates/maquette-scad/maquette-scad.typ": *
#import "../../crates/maquette/maquette/maquette.typ": render-ply
#set page(width: 1400pt, height: auto, margin: 24pt, fill: rgb("#e9e9ec"))
#set text(font: "DejaVu Sans", size: 22pt)

= Cyclone-PCB-Factory — compiled in-plugin from Typst
#text(size: 15pt)[`compile-scad-tree("Cyclone.scad", root: …)` walks use/include, runs Manifold, returns PLY — all in the wasm plugin.]

#let model = compile-scad-tree("Cyclone.scad", root: "/examples/scad/cyclone-src/", fn: 8)

#let cfg = openscad-view + (width: 1400, height: 1400, zoom: 1.4, up: (0, 0, 1))
#render-ply(model, cfg + (azimuth: 35, elevation: 20), width: 100%)
