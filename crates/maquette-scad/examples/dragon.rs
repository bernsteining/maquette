// Render the dotSCAD "dragon and pearl" example (JustinSDK/dotSCAD) — a heavy
// test of the evaluator (bezier/sweep/path_extrude/L-systems, list comprehensions)
// and the Manifold kernel (lots of polyhedron() from swept sections + convex hull).
//   git clone --depth 1 https://github.com/JustinSDK/dotSCAD /tmp/dotscad
//   cargo run --release --example dragon
use std::collections::HashMap;
use std::fs;
use std::path::Path;
fn collect(dir: &Path, root: &Path, map: &mut HashMap<String, String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, root, map);
        } else if matches!(p.extension().and_then(|s| s.to_str()), Some("scad")) {
            let key = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            map.insert(key, fs::read_to_string(&p).unwrap_or_default());
        }
    }
}
fn main() {
    let root = Path::new("/tmp/dotscad");
    let mut files = HashMap::new();
    collect(root, root, &mut files);
    println!("loaded {} files", files.len());
    let driver = "include <examples/dragon/torus_knot_dragon_low_poly.scad>\n";
    // dotSCAD's recursive helpers (convex_hull3, bezier_smooth, fibonacci_lattice)
    // recurse deep enough to overflow the default 8 MB stack, so run the compile on
    // a thread with a large stack. (The wasm plugin can't grow its stack, so heavy
    // recursive models are native-harness-only — same split as the big meshes.)
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(move || maquette_scad::compile_scad(driver, files, 32, HashMap::new()))
        .unwrap();
    match handle.join().unwrap() {
        Ok(b) => {
            let out = "/home/lisbeth/documents/prog/perso/typst/maquette/examples/scad/dragon.ply";
            fs::write(out, &b).unwrap();
            println!("OK: {} PLY bytes -> {out}", b.len());
        }
        Err(e) => println!("ERR: {e}"),
    }
}
