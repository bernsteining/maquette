//! Sampled 2D texture with wrap + filter modes and a box-filtered mip chain.
//!
//! Stored as raw RGBA8 mips (no colorspace conversion at decode time). The
//! shader converts sRGB→linear per-sample via the existing LUT, since glTF's
//! texture semantic dictates the space (baseColor & emissive are sRGB;
//! MR, normal, occlusion are linear).
//!
//! Mip chain is generated at load time by 2× box filter, down to a floor of
//! 4×4 (or whichever dimension bottoms out first). Per-triangle LOD is picked
//! from the ratio of texel area to screen area; bilinear sample from the
//! chosen level. Trilinear (blend two mips) is a v3 improvement.
//!
//! Returned samples are `[f32; 4]` in `[0, 1]`. Caller applies sRGB→linear.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wrap { Repeat, ClampToEdge, MirroredRepeat }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter { Nearest, Linear }

pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, `width · height · 4` bytes.
    pub rgba: Vec<u8>,
}

pub struct Texture {
    /// Mip 0 is the largest; each subsequent level is half-sized (rounded down).
    pub mips: Vec<MipLevel>,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
    pub mag_filter: Filter,
    pub min_filter: Filter,
    /// `0.5 · log₂(base_w · base_h)` — half the constant term of the shader's
    /// per-triangle LOD formula `0.5 · log₂(lod_scale · w · h)`, precomputed
    /// once at texture load so the shader only pays `0.5 · log₂(lod_scale)`
    /// (an add + a log, but the `w · h` factor drops out).
    pub lod_bias: f32,
}

impl Texture {
    /// Sample the texture. `lod` is a floating LOD value where 0 = mip 0
    /// (highest res). Picks `mag_filter` when the texture is being magnified
    /// (lod ≤ 0, i.e. one texel maps to ≥ 1 pixel) and `min_filter` when
    /// minified — glTF spec §3.8.4.4.
    #[inline(always)]
    pub fn sample_lod(&self, uv: [f32; 2], lod: f32) -> [f32; 4] {
        let max_level = self.mips.len().saturating_sub(1) as f32;
        let lod = lod.clamp(0.0, max_level);
        let u = wrap_coord(uv[0], self.wrap_s);
        let v = wrap_coord(uv[1], self.wrap_t);
        // Mag path (lod ≤ 0 ≈ oversampling) uses mag_filter — mag is either
        // Nearest or Linear per spec, no mipmap variant. Min path picks
        // min_filter with its full mipmap semantics.
        let filter = if lod <= 0.5 { self.mag_filter } else { self.min_filter };
        match filter {
            Filter::Nearest => sample_nearest(&self.mips[lod.round() as usize], u, v),
            Filter::Linear => {
                let lo = lod.floor() as usize;
                let hi = (lo + 1).min(max_level as usize);
                let frac = lod - lo as f32;
                let a = sample_bilinear(&self.mips[lo], u, v, self.wrap_s, self.wrap_t);
                if frac < 1e-4 || lo == hi { return a; }
                let b = sample_bilinear(&self.mips[hi], u, v, self.wrap_s, self.wrap_t);
                let ifrac = 1.0 - frac;
                [
                    a[0]*ifrac + b[0]*frac,
                    a[1]*ifrac + b[1]*frac,
                    a[2]*ifrac + b[2]*frac,
                    a[3]*ifrac + b[3]*frac,
                ]
            }
        }
    }

    /// Convenience: sample mip 0 (used by callers that don't compute LOD).
    #[inline]
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        self.sample_lod(uv, 0.0)
    }

    /// Base level dimensions, for the LOD calculation on the caller side.
    #[inline]
    pub fn base_dims(&self) -> (u32, u32) {
        let m = &self.mips[0];
        (m.width, m.height)
    }
}

/// Build a mip chain by repeated 2× box downsampling. Bottoms out when either
/// dimension would drop below `MIN_DIM`. Base level is `base`.
pub fn build_mips(base: MipLevel) -> Vec<MipLevel> {
    const MIN_DIM: u32 = 4;
    let mut mips = Vec::with_capacity(12);
    mips.push(base);
    loop {
        let cur = mips.last().unwrap();
        if cur.width <= MIN_DIM || cur.height <= MIN_DIM { break; }
        mips.push(downsample_2x(cur));
    }
    mips
}

/// Public alias so the scene loader can pre-cap texture resolution with the
/// same box filter that builds mips.
#[inline]
pub fn downsample_2x_pub(src: &MipLevel) -> MipLevel { downsample_2x(src) }

/// 2× box filter — average of 2×2 texel blocks. Correct for linear textures;
/// slightly off for sRGB (should un-gamma, average, re-gamma) but visually
/// fine and much cheaper. Fix is trivial when we care: a pair of LUT lookups
/// per texel per level.
fn downsample_2x(src: &MipLevel) -> MipLevel {
    let nw = (src.width  / 2).max(1);
    let nh = (src.height / 2).max(1);
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    let sw = src.width as usize;
    for y in 0..nh as usize {
        let sy = y * 2;
        for x in 0..nw as usize {
            let sx = x * 2;
            let o = (y * nw as usize + x) * 4;
            for c in 0..4 {
                let a = src.rgba[(sy * sw + sx) * 4 + c] as u32;
                let b = src.rgba[(sy * sw + sx + 1) * 4 + c] as u32;
                let cc = src.rgba[((sy + 1) * sw + sx) * 4 + c] as u32;
                let d = src.rgba[((sy + 1) * sw + sx + 1) * 4 + c] as u32;
                out[o + c] = ((a + b + cc + d + 2) / 4) as u8;
            }
        }
    }
    MipLevel { width: nw, height: nh, rgba: out }
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

#[inline]
fn sample_nearest(mip: &MipLevel, u: f32, v: f32) -> [f32; 4] {
    let x = ((u * mip.width  as f32) as i32).clamp(0, mip.width  as i32 - 1);
    let y = ((v * mip.height as f32) as i32).clamp(0, mip.height as i32 - 1);
    pixel_norm(mip, x as u32, y as u32)
}

#[inline(always)]
fn sample_bilinear(mip: &MipLevel, u: f32, v: f32, wrap_s: Wrap, wrap_t: Wrap) -> [f32; 4] {
    let x = u * mip.width  as f32 - 0.5;
    let y = v * mip.height as f32 - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let (x0, x1) = neighbour_indices(x0, mip.width  as i32, wrap_s);
    let (y0, y1) = neighbour_indices(y0, mip.height as i32, wrap_t);

    // Load 4 texels as `v128` (RGBA8→f32×4 in one op chain per texel), then
    // do the bilinear weighted sum fully in SIMD. Was scalar per-channel:
    // 4 texels × 4 chans × 3 lerp ops (`p·ix + p·fx`, then y-lerp) = 48+
    // scalar ops. SIMD collapses that to ~15 f32x4 ops. Hot for any textured
    // material — this is called ~5-10 times per pixel with a typical PBR
    // asset (base, MR, normal, emissive, occlusion, [transmission]) bound.
    use std::arch::wasm32::*;
    let p00 = load_texel_simd(mip, x0 as u32, y0 as u32);
    let p10 = load_texel_simd(mip, x1 as u32, y0 as u32);
    let p01 = load_texel_simd(mip, x0 as u32, y1 as u32);
    let p11 = load_texel_simd(mip, x1 as u32, y1 as u32);
    let fx4 = f32x4_splat(fx);
    let ix4 = f32x4_splat(1.0 - fx);
    let fy4 = f32x4_splat(fy);
    let iy4 = f32x4_splat(1.0 - fy);
    let top = f32x4_add(f32x4_mul(p00, ix4), f32x4_mul(p10, fx4));
    let bot = f32x4_add(f32x4_mul(p01, ix4), f32x4_mul(p11, fx4));
    let out = f32x4_add(f32x4_mul(top, iy4), f32x4_mul(bot, fy4));
    [
        f32x4_extract_lane::<0>(out),
        f32x4_extract_lane::<1>(out),
        f32x4_extract_lane::<2>(out),
        f32x4_extract_lane::<3>(out),
    ]
}

/// Load one RGBA8 texel from `mip` at `(x, y)` and expand to `f32x4` in
/// `[0, 1]`. Replaces scalar `pixel_norm` — the u32 unaligned load + SIMD
/// unsigned-extend chain hits ~5 wasm ops vs the 8 (4 loads + 4 muls) of
/// the scalar version. Bounds are trusted: callers pass wrap/clamped x/y.
#[inline(always)]
fn load_texel_simd(mip: &MipLevel, x: u32, y: u32) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::*;
    let off = ((y * mip.width + x) * 4) as usize;
    unsafe {
        // Load the 4-byte RGBA texel as a u32 into the low lane of v128 (the
        // remaining 12 bytes are don't-care — they get shifted out below).
        let bytes = std::ptr::read_unaligned(mip.rgba.as_ptr().add(off) as *const u32);
        let v = i32x4(bytes as i32, 0, 0, 0);
        // Widen bytes → u16 → u32 via two unsigned extends: byte 0..3 of `v`
        // become the four u32 lanes.
        let u16s = u16x8_extend_low_u8x16(v);
        let u32s = u32x4_extend_low_u16x8(u16s);
        // Convert to f32x4 (unsigned) and normalise to [0, 1].
        let f = f32x4_convert_u32x4(u32s);
        f32x4_mul(f, f32x4_splat(1.0 / 255.0))
    }
}

#[inline(always)]
fn pixel_norm(mip: &MipLevel, x: u32, y: u32) -> [f32; 4] {
    let off = ((y * mip.width + x) * 4) as usize;
    let inv = 1.0 / 255.0;
    [
        mip.rgba[off]     as f32 * inv,
        mip.rgba[off + 1] as f32 * inv,
        mip.rgba[off + 2] as f32 * inv,
        mip.rgba[off + 3] as f32 * inv,
    ]
}

#[inline(always)]
fn wrap_coord(u: f32, wrap: Wrap) -> f32 {
    match wrap {
        Wrap::Repeat => u - u.floor(),
        Wrap::ClampToEdge => u.clamp(0.0, 1.0),
        Wrap::MirroredRepeat => {
            let a = u.floor();
            let f = u - a;
            if (a as i32) & 1 == 0 { f } else { 1.0 - f }
        }
    }
}

#[inline(always)]
fn neighbour_indices(x0: i32, w: i32, wrap: Wrap) -> (i32, i32) {
    match wrap {
        Wrap::Repeat => (rem_euclid(x0, w), rem_euclid(x0 + 1, w)),
        Wrap::ClampToEdge => (x0.clamp(0, w - 1), (x0 + 1).clamp(0, w - 1)),
        Wrap::MirroredRepeat => (mirror(x0, w), mirror(x0 + 1, w)),
    }
}

#[inline(always)]
fn rem_euclid(a: i32, b: i32) -> i32 {
    let r = a % b;
    if r < 0 { r + b } else { r }
}

#[inline]
fn mirror(a: i32, w: i32) -> i32 {
    let period = 2 * w;
    let m = rem_euclid(a, period);
    if m < w { m } else { period - 1 - m }
}
