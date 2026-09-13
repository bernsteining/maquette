/// Minimal 3D vector math and utilities — no external dependency needed.

use std::collections::HashMap;

// Vec3 / Mat4 / FxHasher are shared render primitives — they live in
// maquette-core (byte-identical) and are re-exported so `crate::math::…`
// call sites are unchanged. Mesh-only utilities (parsers, adjacency, the
// SIMD view transform) stay below.
pub use maquette_core::math::{FxBuildHasher, FxHashMap, Mat4, Vec3};


#[inline]
pub fn fx_hashmap<K, V>() -> FxHashMap<K, V> { HashMap::with_hasher(FxBuildHasher::default()) }

#[inline]
pub fn fx_hashmap_cap<K, V>(cap: usize) -> FxHashMap<K, V> {
    HashMap::with_capacity_and_hasher(cap, FxBuildHasher::default())
}


/// Quantize a vertex position to integer keys for hashing (~1e-6 resolution).
#[inline]
pub fn quantize(v: Vec3) -> (i64, i64, i64) {
    (
        (v.x * 1e6).round() as i64,
        (v.y * 1e6).round() as i64,
        (v.z * 1e6).round() as i64,
    )
}

/// Canonical edge key so A→B and B→A hash the same.
#[inline]
pub fn edge_key(
    a: (i64, i64, i64),
    b: (i64, i64, i64),
) -> ((i64, i64, i64), (i64, i64, i64)) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Build edge-adjacency graph from triangles (vertex → neighbor vertices).
pub fn build_adjacency(triangles: &[crate::parser::Triangle]) -> FxHashMap<(i64, i64, i64), Vec<(i64, i64, i64)>> {
    let mut adjacency: FxHashMap<(i64, i64, i64), Vec<(i64, i64, i64)>> = fx_hashmap();
    for tri in triangles {
        let keys = [
            quantize(tri.vertices[0]),
            quantize(tri.vertices[1]),
            quantize(tri.vertices[2]),
        ];
        for i in 0..3 {
            let j = (i + 1) % 3;
            adjacency.entry(keys[i]).or_default().push(keys[j]);
            adjacency.entry(keys[j]).or_default().push(keys[i]);
        }
    }
    adjacency
}

/// Fast manual float parser. 2-5x faster than `str::parse::<f64>()` for typical
/// 3D model data (no NaN/Inf, limited exponent range).
#[inline]
pub fn parse_f64_fast(s: &str) -> Option<f64> {
    parse_f64_bytes(s.as_bytes())
}

/// Byte-slice core of [`parse_f64_fast`], so parsers can work directly on
/// `&[u8]` tokens without a `&str`/UTF-8 layer.
#[inline]
pub fn parse_f64_bytes(b: &[u8]) -> Option<f64> {
    let len = b.len();
    if len == 0 { return None; }
    let mut i = 0;

    // Sign
    let neg = b[i] == b'-';
    if neg || b[i] == b'+' { i += 1; }
    if i >= len { return None; }

    // Integer part
    let mut int_val: u64 = 0;
    let mut has_digits = false;
    while i < len && b[i] >= b'0' && b[i] <= b'9' {
        int_val = int_val * 10 + (b[i] - b'0') as u64;
        has_digits = true;
        i += 1;
    }

    // Fractional part
    let mut frac_val: u64 = 0;
    let mut frac_digits: u32 = 0;
    if i < len && b[i] == b'.' {
        i += 1;
        while i < len && b[i] >= b'0' && b[i] <= b'9' {
            frac_val = frac_val * 10 + (b[i] - b'0') as u64;
            frac_digits += 1;
            has_digits = true;
            i += 1;
        }
    }

    if !has_digits { return None; }

    let mut result = int_val as f64;
    if frac_digits > 0 {
        // Precomputed powers of 10 (up to 18 digits)
        const POW10: [f64; 19] = [
            1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9,
            1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18,
        ];
        let div = if (frac_digits as usize) < POW10.len() {
            POW10[frac_digits as usize]
        } else {
            10.0_f64.powi(frac_digits as i32)
        };
        result += frac_val as f64 / div;
    }

    // Exponent
    if i < len && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        let exp_neg = i < len && b[i] == b'-';
        if exp_neg || (i < len && b[i] == b'+') { i += 1; }
        let mut exp: i32 = 0;
        while i < len && b[i] >= b'0' && b[i] <= b'9' {
            exp = exp * 10 + (b[i] - b'0') as i32;
            i += 1;
        }
        if exp_neg { exp = -exp; }
        result *= 10.0_f64.powi(exp);
    }

    if neg { result = -result; }
    Some(result)
}

/// Parse 3 floats from a whitespace-token iterator into a Vec3.
#[inline]
pub fn parse_vec3_iter<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<Vec3> {
    let x = parts.next().and_then(parse_f64_fast)?;
    let y = parts.next().and_then(parse_f64_fast)?;
    let z = parts.next().and_then(parse_f64_fast)?;
    Some(Vec3::new(x, y, z))
}

/// Parse 3 floats from a byte-token iterator into a Vec3.
#[inline]
pub fn parse_vec3_bytes<'a>(parts: &mut impl Iterator<Item = &'a [u8]>) -> Option<Vec3> {
    let x = parts.next().and_then(parse_f64_bytes)?;
    let y = parts.next().and_then(parse_f64_bytes)?;
    let z = parts.next().and_then(parse_f64_bytes)?;
    Some(Vec3::new(x, y, z))
}

/// Iterator over ASCII-whitespace-separated tokens of a byte slice — a byte-level
/// `split_ascii_whitespace` with no `&str`/UTF-8 layer.
pub struct AsciiTokens<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> AsciiTokens<'a> {
    #[inline]
    pub fn new(b: &'a [u8]) -> Self {
        AsciiTokens { b, i: 0 }
    }
}

impl<'a> Iterator for AsciiTokens<'a> {
    type Item = &'a [u8];
    #[inline]
    fn next(&mut self) -> Option<&'a [u8]> {
        let b = self.b;
        while self.i < b.len() && b[self.i].is_ascii_whitespace() { self.i += 1; }
        if self.i >= b.len() { return None; }
        let start = self.i;
        while self.i < b.len() && !b[self.i].is_ascii_whitespace() { self.i += 1; }
        Some(&b[start..self.i])
    }
}

/// Fast manual integer parser for OBJ indices. Handles negative (relative) indices.
#[inline]
pub fn parse_i64_fast(s: &str) -> Option<i64> {
    parse_i64_bytes(s.as_bytes())
}

/// Byte-slice core of [`parse_i64_fast`].
#[inline]
pub fn parse_i64_bytes(b: &[u8]) -> Option<i64> {
    let len = b.len();
    if len == 0 { return None; }
    let mut i = 0;
    let neg = b[i] == b'-';
    if neg { i += 1; }
    if i >= len || b[i] < b'0' || b[i] > b'9' { return None; }
    let mut val: i64 = 0;
    while i < len && b[i] >= b'0' && b[i] <= b'9' {
        val = val * 10 + (b[i] - b'0') as i64;
        i += 1;
    }
    Some(if neg { -val } else { val })
}


// ---------------------------------------------------------------------------
// SIMD f32 view matrix — pre-splatted coefficients for batch vertex transforms
// ---------------------------------------------------------------------------

use std::arch::wasm32::*;

/// View matrix with pre-splatted f32x4 coefficients.
/// Transforms 3 triangle vertices in one call using 9 SIMD multiply-adds
/// instead of 3 × 9 = 27 scalar multiply-adds.
pub struct ViewMatSimd {
    m00: v128, m01: v128, m02: v128, m03: v128,
    m10: v128, m11: v128, m12: v128, m13: v128,
    m20: v128, m21: v128, m22: v128, m23: v128,
}

impl ViewMatSimd {
    /// Build from a Mat4 (pre-splats all 12 coefficients once).
    pub fn from_mat4(m: &Mat4) -> Self {
        Self {
            m00: f32x4_splat(m.0[0][0] as f32), m01: f32x4_splat(m.0[0][1] as f32),
            m02: f32x4_splat(m.0[0][2] as f32), m03: f32x4_splat(m.0[0][3] as f32),
            m10: f32x4_splat(m.0[1][0] as f32), m11: f32x4_splat(m.0[1][1] as f32),
            m12: f32x4_splat(m.0[1][2] as f32), m13: f32x4_splat(m.0[1][3] as f32),
            m20: f32x4_splat(m.0[2][0] as f32), m21: f32x4_splat(m.0[2][1] as f32),
            m22: f32x4_splat(m.0[2][2] as f32), m23: f32x4_splat(m.0[2][3] as f32),
        }
    }

    /// Transform 3 triangle vertices at once. Returns camera-space [Vec3; 3].
    #[inline(always)]
    pub fn transform_tri(&self, v0: Vec3, v1: Vec3, v2: Vec3) -> [Vec3; 3] {
        let px = f32x4(v0.x as f32, v1.x as f32, v2.x as f32, 0.0);
        let py = f32x4(v0.y as f32, v1.y as f32, v2.y as f32, 0.0);
        let pz = f32x4(v0.z as f32, v1.z as f32, v2.z as f32, 0.0);

        let cx = f32x4_add(f32x4_add(f32x4_mul(self.m00, px), f32x4_mul(self.m01, py)),
                           f32x4_add(f32x4_mul(self.m02, pz), self.m03));
        let cy = f32x4_add(f32x4_add(f32x4_mul(self.m10, px), f32x4_mul(self.m11, py)),
                           f32x4_add(f32x4_mul(self.m12, pz), self.m13));
        let cz = f32x4_add(f32x4_add(f32x4_mul(self.m20, px), f32x4_mul(self.m21, py)),
                           f32x4_add(f32x4_mul(self.m22, pz), self.m23));

        [
            Vec3 { x: f32x4_extract_lane::<0>(cx) as f64, y: f32x4_extract_lane::<0>(cy) as f64, z: f32x4_extract_lane::<0>(cz) as f64 },
            Vec3 { x: f32x4_extract_lane::<1>(cx) as f64, y: f32x4_extract_lane::<1>(cy) as f64, z: f32x4_extract_lane::<1>(cz) as f64 },
            Vec3 { x: f32x4_extract_lane::<2>(cx) as f64, y: f32x4_extract_lane::<2>(cy) as f64, z: f32x4_extract_lane::<2>(cz) as f64 },
        ]
    }
}
