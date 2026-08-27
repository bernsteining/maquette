Logo(50);

module Logo(size = 50, $fn = 100) {
  hole = size / 2;
  cylinderHeight = size * 1.25;

  union() {
    difference() {
      sphere(d = size);
      cylinder(d = hole, h = cylinderHeight, center = true);
      rotate([90, 0, 0]) cylinder(d = hole, h = cylinderHeight, center = true);
    }
    // The "highlighted" cylinder from the OpenSCAD IDE preview — rendered
    // as a solid coloured tube going horizontally through the sphere.
    color([0.9, 0.35, 0.35])
      rotate([0, 90, 0])
        cylinder(d = hole, h = cylinderHeight, center = true);
  }
}
