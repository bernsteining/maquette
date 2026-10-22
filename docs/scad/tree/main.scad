use <parts/plate.scad>
use <parts/bolt.scad>

plate();
for (x = [8, 52], y = [8, 32])
  translate([x, y, -6]) color([0.72, 0.75, 0.8]) bolt();
