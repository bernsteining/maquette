/// Model parsing cache — avoids reparsing identical model bytes across render calls.
///
/// Safety: all access is `unsafe` via raw pointers to `static mut`. This is safe
/// because WASM execution is single-threaded (same justification as `color.rs` LUTs).

use crate::config::GroupAppearance;
use crate::parser::Triangle;
use crate::ply_parser::PlyData;
use std::collections::HashMap;
use std::ptr::{addr_of, addr_of_mut};

static mut STL_CACHE: Option<HashMap<u64, Vec<Triangle>>> = None;
static mut OBJ_CACHE: Option<HashMap<u64, (Vec<Triangle>, HashMap<u32, GroupAppearance>)>> = None;
static mut PLY_CACHE: Option<HashMap<u64, PlyData>> = None;

/// FNV-1a hash over a byte slice.
fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn get_stl(data: &[u8]) -> Option<&'static Vec<Triangle>> {
    unsafe { (*addr_of!(STL_CACHE)).as_ref()?.get(&fnv1a(data)) }
}

pub fn put_stl(data: &[u8], triangles: Vec<Triangle>) {
    unsafe {
        (*addr_of_mut!(STL_CACHE))
            .get_or_insert_with(HashMap::new)
            .insert(fnv1a(data), triangles);
    }
}

pub fn get_obj(data: &[u8]) -> Option<&'static (Vec<Triangle>, HashMap<u32, GroupAppearance>)> {
    unsafe { (*addr_of!(OBJ_CACHE)).as_ref()?.get(&fnv1a(data)) }
}

pub fn put_obj(data: &[u8], result: (Vec<Triangle>, HashMap<u32, GroupAppearance>)) {
    unsafe {
        (*addr_of_mut!(OBJ_CACHE))
            .get_or_insert_with(HashMap::new)
            .insert(fnv1a(data), result);
    }
}

pub fn get_ply(data: &[u8]) -> Option<&'static PlyData> {
    unsafe { (*addr_of!(PLY_CACHE)).as_ref()?.get(&fnv1a(data)) }
}

pub fn put_ply(data: &[u8], result: PlyData) {
    unsafe {
        (*addr_of_mut!(PLY_CACHE))
            .get_or_insert_with(HashMap::new)
            .insert(fnv1a(data), result);
    }
}
