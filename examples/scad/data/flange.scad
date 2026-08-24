// Parametric bolt flange
$fn      = 48;
wall     = 2;
n        = 6;      // number of posts
r_bolt   = 3;
r_ring   = 20;
post_h   = 12;

module post(h) {
    cylinder(h = h, r = r_bolt);
}

module ring() {
    difference() {
        cylinder(h = 4, r = r_ring + wall);
        translate([0, 0, -1]) cylinder(h = 20, r = r_ring - wall);
    }
}

module flange() {
    union() {
        ring();
        for (i = [0 : n - 1]) {
            rotate([0, 0, i * 360 / n])
                translate([r_ring, 0, 0])
                    post(post_h);
        }
    }
}

flange();
