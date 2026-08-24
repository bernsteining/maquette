//! Procedural HDR environment map for image-based lighting.
//!
//! Uses **octahedral direction encoding** (Meyer et al.). Direction ↔ UV is
//! all abs/sign/mul/add — no `atan2` or `acos` in the per-pixel sample path,
//! which halves shader cost on IBL-heavy renders vs. equirectangular.
//!
//! Storage: 128×128 linear f32 RGB with a box-filter mip chain. The whole
//! sphere unfolds into a single square (upper hemisphere fills the outer
//! diamond, lower hemisphere fills the four corners via the sign-fold).
//!
//! Encode (direction → UV) — used at sample time:
//!   p = dir / (|dir.x| + |dir.y| + |dir.z|)
//!   if p.y >= 0: (u, v) = (p.x, p.z)
//!   else:        (u, v) = ((1 - |p.z|) · sign(p.x), (1 - |p.x|) · sign(p.z))
//!   uv = ((u + 1) · 0.5, (v + 1) · 0.5)
//!
//! Decode (UV → direction) — used at build time:
//!   inverse of the above, then normalise.
//!
//! Diffuse: sample at N with high mip (heavily blurred → hemispheric average).
//! Specular: sample at R = reflect(-V, N) with mip = roughness · max_lod.

use crate::math::Vec3;

const SIDE: u32 = 512;

/// A single mip level of the octahedral map. Row-major RGB f32.
struct MipF32 {
    width: u32,
    height: u32,
    rgb: Vec<f32>,
}

pub struct IblEnvironment {
    mips: Vec<MipF32>,
    pub max_lod: f32,
    /// Cosine-weighted diffuse irradiance map — a small (32²) octahedral
    /// texture containing `∫ env(D') · max(0, N·D') dω'` for every
    /// direction N. Used for diffuse IBL sampling; using the box-filtered
    /// smallest mip instead (what we used to do) creates visible quadrant
    /// banding on smoothly-curved surfaces because box radiance ≠ cosine
    /// irradiance.
    diffuse: MipF32,
}

impl IblEnvironment {
    /// Build from a decoded equirectangular HDR (2:1 aspect, linear RGB f32).
    /// Reprojects into the octahedral square, generates the mip chain, applies
    /// `intensity` scale and `rotation` (radians around the up/+Y axis).
    pub fn build_from_equirect(equirect: &[f32], eq_w: u32, eq_h: u32, intensity: f32, rotation: f32) -> Self {
        assert!((eq_w * eq_h * 3) as usize == equirect.len(), "equirect RGB length mismatch");
        let mut base_rgb = Vec::with_capacity((SIDE * SIDE * 3) as usize);
        let (rc, rs) = (rotation.cos(), rotation.sin());
        for j in 0..SIDE {
            for i in 0..SIDE {
                let u = (i as f32 + 0.5) / SIDE as f32;
                let v = (j as f32 + 0.5) / SIDE as f32;
                let (dx, dy, dz) = octahedral_decode(u, v);
                // Rotate around +Y — spec-canonical HDR orientation control.
                let (rx, rz) = (rc * dx + rs * dz, -rs * dx + rc * dz);
                // Convert unit dir → equirect UV. `atan2` is only paid at
                // build time (one-shot), not per-pixel. Standard convention:
                //   u = 0.5 + atan2(z, x) / (2π); v = 0.5 − asin(y) / π.
                let phi = rz.atan2(rx);
                let theta = dy.clamp(-1.0, 1.0).asin();
                let eu = (0.5 + phi / (2.0 * std::f32::consts::PI)).rem_euclid(1.0);
                let ev = (0.5 - theta / std::f32::consts::PI).clamp(0.0, 1.0);
                let ex = (eu * eq_w as f32) as usize;
                let ey = (ev * eq_h as f32) as usize;
                let ex = ex.min(eq_w as usize - 1);
                let ey = ey.min(eq_h as usize - 1);
                let off = (ey * eq_w as usize + ex) * 3;
                base_rgb.push((equirect[off]     * intensity).max(0.0));
                base_rgb.push((equirect[off + 1] * intensity).max(0.0));
                base_rgb.push((equirect[off + 2] * intensity).max(0.0));
            }
        }
        Self::finish(base_rgb)
    }

    fn finish(base_rgb: Vec<f32>) -> Self {
        let mut mips = vec![MipF32 { width: SIDE, height: SIDE, rgb: base_rgb }];
        loop {
            let cur = mips.last().unwrap();
            if cur.width <= 4 || cur.height <= 4 { break; }
            mips.push(downsample_2x(cur));
        }
        let max_lod = mips.len().saturating_sub(1) as f32;
        // Convolve the mid-chain mip with a cosine hemisphere to get the
        // diffuse irradiance map. Using a coarse source mip (32² or 16²)
        // keeps the convolution O(2K samples per output texel) which is
        // ~5 ms one-shot at DIFFUSE_SIDE = 32.
        let diffuse = build_diffuse_irradiance(&mips);
        Self { mips, max_lod, diffuse }
    }

    pub fn build(sky: [f32; 3], ground: [f32; 3], intensity: f32, sun_dir: Vec3) -> Self {
        let mut base_rgb = Vec::with_capacity((SIDE * SIDE * 3) as usize);
        let sun = sun_dir.normalized();
        for j in 0..SIDE {
            for i in 0..SIDE {
                // Texel center in UV space.
                let u = (i as f32 + 0.5) / SIDE as f32;
                let v = (j as f32 + 0.5) / SIDE as f32;
                let (dx, dy, dz) = octahedral_decode(u, v);

                // Hemispheric sky/ground blend by dir.y (up axis).
                let t = ((dy + 1.0) * 0.5).clamp(0.0, 1.0);
                let mut r = (ground[0] + t * (sky[0] - ground[0])) * intensity;
                let mut g = (ground[1] + t * (sky[1] - ground[1])) * intensity;
                let mut b = (ground[2] + t * (sky[2] - ground[2])) * intensity;

                // HDR sun-region highlight — a large, smoothly-falling
                // radiance bump so metals get pop from a directional
                // "sun". A previous impl used a 5.7° hard disc at 8×
                // intensity, but the sharp step aliases badly through
                // the 2×2 box-filter mip chain: mid-roughness samples
                // saw the sun spike echoed across mip texels, showing
                // as a visible grid/hex pattern on smooth surfaces.
                //
                // Cosine-squared falloff over a wide angle spreads the
                // energy over enough texels that the mip chain no
                // longer aliases. Peak intensity reduced (3× instead of
                // 8×) to keep the total energy roughly equivalent to
                // the old hard disc.
                let d = dx * sun.x as f32 + dy * sun.y as f32 + dz * sun.z as f32;
                if d > 0.0 {
                    // `d²` grows sun-ward; `d^32` keeps it fairly tight
                    // (~10° half-angle FWHM) while staying C¹-smooth
                    // and mip-safe.
                    let d2 = d * d;
                    let d8 = d2 * d2 * d2 * d2;
                    let d32 = d8 * d8 * d8 * d8;
                    let sun_boost = 3.0 * intensity * d32;
                    r += sun_boost;
                    g += sun_boost;
                    b += sun_boost;
                }
                base_rgb.push(r.max(0.0));
                base_rgb.push(g.max(0.0));
                base_rgb.push(b.max(0.0));
            }
        }
        Self::finish(base_rgb)
    }

    /// Diffuse IBL — cosine-weighted irradiance in direction `(dx,dy,dz)`.
    /// Sample this instead of `sample_dir(_, max_lod)` for the diffuse
    /// ambient contribution.
    #[inline(always)]
    pub fn sample_diffuse(&self, dx: f32, dy: f32, dz: f32) -> [f32; 3] {
        sample_seam_aware(&self.diffuse, dx, dy, dz)
    }

    /// Scalar sample at a world-space direction. Returns linear RGB in HDR range.
    #[inline(always)]
    pub fn sample_dir(&self, dx: f32, dy: f32, dz: f32, lod: f32) -> [f32; 3] {
        let clamped = lod.clamp(0.0, self.max_lod);
        let lo = clamped.floor() as usize;
        let hi = (lo + 1).min(self.max_lod as usize);
        let frac = clamped - lo as f32;
        let a = sample_seam_aware(&self.mips[lo], dx, dy, dz);
        if frac < 1e-4 || lo == hi { return a; }
        let b = sample_seam_aware(&self.mips[hi], dx, dy, dz);
        let ifrac = 1.0 - frac;
        [
            a[0] * ifrac + b[0] * frac,
            a[1] * ifrac + b[1] * frac,
            a[2] * ifrac + b[2] * frac,
        ]
    }
}

/// Encode a (not-necessarily-unit) direction as octahedral UV in [0, 1]².
/// Zero-vector maps to the +Y pole. No transcendentals.
#[inline(always)]
fn octahedral_encode(dx: f32, dy: f32, dz: f32) -> (f32, f32) {
    let l1 = dx.abs() + dy.abs() + dz.abs();
    if l1 < 1e-14 { return (0.5, 0.5); }
    let inv = 1.0 / l1;
    let (px, py, pz) = (dx * inv, dy * inv, dz * inv);
    let (u, v) = if py >= 0.0 {
        (px, pz)
    } else {
        let sx = if px >= 0.0 { 1.0 } else { -1.0 };
        let sz = if pz >= 0.0 { 1.0 } else { -1.0 };
        ((1.0 - pz.abs()) * sx, (1.0 - px.abs()) * sz)
    };
    (u * 0.5 + 0.5, v * 0.5 + 0.5)
}

/// Build a small octahedral map of cosine-weighted diffuse irradiance:
/// each output texel N stores `Σ env(D_k) · max(0, N·D_k)` for a set of
/// directions D_k sampled uniformly on the sphere. Divided by the sum of
/// weights so intensity matches the average env radiance.
///
/// The convolution source is a MID mip (16²), not the base (SIDE²), which
/// keeps this ~2K env samples × 32² outputs = ~2M ops. One-shot, ~5 ms.
fn build_diffuse_irradiance(mips: &[MipF32]) -> MipF32 {
    const OUT_SIDE: u32 = 32;
    // Pick a source mip small enough to keep the double loop cheap but
    // large enough to preserve the sun-region highlight direction. 16²
    // (~256 texels) is a good compromise.
    let src = mips.iter().min_by_key(|m| {
        // Prefer the largest mip whose linear dimension is ≤ 16.
        if m.width > 16 { u32::MAX } else { 16 - m.width }
    }).unwrap_or(mips.last().unwrap());
    let sw = src.width as usize;
    let sh = src.height as usize;

    // Precompute src texel direction + solid-angle weight. Solid angle
    // for an octahedral texel scales with 1 / |dv/du × dv/dv| but for a
    // coarse map the differences are small; a plain 1.0 weight is close
    // enough and this stays a one-shot cost.
    let mut src_dirs = Vec::with_capacity(sw * sh);
    for j in 0..sh {
        for i in 0..sw {
            let u = (i as f32 + 0.5) / sw as f32;
            let v = (j as f32 + 0.5) / sh as f32;
            let (dx, dy, dz) = octahedral_decode(u, v);
            src_dirs.push((dx, dy, dz));
        }
    }

    let mut rgb = vec![0.0f32; (OUT_SIDE * OUT_SIDE * 3) as usize];
    for j in 0..OUT_SIDE {
        for i in 0..OUT_SIDE {
            let u = (i as f32 + 0.5) / OUT_SIDE as f32;
            let v = (j as f32 + 0.5) / OUT_SIDE as f32;
            let (nx, ny, nz) = octahedral_decode(u, v);

            let mut sum_r = 0.0f32;
            let mut sum_g = 0.0f32;
            let mut sum_b = 0.0f32;
            let mut sum_w = 0.0f32;
            for (k, &(dx, dy, dz)) in src_dirs.iter().enumerate() {
                let n_dot_d = nx * dx + ny * dy + nz * dz;
                if n_dot_d <= 0.0 { continue; }
                let off = k * 3;
                sum_r += src.rgb[off]     * n_dot_d;
                sum_g += src.rgb[off + 1] * n_dot_d;
                sum_b += src.rgb[off + 2] * n_dot_d;
                sum_w += n_dot_d;
            }
            let inv_w = if sum_w > 1e-6 { 1.0 / sum_w } else { 0.0 };
            let o = (j as usize * OUT_SIDE as usize + i as usize) * 3;
            rgb[o]     = sum_r * inv_w;
            rgb[o + 1] = sum_g * inv_w;
            rgb[o + 2] = sum_b * inv_w;
        }
    }
    MipF32 { width: OUT_SIDE, height: OUT_SIDE, rgb }
}

/// Inverse of `octahedral_encode` — used at build time to fill the map.
#[inline]
fn octahedral_decode(u: f32, v: f32) -> (f32, f32, f32) {
    let (x, z) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    let y = 1.0 - x.abs() - z.abs();
    let (mut x, mut z) = (x, z);
    if y < 0.0 {
        let sx = if x >= 0.0 { 1.0 } else { -1.0 };
        let sz = if z >= 0.0 { 1.0 } else { -1.0 };
        let (nx, nz) = ((1.0 - z.abs()) * sx, (1.0 - x.abs()) * sz);
        x = nx; z = nz;
    }
    let len = (x * x + y * y + z * z).sqrt().max(1e-14);
    (x / len, y / len, z / len)
}

/// Bilinear sample of one mip. Clamp both axes (octahedral wraps oddly, so
/// clamp is safer than repeat for MVP).
#[inline(always)]
fn sample_mip_bilinear(mip: &MipF32, u: f32, v: f32) -> [f32; 3] {
    let uc = u.clamp(0.0, 1.0);
    let vc = v.clamp(0.0, 1.0);
    let x = uc * mip.width as f32 - 0.5;
    let y = vc * mip.height as f32 - 0.5;
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let w = mip.width as i32;
    let h = mip.height as i32;
    let x0c = x0.clamp(0, w - 1);
    let x1c = (x0 + 1).clamp(0, w - 1);
    let y0c = y0.clamp(0, h - 1);
    let y1c = (y0 + 1).clamp(0, h - 1);
    let p = |xi: i32, yi: i32| -> [f32; 3] {
        let off = ((yi as usize * mip.width as usize + xi as usize) * 3) as usize;
        [mip.rgb[off], mip.rgb[off + 1], mip.rgb[off + 2]]
    };
    let p00 = p(x0c, y0c);
    let p10 = p(x1c, y0c);
    let p01 = p(x0c, y1c);
    let p11 = p(x1c, y1c);
    let ix = 1.0 - fx;
    let iy = 1.0 - fy;
    [
        (p00[0]*ix + p10[0]*fx)*iy + (p01[0]*ix + p11[0]*fx)*fy,
        (p00[1]*ix + p10[1]*fx)*iy + (p01[1]*ix + p11[1]*fx)*fy,
        (p00[2]*ix + p10[2]*fx)*iy + (p01[2]*ix + p11[2]*fx)*fy,
    ]
}

/// Seam-aware sampling that side-steps the octahedral encoding's
/// derivative discontinuity (at the equator fold `py = 0` and along the
/// outer-square branch cuts). Plain bilinear interpolates using UV
/// weights, which is wrong across the discontinuity — texels that are
/// atlas-adjacent may represent very different 3D directions. Here we
/// decode each of the four bilinear taps back to its direction, weight
/// by the dot product with the query direction, and average.
///
/// Texels whose decoded direction disagrees with the query (dot ≤ 0)
/// get zero weight, so the discontinuity is treated as data rather than
/// coordinates. Cost: 4 decodes + 4 dot products vs the plain 4 taps.
/// Falls back to plain bilinear weights when all four texels agree with
/// the query — the interior of a hemisphere, which is most of the map.
#[inline(always)]
fn sample_seam_aware(mip: &MipF32, dx: f32, dy: f32, dz: f32) -> [f32; 3] {
    let (u, v) = octahedral_encode(dx, dy, dz);
    let uc = u.clamp(0.0, 1.0);
    let vc = v.clamp(0.0, 1.0);
    let mw = mip.width as f32;
    let mh = mip.height as f32;
    let x = uc * mw - 0.5;
    let y = vc * mh - 0.5;
    let x0i = x.floor() as i32;
    let y0i = y.floor() as i32;
    let fx = x - x0i as f32;
    let fy = y - y0i as f32;
    let wi = mip.width as i32;
    let hi = mip.height as i32;
    let x0 = x0i.clamp(0, wi - 1);
    let x1 = (x0i + 1).clamp(0, wi - 1);
    let y0 = y0i.clamp(0, hi - 1);
    let y1 = (y0i + 1).clamp(0, hi - 1);

    // Normalise query dir once — dot product is scale-independent but
    // callers may pass unnormalised reflection vectors.
    let inv_len = 1.0 / (dx * dx + dy * dy + dz * dz).sqrt().max(1e-14);
    let qx = dx * inv_len;
    let qy = dy * inv_len;
    let qz = dz * inv_len;

    let sw = mip.width as usize;
    let inv_mw = 1.0 / mw;
    let inv_mh = 1.0 / mh;

    // Per-tap: decode texel's direction, dot with query, weight = dot × bilinear.
    // Inlined four times to avoid the closure that made wasmi's translator
    // fail with "cmp+branch fusion must succeed".
    let ix = 1.0 - fx;
    let iy = 1.0 - fy;

    let (tu, tv) = ((x0 as f32 + 0.5) * inv_mw, (y0 as f32 + 0.5) * inv_mh);
    let (tdx, tdy, tdz) = octahedral_decode(tu, tv);
    let w00 = (qx * tdx + qy * tdy + qz * tdz).max(0.0) * ix * iy;
    let off00 = (y0 as usize * sw + x0 as usize) * 3;

    let (tu, tv) = ((x1 as f32 + 0.5) * inv_mw, (y0 as f32 + 0.5) * inv_mh);
    let (tdx, tdy, tdz) = octahedral_decode(tu, tv);
    let w10 = (qx * tdx + qy * tdy + qz * tdz).max(0.0) * fx * iy;
    let off10 = (y0 as usize * sw + x1 as usize) * 3;

    let (tu, tv) = ((x0 as f32 + 0.5) * inv_mw, (y1 as f32 + 0.5) * inv_mh);
    let (tdx, tdy, tdz) = octahedral_decode(tu, tv);
    let w01 = (qx * tdx + qy * tdy + qz * tdz).max(0.0) * ix * fy;
    let off01 = (y1 as usize * sw + x0 as usize) * 3;

    let (tu, tv) = ((x1 as f32 + 0.5) * inv_mw, (y1 as f32 + 0.5) * inv_mh);
    let (tdx, tdy, tdz) = octahedral_decode(tu, tv);
    let w11 = (qx * tdx + qy * tdy + qz * tdz).max(0.0) * fx * fy;
    let off11 = (y1 as usize * sw + x1 as usize) * 3;

    let sum_w = w00 + w10 + w01 + w11;
    if sum_w < 1e-6 {
        // All four taps disagree with the query direction — fall back
        // to nearest. Rare, mostly near polar singularities.
        let off = if fx < 0.5 {
            if fy < 0.5 { off00 } else { off01 }
        } else {
            if fy < 0.5 { off10 } else { off11 }
        };
        return [mip.rgb[off], mip.rgb[off + 1], mip.rgb[off + 2]];
    }
    let inv = 1.0 / sum_w;
    [
        (mip.rgb[off00]     * w00 + mip.rgb[off10]     * w10 + mip.rgb[off01]     * w01 + mip.rgb[off11]     * w11) * inv,
        (mip.rgb[off00 + 1] * w00 + mip.rgb[off10 + 1] * w10 + mip.rgb[off01 + 1] * w01 + mip.rgb[off11 + 1] * w11) * inv,
        (mip.rgb[off00 + 2] * w00 + mip.rgb[off10 + 2] * w10 + mip.rgb[off01 + 2] * w01 + mip.rgb[off11 + 2] * w11) * inv,
    ]
}

fn downsample_2x(src: &MipF32) -> MipF32 {
    let nw = (src.width / 2).max(1);
    let nh = (src.height / 2).max(1);
    let mut rgb = vec![0.0f32; (nw * nh * 3) as usize];
    let sw = src.width as usize;
    for y in 0..nh as usize {
        let sy = y * 2;
        for x in 0..nw as usize {
            let sx = x * 2;
            let o = (y * nw as usize + x) * 3;
            for c in 0..3 {
                let a = src.rgb[(sy * sw + sx) * 3 + c];
                let b = src.rgb[(sy * sw + sx + 1) * 3 + c];
                let cc = src.rgb[((sy + 1) * sw + sx) * 3 + c];
                let d = src.rgb[((sy + 1) * sw + sx + 1) * 3 + c];
                rgb[o + c] = (a + b + cc + d) * 0.25;
            }
        }
    }
    MipF32 { width: nw, height: nh, rgb }
}
