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

// ── panic diagnostics ─────────────────────────────────────────────────────
//
// Rust's `panic = "abort"` on `wasm32-unknown-unknown` compiles a panic into
// the `unreachable` wasm instruction — the host sees a bare `TrapCode
// (UnreachableCodeReached)` and nothing else. We install a hook (once, at
// wasm-func entry) that captures the panic location + message into a
// thread-local buffer, and expose `get_last_panic` so callers can retrieve
// it after a trap. Between entry-point installation and the trap the buffer
// gets populated; the trap itself doesn't clear it.
use std::cell::RefCell;
use std::sync::Once;
thread_local! {
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}
static PANIC_HOOK: Once = Once::new();
fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let loc = info.location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "?".into());
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() { (*s).to_string() }
                else if let Some(s) = info.payload().downcast_ref::<String>() { s.clone() }
                else { "(non-string panic payload)".into() };
            LAST_PANIC.with(|p| *p.borrow_mut() = format!("panic at {}: {}", loc, msg));
        }));
    });
}

/// Fetch the last panic message captured by our hook. Empty string when the
/// hook never fired. Callers use this after a trap to figure out what blew up.
#[wasm_func]
fn get_last_panic() -> Vec<u8> {
    install_panic_hook();
    LAST_PANIC.with(|p| p.borrow().clone().into_bytes())
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
    let loaded = if sidecars_bundle.is_empty() {
        gltf_loader::parse(gltf_data)?
    } else {
        gltf_loader::parse_split(gltf_data, sidecars_bundle)?
    };
    let opts = scene::SceneOpts::from_config(&config);
    let scene = cache::scene_for(gltf_data, &loaded, opts);
    Ok(render::render(scene, &config))
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
