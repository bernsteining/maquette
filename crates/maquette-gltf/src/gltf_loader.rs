/// Parse a glTF or GLB byte slice into a `gltf::Gltf` plus the associated
/// buffer data.
///
/// Three input shapes are supported:
///   1. `.glb` (single file, JSON + BIN chunk together) — pass via `parse`.
///   2. `.gltf` with every buffer/image embedded as a `data:` URI — `parse`.
///   3. `.gltf` split into external `.bin`/image files — `parse_split`, with
///      the sidecar files packed into a bundle by the Typst wrapper.
///
/// The wasm sandbox has no filesystem access, so external URIs are resolved
/// via the sidecar bundle instead. The wrapper walks the glTF JSON, reads
/// each referenced URI, and hands the plugin a packed `HashMap<uri, bytes>`.

use gltf::Gltf;
use std::collections::HashMap;

/// Parsed asset ready for scene traversal. Owns the buffer bytes so callers
/// can pass short-lived byte slices into `parse`.
pub struct LoadedGltf {
    pub document: gltf::Document,
    /// One entry per glTF buffer, in `buffers[]` order. For GLB, `buffers[0]`
    /// is the BIN chunk. Buffers we couldn't resolve stay empty — the scene
    /// traversal will treat any primitive that references them as unrenderable.
    pub buffers: Vec<Vec<u8>>,
    /// Sidecar files, keyed by URI. Populated for split `.gltf` inputs;
    /// consumed by `scene::load_texture` when an image references an external
    /// URI. Empty for GLB / fully-embedded `.gltf`.
    pub sidecars: HashMap<String, Vec<u8>>,
}

pub fn parse(bytes: &[u8]) -> Result<LoadedGltf, String> {
    parse_impl(bytes, HashMap::new())
}

/// Split-glTF variant. `sidecars_bundle` is a packed `HashMap<uri, bytes>`
/// produced by the Typst wrapper walking the glTF JSON. Format:
///   `[n_files u32 LE]`
///   `for i in 0..n: [name_len u16 LE][name utf-8][data_off u32 LE][data_len u32 LE]`
///   `[concatenated file bodies at their offsets]`
pub fn parse_split(bytes: &[u8], sidecars_bundle: &[u8]) -> Result<LoadedGltf, String> {
    let sidecars = parse_sidecar_bundle(sidecars_bundle)?;
    parse_impl(bytes, sidecars)
}

fn parse_impl(bytes: &[u8], sidecars: HashMap<String, Vec<u8>>) -> Result<LoadedGltf, String> {
    // `from_slice_without_validation` skips the `extensionsRequired`
    // whitelist check — otherwise gltf-rs rejects files declaring
    // EXT_meshopt_compression or KHR_mesh_quantization since they're not in
    // its ENABLED_EXTENSIONS list. We handle meshopt below and quantization
    // is transparent (accessor `.into_f32()` converts from any format).
    let Gltf { document, blob } = Gltf::from_slice_without_validation(bytes)
        .map_err(|e| format!("glTF parse error: {}", e))?;

    let mut buffers: Vec<Vec<u8>> = Vec::with_capacity(document.buffers().len());
    for buffer in document.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                // GLB internal chunk. Present exactly once, as buffer index 0.
                let blob = blob.as_ref().ok_or_else(||
                    "glTF references BIN buffer but the input is not a GLB".to_string())?;
                buffers.push(blob.clone());
            }
            gltf::buffer::Source::Uri(uri) => {
                if let Some(bytes) = decode_data_uri(uri) {
                    buffers.push(bytes);
                } else if let Some(bytes) = sidecars.get(uri) {
                    // External `.bin` — resolved via the sidecar bundle the
                    // wrapper packed. Clone into an owned Vec (matches the
                    // storage of the other two branches).
                    buffers.push(bytes.clone());
                } else {
                    return Err(format!(
                        "glTF buffer {} references external file '{}' but no \
                         matching sidecar was provided. If you're calling the \
                         plugin directly, pass the file's bytes in the sidecar \
                         bundle; if you're using the wrapper, make sure the \
                         file exists next to the .gltf.",
                        buffer.index(), uri));
                }
            }
        }
    }

    // EXT_meshopt_compression: bufferViews may carry a compressed source
    // pointing at a different buffer region. Decompress into the target
    // bufferView's placeholder location so accessor reads see raw data.
    // Runs before scene traversal — gltf-rs never sees the compressed bytes.
    decompress_meshopt(&document, &mut buffers)?;

    Ok(LoadedGltf { document, buffers, sidecars })
}

/// Decode the packed sidecar bundle produced by the wrapper. Returns an empty
/// map for a 0-byte input so callers can uniformly pass an empty bundle when
/// they have no sidecars. Any structural inconsistency (truncation, offsets
/// past end) is a hard error — the wrapper packs deterministically, so a bad
/// bundle indicates a bug we want to surface immediately.
fn parse_sidecar_bundle(bundle: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
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

/// Walk every bufferView and, if it declares EXT_meshopt_compression, decode
/// the compressed source into the target bufferView's region. See the extension
/// spec:
///   https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Vendor/EXT_meshopt_compression
///
/// The compressed data lives at `(ext.buffer, ext.byteOffset, ext.byteLength)`
/// and decodes to exactly `count · byteStride` bytes; those bytes overwrite
/// the bufferView's own `(buffer, byteOffset, byteLength)` region. The target
/// region typically comes from a "fallback" (zero-filled) buffer whose
/// `extensions.EXT_meshopt_compression.fallback` flag is `true`.
fn decompress_meshopt(document: &gltf::Document, buffers: &mut Vec<Vec<u8>>) -> Result<(), String> {
    // Collect the (target_buffer, target_offset, target_length, decoded_bytes)
    // triples first, then apply them — avoids double-borrowing `buffers` when
    // source and target buffer indices happen to match.
    let mut patches: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for view in document.views() {
        let Some(ext) = view.extension_value("EXT_meshopt_compression") else { continue; };
        let src_buffer = ext.get("buffer").and_then(|v| v.as_u64()).ok_or("meshopt: missing buffer")? as usize;
        let src_offset = ext.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let src_length = ext.get("byteLength").and_then(|v| v.as_u64()).ok_or("meshopt: missing byteLength")? as usize;
        let stride = ext.get("byteStride").and_then(|v| v.as_u64()).ok_or("meshopt: missing byteStride")? as usize;
        let count = ext.get("count").and_then(|v| v.as_u64()).ok_or("meshopt: missing count")? as usize;
        let mode = ext.get("mode").and_then(|v| v.as_str()).unwrap_or("ATTRIBUTES");
        let filter = ext.get("filter").and_then(|v| v.as_str()).unwrap_or("NONE");

        let src = buffers.get(src_buffer).ok_or("meshopt: source buffer out of range")?;
        if src_offset + src_length > src.len() {
            return Err("meshopt: source region past buffer end".into());
        }
        let src_bytes = &src[src_offset .. src_offset + src_length];

        let decoded_len = count * stride;
        let mut decoded = vec![0u8; decoded_len];
        match mode {
            "ATTRIBUTES" => {
                // The pure-Rust decoder's `decode_vertex_buffer` treats the
                // destination as a slice of typed vertices. We use `[u8; N]`
                // sized by stride — but generic const N means we'd need one
                // path per stride. Cheat: hand-fill via a wrapper vertex type
                // dispatched by stride.
                decode_attributes(stride, count, src_bytes, &mut decoded)?;
            }
            "TRIANGLES" => {
                decode_triangles(stride, count, src_bytes, &mut decoded)?;
            }
            "INDICES" => {
                decode_index_sequence(stride, count, src_bytes, &mut decoded)?;
            }
            _ => return Err(format!("meshopt: unknown mode {}", mode)),
        }

        // Apply per-filter post-processing in place.
        apply_meshopt_filter(filter, stride, count, &mut decoded)?;

        // Target region — where the bufferView says its data lives.
        let target_buffer = view.buffer().index();
        let target_offset = view.offset();
        patches.push((target_buffer, target_offset, decoded));
    }

    for (buf_idx, offset, decoded) in patches {
        let buf = &mut buffers[buf_idx];
        if offset + decoded.len() > buf.len() {
            // Grow the target buffer if the placeholder is shorter than the
            // decoded output — some encoders declare a zero-length fallback.
            buf.resize(offset + decoded.len(), 0);
        }
        buf[offset .. offset + decoded.len()].copy_from_slice(&decoded);
    }
    Ok(())
}

fn decode_attributes(stride: usize, count: usize, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    // meshopt-rs's decoder is generic over the vertex type. Dispatch on
    // stride to a fixed-size `[u8; N]` slice view — covers the strides that
    // meshopt actually emits (multiples of 4, up to 64 for typical assets).
    macro_rules! try_stride { ($n:literal) => { if stride == $n {
        let dest: &mut [[u8; $n]] = unsafe {
            std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut [u8; $n], count)
        };
        return meshopt_rs::vertex::buffer::decode_vertex_buffer(dest, src)
            .map_err(|e| format!("meshopt attributes decode: {:?}", e));
    } } }
    try_stride!(4); try_stride!(8); try_stride!(12); try_stride!(16);
    try_stride!(20); try_stride!(24); try_stride!(28); try_stride!(32);
    try_stride!(36); try_stride!(40); try_stride!(44); try_stride!(48);
    try_stride!(52); try_stride!(56); try_stride!(60); try_stride!(64);
    Err(format!("meshopt attributes: unsupported stride {}", stride))
}

fn decode_triangles(stride: usize, count: usize, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    // `meshopt-rs` decode requires `T: From<u32>` — that's fine for u32 but
    // not u16 (potential overflow). Decode to u32 always, then downcast per
    // stride. Cost: extra 2× temp allocation for u16 indices, negligible.
    let mut tmp = vec![0u32; count];
    meshopt_rs::index::buffer::decode_index_buffer(&mut tmp, src)
        .map_err(|e| format!("meshopt triangles decode: {:?}", e))?;
    write_indices(stride, &tmp, dst)
}

fn decode_index_sequence(stride: usize, count: usize, src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    let mut tmp = vec![0u32; count];
    meshopt_rs::index::sequence::decode_index_sequence(&mut tmp, src)
        .map_err(|e| format!("meshopt index sequence decode: {:?}", e))?;
    write_indices(stride, &tmp, dst)
}

fn write_indices(stride: usize, indices: &[u32], dst: &mut [u8]) -> Result<(), String> {
    match stride {
        2 => {
            let d: &mut [u16] = unsafe { std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u16, indices.len()) };
            for (i, &v) in indices.iter().enumerate() { d[i] = v as u16; }
            Ok(())
        }
        4 => {
            let d: &mut [u32] = unsafe { std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u32, indices.len()) };
            d.copy_from_slice(indices);
            Ok(())
        }
        _ => Err(format!("meshopt: unsupported index stride {}", stride)),
    }
}

fn apply_meshopt_filter(filter: &str, stride: usize, count: usize, buf: &mut [u8]) -> Result<(), String> {
    match filter {
        "NONE" => Ok(()),
        "OCTAHEDRAL" => {
            // Octahedral filter operates on 4-tuples (xy encoded pair + w).
            // Stride 4 → u8 quads, stride 8 → u16 quads.
            if stride == 4 {
                let data: &mut [[u8; 4]] = unsafe {
                    std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut [u8; 4], count)
                };
                meshopt_rs::vertex::filter::decode_filter_oct_8(data);
            } else if stride == 8 {
                let data: &mut [[u16; 4]] = unsafe {
                    std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut [u16; 4], count)
                };
                meshopt_rs::vertex::filter::decode_filter_oct_16(data);
            } else {
                return Err(format!("meshopt OCTAHEDRAL: unsupported stride {}", stride));
            }
            Ok(())
        }
        "QUATERNION" => {
            if stride != 8 {
                return Err(format!("meshopt QUATERNION: unsupported stride {}", stride));
            }
            let data: &mut [[u16; 4]] = unsafe {
                std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut [u16; 4], count)
            };
            meshopt_rs::vertex::filter::decode_filter_quat(data);
            Ok(())
        }
        "EXPONENTIAL" => {
            // Operates on u32 words. Element count = count * stride / 4.
            if stride % 4 != 0 {
                return Err(format!("meshopt EXPONENTIAL: stride {} not a multiple of 4", stride));
            }
            let n = count * stride / 4;
            let data: &mut [u32] = unsafe {
                std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u32, n)
            };
            meshopt_rs::vertex::filter::decode_filter_exp(data);
            Ok(())
        }
        _ => Err(format!("meshopt: unknown filter {}", filter)),
    }
}

/// Return the raw bytes for `data:...;base64,...` URIs, or None for anything
/// else (external paths). Enables split `.gltf` files where buffers or
/// images are embedded inline. Only base64-encoded data URIs are supported —
/// URL-encoded ASCII is uncommon in glTF and adds an escape parser.
pub(crate) fn decode_data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (_media, payload) = rest.split_once(',')?;
    // Expect the media prefix to end in `;base64`. Non-base64 payloads
    // (URL-encoded text) fall through as None.
    if !_media.split(';').any(|p| p.eq_ignore_ascii_case("base64")) {
        return None;
    }
    base64_decode(payload)
}

/// Standard base64 (RFC 4648) decoder. Whitespace tolerated in the input
/// (some encoders wrap long lines); padding optional; URL-safe alphabet
/// accepted. Small enough to hand-roll — avoids the `base64` crate dep.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for b in s.bytes() {
        let v: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => 26 + (b - b'a') as u32,
            b'0'..=b'9' => 52 + (b - b'0') as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break, // padding — stop consuming
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            _ => return None, // invalid character
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8 & 0xff);
        }
    }
    Some(out)
}
