#import "../../scad/maquette-scad.typ": openscad-text, scad-highlighting
#import "../../maquette/maquette.typ": render-ply

// The maquette-scad module ships an OpenSCAD grammar (openscad.sublime-syntax)
// and a `scad-highlighting` show rule, so ```scad code blocks are highlighted —
// pair the source with its live render for documentation.

#show: scad-highlighting
#set page(width: 780pt, height: auto, margin: 22pt, fill: white)
#set text(font: "DejaVu Sans", size: 12pt)

#let pair(src, ..args) = grid(
  columns: (1.25fr, 1fr), column-gutter: 18pt, align: horizon,
  raw(src, lang: "scad", block: true),
  render-ply(openscad-text(src), width: 230pt, ..args),
)

#pair("// hex nut — module, user function, trig, $fn override
$fn = 64;
function flat2rad(af) = af / 2 / cos(30);
module nut(af = 16, h = 8, hole = 8) {
    difference() {
        cylinder(h = h, r = flat2rad(af), $fn = 6);
        translate([0, 0, -1]) cylinder(h = h + 2, d = hole);
    }
}
nut();", up: (0, 0, 1), azimuth: 30, elevation: 55)

#v(10pt)

#pair("// translucent shell around a solid core (per-part alpha)
union() {
    color(\"red\") sphere(6);
    color([0.35, 0.55, 0.95], 0.28)
        difference() {
            cube(22, center = true);
            cube(17, center = true);
        }
}", azimuth: 30, elevation: 22)
