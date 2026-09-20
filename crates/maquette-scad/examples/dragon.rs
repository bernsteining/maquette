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
