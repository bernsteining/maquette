use crate::color::parse_hex_color;
use crate::math::{parse_vec3_iter, parse_i64_fast, Vec3};
use crate::parser::Triangle;
use crate::config::{GroupAppearance, GroupStyle};
use std::collections::HashMap;

/// Parse OBJ format data with optional per-face materials and group highlighting.
/// Materials map material names to hex color strings (e.g. "red" → "#ff0000").
/// Highlight maps group names (`g`/`o`) to a color or full appearance override.
///
/// Returns the triangle list and a map from group_id → GroupAppearance for groups
/// that have full appearance overrides (not just a color).
pub fn parse_obj(
    data: &[u8],
    materials: &HashMap<String, String>,
    highlight: &HashMap<String, GroupStyle>,
) -> Result<(Vec<Triangle>, HashMap<u32, GroupAppearance>), String> {
    let text = std::str::from_utf8(data).map_err(|_| "invalid UTF-8 in OBJ")?;

    let mut vertices: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut triangles: Vec<Triangle> = Vec::new();
    let mut group_styles: HashMap<u32, GroupAppearance> = HashMap::new();
    let mut current_color: Option<(u8, u8, u8)> = None;
    let mut current_highlight: Option<(u8, u8, u8)> = None;
    let mut current_group: Option<u32> = None;
    let mut group_counter: u32 = 0;

    // Reusable buffer for face indices (avoids per-face allocation)
    let mut face_buf: Vec<(usize, Option<usize>)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let keyword = match parts.next() {
            Some(k) => k,
            None => continue,
        };

        match keyword {
            "v" => {
                vertices.push(parse_vec3_iter(&mut parts)
                    .ok_or("vertex needs 3 valid coordinates")?);
            }
            "vn" => {
                normals.push(parse_vec3_iter(&mut parts)
                    .ok_or("normal needs 3 valid coordinates")?);
            }
            "usemtl" => {
                let mtl_name = match parts.next() {
                    Some(n) => n,
                    None => continue,
                };
                if mtl_name.starts_with('#') && mtl_name.len() >= 7 {
                    current_color = Some(parse_hex_color(mtl_name));
                } else if let Some(hex) = materials.get(mtl_name) {
                    current_color = Some(parse_hex_color(hex));
                } else {
                    current_color = None;
                }
            }
            "f" => {
                face_buf.clear();
                let nv = vertices.len();
                let nn = normals.len();
                for p in parts {
                    if let Some(idx) = parse_face_index(p, nv, nn) {
                        face_buf.push(idx);
                    }
                }

                if face_buf.len() < 3 {
                    continue;
                }

                // Fan triangulation from first vertex
                let v0 = vertices[face_buf[0].0];
                let face_normal = face_buf[0].1.map(|ni| normals[ni]);
                let face_color = current_highlight.or(current_color);
                for i in 1..face_buf.len() - 1 {
                    let v1 = vertices[face_buf[i].0];
                    let v2 = vertices[face_buf[i + 1].0];

                    let normal = face_normal.unwrap_or_else(|| {
                        (v1 - v0).cross(v2 - v0).normalized()
                    });

                    triangles.push(Triangle {
                        vertices: [v0, v1, v2],
                        normal,
                        color: face_color,
                        vertex_colors: None,
                        group_id: current_group,
                    });
                }
            }
            "g" | "o" => {
                let mut name = String::new();
                for p in parts { if !name.is_empty() { name.push(' '); } name.push_str(p); }
                let gid = group_counter;
                current_group = Some(gid);
                group_counter += 1;

                if let Some(style) = highlight.get(&name) {
                    // Extract color for per-triangle coloring
                    if let Some(hex) = style.color_hex() {
                        current_highlight = Some(parse_hex_color(hex));
                    } else {
                        current_highlight = None;
                    }
                    // Store appearance (or default) with group name
                    let mut ga = style.appearance().cloned().unwrap_or_default();
                    ga.name = Some(name);
                    group_styles.insert(gid, ga);
                } else {
                    // No highlight — still record the group name for annotations
                    group_styles.insert(gid, GroupAppearance {
                        name: Some(name),
                        ..Default::default()
                    });
                    if keyword == "o" {
                        current_highlight = None;
                    }
                }
            }
            _ => {} // skip mtllib, s, vt, etc.
        }
    }

    Ok((triangles, group_styles))
}

/// Parse a face vertex index like "1", "1/2", "1/2/3", or "1//3".
/// Returns (vertex_index, Option<normal_index>), 0-based.
/// Uses manual parsing to avoid split('/').collect() allocation.
#[inline]
fn parse_face_index(s: &str, nv: usize, nn: usize) -> Option<(usize, Option<usize>)> {
    let b = s.as_bytes();
    // Find first '/'
    let slash1 = b.iter().position(|&c| c == b'/');
    let vi_str = match slash1 {
        Some(pos) => &s[..pos],
        None => s,
    };
    let vi = resolve_index(vi_str, nv)?;

    let ni = if let Some(pos1) = slash1 {
        // Find second '/'
        let rest = &b[pos1 + 1..];
        if let Some(pos2) = rest.iter().position(|&c| c == b'/') {
            let ni_str = &s[pos1 + 1 + pos2 + 1..];
            if !ni_str.is_empty() { resolve_index(ni_str, nn) } else { None }
        } else {
            None
        }
    } else {
        None
    };
    Some((vi, ni))
}

/// Convert 1-based (or negative) OBJ index to 0-based. Uses fast integer parser.
#[inline]
fn resolve_index(s: &str, count: usize) -> Option<usize> {
    let idx = parse_i64_fast(s)?;
    if idx > 0 {
        let i = (idx - 1) as usize;
        if i < count { Some(i) } else { None }
    } else if idx < 0 {
        let i = count as i64 + idx;
        if i >= 0 { Some(i as usize) } else { None }
    } else {
        None
    }
}
