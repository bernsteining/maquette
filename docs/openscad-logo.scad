module Logo(size = 50, $fn = 100) {
  hole = size / 2;
  len  = size * 1.25;
  module rod() cylinder(d = hole, h = len, center = true);

  union() {
    difference() {
      sphere(d = size);
      rod();
      rotate([90, 0, 0]) rod();
    }
    color([0.5, 0.3, 0.1, 0.6])
      rotate([0, 90, 0]) rod();
  }
}

Logo(50);
