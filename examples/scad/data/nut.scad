// Parametric hex nut — trig, $fn override per-call, diameter args, function
$fn = 64;

function flat2rad(af) = af / 2 / cos(30);   // across-flats -> circumradius

module nut(af = 16, h = 8, hole = 8) {
    difference() {
        cylinder(h = h, r = flat2rad(af), $fn = 6);      // hex prism
        translate([0, 0, -1]) cylinder(h = h + 2, d = hole);
    }
}

nut();
