#import "../../scad/maquette-scad.typ": openscad-text
#import "../../maquette/maquette.typ": render-ply

// Kernel + library features:
//  - linear_extrude with twist / scale (built manually: earcut caps + sliced walls)
//  - use <lib> pulling a module + function from a library passed via `files:`

#set page(width: 1180pt, height: auto, margin: 24pt, fill: white)
#set text(font: "DejaVu Sans", size: 13pt)
#let cell(title, body) = stack(spacing: 8pt, body, align(center, text(weight: "bold", title)))

#let twisted = openscad-text("$fn=48; linear_extrude(height=30, twist=180, slices=40) square([12,12], center=true);")
#let horn = openscad-text("$fn=48; linear_extrude(height=26, twist=360, scale=0.4, slices=60) translate([6,0]) square([5,5], center=true);")

#let lib = "function golden() = (1+sqrt(5))/2;
module rounded_box(s, r) { minkowski() { cube(s, center=true); sphere(r, $fn=12); } }"
#let usebox = openscad-text(
  "use <mylib.scad>\n$fn=24;\nrounded_box([16,12,8], 2);\ntranslate([0,26,0]) cylinder(h=golden()*6, r=4);",
  files: ("mylib.scad": lib),
)

#grid(columns: (1fr, 1fr, 1fr), column-gutter: 16pt,
  cell([`linear_extrude` twist], render-ply(twisted, width: 300pt, up:(0,0,1), azimuth: 30, elevation: 22)),
  cell([twist + `scale` (horn)], render-ply(horn, width: 300pt, up:(0,0,1), azimuth: 30, elevation: 22)),
  cell([`use <lib>` (module + fn)], render-ply(usebox, width: 300pt, up:(0,0,1), azimuth: 30, elevation: 26)),
)
