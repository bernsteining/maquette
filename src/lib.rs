use wasm_minimal_protocol::*;

initiate_protocol!();

mod annotations;
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

/// Entry point: receives STL bytes + JSON config, returns SVG string.
#[wasm_func]
fn render_stl(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;

    let triangles = parser::parse_stl(stl_data)?;
    let empty = HashMap::new();

    let svg = render::render(&triangles, &config, &empty);

    Ok(svg.into_bytes())
}

/// Entry point: receives OBJ text + JSON config, returns SVG string.
#[wasm_func]
fn render_obj(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;

    let (triangles, group_styles) = obj_parser::parse_obj(obj_data, &config.materials, &config.highlight)?;

    let svg = render::render(&triangles, &config, &group_styles);

    Ok(svg.into_bytes())
}

/// Entry point: receives STL bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_stl_png(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;

    let triangles = parser::parse_stl(stl_data)?;
    let empty = HashMap::new();

    render::render_png(&triangles, &config, &empty)
}

/// Entry point: receives OBJ text + JSON config, returns PNG bytes.
#[wasm_func]
fn render_obj_png(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;

    let (triangles, group_styles) = obj_parser::parse_obj(obj_data, &config.materials, &config.highlight)?;

    render::render_png(&triangles, &config, &group_styles)
}

/// Returns JSON with model info (triangle count, bbox, etc.) for STL.
#[wasm_func]
fn get_stl_info(stl_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = parser::parse_stl(stl_data)?;
    Ok(render::get_info(&triangles, &config).into_bytes())
}

/// Returns JSON with model info (triangle count, bbox, etc.) for OBJ.
#[wasm_func]
fn get_obj_info(obj_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let (triangles, _group_styles) = obj_parser::parse_obj(obj_data, &config.materials, &config.highlight)?;
    Ok(render::get_info(&triangles, &config).into_bytes())
}

fn resolve_ply(ply_data: &[u8], config: &RenderConfig) -> Result<Vec<parser::Triangle>, String> {
    match ply_parser::parse_ply(ply_data)? {
        ply_parser::PlyData::Mesh(t) => Ok(t),
        ply_parser::PlyData::Points(cloud) => Ok(render::pointcloud_to_triangles(&cloud, config)),
    }
}

/// Entry point: receives PLY bytes + JSON config, returns SVG string.
#[wasm_func]
fn render_ply(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = resolve_ply(ply_data, &config)?;
    let empty = HashMap::new();
    Ok(render::render(&triangles, &config, &empty).into_bytes())
}

/// Entry point: receives PLY bytes + JSON config, returns PNG bytes.
#[wasm_func]
fn render_ply_png(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = resolve_ply(ply_data, &config)?;
    let empty = HashMap::new();
    render::render_png(&triangles, &config, &empty)
}

/// Returns JSON with model info (triangle count, bbox, etc.) for PLY.
#[wasm_func]
fn get_ply_info(ply_data: &[u8], config_json: &[u8]) -> Result<Vec<u8>, String> {
    let config = parse_config(config_json)?;
    let triangles = resolve_ply(ply_data, &config)?;
    Ok(render::get_info(&triangles, &config).into_bytes())
}
