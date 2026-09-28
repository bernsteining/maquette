use super::*;
use crate::ssao::{noise_index, noise_rotation, precompute_sample_offsets, NOISE_ROTATIONS};
use std::cell::RefCell;

thread_local! {
    static AO_BUFFER:    RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static FLAT_OFFSETS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static Z_BIASES:     RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static KAWASE_SCRATCH: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

impl PixelBuffer {
    /// Apply screen-space outline detection on the depth buffer.
    /// Detects edges via depth discontinuities and normal-from-depth changes.
    /// Uses f32x4 SIMD for 4-neighbor depth checks (true 4-element parallelism).
    ///
    /// Without `scene`, a depth jump over 1.5% of the pixel's distance is an
    /// edge. With `scene = (camera, threshold)`, the jump is measured in
    /// scene-space pixel footprints instead (an edge when it exceeds
    /// `threshold` pixels' worth), and creases are found from true
    /// scene-space slopes, so outlines do not depend on distance, zoom or
    /// scene scale.
    pub fn apply_outline(&mut self, color: (u8, u8, u8), width: f64, scene: Option<(crate::ssao::DepthCamera, f32)>) {
        let w = self.width as i32;
        let h = self.height as i32;
        let step = (width * 0.5).fmax(1.0) as i32;
        let wu = w as usize;

        #[inline(always)]
        fn depth_raw(x: i32, y: i32, zbuf: &[f32], w: i32, h: i32) -> f32 {
            if x < 0 || x >= w || y < 0 || y >= h { return f32::NEG_INFINITY; }
            unsafe { *zbuf.get_unchecked(y as usize * w as usize + x as usize) }
        }

        #[inline(always)]
        fn is_normal_edge(r1: f32, l1: f32, d1: f32, u1: f32, r2: f32, l2: f32, d2: f32, u2: f32) -> bool {
            let dx1 = r1 - l1; let dy1 = d1 - u1;
            let dx2 = r2 - l2; let dy2 = d2 - u2;
            let raw_dot = dx1 * dx2 + dy1 * dy2 + 1.0;
            if raw_dot <= 0.0 { return true; }
            let len1_sq = dx1 * dx1 + dy1 * dy1 + 1.0;
            let len2_sq = dx2 * dx2 + dy2 * dy2 + 1.0;
            raw_dot * raw_dot < 0.36 * len1_sq * len2_sq
        }

        let n = (w * h) as usize;
        let mut edge_mask = vec![false; n];
        let neg_inf_v = f32x4_splat(f32::NEG_INFINITY);

        for y in 0..h {
            for x in 0..w {
                let idx = y as usize * wu + x as usize;
                let center = unsafe { *self.zbuf.get_unchecked(idx) };
                if center == f32::NEG_INFINITY { continue; }

                let dr = depth_raw(x + step, y, &self.zbuf, w, h);
                let dl = depth_raw(x - step, y, &self.zbuf, w, h);
                let dd = depth_raw(x, y + step, &self.zbuf, w, h);
                let du = depth_raw(x, y - step, &self.zbuf, w, h);
                let depths = f32x4(dr, dl, dd, du);

                let valid = f32x4_ne(depths, neg_inf_v);
                if i32x4_bitmask(valid) != 0xF {
                    edge_mask[idx] = true;
                    continue;
                }

                let (threshold, slope) = match scene {
                    Some((cam, t)) => {
                        let span = cam.pixel_size(center) * step as f32;
                        (t * span, 0.5 / span)
                    }
                    None => (0.015 * center.abs().fmax(0.001), 1.0),
                };
                let center_v = f32x4_splat(center);
                let abs_diff = f32x4_abs(f32x4_sub(depths, center_v));
                let within = f32x4_le(abs_diff, f32x4_splat(threshold));
                if i32x4_bitmask(within) != 0xF {
                    edge_mask[idx] = true;
                    continue;
                }

                for &(nx, ny, nd) in &[
                    (x + step, y, dr),
                    (x - step, y, dl),
                    (x, y + step, dd),
                    (x, y - step, du),
                ] {
                    let nr = depth_raw(nx + step, ny, &self.zbuf, w, h);
                    let nl = depth_raw(nx - step, ny, &self.zbuf, w, h);
                    let ndd = depth_raw(nx, ny + step, &self.zbuf, w, h);
                    let nu = depth_raw(nx, ny - step, &self.zbuf, w, h);
                    let nr = if nr == f32::NEG_INFINITY { nd } else { nr };
                    let nl = if nl == f32::NEG_INFINITY { nd } else { nl };
                    let ndd = if ndd == f32::NEG_INFINITY { nd } else { ndd };
                    let nu = if nu == f32::NEG_INFINITY { nd } else { nu };

                    if is_normal_edge(dr * slope, dl * slope, dd * slope, du * slope, nr * slope, nl * slope, ndd * slope, nu * slope) {
                        edge_mask[idx] = true;
                        break;
                    }
                }
            }
        }

        unsafe {
        for i in 0..n {
            if *edge_mask.get_unchecked(i) {
                let p = self.pixels.as_mut_ptr().add(i * 3);
                *p = color.0; *p.add(1) = color.1; *p.add(2) = color.2;
            }
        }
        }
    }

    /// Downsample by averaging NxN pixel blocks (for supersampling AA).
    /// Downsample by the given factor, averaging factor×factor source blocks.
    /// SIMD-accelerated for factor=2 and factor=4 (two passes of 2×).
    pub fn downsample(&self, factor: usize) -> Self {
        if factor <= 1 { return Self { width: self.width, height: self.height, pixels: self.pixels.clone(), zbuf: self.zbuf.clone(), ..Default::default() }; }
        if factor == 2 { return self.downsample_2x(); }
        if factor == 4 { return self.downsample_2x().downsample_2x(); }
        let nw = self.width / factor;
        let nh = self.height / factor;
        let count = (factor * factor) as u32;
        let half = count / 2;
        let src_w = self.width;
        let mut pixels = vec![0u8; nw * nh * 3];
        for ny in 0..nh {
            let src_y_base = ny * factor;
            for nx in 0..nw {
                let src_x_base = nx * factor;
                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                for sy in 0..factor {
                    let row_base = ((src_y_base + sy) * src_w + src_x_base) * 3;
                    for sx in 0..factor {
                        let si = row_base + sx * 3;
                        unsafe {
                            sum_r += *self.pixels.get_unchecked(si) as u32;
                            sum_g += *self.pixels.get_unchecked(si + 1) as u32;
                            sum_b += *self.pixels.get_unchecked(si + 2) as u32;
                        }
                    }
                }
                let di = (ny * nw + nx) * 3;
                unsafe {
                    *pixels.get_unchecked_mut(di) = ((sum_r + half) / count) as u8;
                    *pixels.get_unchecked_mut(di + 1) = ((sum_g + half) / count) as u8;
                    *pixels.get_unchecked_mut(di + 2) = ((sum_b + half) / count) as u8;
                }
            }
        }
        Self { width: nw, height: nh, pixels, ..Default::default() }
    }

    /// `downsample` that also keeps the nearest depth of each block, for
    /// `to_rgba8_transparent_by_depth`.
    pub fn downsample_with_depth(&self, factor: usize) -> Self {
        let mut out = self.downsample(factor);
        if factor <= 1 { return out; }
        out.zbuf = if factor == 2 {
            Self::nearest_depth_2x(&self.zbuf, self.width, out.width, out.height)
        } else if factor == 4 {
            let half = Self::nearest_depth_2x(&self.zbuf, self.width, self.width / 2, self.height / 2);
            Self::nearest_depth_2x(&half, self.width / 2, out.width, out.height)
        } else {
            let (nw, nh, src_w) = (out.width, out.height, self.width);
            let mut zbuf = vec![f32::NEG_INFINITY; nw * nh];
            for ny in 0..nh {
                for nx in 0..nw {
                    let mut z = f32::NEG_INFINITY;
                    for sy in 0..factor {
                        for sx in 0..factor {
                            let sz = self.zbuf[(ny * factor + sy) * src_w + nx * factor + sx];
                            if sz > z { z = sz; }
                        }
                    }
                    zbuf[ny * nw + nx] = z;
                }
            }
            zbuf
        };
        out
    }

    /// Nearest (largest) depth of each 2×2 block; −∞ (empty) never wins.
    fn nearest_depth_2x(src: &[f32], src_w: usize, nw: usize, nh: usize) -> Vec<f32> {
        let mut out = vec![f32::NEG_INFINITY; nw * nh];
        let sp = src.as_ptr();
        for ny in 0..nh {
            let (r0, r1) = (2 * ny * src_w, (2 * ny + 1) * src_w);
            let mut nx = 0;
            while nx + 4 <= nw && 2 * nx + 8 <= src_w {
                unsafe {
                    let ld = |i: usize| v128_load(sp.add(i) as *const v128);
                    let a = f32x4_max(ld(r0 + 2 * nx), ld(r1 + 2 * nx));
                    let b = f32x4_max(ld(r0 + 2 * nx + 4), ld(r1 + 2 * nx + 4));
                    let m = f32x4_max(i32x4_shuffle::<0, 2, 4, 6>(a, b), i32x4_shuffle::<1, 3, 5, 7>(a, b));
                    v128_store(out.as_mut_ptr().add(ny * nw + nx) as *mut v128, m);
                }
                nx += 4;
            }
            for nx in nx..nw {
                let (i0, i1) = (r0 + 2 * nx, r1 + 2 * nx);
                out[ny * nw + nx] = src[i0].fmax(src[i0 + 1]).fmax(src[i1]).fmax(src[i1 + 1]);
            }
        }
        out
    }

    /// SIMD 2× downsample: processes 4 output pixels per iteration using
    /// byte shuffles to deinterleave RGB, u16 widening for accumulation,
    /// and re-interleave for output. ~6× fewer instructions than scalar.
    fn downsample_2x(&self) -> Self {
        let nw = self.width / 2;
        let nh = self.height / 2;
        let src_w3 = self.width * 3;
        let src = &self.pixels;
        let mut out = vec![0u8; nw * nh * 3];

        unsafe {
            let half_v = i16x8_splat(2);

            for ny in 0..nh {
                let row0 = ny * 2 * src_w3;
                let row1 = row0 + src_w3;
                let mut nx = 0usize;

                while nx + 4 <= nw {
                    let sx = nx * 6;

                    let a0 = v128_load(src.as_ptr().add(row0 + sx) as *const v128);
                    let b0 = v128_load(src.as_ptr().add(row0 + sx + 8) as *const v128);
                    let a1 = v128_load(src.as_ptr().add(row1 + sx) as *const v128);
                    let b1 = v128_load(src.as_ptr().add(row1 + sx + 8) as *const v128);

                    let even0 = i8x16_shuffle::<
                        0, 6, 12, 26,  1, 7, 13, 27,  2, 8, 14, 28,  0, 0, 0, 0
                    >(a0, b0);
                    let odd0 = i8x16_shuffle::<
                        3, 9, 15, 29,  4, 10, 24, 30,  5, 11, 25, 31,  0, 0, 0, 0
                    >(a0, b0);

                    let sum0_lo = i16x8_add(
                        u16x8_extend_low_u8x16(even0), u16x8_extend_low_u8x16(odd0));
                    let sum0_hi = i16x8_add(
                        u16x8_extend_high_u8x16(even0), u16x8_extend_high_u8x16(odd0));

                    let even1 = i8x16_shuffle::<
                        0, 6, 12, 26,  1, 7, 13, 27,  2, 8, 14, 28,  0, 0, 0, 0
                    >(a1, b1);
                    let odd1 = i8x16_shuffle::<
                        3, 9, 15, 29,  4, 10, 24, 30,  5, 11, 25, 31,  0, 0, 0, 0
                    >(a1, b1);
                    let sum1_lo = i16x8_add(
                        u16x8_extend_low_u8x16(even1), u16x8_extend_low_u8x16(odd1));
                    let sum1_hi = i16x8_add(
                        u16x8_extend_high_u8x16(even1), u16x8_extend_high_u8x16(odd1));

                    let avg_lo = u16x8_shr(i16x8_add(
                        i16x8_add(sum0_lo, sum1_lo), half_v), 2);
                    let avg_hi = u16x8_shr(i16x8_add(
                        i16x8_add(sum0_hi, sum1_hi), half_v), 2);

                    let packed = u8x16_narrow_i16x8(avg_lo, avg_hi);

                    let rgb = i8x16_shuffle::<
                        0, 4, 8,  1, 5, 9,  2, 6, 10,  3, 7, 11,  0, 0, 0, 0
                    >(packed, packed);

                    let di = (ny * nw + nx) * 3;
                    let p = out.as_mut_ptr().add(di);
                    (p as *mut i32).write_unaligned(i32x4_extract_lane::<0>(rgb));
                    (p.add(4) as *mut i32).write_unaligned(i32x4_extract_lane::<1>(rgb));
                    (p.add(8) as *mut i32).write_unaligned(i32x4_extract_lane::<2>(rgb));

                    nx += 4;
                }

                while nx < nw {
                    let sx = nx * 2;
                    let r0 = (ny * 2 * self.width + sx) * 3;
                    let r1 = r0 + src_w3;
                    {
                        let sum_r = *src.get_unchecked(r0) as u32 + *src.get_unchecked(r0+3) as u32
                                  + *src.get_unchecked(r1) as u32 + *src.get_unchecked(r1+3) as u32;
                        let sum_g = *src.get_unchecked(r0+1) as u32 + *src.get_unchecked(r0+4) as u32
                                  + *src.get_unchecked(r1+1) as u32 + *src.get_unchecked(r1+4) as u32;
                        let sum_b = *src.get_unchecked(r0+2) as u32 + *src.get_unchecked(r0+5) as u32
                                  + *src.get_unchecked(r1+2) as u32 + *src.get_unchecked(r1+5) as u32;
                        let di = (ny * nw + nx) * 3;
                        *out.get_unchecked_mut(di) = ((sum_r + 2) / 4) as u8;
                        *out.get_unchecked_mut(di + 1) = ((sum_g + 2) / 4) as u8;
                        *out.get_unchecked_mut(di + 2) = ((sum_b + 2) / 4) as u8;
                    }
                    nx += 1;
                }
            }
        }

        Self { width: nw, height: nh, pixels: out, ..Default::default() }
    }

    /// Screen-Space Ambient Occlusion. Modulates the RGB pixel buffer by a
    /// per-pixel AO term derived from the depth buffer + bilateral blur.
    #[inline(never)]
    pub fn apply_ssao(&mut self, params: &crate::ssao::SSAOParams) {
        let (w, h) = (self.width, self.height);
        let mut ao_buffer = AO_BUFFER.with(|c| std::mem::take(&mut *c.borrow_mut()));
        ssao_occlusion(&self.zbuf, w, h, params, 0, h, &mut ao_buffer);
        let blurred = crate::ssao::bilateral_blur_separable(&ao_buffer, &self.zbuf, w, h, 4);
        self.modulate_rows(&blurred, 0);
        AO_BUFFER.with(|c| *c.borrow_mut() = ao_buffer);
    }

    /// [`apply_ssao`](Self::apply_ssao) for a buffer holding rows
    /// `y_off..y_off + height` of a `full_w × full_h` frame whose depth is
    /// `full_zbuf`: the result equals those rows of a whole-frame pass.
    #[inline(never)]
    pub fn apply_ssao_band(&mut self, params: &crate::ssao::SSAOParams, full_zbuf: &[f32], full_w: usize, full_h: usize, y_off: usize) {
        const BLUR: usize = 4;
        let (y0, y1) = (y_off.saturating_sub(BLUR), (y_off + self.height + BLUR).min(full_h));
        let mut ao_buffer = AO_BUFFER.with(|c| std::mem::take(&mut *c.borrow_mut()));
        ssao_occlusion(full_zbuf, full_w, full_h, params, y0, y1, &mut ao_buffer);
        let (zmin, zmax) = crate::ssao::depth_bounds(full_zbuf);
        let blurred = crate::ssao::bilateral_blur_in_range(&ao_buffer, &full_zbuf[y0 * full_w..y1 * full_w], full_w, y1 - y0, BLUR as i32, zmin, zmax);
        self.modulate_rows(&blurred, (y_off - y0) * full_w);
        AO_BUFFER.with(|c| *c.borrow_mut() = ao_buffer);
    }

    /// Rewrite every pixel's channels from their 0–255 values: `simd` gets
    /// four pixels at a time as R, G, B lanes starting at pixel `i`,
    /// `scalar` one pixel; results are truncated and saturated to `u8`.
    /// `keep(i)` may report that the four pixels from `i` stay unchanged.
    fn map_rgb(&mut self, keep: impl Fn(usize) -> bool, simd: impl Fn(usize, [v128; 3]) -> [v128; 3], scalar: impl Fn(usize, [f32; 3]) -> [f32; 3]) {
        let n = self.width * self.height;
        let pp = self.pixels.as_mut_ptr();
        let simd_end = if n >= 6 { ((n - 2) / 4) * 4 } else { 0 };
        let mut i = 0;
        while i < simd_end {
            if keep(i) { i += 4; continue; }
            unsafe {
                let raw = v128_load(pp.add(i * 3) as *const v128);
                let lane = |v: v128| f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(v)));
                let [r, g, b] = simd(i, [
                    lane(i8x16_shuffle::<0, 3, 6, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw)),
                    lane(i8x16_shuffle::<1, 4, 7, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw)),
                    lane(i8x16_shuffle::<2, 5, 8, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw)),
                ]);
                let (r, g, b) = (i32x4_trunc_sat_f32x4(r), i32x4_trunc_sat_f32x4(g), i32x4_trunc_sat_f32x4(b));
                let packed = u8x16_narrow_i16x8(u16x8_narrow_i32x4(r, g), u16x8_narrow_i32x4(b, b));
                let rgb = i8x16_shuffle::<0, 4, 8, 1, 5, 9, 2, 6, 10, 3, 7, 11, 0, 0, 0, 0>(packed, packed);
                let p = pp.add(i * 3);
                (p as *mut i32).write_unaligned(i32x4_extract_lane::<0>(rgb));
                (p.add(4) as *mut i32).write_unaligned(i32x4_extract_lane::<1>(rgb));
                (p.add(8) as *mut i32).write_unaligned(i32x4_extract_lane::<2>(rgb));
            }
            i += 4;
        }
        for i in simd_end..n {
            let px = &mut self.pixels[i * 3..i * 3 + 3];
            let out = scalar(i, [px[0] as f32, px[1] as f32, px[2] as f32]);
            for c in 0..3 { px[c] = out[c] as u8; }
        }
    }

    fn modulate_rows(&mut self, ao: &[f32], ao_start: usize) {
        let (ap, half, one) = (ao[ao_start..].as_ptr(), f32x4_splat(0.5), f32x4_splat(1.0));
        self.map_rgb(
            |i| i32x4_bitmask(f32x4_eq(unsafe { v128_load(ap.add(i) as *const v128) }, one)) == 0xF,
            |i, rgb| {
                let a = unsafe { v128_load(ap.add(i) as *const v128) };
                rgb.map(|c| f32x4_add(f32x4_mul(c, a), half))
            },
            |i, rgb| {
                let a = ao[ao_start + i];
                rgb.map(|c| c * a + 0.5)
            },
        );
    }

    /// Dual Kawase downsample of one colour plane: 5-tap filter, four
    /// destination pixels per SIMD step away from the borders.
    fn kawase_down_plane(src: &[f32], w: usize, h: usize, dst: &mut [f32], dw: usize, dh: usize) {
        let tap = |x: usize, y: usize| -> f32 {
            let (cx, cy) = (2 * x, 2 * y);
            let (xl, xr) = (cx.saturating_sub(1), (cx + 1).min(w - 1));
            let (yu, yd) = (cy.saturating_sub(1), (cy + 1).min(h - 1));
            let corners = (src[yu * w + xl] + src[yu * w + xr]) + (src[yd * w + xl] + src[yd * w + xr]);
            src[cy * w + cx] * 0.5 + corners * 0.125
        };
        let (half, eighth) = (f32x4_splat(0.5), f32x4_splat(0.125));
        let simd_end = if w >= 9 { ((w - 8) / 2 + 1).min(dw) } else { 0 };
        let sp = src.as_ptr();
        for y in 0..dh {
            let cy = 2 * y;
            if cy == 0 || cy + 1 >= h {
                for x in 0..dw { dst[y * dw + x] = tap(x, y); }
                continue;
            }
            let (ru, rc, rd) = ((cy - 1) * w, cy * w, (cy + 1) * w);
            dst[y * dw] = tap(0, y);
            let mut x = 1;
            while x + 4 <= simd_end {
                unsafe {
                    let lo = |row: usize, off: usize| v128_load(sp.add(row + 2 * x + off - 1) as *const v128);
                    let odd = |row: usize| (i32x4_shuffle::<0, 2, 4, 6>(lo(row, 0), lo(row, 4)), i32x4_shuffle::<0, 2, 4, 6>(lo(row, 2), lo(row, 6)));
                    let (tl, tr) = odd(ru);
                    let (bl, br) = odd(rd);
                    let c = i32x4_shuffle::<1, 3, 5, 7>(lo(rc, 0), lo(rc, 4));
                    let corners = f32x4_add(f32x4_add(tl, tr), f32x4_add(bl, br));
                    v128_store(dst.as_mut_ptr().add(y * dw + x) as *mut v128, f32x4_add(f32x4_mul(c, half), f32x4_mul(corners, eighth)));
                }
                x += 4;
            }
            for x in x..dw { dst[y * dw + x] = tap(x, y); }
        }
    }

    /// Dual Kawase upsample of one colour plane: every destination pixel
    /// reads the 9-tap neighbourhood of source pixel `(x/2, y/2)`, so the
    /// filter runs once per source pixel and each result fills its 2×2 block.
    fn kawase_up_plane(src: &[f32], sw: usize, sh: usize, dst: &mut [f32], dw: usize, dh: usize) {
        let (cross_w, diag_w, center_w) = (2.0f32 / 12.0, 1.0f32 / 12.0, 4.0f32 / 12.0);
        let tap = |sx: usize, sy: usize| -> f32 {
            let (xl, xr) = (sx.saturating_sub(1), (sx + 1).min(sw - 1));
            let (ru, rc, rd) = (sy.saturating_sub(1) * sw, sy * sw, (sy + 1).min(sh - 1) * sw);
            let cross = (src[rc + xl] + src[rc + xr]) + (src[ru + sx] + src[rd + sx]);
            let diag = (src[ru + xl] + src[ru + xr]) + (src[rd + xl] + src[rd + xr]);
            (cross * cross_w + diag * diag_w) + src[rc + sx] * center_w
        };
        let (cw, dgw, ctw) = (f32x4_splat(cross_w), f32x4_splat(diag_w), f32x4_splat(center_w));
        let mut blurred = KAWASE_SCRATCH.with(|c| std::mem::take(&mut *c.borrow_mut()));
        blurred.clear();
        blurred.resize(sw * sh + 8, 0.0);
        let sp = src.as_ptr();
        for sy in 0..sh {
            let (ru, rc, rd) = (sy.saturating_sub(1) * sw, sy * sw, (sy + 1).min(sh - 1) * sw);
            let out = &mut blurred[rc..];
            let mut sx = 0;
            while sx < 1.min(sw) { out[sx] = tap(sx, sy); sx += 1; }
            while sx + 5 <= sw {
                unsafe {
                    let ld = |i: usize| v128_load(sp.add(i) as *const v128);
                    let cross = f32x4_add(f32x4_add(ld(rc + sx - 1), ld(rc + sx + 1)), f32x4_add(ld(ru + sx), ld(rd + sx)));
                    let diag = f32x4_add(f32x4_add(ld(ru + sx - 1), ld(ru + sx + 1)), f32x4_add(ld(rd + sx - 1), ld(rd + sx + 1)));
                    let v = f32x4_add(f32x4_add(f32x4_mul(cross, cw), f32x4_mul(diag, dgw)), f32x4_mul(ld(rc + sx), ctw));
                    v128_store(out.as_mut_ptr().add(sx) as *mut v128, v);
                }
                sx += 4;
            }
            for sx in sx..sw { out[sx] = tap(sx, sy); }
        }
        let bp = blurred.as_ptr();
        for y in 0..dh {
            let row = (y / 2).min(sh - 1) * sw;
            let out = &mut dst[y * dw..(y + 1) * dw];
            let mut x = 0;
            while x + 8 <= dw && x / 2 + 4 <= sw {
                unsafe {
                    let v = v128_load(bp.add(row + x / 2) as *const v128);
                    v128_store(out.as_mut_ptr().add(x) as *mut v128, i32x4_shuffle::<0, 0, 1, 1>(v, v));
                    v128_store(out.as_mut_ptr().add(x + 4) as *mut v128, i32x4_shuffle::<2, 2, 3, 3>(v, v));
                }
                x += 8;
            }
            for x in x..dw { out[x] = blurred[row + (x / 2).min(sw - 1)]; }
        }
        KAWASE_SCRATCH.with(|c| *c.borrow_mut() = blurred);
    }

    /// Dual Kawase blur of a planar RGB buffer (three planes of `plane`
    /// floats) via a mip chain with ping-pong buffers.
    fn dual_kawase_blur(mut buf_a: Vec<f32>, plane: usize, w: usize, h: usize, radius: usize) -> Vec<f32> {
        let max_levels = ((w.min(h) as f32).log2() as usize).saturating_sub(1);
        let levels = ((radius + 3) / 4).max(1).min(max_levels).min(8);
        let mut buf_b = vec![0.0f32; buf_a.len()];
        let mut dims: Vec<(usize, usize)> = Vec::with_capacity(levels + 1);
        dims.push((w, h));
        let mut src_is_a = true;
        let pass = |from: &[f32], to: &mut [f32], f: &dyn Fn(&[f32], &mut [f32])| {
            for c in 0..3 { f(&from[c * plane..(c + 1) * plane], &mut to[c * plane..(c + 1) * plane]); }
        };

        for _ in 0..levels {
            let (pw, ph) = dims[dims.len() - 1];
            if pw < 4 || ph < 4 { break; }
            let (dw, dh) = (pw / 2, ph / 2);
            let down = |s: &[f32], d: &mut [f32]| Self::kawase_down_plane(s, pw, ph, d, dw, dh);
            if src_is_a { pass(&buf_a, &mut buf_b, &down); } else { pass(&buf_b, &mut buf_a, &down); }
            dims.push((dw, dh));
            src_is_a = !src_is_a;
        }

        for i in (0..dims.len() - 1).rev() {
            let ((cw, ch), (tw, th)) = (dims[i + 1], dims[i]);
            let up = |s: &[f32], d: &mut [f32]| Self::kawase_up_plane(s, cw, ch, d, tw, th);
            if src_is_a { pass(&buf_a, &mut buf_b, &up); } else { pass(&buf_b, &mut buf_a, &up); }
            src_is_a = !src_is_a;
        }

        if src_is_a { buf_a } else { buf_b }
    }

    /// Planar RGB scratch buffer for bloom and glow: three zeroed planes of
    /// `w·h` floats plus SIMD read slack; returns it with the plane length.
    fn rgb_planes(&self) -> (Vec<f32>, usize) {
        let plane = self.width * self.height + 8;
        (vec![0.0f32; plane * 3], plane)
    }

    /// Blur a planar RGB source and additively blend it onto the pixels.
    fn blur_and_blend(&mut self, source: Vec<f32>, plane: usize, intensity: f32, radius: usize) {
        let blurred = Self::dual_kawase_blur(source, plane, self.width, self.height, radius);
        let bp = blurred.as_ptr();
        let iv = f32x4_splat(intensity);
        self.map_rgb(
            |_| false,
            |i, rgb| {
                let mut c = 0;
                rgb.map(|v| {
                    let b = unsafe { v128_load(bp.add(c * plane + i) as *const v128) };
                    c += 1;
                    f32x4_add(v, f32x4_mul(b, iv))
                })
            },
            |i, rgb| {
                let mut c = 0;
                rgb.map(|v| {
                    let b = blurred[c * plane + i];
                    c += 1;
                    v + b * intensity
                })
            },
        );
    }

    /// Bloom: extract bright pixels by luminance threshold, blur, add back.
    pub fn apply_bloom(&mut self, threshold: f32, intensity: f32, radius: usize) {
        let n = self.width * self.height;
        let (mut buf, plane) = self.rgb_planes();

        let simd_end = if n >= 6 { ((n - 2) / 4) * 4 } else { 0 };
        let lum_r = f32x4_splat(0.2126);
        let lum_g = f32x4_splat(0.7152);
        let lum_b = f32x4_splat(0.0722);
        let thresh_v = f32x4_splat(threshold * 255.0);
        let inv255 = f32x4_splat(1.0 / 255.0);
        let thresh_s = f32x4_splat(threshold);
        let one_v = f32x4_splat(1.0);
        let pp = self.pixels.as_ptr();
        unsafe {
        let mut i = 0usize;
        while i < simd_end {
            let raw = v128_load(pp.add(i * 3) as *const v128);
            let rb = i8x16_shuffle::<0, 3, 6, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            let gb = i8x16_shuffle::<1, 4, 7, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            let bb = i8x16_shuffle::<2, 5, 8, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            let rf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(rb)));
            let gf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(gb)));
            let bf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(bb)));
            let lum4 = f32x4_add(f32x4_add(f32x4_mul(lum_r, rf), f32x4_mul(lum_g, gf)), f32x4_mul(lum_b, bf));
            let above = f32x4_gt(lum4, thresh_v);
            if i32x4_bitmask(above) != 0 {
                let factor = v128_and(f32x4_min(f32x4_sub(f32x4_mul(lum4, inv255), thresh_s), one_v), above);
                let bp = buf.as_mut_ptr().add(i);
                v128_store(bp as *mut v128, f32x4_mul(rf, factor));
                v128_store(bp.add(plane) as *mut v128, f32x4_mul(gf, factor));
                v128_store(bp.add(2 * plane) as *mut v128, f32x4_mul(bf, factor));
            }
            i += 4;
        }
        for i in simd_end..n {
            let p = pp.add(i * 3);
            let rf = *p as f32;
            let gf = *p.add(1) as f32;
            let bf = *p.add(2) as f32;
            let lum = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
            if lum > threshold * 255.0 {
                let factor = (lum / 255.0 - threshold).fmin(1.0);
                buf[i] = rf * factor;
                buf[plane + i] = gf * factor;
                buf[2 * plane + i] = bf * factor;
            }
        }
        }
        self.blur_and_blend(buf, plane, intensity, radius);
    }

    /// Glow: extract all foreground pixels (model silhouette), blur, add back.
    /// Creates a light-emitting aura around the entire model.
    pub fn apply_glow(&mut self, color: (u8, u8, u8), intensity: f32, radius: usize) {
        let (mut buf, plane) = self.rgb_planes();
        let rgb = [color.0 as f32, color.1 as f32, color.2 as f32];
        for (i, &z) in self.zbuf.iter().enumerate() {
            if z != f32::NEG_INFINITY {
                for (c, v) in rgb.iter().enumerate() { buf[c * plane + i] = *v; }
            }
        }
        self.blur_and_blend(buf, plane, intensity, radius);
    }

    /// Depth cueing: blends each covered pixel toward the fog colour by
    /// `smoothstep(near, far, −z)`, `z` being the view-space depth stored in
    /// the depth buffer.
    pub fn apply_fog(&mut self, fog: &crate::effects::Fog) {
        let (near, span) = (fog.near, (fog.far - fog.near).fmax(1e-12));
        let color = [fog.color.0 as f32, fog.color.1 as f32, fog.color.2 as f32];
        let (near_v, span_v, zero, one, two, three, half) =
            (f32x4_splat(near), f32x4_splat(span), f32x4_splat(0.0), f32x4_splat(1.0), f32x4_splat(2.0), f32x4_splat(3.0), f32x4_splat(0.5));
        let color_v = [f32x4_splat(color[0]), f32x4_splat(color[1]), f32x4_splat(color[2])];
        let zp = self.zbuf.as_ptr();
        let zbuf = std::mem::take(&mut self.zbuf);
        let empty = f32x4_splat(f32::NEG_INFINITY);
        self.map_rgb(
            |i| i32x4_bitmask(f32x4_eq(unsafe { v128_load(zp.add(i) as *const v128) }, empty)) == 0xF,
            |i, rgb| {
                let z = unsafe { v128_load(zp.add(i) as *const v128) };
                let t = f32x4_min(f32x4_max(f32x4_div(f32x4_sub(f32x4_neg(z), near_v), span_v), zero), one);
                let f = v128_and(f32x4_mul(f32x4_mul(t, t), f32x4_sub(three, f32x4_mul(two, t))), f32x4_ne(z, f32x4_splat(f32::NEG_INFINITY)));
                let mut c = 0;
                rgb.map(|p| {
                    let v = f32x4_add(f32x4_add(p, f32x4_mul(f32x4_sub(color_v[c], p), f)), half);
                    c += 1;
                    v
                })
            },
            |i, rgb| {
                let z = zbuf[i];
                if z == f32::NEG_INFINITY { return rgb; }
                let t = ((-z - near) / span).fmax(0.0).fmin(1.0);
                let f = t * t * (3.0 - 2.0 * t);
                let mut c = 0;
                rgb.map(|p| {
                    let v = p + (color[c] - p) * f + 0.5;
                    c += 1;
                    v
                })
            },
        );
        self.zbuf = zbuf;
    }

    /// Sharpen using a direct 3×3 kernel with ring buffer (3 rows instead of full clone).
    /// Kernel: center = 1 + 8s/9, neighbors = -s/9, where s = strength.
    pub fn apply_sharpen(&mut self, strength: f32) {
        let w = self.width;
        let h = self.height;
        if w < 3 || h < 3 { return; }
        let stride = w * 3;
        let neg = -strength * (1.0 / 9.0);
        let center = 1.0 + strength * (8.0 / 9.0);
        let (neg_v, center_v, zero_v, max_v) = (f32x4_splat(neg), f32x4_splat(center), f32x4_splat(0.0), f32x4_splat(255.0));

        let mut ring = vec![0u8; stride * 3];
        ring[..stride].copy_from_slice(&self.pixels[..stride]);
        ring[stride..stride * 2].copy_from_slice(&self.pixels[stride..stride * 2]);

        for y in 1..h - 1 {
            let ring_next = ((y + 1) % 3) * stride;
            let next_src = (y + 1) * stride;
            ring[ring_next..ring_next + stride].copy_from_slice(&self.pixels[next_src..next_src + stride]);

            let rp = ((y - 1) % 3) * stride;
            let rc = (y % 3) * stride;
            let rn = ring_next;
            let dst = y * stride;

            let rb = ring.as_ptr();
            let mut i = 3;
            while i + 19 <= stride {
                unsafe {
                    let ld = |r: usize, d: usize| v128_load(rb.add(r + i + d - 3) as *const v128);
                    let wide = |v: v128| (u16x8_extend_low_u8x16(v), u16x8_extend_high_u8x16(v));
                    let add = |a: (v128, v128), b: (v128, v128)| (i16x8_add(a.0, b.0), i16x8_add(a.1, b.1));
                    let mut sum = add(wide(ld(rp, 0)), wide(ld(rp, 3)));
                    for v in [ld(rp, 6), ld(rc, 0), ld(rc, 6), ld(rn, 0), ld(rn, 3), ld(rn, 6)] {
                        sum = add(sum, wide(v));
                    }
                    let c = wide(ld(rc, 3));
                    let quarter = |c16: v128, s16: v128, high: bool| {
                        let (c32, s32) = if high {
                            (u32x4_extend_high_u16x8(c16), u32x4_extend_high_u16x8(s16))
                        } else {
                            (u32x4_extend_low_u16x8(c16), u32x4_extend_low_u16x8(s16))
                        };
                        let v = f32x4_add(f32x4_mul(center_v, f32x4_convert_u32x4(c32)), f32x4_mul(neg_v, f32x4_convert_u32x4(s32)));
                        i32x4_trunc_sat_f32x4(f32x4_min(f32x4_max(v, zero_v), max_v))
                    };
                    let lo = u16x8_narrow_i32x4(quarter(c.0, sum.0, false), quarter(c.0, sum.0, true));
                    let hi = u16x8_narrow_i32x4(quarter(c.1, sum.1, false), quarter(c.1, sum.1, true));
                    v128_store(self.pixels.as_mut_ptr().add(dst + i) as *mut v128, u8x16_narrow_i16x8(lo, hi));
                }
                i += 16;
            }

            for x in i / 3..w - 1 {
                let xo = x * 3;
                for c in 0..3 {
                    if xo + c < i { continue; }
                    let sum_neighbors =
                        ring[rp + xo - 3 + c] as f32
                        + ring[rp + xo + c] as f32
                        + ring[rp + xo + 3 + c] as f32
                        + ring[rc + xo - 3 + c] as f32
                        + ring[rc + xo + 3 + c] as f32
                        + ring[rn + xo - 3 + c] as f32
                        + ring[rn + xo + c] as f32
                        + ring[rn + xo + 3 + c] as f32;
                    let v = center * ring[rc + xo + c] as f32 + neg * sum_neighbors;
                    self.pixels[dst + xo + c] = v.fmax(0.0).fmin(255.0) as u8;
                }
            }
        }
    }

    /// Encode the pixel buffer as PNG.
    /// Expand the opaque RGB buffer to straight RGBA8 (alpha = 255).
    /// Returns `(width, height, rgba)`. Backs the raw output path.
    pub fn to_rgba8(&self) -> (u32, u32, Vec<u8>) {
        let n = self.width * self.height;
        let src = &self.pixels[..n * 3];
        let mut rgba = vec![0u8; n * 4];
        let opaque = u8x16_splat(255);
        let mut i = 0;
        while i * 3 + 16 <= src.len() {
            unsafe {
                let px = v128_load(src.as_ptr().add(i * 3) as *const v128);
                let out = i8x16_shuffle::<0, 1, 2, 16, 3, 4, 5, 16, 6, 7, 8, 16, 9, 10, 11, 16>(px, opaque);
                v128_store(rgba.as_mut_ptr().add(i * 4) as *mut v128, out);
            }
            i += 4;
        }
        while i < n {
            rgba[i * 4] = src[i * 3];
            rgba[i * 4 + 1] = src[i * 3 + 1];
            rgba[i * 4 + 2] = src[i * 3 + 2];
            rgba[i * 4 + 3] = 255;
            i += 1;
        }
        (self.width as u32, self.height as u32, rgba)
    }

    /// Reduce a supersampled frame to output resolution, averaging each
    /// `factor`×`factor` block. With `transparent`, the z-buffer (and
    /// translucent coverage) acts as model coverage: colour is averaged over
    /// the covered subpixels only, with the background removed, and the
    /// covered fraction is returned as per-pixel alpha — so anti-aliased
    /// edges fade out with no background fringe. With `with_depth`, each
    /// output pixel keeps the nearest depth of its block.
    pub fn resolve(&self, factor: usize, transparent: bool, with_depth: bool) -> (Self, Option<Vec<u8>>) {
        if !transparent {
            let out = if with_depth { self.downsample_with_depth(factor) } else { self.downsample(factor) };
            return (out, None);
        }
        let f = factor.max(1);
        let (nw, nh) = (self.width / f, self.height / f);
        let count = (f * f) as f32;
        let [bgr, bgg, bgb] = self.bg;
        let mut out = Self { width: nw, height: nh, pixels: vec![0u8; nw * nh * 3], ..Default::default() };
        if with_depth { out.zbuf = vec![f32::NEG_INFINITY; nw * nh]; }
        let mut alpha = vec![0u8; nw * nh];
        for ny in 0..nh {
            for nx in 0..nw {
                let (mut pr, mut pg, mut pb, mut asum) = (0f32, 0f32, 0f32, 0f32);
                let mut z = f32::NEG_INFINITY;
                for sy in 0..f {
                    let row = (ny * f + sy) * self.width + nx * f;
                    for sx in 0..f {
                        let si = row + sx;
                        let sz = self.zbuf[si];
                        if sz != f32::NEG_INFINITY && sz > z { z = sz; }
                        let cov = if sz != f32::NEG_INFINITY { 1.0 } else { self.tcov.get(si).copied().unwrap_or(0.0) };
                        if cov > 0.0 {
                            let pi = si * 3;
                            let show = 1.0 - cov;
                            pr += self.pixels[pi] as f32 - show * bgr;
                            pg += self.pixels[pi + 1] as f32 - show * bgg;
                            pb += self.pixels[pi + 2] as f32 - show * bgb;
                            asum += cov;
                        }
                    }
                }
                let di = ny * nw + nx;
                if with_depth { out.zbuf[di] = z; }
                if asum > 0.0 {
                    out.pixels[di * 3] = (pr / asum).fround().clamp(0.0, 255.0) as u8;
                    out.pixels[di * 3 + 1] = (pg / asum).fround().clamp(0.0, 255.0) as u8;
                    out.pixels[di * 3 + 2] = (pb / asum).fround().clamp(0.0, 255.0) as u8;
                    alpha[di] = (asum / count * 255.0).fround().clamp(0.0, 255.0) as u8;
                }
            }
        }
        (out, Some(alpha))
    }

    /// Straight RGBA8 with the given per-pixel alpha, or opaque without one.
    pub fn to_rgba8_alpha(&self, alpha: Option<&[u8]>) -> (u32, u32, Vec<u8>) {
        let Some(alpha) = alpha else { return self.to_rgba8() };
        let mut rgba = vec![0u8; self.width * self.height * 4];
        for (i, &a) in alpha.iter().enumerate() {
            if a == 0 { continue; }
            rgba[i * 4..i * 4 + 3].copy_from_slice(&self.pixels[i * 3..i * 3 + 3]);
            rgba[i * 4 + 3] = a;
        }
        (self.width as u32, self.height as u32, rgba)
    }

    /// Apply `effects` to this output-resolution frame, in the order listed
    /// on [`PostEffects`](crate::effects::PostEffects).
    pub fn apply_post(&mut self, effects: &crate::effects::PostEffects) {
        if let Some(ssao) = &effects.ssao { self.apply_ssao(ssao); }
        if let Some(fog) = &effects.fog { self.apply_fog(fog); }
        if let Some(b) = &effects.bloom { self.apply_bloom(b.threshold, b.intensity, b.radius); }
        if let Some(g) = &effects.glow { self.apply_glow(g.color, g.intensity, g.radius); }
        if let Some(strength) = effects.sharpen { self.apply_sharpen(strength); }
        if effects.fxaa { crate::fxaa::apply_fxaa(&mut self.pixels, self.width, self.height); }
    }

    /// RGBA8 where pixels never touched by the rasterizer (zbuf = −∞) become
    /// fully transparent. Used when the config wants a transparent background.
    pub fn to_rgba8_transparent_by_depth(&self) -> (u32, u32, Vec<u8>) {
        let n = self.width * self.height;
        let mut rgba = vec![0u8; n * 4];
        for i in 0..n {
            if self.zbuf[i] != f32::NEG_INFINITY {
                rgba[i * 4]     = self.pixels[i * 3];
                rgba[i * 4 + 1] = self.pixels[i * 3 + 1];
                rgba[i * 4 + 2] = self.pixels[i * 3 + 2];
                rgba[i * 4 + 3] = 255;
            }
        }
        (self.width as u32, self.height as u32, rgba)
    }
}

fn ssao_occlusion(zbuf: &[f32], w: usize, h: usize, params: &crate::ssao::SSAOParams, y0: usize, y1: usize, ao_buffer: &mut Vec<f32>) {
    if let Some(cam) = params.camera {
        ssao_occlusion_scene(zbuf, w, h, params, &cam, y0, y1, ao_buffer);
        return;
    }
    let w_i32 = w as i32;
    let h_i32 = h as i32;

    let (zmin, zmax) = crate::ssao::depth_bounds(zbuf);
    let depth_range = (zmax - zmin).fmax(0.001);

    let radius_px = (params.radius * w.min(h) as f64) as f32;
    let bias_scaled = params.bias as f32 * depth_range;
    let strength = params.strength as f32;
    let offsets = precompute_sample_offsets(params.samples, radius_px, bias_scaled);

    let num_samples = offsets[0].len();
    let batches = num_samples / 4;
    let n_rot = NOISE_ROTATIONS;
    let mut flat_offsets = FLAT_OFFSETS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let mut z_biases     = Z_BIASES    .with(|c| std::mem::take(&mut *c.borrow_mut()));
    flat_offsets.clear(); flat_offsets.resize(n_rot * num_samples, 0);
    z_biases.clear();     z_biases.resize(n_rot * num_samples, 0.0);
    let mut max_dx = 0i32;
    let mut max_dy = 0i32;
    for (p, pattern) in offsets.iter().enumerate() {
        for (s, sample) in pattern.iter().enumerate() {
            flat_offsets[p * num_samples + s] = sample.dy * w_i32 + sample.dx;
            z_biases[p * num_samples + s] = sample.z_bias;
            max_dx = max_dx.max(sample.dx.abs());
            max_dy = max_dy.max(sample.dy.abs());
        }
    }

    let margin_x = max_dx;
    let margin_y = max_dy;
    let interior_x_end = (w_i32 - margin_x).max(margin_x);
    let interior_y_end = (h_i32 - margin_y).max(margin_y);
    let neg_inf_v = f32x4_splat(f32::NEG_INFINITY);
    let zbuf_ptr = zbuf.as_ptr();

    ao_buffer.clear();
    ao_buffer.resize(w * (y1 - y0), 1.0);
    let base = y0 * w;

    macro_rules! ssao_scalar_pixel {
        ($x:expr, $y:expr, $idx:expr) => {
            let depth = unsafe { *zbuf.get_unchecked($idx) };
            if depth != f32::NEG_INFINITY {
                let pattern = &offsets[noise_index($x, $y)];
                let mut occlusion = 0u32;
                let mut valid = 0u32;
                for s in pattern {
                    let sx = $x + s.dx;
                    let sy = $y + s.dy;
                    if sx < 0 || sx >= w_i32 || sy < 0 || sy >= h_i32 { continue; }
                    let sd = unsafe { *zbuf.get_unchecked(sy as usize * w + sx as usize) };
                    if sd == f32::NEG_INFINITY { continue; }
                    valid += 1;
                    if sd > depth + s.z_bias { occlusion += 1; }
                }
                if valid > 0 {
                    unsafe { *ao_buffer.get_unchecked_mut($idx - base) =
                        (1.0 - (occlusion as f32 / valid as f32 * strength).fmin(1.0)).fmax(0.0) };
                }
            }
        };
    }

    for y in y0 as i32..y1 as i32 {
        let row = y as usize * w;
        let is_interior_y = y >= margin_y && y < interior_y_end;

        if !is_interior_y {
            for x in 0..w_i32 {
                let idx = row + x as usize;
                ssao_scalar_pixel!(x, y, idx);
            }
        } else {
            for x in 0..margin_x.min(w_i32) {
                let idx = row + x as usize;
                ssao_scalar_pixel!(x, y, idx);
            }
            for x in margin_x..interior_x_end {
                let idx = row + x as usize;
                let depth = unsafe { *zbuf.get_unchecked(idx) };
                if depth == f32::NEG_INFINITY { continue; }
                let pi = noise_index(x, y);
                let offs = unsafe { flat_offsets.as_ptr().add(pi * num_samples) };
                let zbs = unsafe { z_biases.as_ptr().add(pi * num_samples) };
                let idx_i32 = idx as i32;
                let depth_v = f32x4_splat(depth);
                let mut valid = 0u32;
                let mut occluded = 0u32;
                for b in 0..batches {
                    let base = b * 4;
                    let o0 = unsafe { *offs.add(base) };
                    let o1 = unsafe { *offs.add(base + 1) };
                    let o2 = unsafe { *offs.add(base + 2) };
                    let o3 = unsafe { *offs.add(base + 3) };
                    let sd4 = f32x4(
                        unsafe { *zbuf_ptr.add((idx_i32 + o0) as usize) },
                        unsafe { *zbuf_ptr.add((idx_i32 + o1) as usize) },
                        unsafe { *zbuf_ptr.add((idx_i32 + o2) as usize) },
                        unsafe { *zbuf_ptr.add((idx_i32 + o3) as usize) },
                    );
                    let valid_mask = f32x4_ne(sd4, neg_inf_v);
                    let zb4 = unsafe { v128_load(zbs.add(base) as *const v128) };
                    let threshold = f32x4_add(depth_v, zb4);
                    let occ_mask = v128_and(f32x4_gt(sd4, threshold), valid_mask);
                    valid += i32x4_bitmask(valid_mask).count_ones();
                    occluded += i32x4_bitmask(occ_mask).count_ones();
                }
                for s in batches * 4..num_samples {
                    let sd = unsafe { *zbuf_ptr.add((idx_i32 + *offs.add(s)) as usize) };
                    if sd == f32::NEG_INFINITY { continue; }
                    valid += 1;
                    if sd > depth + unsafe { *zbs.add(s) } { occluded += 1; }
                }
                if valid > 0 {
                    unsafe { *ao_buffer.get_unchecked_mut(idx - base) =
                        (1.0 - (occluded as f32 / valid as f32 * strength).fmin(1.0)).fmax(0.0) };
                }
            }
            for x in interior_x_end..w_i32 {
                let idx = row + x as usize;
                ssao_scalar_pixel!(x, y, idx);
            }
        }
    }

    FLAT_OFFSETS.with(|c| *c.borrow_mut() = flat_offsets);
    Z_BIASES    .with(|c| *c.borrow_mut() = z_biases);
}

#[allow(clippy::too_many_arguments)]
fn ssao_occlusion_scene(zbuf: &[f32], w: usize, h: usize, params: &crate::ssao::SSAOParams, cam: &crate::ssao::DepthCamera, y0: usize, y1: usize, ao_buffer: &mut Vec<f32>) {
    let kernel = crate::ssao::hemisphere_kernel(params.samples);
    let rotations: Vec<(f32, f32)> = (0..NOISE_ROTATIONS).map(noise_rotation).collect();
    let radius = params.radius.fmax(1e-9) as f32;
    let bias = params.bias as f32;
    let strength = 2.0 * params.strength as f32;
    let (wi, hi) = (w as i32, h as i32);
    let depth_at = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= wi || y >= hi { f32::NEG_INFINITY } else { unsafe { *zbuf.get_unchecked(y as usize * w + x as usize) } }
    };
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let lanes: Vec<(v128, v128, v128)> = kernel.chunks_exact(4)
        .map(|c| (f32x4(c[0][0], c[1][0], c[2][0], c[3][0]), f32x4(c[0][1], c[1][1], c[2][1], c[3][1]), f32x4(c[0][2], c[1][2], c[2][2], c[3][2])))
        .collect();
    let (fx, fy, cx, cy, persp) = match *cam {
        crate::ssao::DepthCamera::Perspective { fx, fy, cx, cy } => (fx, fy, cx, cy, true),
        crate::ssao::DepthCamera::Ortho { sx, sy, cx, cy } => (sx, sy, cx, cy, false),
    };
    let (fx_v, fy_v, cx_v, cy_v) = (f32x4_splat(fx), f32x4_splat(fy), f32x4_splat(cx), f32x4_splat(cy));
    let (radius_v, bias_v, w_v, h_v) = (f32x4_splat(radius), f32x4_splat(bias), f32x4_splat(w as f32), f32x4_splat(h as f32));
    let (zero_v, one_v, two_v, three_v, eps_v, tiny_v) = (f32x4_splat(0.0), f32x4_splat(1.0), f32x4_splat(2.0), f32x4_splat(3.0), f32x4_splat(1e-6), f32x4_splat(1e-12));

    ao_buffer.clear();
    ao_buffer.resize(w * (y1 - y0), 1.0);
    for y in y0 as i32..y1 as i32 {
        for x in 0..wi {
            let z = depth_at(x, y);
            if z == f32::NEG_INFINITY { continue; }
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let p = cam.unproject(fx, fy, z);
            let side = |dx: i32, dy: i32| -> Option<[f32; 3]> {
                let (a, b) = (depth_at(x + dx, y + dy), depth_at(x - dx, y - dy));
                let pick = match (a != f32::NEG_INFINITY, b != f32::NEG_INFINITY) {
                    (true, true) => if (a - z).abs() <= (b - z).abs() { 1 } else { -1 },
                    (true, false) => 1,
                    (false, true) => -1,
                    (false, false) => return None,
                };
                let (sx, sy) = (dx * pick, dy * pick);
                let q = cam.unproject(fx + sx as f32, fy + sy as f32, depth_at(x + sx, y + sy));
                let d = sub(q, p);
                Some(if pick > 0 { d } else { [-d[0], -d[1], -d[2]] })
            };
            let mut n = match (side(1, 0), side(0, 1)) {
                (Some(t), Some(b)) => [b[1] * t[2] - b[2] * t[1], b[2] * t[0] - b[0] * t[2], b[0] * t[1] - b[1] * t[0]],
                _ => [0.0, 0.0, 1.0],
            };
            let toward = match cam {
                crate::ssao::DepthCamera::Perspective { .. } => [-p[0], -p[1], -p[2]],
                crate::ssao::DepthCamera::Ortho { .. } => [0.0, 0.0, 1.0],
            };
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len < 1e-12 { n = [0.0, 0.0, 1.0]; } else { n = [n[0] / len, n[1] / len, n[2] / len]; }
            if n[0] * toward[0] + n[1] * toward[1] + n[2] * toward[2] < 0.0 { n = [-n[0], -n[1], -n[2]]; }

            let (cos_a, sin_a) = rotations[noise_index(x, y)];
            let r = [cos_a, sin_a, 0.0];
            let rn = r[0] * n[0] + r[1] * n[1];
            let mut t = [r[0] - n[0] * rn, r[1] - n[1] * rn, -n[2] * rn];
            let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            if tl < 1e-6 { t = if n[0].abs() < 0.9 { [1.0 - n[0] * n[0], -n[0] * n[1], -n[0] * n[2]] } else { [-n[1] * n[0], 1.0 - n[1] * n[1], -n[1] * n[2]] }; }
            let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().fmax(1e-12);
            let t = [t[0] / tl, t[1] / tl, t[2] / tl];
            let b = [n[1] * t[2] - n[2] * t[1], n[2] * t[0] - n[0] * t[2], n[0] * t[1] - n[1] * t[0]];

            let mut occlusion = 0.0f32;
            let mut valid = 0u32;
            let splat3 = |a: [f32; 3]| [f32x4_splat(a[0]), f32x4_splat(a[1]), f32x4_splat(a[2])];
            let (tv, bv, nv, pv) = (splat3(t), splat3(b), splat3(n), splat3(p));
            for (kx, ky, kz) in &lanes {
                let axis = |a: usize| f32x4_add(pv[a], f32x4_mul(f32x4_add(f32x4_add(f32x4_mul(tv[a], *kx), f32x4_mul(bv[a], *ky)), f32x4_mul(nv[a], *kz)), radius_v));
                let (sx, sy, sz) = (axis(0), axis(1), axis(2));
                let inv = if persp { f32x4_div(one_v, f32x4_max(f32x4_neg(sz), eps_v)) } else { one_v };
                let px = f32x4_add(cx_v, f32x4_mul(f32x4_mul(sx, fx_v), inv));
                let py = f32x4_sub(cy_v, f32x4_mul(f32x4_mul(sy, fy_v), inv));
                let inside = v128_and(
                    v128_and(f32x4_ge(px, zero_v), f32x4_ge(py, zero_v)),
                    v128_and(f32x4_lt(px, w_v), f32x4_lt(py, h_v)),
                );
                let mask = i32x4_bitmask(inside);
                if mask == 0 { continue; }
                valid += mask.count_ones();
                let (ix, iy) = (i32x4_trunc_sat_f32x4(px), i32x4_trunc_sat_f32x4(py));
                let lane_depth = |bit: u8, xi: i32, yi: i32| if mask & bit != 0 { unsafe { *zbuf.get_unchecked(yi as usize * w + xi as usize) } } else { f32::NEG_INFINITY };
                let sd = f32x4(
                    lane_depth(1, i32x4_extract_lane::<0>(ix), i32x4_extract_lane::<0>(iy)),
                    lane_depth(2, i32x4_extract_lane::<1>(ix), i32x4_extract_lane::<1>(iy)),
                    lane_depth(4, i32x4_extract_lane::<2>(ix), i32x4_extract_lane::<2>(iy)),
                    lane_depth(8, i32x4_extract_lane::<3>(ix), i32x4_extract_lane::<3>(iy)),
                );
                let hit = i32x4_bitmask(f32x4_ge(sd, f32x4_add(sz, bias_v)));
                if hit == 0 { continue; }
                let range = f32x4_min(f32x4_div(radius_v, f32x4_max(f32x4_abs(f32x4_sub(f32x4_splat(z), sd)), tiny_v)), one_v);
                let weight = f32x4_mul(f32x4_mul(range, range), f32x4_sub(three_v, f32x4_mul(two_v, range)));
                if hit & 1 != 0 { occlusion += f32x4_extract_lane::<0>(weight); }
                if hit & 2 != 0 { occlusion += f32x4_extract_lane::<1>(weight); }
                if hit & 4 != 0 { occlusion += f32x4_extract_lane::<2>(weight); }
                if hit & 8 != 0 { occlusion += f32x4_extract_lane::<3>(weight); }
            }
            for k in &kernel[lanes.len() * 4..] {
                let s = [
                    p[0] + (t[0] * k[0] + b[0] * k[1] + n[0] * k[2]) * radius,
                    p[1] + (t[1] * k[0] + b[1] * k[1] + n[1] * k[2]) * radius,
                    p[2] + (t[2] * k[0] + b[2] * k[1] + n[2] * k[2]) * radius,
                ];
                let (sx, sy) = cam.project(s);
                if !(sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32) { continue; }
                valid += 1;
                let sd = depth_at(sx as i32, sy as i32);
                if sd >= s[2] + bias {
                    let range = (radius / (z - sd).abs().fmax(1e-12)).fmin(1.0);
                    occlusion += range * range * (3.0 - 2.0 * range);
                }
            }
            if valid > 0 {
                ao_buffer[(y as usize - y0) * w + x as usize] = (1.0 - (occlusion / valid as f32 * strength).fmin(1.0)).fmax(0.0);
            }
        }
    }
}
