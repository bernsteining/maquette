/// Software triangle rasterizer with z-buffer and PNG encoding.
// TODO: If Typst switches to a JIT WASM engine (e.g. Wasmtime), implement
// tile-based rasterization (bin triangles into 16x16 tiles) for L1 cache locality.

use std::arch::wasm32::*;

const HIZ_SHIFT: usize = 4; // 16×16 tiles
const HIZ_SIZE: usize = 1 << HIZ_SHIFT;

pub struct PixelBuffer {
    pub width: usize,
    pub height: usize,
    /// RGB pixel data (width * height * 3 bytes).
    pub pixels: Vec<u8>,
    /// Depth buffer (one f32 per pixel, initialized to -infinity).
    zbuf: Vec<f32>,
    /// Hi-Z: conservative lower bound on min zbuf per 16×16 tile.
    hiz: Vec<f32>,
    hiz_tiles_x: usize,
}

impl PixelBuffer {
    pub fn new(width: usize, height: usize, bg: (u8, u8, u8)) -> Self {
        let n = width * height;
        let pixel = [bg.0, bg.1, bg.2];
        let pixels = pixel.repeat(n);
        let hiz_tiles_x = (width + HIZ_SIZE - 1) >> HIZ_SHIFT;
        let hiz_tiles_y = (height + HIZ_SIZE - 1) >> HIZ_SHIFT;
        Self {
            width,
            height,
            pixels,
            zbuf: vec![f32::NEG_INFINITY; n],
            hiz: vec![f32::NEG_INFINITY; hiz_tiles_x * hiz_tiles_y],
            hiz_tiles_x,
        }
    }

    /// Check if a triangle can be skipped entirely via Hi-Z.
    /// Returns true if the triangle's closest point is behind all overlapping tiles.
    #[inline]
    pub fn hiz_can_skip(&self, pts: &[(f64, f64); 3], tri_max_depth: f32) -> bool {
        let w = (self.width - 1) as f64;
        let h = (self.height - 1) as f64;
        let min_tx = (pts[0].0.min(pts[1].0).min(pts[2].0).max(0.0) as usize) >> HIZ_SHIFT;
        let max_tx = (pts[0].0.max(pts[1].0).max(pts[2].0).min(w) as usize) >> HIZ_SHIFT;
        let min_ty = (pts[0].1.min(pts[1].1).min(pts[2].1).max(0.0) as usize) >> HIZ_SHIFT;
        let max_ty = (pts[0].1.max(pts[1].1).max(pts[2].1).min(h) as usize) >> HIZ_SHIFT;

        for ty in min_ty..=max_ty {
            let row = ty * self.hiz_tiles_x;
            for tx in min_tx..=max_tx {
                if tri_max_depth > unsafe { *self.hiz.get_unchecked(row + tx) } {
                    return false;
                }
            }
        }
        true
    }

    /// Update Hi-Z after rasterizing a triangle.
    /// Scans actual zbuf values for each overlapping tile. Only sets hiz once
    /// every pixel in the tile has been written (no -inf left). Front-to-back
    /// rendering ensures hiz never needs re-scanning after being set.
    #[inline]
    pub fn hiz_update(&mut self, pts: &[(f64, f64); 3]) {
        let w = (self.width - 1) as f64;
        let h = (self.height - 1) as f64;
        let min_tx = (pts[0].0.min(pts[1].0).min(pts[2].0).max(0.0) as usize) >> HIZ_SHIFT;
        let max_tx = (pts[0].0.max(pts[1].0).max(pts[2].0).min(w) as usize) >> HIZ_SHIFT;
        let min_ty = (pts[0].1.min(pts[1].1).min(pts[2].1).max(0.0) as usize) >> HIZ_SHIFT;
        let max_ty = (pts[0].1.max(pts[1].1).max(pts[2].1).min(h) as usize) >> HIZ_SHIFT;

        for ty in min_ty..=max_ty {
            let row = ty * self.hiz_tiles_x;
            for tx in min_tx..=max_tx {
                let idx = row + tx;
                // Already valid — front-to-back means hiz can only decrease, skip rescan
                if unsafe { *self.hiz.get_unchecked(idx) } != f32::NEG_INFINITY { continue; }
                // Scan tile pixels: if any pixel is still -inf, hiz stays -inf
                let px_start = tx << HIZ_SHIFT;
                let py_start = ty << HIZ_SHIFT;
                let px_end = (px_start + HIZ_SIZE).min(self.width);
                let py_end = (py_start + HIZ_SIZE).min(self.height);
                let mut tile_min = f32::INFINITY;
                let mut all_covered = true;
                'scan: for py in py_start..py_end {
                    let row_base = py * self.width;
                    for px in px_start..px_end {
                        let z = unsafe { *self.zbuf.get_unchecked(row_base + px) };
                        if z == f32::NEG_INFINITY { all_covered = false; break 'scan; }
                        if z < tile_min { tile_min = z; }
                    }
                }
                if all_covered {
                    unsafe { *self.hiz.get_unchecked_mut(idx) = tile_min; }
                }
            }
        }
    }

    /// Rasterize a filled triangle with z-buffer depth testing.
    /// Uses scanline clipping + f32x4 SIMD (4 pixels per iteration).
    pub fn rasterize_triangle(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        r: u8,
        g: u8,
        b: u8,
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &mut self.zbuf;
        let pixels = &mut self.pixels;

        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;

        unsafe {
            let d0v = f32x4_splat(d0);
            let d1v = f32x4_splat(d1);
            let d2v = f32x4_splat(d2);
            let zero = f32x4_splat(0.0);

            let mut row_w0 = setup.row_w0;
            let mut row_w1 = setup.row_w1;
            let mut row_w2 = setup.row_w2;

            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let w0_base = (row_w0 + offset * setup.dw0_dx) as f32;
                    let w1_base = (row_w1 + offset * setup.dw1_dx) as f32;
                    let w2_base = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;

                    let mut w0v = f32x4(w0_base, w0_base + dw0, w0_base + 2.0 * dw0, w0_base + 3.0 * dw0);
                    let mut w1v = f32x4(w1_base, w1_base + dw1, w1_base + 2.0 * dw1, w1_base + 3.0 * dw1);
                    let mut w2v = f32x4(w2_base, w2_base + dw2, w2_base + 2.0 * dw2, w2_base + 3.0 * dw2);
                    let dw0_dx4 = f32x4_splat(dw0 * 4.0);
                    let dw1_dx4 = f32x4_splat(dw1 * 4.0);
                    let dw2_dx4 = f32x4_splat(dw2 * 4.0);

                    let row_base = py * width;
                    let mut px = xl;

                    // SIMD loop: 4 pixels per iteration
                    while px + 3 <= xr {
                        let inside = v128_and(v128_and(
                            f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                        let in_mask = i32x4_bitmask(inside);

                        if in_mask != 0 {
                            let depth_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, d0v), f32x4_mul(w1v, d1v)), f32x4_mul(w2v, d2v));
                            let idx0 = row_base + px;
                            let zbuf_v = v128_load(zbuf.as_ptr().add(idx0) as *const v128);
                            let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
                            let wmask = i32x4_bitmask(pass);

                            if wmask & 1 != 0 {
                                *zbuf.get_unchecked_mut(idx0) = f32x4_extract_lane::<0>(depth_v);
                                let p = pixels.as_mut_ptr().add(idx0 * 3);
                                *p = r; *p.add(1) = g; *p.add(2) = b;
                            }
                            if wmask & 2 != 0 {
                                *zbuf.get_unchecked_mut(idx0 + 1) = f32x4_extract_lane::<1>(depth_v);
                                let p = pixels.as_mut_ptr().add((idx0 + 1) * 3);
                                *p = r; *p.add(1) = g; *p.add(2) = b;
                            }
                            if wmask & 4 != 0 {
                                *zbuf.get_unchecked_mut(idx0 + 2) = f32x4_extract_lane::<2>(depth_v);
                                let p = pixels.as_mut_ptr().add((idx0 + 2) * 3);
                                *p = r; *p.add(1) = g; *p.add(2) = b;
                            }
                            if wmask & 8 != 0 {
                                *zbuf.get_unchecked_mut(idx0 + 3) = f32x4_extract_lane::<3>(depth_v);
                                let p = pixels.as_mut_ptr().add((idx0 + 3) * 3);
                                *p = r; *p.add(1) = g; *p.add(2) = b;
                            }
                        }

                        w0v = f32x4_add(w0v, dw0_dx4);
                        w1v = f32x4_add(w1v, dw1_dx4);
                        w2v = f32x4_add(w2v, dw2_dx4);
                        px += 4;
                    }

                    // Scalar remainder
                    let mut w0s = f32x4_extract_lane::<0>(w0v);
                    let mut w1s = f32x4_extract_lane::<0>(w1v);
                    let mut w2s = f32x4_extract_lane::<0>(w2v);
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let idx = row_base + px;
                            if depth > *zbuf.get_unchecked(idx) {
                                *zbuf.get_unchecked_mut(idx) = depth;
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                *p = r; *p.add(1) = g; *p.add(2) = b;
                            }
                        }
                        w0s += dw0; w1s += dw1; w2s += dw2;
                        px += 1;
                    }
                }

                row_w0 += setup.dw0_dy;
                row_w1 += setup.dw1_dy;
                row_w2 += setup.dw2_dy;
            }
        }
    }

    /// Rasterize a triangle into a boolean shadow mask (no depth test).
    /// Uses scanline clipping + f32x4 SIMD (4 pixels per iteration).
    pub fn rasterize_shadow_mask(
        mask: &mut [bool],
        width: usize,
        height: usize,
        pts: &[(f64, f64); 3],
    ) {
        let setup = match TriSetup::new(pts, width, height) {
            Some(s) => s,
            None => return,
        };

        let zero = f32x4_splat(0.0);
        let mut row_w0 = setup.row_w0;
        let mut row_w1 = setup.row_w1;
        let mut row_w2 = setup.row_w2;

        for py in setup.min_y..=setup.max_y {
            if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                let offset = (xl - setup.min_x) as f64;
                let w0_base = (row_w0 + offset * setup.dw0_dx) as f32;
                let w1_base = (row_w1 + offset * setup.dw1_dx) as f32;
                let w2_base = (row_w2 + offset * setup.dw2_dx) as f32;
                let dw0 = setup.dw0_dx as f32;
                let dw1 = setup.dw1_dx as f32;
                let dw2 = setup.dw2_dx as f32;

                let mut w0v = f32x4(w0_base, w0_base + dw0, w0_base + 2.0 * dw0, w0_base + 3.0 * dw0);
                let mut w1v = f32x4(w1_base, w1_base + dw1, w1_base + 2.0 * dw1, w1_base + 3.0 * dw1);
                let mut w2v = f32x4(w2_base, w2_base + dw2, w2_base + 2.0 * dw2, w2_base + 3.0 * dw2);
                let dw0_dx4 = f32x4_splat(dw0 * 4.0);
                let dw1_dx4 = f32x4_splat(dw1 * 4.0);
                let dw2_dx4 = f32x4_splat(dw2 * 4.0);

                let row_base = py * width;
                let mut px = xl;

                unsafe {
                while px + 3 <= xr {
                    let inside = v128_and(v128_and(
                        f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                    let wmask = i32x4_bitmask(inside);
                    if wmask & 1 != 0 { *mask.get_unchecked_mut(row_base + px) = true; }
                    if wmask & 2 != 0 { *mask.get_unchecked_mut(row_base + px + 1) = true; }
                    if wmask & 4 != 0 { *mask.get_unchecked_mut(row_base + px + 2) = true; }
                    if wmask & 8 != 0 { *mask.get_unchecked_mut(row_base + px + 3) = true; }

                    w0v = f32x4_add(w0v, dw0_dx4);
                    w1v = f32x4_add(w1v, dw1_dx4);
                    w2v = f32x4_add(w2v, dw2_dx4);
                    px += 4;
                }

                let mut w0s = f32x4_extract_lane::<0>(w0v);
                let mut w1s = f32x4_extract_lane::<0>(w1v);
                let mut w2s = f32x4_extract_lane::<0>(w2v);
                while px <= xr {
                    if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                        *mask.get_unchecked_mut(row_base + px) = true;
                    }
                    w0s += dw0; w1s += dw1; w2s += dw2;
                    px += 1;
                }
                }
            }

            row_w0 += setup.dw0_dy;
            row_w1 += setup.dw1_dy;
            row_w2 += setup.dw2_dy;
        }
    }

    /// Apply shadow: blend shadow color with existing pixels where mask is set.
    // JIT-WASM SIMD: replace per-pixel loop body with f32x4 RGB blend:
    //   let existing = f32x4(pixels[pi], pixels[pi+1], pixels[pi+2], 0.0);
    //   let blended = f32x4_add(f32x4_mul(existing, inv_v), shadow_v);
    //   extract lanes back to u8 via i32x4_trunc_sat_f32x4(f32x4_nearest(blended))
    pub fn apply_shadow(&mut self, mask: &[bool], sr: u8, sg: u8, sb: u8, opacity: f64) {
        // Fixed-point blend: (existing * inv + shadow * 256) >> 8
        let inv = ((1.0 - opacity) * 256.0) as u32;
        let s_r = (sr as f64 * opacity * 256.0) as u32;
        let s_g = (sg as f64 * opacity * 256.0) as u32;
        let s_b = (sb as f64 * opacity * 256.0) as u32;
        let pixels = &mut self.pixels;
        unsafe {
        for i in 0..mask.len() {
            if *mask.get_unchecked(i) {
                let p = pixels.as_mut_ptr().add(i * 3);
                *p = ((*p as u32 * inv + s_r) >> 8) as u8;
                *p.add(1) = ((*p.add(1) as u32 * inv + s_g) >> 8) as u8;
                *p.add(2) = ((*p.add(2) as u32 * inv + s_b) >> 8) as u8;
            }
        }
        }
    }

    /// Rasterize a triangle with viewport offset (for grid mode).
    #[inline]
    pub fn rasterize_triangle_offset(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        r: u8,
        g: u8,
        b: u8,
        ox: f64,
        oy: f64,
    ) {
        let offset_pts = offset(pts, ox, oy);
        self.rasterize_triangle(&offset_pts, depths, r, g, b);
    }

    /// Rasterize into shadow mask with viewport offset.
    #[inline]
    pub fn rasterize_shadow_mask_offset(
        mask: &mut [bool],
        width: usize,
        height: usize,
        pts: &[(f64, f64); 3],
        ox: f64,
        oy: f64,
    ) {
        let offset_pts = offset(pts, ox, oy);
        Self::rasterize_shadow_mask(mask, width, height, &offset_pts);
    }

    /// Rasterize a triangle with per-vertex colors (Gouraud shading) and z-buffer.
    /// Uses scanline clipping + f32x4 SIMD (4 pixels per iteration).
    pub fn rasterize_triangle_smooth(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        colors: &[(u8, u8, u8); 3],
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &mut self.zbuf;
        let pixels = &mut self.pixels;

        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;

        unsafe {
            let d0v = f32x4_splat(d0);
            let d1v = f32x4_splat(d1);
            let d2v = f32x4_splat(d2);
            let zero = f32x4_splat(0.0);
            let c0 = f32x4(colors[0].0 as f32, colors[0].1 as f32, colors[0].2 as f32, 0.0);
            let c1 = f32x4(colors[1].0 as f32, colors[1].1 as f32, colors[1].2 as f32, 0.0);
            let c2 = f32x4(colors[2].0 as f32, colors[2].1 as f32, colors[2].2 as f32, 0.0);

            let mut row_w0 = setup.row_w0;
            let mut row_w1 = setup.row_w1;
            let mut row_w2 = setup.row_w2;

            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let w0_base = (row_w0 + offset * setup.dw0_dx) as f32;
                    let w1_base = (row_w1 + offset * setup.dw1_dx) as f32;
                    let w2_base = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;

                    let mut w0v = f32x4(w0_base, w0_base + dw0, w0_base + 2.0 * dw0, w0_base + 3.0 * dw0);
                    let mut w1v = f32x4(w1_base, w1_base + dw1, w1_base + 2.0 * dw1, w1_base + 3.0 * dw1);
                    let mut w2v = f32x4(w2_base, w2_base + dw2, w2_base + 2.0 * dw2, w2_base + 3.0 * dw2);
                    let dw0_dx4 = f32x4_splat(dw0 * 4.0);
                    let dw1_dx4 = f32x4_splat(dw1 * 4.0);
                    let dw2_dx4 = f32x4_splat(dw2 * 4.0);

                    let row_base = py * width;
                    let mut px = xl;

                    while px + 3 <= xr {
                        let inside = v128_and(v128_and(
                            f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                        let in_mask = i32x4_bitmask(inside);

                        if in_mask != 0 {
                            let depth_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, d0v), f32x4_mul(w1v, d1v)), f32x4_mul(w2v, d2v));
                            let idx0 = row_base + px;
                            let zbuf_v = v128_load(zbuf.as_ptr().add(idx0) as *const v128);
                            let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
                            let wmask = i32x4_bitmask(pass);

                            macro_rules! write_smooth {
                                ($lane:literal, $off:expr) => {
                                    if wmask & (1 << $lane) != 0 {
                                        let idx = idx0 + $off;
                                        *zbuf.get_unchecked_mut(idx) = f32x4_extract_lane::<$lane>(depth_v);
                                        let ws0 = f32x4_extract_lane::<$lane>(w0v);
                                        let ws1 = f32x4_extract_lane::<$lane>(w1v);
                                        let ws2 = f32x4_extract_lane::<$lane>(w2v);
                                        let rgb = f32x4_add(f32x4_add(
                                            f32x4_mul(f32x4_splat(ws0), c0),
                                            f32x4_mul(f32x4_splat(ws1), c1)),
                                            f32x4_mul(f32x4_splat(ws2), c2));
                                        let rgb = i32x4_trunc_sat_f32x4(f32x4_nearest(rgb));
                                        let p = pixels.as_mut_ptr().add(idx * 3);
                                        *p     = i32x4_extract_lane::<0>(rgb) as u8;
                                        *p.add(1) = i32x4_extract_lane::<1>(rgb) as u8;
                                        *p.add(2) = i32x4_extract_lane::<2>(rgb) as u8;
                                    }
                                }
                            }
                            write_smooth!(0, 0);
                            write_smooth!(1, 1);
                            write_smooth!(2, 2);
                            write_smooth!(3, 3);
                        }

                        w0v = f32x4_add(w0v, dw0_dx4);
                        w1v = f32x4_add(w1v, dw1_dx4);
                        w2v = f32x4_add(w2v, dw2_dx4);
                        px += 4;
                    }

                    // Scalar remainder
                    let mut w0s = f32x4_extract_lane::<0>(w0v);
                    let mut w1s = f32x4_extract_lane::<0>(w1v);
                    let mut w2s = f32x4_extract_lane::<0>(w2v);
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let idx = row_base + px;
                            if depth > *zbuf.get_unchecked(idx) {
                                *zbuf.get_unchecked_mut(idx) = depth;
                                let rgb = f32x4_add(f32x4_add(
                                    f32x4_mul(f32x4_splat(w0s), c0),
                                    f32x4_mul(f32x4_splat(w1s), c1)),
                                    f32x4_mul(f32x4_splat(w2s), c2));
                                let rgb = i32x4_trunc_sat_f32x4(f32x4_nearest(rgb));
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                *p     = i32x4_extract_lane::<0>(rgb) as u8;
                                *p.add(1) = i32x4_extract_lane::<1>(rgb) as u8;
                                *p.add(2) = i32x4_extract_lane::<2>(rgb) as u8;
                            }
                        }
                        w0s += dw0; w1s += dw1; w2s += dw2;
                        px += 1;
                    }
                }

                row_w0 += setup.dw0_dy;
                row_w1 += setup.dw1_dy;
                row_w2 += setup.dw2_dy;
            }
        }
    }

    /// Rasterize a transparent triangle: test z-buffer but don't write it, alpha-blend.
    /// Uses scanline clipping + f32x4 SIMD (4 pixels per iteration).
    pub fn rasterize_triangle_blend(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        r: u8,
        g: u8,
        b: u8,
        opacity: f64,
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &self.zbuf;
        let pixels = &mut self.pixels;
        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;

        unsafe {
            let d0v = f32x4_splat(d0);
            let d1v = f32x4_splat(d1);
            let d2v = f32x4_splat(d2);
            let zero = f32x4_splat(0.0);
            let inv = f32x4_splat((1.0 - opacity) as f32);
            let src = f32x4(r as f32 * opacity as f32, g as f32 * opacity as f32, b as f32 * opacity as f32, 0.0);

            let mut row_w0 = setup.row_w0;
            let mut row_w1 = setup.row_w1;
            let mut row_w2 = setup.row_w2;

            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let w0_base = (row_w0 + offset * setup.dw0_dx) as f32;
                    let w1_base = (row_w1 + offset * setup.dw1_dx) as f32;
                    let w2_base = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;

                    let mut w0v = f32x4(w0_base, w0_base + dw0, w0_base + 2.0 * dw0, w0_base + 3.0 * dw0);
                    let mut w1v = f32x4(w1_base, w1_base + dw1, w1_base + 2.0 * dw1, w1_base + 3.0 * dw1);
                    let mut w2v = f32x4(w2_base, w2_base + dw2, w2_base + 2.0 * dw2, w2_base + 3.0 * dw2);
                    let dw0_dx4 = f32x4_splat(dw0 * 4.0);
                    let dw1_dx4 = f32x4_splat(dw1 * 4.0);
                    let dw2_dx4 = f32x4_splat(dw2 * 4.0);

                    let row_base = py * width;
                    let mut px = xl;

                    while px + 3 <= xr {
                        let inside = v128_and(v128_and(
                            f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                        let in_mask = i32x4_bitmask(inside);

                        if in_mask != 0 {
                            let depth_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, d0v), f32x4_mul(w1v, d1v)), f32x4_mul(w2v, d2v));
                            let idx0 = row_base + px;
                            let zbuf_v = v128_load(zbuf.as_ptr().add(idx0) as *const v128);
                            let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
                            let wmask = i32x4_bitmask(pass);

                            macro_rules! blend_flat {
                                ($lane:literal, $off:expr) => {
                                    if wmask & (1 << $lane) != 0 {
                                        let p = pixels.as_mut_ptr().add((idx0 + $off) * 3);
                                        let existing = f32x4(*p as f32, *p.add(1) as f32, *p.add(2) as f32, 0.0);
                                        let blended = f32x4_add(f32x4_mul(existing, inv), src);
                                        let result = i32x4_trunc_sat_f32x4(f32x4_nearest(blended));
                                        *p = i32x4_extract_lane::<0>(result) as u8;
                                        *p.add(1) = i32x4_extract_lane::<1>(result) as u8;
                                        *p.add(2) = i32x4_extract_lane::<2>(result) as u8;
                                    }
                                }
                            }
                            blend_flat!(0, 0);
                            blend_flat!(1, 1);
                            blend_flat!(2, 2);
                            blend_flat!(3, 3);
                        }

                        w0v = f32x4_add(w0v, dw0_dx4);
                        w1v = f32x4_add(w1v, dw1_dx4);
                        w2v = f32x4_add(w2v, dw2_dx4);
                        px += 4;
                    }

                    let mut w0s = f32x4_extract_lane::<0>(w0v);
                    let mut w1s = f32x4_extract_lane::<0>(w1v);
                    let mut w2s = f32x4_extract_lane::<0>(w2v);
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let idx = row_base + px;
                            if depth > *zbuf.get_unchecked(idx) {
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                let existing = f32x4(*p as f32, *p.add(1) as f32, *p.add(2) as f32, 0.0);
                                let blended = f32x4_add(f32x4_mul(existing, inv), src);
                                let result = i32x4_trunc_sat_f32x4(f32x4_nearest(blended));
                                *p = i32x4_extract_lane::<0>(result) as u8;
                                *p.add(1) = i32x4_extract_lane::<1>(result) as u8;
                                *p.add(2) = i32x4_extract_lane::<2>(result) as u8;
                            }
                        }
                        w0s += dw0; w1s += dw1; w2s += dw2;
                        px += 1;
                    }
                }

                row_w0 += setup.dw0_dy;
                row_w1 += setup.dw1_dy;
                row_w2 += setup.dw2_dy;
            }
        }
    }

    /// Rasterize a transparent triangle with per-vertex colors (Gouraud), alpha-blend.
    /// Uses scanline clipping + f32x4 SIMD (4 pixels per iteration).
    pub fn rasterize_triangle_smooth_blend(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        colors: &[(u8, u8, u8); 3],
        opacity: f64,
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &self.zbuf;
        let pixels = &mut self.pixels;
        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;

        unsafe {
            let d0v = f32x4_splat(d0);
            let d1v = f32x4_splat(d1);
            let d2v = f32x4_splat(d2);
            let zero = f32x4_splat(0.0);
            let inv = f32x4_splat((1.0 - opacity) as f32);
            let opa = f32x4_splat(opacity as f32);
            let c0 = f32x4(colors[0].0 as f32, colors[0].1 as f32, colors[0].2 as f32, 0.0);
            let c1 = f32x4(colors[1].0 as f32, colors[1].1 as f32, colors[1].2 as f32, 0.0);
            let c2 = f32x4(colors[2].0 as f32, colors[2].1 as f32, colors[2].2 as f32, 0.0);

            let mut row_w0 = setup.row_w0;
            let mut row_w1 = setup.row_w1;
            let mut row_w2 = setup.row_w2;

            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let w0_base = (row_w0 + offset * setup.dw0_dx) as f32;
                    let w1_base = (row_w1 + offset * setup.dw1_dx) as f32;
                    let w2_base = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;

                    let mut w0v = f32x4(w0_base, w0_base + dw0, w0_base + 2.0 * dw0, w0_base + 3.0 * dw0);
                    let mut w1v = f32x4(w1_base, w1_base + dw1, w1_base + 2.0 * dw1, w1_base + 3.0 * dw1);
                    let mut w2v = f32x4(w2_base, w2_base + dw2, w2_base + 2.0 * dw2, w2_base + 3.0 * dw2);
                    let dw0_dx4 = f32x4_splat(dw0 * 4.0);
                    let dw1_dx4 = f32x4_splat(dw1 * 4.0);
                    let dw2_dx4 = f32x4_splat(dw2 * 4.0);

                    let row_base = py * width;
                    let mut px = xl;

                    while px + 3 <= xr {
                        let inside = v128_and(v128_and(
                            f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                        let in_mask = i32x4_bitmask(inside);

                        if in_mask != 0 {
                            let depth_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, d0v), f32x4_mul(w1v, d1v)), f32x4_mul(w2v, d2v));
                            let idx0 = row_base + px;
                            let zbuf_v = v128_load(zbuf.as_ptr().add(idx0) as *const v128);
                            let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
                            let wmask = i32x4_bitmask(pass);

                            macro_rules! blend_smooth {
                                ($lane:literal, $off:expr) => {
                                    if wmask & (1 << $lane) != 0 {
                                        let p = pixels.as_mut_ptr().add((idx0 + $off) * 3);
                                        let ws0 = f32x4_extract_lane::<$lane>(w0v);
                                        let ws1 = f32x4_extract_lane::<$lane>(w1v);
                                        let ws2 = f32x4_extract_lane::<$lane>(w2v);
                                        let interp = f32x4_add(f32x4_add(
                                            f32x4_mul(f32x4_splat(ws0), c0),
                                            f32x4_mul(f32x4_splat(ws1), c1)),
                                            f32x4_mul(f32x4_splat(ws2), c2));
                                        let existing = f32x4(*p as f32, *p.add(1) as f32, *p.add(2) as f32, 0.0);
                                        let blended = f32x4_add(f32x4_mul(existing, inv), f32x4_mul(interp, opa));
                                        let result = i32x4_trunc_sat_f32x4(f32x4_nearest(blended));
                                        *p = i32x4_extract_lane::<0>(result) as u8;
                                        *p.add(1) = i32x4_extract_lane::<1>(result) as u8;
                                        *p.add(2) = i32x4_extract_lane::<2>(result) as u8;
                                    }
                                }
                            }
                            blend_smooth!(0, 0);
                            blend_smooth!(1, 1);
                            blend_smooth!(2, 2);
                            blend_smooth!(3, 3);
                        }

                        w0v = f32x4_add(w0v, dw0_dx4);
                        w1v = f32x4_add(w1v, dw1_dx4);
                        w2v = f32x4_add(w2v, dw2_dx4);
                        px += 4;
                    }

                    let mut w0s = f32x4_extract_lane::<0>(w0v);
                    let mut w1s = f32x4_extract_lane::<0>(w1v);
                    let mut w2s = f32x4_extract_lane::<0>(w2v);
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let idx = row_base + px;
                            if depth > *zbuf.get_unchecked(idx) {
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                let interp = f32x4_add(f32x4_add(
                                    f32x4_mul(f32x4_splat(w0s), c0),
                                    f32x4_mul(f32x4_splat(w1s), c1)),
                                    f32x4_mul(f32x4_splat(w2s), c2));
                                let existing = f32x4(*p as f32, *p.add(1) as f32, *p.add(2) as f32, 0.0);
                                let blended = f32x4_add(f32x4_mul(existing, inv), f32x4_mul(interp, opa));
                                let result = i32x4_trunc_sat_f32x4(f32x4_nearest(blended));
                                *p = i32x4_extract_lane::<0>(result) as u8;
                                *p.add(1) = i32x4_extract_lane::<1>(result) as u8;
                                *p.add(2) = i32x4_extract_lane::<2>(result) as u8;
                            }
                        }
                        w0s += dw0; w1s += dw1; w2s += dw2;
                        px += 1;
                    }
                }

                row_w0 += setup.dw0_dy;
                row_w1 += setup.dw1_dy;
                row_w2 += setup.dw2_dy;
            }
        }
    }

    /// Draw triangle edges (no depth test, draws on top).
    #[inline]
    pub fn draw_triangle_edges(&mut self, pts: &[(f64, f64); 3], r: u8, g: u8, b: u8) {
        for e in 0..3 {
            let (x0, y0) = pts[e];
            let (x1, y1) = pts[(e + 1) % 3];
            self.draw_line(x0, y0, x1, y1, r, g, b);
        }
    }

    /// Draw a line using Bresenham's algorithm (no depth test, draws on top).
    pub fn draw_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: u8, g: u8, b: u8) {
        let w = self.width as i64;
        let h = self.height as i64;
        let mut ix0 = x0.round() as i64;
        let mut iy0 = y0.round() as i64;
        let ix1 = x1.round() as i64;
        let iy1 = y1.round() as i64;

        let dx = (ix1 - ix0).abs();
        let dy = -(iy1 - iy0).abs();
        let sx: i64 = if ix0 < ix1 { 1 } else { -1 };
        let sy: i64 = if iy0 < iy1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            if ix0 >= 0 && ix0 < w && iy0 >= 0 && iy0 < h {
                unsafe {
                    let p = self.pixels.as_mut_ptr().add((iy0 as usize * self.width + ix0 as usize) * 3);
                    *p = r; *p.add(1) = g; *p.add(2) = b;
                }
            }
            if ix0 == ix1 && iy0 == iy1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                ix0 += sx;
            }
            if e2 <= dx {
                err += dx;
                iy0 += sy;
            }
        }
    }

    /// Draw triangle edges with depth testing against the z-buffer.
    pub fn draw_triangle_edges_z(&mut self, pts: &[(f64, f64); 3], depths: &[f64; 3], r: u8, g: u8, b: u8) {
        for e in 0..3 {
            let n = (e + 1) % 3;
            self.draw_line_z(pts[e].0, pts[e].1, depths[e] as f32, pts[n].0, pts[n].1, depths[n] as f32, r, g, b);
        }
    }

    /// Draw a line with depth interpolation and z-buffer testing.
    fn draw_line_z(&mut self, x0: f64, y0: f64, z0: f32, x1: f64, y1: f64, z1: f32, r: u8, g: u8, b: u8) {
        let w = self.width as i64;
        let h = self.height as i64;
        let mut ix = x0.round() as i64;
        let mut iy = y0.round() as i64;
        let ix1 = x1.round() as i64;
        let iy1 = y1.round() as i64;

        let dx = (ix1 - ix).abs();
        let dy = -(iy1 - iy).abs();
        let sx: i64 = if ix < ix1 { 1 } else { -1 };
        let sy: i64 = if iy < iy1 { 1 } else { -1 };
        let mut err = dx + dy;
        let steps = dx.max(-dy) as f32;
        let inv_steps = if steps > 0.0 { 1.0 / steps } else { 0.0 };
        let mut step = 0f32;

        loop {
            if ix >= 0 && ix < w && iy >= 0 && iy < h {
                let t = step * inv_steps;
                let z = z0 + (z1 - z0) * t;
                let idx = iy as usize * self.width + ix as usize;
                if z >= self.zbuf[idx] {
                    unsafe {
                        let p = self.pixels.as_mut_ptr().add(idx * 3);
                        *p = r; *p.add(1) = g; *p.add(2) = b;
                    }
                }
            }
            if ix == ix1 && iy == iy1 { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; ix += sx; step += 1.0; }
            if e2 <= dx { err += dx; iy += sy; if e2 < dy { step += 1.0; } }
        }
    }

    /// Apply screen-space outline detection on the depth buffer.
    /// Detects edges via depth discontinuities and normal-from-depth changes.
    /// Uses f32x4 SIMD for 4-neighbor depth checks (true 4-element parallelism).
    pub fn apply_outline(&mut self, color: (u8, u8, u8), width: f64) {
        let w = self.width as i32;
        let h = self.height as i32;
        let step = (width * 0.5).max(1.0) as i32;
        let wu = w as usize;

        #[inline(always)]
        fn depth_raw(x: i32, y: i32, zbuf: &[f32], w: i32, h: i32) -> f32 {
            if x < 0 || x >= w || y < 0 || y >= h { return f32::NEG_INFINITY; }
            unsafe { *zbuf.get_unchecked(y as usize * w as usize + x as usize) }
        }

        // Compare normals without sqrt: dot < threshold iff dot² < threshold² * len1² * len2²
        // Normals are (-dx, -dy, 1) unnormalized, so dot = dx1*dx2 + dy1*dy2 + 1.
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

                // Load 4 neighbor depths into f32x4 [right, left, down, up]
                let dr = depth_raw(x + step, y, &self.zbuf, w, h);
                let dl = depth_raw(x - step, y, &self.zbuf, w, h);
                let dd = depth_raw(x, y + step, &self.zbuf, w, h);
                let du = depth_raw(x, y - step, &self.zbuf, w, h);
                let depths = f32x4(dr, dl, dd, du);

                // SIMD: check all 4 neighbors valid (not NEG_INFINITY)
                let valid = f32x4_ne(depths, neg_inf_v);
                if i32x4_bitmask(valid) != 0xF {
                    edge_mask[idx] = true;
                    continue;
                }

                // SIMD: depth discontinuity — all 4 |neighbor - center| <= threshold
                let threshold = 0.015 * center.abs().max(0.001);
                let center_v = f32x4_splat(center);
                let abs_diff = f32x4_abs(f32x4_sub(depths, center_v));
                let within = f32x4_le(abs_diff, f32x4_splat(threshold));
                if i32x4_bitmask(within) != 0xF {
                    edge_mask[idx] = true;
                    continue;
                }

                // Normal discontinuity — sqrt-free comparison
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

                    if is_normal_edge(dr, dl, dd, du, nr, nl, ndd, nu) {
                        edge_mask[idx] = true;
                        break;
                    }
                }
            }
        }

        // Write outline color where edges were detected
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
        if factor == 2 { return self.downsample_2x(); }
        if factor == 4 { return self.downsample_2x().downsample_2x(); }
        // Generic scalar path for other factors
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
        Self { width: nw, height: nh, pixels, zbuf: Vec::new(), hiz: Vec::new(), hiz_tiles_x: 0 }
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
            let half_v = i16x8_splat(2); // rounding: (sum + 2) >> 2

            for ny in 0..nh {
                let row0 = ny * 2 * src_w3;
                let row1 = row0 + src_w3;
                let mut nx = 0usize;

                // SIMD: 4 output pixels per iteration (reads 8 source pixels per row)
                while nx + 4 <= nw {
                    let sx = nx * 6; // 2 src pixels per out pixel × 3 bytes

                    // 2 overlapping 16-byte loads per row cover 24 bytes (8 pixels)
                    let a0 = v128_load(src.as_ptr().add(row0 + sx) as *const v128);
                    let b0 = v128_load(src.as_ptr().add(row0 + sx + 8) as *const v128);
                    let a1 = v128_load(src.as_ptr().add(row1 + sx) as *const v128);
                    let b1 = v128_load(src.as_ptr().add(row1 + sx + 8) as *const v128);

                    // Deinterleave even pixels (P0,P2,P4,P6) → [R R R R  G G G G  B B B B  _]
                    let even0 = i8x16_shuffle::<
                        0, 6, 12, 26,  1, 7, 13, 27,  2, 8, 14, 28,  0, 0, 0, 0
                    >(a0, b0);
                    // Deinterleave odd pixels (P1,P3,P5,P7)
                    let odd0 = i8x16_shuffle::<
                        3, 9, 15, 29,  4, 10, 24, 30,  5, 11, 25, 31,  0, 0, 0, 0
                    >(a0, b0);

                    // Widen u8→u16 and add even+odd pairs
                    let sum0_lo = i16x8_add(
                        u16x8_extend_low_u8x16(even0), u16x8_extend_low_u8x16(odd0));
                    let sum0_hi = i16x8_add(
                        u16x8_extend_high_u8x16(even0), u16x8_extend_high_u8x16(odd0));

                    // Row 1: same deinterleave + pair sum
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

                    // Combine rows, add rounding, divide by 4
                    let avg_lo = u16x8_shr(i16x8_add(
                        i16x8_add(sum0_lo, sum1_lo), half_v), 2);
                    let avg_hi = u16x8_shr(i16x8_add(
                        i16x8_add(sum0_hi, sum1_hi), half_v), 2);

                    // Narrow u16→u8: [R0 R1 R2 R3 G0 G1 G2 G3 | B0 B1 B2 B3 _ _ _ _]
                    let packed = u8x16_narrow_i16x8(avg_lo, avg_hi);

                    // Re-interleave to RGB output order
                    let rgb = i8x16_shuffle::<
                        0, 4, 8,  1, 5, 9,  2, 6, 10,  3, 7, 11,  0, 0, 0, 0
                    >(packed, packed);

                    // Store 12 bytes (4 output pixels) via 3 u32 writes
                    let di = (ny * nw + nx) * 3;
                    let p = out.as_mut_ptr().add(di);
                    (p as *mut i32).write_unaligned(i32x4_extract_lane::<0>(rgb));
                    (p.add(4) as *mut i32).write_unaligned(i32x4_extract_lane::<1>(rgb));
                    (p.add(8) as *mut i32).write_unaligned(i32x4_extract_lane::<2>(rgb));

                    nx += 4;
                }

                // Scalar remainder (0-3 pixels)
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

        Self { width: nw, height: nh, pixels: out, zbuf: Vec::new(), hiz: Vec::new(), hiz_tiles_x: 0 }
    }

    /// Apply Screen-Space Ambient Occlusion (SSAO) to the rendered image.
    /// Darkens pixels based on depth buffer occlusion.
    pub fn apply_ssao(&mut self, params: &crate::ssao::SSAOParams) {
        let w = self.width;
        let h = self.height;
        let w_i32 = w as i32;
        let h_i32 = h as i32;

        // Compute depth range for scaling bias relative to scene
        let mut zmin = f32::MAX;
        let mut zmax = f32::MIN;
        for &d in &self.zbuf {
            if d != f32::NEG_INFINITY {
                if d < zmin { zmin = d; }
                if d > zmax { zmax = d; }
            }
        }
        let depth_range = (zmax - zmin).max(0.001);

        // Pre-compute all sample offsets (16 noise patterns x N samples)
        let radius_px = (params.radius * w.min(h) as f64) as f32;
        let bias_scaled = params.bias as f32 * depth_range;
        let strength = params.strength as f32;
        let offsets = crate::ssao::precompute_sample_offsets(params.samples, radius_px, bias_scaled);

        // Precompute flat offsets (dy*w+dx) and z_biases per pattern for fast sampling
        let num_samples = offsets[0].len();
        let batches = num_samples / 4;
        let mut flat_offsets = vec![0i32; 16 * num_samples];
        let mut z_biases = vec![0.0f32; 16 * num_samples];
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

        // Interior zone: pixels where all sample offsets are guaranteed in-bounds
        let margin_x = max_dx;
        let margin_y = max_dy;
        let interior_x_end = (w_i32 - margin_x).max(margin_x);
        let interior_y_end = (h_i32 - margin_y).max(margin_y);
        let neg_inf_v = f32x4_splat(f32::NEG_INFINITY);
        let zbuf_ptr = self.zbuf.as_ptr();

        // Compute SSAO per pixel
        let mut ao_buffer = vec![1.0_f32; w * h];

        // Scalar border pixel helper (with bounds checks)
        macro_rules! ssao_scalar_pixel {
            ($x:expr, $y:expr, $idx:expr) => {
                let depth = unsafe { *self.zbuf.get_unchecked($idx) };
                if depth != f32::NEG_INFINITY {
                    let pattern = &offsets[(($y & 3) * 4 + ($x & 3)) as usize];
                    let mut occlusion = 0u32;
                    let mut valid = 0u32;
                    for s in pattern {
                        let sx = $x + s.dx;
                        let sy = $y + s.dy;
                        if sx < 0 || sx >= w_i32 || sy < 0 || sy >= h_i32 { continue; }
                        let sd = unsafe { *self.zbuf.get_unchecked(sy as usize * w + sx as usize) };
                        if sd == f32::NEG_INFINITY { continue; }
                        valid += 1;
                        if sd > depth + s.z_bias { occlusion += 1; }
                    }
                    if valid > 0 {
                        unsafe { *ao_buffer.get_unchecked_mut($idx) =
                            (1.0 - (occlusion as f32 / valid as f32 * strength).min(1.0)).max(0.0) };
                    }
                }
            };
        }

        for y in 0..h_i32 {
            let row = y as usize * w;
            let is_interior_y = y >= margin_y && y < interior_y_end;

            if !is_interior_y {
                // Full border row: scalar with bounds checks
                for x in 0..w_i32 {
                    let idx = row + x as usize;
                    ssao_scalar_pixel!(x, y, idx);
                }
            } else {
                // Left border
                for x in 0..margin_x.min(w_i32) {
                    let idx = row + x as usize;
                    ssao_scalar_pixel!(x, y, idx);
                }

                // Interior: SIMD 4 samples per batch, no bounds checks
                for x in margin_x..interior_x_end {
                    let idx = row + x as usize;
                    let depth = unsafe { *self.zbuf.get_unchecked(idx) };
                    if depth == f32::NEG_INFINITY { continue; }

                    let pi = ((y & 3) * 4 + (x & 3)) as usize;
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

                    // Scalar remainder
                    for s in batches * 4..num_samples {
                        let sd = unsafe { *zbuf_ptr.add((idx_i32 + *offs.add(s)) as usize) };
                        if sd == f32::NEG_INFINITY { continue; }
                        valid += 1;
                        if sd > depth + unsafe { *zbs.add(s) } { occluded += 1; }
                    }

                    if valid > 0 {
                        unsafe { *ao_buffer.get_unchecked_mut(idx) =
                            (1.0 - (occluded as f32 / valid as f32 * strength).min(1.0)).max(0.0) };
                    }
                }

                // Right border
                for x in interior_x_end..w_i32 {
                    let idx = row + x as usize;
                    ssao_scalar_pixel!(x, y, idx);
                }
            }
        }

        // Separable bilateral blur (horizontal + vertical, O(2r) instead of O(r^2))
        let ao_buffer = crate::ssao::bilateral_blur_separable(&ao_buffer, &self.zbuf, w, h, 4);

        // Apply AO by darkening pixels: pixel = pixel * ao
        // JIT-WASM SIMD: f32x4_mul(f32x4(r, g, b, 0), f32x4_splat(ao))
        //   then i32x4_trunc_sat_f32x4(f32x4_nearest(...)) to extract u8.
        unsafe {
        for i in 0..w * h {
            let ao = *ao_buffer.get_unchecked(i);
            let p = self.pixels.as_mut_ptr().add(i * 3);
            *p = (*p as f32 * ao + 0.5) as u8;
            *p.add(1) = (*p.add(1) as f32 * ao + 0.5) as u8;
            *p.add(2) = (*p.add(2) as f32 * ao + 0.5) as u8;
        }
        }
    }

    /// Dual Kawase downsample: 5-tap filter into pre-allocated dst slice.
    fn kawase_down_into(src: &[f32], w: usize, h: usize, dst: &mut [f32], dw: usize, dh: usize) {
        let w_i = w as i32;
        let h_i = h as i32;
        let half = f32x4_splat(0.5);
        let eighth = f32x4_splat(0.125);
        let sp = src.as_ptr();
        for y in 0..dh {
            for x in 0..dw {
                let cx = (x * 2) as i32;
                let cy = (y * 2) as i32;
                let di = (y * dw + x) * 3;
                let ci = (cy as usize * w + cx as usize) * 3;
                let tli = ((cy - 1).max(0) as usize * w + (cx - 1).max(0) as usize) * 3;
                let tri = ((cy - 1).max(0) as usize * w + (cx + 1).min(w_i - 1) as usize) * 3;
                let bli = ((cy + 1).min(h_i - 1) as usize * w + (cx - 1).max(0) as usize) * 3;
                let bri = ((cy + 1).min(h_i - 1) as usize * w + (cx + 1).min(w_i - 1) as usize) * 3;
                unsafe {
                    // v128_load reads 4 f32 [R,G,B,X]; 4th lane is garbage but not stored
                    let cv = v128_load(sp.add(ci) as *const v128);
                    let corners = f32x4_add(
                        f32x4_add(v128_load(sp.add(tli) as *const v128), v128_load(sp.add(tri) as *const v128)),
                        f32x4_add(v128_load(sp.add(bli) as *const v128), v128_load(sp.add(bri) as *const v128)),
                    );
                    let result = f32x4_add(f32x4_mul(cv, half), f32x4_mul(corners, eighth));
                    let dp = dst.as_mut_ptr().add(di);
                    *dp = f32x4_extract_lane::<0>(result);
                    *dp.add(1) = f32x4_extract_lane::<1>(result);
                    *dp.add(2) = f32x4_extract_lane::<2>(result);
                }
            }
        }
    }

    /// Dual Kawase upsample: 9-tap filter into pre-allocated dst slice.
    fn kawase_up_into(src: &[f32], sw: usize, sh: usize, dst: &mut [f32], dw: usize, dh: usize) {
        let sw_i = sw as i32;
        let sh_i = sh as i32;
        let cross_w = f32x4_splat(2.0 / 12.0);
        let diag_w = f32x4_splat(1.0 / 12.0);
        let center_w = f32x4_splat(4.0 / 12.0);
        let sp = src.as_ptr();
        for y in 0..dh {
            let sy = ((y as i32) / 2).min(sh_i - 1);
            let row_u = (sy - 1).max(0) as usize * sw;
            let row_c = sy as usize * sw;
            let row_d = (sy + 1).min(sh_i - 1) as usize * sw;
            for x in 0..dw {
                let sx = ((x as i32) / 2).min(sw_i - 1);
                let di = (y * dw + x) * 3;
                let xl = (sx - 1).max(0) as usize;
                let xr = (sx + 1).min(sw_i - 1) as usize;
                let sxu = sx as usize;
                let ci = row_c * 3 + sxu * 3;
                let li = row_c * 3 + xl * 3;
                let ri = row_c * 3 + xr * 3;
                let ui = row_u * 3 + sxu * 3;
                let ddi = row_d * 3 + sxu * 3;
                let tli = row_u * 3 + xl * 3;
                let tri = row_u * 3 + xr * 3;
                let bli = row_d * 3 + xl * 3;
                let bri = row_d * 3 + xr * 3;
                unsafe {
                    let cross = f32x4_add(
                        f32x4_add(v128_load(sp.add(li) as *const v128), v128_load(sp.add(ri) as *const v128)),
                        f32x4_add(v128_load(sp.add(ui) as *const v128), v128_load(sp.add(ddi) as *const v128)),
                    );
                    let diag = f32x4_add(
                        f32x4_add(v128_load(sp.add(tli) as *const v128), v128_load(sp.add(tri) as *const v128)),
                        f32x4_add(v128_load(sp.add(bli) as *const v128), v128_load(sp.add(bri) as *const v128)),
                    );
                    let result = f32x4_add(f32x4_add(
                        f32x4_mul(cross, cross_w), f32x4_mul(diag, diag_w)),
                        f32x4_mul(v128_load(sp.add(ci) as *const v128), center_w),
                    );
                    let dp = dst.as_mut_ptr().add(di);
                    *dp = f32x4_extract_lane::<0>(result);
                    *dp.add(1) = f32x4_extract_lane::<1>(result);
                    *dp.add(2) = f32x4_extract_lane::<2>(result);
                }
            }
        }
    }

    /// Dual Kawase blur via mip chain with ping-pong buffers (2 allocations total).
    fn dual_kawase_blur(source: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
        let max_levels = ((w.min(h) as f32).log2() as usize).saturating_sub(1);
        let levels = ((radius + 3) / 4).max(1).min(max_levels).min(8);

        let full = w * h * 3;
        // +4 padding so v128_load on last pixel's RGB doesn't overrun
        let mut buf_a = source.to_vec();
        buf_a.extend_from_slice(&[0.0; 4]);
        let mut buf_b = vec![0.0f32; full + 4];

        // Track mip dimensions
        let mut dims: Vec<(usize, usize)> = Vec::with_capacity(levels + 1);
        dims.push((w, h));
        let mut src_is_a = true;

        // Downsample chain: ping-pong between buf_a and buf_b
        for _ in 0..levels {
            let &(pw, ph) = unsafe { dims.last().unwrap_unchecked() };
            if pw < 4 || ph < 4 { break; }
            let dw = pw / 2;
            let dh = ph / 2;
            if src_is_a {
                Self::kawase_down_into(&buf_a, pw, ph, &mut buf_b, dw, dh);
            } else {
                Self::kawase_down_into(&buf_b, pw, ph, &mut buf_a, dw, dh);
            }
            dims.push((dw, dh));
            src_is_a = !src_is_a;
        }

        // Upsample chain: continue ping-pong
        let last = dims.len() - 1;
        for i in (0..last).rev() {
            let (cw, ch) = dims[i + 1];
            let (tw, th) = dims[i];
            if src_is_a {
                Self::kawase_up_into(&buf_a, cw, ch, &mut buf_b, tw, th);
            } else {
                Self::kawase_up_into(&buf_b, cw, ch, &mut buf_a, tw, th);
            }
            src_is_a = !src_is_a;
        }

        if src_is_a { buf_a } else { buf_b }
    }

    /// Blur an RGB f32 source buffer and additively blend onto self.pixels.
    /// Uses Dual Kawase mip chain for multi-scale bloom.
    fn blur_and_blend(&mut self, source: &[f32], intensity: f32, radius: usize) {
        let w = self.width;
        let h = self.height;
        let n = w * h;

        let blurred = Self::dual_kawase_blur(source, w, h, radius);

        // Additive blend back (SIMD: 4 u8 channels at a time)
        let total = n * 3;
        let simd_end = (total / 4) * 4;
        let intensity_v = f32x4_splat(intensity);
        let max_v = f32x4_splat(255.0);
        let pp = self.pixels.as_mut_ptr();
        let bp = blurred.as_ptr();
        unsafe {
        let mut i = 0usize;
        while i < simd_end {
            // Load 4 u8 channels, widen to f32x4
            let raw = v128_load32_zero(pp.add(i) as *const u32);
            let u16v = u16x8_extend_low_u8x16(raw);
            let u32v = u32x4_extend_low_u16x8(u16v);
            let pf = f32x4_convert_u32x4(u32v);
            // Load 4 f32 blur values, multiply by intensity, add
            let bv = f32x4_mul(v128_load(bp.add(i) as *const v128), intensity_v);
            let result = f32x4_min(f32x4_add(pf, bv), max_v);
            // Narrow f32 → i32 → u16 → u8, store 4 bytes
            let i32v = i32x4_trunc_sat_f32x4(result);
            let i16v = u16x8_narrow_i32x4(i32v, i32v);
            let u8v = u8x16_narrow_i16x8(i16v, i16v);
            *(pp.add(i) as *mut u32) = i32x4_extract_lane::<0>(u8v) as u32;
            i += 4;
        }
        for i in simd_end..total {
            let v = *pp.add(i) as f32 + *bp.add(i) * intensity;
            *pp.add(i) = if v > 255.0 { 255 } else { v as u8 };
        }
        }
    }

    /// Bloom: extract bright pixels by luminance threshold, blur, add back.
    pub fn apply_bloom(&mut self, threshold: f32, intensity: f32, radius: usize) {
        let n = self.width * self.height;
        let mut buf = vec![0.0f32; n * 3 + 4]; // +4 for kawase v128_load padding

        // SIMD: process 4 pixels at a time using shuffle deinterleave
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
            let pi = i * 3;
            // Load 16 bytes: [R0 G0 B0 R1 G1 B1 R2 G2 B2 R3 G3 B3 X X X X]
            let raw = v128_load(pp.add(pi) as *const v128);
            // Deinterleave to separate R, G, B (4 values each in low bytes)
            let rb = i8x16_shuffle::<0, 3, 6, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            let gb = i8x16_shuffle::<1, 4, 7, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            let bb = i8x16_shuffle::<2, 5, 8, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0>(raw, raw);
            // Widen u8 → f32
            let rf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(rb)));
            let gf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(gb)));
            let bf = f32x4_convert_u32x4(u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(bb)));
            // Luma = 0.2126*R + 0.7152*G + 0.0722*B
            let lum4 = f32x4_add(f32x4_add(f32x4_mul(lum_r, rf), f32x4_mul(lum_g, gf)), f32x4_mul(lum_b, bf));
            let above = f32x4_gt(lum4, thresh_v);
            let mask = i32x4_bitmask(above);
            if mask != 0 {
                let factor = f32x4_min(f32x4_sub(f32x4_mul(lum4, inv255), thresh_s), one_v);
                let factor = v128_and(factor, above); // zero out below-threshold
                let rout = f32x4_mul(rf, factor);
                let gout = f32x4_mul(gf, factor);
                let bout = f32x4_mul(bf, factor);
                let bp = buf.as_mut_ptr().add(pi);
                *bp       = f32x4_extract_lane::<0>(rout);
                *bp.add(1) = f32x4_extract_lane::<0>(gout);
                *bp.add(2) = f32x4_extract_lane::<0>(bout);
                *bp.add(3) = f32x4_extract_lane::<1>(rout);
                *bp.add(4) = f32x4_extract_lane::<1>(gout);
                *bp.add(5) = f32x4_extract_lane::<1>(bout);
                *bp.add(6) = f32x4_extract_lane::<2>(rout);
                *bp.add(7) = f32x4_extract_lane::<2>(gout);
                *bp.add(8) = f32x4_extract_lane::<2>(bout);
                *bp.add(9) = f32x4_extract_lane::<3>(rout);
                *bp.add(10) = f32x4_extract_lane::<3>(gout);
                *bp.add(11) = f32x4_extract_lane::<3>(bout);
            }
            i += 4;
        }
        // Scalar remainder
        for i in simd_end..n {
            let pi = i * 3;
            let p = pp.add(pi);
            let rf = *p as f32;
            let gf = *p.add(1) as f32;
            let bf = *p.add(2) as f32;
            let lum = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
            if lum > threshold * 255.0 {
                let factor = (lum / 255.0 - threshold).min(1.0);
                *buf.get_unchecked_mut(pi) = rf * factor;
                *buf.get_unchecked_mut(pi + 1) = gf * factor;
                *buf.get_unchecked_mut(pi + 2) = bf * factor;
            }
        }
        }
        self.blur_and_blend(&buf, intensity, radius);
    }

    /// Glow: extract all foreground pixels (model silhouette), blur, add back.
    /// Creates a light-emitting aura around the entire model.
    pub fn apply_glow(&mut self, color: (u8, u8, u8), intensity: f32, radius: usize) {
        let n = self.width * self.height;
        let cr = color.0 as f32;
        let cg = color.1 as f32;
        let cb = color.2 as f32;
        let mut buf = vec![0.0f32; n * 3 + 4]; // +4 for kawase v128_load padding

        // SIMD: batch depth test 4 pixels at a time
        let simd_end = (n / 4) * 4;
        let neg_inf_v = f32x4_splat(f32::NEG_INFINITY);
        let zp = self.zbuf.as_ptr();
        unsafe {
        let mut i = 0usize;
        while i < simd_end {
            let d4 = v128_load(zp.add(i) as *const v128);
            let mask = i32x4_bitmask(f32x4_ne(d4, neg_inf_v));
            if mask != 0 {
                let bp = buf.as_mut_ptr().add(i * 3);
                if mask & 1 != 0 { *bp = cr; *bp.add(1) = cg; *bp.add(2) = cb; }
                if mask & 2 != 0 { *bp.add(3) = cr; *bp.add(4) = cg; *bp.add(5) = cb; }
                if mask & 4 != 0 { *bp.add(6) = cr; *bp.add(7) = cg; *bp.add(8) = cb; }
                if mask & 8 != 0 { *bp.add(9) = cr; *bp.add(10) = cg; *bp.add(11) = cb; }
            }
            i += 4;
        }
        for i in simd_end..n {
            if *self.zbuf.get_unchecked(i) != f32::NEG_INFINITY {
                let p = buf.as_mut_ptr().add(i * 3);
                *p = cr; *p.add(1) = cg; *p.add(2) = cb;
            }
        }
        }
        self.blur_and_blend(&buf, intensity, radius);
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

        // Ring buffer: 3 rows (prev, curr, next)
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

            for x in 1..w - 1 {
                let xo = x * 3;
                for c in 0..3 {
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
                    self.pixels[dst + xo + c] = v.max(0.0).min(255.0) as u8;
                }
            }
        }
    }

    /// Encode the pixel buffer as PNG.
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        Ok(crate::png_encoder::encode_png_rgb8(
            self.width as u32,
            self.height as u32,
            &self.pixels,
        ))
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Offset triangle points by (ox, oy).
#[inline]
fn offset(pts: &[(f64, f64); 3], ox: f64, oy: f64) -> [(f64, f64); 3] {
    [
        (pts[0].0 + ox, pts[0].1 + oy),
        (pts[1].0 + ox, pts[1].1 + oy),
        (pts[2].0 + ox, pts[2].1 + oy),
    ]
}

/// Signed edge function: positive if P is to the left of edge A→B.
#[inline]
fn edge(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

// ---------------------------------------------------------------------------
// Triangle setup for scanline + SIMD rasterization
// ---------------------------------------------------------------------------

struct TriSetup {
    min_x: usize, max_x: usize,
    min_y: usize, max_y: usize,
    dw0_dx: f64, dw0_dy: f64,
    dw1_dx: f64, dw1_dy: f64,
    dw2_dx: f64, dw2_dy: f64,
    row_w0: f64, row_w1: f64, row_w2: f64,
}

impl TriSetup {
    #[inline]
    fn new(pts: &[(f64, f64); 3], width: usize, height: usize) -> Option<Self> {
        let w = width as f64;
        let h = height as f64;

        let min_x = pts[0].0.min(pts[1].0).min(pts[2].0).max(0.0) as usize;
        let max_x = (pts[0].0.max(pts[1].0).max(pts[2].0).min(w - 1.0) as usize).min(width - 1);
        let min_y = pts[0].1.min(pts[1].1).min(pts[2].1).max(0.0) as usize;
        let max_y = (pts[0].1.max(pts[1].1).max(pts[2].1).min(h - 1.0) as usize).min(height - 1);

        let area = edge(pts[0], pts[1], pts[2]);
        if area.abs() < 1e-6 { return None; }
        let inv_area = 1.0 / area;

        let dw0_dx = (pts[1].1 - pts[2].1) * inv_area;
        let dw0_dy = (pts[2].0 - pts[1].0) * inv_area;
        let dw1_dx = (pts[2].1 - pts[0].1) * inv_area;
        let dw1_dy = (pts[0].0 - pts[2].0) * inv_area;
        let dw2_dx = (pts[0].1 - pts[1].1) * inv_area;
        let dw2_dy = (pts[1].0 - pts[0].0) * inv_area;

        let p0 = (min_x as f64 + 0.5, min_y as f64 + 0.5);
        let row_w0 = edge(pts[1], pts[2], p0) * inv_area;
        let row_w1 = edge(pts[2], pts[0], p0) * inv_area;
        let row_w2 = edge(pts[0], pts[1], p0) * inv_area;

        Some(Self {
            min_x, max_x, min_y, max_y,
            dw0_dx, dw0_dy, dw1_dx, dw1_dy, dw2_dx, dw2_dy,
            row_w0, row_w1, row_w2,
        })
    }

    /// Compute conservative scanline X range for the current row weights.
    /// Returns None if the row has no interior pixels.
    #[inline]
    fn scanline(&self, row_w0: f64, row_w1: f64, row_w2: f64) -> Option<(usize, usize)> {
        let mut left = self.min_x as f64;
        let mut right = self.max_x as f64;

        for &(w, dw) in &[(row_w0, self.dw0_dx), (row_w1, self.dw1_dx), (row_w2, self.dw2_dx)] {
            if dw.abs() < 1e-12 {
                if w < -1e-9 { return None; }
            } else {
                let x_cross = self.min_x as f64 - w / dw;
                if dw > 0.0 {
                    left = left.max(x_cross);
                } else {
                    right = right.min(x_cross);
                }
            }
        }

        // Conservative: expand by 1 pixel each side to catch FP edge cases
        let xl = ((left - 1.0).max(self.min_x as f64)) as usize;
        let xr = (((right + 1.0) as usize).min(self.max_x)).min(self.max_x);
        if xl > xr { return None; }
        Some((xl, xr))
    }
}

