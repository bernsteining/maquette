// The Cyclone-PCB-Factory CNC mill (github.com/carlosgs/Cyclone-PCB-Factory)
// compiled by maquette-scad — the whole MCAD + obiscad + standard_parts tree,
// config, and every part module via use/include. The full `include <Cyclone.scad>`
// mesh is ~11 MB (too big for a Typst wasm plugin's memory), so it's compiled
// once to a .ply by the native harness (scad/examples/repro.rs) and rendered
// here by maquette. Compiled by the Manifold CSG kernel, so the whole assembly
// is watertight (0 open edges) and every part renders solid from every angle.
// Sources + .ply are gitignored; to reproduce, put the repo's Source_files/ in
// examples/scad/cyclone-src/ then `cargo run --release --example repro`.
#import "../../crates/maquette-scad/maquette-scad.typ": *
#import "../../crates/maquette/maquette/maquette.typ": render-ply
#set page(width: 2000pt, height: auto, margin: 24pt, fill: rgb("#e9e9ec"))
#set text(font: "DejaVu Sans", size: 22pt)

= Cyclone-PCB-Factory — full machine
#text(size: 15pt)[Real OpenSCAD compiled by maquette-scad (Manifold kernel) · `include <Cyclone.scad>` · watertight · 2400px, 2× supersampled.]

#let full = read("cyclone-full.ply", encoding: none)
// Pass render config as a POSITIONAL dict: render-ply reserves the named
// width/height for DISPLAY size, so the pixel buffer must go here. antialias:2
// supersamples (renders 4800px then downsamples) — this is what keeps thin
// plates/rods from dropping out at grazing angles. zoom fills the frame.
#let cfg = openscad-view + (width: 2400, height: 2400, antialias: 2, zoom: 1.4, up: (0, 0, 1))
#let shot = (az, el) => render-ply(full, cfg + (azimuth: az, elevation: el), width: 100%)
#v(6pt)
#shot(35, 20)
#v(12pt)
#shot(125, 28)
