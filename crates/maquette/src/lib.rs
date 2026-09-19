#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] use wasm_minimal_protocol::*;

#[cfg(target_arch = "wasm32")] initiate_protocol!();

mod annotations;
mod cache;
mod clip;
mod color_map;
mod config;
mod decimate;
mod explode;
mod expr;
mod math;
mod obj_parser;
mod outline;
mod parser;
mod ply_parser;
mod projection;
mod rasterizer;
mod render;
mod shading;
mod shadow;
mod smooth;
mod ssao;
mod svg;

// Shared render primitives now live in maquette-core (deduplicated from this
// crate's former copies). Re-export under the old paths so `crate::color::…`
// and `crate::fxaa::…` call sites are unchanged.
use maquette_core::{color, fxaa};
use maquette_core::texture::{build_mips, Filter, MipLevel, Texture, Wrap};
use config::RenderConfig;
use std::collections::HashMap;

fn parse_config(config_json: &[u8]) -> Result<RenderConfig, String> {
    color::init_color_luts();
    let s = std::str::from_utf8(config_json)
        .map_err(|_| "config: invalid UTF-8")?;
    config::parse_config_json(s)
}

fn cached_stl(data: &[u8]) -> Result<&'static Vec<parser::Triangle>, String> {
    if let Some(t) = cache::get_stl(data) {
        return Ok(t);
    }
    let t = parser::parse_stl(data)?;
    cache::put_stl(data, t);
    Ok(cache::get_stl(data).unwrap())
}

fn cached_obj(
    data: &[u8],
    config: &RenderConfig,
) -> Result<CachedObj, String> {
    // Merge auto-loaded `.mtl` Kd colors under the config's `materials` map
    // (config wins on conflicts — the user's explicit override always beats
    // the sidecar). Only builds an owned map when either source is non-empty.
    let materials_ref: &HashMap<String, String>;
    let mut merged: HashMap<String, String>;
    if !config.mtl.is_empty() {
        merged = obj_parser::parse_mtl(&config.mtl);
        for (k, v) in &config.materials { merged.insert(k.clone(), v.clone()); }
        materials_ref = &merged;
    } else {
        materials_ref = &config.materials;
    }
    // Reparse when materials (from any source) or highlights are present —
    // they affect triangle colors and can't be shared across configs.
    let empty_tex: HashMap<String, u16> = HashMap::new();
    if !materials_ref.is_empty() || !config.highlight.is_empty() {
        let (triangles, group_styles) =
            obj_parser::parse_obj(data, materials_ref, &config.highlight, &empty_tex)?;
        return Ok(CachedObj::Owned(triangles, group_styles));
    }
    if let Some(r) = cache::get_obj(data) {
        return Ok(CachedObj::Ref(r));
    }
    let empty_mat = HashMap::new();
    let empty_hl = HashMap::new();
    let result = obj_parser::parse_obj(data, &empty_mat, &empty_hl, &empty_tex)?;
    cache::put_obj(data, result);
    Ok(CachedObj::Ref(cache::get_obj(data).unwrap()))
}

enum CachedObj {
    Ref(&'static (Vec<parser::Triangle>, HashMap<u32, config::GroupAppearance>)),
    Owned(Vec<parser::Triangle>, HashMap<u32, config::GroupAppearance>),
}

impl CachedObj {
    fn triangles(&self) -> &[parser::Triangle] {
        match self {
            CachedObj::Ref(r) => &r.0,
            CachedObj::Owned(t, _) => t,
        }
    }
    fn group_styles(&self) -> &HashMap<u32, config::GroupAppearance> {
        match self {
            CachedObj::Ref(r) => &r.1,
            CachedObj::Owned(_, g) => g,
        }
    }
}

/// Build the OBJ texture table from the material library and a sidecar bundle
/// of image files. Returns the decoded textures plus a `material name → table
/// index` map that `parse_obj` uses to bind `usemtl` to a texture.
///
/// The MTL text (`config.mtl`) is scanned for `map_Kd <file>`; each referenced
/// file is looked up in the bundle (keyed by the same path, backslashes
/// normalised) and decoded. Images shared by several materials decode once and
/// share a table slot. Materials whose `map_Kd` file is missing from the bundle
/// are simply left untextured (they fall back to their `Kd` colour).
fn build_obj_textures(
    config: &RenderConfig,
    tex_bundle: &[u8],
) -> Result<(Vec<Texture>, HashMap<String, u16>), String> {
    let mut textures: Vec<Texture> = Vec::new();
    let mut tex_index: HashMap<String, u16> = HashMap::new();
    if config.mtl.is_empty() || tex_bundle.is_empty() {
        return Ok((textures, tex_index));
    }
    let map_kd = obj_parser::parse_mtl_textures(&config.mtl);
    if map_kd.is_empty() {
        return Ok((textures, tex_index));
    }
    let files = maquette_core::bundle::parse_sidecar_bundle(tex_bundle)?;
    // Dedup decoded images by filename so one bitmap shared by N materials
    // occupies a single texture slot.
    let mut by_file: HashMap<String, u16> = HashMap::new();
    for (material, file) in &map_kd {
        let slot = match by_file.get(file) {
            Some(&i) => i,
            None => {
                let Some(bytes) = files.get(file) else { continue };
                // PNG / JPEG / TGA — keeps the WebP (VP8) decoder out of this
                // plugin's wasm (~200 KB); OBJ textures are never WebP.
                let decoded = maquette_core::texture_decode::decode_obj_texture(file, bytes)
                    .map_err(|e| format!("texture '{}': {}", file, e))?;
                let base = MipLevel { width: decoded.width, height: decoded.height, rgba: decoded.rgba };
                let (bw, bh) = (base.width, base.height);
                let tex = Texture {
                    mips: build_mips(base),
                    wrap_s: Wrap::Repeat,
                    wrap_t: Wrap::Repeat,
                    mag_filter: Filter::Linear,
                    min_filter: Filter::Linear,
                    lod_bias: 0.5 * ((bw * bh) as f32).log2(),
                };
                let i = textures.len() as u16;
                textures.push(tex);
                by_file.insert(file.clone(), i);
                i
            }
        };
        tex_index.insert(material.clone(), slot);
    }
    Ok((textures, tex_index))
}

fn cached_ply(data: &[u8], config: &RenderConfig) -> Result<Vec<parser::Triangle>, String> {
    // When the user names an explicit scalar property (color_map_property),
    // we can't share the cached parse — a different name picks a different
    // vertex-property slot in the header. Fall through to an uncached
    // reparse; cheap since PLYs typically fit in cache memory-wise.
    let want = config.color_map_property.as_str();
    if !want.is_empty() {
        return match ply_parser::parse_ply_with(data, Some(want))? {
            ply_parser::PlyData::Mesh(t) => Ok(t),
            ply_parser::PlyData::Points(cloud) => Ok(render::pointcloud_to_triangles(&cloud, config)),
        };
    }
    if let Some(ply) = cache::get_ply(data) {
        return match ply {
            ply_parser::PlyData::Mesh(t) => Ok(t.clone()),
            ply_parser::PlyData::Points(cloud) => {
                Ok(render::pointcloud_to_triangles(cloud, config))
            }
        };
    }
    let ply = ply_parser::parse_ply(data)?;
    cache::put_ply(data, ply);
    let cached = cache::get_ply(data).unwrap();
    match cached {
        ply_parser::PlyData::Mesh(t) => Ok(t.clone()),
        ply_parser::PlyData::Points(cloud) => {
            Ok(render::pointcloud_to_triangles(cloud, config))
        }
    }
}

/// Entry point: receives STL bytes + JSON config, returns SVG string.
#[wasm_func]
fn render_stl(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_stl(stl_data)?;
    let empty = HashMap::new();
    let key = cache::hash(stl_data);
    let svg = render::render(triangles, &config, &empty, Some(key), Some(key));
    Ok(svg.into_bytes())
}

/// Entry point: receives OBJ text + JSON config, returns SVG string.
#[wasm_func]
fn render_obj(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let obj = cached_obj(obj_data, &config)?;
    let key = cache::hash(obj_data);
    // Preprocessed-mesh cache only when materials/highlight are absent (otherwise
    // the parsed triangles' colors depend on config not captured by the data hash).
    let prep_key = if config.materials.is_empty() && config.highlight.is_empty() && config.mtl.is_empty() { Some(key) } else { None };
    let svg = render::render(obj.triangles(), &config, obj.group_styles(), Some(key), prep_key);
    Ok(svg.into_bytes())
}

/// Entry point: receives STL bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_stl_png(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_stl(stl_data)?;
    let empty = HashMap::new();
    let key = cache::hash(stl_data);
    render::render_raster(triangles, &config, &empty, Some(key), Some(key), &[])
}

/// Entry point: receives OBJ text + JSON config, returns PNG bytes.
#[wasm_func]
fn render_obj_png(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let obj = cached_obj(obj_data, &config)?;
    let key = cache::hash(obj_data);
    let prep_key = if config.materials.is_empty() && config.highlight.is_empty() && config.mtl.is_empty() { Some(key) } else { None };
    render::render_raster(obj.triangles(), &config, obj.group_styles(), Some(key), prep_key, &[])
}

/// Textured OBJ → PNG. Third arg is a packed sidecar bundle (see
/// `maquette_core::bundle`) of the image files the `.mtl`'s `map_Kd` entries
/// reference, keyed by filename. The Typst wrapper walks the OBJ+MTL and reads
/// each texture so the user still calls the plugin with a single `read(...)`.
///
/// Behaves exactly like `render_obj_png` when the bundle is empty or the MTL
/// declares no usable `map_Kd`, so an OBJ that merely ships colours through
/// this entry renders identically to the plain path.
#[wasm_func]
fn render_obj_png_tex(obj_data: &[u8], config_json: &[u8], tex_bundle: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let (textures, tex_index) = build_obj_textures(&config, tex_bundle)?;
    // No usable textures → identical to the plain OBJ PNG path (keeps the mesh
    // cache and preprocessed-mesh cache in play).
    if textures.is_empty() {
        let obj = cached_obj(obj_data, &config)?;
        let key = cache::hash(obj_data);
        let prep_key = if config.materials.is_empty() && config.highlight.is_empty() && config.mtl.is_empty() { Some(key) } else { None };
        return render::render_raster(obj.triangles(), &config, obj.group_styles(), Some(key), prep_key, &[]);
    }
    // Merge MTL Kd colours under config materials (config wins), same as
    // `cached_obj`, then parse with the texture-index map so `usemtl` binds UVs.
    let mut merged = obj_parser::parse_mtl(&config.mtl);
    for (k, v) in &config.materials { merged.insert(k.clone(), v.clone()); }
    let (triangles, group_styles) =
        obj_parser::parse_obj(obj_data, &merged, &config.highlight, &tex_index)?;
    let key = cache::hash(obj_data);
    // Geometry is unaffected by textures, so the smooth-normal cache (data_key)
    // is safe to keep; the preprocessed-mesh cache is not (colours/materials).
    render::render_raster(&triangles, &config, &group_styles, Some(key), None, &textures)
}

/// Returns JSON with model info (triangle count, bbox, etc.) for STL.
#[wasm_func]
fn get_stl_info(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_stl(stl_data)?;
    Ok(render::get_info(triangles, &config).into_bytes())
}

/// Returns JSON with model info (triangle count, bbox, etc.) for OBJ.
#[wasm_func]
fn get_obj_info(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let obj = cached_obj(obj_data, &config)?;
    Ok(render::get_info(obj.triangles(), &config).into_bytes())
}

/// Entry point: receives PLY bytes + JSON config, returns SVG string.
#[wasm_func]
fn render_ply(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_ply(ply_data, &config)?;
    let empty = HashMap::new();
    Ok(render::render(&triangles, &config, &empty, None, None).into_bytes())
}

/// Entry point: receives PLY bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_ply_png(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_ply(ply_data, &config)?;
    let empty = HashMap::new();
    render::render_raster(&triangles, &config, &empty, None, None, &[])
}

/// Returns JSON with model info (triangle count, bbox, etc.) for PLY.
#[wasm_func]
fn get_ply_info(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_ply(ply_data, &config)?;
    Ok(render::get_info(&triangles, &config).into_bytes())
}

// Off-wasm stand-ins for the protocol glue that `initiate_protocol!` provides on
// wasm (gated out here), so the `#[wasm_func]` export wrappers still type-check.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn __write_args_to_buffer(_ptr: *mut u8) {}
#[cfg(not(target_arch = "wasm32"))]
unsafe fn __send_result_to_host(_ptr: *const u8, _len: usize) {}
#[cfg(not(target_arch = "wasm32"))]
trait __ToResult {
    type Ok: ::core::convert::AsRef<[u8]>;
    type Err: ::core::fmt::Display;
    fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err>;
}
#[cfg(not(target_arch = "wasm32"))]
impl __ToResult for Vec<u8> {
    type Ok = Self;
    type Err = ::core::convert::Infallible;
    fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
}
#[cfg(not(target_arch = "wasm32"))]
impl __ToResult for Box<[u8]> {
    type Ok = Self;
    type Err = ::core::convert::Infallible;
    fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
}
#[cfg(not(target_arch = "wasm32"))]
impl<'a> __ToResult for &'a [u8] {
    type Ok = Self;
    type Err = ::core::convert::Infallible;
    fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
}
#[cfg(not(target_arch = "wasm32"))]
impl<T: ::core::convert::AsRef<[u8]>, E: ::core::fmt::Display> __ToResult
    for ::core::result::Result<T, E>
{
    type Ok = T;
    type Err = E;
    fn to_result(self) -> Self { self }
}
