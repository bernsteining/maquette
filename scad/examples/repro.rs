// Native repro harness for the csgrs panic hit by the Cyclone-PCB-Factory
// X-carriage. Running on x86 (not wasm) means a panic prints its message +
// location + backtrace — which wasmi swallows. Point it at a checkout:
//   git clone --depth 1 https://github.com/carlosgs/Cyclone-PCB-Factory /tmp/cyclone
//   RUST_BACKTRACE=1 cargo run --example repro
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn collect(dir: &Path, root: &Path, map: &mut HashMap<String, String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            collect(&p, root, map);
        } else if matches!(p.extension().and_then(|s| s.to_str()), Some("scad") | Some("h")) {
            let key = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            map.insert(key, fs::read_to_string(&p).unwrap_or_default());
        }
    }
}

fn main() {
    let root = Path::new("/tmp/cyclone/Source_files");
    let mut files = HashMap::new();
    collect(root, root, &mut files);
    println!("loaded {} files", files.len());
    let driver = "include <Cyclone.scad>\n";
    match maquette_scad::compile_scad(driver, files, 8, HashMap::new()) {
        Ok(b) => {
            let out = "/home/lisbeth/documents/prog/perso/typst/maquette/examples/scad/cyclone-full.ply";
            std::fs::write(out, &b).unwrap();
            println!("OK: {} PLY bytes -> {out}", b.len());
        }
        Err(e) => println!("ERR: {e}"),
    }
}
