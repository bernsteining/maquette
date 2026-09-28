use crate::math::FloatExt;
/// Screen-Space Ambient Occlusion (SSAO) implementation.
/// Pre-computes integer sample offsets per noise pattern for fast per-pixel sampling.
/// Uses separable bilateral blur for noise reduction.

#[cfg(target_arch = "wasm32")] use std::arch::wasm32::*; #[cfg(not(target_arch = "wasm32"))] use crate::simd::*;

/// Parameters for SSAO computation.
#[derive(Clone)]
pub struct SSAOParams {
    pub samples: usize,
    /// Sampling radius: a fraction of the image's shorter side, or scene
    /// units when `camera` is set.
    pub radius: f64,
    /// Depth bias: a fraction of the frame's depth range, or scene units when
    /// `camera` is set.
    pub bias: f64,
    pub strength: f64,
    /// Sample a scene-space hemisphere around each pixel's reconstructed
    /// position instead of a fixed screen-space disc, so the effect does not
    /// change with image size or zoom. `strength` 1 then fully darkens a
    /// pixel whose hemisphere is half blocked.
    pub camera: Option<DepthCamera>,
}

impl SSAOParams {
    /// Parameters as configured by a renderer. With `scene = Some((camera,
    /// model_radius))`, sampling happens in scene space: `radius` defaults to
    /// 10% of `model_radius` and `bias` is a fraction of the radius. Without
    /// it, `radius` defaults to half the image's shorter side.
    pub fn new(samples: usize, radius: Option<f64>, bias: f64, strength: f64, scene: Option<(DepthCamera, f64)>) -> Self {
        match scene {
            Some((camera, model_radius)) => {
                let radius = radius.unwrap_or(0.1 * model_radius);
                Self { samples, radius, bias: bias * radius, strength, camera: Some(camera) }
            }
            None => Self { samples, radius: radius.unwrap_or(0.5), bias, strength, camera: None },
        }
    }
}

impl Default for SSAOParams {
    fn default() -> Self {
        Self {
            samples: 16,
            radius: 0.5,
            bias: 0.025,
            strength: 1.0,
            camera: None,
        }
    }
}

/// Maps view-space points (camera looking down −z, y up, depth buffer holding
/// view-space z) to continuous buffer pixel coordinates, whose pixel `(x, y)`
/// covers `[x, x+1) × [y, y+1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthCamera {
    /// Pinhole: `px = cx + x·fx/−z`, `py = cy − y·fy/−z`.
    Perspective { fx: f32, fy: f32, cx: f32, cy: f32 },
    /// Parallel: `px = cx + x·sx`, `py = cy − y·sy`.
    Ortho { sx: f32, sy: f32, cx: f32, cy: f32 },
}

impl DepthCamera {
    /// Scene-space height of one pixel at view depth `z`.
    #[inline(always)]
    pub fn pixel_size(&self, z: f32) -> f32 {
        match *self {
            DepthCamera::Perspective { fy, .. } => (-z).fmax(1e-6) / fy,
            DepthCamera::Ortho { sy, .. } => 1.0 / sy,
        }
    }

    /// View-space point at depth `z` under pixel coordinate `(px, py)`.
    #[inline(always)]
    pub fn unproject(&self, px: f32, py: f32, z: f32) -> [f32; 3] {
        match *self {
            DepthCamera::Perspective { fx, fy, cx, cy } => {
                let d = (-z).fmax(1e-6);
                [(px - cx) * d / fx, (cy - py) * d / fy, z]
            }
            DepthCamera::Ortho { sx, sy, cx, cy } => [(px - cx) / sx, (cy - py) / sy, z],
        }
    }

    /// Pixel coordinate of view-space point `p`.
    #[inline(always)]
    pub fn project(&self, p: [f32; 3]) -> (f32, f32) {
        match *self {
            DepthCamera::Perspective { fx, fy, cx, cy } => {
                let inv = 1.0 / (-p[2]).fmax(1e-6);
                (cx + p[0] * fx * inv, cy - p[1] * fy * inv)
            }
            DepthCamera::Ortho { sx, sy, cx, cy } => (cx + p[0] * sx, cy - p[1] * sy),
        }
    }

    /// The same camera on a buffer downsampled by `factor`.
    pub fn downscaled(self, factor: usize) -> Self {
        let k = 1.0 / factor.max(1) as f32;
        match self {
            DepthCamera::Perspective { fx, fy, cx, cy } => DepthCamera::Perspective { fx: fx * k, fy: fy * k, cx: cx * k, cy: cy * k },
            DepthCamera::Ortho { sx, sy, cx, cy } => DepthCamera::Ortho { sx: sx * k, sy: sy * k, cx: cx * k, cy: cy * k },
        }
    }
}

/// Normal-oriented hemisphere kernel for scene-space SSAO: unit-radius
/// points with `z ≥ 0`, denser near the origin.
pub fn hemisphere_kernel(samples: usize) -> Vec<[f32; 3]> {
    const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
    (0..samples.max(1)).map(|i| {
        let t = (i as f64 + 0.5) / samples.max(1) as f64;
        let cos_t = (1.0 - t).sqrt();
        let sin_t = (1.0 - cos_t * cos_t).fmax(0.0).sqrt();
        let a = i as f64 * GOLDEN_ANGLE;
        let u = ((i as u32).wrapping_mul(0x9E37_79B9) >> 8) as f64 / (1u32 << 24) as f64;
        let scale = 0.1 + 0.9 * u * u;
        [(a.cos() * sin_t * scale) as f32, (a.sin() * sin_t * scale) as f32, (cos_t * scale) as f32]
    }).collect()
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

/// `(cos, sin)` of rotation `n` of the pool: bit-reversed so that nearby
/// indices spread around the circle.
pub fn noise_rotation(n: usize) -> (f32, f32) {
    let br = (n as u32).reverse_bits() >> (32 - 8);
    let angle = (br as f64 / NOISE_ROTATIONS as f64) * std::f64::consts::TAU;
    let (sin_a, cos_a) = angle.sin_cos();
    (cos_a as f32, sin_a as f32)
}

/// Pre-compute all sample offsets for `NOISE_ROTATIONS × samples` — the
/// per-pixel loop then only does integer adds + depth compares.
pub fn precompute_sample_offsets(
    samples: usize,
    radius_px: f32,
    bias_scaled: f32,
) -> Vec<Vec<SampleOffset>> {
    const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653;
    let mut kernel = Vec::with_capacity(samples);
    for i in 0..samples {
        let angle = i as f64 * GOLDEN_ANGLE;
        let r = ((i + 1) as f64 / samples as f64).sqrt();
        let x = angle.cos() * r;
        let y = angle.sin() * r;
        let z = (1.0 - x * x - y * y).fmax(0.0).sqrt();
        let scale = (i as f64 / samples as f64).powi(2) * 0.9 + 0.1;
        kernel.push(((x * scale) as f32, (y * scale) as f32, (z * scale) as f32));
    }

    let mut offsets = Vec::with_capacity(NOISE_ROTATIONS);
    for n in 0..NOISE_ROTATIONS {
        let (cos_f, sin_f) = noise_rotation(n);

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
    let (zmin, zmax) = depth_bounds(depth_buffer);
    bilateral_blur_in_range(ao_buffer, depth_buffer, width, height, blur_radius, zmin, zmax)
}

/// Min and max of the finite (non −∞) depths; `(f32::MAX, f32::MIN)` if none.
pub fn depth_bounds(depth_buffer: &[f32]) -> (f32, f32) {
    let mut zmin = f32::MAX;
    let mut zmax = f32::MIN;
    for &d in depth_buffer {
        if d != f32::NEG_INFINITY {
            if d < zmin { zmin = d; }
            if d > zmax { zmax = d; }
        }
    }
    (zmin, zmax)
}

/// [`bilateral_blur_separable`] with the depth range supplied instead of
/// measured, so a block of rows blurs exactly as it would inside the frame
/// the range was taken from (away from the block's top and bottom `radius`
/// rows).
pub fn bilateral_blur_in_range(
    ao_buffer: &[f32],
    depth_buffer: &[f32],
    width: usize,
    height: usize,
    blur_radius: i32,
    zmin: f32,
    zmax: f32,
) -> Vec<f32> {
    let n = width * height;
    let r = blur_radius;
    let inv_depth_range = 1.0 / (zmax - zmin).fmax(0.001);
    let depth_factor = 50.0 * inv_depth_range;

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

    let mut h_buf = vec![1.0f32; n];
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

        let mut x = hx_simd_start;
        while x < hx_simd_end {
            let cd4 = unsafe { v128_load(dp.add(x) as *const v128) };
            let valid = f32x4_ne(cd4, neg_inf_v);
            if i32x4_bitmask(valid) == 0 {
                x += 4;
                continue;
            }

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

    let mut v_buf = vec![1.0f32; n];
    let vx_simd_end = (width / 4) * 4;

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

    let vy_end = if height > ru { height - ru } else { 0 };
    for y in ru..vy_end {
        let row = y * width;

        let mut x = 0usize;
        while x < vx_simd_end {
            let idx = row + x;
            let cd4 = unsafe { v128_load(depth_buffer.as_ptr().add(idx) as *const v128) };
            let valid = f32x4_ne(cd4, neg_inf_v);
            if i32x4_bitmask(valid) == 0 {
                x += 4;
                continue;
            }

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

