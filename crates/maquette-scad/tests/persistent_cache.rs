use std::collections::HashMap;

fn compile(src: &str, seg: usize) -> Vec<u8> {
    maquette_scad::compile_scad(src, HashMap::new(), seg, HashMap::new()).unwrap_or_else(|e| panic!("compile failed: {e}"))
}

fn cold(src: &str, seg: usize) -> Vec<u8> {
    let src = src.to_string();
    std::thread::spawn(move || compile(&src, seg)).join().unwrap()
}

const LOGO: &str = "module rod(d, h) cylinder(h, d = d, center = true);
module Logo(size = 50, $fn = 100) {
  hole = size / 2; len = size * 1.25;
  union() {
    difference() { sphere(d = size); rod(hole, len); rotate([90, 0, 0]) rod(hole, len); }
    color(COLOR) rotate([0, 90, 0]) rod(hole, len);
  }
}
Logo(SIZE);";

fn logo(color: &str, size: &str) -> String {
    LOGO.replace("COLOR", color).replace("SIZE", size)
}

#[test]
fn warm_compiles_match_cold_ones() {
    let gear = include_str!("../../../examples/data/scad/gear.scad");
    let steps: Vec<(String, usize)> = vec![
        (logo("[0.5, 0.3, 0.1, 0.6]", "50"), 32),
        (logo("[0.5, 0.3, 0.1, 0.6]", "50"), 32),
        (logo("\"red\"", "50"), 32),
        (logo("\"red\"", "40"), 32),
        (logo("\"red\"", "40"), 16),
        (gear.to_string(), 32),
        (gear.replace("$fn", "$fa"), 32),
        (logo("[0.5, 0.3, 0.1, 0.6]", "50"), 32),
        ("for (i = [0:5]) translate([i * 12, 0, 0]) color(i % 2 ? \"blue\" : \"green\") cube(10);".to_string(), 32),
        ("for (i = [0:6]) translate([i * 12, 0, 0]) color(i % 2 ? \"blue\" : \"green\") cube(10);".to_string(), 32),
        ("#cube(10); %sphere(8); translate([20, 0, 0]) cylinder(h = 10, r = 4);".to_string(), 32),
        (gear.to_string(), 32),
    ];
    for (i, (src, seg)) in steps.iter().enumerate() {
        assert!(compile(src, *seg) == cold(src, *seg), "step {i}: warm compile differs from a cold one");
    }
}
