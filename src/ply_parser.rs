use crate::math::{parse_f64_fast, parse_i64_fast, Vec3};
use crate::parser::Triangle;

pub struct PointCloud {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub colors: Vec<(u8, u8, u8)>,
}

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

fn prop_slot(name: &str) -> Option<usize> {
    match name {
        "x" => Some(X), "y" => Some(Y), "z" => Some(Z),
        "nx" => Some(NX), "ny" => Some(NY), "nz" => Some(NZ),
        "red" | "diffuse_red" => Some(R),
        "green" | "diffuse_green" => Some(G),
        "blue" | "diffuse_blue" => Some(B),
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
}

struct FaceLayout {
    count: usize,
    pre_count: usize,  // number of scalar props before list (for ASCII)
    pre_size: usize,   // byte size of pre-list scalars (for binary)
    count_type: PT,
    index_type: PT,
    post_size: usize,
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
            let mut slots = [None; 9];
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
            Ok(Element::Vertex(VertexLayout {
                count, props: scalar_types, prop_offsets, stride,
                sx, sy, sz, has_normals, snx, sny, snz, has_colors, sr, sg, sb,
            }))
        }
        "face" => {
            let mut pre_count = 0;
            let mut pre_size = 0;
            let mut post_size = 0;
            let mut list: Option<(PT, PT)> = None;
            for (_, prop) in &props {
                match prop {
                    Prop::Scalar(pt) => {
                        if list.is_some() { post_size += pt.size(); } else { pre_count += 1; pre_size += pt.size(); }
                    }
                    Prop::List(ct, vt) => {
                        if list.is_some() { return Err("PLY: multiple list properties in face".into()); }
                        list = Some((*ct, *vt));
                    }
                }
            }
            let (count_type, index_type) = list.ok_or("PLY: face has no list property")?;
            Ok(Element::Face(FaceLayout { count, pre_count, pre_size, count_type, index_type, post_size }))
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
        Some(l) if l.trim() == "ply" => {}
        _ => return Err("not a PLY file".into()),
    }

    let mut format = None;
    let mut elements: Vec<Element> = Vec::new();
    let mut cur: Option<(String, usize, Vec<(String, Prop)>)> = None;

    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with("comment") || line == "end_header" { continue; }
        let mut parts = line.split_whitespace();
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
    out: &mut Vec<Triangle>,
) {
    if indices.len() < 3 { return; }
    let has_n = !normals.is_empty();
    let has_c = !colors.is_empty();
    for i in 1..indices.len() - 1 {
        let (i0, i1, i2) = (indices[0], indices[i], indices[i + 1]);
        let (v0, v1, v2) = (positions[i0], positions[i1], positions[i2]);
        let normal = if has_n { normals[i0] } else { (v1 - v0).cross(v2 - v0).normalized() };
        let (color, vertex_colors) = if has_c {
            let (c0, c1, c2) = (colors[i0], colors[i1], colors[i2]);
            (Some((
                ((c0.0 as u16 + c1.0 as u16 + c2.0 as u16) / 3) as u8,
                ((c0.1 as u16 + c1.1 as u16 + c2.1 as u16) / 3) as u8,
                ((c0.2 as u16 + c1.2 as u16 + c2.2 as u16) / 3) as u8,
            )), Some([c0, c1, c2]))
        } else {
            (None, None)
        };
        out.push(Triangle { vertices: [v0, v1, v2], normal, color, vertex_colors, group_id: None });
    }
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
    let text = std::str::from_utf8(data).map_err(|_| "PLY: invalid UTF-8")?;
    let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty());

    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
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

                for _ in 0..vl.count {
                    let line = lines.next().ok_or("PLY: unexpected end of vertex data")?;
                    let b = line.as_bytes();
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
                            buf[prop] = parse_f64_fast(&line[start..i]).unwrap_or(0.0);
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
                }
            }
            Element::Face(fl) => {
                let nv = positions.len();
                let tok_off = fl.pre_count;
                let mut stack_buf = [0usize; 8];
                let mut heap_buf = Vec::new();
                for _ in 0..fl.count {
                    let line = lines.next().ok_or("PLY: unexpected end of face data")?;
                    let mut tokens = line.split_whitespace();
                    for _ in 0..tok_off { tokens.next(); }
                    let face_n = parse_i64_fast(tokens.next().ok_or("PLY: empty face line")?)
                        .ok_or("PLY: bad face count")? as usize;
                    let indices = collect_face_indices(face_n, nv, &mut stack_buf, &mut heap_buf, || {
                        parse_i64_fast(tokens.next().ok_or("PLY: face line too short")?)
                            .ok_or_else(|| "PLY: bad index".into())
                            .map(|v| v as usize)
                    })?;
                    triangulate(indices, &positions, &normals, &colors, &mut triangles);
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
                    off += fl.post_size;
                    triangulate(indices, &positions, &normals, &colors, &mut triangles);
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
