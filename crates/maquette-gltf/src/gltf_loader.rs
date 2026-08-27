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
    // Draco pre-processing: `KHR_draco_mesh_compression` allows accessors to
    // omit their `bufferView` (the values live in the Draco stream, not in
    // a real bufferView). gltf-rs can't consume such accessors, so before
    // handing the JSON off we synthesise the missing bufferViews + a
    // placeholder buffer the Draco pass fills in below.
    let patched;
    let bytes: &[u8] = match preprocess_draco_json(bytes) {
        Ok(Some(p)) => { patched = p; &patched }
        Ok(None) => bytes,      // no Draco primitives → pass through untouched
        Err(e) => return Err(e),
    };

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
                if let Some(n) = uri.strip_prefix(DRACO_PLACEHOLDER_URI_PREFIX) {
                    // Placeholder buffer we injected during Draco preprocess;
                    // `decompress_draco` overwrites the zero contents below.
                    let n: usize = n.parse().map_err(|_| "draco: bad placeholder size")?;
                    buffers.push(vec![0u8; n]);
                } else if let Some(bytes) = decode_data_uri(uri) {
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

    // KHR_draco_mesh_compression: per-primitive Draco payloads. Same idea
    // as meshopt above — decode up front, write the vertex attributes +
    // indices into the accessors' fallback bufferView locations so scene
    // traversal never has to know Draco existed.
    decompress_draco(&document, &mut buffers, bytes)?;

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

/// Byte pattern the fast-reject scan looks for. Presence in the raw glTF
/// bytes is a necessary condition for any Draco primitive to exist — if
/// this substring isn't anywhere in the file, no accessor references the
/// extension and the JSON parse is skippable.
const DRACO_MARKER: &[u8] = b"KHR_draco_mesh_compression";

/// Linear byte-substring scan — `haystack.contains(needle)` for byte slices.
/// Kept tiny; no `memchr` dep. Used exclusively by the fast-reject path.
#[inline]
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// URI scheme we inject on synthesised buffers during Draco preprocessing.
/// Format: `"draco-placeholder:<byte_length>"`. `parse_impl` matches this
/// prefix and allocates a zero-filled Vec of the given size; `decompress_draco`
/// then overwrites the zeros with decoded attribute + index data.
const DRACO_PLACEHOLDER_URI_PREFIX: &str = "draco-placeholder:";

/// Scan the raw glTF/GLB for `KHR_draco_mesh_compression` primitives. If any
/// are present, patch the JSON so every Draco-referenced accessor gains a
/// synthesised `bufferView` pointing into a new buffer (with our special
/// placeholder URI). Returns `Ok(Some(patched))` when patched, `Ok(None)`
/// when the input has no Draco primitives (skip the reserialise cost).
///
/// Supports both `.gltf` (JSON) and `.glb` (binary) inputs — for GLB the
/// JSON chunk is rewritten in place and the wrapping header/lengths are
/// recomputed. The BIN chunk (if any) rides along unchanged.
fn preprocess_draco_json(raw: &[u8]) -> Result<Option<Vec<u8>>, String> {
    // Split JSON out of whichever container we got. Also remember the tail
    // (chunk-1 BIN block for GLB) so we can rebuild the container after.
    let (json_bytes, glb_bin_tail): (Vec<u8>, Vec<u8>) = if raw.starts_with(b"glTF") {
        if raw.len() < 20 { return Err("GLB: truncated header".into()); }
        let json_len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        let json_type = u32::from_le_bytes(raw[16..20].try_into().unwrap());
        if json_type != 0x4E4F534A { return Err("GLB: chunk 0 is not JSON".into()); }
        if 20 + json_len > raw.len() { return Err("GLB: JSON chunk overruns file".into()); }
        (raw[20..20 + json_len].to_vec(), raw[20 + json_len..].to_vec())
    } else {
        (raw.to_vec(), Vec::new())
    };

    // Fast reject: the vast majority of glTF assets don't use Draco. A
    // linear substring scan over the JSON chunk skips the serde_json parse
    // when the extension marker is absent. Kept scoped to the JSON slice
    // (not `raw`) — scanning the GLB's BIN chunk noise dwarfs the JSON
    // parse it would replace.
    if !contains_bytes(&json_bytes, DRACO_MARKER) { return Ok(None); }

    let mut json: serde_json::Value = serde_json::from_slice(&json_bytes)
        .map_err(|e| format!("draco preprocess: bad JSON: {}", e))?;

    // Quick "is there any Draco at all" scan — bail out cheaply on the common case.
    let meshes = match json.get("meshes").and_then(|v| v.as_array()) {
        Some(m) if !m.is_empty() => m.clone(),
        _ => return Ok(None),
    };
    let has_draco = meshes.iter().flat_map(|m|
        m.get("primitives").and_then(|v| v.as_array()).into_iter().flatten()
    ).any(|p| p.get("extensions").and_then(|e| e.get("KHR_draco_mesh_compression")).is_some());
    if !has_draco { return Ok(None); }

    // Gather the accessors that need a fabricated bufferView + compute the
    // total placeholder-buffer size. Same accessor may be referenced by
    // multiple primitives — coalesce so we don't allocate twice.
    let accessors = json.get("accessors").and_then(|v| v.as_array())
        .ok_or("draco preprocess: no accessors")?.clone();
    let mut acc_to_new_bv: std::collections::HashMap<usize, (usize, usize)> = std::collections::HashMap::new();
    let mut cursor = 0usize;
    let mut note_accessor = |acc_idx: usize| -> Result<(), String> {
        if acc_to_new_bv.contains_key(&acc_idx) { return Ok(()); }
        let acc = accessors.get(acc_idx)
            .ok_or_else(|| format!("draco preprocess: accessor {} out of range", acc_idx))?;
        // If the accessor already has a bufferView, gltf-rs can read it
        // directly — skip; decompress_draco writes into that bv location.
        if acc.get("bufferView").is_some() { return Ok(()); }
        let size = accessor_byte_size(acc)?;
        acc_to_new_bv.insert(acc_idx, (cursor, size));
        cursor += size;
        Ok(())
    };
    for mesh in &meshes {
        for prim in mesh.get("primitives").and_then(|v| v.as_array()).into_iter().flatten() {
            let Some(ext) = prim.get("extensions").and_then(|e| e.get("KHR_draco_mesh_compression")) else { continue; };
            let attrs = ext.get("attributes").and_then(|v| v.as_object())
                .ok_or("draco preprocess: extension missing attributes map")?;
            for (name, _id) in attrs {
                // `_id` is the Draco-internal attribute id, consumed at decode
                // time by `decompress_draco`; here we only need the mapping
                // back to the glTF accessor via the primitive's own attributes.
                let acc_idx = prim.get("attributes").and_then(|v| v.as_object())
                    .and_then(|o| o.get(name)).and_then(|v| v.as_u64())
                    .ok_or_else(|| format!("draco preprocess: primitive missing accessor for '{}'", name))?
                    as usize;
                note_accessor(acc_idx)?;
            }
            if let Some(idx_val) = prim.get("indices").and_then(|v| v.as_u64()) {
                note_accessor(idx_val as usize)?;
            }
        }
    }

    // Insert the new buffer entry with our placeholder URI (zero-length
    // decode inside gltf-rs, but our parse_impl catches the prefix and
    // allocates the real byte count). No `byteLength` mismatch worry —
    // gltf-rs isn't strict about it under `from_slice_without_validation`.
    let new_buffer_idx = json.get("buffers").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let buffers_arr = json.as_object_mut().unwrap()
        .entry("buffers").or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut().unwrap();
    buffers_arr.push(serde_json::json!({
        "byteLength": cursor,
        "uri": format!("{}{}", DRACO_PLACEHOLDER_URI_PREFIX, cursor),
    }));

    // Append one bufferView per synthesised accessor. Assign the new indices
    // as we go; `acc_to_new_bv` now holds (offset, size) — we track the new
    // bv index alongside so we can point accessors at them below.
    let existing_bv_len = json.get("bufferViews").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let mut bv_index_for_acc: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let bv_arr = json.as_object_mut().unwrap()
        .entry("bufferViews").or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut().unwrap();
    // Deterministic insertion order → sort acc indices ascending.
    let mut sorted_accs: Vec<usize> = acc_to_new_bv.keys().copied().collect();
    sorted_accs.sort_unstable();
    for acc_idx in &sorted_accs {
        let (offset, size) = acc_to_new_bv[acc_idx];
        bv_arr.push(serde_json::json!({
            "buffer": new_buffer_idx,
            "byteOffset": offset,
            "byteLength": size,
        }));
        bv_index_for_acc.insert(*acc_idx, existing_bv_len + bv_index_for_acc.len());
    }

    // Patch each accessor to reference its new bufferView.
    let acc_arr = json.as_object_mut().unwrap()
        .get_mut("accessors").unwrap().as_array_mut().unwrap();
    for (acc_idx, bv_idx) in &bv_index_for_acc {
        let acc = acc_arr.get_mut(*acc_idx).unwrap().as_object_mut().unwrap();
        acc.insert("bufferView".into(), serde_json::json!(*bv_idx));
        // Ensure byteOffset is 0 (we own the whole bv region).
        acc.insert("byteOffset".into(), serde_json::json!(0));
    }

    // Serialise the patched JSON.
    let new_json = serde_json::to_vec(&json)
        .map_err(|e| format!("draco preprocess: reserialise failed: {}", e))?;

    // Rebuild container.
    if glb_bin_tail.is_empty() {
        // Plain .gltf — just the patched JSON.
        Ok(Some(new_json))
    } else {
        // GLB — 12 B header + [json chunk] + tail (unchanged BIN chunk).
        // JSON chunk length must be 4-byte aligned; pad with spaces (0x20).
        let mut json_padded = new_json;
        while json_padded.len() % 4 != 0 { json_padded.push(0x20); }
        let json_len = json_padded.len();
        let total_len = 12 + 8 + json_len + glb_bin_tail.len();
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total_len as u32).to_le_bytes());
        out.extend_from_slice(&(json_len as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());   // "JSON"
        out.extend_from_slice(&json_padded);
        out.extend_from_slice(&glb_bin_tail);
        Ok(Some(out))
    }
}

/// Byte size an accessor's raw data occupies, given its JSON descriptor.
/// glTF accessors that live in a plain (non-Draco) bufferView may declare a
/// `byteStride` and interleave with other data — but Draco decompresses to
/// tightly-packed values so `count · type_dims · component_size` is correct
/// for our purposes.
fn accessor_byte_size(acc: &serde_json::Value) -> Result<usize, String> {
    let count = acc.get("count").and_then(|v| v.as_u64())
        .ok_or("accessor: missing count")? as usize;
    let type_ = acc.get("type").and_then(|v| v.as_str())
        .ok_or("accessor: missing type")?;
    let n = match type_ {
        "SCALAR" => 1, "VEC2" => 2, "VEC3" => 3, "VEC4" => 4,
        "MAT2" => 4, "MAT3" => 9, "MAT4" => 16,
        other => return Err(format!("accessor: unknown type {}", other)),
    };
    let ct = acc.get("componentType").and_then(|v| v.as_u64())
        .ok_or("accessor: missing componentType")? as u32;
    let c = match ct {
        5120 | 5121 => 1,       // BYTE / UNSIGNED_BYTE
        5122 | 5123 => 2,       // SHORT / UNSIGNED_SHORT
        5125        => 4,       // UNSIGNED_INT
        5126        => 4,       // FLOAT
        other => return Err(format!("accessor: unknown componentType {}", other)),
    };
    Ok(count * n * c)
}

/// Map a glTF attribute-name string (per the spec: `POSITION`, `NORMAL`,
/// `TANGENT`, `COLOR_n`, `TEXCOORD_n`, `JOINTS_n`, `WEIGHTS_n`) to gltf-rs's
/// `Semantic` enum. `gltf::Semantic` doesn't impl `FromStr`, and its typed
/// n-suffixed variants (`Colors(u32)`) mean a raw byte match won't do — we
/// have to split off the trailing digits.
fn parse_gltf_semantic(name: &str) -> Option<gltf::Semantic> {
    match name {
        "POSITION" => Some(gltf::Semantic::Positions),
        "NORMAL"   => Some(gltf::Semantic::Normals),
        "TANGENT"  => Some(gltf::Semantic::Tangents),
        _ => {
            let (prefix, idx_str) = name.rsplit_once('_')?;
            let idx: u32 = idx_str.parse().ok()?;
            match prefix {
                "COLOR"    => Some(gltf::Semantic::Colors(idx)),
                "TEXCOORD" => Some(gltf::Semantic::TexCoords(idx)),
                "JOINTS"   => Some(gltf::Semantic::Joints(idx)),
                "WEIGHTS"  => Some(gltf::Semantic::Weights(idx)),
                _ => None,
            }
        }
    }
}

/// Walk every mesh primitive and, if it declares KHR_draco_mesh_compression,
/// decode the Draco payload into the corresponding accessors' fallback
/// bufferView regions. See the extension spec:
///   https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_draco_mesh_compression
///
/// Extension layout:
///   `primitive.extensions.KHR_draco_mesh_compression = {
///       bufferView: <int>,          // source of the Draco byte stream
///       attributes: { "POSITION": draco_id, "NORMAL": draco_id, ... }
///   }`
/// The primitive's attribute accessors already point at bufferViews whose
/// data is a zero-filled placeholder; scene traversal reads via those, so
/// this pass writes the decoded values into them and never mentions Draco
/// again downstream.
fn decompress_draco(
    document: &gltf::Document,
    buffers: &mut Vec<Vec<u8>>,
    _raw: &[u8],
) -> Result<(), String> {
    use draco_oxide_decoder::Decoder;

    // Collect target (buffer_idx, offset, bytes) triples first, then apply
    // them below — the accessor's bufferView often lives in the same buffer
    // as the Draco source, so we can't hold `&mut buffers[..]` inside the
    // primitive loop.
    let mut patches: Vec<(usize, usize, Vec<u8>)> = Vec::new();

    for mesh in document.meshes() {
        for prim in mesh.primitives() {
            let Some(ext) = prim.extension_value("KHR_draco_mesh_compression") else { continue; };
            let src_bv_idx = ext.get("bufferView").and_then(|v| v.as_u64())
                .ok_or("draco: missing bufferView")? as usize;
            let attr_map = ext.get("attributes").and_then(|v| v.as_object())
                .ok_or("draco: missing attributes map")?;

            // Locate + slice the Draco payload.
            let src_bv = document.views().nth(src_bv_idx)
                .ok_or("draco: source bufferView index out of range")?;
            let src_buf = buffers.get(src_bv.buffer().index())
                .ok_or("draco: source buffer index out of range")?;
            let src_start = src_bv.offset();
            let src_end = src_start + src_bv.length();
            let src = src_buf.get(src_start..src_end)
                .ok_or("draco: source bufferView slice out of range")?;

            // Decode. Returns a Mesh with per-point-indexed attributes plus
            // a faces list of PointIdx triples.
            let decoded = Decoder::new().decode_mesh(src)
                .map_err(|e| format!("draco decode error: {:?}", e))?;

            // Establish a *canonical* index space keyed by POSITION's own
            // dedup — every attribute's output gets reshuffled into it, and
            // face indices get remapped through it. Rationale: Draco lets
            // each attribute dedup along its own axis (positions on (x,y,z),
            // UVs on (u,v)), so POSITION.map[p] and TEXCOORD_1.map[p] index
            // into different unique-value tables. glTF exposes ONE index
            // buffer and ONE `accessor.count` shared across attributes, so
            // the reader has no way to disambiguate which map to use. We
            // pick POSITION as the canonical space (its unique count is what
            // exporters typically set `accessor.count` to) and rewrite every
            // other attribute so its output at slot `i` holds the value of
            // some point `p` with `pos_map[p] == i`.
            //
            // Old code skipped the reshuffle and just used POSITION's map for
            // the face-index remap, leaving other attributes indexed by
            // their OWN dedup — hence LittlestTokyo's texture bleed / magenta
            // patches: TEXCOORD_1 values landed at wrong POSITION-unique
            // slots.
            //
            // For POSITION-unique-idx i, we take the FIRST point that maps
            // to it. When two points sharing a position have distinct values
            // (e.g. UV seams), we drop one — a minor fidelity loss vs
            // rendering garbage.
            let pos_attr = attr_map.iter()
                .find_map(|(name, id)| {
                    if name != "POSITION" { return None; }
                    let draco_id = id.as_u64()? as usize;
                    decoded.attributes.iter().find(|a| a.get_id().as_usize() == draco_id)
                });
            let pos_map = pos_attr.and_then(|a| a.point_map_as_slice());
            // Reverse map: for each POSITION-unique-idx, the first point that
            // maps to it. Length = n_pos_unique. Built once per primitive.
            let (n_pos_unique, first_point_at) = match (pos_attr, pos_map) {
                (Some(pa), Some(pm)) => {
                    let n = pa.num_unique_values();
                    let mut fpa: Vec<usize> = vec![usize::MAX; n];
                    for (p, &ui) in pm.iter().enumerate() {
                        let u = usize::from(ui);
                        if u < n && fpa[u] == usize::MAX { fpa[u] = p; }
                    }
                    (n, fpa)
                }
                _ => (0, Vec::new()),
            };

            // For each declared attribute, serialise POSITION-aligned values
            // into the accessor's fallback bufferView region.
            for (attr_name, id_val) in attr_map {
                let draco_id = id_val.as_u64()
                    .ok_or("draco: attribute id not an integer")? as usize;
                let semantic = parse_gltf_semantic(attr_name)
                    .ok_or_else(|| format!("draco: unknown attribute name '{}'", attr_name))?;
                let accessor = prim.get(&semantic)
                    .ok_or_else(|| format!("draco: no accessor for attribute {}", attr_name))?;
                let target_bv = accessor.view()
                    .ok_or("draco: accessor has no bufferView")?;
                let attr = decoded.attributes.iter()
                    .find(|a| a.get_id().as_usize() == draco_id)
                    .ok_or_else(|| format!("draco: attribute id {} not present in decoded stream", draco_id))?;

                let value_size = attr.get_component_type().size() * attr.get_num_components();
                // Expand each attribute to its full per-point form using its
                // OWN `point_to_att_val_map`. Draco stores deduplicated unique
                // values internally, and each attribute can dedup along a
                // different axis (positions on `(x,y,z)`, UVs on `(u,v)`), so
                // their maps differ. The glTF accessor exposes N per-point
                // values (`accessor.count == points`), and face indices are
                // direct point indices into that N — the only way a single
                // index buffer works across all attributes at all.
                //
                // Old code wrote just the `n_unique` deduplicated values and
                // remapped face indices through the first attribute's map,
                // assuming every attribute shared it. Broke on assets with
                // per-attribute dedup (three.js's LittlestTokyo: TEXCOORD_1
                // has its own map that differs from POSITION, so UVs landed
                // on the wrong texture-atlas cells — hence the diagonal
                // texture-bleed streaks and magenta patches).
                let unique = attr.get_data_as_bytes();
                let this_map = attr.point_map_as_slice();
                let n_unique_this = attr.num_unique_values();
                // Write `n_pos_unique` values (== `accessor.count` for
                // position-aligned attributes). For POSITION itself the loop
                // is a straight copy of the unique buffer; for others we look
                // up each POSITION-unique-idx via `first_point_at[i]` and
                // fetch that point's value through this attribute's own map.
                let out_count = n_pos_unique.max(1);
                let mut bytes = Vec::with_capacity(out_count * value_size);
                if attr_name == "POSITION" {
                    bytes.extend_from_slice(&unique[..out_count.min(n_unique_this) * value_size]);
                } else if let Some(tm) = this_map {
                    for i in 0..n_pos_unique {
                        let p = first_point_at[i];
                        let ui = if p < tm.len() {
                            usize::from(tm[p]).min(n_unique_this.saturating_sub(1))
                        } else { 0 };
                        let src_off = ui * value_size;
                        bytes.extend_from_slice(&unique[src_off..src_off + value_size]);
                    }
                } else {
                    // No map — attribute is already stored per-point/unique,
                    // just copy what fits.
                    bytes.extend_from_slice(&unique[..out_count.min(n_unique_this) * value_size]);
                }
                if bytes.len() < out_count * value_size {
                    bytes.resize(out_count * value_size, 0);
                }
                patches.push((target_bv.buffer().index(), target_bv.offset() + accessor.offset(), bytes));
            }

            // Indices: flatten faces into u16 / u32 / u8 depending on the
            // accessor's component type. glTF indices are always UNSIGNED_*.
            // Face point-indices get remapped through POSITION's map to land
            // in the canonical [0, n_pos_unique) space every attribute was
            // written into above.
            if let Some(idx_accessor) = prim.indices() {
                let target_bv = idx_accessor.view()
                    .ok_or("draco: indices accessor has no bufferView")?;
                let mut flat: Vec<u32> = Vec::with_capacity(decoded.faces.len() * 3);
                for f in &decoded.faces {
                    for p in f {
                        let pi = usize::from(*p);
                        let idx = match pos_map {
                            Some(m) if pi < m.len() => usize::from(m[pi]) as u32,
                            _ => pi as u32,
                        };
                        flat.push(idx);
                    }
                }
                let bytes = match idx_accessor.data_type() {
                    gltf::accessor::DataType::U16 => {
                        let mut b = Vec::with_capacity(flat.len() * 2);
                        for i in &flat { b.extend_from_slice(&(*i as u16).to_le_bytes()); }
                        b
                    }
                    gltf::accessor::DataType::U32 => {
                        let mut b = Vec::with_capacity(flat.len() * 4);
                        for i in &flat { b.extend_from_slice(&i.to_le_bytes()); }
                        b
                    }
                    gltf::accessor::DataType::U8 => {
                        let mut b = Vec::with_capacity(flat.len());
                        for i in &flat { b.push(*i as u8); }
                        b
                    }
                    other => return Err(format!("draco: indices accessor unsupported type {:?}", other)),
                };
                patches.push((target_bv.buffer().index(), target_bv.offset() + idx_accessor.offset(), bytes));
            }
        }
    }

    // Apply all patches. Grow the destination buffer if the placeholder was
    // shorter than the decoded payload (encoders sometimes emit zero-length
    // fallback buffers).
    for (buf_idx, offset, bytes) in patches {
        let buf = &mut buffers[buf_idx];
        if offset + bytes.len() > buf.len() {
            buf.resize(offset + bytes.len(), 0);
        }
        buf[offset .. offset + bytes.len()].copy_from_slice(&bytes);
    }
    Ok(())
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
