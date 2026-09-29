//! `maquette-gltf` — Typst plugin that renders glTF 2.0 assets at compile
//! time. Sibling to the `maquette` plugin (STL/OBJ/PLY); shares the format-
//! agnostic render primitives via `maquette-core`.

#![allow(dead_code, static_mut_refs)]

use wasm_minimal_protocol::*;

#[cfg(target_arch = "wasm32")] initiate_protocol!();

mod cache;
mod config;
mod gltf_loader;
mod pbr;
use maquette_core::prof;
mod render;
mod scene;

maquette_core::panic_export!();

/// Most supersampled samples a plain render fits in wasm32 memory with the
/// deferred shading buffers (measured: 125 M renders, 134 M runs out; shadows
/// and SSAO lower the real ceiling somewhat).
const MAX_RASTER_SAMPLES: u64 = 125_000_000;
use maquette_core::panic::install_hook as install_panic_hook;

/// 16-byte handle for a GLB/glTF file. Once a render has cached its scene,
/// `render_gltf` accepts the handle in place of the file, skipping the copy and
/// the re-hash; on a scene-cache miss it fails and the caller resends the file.
#[cfg_attr(target_arch = "wasm32", wasm_func)]
fn model_key(gltf_data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(cache::SceneInput::new(gltf_data, &[]).handle())
}

/// Render a glTF (JSON) or GLB (binary) file to raw RGBA bytes.
///
/// Wire format returned to the Typst wrapper:
///   `[0x00][w u32 LE][h u32 LE][rgba8...]`
///
/// The wrapper feeds this straight to `image(..., format: (encoding: "rgba8", ...))`
/// so there's no PNG encode/decode round-trip.
#[wasm_func]
fn render_gltf(gltf_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    render_impl(gltf_data, config_json, &[], &[])
}

/// Variant of `render_gltf` that accepts an HDR environment as a third bytes
/// arg (Radiance .hdr / RGBE). Empty slice = no HDR (falls back to procedural
/// or config-embedded HDR). Passing HDR out-of-band avoids blowing up the
/// config JSON size by ~5× when a 1 MB HDR ships as bytes.
#[wasm_func]
fn render_gltf_hdr(gltf_data: &[u8], config_json: &[u8], hdr_data: &[u8]) -> Result<Vec<u8>, String> {
    render_impl(gltf_data, config_json, hdr_data, &[])
}

/// Split-glTF variant: `sidecars_bundle` is a packed table of the external
/// files referenced by the `.gltf` (its `.bin` buffer(s) and any external
/// PNG/JPG images). The Typst wrapper builds the bundle by walking the JSON,
/// so the user still calls the plugin with a single `read(...)` on the
/// `.gltf` path. Bundle layout is documented on `gltf_loader::parse_split`.
///
/// No HDR arg on this entry point — split `.gltf` + external HDR combos are
/// rare in the wild; if you need one, embed the HDR inline in the config or
/// switch to GLB and use `render_gltf_hdr`.
#[wasm_func]
fn render_gltf_split(gltf_data: &[u8], config_json: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
    render_impl(gltf_data, config_json, &[], sidecars_bundle)
}

fn render_impl(gltf_data: &[u8], config_json: &[u8], hdr_data: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
    install_panic_hook();
    let mut config = config::parse(config_json)?;
    maquette_core::effects::check_raster_size(config.width.max(1), config.height.max(1), config.antialias.clamp(1, 4), MAX_RASTER_SAMPLES)?;
    if !hdr_data.is_empty() {
        use std::hash::Hasher;
        let mut h = maquette_core::math::FxHasher::default();
        h.write_u64(config.shading_key);
        h.write(hdr_data);
        config.shading_key = h.finish() | 1;
        if let Some(ref mut ibl) = config.ibl {
            ibl.hdr_bytes = Some(hdr_data.to_vec());
        } else {
            let mut c = crate::config::IblCfg::default();
            c.hdr_bytes = Some(hdr_data.to_vec());
            config.ibl = Some(c);
        }
    }
    let opts = scene::SceneOpts::from_config(&config);
    prof::mark(1);
    let input = cache::SceneInput::new(gltf_data, sidecars_bundle);
    let (scene_key, scene) = cache::scene_for(&input, opts, || {
        if sidecars_bundle.is_empty() {
            gltf_loader::parse(gltf_data)
        } else {
            gltf_loader::parse_split(gltf_data, sidecars_bundle)
        }
    })?;
    prof::mark(2);
    Ok(render::render(scene, scene_key, &config))
}

/// Finish a band rendered with `band` and `ssao`: `y0` is the band's first
/// row as a little-endian `u32`, `full_depth` the whole frame's depth gathered
/// from every band's render output. Returns the band's raw RGBA.
#[cfg_attr(target_arch = "wasm32", wasm_func)]
fn finish_gltf_band(y0: &[u8], full_depth: &[u8]) -> Result<Vec<u8>, String> {
    install_panic_hook();
    let y0: [u8; 4] = y0.try_into().map_err(|_| "band row must be 4 bytes".to_string())?;
    render::finish_band(u32::from_le_bytes(y0) as usize, full_depth)
}

/// Return scene metadata (triangle count, bounding box, animation length) as
/// JSON, without rendering. Skips texture decoding for speed — info only
/// needs geometry. `max_animation_time` is the largest input-time keyframe
/// across every channel of every animation in the glTF; 0.0 for a static
/// asset. Callers use it to build a scrub slider bounded to the actual
/// animation length.
#[wasm_func]
fn get_gltf_info(gltf_data: &[u8], _config_json: &[u8]) -> Result<Vec<u8>, String> {
    info_impl(gltf_data, &[])
}

/// Split-glTF variant of `get_gltf_info` — same output, but resolves external
/// `.bin` references via a sidecar bundle. Geometry is stored in the buffers
/// so info queries need buffer resolution too (bounding box, triangle count,
/// animation length).
#[wasm_func]
fn get_gltf_info_split(gltf_data: &[u8], _config_json: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
    info_impl(gltf_data, sidecars_bundle)
}

fn info_impl(gltf_data: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
    install_panic_hook();
    let loaded = if sidecars_bundle.is_empty() {
        gltf_loader::parse(gltf_data)?
    } else {
        gltf_loader::parse_split(gltf_data, sidecars_bundle)?
    };
    let scene = scene::flatten_geometry_only(&loaded);
    let (center, radius) = scene.bounds();
    let max_animation_time = max_animation_endpoint(&loaded);
    let json = format!(
        r#"{{"triangles":{},"bbox_min":[{},{},{}],"bbox_max":[{},{},{}],"center":[{},{},{}],"radius":{},"max_animation_time":{}}}"#,
        scene.triangles.len(),
        scene.bbox_min.x, scene.bbox_min.y, scene.bbox_min.z,
        scene.bbox_max.x, scene.bbox_max.y, scene.bbox_max.z,
        center.x, center.y, center.z,
        radius,
        max_animation_time,
    );
    Ok(json.into_bytes())
}

/// Walk every animation channel and return the largest input-time keyframe.
/// 0.0 for a static asset. Cheap: reads only the sampler input accessors,
/// not any output data. Cost is O(anim_channels × log(keyframes)) at worst
/// — a handful of ms even for CesiumMan-scale assets.
fn max_animation_endpoint(loaded: &gltf_loader::LoadedGltf) -> f32 {
    let mut max_t = 0.0f32;
    for anim in loaded.document.animations() {
        for channel in anim.channels() {
            let reader = channel.reader(|buffer| {
                loaded.buffers.get(buffer.index()).map(|v| v.as_slice())
            });
            if let Some(iter) = reader.read_inputs() {
                if let Some(last) = iter.last() {
                    if last > max_t { max_t = last; }
                }
            }
        }
    }
    max_t
}

maquette_core::native_protocol!();

#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    use super::*;

    pub fn render_gltf(gltf_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
        render_impl(gltf_data, config_json, &[], &[])
    }

    pub fn render_gltf_hdr(gltf_data: &[u8], config_json: &[u8], hdr_data: &[u8]) -> Result<Vec<u8>, String> {
        render_impl(gltf_data, config_json, hdr_data, &[])
    }

    pub fn render_gltf_split(gltf_data: &[u8], config_json: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
        render_impl(gltf_data, config_json, &[], sidecars_bundle)
    }

    pub fn model_key(gltf_data: &[u8]) -> Result<Vec<u8>, String> {
        super::model_key(gltf_data)
    }

    pub fn finish_gltf_band(y0: &[u8], full_depth: &[u8]) -> Result<Vec<u8>, String> {
        super::finish_gltf_band(y0, full_depth)
    }

    pub fn get_gltf_info(gltf_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
        let _ = config_json;
        info_impl(gltf_data, &[])
    }

    pub fn get_gltf_info_split(gltf_data: &[u8], config_json: &[u8], sidecars_bundle: &[u8]) -> Result<Vec<u8>, String> {
        let _ = config_json;
        info_impl(gltf_data, sidecars_bundle)
    }
}
