// Parametric involute-style spur gear — trapezoidal teeth swept around a hub,
// with a bore and lightening holes. Adjust `teeth`, `mod`, `depth` to rescale.
// A classic OpenSCAD showcase: parametric variables + for-loop composition
// + boolean CSG all in one small file.

teeth  = 24;    // number of teeth
mod    = 2;     // tooth width scale (metric "module")
depth  = 8;     // gear thickness
bore   = 5;     // shaft-hole radius
holes  = 6;     // lightening holes around the hub

$fn = 96;

pitch_r = teeth * mod / 2;
root_r  = pitch_r - mod * 1.25;
tip_r   = pitch_r + mod;
tooth_a = 360 / teeth;
hole_r  = mod * 1.4;
hole_pcd = (root_r + bore) / 2;

module tooth() {
  polygon([
    [ root_r * cos(-tooth_a * 0.30), root_r * sin(-tooth_a * 0.30)],
    [ tip_r  * cos(-tooth_a * 0.15), tip_r  * sin(-tooth_a * 0.15)],
    [ tip_r  * cos( tooth_a * 0.15), tip_r  * sin( tooth_a * 0.15)],
    [ root_r * cos( tooth_a * 0.30), root_r * sin( tooth_a * 0.30)],
  ]);
}

difference() {
  linear_extrude(depth, convexity = 6)
    union() {
      circle(r = root_r);
      for (i = [0 : teeth - 1]) rotate([0, 0, i * tooth_a]) tooth();
    }
  translate([0, 0, -0.5]) cylinder(h = depth + 1, r = bore);
  for (i = [0 : holes - 1]) rotate([0, 0, i * 360 / holes])
    translate([hole_pcd, 0, -0.5]) cylinder(h = depth + 1, r = hole_r);
}
