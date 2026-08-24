/// Screen-Space Ambient Occlusion (SSAO) implementation.
/// Pre-computes integer sample offsets per noise pattern for fast per-pixel sampling.
/// Uses separable bilateral blur for noise reduction.

use std::arch::wasm32::*;

/// Parameters for SSAO computation.
#[derive(Clone)]
pub struct SSAOParams {
    pub samples: usize,      // Number of samples per pixel
    pub radius: f64,         // Sampling radius in screen space
    pub bias: f64,           // Depth bias to prevent self-occlusion
    pub strength: f64,       // AO effect strength (0.0-2.0)
}

impl Default for SSAOParams {
    fn default() -> Self {
        Self {
            samples: 16,
            radius: 0.5,
            bias: 0.025,
            strength: 1.0,
        }
    }
}

/// Pre-computed integer sample offset for one kernel sample at one noise rotation.
pub struct SampleOffset {
    pub dx: i32,
    pub dy: i32,
    pub z_bias: f32,
}

/// Number of unique rotations in the pool. Indexed per pixel via
/// `noise_index(x, y)`. 256 (vs. the old 16) survives SSAA 2× downsample:
/// at hi-res the pattern period is much larger than the 4-pixel bilateral
/// blur radius, so no residual tile shows through as a hex grid.
pub const NOISE_ROTATIONS: usize = 256;

/// Cheap integer hash → rotation index. Decorrelates neighbouring pixels
/// entirely — no 4×4 or 8×8 tile visible in the output. Replaces the
/// previous `(y & 3) * 4 + (x & 3)` tile lookup, which was a screen-space
/// grid of at-most 16 unique rotations. Combined with golden-angle
/// samples this fully hides the SSAO pattern.
#[inline(always)]
pub fn noise_index(x: i32, y: i32) -> usize {
    let ux = x as u32;
    let uy = y as u32;
    let h = ux.wrapping_mul(0x9e3779b1)
        .wrapping_add(uy.wrapping_mul(0x85ebca77))
        .wrapping_add(ux.wrapping_mul(uy).wrapping_mul(0xc2b2ae3d));
    let h = h ^ (h >> 16);
    let h = h.wrapping_mul(0x7feb352d);
    let h = h ^ (h >> 15);
    (h as usize) & (NOISE_ROTATIONS - 1)
}

/// Pre-compute all sample offsets for `NOISE_ROTATIONS × samples` — the
/// per-pixel loop then only does integer adds + depth compares.
pub fn precompute_sample_offsets(
    samples: usize,
    radius_px: f32,
    bias_scaled: f32,
) -> Vec<Vec<SampleOffset>> {
    use std::f64::consts::TAU;

    // Golden-angle sunflower distribution — samples spaced by 137.508°
    // (2π · (1 − 1/φ)). No two lie on the same ray. Uniform 2π/N spacing
    // (what we used before) gave every pixel the same 16-ray rosette, and
    // rotating by a fraction of 2π/N kept sample rays aligned across
    // neighbouring pixels.
    const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
    let mut kernel = Vec::with_capacity(samples);
    for i in 0..samples {
        let angle = i as f64 * GOLDEN_ANGLE;
        let r = ((i + 1) as f64 / samples as f64).sqrt();
        let x = angle.cos() * r;
        let y = angle.sin() * r;
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        let scale = (i as f64 / samples as f64).powi(2) * 0.9 + 0.1;
        kernel.push(((x * scale) as f32, (y * scale) as f32, (z * scale) as f32));
    }

    let mut offsets = Vec::with_capacity(NOISE_ROTATIONS);
    for n in 0..NOISE_ROTATIONS {
        // Bit-reverse the index so consecutive n values scatter over the
        // full hemisphere — combined with the hash lookup, adjacent pixels
        // always draw visually distinct rotations.
        let br = (n as u32).reverse_bits() >> (32 - 8);
        let angle = (br as f64 / NOISE_ROTATIONS as f64) * TAU;
        let (sin_a, cos_a) = angle.sin_cos();
        let cos_f = cos_a as f32;
        let sin_f = sin_a as f32;

        let mut pattern = Vec::with_capacity(samples);
        for &(kx, ky, kz) in &kernel {
            let rx = kx * cos_f - ky * sin_f;
            let ry = kx * sin_f + ky * cos_f;
            pattern.push(SampleOffset {
                dx: (rx * radius_px + 0.5) as i32,
                dy: (ry * radius_px + 0.5) as i32,
                z_bias: kz * bias_scaled,
            });
        }
        offsets.push(pattern);
    }
    offsets
}

/// Schraudolph's fast exp() approximation for f32.
/// ~3% max relative error, sufficient for blur weighting.
#[inline(always)]
fn fast_exp(x: f32) -> f32 {
    f32::from_bits((12102203.0f32 * x + 1065353216.0) as u32)
}

/// Schraudolph's fast exp() for 4 values via f32x4 SIMD.
#[inline(always)]
fn fast_exp_4(x: v128) -> v128 {
    i32x4_trunc_sat_f32x4(f32x4_add(
        f32x4_mul(f32x4_splat(12102203.0), x), f32x4_splat(1065353216.0),
    ))
}

/// Separable bilateral blur: horizontal pass then vertical pass.
/// Reduces per-pixel work from O(r^2) to O(2r) while preserving depth edges.
/// Uses f32x4 SIMD for interior pixels (4 pixels per iteration).
pub fn bilateral_blur_separable(
    ao_buffer: &[f32],
    depth_buffer: &[f32],
    width: usize,
    height: usize,
    blur_radius: i32,
) -> Vec<f32> {
    let n = width * height;
    let r = blur_radius;

    // Compute depth range for normalizing depth weights
    let mut zmin = f32::MAX;
    let mut zmax = f32::MIN;
    for &d in depth_buffer {
        if d != f32::NEG_INFINITY {
            if d < zmin { zmin = d; }
            if d > zmax { zmax = d; }
        }
    }
    let inv_depth_range = 1.0 / (zmax - zmin).max(0.001);
    let depth_factor = 50.0 * inv_depth_range;

    // Pre-compute 1D spatial weights (symmetric, only depends on |d|)
    let inv_r2 = 1.0 / (r * r) as f32;
    let ksize = (2 * r + 1) as usize;
    let mut spatial_w = vec![0.0f32; ksize];
    for d in -r..=r {
        spatial_w[(d + r) as usize] = fast_exp(-(d * d) as f32 * inv_r2);
    }

    let w_i32 = width as i32;
    let h_i32 = height as i32;
    let ru = r as usize;

    let neg_inf_v = f32x4_splat(f32::NEG_INFINITY);
    let zero_v = f32x4_splat(0.0);
    let one_v = f32x4_splat(1.0);
    let df_v = f32x4_splat(depth_factor);

    // --- Horizontal pass ---
    let mut h_buf = vec![1.0f32; n];
    // SIMD safe x range: x+dx..x+dx+3 in [0, width) for all dx in [-r, r]
    // → x >= r and x+r+3 < width → x < width-r-3
    let hx_simd_start = ru;
    let hx_simd_raw_end = if width > 2 * ru + 3 { width - ru - 3 } else { 0 };
    let hx_simd_end = if hx_simd_raw_end > hx_simd_start {
        hx_simd_start + ((hx_simd_raw_end - hx_simd_start) / 4) * 4
    } else { hx_simd_start };

    for y in 0..height {
        let row = y * width;
        let dp = unsafe { depth_buffer.as_ptr().add(row) };
        let ap = unsafe { ao_buffer.as_ptr().add(row) };
        let hp = unsafe { h_buf.as_mut_ptr().add(row) };

        // Left boundary: scalar
        for x in 0..hx_simd_start.min(width) {
            let cd = unsafe { *dp.add(x) };
            if cd == f32::NEG_INFINITY { continue; }
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dx in -r..=r {
                let sx = x as i32 + dx;
                if sx < 0 || sx >= w_i32 { continue; }
                let sd = unsafe { *dp.add(sx as usize) };
                if sd == f32::NEG_INFINITY { continue; }
                let dw = fast_exp(-(cd - sd).abs() * depth_factor);
                let w = dw * unsafe { *spatial_w.get_unchecked((dx + r) as usize) };
                sum += unsafe { *ap.add(sx as usize) } * w;
                wsum += w;
            }
            if wsum > 0.0 { unsafe { *hp.add(x) = sum / wsum; } }
            else { unsafe { *hp.add(x) = *ap.add(x); } }
        }

        // Interior: SIMD 4 pixels per iteration
        let mut x = hx_simd_start;
        while x < hx_simd_end {
            let cd4 = unsafe { v128_load(dp.add(x) as *const v128) };
            let valid = f32x4_ne(cd4, neg_inf_v);

            let mut sum4 = zero_v;
            let mut wsum4 = zero_v;
            for dx in -r..=r {
                let si = (x as i32 + dx) as usize;
                let sd4 = unsafe { v128_load(dp.add(si) as *const v128) };
                let sd_ok = f32x4_ne(sd4, neg_inf_v);
                let diff = f32x4_abs(f32x4_sub(cd4, sd4));
                let dw4 = fast_exp_4(f32x4_neg(f32x4_mul(diff, df_v)));
                let sw = f32x4_splat(unsafe { *spatial_w.get_unchecked((dx + r) as usize) });
                let w4 = v128_and(f32x4_mul(dw4, sw), v128_and(valid, sd_ok));
                let ao4 = unsafe { v128_load(ap.add(si) as *const v128) };
                sum4 = f32x4_add(sum4, f32x4_mul(ao4, w4));
                wsum4 = f32x4_add(wsum4, w4);
            }
            let result = f32x4_div(sum4, wsum4);
            let ao_orig = unsafe { v128_load(ap.add(x) as *const v128) };
            let result = v128_bitselect(result, ao_orig, f32x4_gt(wsum4, zero_v));
            let result = v128_bitselect(result, one_v, valid);
            unsafe { v128_store(hp.add(x) as *mut v128, result) };
            x += 4;
        }

        // Right boundary + remainder: scalar
        for x in hx_simd_end.max(hx_simd_start)..width {
            let cd = unsafe { *dp.add(x) };
            if cd == f32::NEG_INFINITY { continue; }
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dx in -r..=r {
                let sx = x as i32 + dx;
                if sx < 0 || sx >= w_i32 { continue; }
                let sd = unsafe { *dp.add(sx as usize) };
                if sd == f32::NEG_INFINITY { continue; }
                let dw = fast_exp(-(cd - sd).abs() * depth_factor);
                let w = dw * unsafe { *spatial_w.get_unchecked((dx + r) as usize) };
                sum += unsafe { *ap.add(sx as usize) } * w;
                wsum += w;
            }
            if wsum > 0.0 { unsafe { *hp.add(x) = sum / wsum; } }
            else { unsafe { *hp.add(x) = *ap.add(x); } }
        }
    }

    // --- Vertical pass ---
    let mut v_buf = vec![1.0f32; n];
    // SIMD x range: just need x+3 < width
    let vx_simd_end = (width / 4) * 4;

    // Top boundary rows: scalar
    for y in 0..ru.min(height) {
        for x in 0..width {
            let idx = y * width + x;
            let cd = unsafe { *depth_buffer.get_unchecked(idx) };
            if cd == f32::NEG_INFINITY { continue; }
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dy in -r..=r {
                let sy = y as i32 + dy;
                if sy < 0 || sy >= h_i32 { continue; }
                let si = sy as usize * width + x;
                let sd = unsafe { *depth_buffer.get_unchecked(si) };
                if sd == f32::NEG_INFINITY { continue; }
                let dw = fast_exp(-(cd - sd).abs() * depth_factor);
                let w = dw * unsafe { *spatial_w.get_unchecked((dy + r) as usize) };
                sum += unsafe { *h_buf.get_unchecked(si) } * w;
                wsum += w;
            }
            unsafe { *v_buf.get_unchecked_mut(idx) = if wsum > 0.0 { sum / wsum } else { *h_buf.get_unchecked(idx) } };
        }
    }

    // Interior rows: SIMD over x, all dy in [-r, r] valid
    let vy_end = if height > ru { height - ru } else { 0 };
    for y in ru..vy_end {
        let row = y * width;

        // SIMD: 4 consecutive x values
        let mut x = 0usize;
        while x < vx_simd_end {
            let idx = row + x;
            let cd4 = unsafe { v128_load(depth_buffer.as_ptr().add(idx) as *const v128) };
            let valid = f32x4_ne(cd4, neg_inf_v);

            let mut sum4 = zero_v;
            let mut wsum4 = zero_v;
            for dy in -r..=r {
                let si = (y as i32 + dy) as usize * width + x;
                let sd4 = unsafe { v128_load(depth_buffer.as_ptr().add(si) as *const v128) };
                let sd_ok = f32x4_ne(sd4, neg_inf_v);
                let diff = f32x4_abs(f32x4_sub(cd4, sd4));
                let dw4 = fast_exp_4(f32x4_neg(f32x4_mul(diff, df_v)));
                let sw = f32x4_splat(unsafe { *spatial_w.get_unchecked((dy + r) as usize) });
                let w4 = v128_and(f32x4_mul(dw4, sw), v128_and(valid, sd_ok));
                let ao4 = unsafe { v128_load(h_buf.as_ptr().add(si) as *const v128) };
                sum4 = f32x4_add(sum4, f32x4_mul(ao4, w4));
                wsum4 = f32x4_add(wsum4, w4);
            }
            let result = f32x4_div(sum4, wsum4);
            let h_orig = unsafe { v128_load(h_buf.as_ptr().add(idx) as *const v128) };
            let result = v128_bitselect(result, h_orig, f32x4_gt(wsum4, zero_v));
            let result = v128_bitselect(result, one_v, valid);
            unsafe { v128_store(v_buf.as_mut_ptr().add(idx) as *mut v128, result) };
            x += 4;
        }

        // Scalar remainder for x
        for x in vx_simd_end..width {
            let idx = row + x;
            let cd = unsafe { *depth_buffer.get_unchecked(idx) };
            if cd == f32::NEG_INFINITY { continue; }
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dy in -r..=r {
                let si = (y as i32 + dy) as usize * width + x;
                let sd = unsafe { *depth_buffer.get_unchecked(si) };
                if sd == f32::NEG_INFINITY { continue; }
                let dw = fast_exp(-(cd - sd).abs() * depth_factor);
                let w = dw * unsafe { *spatial_w.get_unchecked((dy + r) as usize) };
                sum += unsafe { *h_buf.get_unchecked(si) } * w;
                wsum += w;
            }
            unsafe { *v_buf.get_unchecked_mut(idx) = if wsum > 0.0 { sum / wsum } else { *h_buf.get_unchecked(idx) } };
        }
    }

    // Bottom boundary rows: scalar
    for y in vy_end..height {
        for x in 0..width {
            let idx = y * width + x;
            let cd = unsafe { *depth_buffer.get_unchecked(idx) };
            if cd == f32::NEG_INFINITY { continue; }
            let mut sum = 0.0f32;
            let mut wsum = 0.0f32;
            for dy in -r..=r {
                let sy = y as i32 + dy;
                if sy < 0 || sy >= h_i32 { continue; }
                let si = sy as usize * width + x;
                let sd = unsafe { *depth_buffer.get_unchecked(si) };
                if sd == f32::NEG_INFINITY { continue; }
                let dw = fast_exp(-(cd - sd).abs() * depth_factor);
                let w = dw * unsafe { *spatial_w.get_unchecked((dy + r) as usize) };
                sum += unsafe { *h_buf.get_unchecked(si) } * w;
                wsum += w;
            }
            unsafe { *v_buf.get_unchecked_mut(idx) = if wsum > 0.0 { sum / wsum } else { *h_buf.get_unchecked(idx) } };
        }
    }

    v_buf
}
