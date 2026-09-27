use crate::math::FloatExt;
use crate::color::parse_hex_color;
use crate::math::{find_newline, parse_f64_from, parse_i64_bytes, AsciiTokens, Vec3};
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
                let vals: Vec<f64> = parts.filter_map(|s| s.parse().ok()).collect();
                if vals.len() >= 3 {
                    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).fround() as u8;
                    kd = Some((byte(vals[0]), byte(vals[1]), byte(vals[2])));
                }
            }
            "d" => {
                if let Some(v) = parts.next().and_then(|s| s.parse::<f64>().ok()) {
                    alpha = Some((v.clamp(0.0, 1.0) * 255.0).fround() as u8);
                }
            }
            "Tr" => {
                if let Some(v) = parts.next().and_then(|s| s.parse::<f64>().ok()) {
                    alpha = Some(((1.0 - v.clamp(0.0, 1.0)) * 255.0).fround() as u8);
                }
            }
            _ => {}
        }
    }
    flush(&current, &kd, &alpha, &mut out);
    out
}

/// Parse one or more concatenated `.mtl` files for diffuse texture maps.
/// Returns a map of material name (`newmtl`) → `map_Kd` filename, verbatim as
/// written in the MTL (the Typst wrapper resolves it to bytes and the plugin's
/// texture-index map keys off the same material name).
///
/// `map_Kd` may carry leading options (`-o`, `-s`, `-bm`, …) before the
/// filename; we take the last whitespace-separated token as the path, which is
/// correct for the overwhelmingly common `map_Kd texture.png` form and for
/// option-prefixed forms whose filename has no spaces. Backslashes are
/// normalised to `/` so Windows-authored paths match sidecar keys.
pub fn parse_mtl_textures(data: &str) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut current: Option<String> = None;
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let mut parts = line.split_ascii_whitespace();
        let Some(kw) = parts.next() else { continue };
        match kw {
            "newmtl" => current = parts.next().map(String::from),
            "map_Kd" => {
                if let Some(name) = &current {
                    if let Some(file) = parts.last() {
                        out.insert(name.clone(), file.replace('\\', "/"));
                    }
                }
            }
            _ => {}
        }
    }
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
    tex_index: &HashMap<String, u16>,
) -> Result<(Vec<Triangle>, HashMap<u32, GroupAppearance>), String> {
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut triangles: Vec<Triangle> = Vec::new();
    let mut group_styles: HashMap<u32, GroupAppearance> = HashMap::new();
    let mut current_color: Option<(u8, u8, u8)> = None;
    let mut current_highlight: Option<(u8, u8, u8)> = None;
    let mut current_tex: Option<u16> = None;
    let mut current_group: Option<u32> = None;
    let mut group_counter: u32 = 0;
    let mut current_smooth: Option<u32> = Some(0);

    let mut face_buf: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();

    let n = data.len();
    let mut pos = 0;
    while pos < n {
        let mut i = pos;
        while i < n && data[i] <= b' ' && data[i] != b'\n' { i += 1; }
        if i >= n { break; }
        if data[i] == b'\n' {
            pos = i + 1;
            continue;
        }
        let ks = i;
        while i < n && data[i] > b' ' { i += 1; }
        let keyword = &data[ks..i];

        match keyword {
            b"v" => {
                let v = parse_vec3_at(data, &mut i).ok_or("vertex needs 3 valid coordinates")?;
                vertices.push(v);
                pos = find_newline(data, i) + 1;
            }
            b"vn" => {
                let v = parse_vec3_at(data, &mut i).ok_or("normal needs 3 valid coordinates")?;
                normals.push(v);
                pos = find_newline(data, i) + 1;
            }
            b"f" => {
                face_buf.clear();
                let nv = vertices.len();
                let nt = texcoords.len();
                let nn = normals.len();
                loop {
                    while i < n && data[i] <= b' ' && data[i] != b'\n' { i += 1; }
                    if i >= n || data[i] == b'\n' { break; }
                    let ts = i;
                    while i < n && data[i] > b' ' { i += 1; }
                    if let Some(idx) = parse_face_index(&data[ts..i], nv, nt, nn) {
                        face_buf.push(idx);
                    }
                }
                pos = i + 1;

                if face_buf.len() < 3 {
                    continue;
                }

                let v0 = vertices[face_buf[0].0];
                let face_color = current_highlight.or(current_color);
                for i in 1..face_buf.len() - 1 {
                    let v1 = vertices[face_buf[i].0];
                    let v2 = vertices[face_buf[i + 1].0];
                    let normal = Vec3::face_normal(v0, v1, v2).unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                    let vertex_normals = match (face_buf[0].2, face_buf[i].2, face_buf[i + 1].2) {
                        (Some(n0), Some(n1), Some(n2)) => Some([normals[n0], normals[n1], normals[n2]]),
                        _ => None,
                    };
                    let uvs = match (current_tex, face_buf[0].1, face_buf[i].1, face_buf[i + 1].1) {
                        (Some(_), Some(t0), Some(t1), Some(t2)) =>
                            Some([texcoords[t0], texcoords[t1], texcoords[t2]]),
                        _ => None,
                    };
                    let tex = if uvs.is_some() { current_tex } else { None };
                    triangles.push(Triangle { splat: false,
                        vertices: [v0, v1, v2],
                        normal,
                        color: face_color,
                        vertex_colors: None,
                        group_id: current_group,
                        alpha: None,
                        vertex_normals,
                        smoothing_group: current_smooth,
                        vertex_scalars: None,
                        uvs,
                        tex,
                    });
                }
            }
            _ => {
                let eol = find_newline(data, i);
                pos = eol + 1;
                let mut parts = AsciiTokens::new(&data[i..eol]);
                match keyword {
                b"vt" => {
                    let u = parts.next().and_then(crate::math::parse_f64_bytes).unwrap_or(0.0) as f32;
                    let v = parts.next().and_then(crate::math::parse_f64_bytes).unwrap_or(0.0) as f32;
                    texcoords.push([u, 1.0 - v]);
                }
                b"s" => {
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
                    let name_str = std::str::from_utf8(name).ok();
                    current_color = if name.first() == Some(&b'#') && name.len() >= 7 {
                        name_str.map(parse_hex_color)
                    } else if let Some(s) = name_str {
                        materials.get(s).map(|hex| parse_hex_color(hex))
                    } else {
                        None
                    };
                    current_tex = name_str.and_then(|s| tex_index.get(s).copied());
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
                        if let Some(hex) = style.color_hex() {
                            current_highlight = Some(parse_hex_color(hex));
                        } else {
                            current_highlight = None;
                        }
                        let mut ga = style.appearance().cloned().unwrap_or_default();
                        ga.name = Some(name);
                        group_styles.insert(gid, ga);
                    } else {
                        group_styles.insert(gid, GroupAppearance {
                            name: Some(name),
                            ..Default::default()
                        });
                        if keyword == b"o" {
                            current_highlight = None;
                        }
                    }
                }
                    _ => {}
                }
            }
        }
    }

    Ok((triangles, group_styles))
}

/// Parse three floats from the current line starting at `data[*i]`, leaving
/// `*i` just past the third token. Same result as splitting the line into
/// tokens and parsing each with `parse_f64_bytes`, without the separate
/// tokenizing pass.
#[inline]
fn parse_vec3_at(data: &[u8], i: &mut usize) -> Option<Vec3> {
    let mut c = [0.0f64; 3];
    for v in &mut c {
        let n = data.len();
        let mut j = *i;
        while j < n && data[j] <= b' ' && data[j] != b'\n' { j += 1; }
        if j >= n || data[j] == b'\n' { return None; }
        let (x, e) = parse_f64_from(data, j)?;
        j = e;
        while j < n && data[j] > b' ' { j += 1; }
        *i = j;
        *v = x;
    }
    Some(Vec3::new(c[0], c[1], c[2]))
}

/// Parse a face vertex index like "1", "1/2", "1/2/3", or "1//3".
/// Returns (vertex_index, Option<texcoord_index>, Option<normal_index>), 0-based.
/// Uses manual parsing to avoid split('/').collect() allocation.
#[inline]
fn parse_face_index(b: &[u8], nv: usize, nt: usize, nn: usize) -> Option<(usize, Option<usize>, Option<usize>)> {
    let slash1 = b.iter().position(|&c| c == b'/');
    let vi_b = match slash1 {
        Some(pos) => &b[..pos],
        None => b,
    };
    let vi = resolve_index(vi_b, nv)?;

    let (ti, ni) = if let Some(pos1) = slash1 {
        let rest = &b[pos1 + 1..];
        if let Some(pos2) = rest.iter().position(|&c| c == b'/') {
            let ti_b = &rest[..pos2];
            let ni_b = &rest[pos2 + 1..];
            let ti = if !ti_b.is_empty() { resolve_index(ti_b, nt) } else { None };
            let ni = if !ni_b.is_empty() { resolve_index(ni_b, nn) } else { None };
            (ti, ni)
        } else {
            let ti = if !rest.is_empty() { resolve_index(rest, nt) } else { None };
            (ti, None)
        }
    } else {
        (None, None)
    };
    Some((vi, ti, ni))
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
