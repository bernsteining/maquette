//! Model parsing cache — avoids reparsing identical model bytes across render
//! calls, plus bounded caches of meshes and normals derived from them.

use crate::config::GroupStyles;
use crate::math::Vec3;
use crate::parser::Triangle;
use crate::ply_parser::PlyData;
use crate::smooth::SmoothData;
use maquette_core::cache::{Global, KeyedCache};

type ObjData = (Vec<Triangle>, GroupStyles);
/// Preprocessed mesh (clustered/clipped/exploded/normalized) + its bbox.
type PrepData = (Vec<Triangle>, Vec3, Vec3);

const CLOUD_CAP: usize = 4;
const DERIVED_CAP: usize = 16;

static STL_CACHE: Global<KeyedCache<Vec<Triangle>>> = Global::new(KeyedCache::new(0));
static OBJ_CACHE: Global<KeyedCache<ObjData>> = Global::new(KeyedCache::new(0));
static PLY_CACHE: Global<KeyedCache<PlyData>> = Global::new(KeyedCache::new(0));
static SMOOTH_CACHE: Global<KeyedCache<SmoothData>> = Global::new(KeyedCache::new(DERIVED_CAP));
static PREP_CACHE: Global<KeyedCache<PrepData>> = Global::new(KeyedCache::new(DERIVED_CAP));
static CLOUD_CACHE: Global<KeyedCache<Vec<Triangle>>> = Global::new(KeyedCache::new(CLOUD_CAP));

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

pub fn get_stl(key: u64) -> Option<&'static Vec<Triangle>> { STL_CACHE.get().get(key) }

pub fn put_stl(key: u64, triangles: Vec<Triangle>) { STL_CACHE.get().insert(key, triangles); }

pub fn get_obj(key: u64) -> Option<&'static ObjData> { OBJ_CACHE.get().get(key) }

pub fn put_obj(key: u64, result: ObjData) { OBJ_CACHE.get().insert(key, result); }

pub fn get_ply(key: u64) -> Option<&'static PlyData> { PLY_CACHE.get().get(key) }

pub fn put_ply(key: u64, result: PlyData) { PLY_CACHE.get().insert(key, result); }

/// Smooth (per-vertex normal) cache, keyed by a geometry hash that combines the
/// model-data hash with the geometry-affecting config. Vertex normals depend
/// only on geometry — not color/material/camera/shading — so renders that vary
/// only those reuse the cached normals instead of recomputing them.
pub fn smooth(key: u64, make: impl FnOnce() -> SmoothData) -> &'static SmoothData {
    SMOOTH_CACHE.get().get_or_insert_with(key, make)
}

/// Preprocessed-mesh cache, keyed by a hash that combines the model-data hash
/// with every preprocess-affecting config field. Lets renders that vary only
/// camera/lighting/shading skip the clone + color-map + clip + explode +
/// normalize pass. STL/OBJ-without-materials only.
pub fn prep(key: u64, make: impl FnOnce() -> PrepData) -> &'static PrepData {
    PREP_CACHE.get().get_or_insert_with(key, make)
}

pub fn get_cloud(key: u64) -> Option<&'static Vec<Triangle>> { CLOUD_CACHE.get().get(key) }

pub fn put_cloud(key: u64, triangles: Vec<Triangle>) { CLOUD_CACHE.get().insert(key, triangles); }
