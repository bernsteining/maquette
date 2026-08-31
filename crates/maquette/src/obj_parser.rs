use crate::color::parse_hex_color;
use crate::math::{parse_vec3_bytes, parse_i64_bytes, AsciiTokens, Vec3};
use crate::parser::Triangle;
use crate::config::{GroupAppearance, GroupStyle};
use std::collections::HashMap;

/// Parse one or more concatenated Wavefront `.mtl` files. Returns a map of
/// material name (from `newmtl`) to a `#RRGGBB` hex string derived from `Kd`.
/// Alphas from `d` / `Tr` are folded into the hex string as `#RRGGBBAA`
/// when < 1 so the OBJ pipeline can propagate them via `parse_hex_color`.
///
/// Everything but `newmtl`/`Kd`/`d`/`Tr` is ignored (Ka/Ks/Ns/map_*/etc.):
/// maquette has no PBR pipeline for OBJ, and the diffuse colour is what
/// makes dropped OBJ+MTL bundles "just look right" out of the box.
pub fn parse_mtl(data: &str) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut current: Option<String> = None;
    // Buffer the material until we hit the next `newmtl` (or EOF): Kd may
    // appear before or after `d`, so we compose the hex string on close.
    let mut kd: Option<(u8, u8, u8)> = None;
    let mut alpha: Option<u8> = None;
    let flush = |name: &Option<String>,
                 kd: &Option<(u8, u8, u8)>,
                 alpha: &Option<u8>,
                 out: &mut HashMap<String, String>| {
        if let (Some(name), Some((r, g, b))) = (name.as_deref(), kd) {
            let hex = match alpha {
                Some(a) if *a < 255 => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
                _ => format!("#{r:02x}{g:02x}{b:02x}"),
            };
            out.insert(name.to_string(), hex);
        }
    };
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let mut parts = line.split_ascii_whitespace();
        let Some(kw) = parts.next() else { continue };
        match kw {
            "newmtl" => {
                flush(&current, &kd, &alpha, &mut out);
                current = parts.next().map(String::from);
                kd = None;
                alpha = None;
            }
            "Kd" => {
                // Kd R G B — floats 0..1 in the spec.
                let vals: Vec<f64> = parts.filter_map(|s| s.parse().ok()).collect();
                if vals.len() >= 3 {
                    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    kd = Some((byte(vals[0]), byte(vals[1]), byte(vals[2])));
                }
            }
            "d" => {
                // d = dissolve. 1.0 = opaque, 0.0 = transparent.
                if let Some(v) = parts.next().and_then(|s| s.parse::<f64>().ok()) {
                    alpha = Some((v.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
            "Tr" => {
                // Tr = transparency. Inverse of d — some tools emit it instead.
                // If both appear, last-write wins (rare in practice).
                if let Some(v) = parts.next().and_then(|s| s.parse::<f64>().ok()) {
                    alpha = Some(((1.0 - v.clamp(0.0, 1.0)) * 255.0).round() as u8);
                }
            }
            _ => {}
        }
    }
    flush(&current, &kd, &alpha, &mut out);
    out
}

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
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut triangles: Vec<Triangle> = Vec::new();
    let mut group_styles: HashMap<u32, GroupAppearance> = HashMap::new();
    let mut current_color: Option<(u8, u8, u8)> = None;
    let mut current_highlight: Option<(u8, u8, u8)> = None;
    let mut current_group: Option<u32> = None;
    let mut group_counter: u32 = 0;
    // Active OBJ smoothing group. `None` = `s off` / `s 0` — faces in this
    // state are treated as their own island so nothing merges them (see
    // smooth::compute_vertex_normals fallback quantisation by group id).
    let mut current_smooth: Option<u32> = None;

    // Reusable buffer for face indices (avoids per-face allocation)
    let mut face_buf: Vec<(usize, Option<usize>)> = Vec::new();

    // Parse OBJ bytes directly (ASCII) — no whole-buffer UTF-8 validation, no
    // Unicode-aware `.lines()`/`.trim()`/`split_whitespace`.
    let n = data.len();
    let mut pos = 0;
    while pos < n {
        let mut eol = pos;
        while eol < n && data[eol] != b'\n' { eol += 1; }
        let mut line = &data[pos..eol];
        pos = eol + 1;
        if line.last() == Some(&b'\r') { line = &line[..line.len() - 1]; }

        let mut parts = AsciiTokens::new(line);
        let keyword = match parts.next() {
            Some(k) => k,
            None => continue,
        };

        match keyword {
            b"v" => {
                vertices.push(parse_vec3_bytes(&mut parts)
                    .ok_or("vertex needs 3 valid coordinates")?);
            }
            b"vn" => {
                normals.push(parse_vec3_bytes(&mut parts)
                    .ok_or("normal needs 3 valid coordinates")?);
            }
            b"s" => {
                // `s <n>` starts smoothing group n; `s off` / `s 0` disables
                // it. Faces tagged with a smoothing group share averaged
                // normals with same-group siblings across shared vertices;
                // faces with `None` don't merge, leaving crease edges crisp.
                let tok = parts.next().unwrap_or(b"off");
                current_smooth = if tok == b"off" || tok == b"0" {
                    None
                } else {
                    std::str::from_utf8(tok).ok()
                        .and_then(|s| s.parse::<u32>().ok())
                        .filter(|&n| n != 0)
                };
            }
            b"usemtl" => {
                let name = match parts.next() {
                    Some(n) => n,
                    None => continue,
                };
                current_color = if name.first() == Some(&b'#') && name.len() >= 7 {
                    std::str::from_utf8(name).ok().map(parse_hex_color)
                } else if let Ok(s) = std::str::from_utf8(name) {
                    materials.get(s).map(|hex| parse_hex_color(hex))
                } else {
                    None
                };
            }
            b"f" => {
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

                // Fan triangulation from first vertex. Preserve per-corner
                // `vn` indices — a face contributes vertex_normals only when
                // ALL its corners cite one, so smooth shading sees a
                // coherent per-vertex normal set (mixed cases fall back to
                // face-normal averaging).
                let v0 = vertices[face_buf[0].0];
                let face_color = current_highlight.or(current_color);
                for i in 1..face_buf.len() - 1 {
                    let v1 = vertices[face_buf[i].0];
                    let v2 = vertices[face_buf[i + 1].0];
                    // Face normal from geometry — deterministic and correct
                    // regardless of per-corner normals.
                    let normal = Vec3::face_normal(v0, v1, v2).unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                    let vertex_normals = match (face_buf[0].1, face_buf[i].1, face_buf[i + 1].1) {
                        (Some(n0), Some(n1), Some(n2)) => Some([normals[n0], normals[n1], normals[n2]]),
                        _ => None,
                    };
                    triangles.push(Triangle {
                        vertices: [v0, v1, v2],
                        normal,
                        color: face_color,
                        vertex_colors: None,
                        group_id: current_group,
                        alpha: None,
                        vertex_normals,
                        smoothing_group: current_smooth,
                        vertex_scalars: None,
                    });
                }
            }
            b"g" | b"o" => {
                let mut name = String::new();
                for p in parts {
                    if !name.is_empty() { name.push(' '); }
                    if let Ok(s) = std::str::from_utf8(p) { name.push_str(s); }
                }
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
                    if keyword == b"o" {
                        current_highlight = None;
                    }
                }
            }
            _ => {} // skip mtllib, s, vt, comments, etc.
        }
    }

    Ok((triangles, group_styles))
}

/// Parse a face vertex index like "1", "1/2", "1/2/3", or "1//3".
/// Returns (vertex_index, Option<normal_index>), 0-based.
/// Uses manual parsing to avoid split('/').collect() allocation.
#[inline]
fn parse_face_index(b: &[u8], nv: usize, nn: usize) -> Option<(usize, Option<usize>)> {
    // Find first '/'
    let slash1 = b.iter().position(|&c| c == b'/');
    let vi_b = match slash1 {
        Some(pos) => &b[..pos],
        None => b,
    };
    let vi = resolve_index(vi_b, nv)?;

    let ni = if let Some(pos1) = slash1 {
        let rest = &b[pos1 + 1..];
        if let Some(pos2) = rest.iter().position(|&c| c == b'/') {
            let ni_b = &b[pos1 + 1 + pos2 + 1..];
            if !ni_b.is_empty() { resolve_index(ni_b, nn) } else { None }
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
fn resolve_index(b: &[u8], count: usize) -> Option<usize> {
    let idx = parse_i64_bytes(b)?;
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
