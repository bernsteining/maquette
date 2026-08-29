module Logo(size = 50, $fn = 100) {
  hole = size / 2;
  cylinderHeight = size * 1.25;

  union() {
    difference() {
      sphere(d = size);
      cylinder(d = hole, h = cylinderHeight, center = true);
      rotate([90, 0, 0]) cylinder(d = hole, h = cylinderHeight, center = true);
    }
    color([1.0, 0.4, 0.4, 0.3])
      rotate([0, 90, 0])
        cylinder(d = hole, h = cylinderHeight, center = true);
  }
}

Logo(50);
