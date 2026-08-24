//! Static caches — keyed on fast hashes of the input glTF bytes.
//!
//! Typst instantiates a plugin once per document and reuses the wasm instance
//! across function calls, so static state is preserved between calls inside
//! a single compilation. Single-threaded inside the wasm sandbox, so we can
//! `static mut` + `transmute` freely.
//!
//! Two-tier cache:
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
use maquette_core::math::Vec3;
use crate::scene::{Scene, SceneOpts, TextureLoadOpts};
use maquette_core::texture::Texture;

static mut SCENE_CACHE: Option<(u64, Scene)> = None;
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
    h.write_u8(0); // domain tag: procedural
    for v in [sky[0], sky[1], sky[2], ground[0], ground[1], ground[2], intensity,
              sun_dir.x as f32, sun_dir.y as f32, sun_dir.z as f32] {
        h.write_u32(v.to_bits());
    }
    h.finish()
}

fn ibl_hash_hdr(bytes: &[u8], intensity: f32, rotation: f32) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u8(1); // domain tag: HDR
    h.write(bytes);
    h.write_u32(intensity.to_bits());
    h.write_u32(rotation.to_bits());
    h.finish()
}

/// Get-or-decode the texture list for `(bytes, opts.textures)`. Returned
/// slice lives forever (leaky cache). Called from scene::flatten so that
/// distinct animation frames of the same asset share the decoded textures.
pub fn textures_for(
    bytes: &[u8],
    loaded: &LoadedGltf,
    opts: TextureLoadOpts,
) -> &'static [Texture] {
    let key = tex_hash(bytes, opts);
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

/// Get-or-compute the flattened scene. The compute path uses the texture
/// cache above so animation frames don't redo the ~150 ms decode+mip cost
/// per frame.
pub fn scene_for(bytes: &[u8], loaded: &LoadedGltf, opts: SceneOpts) -> &'static Scene {
    let key = hash(bytes, opts);
    unsafe {
        if let Some((k, ref s)) = SCENE_CACHE {
            if k == key { return std::mem::transmute::<&Scene, &'static Scene>(s); }
        }
        let scene = crate::scene::flatten_with_cached_textures(loaded, opts, bytes);
        SCENE_CACHE = Some((key, scene));
        let (_, ref s) = SCENE_CACHE.as_ref().unwrap();
        std::mem::transmute::<&Scene, &'static Scene>(s)
    }
}

fn hash(bytes: &[u8], opts: SceneOpts) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write(bytes);
    h.write_u8(opts.textures.disabled as u8);
    h.write_u32(opts.textures.max_size.unwrap_or(u32::MAX));
    h.write_u32(opts.time.to_bits());
    h.write_u32(opts.variant);
    h.finish()
}

fn tex_hash(bytes: &[u8], opts: TextureLoadOpts) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write(bytes);
    h.write_u8(opts.disabled as u8);
    h.write_u32(opts.max_size.unwrap_or(u32::MAX));
    h.finish()
}
