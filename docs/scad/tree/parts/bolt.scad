module bolt() {
  cylinder(h = 11, r = 2.6, $fn = 32);
  translate([0, 0, 11]) cylinder(h = 3, r = 4.6, $fn = 6);
}
