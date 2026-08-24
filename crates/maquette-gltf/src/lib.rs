//! `maquette-gltf` — Typst plugin that renders glTF 2.0 assets at compile
//! time. Sibling to the `maquette` plugin (STL/OBJ/PLY); shares the format-
//! agnostic render primitives via `maquette-core`.

use wasm_minimal_protocol::*;

initiate_protocol!();

mod cache;
mod config;
mod gltf_loader;
mod pbr;
mod render;
mod scene;

/// Render a glTF (JSON) or GLB (binary) file to raw RGBA bytes.
///
/// Wire format returned to the Typst wrapper:
///   `[0x00][w u32 LE][h u32 LE][rgba8...]`
///
/// The wrapper feeds this straight to `image(..., format: (encoding: "rgba8", ...))`
/// so there's no PNG encode/decode round-trip.
#[wasm_func]
fn render_gltf(gltf_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    render_impl(gltf_data, config_json, &[])
}

/// Variant of `render_gltf` that accepts an HDR environment as a third bytes
/// arg (Radiance .hdr / RGBE). Empty slice = no HDR (falls back to procedural
/// or config-embedded HDR). Passing HDR out-of-band avoids blowing up the
/// config JSON size by ~5× when a 1 MB HDR ships as bytes.
#[wasm_func]
fn render_gltf_hdr(gltf_data: &[u8], config_json: &[u8], hdr_data: &[u8]) -> Result<Vec<u8>, String> {
    render_impl(gltf_data, config_json, hdr_data)
}

fn render_impl(gltf_data: &[u8], config_json: &[u8], hdr_data: &[u8]) -> Result<Vec<u8>, String> {
    maquette_core::color::init_color_luts();
    let mut config = config::parse(config_json)?;
    if !hdr_data.is_empty() {
        // Third-arg HDR always wins over any config-embedded HDR bytes.
        if let Some(ref mut ibl) = config.ibl {
            ibl.hdr_bytes = Some(hdr_data.to_vec());
        } else {
            let mut c = crate::config::IblCfg::default();
            c.hdr_bytes = Some(hdr_data.to_vec());
            config.ibl = Some(c);
        }
    }
    let loaded = gltf_loader::parse(gltf_data)?;
    let opts = scene::SceneOpts::from_config(&config);
    let scene = cache::scene_for(gltf_data, &loaded, opts);
    Ok(render::render(scene, &config))
}

/// Return scene metadata (triangle count, bounding box) as JSON, without
/// rendering. Skips texture decoding for speed — info only needs geometry.
#[wasm_func]
fn get_gltf_info(gltf_data: &[u8], _config_json: &[u8]) -> Result<Vec<u8>, String> {
    let loaded = gltf_loader::parse(gltf_data)?;
    let scene = scene::flatten_geometry_only(&loaded);
    let (center, radius) = scene.bounds();
    let json = format!(
        r#"{{"triangles":{},"bbox_min":[{},{},{}],"bbox_max":[{},{},{}],"center":[{},{},{}],"radius":{}}}"#,
        scene.triangles.len(),
        scene.bbox_min.x, scene.bbox_min.y, scene.bbox_min.z,
        scene.bbox_max.x, scene.bbox_max.y, scene.bbox_max.z,
        center.x, center.y, center.z,
        radius,
    );
    Ok(json.into_bytes())
}
