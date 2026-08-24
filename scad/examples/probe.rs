// Validate the Manifold backend: watertightness, per-region color through
// booleans, and revolve/twist orientation. Writes PLYs to /tmp for analysis.
use std::collections::HashMap;
fn main() {
    let cases: &[(&str, &str)] = &[
        ("box_bore",   "difference(){ cube(10, center=true); cylinder(h=30, r=2, center=true, $fn=48); }"),
        ("box_notch",  "difference(){ cube(10, center=true); translate([3,3,3]) cube(6); }"),
        ("col_union",  "union(){ color([0,0,0]) cube(10); color([1,0.5,0]) translate([12,0,0]) cube(10); }"),
        ("col_diff",   "color([1,0,0]) difference(){ cube(10, center=true); sphere(6, $fn=32); }"),
        ("torus",      "rotate_extrude($fn=48) translate([10,0,0]) circle(2, $fn=24);"),
        ("twist",      "linear_extrude(height=20, twist=90, $fn=8) square(10, center=true);"),
        ("gear_ish",   "difference(){ cylinder(h=5,r=10,$fn=64); for(i=[0:11]) rotate([0,0,i*30]) translate([10,0,0]) cylinder(h=6,r=1.5,center=true,$fn=16); translate([0,0,-1]) cylinder(h=8,r=3,$fn=32); }"),
    ];
    for (name, src) in cases {
        match maquette_scad::compile_scad(src, HashMap::new(), 32, HashMap::new()) {
            Ok(b) => { std::fs::write(format!("/tmp/mp_{name}.ply"), &b).unwrap(); println!("{name}: OK {} bytes", b.len()); }
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
