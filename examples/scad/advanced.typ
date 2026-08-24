#import "../../scad/maquette-scad.typ": compile-scad
#import "../../maquette/maquette.typ": render-ply

// Advanced OpenSCAD language features, evaluated by scad/src/scad.rs:
//  - list comprehensions (with let/trig) driving polygon points
//  - children() module composition
//  - the official OpenSCAD list_comprehensions.scad example (2D grid, auto-extruded)

#set page(width: 1180pt, height: auto, margin: 24pt, fill: white)
#set text(font: "DejaVu Sans", size: 13pt)

#let cell(title, body) = stack(spacing: 8pt, body, align(center, text(weight: "bold", title)))

#let flower = compile-scad("
$fn = 64;
n = 120;
points = [ for (i = [0:n-1]) let (a = i*360/n, r = 12 + 3*cos(6*a))
             [r*cos(a), r*sin(a)] ];
linear_extrude(height = 5) polygon(points);
")

#let corners = compile-scad("
module at_corners(size = 20) {
    for (x = [-1, 1], y = [-1, 1])
        translate([x*size/2, y*size/2, 0]) children();
}
union() {
    cube([24, 24, 3], center = true);
    at_corners(20) cylinder(h = 10, r = 3, $fn = 24);
}
")

#grid(columns: (1fr, 1fr, 1fr), column-gutter: 16pt,
  cell([list comprehension \ (`for … let`)],
       render-ply(flower, width: 320pt, up: (0,0,1), azimuth: 0, elevation: 62)),
  cell([`children()` \ composition],
       render-ply(corners, width: 320pt, up: (0,0,1), azimuth: 30, elevation: 28)),
  cell([official `list_comprehensions.scad` \ (ngons · rounded · stars)],
       render-ply(compile-scad(read("data/listcomp.scad")), width: 320pt, up: (0,0,1), azimuth: 0, elevation: 70)),
)
