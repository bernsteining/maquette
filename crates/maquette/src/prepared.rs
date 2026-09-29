//! Pre-parsed mesh blobs. `prepare_*` parses a model once and returns it in
//! this format; the Typst wrapper passes the blob to every render instead of
//! the raw file. Typst memoizes the `prepare_*` call and shares its result
//! across threads, so each model is parsed once per document even though every
//! layout thread runs its own plugin instance. Decoding is much cheaper than
//! parsing, and the header carries the raw file's cache key so renders skip
//! re-hashing the input.
//!
//! Layout (little-endian): `MAGIC`, key `u64`, triangle count `u32`, then per
//! triangle a `u16` presence mask and its fields, then group count `u32` and
//! per group its id `u32` and name (`u32` length, `u32::MAX` = none, bytes).

use crate::config::{GroupAppearance, GroupStyles};
use crate::math::Vec3;
use crate::parser::Triangle;

const MAGIC: &[u8; 8] = b"\0MQMESH\x01";

const COLOR: u16 = 1;
const VERTEX_COLORS: u16 = 1 << 1;
const GROUP: u16 = 1 << 2;
const ALPHA: u16 = 1 << 3;
const VERTEX_NORMALS: u16 = 1 << 4;
const SMOOTHING: u16 = 1 << 5;
const SCALARS: u16 = 1 << 6;
const UVS: u16 = 1 << 7;
const TEX: u16 = 1 << 8;

/// The raw file's cache key if `data` is a prepared blob.
pub fn key_of(data: &[u8]) -> Option<u64> {
    maquette_core::cache::handle_key(MAGIC, data)
}

/// Header-only blob for `key`: accepted wherever a prepared blob is, and served
/// from the per-instance model cache when that key is already parsed there
/// (otherwise decoding it fails as truncated).
pub fn header(key: u64) -> Vec<u8> {
    maquette_core::cache::handle(MAGIC, key)
}

pub fn encode(key: u64, triangles: &[Triangle], groups: &GroupStyles) -> Vec<u8> {
    let mut out = header(key);
    out.reserve(4 + triangles.len() * 104);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    let f64s = |out: &mut Vec<u8>, v: &[f64]| v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
    let vec3 = |out: &mut Vec<u8>, v: Vec3| f64s(out, &[v.x, v.y, v.z]);
    for t in triangles {
        let mask = t.color.map_or(0, |_| COLOR)
            | t.vertex_colors.map_or(0, |_| VERTEX_COLORS)
            | t.group_id.map_or(0, |_| GROUP)
            | t.alpha.map_or(0, |_| ALPHA)
            | t.vertex_normals.map_or(0, |_| VERTEX_NORMALS)
            | t.smoothing_group.map_or(0, |_| SMOOTHING)
            | t.vertex_scalars.map_or(0, |_| SCALARS)
            | t.uvs.map_or(0, |_| UVS)
            | t.tex.map_or(0, |_| TEX);
        out.extend_from_slice(&mask.to_le_bytes());
        t.vertices.iter().for_each(|&v| vec3(&mut out, v));
        vec3(&mut out, t.normal);
        if let Some((r, g, b)) = t.color { out.extend_from_slice(&[r, g, b]); }
        if let Some(c) = t.vertex_colors { c.iter().for_each(|&(r, g, b)| out.extend_from_slice(&[r, g, b])); }
        if let Some(g) = t.group_id { out.extend_from_slice(&g.to_le_bytes()); }
        if let Some(a) = t.alpha { out.extend_from_slice(&a.to_le_bytes()); }
        if let Some(n) = t.vertex_normals { n.iter().for_each(|&v| vec3(&mut out, v)); }
        if let Some(s) = t.smoothing_group { out.extend_from_slice(&s.to_le_bytes()); }
        if let Some(s) = t.vertex_scalars { f64s(&mut out, &s); }
        if let Some(uv) = t.uvs { uv.iter().flatten().for_each(|x| out.extend_from_slice(&x.to_le_bytes())); }
        if let Some(i) = t.tex { out.extend_from_slice(&i.to_le_bytes()); }
    }
    let mut ids: Vec<&u32> = groups.keys().collect();
    ids.sort();
    out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    for id in ids {
        out.extend_from_slice(&id.to_le_bytes());
        match &groups[id].name {
            Some(name) => {
                out.extend_from_slice(&(name.len() as u32).to_le_bytes());
                out.extend_from_slice(name.as_bytes());
            }
            None => out.extend_from_slice(&u32::MAX.to_le_bytes()),
        }
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.pos + N;
        let s = self.b.get(self.pos..end).ok_or("prepared mesh: truncated")?;
        self.pos = end;
        Ok(s.try_into().unwrap())
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("prepared mesh: truncated")?;
        let s = self.b.get(self.pos..end).ok_or("prepared mesh: truncated")?;
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> { Ok(u16::from_le_bytes(self.take()?)) }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.take()?)) }
    fn f32(&mut self) -> Result<f32, String> { Ok(f32::from_le_bytes(self.take()?)) }
    fn f64(&mut self) -> Result<f64, String> { Ok(f64::from_le_bytes(self.take()?)) }
    fn rgb(&mut self) -> Result<(u8, u8, u8), String> { let [r, g, b] = self.take()?; Ok((r, g, b)) }
    fn vec3(&mut self) -> Result<Vec3, String> {
        let b: [u8; 24] = self.take()?;
        let f = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Ok(Vec3::new(f(0), f(8), f(16)))
    }
}

pub fn decode(data: &[u8]) -> Result<(Vec<Triangle>, GroupStyles), String> {
    key_of(data).ok_or("prepared mesh: bad header")?;
    let mut r = Reader { b: data, pos: 16 };
    let n = r.u32()? as usize;
    let mut triangles = Vec::with_capacity(n.min(data.len() / 98));
    for _ in 0..n {
        let mask = r.u16()?;
        let vertices = [r.vec3()?, r.vec3()?, r.vec3()?];
        let normal = r.vec3()?;
        let has = |bit: u16| mask & bit != 0;
        let color = if has(COLOR) { Some(r.rgb()?) } else { None };
        let vertex_colors = if has(VERTEX_COLORS) { Some([r.rgb()?, r.rgb()?, r.rgb()?]) } else { None };
        let group_id = if has(GROUP) { Some(r.u32()?) } else { None };
        let alpha = if has(ALPHA) { Some(r.f32()?) } else { None };
        let vertex_normals = if has(VERTEX_NORMALS) { Some([r.vec3()?, r.vec3()?, r.vec3()?]) } else { None };
        let smoothing_group = if has(SMOOTHING) { Some(r.u32()?) } else { None };
        let vertex_scalars = if has(SCALARS) { Some([r.f64()?, r.f64()?, r.f64()?]) } else { None };
        let uvs = if has(UVS) {
            Some([[r.f32()?, r.f32()?], [r.f32()?, r.f32()?], [r.f32()?, r.f32()?]])
        } else {
            None
        };
        let tex = if has(TEX) { Some(r.u16()?) } else { None };
        triangles.push(Triangle { splat: false,
            vertices, normal, color, vertex_colors, group_id, alpha,
            vertex_normals, smoothing_group, vertex_scalars, uvs, tex,
        });
    }
    let ng = r.u32()? as usize;
    let mut groups = crate::math::fx_hashmap_cap(ng.min(data.len()));
    for _ in 0..ng {
        let id = r.u32()?;
        let len = r.u32()?;
        let name = if len == u32::MAX {
            None
        } else {
            Some(String::from_utf8(r.bytes(len as usize)?.to_vec()).map_err(|_| "prepared mesh: bad group name")?)
        };
        groups.insert(id, GroupAppearance { name, ..Default::default() });
    }
    if r.pos != data.len() {
        return Err("prepared mesh: trailing bytes".into());
    }
    Ok((triangles, groups))
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, key_of};
    use std::collections::HashMap;

    fn models(ext: &str) -> Vec<std::path::PathBuf> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/data");
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); } else if p.extension().is_some_and(|x| x == ext) { out.push(p); }
            }
        }
        out
    }

    #[test]
    fn round_trips_every_example_mesh() {
        let mut checked = 0;
        for p in models("obj") {
            let data = std::fs::read(&p).unwrap();
            let Ok((tris, groups)) = crate::obj_parser::parse_obj(&data, &HashMap::new(), &HashMap::new(), &HashMap::new()) else { continue };
            let blob = encode(7, &tris, &groups);
            assert_eq!(key_of(&blob), Some(7));
            let (t2, g2) = decode(&blob).unwrap();
            assert_eq!(encode(7, &t2, &g2), blob, "{}", p.display());
            assert_eq!(t2.len(), tris.len());
            checked += 1;
        }
        for p in models("stl") {
            let data = std::fs::read(&p).unwrap();
            let Ok(tris) = crate::parser::parse_stl(&data) else { continue };
            let blob = encode(9, &tris, &crate::config::GroupStyles::default());
            let (t2, g2) = decode(&blob).unwrap();
            assert!(g2.is_empty());
            assert_eq!(encode(9, &t2, &g2), blob, "{}", p.display());
            checked += 1;
        }
        assert!(checked > 5);
        assert!(decode(&encode(1, &[], &crate::config::GroupStyles::default())[..15]).is_err());
    }
}
