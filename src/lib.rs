use wasm_minimal_protocol::*;

initiate_protocol!();

mod annotations;
mod cache;
mod clip;
mod color;
mod color_map;
mod config;
mod explode;
mod expr;
mod fxaa;
mod math;
mod obj_parser;
mod outline;
mod parser;
mod ply_parser;
mod png_encoder;
mod projection;
mod rasterizer;
mod render;
mod shading;
mod smooth;
mod ssao;
mod svg;

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
    // Reparse when materials or highlights are present (they affect triangle colors).
    if !config.materials.is_empty() || !config.highlight.is_empty() {
        let (triangles, group_styles) =
            obj_parser::parse_obj(data, &config.materials, &config.highlight)?;
        return Ok(CachedObj::Owned(triangles, group_styles));
    }
    if let Some(r) = cache::get_obj(data) {
        return Ok(CachedObj::Ref(r));
    }
    let empty_mat = HashMap::new();
    let empty_hl = HashMap::new();
    let result = obj_parser::parse_obj(data, &empty_mat, &empty_hl)?;
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

fn cached_ply(data: &[u8], config: &RenderConfig) -> Result<Vec<parser::Triangle>, String> {
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
    let svg = render::render(triangles, &config, &empty);
    Ok(svg.into_bytes())
}

/// Entry point: receives OBJ text + JSON config, returns SVG string.
#[wasm_func]
fn render_obj(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let obj = cached_obj(obj_data, &config)?;
    let svg = render::render(obj.triangles(), &config, obj.group_styles());
    Ok(svg.into_bytes())
}

/// Entry point: receives STL bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_stl_png(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_stl(stl_data)?;
    let empty = HashMap::new();
    render::render_png(triangles, &config, &empty)
}

/// Entry point: receives OBJ text + JSON config, returns PNG bytes.
#[wasm_func]
fn render_obj_png(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let obj = cached_obj(obj_data, &config)?;
    render::render_png(obj.triangles(), &config, obj.group_styles())
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
    Ok(render::render(&triangles, &config, &empty).into_bytes())
}

/// Entry point: receives PLY bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_ply_png(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_ply(ply_data, &config)?;
    let empty = HashMap::new();
    render::render_png(&triangles, &config, &empty)
}

/// Returns JSON with model info (triangle count, bbox, etc.) for PLY.
#[wasm_func]
fn get_ply_info(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = cached_ply(ply_data, &config)?;
    Ok(render::get_info(&triangles, &config).into_bytes())
}
