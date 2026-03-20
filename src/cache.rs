/// Model parsing cache — avoids reparsing identical model bytes across render calls.
///
/// Uses Vec<(u64, T)> with linear scan instead of HashMap to minimize codegen.
/// A document typically has fewer than 10 distinct models, so linear scan is faster
/// than any hash table at this scale.
///
/// Safety: all access is `unsafe` via raw pointers to `static mut`. This is safe
/// because WASM execution is single-threaded (same justification as `color.rs` LUTs).

use crate::config::GroupAppearance;
use crate::parser::Triangle;
use crate::ply_parser::PlyData;
use std::collections::HashMap;
use std::ptr::{addr_of, addr_of_mut};

type StlEntry = (u64, Vec<Triangle>);
type ObjEntry = (u64, (Vec<Triangle>, HashMap<u32, GroupAppearance>));
type PlyEntry = (u64, PlyData);

static mut STL_CACHE: Vec<StlEntry> = Vec::new();
static mut OBJ_CACHE: Vec<ObjEntry> = Vec::new();
static mut PLY_CACHE: Vec<PlyEntry> = Vec::new();

/// FNV-1a hash over a byte slice.
fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn get<T>(cache: &[(u64, T)], key: u64) -> Option<&T> {
    cache.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

pub fn get_stl(data: &[u8]) -> Option<&'static Vec<Triangle>> {
    unsafe { get(&*addr_of!(STL_CACHE), fnv1a(data)) }
}

pub fn put_stl(data: &[u8], triangles: Vec<Triangle>) {
    unsafe { (*addr_of_mut!(STL_CACHE)).push((fnv1a(data), triangles)) }
}

pub fn get_obj(data: &[u8]) -> Option<&'static (Vec<Triangle>, HashMap<u32, GroupAppearance>)> {
    unsafe { get(&*addr_of!(OBJ_CACHE), fnv1a(data)) }
}

pub fn put_obj(data: &[u8], result: (Vec<Triangle>, HashMap<u32, GroupAppearance>)) {
    unsafe { (*addr_of_mut!(OBJ_CACHE)).push((fnv1a(data), result)) }
}

pub fn get_ply(data: &[u8]) -> Option<&'static PlyData> {
    unsafe { get(&*addr_of!(PLY_CACHE), fnv1a(data)) }
}

pub fn put_ply(data: &[u8], result: PlyData) {
    unsafe { (*addr_of_mut!(PLY_CACHE)).push((fnv1a(data), result)) }
}
