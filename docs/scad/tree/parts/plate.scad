module plate(w = 60, d = 40, h = 4) {
  difference() {
    cube([w, d, h]);
    for (x = [8, w - 8], y = [8, d - 8])
      translate([x, y, -1]) cylinder(h = h + 2, r = 3, $fn = 32);
  }
}
