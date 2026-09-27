/// Model parsing cache — avoids reparsing identical model bytes across render calls.
///
/// Uses Vec<(u64, T)> with linear scan instead of HashMap to minimize codegen.
/// A document typically has fewer than 10 distinct models, so linear scan is faster
/// than any hash table at this scale.
///
/// Safety: all access is `unsafe` via raw pointers to `static mut`. This is safe
/// because WASM execution is single-threaded (same justification as `color.rs` LUTs).

use crate::config::GroupAppearance;
use crate::math::Vec3;
use crate::parser::Triangle;
use crate::ply_parser::PlyData;
use crate::smooth::SmoothData;
use std::collections::HashMap;
use std::ptr::{addr_of, addr_of_mut};

type StlEntry = (u64, Vec<Triangle>);
type ObjEntry = (u64, (Vec<Triangle>, HashMap<u32, GroupAppearance>));
type PlyEntry = (u64, PlyData);
type SmoothEntry = (u64, SmoothData);
/// Preprocessed mesh (clustered/clipped/exploded/normalized) + its bbox.
type PrepEntry = (u64, (Vec<Triangle>, Vec3, Vec3));

static mut STL_CACHE: Vec<StlEntry> = Vec::new();
static mut OBJ_CACHE: Vec<ObjEntry> = Vec::new();
static mut PLY_CACHE: Vec<PlyEntry> = Vec::new();
static mut SMOOTH_CACHE: Vec<SmoothEntry> = Vec::new();
static mut PREP_CACHE: Vec<PrepEntry> = Vec::new();
static mut CLOUD_CACHE: Vec<StlEntry> = Vec::new();

/// Model-data hash, used as the cache key for parsed geometry and as the base
/// of the smooth/preprocess keys. Consumes 8 bytes per step (plus the length and
/// a final avalanche) so a multi-megabyte model costs a fraction of a
/// byte-at-a-time hash under the interpreter.
pub fn hash(data: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let n = data.len();
    let mut h = (n as u64).wrapping_mul(K) ^ 0xcbf2_9ce4_8422_2325;
    let words = n / 8;
    let ptr = data.as_ptr();
    for i in 0..words {
        let w = unsafe { ptr.add(i * 8).cast::<u64>().read_unaligned() };
        h = (h ^ w).wrapping_mul(K).rotate_left(31);
    }
    let mut tail = 0u64;
    for (i, &b) in data[words * 8..].iter().enumerate() {
        tail |= (b as u64) << (i * 8);
    }
    h = (h ^ tail).wrapping_mul(K);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 32)
}

fn get<T>(cache: &[(u64, T)], key: u64) -> Option<&T> {
    cache.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

pub fn get_stl(key: u64) -> Option<&'static Vec<Triangle>> {
    unsafe { get(&*addr_of!(STL_CACHE), key) }
}

pub fn put_stl(key: u64, triangles: Vec<Triangle>) {
    unsafe { (*addr_of_mut!(STL_CACHE)).push((key, triangles)) }
}

pub fn get_obj(key: u64) -> Option<&'static (Vec<Triangle>, HashMap<u32, GroupAppearance>)> {
    unsafe { get(&*addr_of!(OBJ_CACHE), key) }
}

pub fn put_obj(key: u64, result: (Vec<Triangle>, HashMap<u32, GroupAppearance>)) {
    unsafe { (*addr_of_mut!(OBJ_CACHE)).push((key, result)) }
}

pub fn get_ply(key: u64) -> Option<&'static PlyData> {
    unsafe { get(&*addr_of!(PLY_CACHE), key) }
}

pub fn put_ply(key: u64, result: PlyData) {
    unsafe { (*addr_of_mut!(PLY_CACHE)).push((key, result)) }
}

/// Smooth (per-vertex normal) cache, keyed by a geometry hash that combines the
/// model-data hash with the geometry-affecting config. Vertex normals depend
/// only on geometry — not color/material/camera/shading — so renders that vary
/// only those reuse the cached normals instead of recomputing them.
pub fn get_smooth(key: u64) -> Option<&'static SmoothData> {
    unsafe { get(&*addr_of!(SMOOTH_CACHE), key) }
}

pub fn put_smooth(key: u64, data: SmoothData) {
    unsafe { push_bounded(&mut *addr_of_mut!(SMOOTH_CACHE), key, data) }
}

/// Preprocessed-mesh cache, keyed by a hash that combines the model-data hash
/// with every preprocess-affecting config field. Lets renders that vary only
/// camera/lighting/shading skip the clone + color-map + clip + explode +
/// normalize pass. STL/OBJ-without-materials only.
pub fn get_prep(key: u64) -> Option<&'static (Vec<Triangle>, Vec3, Vec3)> {
    unsafe { get(&*addr_of!(PREP_CACHE), key) }
}

pub fn put_prep(key: u64, data: (Vec<Triangle>, Vec3, Vec3)) {
    unsafe { push_bounded(&mut *addr_of_mut!(PREP_CACHE), key, data) }
}

pub fn get_cloud(key: u64) -> Option<&'static Vec<Triangle>> {
    unsafe { get(&*addr_of!(CLOUD_CACHE), key) }
}

pub fn put_cloud(key: u64, triangles: Vec<Triangle>) {
    let cache = unsafe { &mut *addr_of_mut!(CLOUD_CACHE) };
    if cache.len() >= CLOUD_CAP { cache.remove(0); }
    cache.push((key, triangles));
}

const CLOUD_CAP: usize = 4;
const DERIVED_CAP: usize = 16;

fn push_bounded<T>(cache: &mut Vec<(u64, T)>, key: u64, data: T) {
    if cache.len() >= DERIVED_CAP { cache.remove(0); }
    cache.push((key, data));
}
