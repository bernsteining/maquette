use crate::math::{parse_f64_bytes, parse_i64_bytes, AsciiTokens, Vec3};
use crate::parser::Triangle;

#[derive(Clone)]
pub struct PointCloud {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub colors: Vec<(u8, u8, u8)>,
}

#[derive(Clone)]
pub enum PlyData {
    Mesh(Vec<Triangle>),
    Points(PointCloud),
}

#[derive(Clone, Copy, PartialEq)]
enum Format { Ascii, BinaryLe, BinaryBe }

#[derive(Clone, Copy)]
enum PT { I8, U8, I16, U16, I32, U32, F32, F64 }

impl PT {
    fn size(self) -> usize {
        match self {
            PT::I8 | PT::U8 => 1,
            PT::I16 | PT::U16 => 2,
            PT::I32 | PT::U32 | PT::F32 => 4,
            PT::F64 => 8,
        }
    }

    #[inline]
    fn read_le(self, d: &[u8], o: usize) -> f64 {
        match self {
            PT::I8 => d[o] as i8 as f64,
            PT::U8 => d[o] as f64,
            PT::I16 => i16::from_le_bytes([d[o], d[o + 1]]) as f64,
            PT::U16 => u16::from_le_bytes([d[o], d[o + 1]]) as f64,
            PT::I32 => i32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::U32 => u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::F32 => f32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::F64 => f64::from_le_bytes([d[o], d[o+1], d[o+2], d[o+3], d[o+4], d[o+5], d[o+6], d[o+7]]),
        }
    }

    #[inline]
    fn read_be(self, d: &[u8], o: usize) -> f64 {
        match self {
            PT::I8 => d[o] as i8 as f64,
            PT::U8 => d[o] as f64,
            PT::I16 => i16::from_be_bytes([d[o], d[o + 1]]) as f64,
            PT::U16 => u16::from_be_bytes([d[o], d[o + 1]]) as f64,
            PT::I32 => i32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::U32 => u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::F32 => f32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as f64,
            PT::F64 => f64::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3], d[o+4], d[o+5], d[o+6], d[o+7]]),
        }
    }
}

fn parse_type(s: &str) -> Result<PT, String> {
    match s {
        "char" | "int8" => Ok(PT::I8),
        "uchar" | "uint8" => Ok(PT::U8),
        "short" | "int16" => Ok(PT::I16),
        "ushort" | "uint16" => Ok(PT::U16),
        "int" | "int32" => Ok(PT::I32),
        "uint" | "uint32" => Ok(PT::U32),
        "float" | "float32" => Ok(PT::F32),
        "double" | "float64" => Ok(PT::F64),
        _ => Err("PLY: unknown type".into()),
    }
}

// -- Property & element layout --

enum Prop { Scalar(PT), List(PT, PT) }

const X: usize = 0;
const Y: usize = 1;
const Z: usize = 2;
const NX: usize = 3;
const NY: usize = 4;
const NZ: usize = 5;
const R: usize = 6;
const G: usize = 7;
const B: usize = 8;
const A: usize = 9;
const NSLOTS: usize = 10;

fn prop_slot(name: &str) -> Option<usize> {
    match name {
        "x" => Some(X), "y" => Some(Y), "z" => Some(Z),
        "nx" => Some(NX), "ny" => Some(NY), "nz" => Some(NZ),
        "red" | "diffuse_red" => Some(R),
        "green" | "diffuse_green" => Some(G),
        "blue" | "diffuse_blue" => Some(B),
        "alpha" | "diffuse_alpha" => Some(A),
        _ => None,
    }
}

struct VertexLayout {
    count: usize,
    props: Vec<PT>,
    prop_offsets: Vec<usize>,
    stride: usize,
    // Pre-resolved slot indices (X/Y/Z validated present in finalize_element)
    sx: usize, sy: usize, sz: usize,
    has_normals: bool,
    snx: usize, sny: usize, snz: usize,
    has_colors: bool,
    sr: usize, sg: usize, sb: usize,
    has_alpha: bool,
    sa: usize,
    /// Index of the first "arbitrary" numeric vertex property — anything
    /// that isn't x/y/z/nx/ny/nz/rgba. Common cases: `quality` (scan
    /// confidence), `intensity`, `value`. Values feed into `color_map:
    /// "ply_scalar"` for heatmap-style rendering.
    has_scalar: bool,
    ss: usize,
}

/// Location of one face color/alpha channel: a byte offset (binary reads),
/// a token index (ASCII reads), and the property's scalar type.
#[derive(Clone, Copy)]
struct ColorSlot { byte_off: u32, tok_off: u32, pt: PT }

/// Location of a per-face color/alpha channel — where to read `red`/`green`/
/// `blue`/`alpha` when they appear as face scalar properties (common in
/// MeshLab / CloudCompare / scanner exports). Callers use `pre` if the
/// property comes BEFORE the face's vertex-index list, otherwise `post`.
#[derive(Default, Clone, Copy)]
struct FaceColorLayout {
    r: Option<ColorSlot>,
    g: Option<ColorSlot>,
    b: Option<ColorSlot>,
    a: Option<ColorSlot>,
}

struct FaceLayout {
    count: usize,
    pre_count: usize,  // number of scalar props before list (for ASCII)
    pre_size: usize,   // byte size of pre-list scalars (for binary)
    count_type: PT,
    index_type: PT,
    post_size: usize,
    post_count: usize,  // number of scalar props after list (for ASCII)
    /// Per-face color/alpha props found in the pre-list region.
    pre_col: FaceColorLayout,
    /// Per-face color/alpha props found in the post-list region.
    post_col: FaceColorLayout,
}

enum Element {
    Vertex(VertexLayout),
    Face(FaceLayout),
    Skip(usize, Vec<Prop>),
}

struct Header {
    format: Format,
    elements: Vec<Element>,
}

// -- Header parsing --

fn find_header_end(data: &[u8]) -> Result<usize, String> {
    let marker = b"end_header";
    for i in 0..data.len().saturating_sub(marker.len()) {
        if &data[i..i + marker.len()] == marker {
            let mut end = i + marker.len();
            if end < data.len() && data[end] == b'\r' { end += 1; }
            if end < data.len() && data[end] == b'\n' { end += 1; }
            return Ok(end);
        }
    }
    Err("PLY: missing end_header".into())
}

fn finalize_element(name: &str, count: usize, props: Vec<(String, Prop)>) -> Result<Element, String> {
    match name {
        "vertex" => {
            let mut scalar_types = Vec::new();
            let mut prop_offsets = Vec::new();
            let mut slots = [None; NSLOTS];
            let mut stride = 0;
            for (i, (pname, prop)) in props.iter().enumerate() {
                match prop {
                    Prop::Scalar(pt) => {
                        if let Some(s) = prop_slot(pname) { slots[s] = Some(i); }
                        prop_offsets.push(stride);
                        stride += pt.size();
                        scalar_types.push(*pt);
                    }
                    Prop::List(_, _) => return Err("PLY: list property in vertex element".into()),
                }
            }
            if slots[X].is_none() || slots[Y].is_none() || slots[Z].is_none() {
                return Err("PLY: vertex missing x, y, or z property".into());
            }
            let sx = slots[X].unwrap();
            let sy = slots[Y].unwrap();
            let sz = slots[Z].unwrap();
            let has_normals = slots[NX].is_some() && slots[NY].is_some() && slots[NZ].is_some();
            let (snx, sny, snz) = if has_normals {
                (slots[NX].unwrap(), slots[NY].unwrap(), slots[NZ].unwrap())
            } else { (0, 0, 0) };
            let has_colors = slots[R].is_some() && slots[G].is_some() && slots[B].is_some();
            let (sr, sg, sb) = if has_colors {
                (slots[R].unwrap(), slots[G].unwrap(), slots[B].unwrap())
            } else { (0, 0, 0) };
            let has_alpha = slots[A].is_some();
            let sa = slots[A].unwrap_or(0);
            // First numeric vertex property that isn't in the standard slots
            // (x/y/z/nx/ny/nz/rgba): interpreted as a scalar for color_map.
            // Skips integer-typed props that are almost certainly not colours
            // themselves — floats (F32/F64) and small ints all pass through
            // color_byte-style normalization when sampled.
            let mut scalar = None;
            for (i, (pname, _)) in props.iter().enumerate() {
                if prop_slot(pname).is_none() {
                    scalar = Some(i);
                    break;
                }
            }
            let has_scalar = scalar.is_some();
            let ss = scalar.unwrap_or(0);
            Ok(Element::Vertex(VertexLayout {
                count, props: scalar_types, prop_offsets, stride,
                sx, sy, sz, has_normals, snx, sny, snz, has_colors, sr, sg, sb,
                has_alpha, sa, has_scalar, ss,
            }))
        }
        "face" => {
            let mut pre_count = 0;
            let mut pre_size = 0;
            let mut post_size = 0;
            let mut post_count = 0;
            let mut list: Option<(PT, PT)> = None;
            let mut pre_col = FaceColorLayout::default();
            let mut post_col = FaceColorLayout::default();
            // Walk scalar props twice-through-once so we can note byte offsets
            // AND token indices for per-face color/alpha channels — MeshLab et
            // al. emit them as face-level `red/green/blue[/alpha]` (usually
            // post-list, hence the split trackers).
            for (pname, prop) in &props {
                match prop {
                    Prop::Scalar(pt) => {
                        let (col_slot, byte_off, tok_off) = if list.is_some() {
                            (&mut post_col, post_size, post_count)
                        } else {
                            (&mut pre_col, pre_size, pre_count)
                        };
                        // Record color/alpha positions if the name matches.
                        // Binary reads use byte_off; ASCII reads use tok_off.
                        let slot = ColorSlot {
                            byte_off: byte_off as u32,
                            tok_off: tok_off as u32,
                            pt: *pt,
                        };
                        match pname.as_str() {
                            "red" | "diffuse_red" => col_slot.r = Some(slot),
                            "green" | "diffuse_green" => col_slot.g = Some(slot),
                            "blue" | "diffuse_blue" => col_slot.b = Some(slot),
                            "alpha" | "diffuse_alpha" => col_slot.a = Some(slot),
                            _ => {}
                        }
                        if list.is_some() {
                            post_size += pt.size();
                            post_count += 1;
                        } else {
                            pre_count += 1;
                            pre_size += pt.size();
                        }
                    }
                    Prop::List(ct, vt) => {
                        if list.is_some() { return Err("PLY: multiple list properties in face".into()); }
                        list = Some((*ct, *vt));
                    }
                }
            }
            let (count_type, index_type) = list.ok_or("PLY: face has no list property")?;
            Ok(Element::Face(FaceLayout {
                count, pre_count, pre_size, count_type, index_type, post_size,
                post_count, pre_col, post_col,
            }))
        }
        _ => Ok(Element::Skip(count, props.into_iter().map(|(_, p)| p).collect())),
    }
}

fn parse_header(data: &[u8]) -> Result<(Header, usize), String> {
    let end = find_header_end(data)?;
    let text = std::str::from_utf8(&data[..end])
        .map_err(|_| "PLY header: invalid UTF-8")?;
    let mut lines = text.lines();

    match lines.next() {
        Some(l) if l.trim_ascii() == "ply" => {}
        _ => return Err("not a PLY file".into()),
    }

    let mut format = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut cur: Option<(String, usize, Vec<(String, Prop)>)> = None;

    for line in lines {
        let line = line.trim_ascii();
        if line.is_empty() || line.starts_with("comment") || line == "end_header" { continue; }
        let mut parts = line.split_ascii_whitespace();
        match parts.next().unwrap_or("") {
            "format" => {
                format = Some(match parts.next().ok_or("PLY: missing format")? {
                    "ascii" => Format::Ascii,
                    "binary_little_endian" => Format::BinaryLe,
                    "binary_big_endian" => Format::BinaryBe,
                    _ => return Err("PLY: unknown format".into()),
                });
            }
            "element" => {
                if let Some((name, count, props)) = cur.take() {
                    elements.push(finalize_element(&name, count, props)?);
                }
                let name = parts.next().ok_or("PLY: missing element name")?.to_string();
                let count: usize = parts.next().ok_or("PLY: missing element count")?
                    .parse().map_err(|_| "PLY: bad element count")?;
                cur = Some((name, count, Vec::new()));
            }
            "property" => {
                let elem = cur.as_mut().ok_or("PLY: property outside element")?;
                let tok = parts.next().ok_or("PLY: missing property type")?;
                if tok == "list" {
                    let ct = parse_type(parts.next().ok_or("PLY: missing list count type")?)?;
                    let vt = parse_type(parts.next().ok_or("PLY: missing list value type")?)?;
                    let name = parts.next().unwrap_or("").to_string();
                    elem.2.push((name, Prop::List(ct, vt)));
                } else {
                    let pt = parse_type(tok)?;
                    let name = parts.next().unwrap_or("").to_string();
                    elem.2.push((name, Prop::Scalar(pt)));
                }
            }
            _ => {}
        }
    }
    if let Some((name, count, props)) = cur {
        elements.push(finalize_element(&name, count, props)?);
    }

    Ok((Header { format: format.ok_or("PLY: missing format line")?, elements }, end))
}

// -- Triangulation helper --

fn triangulate(
    indices: &[usize],
    positions: &[Vec3],
    normals: &[Vec3],
    colors: &[(u8, u8, u8)],
    alphas: &[u8],
    scalars: &[f64],
    face_color: Option<(u8, u8, u8)>,
    face_alpha: Option<u8>,
    out: &mut Vec<Triangle>,
) {
    if indices.len() < 3 { return; }
    let has_n = !normals.is_empty();
    let has_c = !colors.is_empty();
    let has_a = !alphas.is_empty();
    let has_s = !scalars.is_empty();
    for i in 1..indices.len() - 1 {
        let (i0, i1, i2) = (indices[0], indices[i], indices[i + 1]);
        let (v0, v1, v2) = (positions[i0], positions[i1], positions[i2]);
        // Face normal always comes from the geometry (cross product). When
        // per-vertex normals are present, they ship separately in
        // `vertex_normals` and take over during smooth shading — using
        // vertex-0's normal as the face normal (as we did before) both drops
        // information and produces wrong flat-shaded results.
        let normal = Vec3::face_normal(v0, v1, v2).unwrap_or(Vec3::new(0.0, 0.0, 0.0));
        let vertex_normals = if has_n {
            Some([normals[i0], normals[i1], normals[i2]])
        } else {
            None
        };
        // Per-face color wins if the file supplies one (MeshLab / scanner
        // convention); otherwise average vertex colors, otherwise config.
        let (color, vertex_colors) = if let Some(fc) = face_color {
            (Some(fc), None)
        } else if has_c {
            let (c0, c1, c2) = (colors[i0], colors[i1], colors[i2]);
            (Some(crate::color::avg3(c0, c1, c2)), Some([c0, c1, c2]))
        } else {
            (None, None)
        };
        // Per-face alpha wins over per-vertex alpha, same reasoning.
        // Fully-opaque (255) stays None so opaque triangles keep the fast render path.
        let alpha = if let Some(fa) = face_alpha {
            if fa < 255 { Some(fa as f32 / 255.0) } else { None }
        } else if has_a {
            let avg = (alphas[i0] as u16 + alphas[i1] as u16 + alphas[i2] as u16) / 3;
            if avg < 255 { Some(avg as f32 / 255.0) } else { None }
        } else {
            None
        };
        let vertex_scalars = if has_s {
            Some([scalars[i0], scalars[i1], scalars[i2]])
        } else {
            None
        };
        out.push(Triangle { vertices: [v0, v1, v2], normal, color, vertex_colors, group_id: None, alpha, vertex_normals, smoothing_group: None, vertex_scalars });
    }
}

/// Convert an f64 property value to a color channel byte, handling the two
/// PLY conventions (uchar 0..255 or float 0..1) transparently.
#[inline]
fn color_byte(v: f64, pt: PT) -> u8 {
    match pt {
        PT::F32 | PT::F64 => (v.clamp(0.0, 1.0) * 255.0).round() as u8,
        _ => v.clamp(0.0, 255.0) as u8,
    }
}

/// Read the RGB triple encoded in `col` from `toks` (one token per scalar
/// face property in the pre or post region). Returns `None` unless all three
/// channels are present.
fn pick_ascii_face_color(toks: &[&[u8]], col: &FaceColorLayout) -> Option<(u8, u8, u8)> {
    let (r, g, b) = (col.r?, col.g?, col.b?);
    let r_tok = *toks.get(r.tok_off as usize)?;
    let g_tok = *toks.get(g.tok_off as usize)?;
    let b_tok = *toks.get(b.tok_off as usize)?;
    Some((
        color_byte(parse_f64_bytes(r_tok)?, r.pt),
        color_byte(parse_f64_bytes(g_tok)?, g.pt),
        color_byte(parse_f64_bytes(b_tok)?, b.pt),
    ))
}

/// Same, for the alpha channel.
fn pick_ascii_face_alpha(toks: &[&[u8]], col: &FaceColorLayout) -> Option<u8> {
    let a = col.a?;
    let a_tok = *toks.get(a.tok_off as usize)?;
    Some(color_byte(parse_f64_bytes(a_tok)?, a.pt))
}

/// Binary variant: read the RGB triple from bytes at `base + byte_offset`
/// for each channel. `base` points to the start of the pre-list or post-list
/// scalar region for the current face.
fn pick_binary_face_color<const BE: bool>(
    data: &[u8], base: usize, col: &FaceColorLayout,
) -> Option<(u8, u8, u8)> {
    let (r, g, b) = (col.r?, col.g?, col.b?);
    let r_off = base + r.byte_off as usize;
    let g_off = base + g.byte_off as usize;
    let b_off = base + b.byte_off as usize;
    if data.len() < r_off + r.pt.size() || data.len() < g_off + g.pt.size() || data.len() < b_off + b.pt.size() {
        return None;
    }
    let rd = |pt: PT, o: usize| if BE { pt.read_be(data, o) } else { pt.read_le(data, o) };
    Some((color_byte(rd(r.pt, r_off), r.pt),
          color_byte(rd(g.pt, g_off), g.pt),
          color_byte(rd(b.pt, b_off), b.pt)))
}

/// Binary variant: read the alpha channel.
fn pick_binary_face_alpha<const BE: bool>(
    data: &[u8], base: usize, col: &FaceColorLayout,
) -> Option<u8> {
    let a = col.a?;
    let a_off = base + a.byte_off as usize;
    if data.len() < a_off + a.pt.size() { return None; }
    let v = if BE { a.pt.read_be(data, a_off) } else { a.pt.read_le(data, a_off) };
    Some(color_byte(v, a.pt))
}

// -- Face index collection (shared by ASCII and binary paths) --

fn collect_face_indices<'a>(
    face_n: usize,
    nv: usize,
    stack_buf: &'a mut [usize; 8],
    heap_buf: &'a mut Vec<usize>,
    mut read_one: impl FnMut() -> Result<usize, String>,
) -> Result<&'a [usize], String> {
    if face_n <= 8 {
        for j in 0..face_n {
            let idx = read_one()?;
            if idx >= nv { return Err("PLY: face index out of range".into()); }
            stack_buf[j] = idx;
        }
        Ok(&stack_buf[..face_n])
    } else {
        heap_buf.clear();
        for _ in 0..face_n {
            let idx = read_one()?;
            if idx >= nv { return Err("PLY: face index out of range".into()); }
            heap_buf.push(idx);
        }
        Ok(heap_buf)
    }
}

// -- ASCII body parsing --

fn parse_ascii(header: &Header, data: &[u8]) -> Result<PlyData, String> {
    // Parse the ASCII body straight from bytes: split on '\n', trim ASCII
    // whitespace (handles trailing '\r'), skip blanks — no whole-buffer
    // from_utf8 / Unicode .lines().
    let mut lines = data.split(|&c| c == b'\n').map(|l| l.trim_ascii()).filter(|l| !l.is_empty());

    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
    let mut alphas: Vec<u8> = Vec::new();
    let mut scalars: Vec<f64> = Vec::new();
    let mut triangles: Vec<Triangle> = Vec::new();

    for elem in &header.elements {
        match elem {
            Element::Vertex(vl) => {
                positions.reserve(vl.count);
                let n_props = vl.props.len().min(16);
                // Only parse properties we actually need
                let mut needed = [false; 16];
                needed[vl.sx] = true; needed[vl.sy] = true; needed[vl.sz] = true;
                if vl.has_normals { needed[vl.snx] = true; needed[vl.sny] = true; needed[vl.snz] = true; }
                if vl.has_colors { needed[vl.sr] = true; needed[vl.sg] = true; needed[vl.sb] = true; }
                if vl.has_alpha { needed[vl.sa] = true; }
                if vl.has_scalar { needed[vl.ss] = true; }

                for _ in 0..vl.count {
                    let line = lines.next().ok_or("PLY: unexpected end of vertex data")?;
                    let b = line;
                    let len = b.len();
                    let mut buf = [0.0f64; 16];
                    let mut i = 0;
                    let mut prop = 0;
                    while i < len && prop < n_props {
                        while i < len && b[i] <= b' ' { i += 1; }
                        if i >= len { break; }
                        let start = i;
                        while i < len && b[i] > b' ' { i += 1; }
                        if needed[prop] {
                            buf[prop] = parse_f64_bytes(&b[start..i]).unwrap_or(0.0);
                        }
                        prop += 1;
                    }
                    if prop < n_props {
                        return Err("PLY: vertex has too few values".into());
                    }
                    positions.push(Vec3::new(buf[vl.sx], buf[vl.sy], buf[vl.sz]));
                    if vl.has_normals {
                        normals.push(Vec3::new(buf[vl.snx], buf[vl.sny], buf[vl.snz]));
                    }
                    if vl.has_colors {
                        colors.push((buf[vl.sr] as u8, buf[vl.sg] as u8, buf[vl.sb] as u8));
                    }
                    if vl.has_alpha {
                        alphas.push(buf[vl.sa] as u8);
                    }
                    if vl.has_scalar {
                        scalars.push(buf[vl.ss]);
                    }
                }
            }
            Element::Face(fl) => {
                let nv = positions.len();
                let mut stack_buf = [0usize; 8];
                let mut heap_buf = Vec::new();
                let mut pre_toks: Vec<&[u8]> = Vec::with_capacity(fl.pre_count);
                let mut post_toks: Vec<&[u8]> = Vec::with_capacity(fl.post_count);
                for _ in 0..fl.count {
                    let line = lines.next().ok_or("PLY: unexpected end of face data")?;
                    let mut tokens = AsciiTokens::new(line);
                    // Collect pre-list scalars — we need them to look up any
                    // per-face color that comes before the vertex list.
                    pre_toks.clear();
                    for _ in 0..fl.pre_count {
                        pre_toks.push(tokens.next().ok_or("PLY: face line too short (pre)")?);
                    }
                    let face_n = parse_i64_bytes(tokens.next().ok_or("PLY: empty face line")?)
                        .ok_or("PLY: bad face count")? as usize;
                    let indices = collect_face_indices(face_n, nv, &mut stack_buf, &mut heap_buf, || {
                        parse_i64_bytes(tokens.next().ok_or("PLY: face line too short")?)
                            .ok_or_else(|| "PLY: bad index".into())
                            .map(|v| v as usize)
                    })?;
                    // Collect post-list scalars (usually where face RGB lives).
                    post_toks.clear();
                    for _ in 0..fl.post_count {
                        post_toks.push(tokens.next().ok_or("PLY: face line too short (post)")?);
                    }
                    let face_color = pick_ascii_face_color(&pre_toks, &fl.pre_col)
                        .or_else(|| pick_ascii_face_color(&post_toks, &fl.post_col));
                    let face_alpha = pick_ascii_face_alpha(&pre_toks, &fl.pre_col)
                        .or_else(|| pick_ascii_face_alpha(&post_toks, &fl.post_col));
                    triangulate(indices, &positions, &normals, &colors, &alphas, &scalars, face_color, face_alpha, &mut triangles);
                }
            }
            Element::Skip(count, _) => {
                for _ in 0..*count { lines.next(); }
            }
        }
    }
    if triangles.is_empty() && !positions.is_empty() {
        Ok(PlyData::Points(PointCloud { positions, normals, colors }))
    } else {
        Ok(PlyData::Mesh(triangles))
    }
}

// -- Binary body parsing (specialized per endianness to eliminate runtime branch) --

fn parse_binary(header: &Header, data: &[u8], be: bool) -> Result<PlyData, String> {
    if be {
        parse_binary_endian::<true>(header, data)
    } else {
        parse_binary_endian::<false>(header, data)
    }
}

fn parse_binary_endian<const BE: bool>(header: &Header, data: &[u8]) -> Result<PlyData, String> {
    let mut off = 0usize;

    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
    let mut alphas: Vec<u8> = Vec::new();
    let mut scalars: Vec<f64> = Vec::new();
    let mut triangles: Vec<Triangle> = Vec::new();

    // Inline reader that uses const generic to eliminate runtime branch
    #[inline(always)]
    fn read<const BE: bool>(pt: PT, d: &[u8], o: usize) -> f64 {
        if BE { pt.read_be(d, o) } else { pt.read_le(d, o) }
    }

    for elem in &header.elements {
        match elem {
            Element::Vertex(vl) => {
                positions.reserve(vl.count);
                let (ptx, ox) = (vl.props[vl.sx], vl.prop_offsets[vl.sx]);
                let (pty, oy) = (vl.props[vl.sy], vl.prop_offsets[vl.sy]);
                let (ptz, oz) = (vl.props[vl.sz], vl.prop_offsets[vl.sz]);

                let (ptnx, onx, ptny, ony, ptnz, onz) = if vl.has_normals {
                    (vl.props[vl.snx], vl.prop_offsets[vl.snx],
                     vl.props[vl.sny], vl.prop_offsets[vl.sny],
                     vl.props[vl.snz], vl.prop_offsets[vl.snz])
                } else {
                    (PT::U8, 0, PT::U8, 0, PT::U8, 0)
                };

                let (ptr, or, ptg, og, ptb, ob) = if vl.has_colors {
                    (vl.props[vl.sr], vl.prop_offsets[vl.sr],
                     vl.props[vl.sg], vl.prop_offsets[vl.sg],
                     vl.props[vl.sb], vl.prop_offsets[vl.sb])
                } else {
                    (PT::U8, 0, PT::U8, 0, PT::U8, 0)
                };

                let (pta, oa) = if vl.has_alpha {
                    (vl.props[vl.sa], vl.prop_offsets[vl.sa])
                } else {
                    (PT::U8, 0)
                };

                let (pts, os) = if vl.has_scalar {
                    (vl.props[vl.ss], vl.prop_offsets[vl.ss])
                } else {
                    (PT::U8, 0)
                };

                for _ in 0..vl.count {
                    if off + vl.stride > data.len() {
                        return Err("PLY: truncated vertex data".into());
                    }
                    positions.push(Vec3::new(
                        read::<BE>(ptx, data, off + ox),
                        read::<BE>(pty, data, off + oy),
                        read::<BE>(ptz, data, off + oz),
                    ));
                    if vl.has_normals {
                        normals.push(Vec3::new(
                            read::<BE>(ptnx, data, off + onx),
                            read::<BE>(ptny, data, off + ony),
                            read::<BE>(ptnz, data, off + onz),
                        ));
                    }
                    if vl.has_colors {
                        colors.push((
                            read::<BE>(ptr, data, off + or) as u8,
                            read::<BE>(ptg, data, off + og) as u8,
                            read::<BE>(ptb, data, off + ob) as u8,
                        ));
                    }
                    if vl.has_alpha {
                        alphas.push(read::<BE>(pta, data, off + oa) as u8);
                    }
                    if vl.has_scalar {
                        scalars.push(read::<BE>(pts, data, off + os));
                    }
                    off += vl.stride;
                }
            }
            Element::Face(fl) => {
                let nv = positions.len();
                let isz = fl.index_type.size();
                let csz = fl.count_type.size();
                // Specialize index reader to avoid per-index match
                let read_idx: fn(&[u8], usize) -> usize = match (fl.index_type, BE) {
                    (PT::U8, _) => |d, o| d[o] as usize,
                    (PT::I8, _) => |d, o| d[o] as i8 as usize,
                    (PT::U16, false) => |d, o| u16::from_le_bytes([d[o], d[o+1]]) as usize,
                    (PT::U16, true) => |d, o| u16::from_be_bytes([d[o], d[o+1]]) as usize,
                    (PT::I16, false) => |d, o| i16::from_le_bytes([d[o], d[o+1]]) as usize,
                    (PT::I16, true) => |d, o| i16::from_be_bytes([d[o], d[o+1]]) as usize,
                    (PT::U32, false) => |d, o| u32::from_le_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::U32, true) => |d, o| u32::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::I32, false) => |d, o| i32::from_le_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::I32, true) => |d, o| i32::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::F32, false) => |d, o| f32::from_le_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::F32, true) => |d, o| f32::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3]]) as usize,
                    (PT::F64, false) => |d, o| f64::from_le_bytes([d[o], d[o+1], d[o+2], d[o+3], d[o+4], d[o+5], d[o+6], d[o+7]]) as usize,
                    (PT::F64, true) => |d, o| f64::from_be_bytes([d[o], d[o+1], d[o+2], d[o+3], d[o+4], d[o+5], d[o+6], d[o+7]]) as usize,
                };
                let mut stack_buf = [0usize; 8];
                let mut heap_buf = Vec::new();
                for _ in 0..fl.count {
                    // Grab per-face color/alpha from the pre-list scalars
                    // BEFORE stepping over them (needs the base offset).
                    let face_color_pre = pick_binary_face_color::<BE>(data, off, &fl.pre_col);
                    let face_alpha_pre = pick_binary_face_alpha::<BE>(data, off, &fl.pre_col);
                    off += fl.pre_size;
                    if off + csz > data.len() {
                        return Err("PLY: truncated face data".into());
                    }
                    let face_n = read::<BE>(fl.count_type, data, off) as usize;
                    off += csz;

                    let indices = collect_face_indices(face_n, nv, &mut stack_buf, &mut heap_buf, || {
                        if off + isz > data.len() {
                            return Err("PLY: truncated face data".into());
                        }
                        let idx = read_idx(data, off);
                        off += isz;
                        Ok(idx)
                    })?;
                    // Now `off` sits at the start of the post-list scalars.
                    let face_color_post = pick_binary_face_color::<BE>(data, off, &fl.post_col);
                    let face_alpha_post = pick_binary_face_alpha::<BE>(data, off, &fl.post_col);
                    off += fl.post_size;
                    let face_color = face_color_pre.or(face_color_post);
                    let face_alpha = face_alpha_pre.or(face_alpha_post);
                    triangulate(indices, &positions, &normals, &colors, &alphas, &scalars, face_color, face_alpha, &mut triangles);
                }
            }
            Element::Skip(count, props) => {
                for _ in 0..*count {
                    for p in props {
                        match p {
                            Prop::Scalar(pt) => off += pt.size(),
                            Prop::List(ct, vt) => {
                                let n = read::<BE>(*ct, data, off) as usize;
                                off += ct.size() + n * vt.size();
                            }
                        }
                    }
                }
            }
        }
    }
    if triangles.is_empty() && !positions.is_empty() {
        Ok(PlyData::Points(PointCloud { positions, normals, colors }))
    } else {
        Ok(PlyData::Mesh(triangles))
    }
}

// -- Public API --

pub fn parse_ply(data: &[u8]) -> Result<PlyData, String> {
    let (header, body_start) = parse_header(data)?;
    let body = &data[body_start..];
    match header.format {
        Format::Ascii => parse_ascii(&header, body),
        Format::BinaryLe => parse_binary(&header, body, false),
        Format::BinaryBe => parse_binary(&header, body, true),
    }
}
