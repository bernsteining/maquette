teeth = 24;
mod = 2;
depth = 8;
bore = 5;
holes = 6;
$fn = 96;

pitch = teeth * mod / 2;
root = pitch - mod * 1.25;
tip = pitch + mod;
step = 360 / teeth;
hole_r = mod * 1.4;
hole_pcd = (root + bore) / 2;

function pt(r, a) = [r * cos(a), r * sin(a)];

module tooth() {
  polygon([
    pt(root, -0.3 * step),
    pt(tip, -0.15 * step),
    pt(tip, 0.15 * step),
    pt(root, 0.3 * step),
  ]);
}

difference() {
  linear_extrude(depth, convexity = 6)
  union() {
    circle(r = root);
    for (i = [0 : teeth - 1])
      rotate([0, 0, i * step]) tooth();
  }
  translate([0, 0, -0.5])
    cylinder(h = depth + 1, r = bore);
  for (i = [0 : holes - 1])
    rotate([0, 0, i * 360 / holes])
      translate([hole_pcd, 0, -0.5])
        cylinder(h = depth + 1, r = hole_r);
}
