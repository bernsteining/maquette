use std::collections::HashMap;

fn compile(src: &str) -> Vec<u8> {
    maquette_scad::compile_scad(src, HashMap::new(), 32, HashMap::new())
        .unwrap_or_else(|e| panic!("compile failed for {src:?}: {e}"))
}

fn vertex_count(ply: &[u8]) -> usize {
    let end = ply
        .windows(10)
        .position(|w| w == b"end_header")
        .expect("no end_header in PLY");
    let head = std::str::from_utf8(&ply[..end]).unwrap();
    for line in head.lines() {
        if let Some(rest) = line.strip_prefix("element vertex ") {
            return rest.trim().parse().unwrap();
        }
    }
    panic!("no vertex count in PLY header");
}

#[test]
fn root_modifier_discards_siblings() {
    let with_root = compile("cube(10); !sphere(5); cube(20);");
    let sphere_only = compile("sphere(5);");
    assert_eq!(vertex_count(&with_root), vertex_count(&sphere_only));
}

#[test]
fn root_modifier_first_wins() {
    let a = compile("!sphere(5); !cube(100);");
    let sphere_only = compile("sphere(5);");
    assert_eq!(vertex_count(&a), vertex_count(&sphere_only));
}

#[test]
fn fa_fs_refine_sphere() {
    let coarse = compile("$fa=12; $fs=2; sphere(50);");
    let fine = compile("$fa=1; $fs=0.5; sphere(50);");
    assert!(
        vertex_count(&fine) > vertex_count(&coarse) * 2,
        "fine {} should far exceed coarse {}",
        vertex_count(&fine),
        vertex_count(&coarse)
    );
}

#[test]
fn stock_fa_fs_preserves_default_facets() {
    let implicit = compile("sphere(50);");
    let stock = compile("$fa=12; $fs=2; sphere(50);");
    assert_eq!(vertex_count(&implicit), vertex_count(&stock));
}

#[test]
fn explicit_fn_overrides_fa_fs() {
    let a = compile("$fa=1; $fs=0.1; sphere(50, $fn=8);");
    let b = compile("sphere(50, $fn=8);");
    assert_eq!(vertex_count(&a), vertex_count(&b));
}

#[test]
fn huge_polygon_coordinate_errors_not_traps() {
    let r = maquette_scad::compile_scad(
        "polygon([[0,0],[1e9,0],[0,1e9]]);",
        HashMap::new(),
        32,
        HashMap::new(),
    );
    assert!(r.is_err(), "out-of-range polygon should be a clean error");
}
