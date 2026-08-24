#import "../../scad/maquette-scad.typ": compile-scad
#import "../../maquette/maquette.typ": render-ply

// import() brings external meshes into a .scad scene. The STL bytes are read in
// Typst (the wasm sandbox can't) and passed via `bin`. Here an imported cube is
// scaled and drilled — real CSG on imported geometry.
#set page(width: auto, height: auto, margin: 8pt, fill: white)
#let src = "
$fn = 48;
difference() {
  scale([12, 12, 12]) import(\"cube.stl\");
  translate([6, 6, -2]) cylinder(h = 20, r = 3.5);
  translate([6, 6, 6]) rotate([0, 90, 0]) cylinder(h = 30, r = 2.5, center = true);
}
"
#render-ply(
  compile-scad(src, bin: ("cube.stl": read("../data/cube.stl", encoding: none))),
  width: 340pt, up: (0,0,1), azimuth: 32, elevation: 26,
)
