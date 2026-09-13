//! Packed sidecar bundle: a `HashMap<String, Vec<u8>>` serialised to a single
//! byte blob so a Typst wrapper can hand a plugin all the external files a
//! model references (glTF `.bin`/images, OBJ `.mtl`/textures) through one
//! `read`-driven argument. Shared by the maquette family so the wire format is
//! defined in exactly one place.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! [n: u32]                          number of entries
//! repeat n times:
//!     [name_len: u16][name: bytes]  UTF-8 key (uri / filename)
//!     [off: u32][len: u32]          blob slice within this same buffer
//! [blobs...]                        the concatenated file bodies
//! ```

use std::collections::HashMap;

/// Decode a packed sidecar bundle. Returns an empty map for a 0-byte input so
/// callers can uniformly pass an empty bundle when they have no sidecars. Any
/// structural inconsistency (truncation, offsets past end) is a hard error —
/// the wrapper packs deterministically, so a bad bundle indicates a bug we
/// want to surface immediately.
pub fn parse_sidecar_bundle(bundle: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    if bundle.is_empty() { return Ok(HashMap::new()); }
    if bundle.len() < 4 { return Err("sidecar bundle: header truncated".into()); }
    let n = u32::from_le_bytes(bundle[0..4].try_into().unwrap()) as usize;
    let mut entries: Vec<(String, usize, usize)> = Vec::with_capacity(n);
    let mut pos = 4usize;
    for _ in 0..n {
        if pos + 2 > bundle.len() { return Err("sidecar bundle: name_len truncated".into()); }
        let name_len = u16::from_le_bytes(bundle[pos..pos+2].try_into().unwrap()) as usize;
        pos += 2;
        if pos + name_len > bundle.len() { return Err("sidecar bundle: name truncated".into()); }
        let name = std::str::from_utf8(&bundle[pos..pos+name_len])
            .map_err(|_| "sidecar bundle: non-utf8 name")?.to_string();
        pos += name_len;
        if pos + 8 > bundle.len() { return Err("sidecar bundle: entry offset/length truncated".into()); }
        let off = u32::from_le_bytes(bundle[pos..pos+4].try_into().unwrap()) as usize;
        pos += 4;
        let len = u32::from_le_bytes(bundle[pos..pos+4].try_into().unwrap()) as usize;
        pos += 4;
        entries.push((name, off, len));
    }
    let mut map = HashMap::with_capacity(entries.len());
    for (name, off, len) in entries {
        let end = off.checked_add(len).ok_or("sidecar bundle: offset overflow")?;
        if end > bundle.len() {
            return Err(format!("sidecar bundle: entry '{}' body out of range", name));
        }
        map.insert(name, bundle[off..end].to_vec());
    }
    Ok(map)
}
