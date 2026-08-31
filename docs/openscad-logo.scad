module rod(d, h) cylinder(h, d = d, center = true);

module Logo(size = 50, $fn = 100) {
  hole = size / 2;
  len  = size * 1.25;

  union() {
    difference() {
      sphere(d = size);
      rod(hole, len);
      rotate([90, 0, 0]) rod(hole, len);
    }
    color([0.5, 0.3, 0.1, 0.6])
      rotate([0, 90, 0]) rod(hole, len);
  }
}

Logo(50);
