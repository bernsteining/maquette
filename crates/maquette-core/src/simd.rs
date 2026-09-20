//! Native stand-in for the `core::arch::wasm32` SIMD intrinsics the renderer
//! uses. Same names, same lane semantics (little-endian), so wasm call sites
//! compile unchanged off-target. Hot lane ops go through `wide` (real SSE/AVX2
//! /NEON, scalar fallback elsewhere); the exotic ops are scalar for clarity.
#![allow(non_camel_case_types)]

use bytemuck::cast;

#[derive(Clone, Copy)]
pub struct v128([u8; 16]);

#[inline]
fn af32(v: v128) -> [f32; 4] { cast(v.0) }
#[inline]
fn ai32(v: v128) -> [i32; 4] { cast(v.0) }
#[inline]
fn au32(v: v128) -> [u32; 4] { cast(v.0) }
#[inline]
fn ai16(v: v128) -> [i16; 8] { cast(v.0) }
#[inline]
fn au16(v: v128) -> [u16; 8] { cast(v.0) }
#[inline]
fn af64(v: v128) -> [f64; 2] { cast(v.0) }
#[inline]
fn vf32(a: [f32; 4]) -> v128 { v128(cast(a)) }
#[inline]
fn vi32(a: [i32; 4]) -> v128 { v128(cast(a)) }
#[inline]
fn vi16(a: [i16; 8]) -> v128 { v128(cast(a)) }
#[inline]
fn vu16(a: [u16; 8]) -> v128 { v128(cast(a)) }
#[inline]
fn vf64(a: [f64; 2]) -> v128 { v128(cast(a)) }

#[inline]
fn wf(v: v128) -> wide::f32x4 { wide::f32x4::new(af32(v)) }
#[inline]
fn uwf(w: wide::f32x4) -> v128 { vf32(w.to_array()) }
#[inline]
fn wi(v: v128) -> wide::i32x4 { wide::i32x4::new(ai32(v)) }
#[inline]
fn uwi(w: wide::i32x4) -> v128 { vi32(w.to_array()) }

#[inline]
pub fn f32x4(a0: f32, a1: f32, a2: f32, a3: f32) -> v128 { vf32([a0, a1, a2, a3]) }
#[inline]
pub fn i32x4(a0: i32, a1: i32, a2: i32, a3: i32) -> v128 { vi32([a0, a1, a2, a3]) }
#[inline]
pub fn f64x2(a0: f64, a1: f64) -> v128 { vf64([a0, a1]) }

#[inline]
pub fn f32x4_splat(a: f32) -> v128 { vf32([a; 4]) }
#[inline]
pub fn f32x4_add(a: v128, b: v128) -> v128 { uwf(wf(a) + wf(b)) }
#[inline]
pub fn f32x4_sub(a: v128, b: v128) -> v128 { uwf(wf(a) - wf(b)) }
#[inline]
pub fn f32x4_mul(a: v128, b: v128) -> v128 { uwf(wf(a) * wf(b)) }
#[inline]
pub fn f32x4_div(a: v128, b: v128) -> v128 { uwf(wf(a) / wf(b)) }
#[inline]
pub fn f32x4_min(a: v128, b: v128) -> v128 { uwf(wf(a).min(wf(b))) }
#[inline]
pub fn f32x4_max(a: v128, b: v128) -> v128 { uwf(wf(a).max(wf(b))) }
#[inline]
pub fn f32x4_neg(a: v128) -> v128 { vf32(af32(a).map(|x| -x)) }
#[inline]
pub fn f32x4_abs(a: v128) -> v128 { uwf(wf(a).abs()) }
#[inline]
pub fn f32x4_sqrt(a: v128) -> v128 { uwf(wf(a).sqrt()) }
#[inline]
pub fn f32x4_floor(a: v128) -> v128 { uwf(wf(a).floor()) }
#[inline]
pub fn f32x4_nearest(a: v128) -> v128 { uwf(wf(a).round()) }
#[inline]
pub fn f32x4_ge(a: v128, b: v128) -> v128 { uwf(wf(a).simd_ge(wf(b))) }
#[inline]
pub fn f32x4_gt(a: v128, b: v128) -> v128 { uwf(wf(a).simd_gt(wf(b))) }
#[inline]
pub fn f32x4_le(a: v128, b: v128) -> v128 { uwf(wf(a).simd_le(wf(b))) }
#[inline]
pub fn f32x4_lt(a: v128, b: v128) -> v128 { uwf(wf(a).simd_lt(wf(b))) }
#[inline]
pub fn f32x4_ne(a: v128, b: v128) -> v128 { uwf(wf(a).simd_ne(wf(b))) }
#[inline]
pub fn f32x4_extract_lane<const N: usize>(a: v128) -> f32 { af32(a)[N] }
#[inline]
pub fn f32x4_replace_lane<const N: usize>(a: v128, x: f32) -> v128 {
    let mut l = af32(a);
    l[N] = x;
    vf32(l)
}
#[inline]
pub fn f32x4_convert_u32x4(a: v128) -> v128 { vf32(au32(a).map(|x| x as f32)) }

#[inline]
pub fn f64x2_splat(a: f64) -> v128 { vf64([a; 2]) }
#[inline]
pub fn f64x2_add(a: v128, b: v128) -> v128 {
    vf64((wide::f64x2::new(af64(a)) + wide::f64x2::new(af64(b))).to_array())
}
#[inline]
pub fn f64x2_mul(a: v128, b: v128) -> v128 {
    vf64((wide::f64x2::new(af64(a)) * wide::f64x2::new(af64(b))).to_array())
}
#[inline]
pub fn f64x2_extract_lane<const N: usize>(a: v128) -> f64 { af64(a)[N] }

#[inline]
pub fn i32x4_splat(a: i32) -> v128 { vi32([a; 4]) }
#[inline]
pub fn i32x4_add(a: v128, b: v128) -> v128 { uwi(wi(a) + wi(b)) }
#[inline]
pub fn i32x4_mul(a: v128, b: v128) -> v128 { uwi(wi(a) * wi(b)) }
#[inline]
pub fn i32x4_eq(a: v128, b: v128) -> v128 { uwi(wi(a).simd_eq(wi(b))) }
#[inline]
pub fn i32x4_shl(a: v128, amt: u32) -> v128 {
    let s = amt & 31;
    vi32(ai32(a).map(|x| ((x as u32) << s) as i32))
}
#[inline]
pub fn i32x4_extract_lane<const N: usize>(a: v128) -> i32 { ai32(a)[N] }
#[inline]
pub fn i32x4_bitmask(a: v128) -> u8 {
    let l = ai32(a);
    let mut m = 0u8;
    for i in 0..4 {
        m |= (((l[i] as u32) >> 31) as u8) << i;
    }
    m
}
#[inline]
pub fn i32x4_trunc_sat_f32x4(a: v128) -> v128 { vi32(af32(a).map(|x| x as i32)) }

#[inline]
pub fn i16x8_splat(a: i16) -> v128 { vi16([a; 8]) }
#[inline]
pub fn i16x8_add(a: v128, b: v128) -> v128 {
    vi16((wide::i16x8::new(ai16(a)) + wide::i16x8::new(ai16(b))).to_array())
}
#[inline]
pub fn i16x8_mul(a: v128, b: v128) -> v128 {
    vi16((wide::i16x8::new(ai16(a)) * wide::i16x8::new(ai16(b))).to_array())
}

#[inline]
pub fn u16x8_extend_low_u8x16(a: v128) -> v128 {
    let b = a.0;
    let mut o = [0u16; 8];
    for i in 0..8 {
        o[i] = b[i] as u16;
    }
    vu16(o)
}
#[inline]
pub fn u16x8_extend_high_u8x16(a: v128) -> v128 {
    let b = a.0;
    let mut o = [0u16; 8];
    for i in 0..8 {
        o[i] = b[i + 8] as u16;
    }
    vu16(o)
}
#[inline]
pub fn u16x8_narrow_i32x4(a: v128, b: v128) -> v128 {
    let (la, lb) = (ai32(a), ai32(b));
    let mut o = [0u16; 8];
    for i in 0..4 {
        o[i] = la[i].clamp(0, 65535) as u16;
        o[i + 4] = lb[i].clamp(0, 65535) as u16;
    }
    vu16(o)
}
#[inline]
pub fn u16x8_shr(a: v128, amt: u32) -> v128 {
    let s = amt & 15;
    vu16(au16(a).map(|x| x >> s))
}

#[inline]
pub fn u32x4_extend_low_u16x8(a: v128) -> v128 {
    let l = au16(a);
    vi32([l[0] as i32, l[1] as i32, l[2] as i32, l[3] as i32])
}

#[inline]
pub fn u8x16_splat(a: u8) -> v128 { v128([a; 16]) }
#[inline]
pub fn u8x16_min(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a.0[i].min(b.0[i]);
    }
    v128(o)
}
#[inline]
pub fn u8x16_max(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a.0[i].max(b.0[i]);
    }
    v128(o)
}
#[inline]
pub fn u8x16_sub(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a.0[i].wrapping_sub(b.0[i]);
    }
    v128(o)
}
#[inline]
pub fn u8x16_sub_sat(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a.0[i].saturating_sub(b.0[i]);
    }
    v128(o)
}
#[inline]
pub fn u8x16_narrow_i16x8(a: v128, b: v128) -> v128 {
    let (la, lb) = (ai16(a), ai16(b));
    let mut o = [0u8; 16];
    for i in 0..8 {
        o[i] = la[i].clamp(0, 255) as u8;
        o[i + 8] = lb[i].clamp(0, 255) as u8;
    }
    v128(o)
}

#[inline]
pub fn i8x16_eq(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = if a.0[i] == b.0[i] { 0xFF } else { 0 };
    }
    v128(o)
}
#[inline]
pub fn i8x16_bitmask(a: v128) -> u16 {
    let mut m = 0u16;
    for i in 0..16 {
        m |= ((a.0[i] >> 7) as u16) << i;
    }
    m
}
#[inline]
pub fn i8x16_shuffle<
    const I0: usize, const I1: usize, const I2: usize, const I3: usize,
    const I4: usize, const I5: usize, const I6: usize, const I7: usize,
    const I8: usize, const I9: usize, const I10: usize, const I11: usize,
    const I12: usize, const I13: usize, const I14: usize, const I15: usize,
>(a: v128, b: v128) -> v128 {
    let idx = [I0, I1, I2, I3, I4, I5, I6, I7, I8, I9, I10, I11, I12, I13, I14, I15];
    let mut o = [0u8; 16];
    for i in 0..16 {
        let j = idx[i];
        o[i] = if j < 16 { a.0[j] } else { b.0[j - 16] };
    }
    v128(o)
}

#[inline]
pub fn v128_and(a: v128, b: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = a.0[i] & b.0[i];
    }
    v128(o)
}
#[inline]
pub fn v128_not(a: v128) -> v128 { v128(a.0.map(|x| !x)) }
#[inline]
pub fn v128_bitselect(a: v128, b: v128, c: v128) -> v128 {
    let mut o = [0u8; 16];
    for i in 0..16 {
        o[i] = (a.0[i] & c.0[i]) | (b.0[i] & !c.0[i]);
    }
    v128(o)
}
#[inline]
pub fn v128_any_true(a: v128) -> bool { a.0.iter().any(|&x| x != 0) }

/// # Safety
/// `m` must point to 16 readable bytes.
#[inline]
pub unsafe fn v128_load(m: *const v128) -> v128 {
    v128(core::ptr::read_unaligned(m as *const [u8; 16]))
}
/// # Safety
/// `m` must point to 16 writable bytes.
#[inline]
pub unsafe fn v128_store(m: *mut v128, a: v128) {
    core::ptr::write_unaligned(m as *mut [u8; 16], a.0);
}
/// # Safety
/// `m` must point to 4 readable bytes.
#[inline]
pub unsafe fn v128_load32_zero(m: *const u32) -> v128 {
    let x = core::ptr::read_unaligned(m);
    vi32([x as i32, 0, 0, 0])
}
