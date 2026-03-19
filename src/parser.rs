use crate::math::{parse_vec3_iter, Vec3};

/// A triangle with three vertices, a face normal, and optional per-face color.
#[derive(Clone, Copy)]
pub struct Triangle {
    pub vertices: [Vec3; 3],
    pub normal: Vec3,
    /// Per-face color from binary STL attribute bytes (RGB565). None = use config color.
    pub color: Option<(u8, u8, u8)>,
    /// Per-vertex colors for smooth color mapping. None = use face color.
    pub vertex_colors: Option<[(u8, u8, u8); 3]>,
    /// OBJ group ID. Triangles with the same group_id belong to the same part.
    pub group_id: Option<u32>,
}

/// Parse STL data — auto-detects ASCII vs binary format.
pub fn parse_stl(data: &[u8]) -> Result<Vec<Triangle>, String> {
    if is_ascii_stl(data) {
        parse_ascii(data)
    } else {
        parse_binary(data)
    }
}

fn is_ascii_stl(data: &[u8]) -> bool {
    // ASCII STL starts with "solid" (but some binary files also do).
    // Heuristic: if it starts with "solid" and contains "facet normal", it's ASCII.
    if data.len() < 80 {
        return data.starts_with(b"solid");
    }
    if !data.starts_with(b"solid") {
        return false;
    }
    // Check if the header region contains "facet" — binary won't have this as text
    let check_len = data.len().min(1000);
    let header_region = &data[..check_len];
    header_region
        .windows(5)
        .any(|w| w == b"facet")
}

fn parse_ascii(data: &[u8]) -> Result<Vec<Triangle>, String> {
    let text = std::str::from_utf8(data).map_err(|_| "invalid UTF-8 in STL")?;
    let mut triangles = Vec::new();
    let mut lines = text.lines().map(|l| l.trim());

    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("facet normal") {
            let normal = parse_vec3(rest.trim())?;
            // Skip "outer loop"
            lines.next();
            let v0 = parse_vertex(&mut lines)?;
            let v1 = parse_vertex(&mut lines)?;
            let v2 = parse_vertex(&mut lines)?;
            // Skip "endloop" and "endfacet"
            lines.next();
            lines.next();
            triangles.push(Triangle {
                vertices: [v0, v1, v2],
                normal,
                color: None,
                vertex_colors: None,
                group_id: None,
            });
        }
    }
    Ok(triangles)
}

fn parse_vertex<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<Vec3, String> {
    let line = lines.next().ok_or("unexpected end of STL")?;
    let rest = line
        .strip_prefix("vertex")
        .ok_or("expected 'vertex' keyword")?;
    parse_vec3(rest.trim())
}

fn parse_vec3(s: &str) -> Result<Vec3, String> {
    parse_vec3_iter(&mut s.split_whitespace()).ok_or_else(|| "expected 3 floats".into())
}

fn parse_binary(data: &[u8]) -> Result<Vec<Triangle>, String> {
    if data.len() < 84 {
        return Err("binary STL too short".into());
    }
    // 80-byte header, then u32 triangle count
    let num_triangles = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
    let expected = 84 + num_triangles * 50; // each triangle = 50 bytes
    if data.len() < expected {
        return Err("binary STL truncated".into());
    }

    // Check header for VisCAM/SolidView color convention:
    // "COLOR=" at bytes 0..6 means per-face colors are present.
    let has_color = &data[0..6] == b"COLOR="
        || has_any_nonzero_attributes(data, num_triangles);

    let mut triangles = Vec::with_capacity(num_triangles);

    // Process triangles using chunks_exact (each triangle = 50 bytes)
    for chunk in data[84..84 + num_triangles * 50].chunks_exact(50) {
        let normal = read_vec3_le(chunk, 0);
        let v0 = read_vec3_le(chunk, 12);
        let v1 = read_vec3_le(chunk, 24);
        let v2 = read_vec3_le(chunk, 36);

        // 2-byte attribute: RGB565 color if bit 15 is set (VisCAM/SolidView convention)
        let attr = u16::from_le_bytes([chunk[48], chunk[49]]);

        let color = if has_color && (attr & 0x8000) != 0 {
            // RGB565: bits 0-4 = blue, 5-9 = green, 10-14 = red
            let b5 = (attr & 0x1F) as u8;
            let g5 = ((attr >> 5) & 0x1F) as u8;
            let r5 = ((attr >> 10) & 0x1F) as u8;
            // Scale 5-bit (0-31) to 8-bit (0-255)
            Some(((r5 << 3) | (r5 >> 2), (g5 << 3) | (g5 >> 2), (b5 << 3) | (b5 >> 2)))
        } else {
            None
        };

        triangles.push(Triangle {
            vertices: [v0, v1, v2],
            normal,
            color,
            vertex_colors: None,
            group_id: None,
        });
    }
    Ok(triangles)
}

/// Check if any triangle has a non-zero attribute byte (heuristic for color data).
fn has_any_nonzero_attributes(data: &[u8], num_triangles: usize) -> bool {
    let check = num_triangles.min(100); // sample first 100
    for i in 0..check {
        let attr_off = 84 + i * 50 + 48;
        let attr = u16::from_le_bytes([data[attr_off], data[attr_off + 1]]);
        if attr != 0 {
            return true;
        }
    }
    false
}

#[inline]
fn read_vec3_le(data: &[u8], offset: usize) -> Vec3 {
    let x = f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as f64;
    let y = f32::from_le_bytes([
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]) as f64;
    let z = f32::from_le_bytes([
        data[offset + 8],
        data[offset + 9],
        data[offset + 10],
        data[offset + 11],
    ]) as f64;
    Vec3::new(x, y, z)
}
