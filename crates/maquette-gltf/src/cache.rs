//! Static caches — keyed on fast hashes of the input glTF bytes.
//!
//! Typst instantiates a plugin once per document and reuses the wasm instance
//! across function calls, so static state is preserved between calls inside
//! a single compilation. Single-threaded inside the wasm sandbox, so we can
//! `static mut` + `transmute` freely.
//!
//! Caches:
//! * **Parse cache** — the parsed glTF for the most recent asset, so an
//!   animation frame at a new `time` re-flattens without re-parsing and
//!   without needing the file bytes again. The previous frame's per-node
//!   emit records let that re-flatten copy every node that didn't move.
//! * **Texture cache** — keyed on `(bytes, texture opts)`. Never invalidated
//!   by animation `time`, so animation frames of the same asset share the
//!   decoded + mipmapped texture pyramid instead of re-decoding per frame.
//!   Leaky (no eviction) — bounded by number of unique assets, small in
//!   practice.
//! * **Scene cache** — keyed on `(bytes, scene opts including time)`. 1-slot,
//!   holds the flattened scene (triangles, materials, cameras, lights). The
//!   scene borrows into the texture cache so re-flatten across time doesn't
//!   redo the expensive JPEG decode + mip chain.

use crate::gltf_loader::LoadedGltf;
use maquette_core::ibl::IblEnvironment;
use maquette_core::shadow::LightShadow;
use maquette_core::math::Vec3;
use crate::scene::{NodeEmit, Scene, SceneOpts, TextureLoadOpts};
use maquette_core::texture::Texture;

static mut SCENE_CACHE: Option<(u64, Scene)> = None;
static mut LOADED_CACHE: Option<(u64, LoadedGltf)> = None;
static mut EMIT_CACHE: Option<(u64, Vec<NodeEmit>)> = None;
static mut SHADOW_CACHE: Vec<(u64, Vec<Option<LightShadow>>)> = Vec::new();
static mut STATIC_SHADOW_CACHE: Option<(u64, Vec<Option<LightShadow>>)> = None;
static mut STATIC_PASS_CACHE: Vec<(u64, StaticPass)> = Vec::new();
static mut WORK_SHADOWS: Option<WorkShadows> = None;
static mut PASS_REQUESTS: Vec<u64> = Vec::new();
/// Leaky Vec of texture-bundle cache entries. Each entry lives forever, so
/// references into it are safely `'static`.
static mut TEXTURE_CACHE: Vec<(u64, Vec<Texture>)> = Vec::new();
/// Leaky IBL env-map cache. Env bake is ~3-5 ms per call and the same
/// sky/ground/sun params get reused across every render call in a document.
static mut IBL_CACHE: Vec<(u64, IblEnvironment)> = Vec::new();

/// Get-or-bake a procedural IBL env map for the given parameters. The
/// per-render-call bake is ~5 ms so caching pays off even for single-page
/// docs when multiple render calls share `ibl`.
pub fn ibl_for(sky: [f32; 3], ground: [f32; 3], intensity: f32, sun_dir: Vec3) -> &'static IblEnvironment {
    let key = ibl_hash_procedural(sky, ground, intensity, sun_dir);
    unsafe {
        for (k, e) in IBL_CACHE.iter() {
            if *k == key { return std::mem::transmute::<&IblEnvironment, &'static IblEnvironment>(e); }
        }
        let env = IblEnvironment::build(sky, ground, intensity, sun_dir);
        IBL_CACHE.push((key, env));
        let (_, e) = IBL_CACHE.last().unwrap();
        std::mem::transmute::<&IblEnvironment, &'static IblEnvironment>(e)
    }
}

/// Get-or-bake an HDR-photograph IBL env map. Keyed on `(bytes hash, intensity,
/// rotation)`. HDR parse + equirect→octahedral is 20-50 ms; cache reuse across
/// render calls in a doc makes it effectively free after the first.
pub fn ibl_for_hdr(hdr_bytes: &[u8], intensity: f32, rotation: f32) -> Result<&'static IblEnvironment, String> {
    let key = ibl_hash_hdr(hdr_bytes, intensity, rotation);
    unsafe {
        for (k, e) in IBL_CACHE.iter() {
            if *k == key { return Ok(std::mem::transmute::<&IblEnvironment, &'static IblEnvironment>(e)); }
        }
        let (rgb, w, h) = maquette_core::rgbe::parse(hdr_bytes)?;
        let env = IblEnvironment::build_from_equirect(&rgb, w, h, intensity, rotation);
        IBL_CACHE.push((key, env));
        let (_, e) = IBL_CACHE.last().unwrap();
        Ok(std::mem::transmute::<&IblEnvironment, &'static IblEnvironment>(e))
    }
}

fn ibl_hash_procedural(sky: [f32; 3], ground: [f32; 3], intensity: f32, sun_dir: Vec3) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u8(0);
    for v in [sky[0], sky[1], sky[2], ground[0], ground[1], ground[2], intensity,
              sun_dir.x as f32, sun_dir.y as f32, sun_dir.z as f32] {
        h.write_u32(v.to_bits());
    }
    h.finish()
}

fn ibl_hash_hdr(bytes: &[u8], intensity: f32, rotation: f32) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u8(1);
    h.write(bytes);
    h.write_u32(intensity.to_bits());
    h.write_u32(rotation.to_bits());
    h.finish()
}

/// Get-or-decode the texture list for `(asset, opts.textures)`. Returned
/// slice lives forever (leaky cache). Called from scene::flatten so that
/// distinct animation frames of the same asset share the decoded textures.
pub fn textures_for(
    asset_key: u64,
    loaded: &LoadedGltf,
    opts: TextureLoadOpts,
) -> &'static [Texture] {
    let key = tex_hash(asset_key, opts);
    unsafe {
        for (k, v) in TEXTURE_CACHE.iter() {
            if *k == key {
                return std::mem::transmute::<&[Texture], &'static [Texture]>(v.as_slice());
            }
        }
        let textures = crate::scene::collect_textures_pub(loaded, opts);
        TEXTURE_CACHE.push((key, textures));
        let (_, v) = TEXTURE_CACHE.last().unwrap();
        std::mem::transmute::<&[Texture], &'static [Texture]>(v.as_slice())
    }
}

/// Get-or-compute the flattened scene and its cache key. `load` (the glTF
/// parse) only runs on a miss, so re-rendering the same asset skips the parse
/// entirely. The compute path uses the texture cache above so animation frames
/// don't redo the ~150 ms decode+mip cost per frame.
pub fn scene_for(
    input: &SceneInput,
    opts: SceneOpts,
    load: impl FnOnce() -> Result<LoadedGltf, String>,
) -> Result<(u64, &'static Scene), String> {
    let key = hash(input.key, opts);
    unsafe {
        if let Some((k, ref s)) = SCENE_CACHE {
            if k == key { return Ok((key, std::mem::transmute::<&Scene, &'static Scene>(s))); }
        }
        if !matches!(LOADED_CACHE, Some((k, _)) if k == input.key) {
            if input.bytes.is_none() {
                return Err("prepared glTF: scene not cached".into());
            }
            LOADED_CACHE = None;
            LOADED_CACHE = Some((input.key, load()?));
        }
        let (_, ref loaded) = LOADED_CACHE.as_ref().unwrap();
        let base = hash(input.key, SceneOpts { time: 0.0, ..opts });
        if matches!(EMIT_CACHE, Some((b, _)) if b == base) && SCENE_CACHE.is_some() {
            let (_, mut scene) = SCENE_CACHE.take().unwrap();
            let (b, mut emits) = EMIT_CACHE.take().unwrap();
            if crate::scene::update_in_place(loaded, opts, &mut scene, &mut emits) {
                SCENE_CACHE = Some((key, scene));
                EMIT_CACHE = Some((b, emits));
                let (_, ref s) = SCENE_CACHE.as_ref().unwrap();
                return Ok((key, std::mem::transmute::<&Scene, &'static Scene>(s)));
            }
        }
        let prev = match (&SCENE_CACHE, &EMIT_CACHE) {
            (Some((_, ps)), Some((b, pe))) if *b == base => Some((ps, pe.as_slice())),
            _ => None,
        };
        let (mut scene, emits) = crate::scene::flatten_with_cached_textures(loaded, opts, input.key, prev);
        scene.static_key = base;
        SCENE_CACHE = Some((key, scene));
        EMIT_CACHE = Some((base, emits));
        let (_, ref s) = SCENE_CACHE.as_ref().unwrap();
        Ok((key, std::mem::transmute::<&Scene, &'static Scene>(s)))
    }
}

/// Get-or-build the shadow maps for `key`, which the caller derives from the
/// scene key plus every other shadow input (lights, up, frustum radius,
/// resolution). Shadows don't depend on the camera, so orbiting reuses them.
/// Keeps the two most recent entries, as the maps are large.
pub fn shadows_for(key: u64, build: impl FnOnce() -> Vec<Option<LightShadow>>) -> &'static [Option<LightShadow>] {
    unsafe {
        let cache = &mut *std::ptr::addr_of_mut!(SHADOW_CACHE);
        if let Some((_, maps)) = cache.iter().find(|(k, _)| *k == key) {
            return std::mem::transmute::<&[Option<LightShadow>], &'static [Option<LightShadow>]>(maps.as_slice());
        }
        if cache.len() >= 2 { cache.remove(0); }
        cache.push((key, build()));
        std::mem::transmute::<&[Option<LightShadow>], &'static [Option<LightShadow>]>(cache.last().unwrap().1.as_slice())
    }
}

/// Get-or-build the shadow maps of a scene's static casters (see
/// `Scene::dynamic`), kept apart from `shadows_for` so animation frames,
/// which each add their moving casters on top, don't evict it.
pub fn static_shadows_for(key: u64, build: impl FnOnce() -> Vec<Option<LightShadow>>) -> &'static [Option<LightShadow>] {
    unsafe {
        let slot = &mut *std::ptr::addr_of_mut!(STATIC_SHADOW_CACHE);
        if !matches!(slot, Some((k, _)) if *k == key) {
            *slot = None;
            *slot = Some((key, build()));
        }
        std::mem::transmute::<&[Option<LightShadow>], &'static [Option<LightShadow>]>(slot.as_ref().unwrap().1.as_slice())
    }
}

/// Shadow maps for the current animation frame, kept between calls and
/// patched in place: `dirty` marks, per single-frustum light, the tiles where
/// `maps` differs from the static-only `base_key` maps.
pub struct WorkShadows {
    pub base_key: u64,
    pub frame_key: u64,
    pub maps: Vec<Option<LightShadow>>,
    pub dirty: Vec<Option<(i64, Vec<bool>)>>,
}

pub fn work_shadows() -> &'static mut Option<WorkShadows> {
    unsafe { &mut *std::ptr::addr_of_mut!(WORK_SHADOWS) }
}

/// One static triangle's projection, recorded by the depth pass.
pub struct StaticEntry {
    pub index: usize,
    pub pts: [(f64, f64); 3],
    pub depths: [f64; 3],
    pub zbuf_depths: [f64; 3],
    pub view_center_z: f64,
    pub blend: bool,
}

/// Depth and ownership buffers after rasterising a scene's static triangles
/// for one camera and resolution, plus what each of those triangles became
/// (deferred opaque/mask, or queued for blending), in draw order.
pub struct StaticPass {
    pub zbuf: Vec<f32>,
    pub vis: Vec<u32>,
    /// Static triangles shaded against static-only shadow maps, when the
    /// shading can be reused; `texels` then holds, per pixel and per light,
    /// the shadow-map texel that pixel samples (see `render::TEXEL_*`).
    pub pixels: Option<Vec<u8>>,
    pub texels: Vec<u32>,
    /// Supersampling blocks the snapshot shaded once, `(top-left, owner + 1)`.
    pub blocks: Vec<(usize, u32)>,
    pub entries: Vec<StaticEntry>,
}

pub fn static_pass(key: u64) -> Option<&'static StaticPass> {
    unsafe {
        let cache = &*std::ptr::addr_of!(STATIC_PASS_CACHE);
        cache.iter().find(|(k, _)| *k == key).map(|(_, p)| std::mem::transmute::<&StaticPass, &'static StaticPass>(p))
    }
}

/// Records a request for the static pass `key` and reports whether it was
/// already requested recently: the pass is only worth building when the same
/// camera and settings come back (scrubbing an animation), not for a one-off.
pub fn static_pass_requested(key: u64) -> bool {
    unsafe {
        let seen = &mut *std::ptr::addr_of_mut!(PASS_REQUESTS);
        if seen.contains(&key) { return true; }
        if seen.len() >= 8 { seen.remove(0); }
        seen.push(key);
        false
    }
}

pub fn put_static_pass(key: u64, pass: StaticPass) {
    unsafe {
        let cache = &mut *std::ptr::addr_of_mut!(STATIC_PASS_CACHE);
        if cache.len() >= 3 { cache.remove(0); }
        cache.push((key, pass));
    }
}

/// What identifies a render's asset: a key over the glTF bytes (plus any
/// sidecar bundle), and the bytes themselves unless the caller passed only a
/// 16-byte handle, in which case the scene must already be cached.
pub struct SceneInput<'a> {
    pub key: u64,
    pub bytes: Option<&'a [u8]>,
}

const HANDLE_MAGIC: &[u8; 8] = b"\0MQGLTF\x01";

impl<'a> SceneInput<'a> {
    pub fn new(gltf: &'a [u8], sidecars: &[u8]) -> Self {
        if gltf.len() == 16 && gltf[..8] == HANDLE_MAGIC[..] {
            return Self { key: u64::from_le_bytes(gltf[8..].try_into().unwrap()), bytes: None };
        }
        Self { key: bytes_key(gltf, sidecars), bytes: Some(gltf) }
    }

    pub fn handle(&self) -> Vec<u8> {
        let mut out = HANDLE_MAGIC.to_vec();
        out.extend_from_slice(&self.key.to_le_bytes());
        out
    }
}

fn bytes_key(gltf: &[u8], sidecars: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u64(gltf.len() as u64);
    h.write(gltf);
    h.write_u64(sidecars.len() as u64);
    h.write(sidecars);
    h.finish()
}

fn hash(bytes_key: u64, opts: SceneOpts) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u64(bytes_key);
    h.write_u8(opts.textures.disabled as u8);
    h.write_u32(opts.textures.max_size.unwrap_or(u32::MAX));
    h.write_u32(opts.time.to_bits());
    h.write_u32(opts.variant);
    h.write_u64(opts.scene_index.map_or(u64::MAX, |i| i as u64));
    h.write_u64(opts.animation_index.map_or(u64::MAX, |i| i as u64));
    h.finish()
}

fn tex_hash(asset_key: u64, opts: TextureLoadOpts) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u64(asset_key);
    h.write_u8(opts.disabled as u8);
    h.write_u32(opts.max_size.unwrap_or(u32::MAX));
    h.finish()
}
