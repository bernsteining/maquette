use super::*;

impl PixelBuffer {
    /// Check if a triangle can be skipped entirely via Hi-Z: true when its
    /// closest point is behind every overlapped tile's depth lower bound.
    ///
    /// Hi-Z is maintained lazily. A tile holds −∞ until all its pixels are
    /// covered; when a test consults such a tile it resumes the tile's scan
    /// (`hiz_scan`) instead of every triangle rescanning after it rasterizes.
    /// Depths only ever increase, so a stored minimum stays a valid lower
    /// bound — the schedule of updates only affects how much gets culled, never
    /// the image.
    #[inline]
    pub fn hiz_can_skip(&mut self, pts: &[(f64, f64); 3], tri_max_depth: f32) -> bool {
        #[inline(always)]
        fn span(a: f64, b: f64, c: f64, hi: f64) -> (usize, usize) {
            let (lo, top) = min_max3(a, b, c);
            let lo = if lo > 0.0 { lo } else { 0.0 };
            let top = if top < hi { top } else { hi };
            ((lo as usize) >> HIZ_SHIFT, (top as usize) >> HIZ_SHIFT)
        }
        let (tx0, tx1) = span(pts[0].0, pts[1].0, pts[2].0, (self.width - 1) as f64);
        let (ty0, ty1) = span(pts[0].1, pts[1].1, pts[2].1, (self.height - 1) as f64);
        for ty in ty0..=ty1 {
            let row = ty * self.hiz_tiles_x;
            for tx in tx0..=tx1 {
                let idx = row + tx;
                let mut hz = unsafe { *self.hiz.get_unchecked(idx) };
                if hz == f32::NEG_INFINITY {
                    let block = unsafe { *self.hiz_block.get_unchecked(idx) } as usize;
                    if unsafe { *self.zbuf.get_unchecked(block) } == f32::NEG_INFINITY {
                        return false;
                    }
                    hz = self.hiz_resume(idx, tx, ty);
                }
                if tri_max_depth > hz {
                    return false;
                }
            }
        }
        true
    }

    /// Continue scanning an incomplete tile from where the last scan stopped;
    /// returns the tile's depth lower bound once every pixel is covered, −∞
    /// otherwise.
    fn hiz_resume(&mut self, idx: usize, tx: usize, ty: usize) -> f32 {
        let px_start = tx << HIZ_SHIFT;
        let py_start = ty << HIZ_SHIFT;
        let px_end = (px_start + HIZ_SIZE).min(self.width);
        let py_end = (py_start + HIZ_SIZE).min(self.height);
        let tw = px_end - px_start;
        let (mut pos, mut tile_min) = unsafe { *self.hiz_scan.get_unchecked(idx) };
        let mut py = py_start + pos as usize / tw;
        let mut px = px_start + pos as usize % tw;
        while py < py_end {
            let row_base = py * self.width;
            while px < px_end {
                let z = unsafe { *self.zbuf.get_unchecked(row_base + px) };
                if z == f32::NEG_INFINITY {
                    unsafe {
                        *self.hiz_scan.get_unchecked_mut(idx) = (pos, tile_min);
                        *self.hiz_block.get_unchecked_mut(idx) = (row_base + px) as u32;
                    }
                    return f32::NEG_INFINITY;
                }
                if z < tile_min { tile_min = z; }
                px += 1;
                pos += 1;
            }
            px = px_start;
            py += 1;
        }
        unsafe { *self.hiz.get_unchecked_mut(idx) = tile_min; }
        tile_min
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
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let d = [depths[0] as f32, depths[1] as f32, depths[2] as f32];
        let mut sink = FlatSink {
            d,
            d_v: [f32x4_splat(d[0]), f32x4_splat(d[1]), f32x4_splat(d[2])],
            rgb: [r, g, b],
            rgb4: unsafe { v128_load([r, g, b, r, g, b, r, g, b, r, g, b, 0, 0, 0, 0].as_ptr() as *const v128) },
            zbuf: &mut self.zbuf,
            pixels: &mut self.pixels,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
    }

    /// Fill the ellipse `pts[0] + s·(pts[1] − pts[0]) + t·(pts[2] − pts[0])`,
    /// `s² + t² ≤ 1` — a disc splat after projection of its centre and two
    /// radius vectors — with depth interpolated over that plane.
    pub fn rasterize_splat(&mut self, pts: &[(f64, f64); 3], depths: &[f64; 3], r: u8, g: u8, b: u8) {
        let (cx, cy) = pts[0];
        let (ax, ay) = (pts[1].0 - cx, pts[1].1 - cy);
        let (bx, by) = (pts[2].0 - cx, pts[2].1 - cy);
        let det = ax * by - bx * ay;
        if det.abs() < 1e-12 || !det.is_finite() { return; }
        let inv = 1.0 / det;
        let (sx, tx) = (by * inv, -ay * inv);
        let aa = sx * sx + tx * tx;
        let (z0, dz1, dz2) = (depths[0], depths[1] - depths[0], depths[2] - depths[0]);
        let hy = (ay * ay + by * by).sqrt();
        let y0 = ((cy - hy - 0.5).ceil()).fmax(0.0) as usize;
        let y1 = (cy + hy - 0.5).floor();
        if y1 < 0.0 { return; }
        let y1 = (y1 as usize).min(self.height - 1);
        for py in y0..=y1 {
            let dy = py as f64 + 0.5 - cy;
            let (s0, t0) = (-bx * dy * inv, ax * dy * inv);
            let bb = s0 * sx + t0 * tx;
            let disc = bb * bb - aa * (s0 * s0 + t0 * t0 - 1.0);
            if disc < 0.0 { continue; }
            let root = disc.sqrt();
            let xa = (cx + (-bb - root) / aa - 0.5).ceil().fmax(0.0);
            let xb = (cx + (-bb + root) / aa - 0.5).floor();
            if xb < xa { continue; }
            let (xa, xb) = (xa as usize, (xb as usize).min(self.width - 1));
            let row = py * self.width;
            for px in xa..=xb {
                let dx = px as f64 + 0.5 - cx;
                let depth = (z0 + (s0 + sx * dx) * dz1 + (t0 + tx * dx) * dz2) as f32;
                let idx = row + px;
                unsafe {
                    if depth > *self.zbuf.get_unchecked(idx) {
                        *self.zbuf.get_unchecked_mut(idx) = depth;
                        let p = self.pixels.as_mut_ptr().add(idx * 3);
                        *p = r; *p.add(1) = g; *p.add(2) = b;
                    }
                }
            }
        }
    }

    /// Overlay engineering-style section hatching onto a triangle (clip caps in
    /// PNG output — the SVG path uses a `<pattern>` fill instead). Draws parallel
    /// lines in screen space so they stay continuous across the triangulated cap,
    /// clipped to the triangle's covered pixels and gated by the z-buffer so only
    /// visible cap fragments are hatched. Lines are anti-aliased by coverage, so
    /// they read cleanly even without supersampling. `spacing`/`half_width` are in
    /// buffer pixels (output units × the antialias factor); `(cos_a, sin_a)` is
    /// the line-angle direction. Colour is blended over the existing cap fill.
    /// `style`: 0 = parallel lines, 1 = cross-hatch, 2 = plus marks. `arm` is the
    /// half-length (buffer px) of a plus mark's arms (used only for style 2).
    #[allow(clippy::too_many_arguments)]
    pub fn hatch_triangle(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        spacing: f64,
        half_width: f64,
        cos_a: f64,
        sin_a: f64,
        style: u8,
        arm: f64,
        color: (u8, u8, u8),
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &self.zbuf;
        let pixels = &mut self.pixels;
        let (hr, hg, hb) = (color.0 as f64, color.1 as f64, color.2 as f64);
        let inv_spacing = 1.0 / spacing;

        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;

        let dist_line = |x: f64| { let r = x * inv_spacing; (r - r.fround()).abs() * spacing };
        let dist_center = |x: f64| { let r = x * inv_spacing - 0.5; (r - r.fround()).abs() * spacing };
        let feather = |d: f64| (half_width + 0.5 - d).clamp(0.0, 1.0);

        let mut row_w0 = setup.row_w0;
        let mut row_w1 = setup.row_w1;
        let mut row_w2 = setup.row_w2;

        unsafe {
            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let mut w0s = (row_w0 + offset * setup.dw0_dx) as f32;
                    let mut w1s = (row_w1 + offset * setup.dw1_dx) as f32;
                    let mut w2s = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;
                    let row_base = py * width;
                    let mut u = (xl as f64) * cos_a + (py as f64) * sin_a;
                    let mut v = -(xl as f64) * sin_a + (py as f64) * cos_a;
                    let mut px = xl;
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let idx = row_base + px;
                            let zb = *zbuf.get_unchecked(idx);
                            if depth >= zb - (zb.abs() * 1e-3 + 1e-2) {
                                let cov = match style {
                                    1 => feather(dist_line(u)).fmax(feather(dist_line(v))),
                                    2 => {
                                        let (du, dv) = (dist_center(u), dist_center(v));
                                        let cv = if dv <= arm { feather(du) } else { 0.0 };
                                        let ch = if du <= arm { feather(dv) } else { 0.0 };
                                        cv.fmax(ch)
                                    }
                                    _ => feather(dist_line(u)),
                                };
                                if cov > 0.0 {
                                    let inv = 1.0 - cov;
                                    let p = pixels.as_mut_ptr().add(idx * 3);
                                    *p = (*p as f64 * inv + hr * cov) as u8;
                                    *p.add(1) = (*p.add(1) as f64 * inv + hg * cov) as u8;
                                    *p.add(2) = (*p.add(2) as f64 * inv + hb * cov) as u8;
                                }
                            }
                        }
                        w0s += dw0;
                        w1s += dw1;
                        w2s += dw2;
                        u += cos_a;
                        v -= sin_a;
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
        let Some(setup) = TriSetup::new(pts, width, height) else { return };
        unsafe { for_each_fragment(&setup, width, &mut MaskSink { mask }) };
    }

    /// Apply shadow: blend shadow color with existing pixels where mask is set.
    pub fn apply_shadow(&mut self, mask: &[bool], sr: u8, sg: u8, sb: u8, opacity: f64) {
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
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let d = [depths[0] as f32, depths[1] as f32, depths[2] as f32];
        let c = colors.map(|c| f32x4(c.0 as f32, c.1 as f32, c.2 as f32, 0.0));
        let mut sink = SmoothSink {
            d,
            d_v: [f32x4_splat(d[0]), f32x4_splat(d[1]), f32x4_splat(d[2])],
            c,
            r: colors.map(|c| f32x4_splat(c.0 as f32)),
            g: colors.map(|c| f32x4_splat(c.1 as f32)),
            b: colors.map(|c| f32x4_splat(c.2 as f32)),
            zbuf: &mut self.zbuf,
            pixels: &mut self.pixels,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
    }

    /// Textured rasterize (OBJ `map_Kd`): affine-interpolate per-corner UVs and
    /// per-vertex lighting, sample the bound texture per pixel and modulate
    /// (albedo × light) in display space. Scalar path — per-pixel texture
    /// sampling dominates, so SIMD coverage would add little. `lod` is the
    /// per-triangle mip level; `light` is the white-albedo shaded vertex colors.
    pub fn rasterize_triangle_textured(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        uvs: &[[f32; 2]; 3],
        light: &[(u8, u8, u8); 3],
        tex: &crate::texture::Texture,
        lod: f32,
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

        let mut row_w0 = setup.row_w0;
        let mut row_w1 = setup.row_w1;
        let mut row_w2 = setup.row_w2;

        unsafe {
            for py in setup.min_y..=setup.max_y {
                if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                    let offset = (xl - setup.min_x) as f64;
                    let mut w0 = (row_w0 + offset * setup.dw0_dx) as f32;
                    let mut w1 = (row_w1 + offset * setup.dw1_dx) as f32;
                    let mut w2 = (row_w2 + offset * setup.dw2_dx) as f32;
                    let dw0 = setup.dw0_dx as f32;
                    let dw1 = setup.dw1_dx as f32;
                    let dw2 = setup.dw2_dx as f32;
                    let row_base = py * width;
                    let mut px = xl;
                    while px <= xr {
                        if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                            let depth = w0 * d0 + w1 * d1 + w2 * d2;
                            let idx = row_base + px;
                            if depth > *zbuf.get_unchecked(idx) {
                                *zbuf.get_unchecked_mut(idx) = depth;
                                let u = w0 * uvs[0][0] + w1 * uvs[1][0] + w2 * uvs[2][0];
                                let v = w0 * uvs[0][1] + w1 * uvs[1][1] + w2 * uvs[2][1];
                                let t = tex.sample_lod([u, v], lod);
                                let lr = w0 * light[0].0 as f32 + w1 * light[1].0 as f32 + w2 * light[2].0 as f32;
                                let lg = w0 * light[0].1 as f32 + w1 * light[1].1 as f32 + w2 * light[2].1 as f32;
                                let lb = w0 * light[0].2 as f32 + w1 * light[1].2 as f32 + w2 * light[2].2 as f32;
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                *p        = (lr * t[0]).fround().clamp(0.0, 255.0) as u8;
                                *p.add(1) = (lg * t[1]).fround().clamp(0.0, 255.0) as u8;
                                *p.add(2) = (lb * t[2]).fround().clamp(0.0, 255.0) as u8;
                            }
                        }
                        w0 += dw0; w1 += dw1; w2 += dw2;
                        px += 1;
                    }
                }
                row_w0 += setup.dw0_dy;
                row_w1 += setup.dw1_dy;
                row_w2 += setup.dw2_dy;
            }
        }
    }

    /// Per-pixel-shaded rasterize (used by per-pixel shadows): interpolates the
    /// vertex colors + world positions affinely, then runs `shade(color, world)`
    /// per covered pixel to produce the final color. Scalar (quality) path.
    pub fn rasterize_triangle_shadowed<F: FnMut((u8, u8, u8), [f64; 3]) -> (u8, u8, u8)>(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        colors: &[(u8, u8, u8); 3],
        world: &[[f64; 3]; 3],
        mut shade: F,
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        let zbuf = &mut self.zbuf;
        let pixels = &mut self.pixels;
        let (c0r, c0g, c0b) = (colors[0].0 as f64, colors[0].1 as f64, colors[0].2 as f64);
        let (c1r, c1g, c1b) = (colors[1].0 as f64, colors[1].1 as f64, colors[1].2 as f64);
        let (c2r, c2g, c2b) = (colors[2].0 as f64, colors[2].1 as f64, colors[2].2 as f64);
        let (d0, d1, d2) = (depths[0], depths[1], depths[2]);

        let mut row_w0 = setup.row_w0;
        let mut row_w1 = setup.row_w1;
        let mut row_w2 = setup.row_w2;
        for py in setup.min_y..=setup.max_y {
            if let Some((xl, xr)) = setup.scanline(row_w0, row_w1, row_w2) {
                let off = (xl - setup.min_x) as f64;
                let mut w0 = row_w0 + off * setup.dw0_dx;
                let mut w1 = row_w1 + off * setup.dw1_dx;
                let mut w2 = row_w2 + off * setup.dw2_dx;
                let row_base = py * width;
                for px in xl..=xr {
                    if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                        let depth = (w0 * d0 + w1 * d1 + w2 * d2) as f32;
                        let idx = row_base + px;
                        unsafe {
                            if depth > *zbuf.get_unchecked(idx) {
                                *zbuf.get_unchecked_mut(idx) = depth;
                                let r = (w0 * c0r + w1 * c1r + w2 * c2r).fround().clamp(0.0, 255.0) as u8;
                                let g = (w0 * c0g + w1 * c1g + w2 * c2g).fround().clamp(0.0, 255.0) as u8;
                                let b = (w0 * c0b + w1 * c1b + w2 * c2b).fround().clamp(0.0, 255.0) as u8;
                                let wx = w0 * world[0][0] + w1 * world[1][0] + w2 * world[2][0];
                                let wy = w0 * world[0][1] + w1 * world[1][1] + w2 * world[2][1];
                                let wz = w0 * world[0][2] + w1 * world[1][2] + w2 * world[2][2];
                                let (fr, fg, fb) = shade((r, g, b), [wx, wy, wz]);
                                let p = pixels.as_mut_ptr().add(idx * 3);
                                *p = fr;
                                *p.add(1) = fg;
                                *p.add(2) = fb;
                            }
                        }
                    }
                    w0 += setup.dw0_dx;
                    w1 += setup.dw1_dx;
                    w2 += setup.dw2_dx;
                }
            }
            row_w0 += setup.dw0_dy;
            row_w1 += setup.dw1_dy;
            row_w2 += setup.dw2_dy;
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
        let src = f32x4(r as f32 * opacity as f32, g as f32 * opacity as f32, b as f32 * opacity as f32, 0.0);
        self.blend_fill::<false>(pts, depths, [src; 3], src, opacity);
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
        let c = colors.map(|c| f32x4(c.0 as f32, c.1 as f32, c.2 as f32, 0.0));
        self.blend_fill::<true>(pts, depths, c, f32x4_splat(0.0), opacity);
    }

    fn blend_fill<const SMOOTH: bool>(&mut self, pts: &[(f64, f64); 3], depths: &[f64; 3], c: [v128; 3], src: v128, opacity: f64) {
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        if self.tcov.is_empty() { self.tcov = vec![0.0; self.width * self.height]; }
        let d = [depths[0] as f32, depths[1] as f32, depths[2] as f32];
        let mut sink = BlendSink::<SMOOTH> {
            d,
            d_v: [f32x4_splat(d[0]), f32x4_splat(d[1]), f32x4_splat(d[2])],
            c,
            src,
            inv: f32x4_splat((1.0 - opacity) as f32),
            opa: f32x4_splat(opacity as f32),
            opf: opacity as f32,
            zbuf: &self.zbuf,
            pixels: &mut self.pixels,
            tcov: &mut self.tcov,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
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
        let mut ix0 = x0.fround() as i64;
        let mut iy0 = y0.fround() as i64;
        let ix1 = x1.fround() as i64;
        let iy1 = y1.fround() as i64;

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
    pub fn draw_triangle_edges_z(&mut self, pts: &[(f64, f64); 3], depths: &[f64; 3], bias: f32, r: u8, g: u8, b: u8) {
        for e in 0..3 {
            let n = (e + 1) % 3;
            self.draw_line_z(pts[e].0, pts[e].1, depths[e] as f32, pts[n].0, pts[n].1, depths[n] as f32, bias, r, g, b);
        }
    }

    /// Draw a line with depth interpolation and z-buffer testing; `bias` lets a
    /// line lying on a surface win against that surface's own depth.
    pub fn draw_line_z(&mut self, x0: f64, y0: f64, z0: f32, x1: f64, y1: f64, z1: f32, bias: f32, r: u8, g: u8, b: u8) {
        let w = self.width as i64;
        let h = self.height as i64;
        let mut ix = x0.fround() as i64;
        let mut iy = y0.fround() as i64;
        let ix1 = x1.fround() as i64;
        let iy1 = y1.fround() as i64;

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
                if z + bias >= self.zbuf[idx] {
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
}


/// Offset triangle points by (ox, oy).
#[inline]
fn offset(pts: &[(f64, f64); 3], ox: f64, oy: f64) -> [(f64, f64); 3] {
    [
        (pts[0].0 + ox, pts[0].1 + oy),
        (pts[1].0 + ox, pts[1].1 + oy),
        (pts[2].0 + ox, pts[2].1 + oy),
    ]
}

/// Write the first 12 bytes of `rgb4` (4 interleaved RGB pixels starting at
/// pixel `idx0`) for the lanes set in `pass`, leaving the other pixels intact.
///
/// # Safety
/// Pixels `idx0..idx0 + 4` must lie inside `pixels`.
#[inline(always)]
unsafe fn store_rgb4(pixels: &mut [u8], idx0: usize, rgb4: v128, pass: v128, wmask: u8) {
    let p = pixels.as_mut_ptr().add(idx0 * 3);
    if idx0 * 3 + 16 <= pixels.len() {
        let m = i8x16_shuffle::<0, 0, 0, 4, 4, 4, 8, 8, 8, 12, 12, 12, 16, 16, 16, 16>(pass, u8x16_splat(0));
        v128_store(p as *mut v128, v128_bitselect(rgb4, v128_load(p as *const v128), m));
    } else {
        let mut b = [0u8; 16];
        v128_store(b.as_mut_ptr() as *mut v128, rgb4);
        for l in 0..4 {
            if wmask & (1 << l) != 0 {
                std::ptr::copy_nonoverlapping(b.as_ptr().add(l * 3), p.add(l * 3), 3);
            }
        }
    }
}

struct FlatSink<'a> {
    d: [f32; 3],
    d_v: [v128; 3],
    rgb: [u8; 3],
    rgb4: v128,
    zbuf: &'a mut [f32],
    pixels: &'a mut [u8],
}

impl FragmentSink for FlatSink<'_> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let depth_v = Varyings::lerp_v(w, &self.d_v);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
        let wmask = i32x4_bitmask(pass);
        if wmask != 0 {
            v128_store(self.zbuf.as_mut_ptr().add(idx0) as *mut v128, v128_bitselect(depth_v, zbuf_v, pass));
            store_rgb4(self.pixels, idx0, self.rgb4, pass, wmask);
        }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let depth = w0s * self.d[0] + w1s * self.d[1] + w2s * self.d[2];
        if depth > *self.zbuf.get_unchecked(idx) {
            *self.zbuf.get_unchecked_mut(idx) = depth;
            let p = self.pixels.as_mut_ptr().add(idx * 3);
            *p = self.rgb[0]; *p.add(1) = self.rgb[1]; *p.add(2) = self.rgb[2];
        }
    }
}

struct MaskSink<'a> {
    mask: &'a mut [bool],
}

impl FragmentSink for MaskSink<'_> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, _w: [v128; 3], inside: v128) {
        let wmask = i32x4_bitmask(inside);
        if wmask & 1 != 0 { *self.mask.get_unchecked_mut(idx0) = true; }
        if wmask & 2 != 0 { *self.mask.get_unchecked_mut(idx0 + 1) = true; }
        if wmask & 4 != 0 { *self.mask.get_unchecked_mut(idx0 + 2) = true; }
        if wmask & 8 != 0 { *self.mask.get_unchecked_mut(idx0 + 3) = true; }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, _w0s: f32, _w1s: f32, _w2s: f32) {
        *self.mask.get_unchecked_mut(idx) = true;
    }
}

struct SmoothSink<'a> {
    d: [f32; 3],
    d_v: [v128; 3],
    c: [v128; 3],
    r: [v128; 3],
    g: [v128; 3],
    b: [v128; 3],
    zbuf: &'a mut [f32],
    pixels: &'a mut [u8],
}

impl FragmentSink for SmoothSink<'_> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let depth_v = Varyings::lerp_v(w, &self.d_v);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let pass = v128_and(inside, f32x4_gt(depth_v, zbuf_v));
        let wmask = i32x4_bitmask(pass);
        if wmask != 0 {
            v128_store(self.zbuf.as_mut_ptr().add(idx0) as *mut v128, v128_bitselect(depth_v, zbuf_v, pass));
            let r = i32x4_trunc_sat_f32x4(f32x4_nearest(Varyings::lerp_v(w, &self.r)));
            let g = i32x4_trunc_sat_f32x4(f32x4_nearest(Varyings::lerp_v(w, &self.g)));
            let b = i32x4_trunc_sat_f32x4(f32x4_nearest(Varyings::lerp_v(w, &self.b)));
            let bytes = u8x16_narrow_i16x8(u16x8_narrow_i32x4(r, g), u16x8_narrow_i32x4(b, f32x4_splat(0.0)));
            let rgb4 = i8x16_shuffle::<0, 4, 8, 1, 5, 9, 2, 6, 10, 3, 7, 11, 12, 13, 14, 15>(bytes, bytes);
            store_rgb4(self.pixels, idx0, rgb4, pass, wmask);
        }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let depth = w0s * self.d[0] + w1s * self.d[1] + w2s * self.d[2];
        if depth > *self.zbuf.get_unchecked(idx) {
            *self.zbuf.get_unchecked_mut(idx) = depth;
            let rgb = f32x4_add(f32x4_add(
                f32x4_mul(f32x4_splat(w0s), self.c[0]),
                f32x4_mul(f32x4_splat(w1s), self.c[1])),
                f32x4_mul(f32x4_splat(w2s), self.c[2]));
            let rgb = i32x4_trunc_sat_f32x4(f32x4_nearest(rgb));
            let p = self.pixels.as_mut_ptr().add(idx * 3);
            *p = i32x4_extract_lane::<0>(rgb) as u8;
            *p.add(1) = i32x4_extract_lane::<1>(rgb) as u8;
            *p.add(2) = i32x4_extract_lane::<2>(rgb) as u8;
        }
    }
}

struct BlendSink<'a, const SMOOTH: bool> {
    d: [f32; 3],
    d_v: [v128; 3],
    c: [v128; 3],
    src: v128,
    inv: v128,
    opa: v128,
    opf: f32,
    zbuf: &'a [f32],
    pixels: &'a mut [u8],
    tcov: &'a mut [f32],
}

impl<const SMOOTH: bool> BlendSink<'_, SMOOTH> {
    #[inline(always)]
    unsafe fn blend(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let p = self.pixels.as_mut_ptr().add(idx * 3);
        let add = if SMOOTH {
            let interp = f32x4_add(f32x4_add(
                f32x4_mul(f32x4_splat(w0s), self.c[0]),
                f32x4_mul(f32x4_splat(w1s), self.c[1])),
                f32x4_mul(f32x4_splat(w2s), self.c[2]));
            f32x4_mul(interp, self.opa)
        } else {
            self.src
        };
        let existing = f32x4(*p as f32, *p.add(1) as f32, *p.add(2) as f32, 0.0);
        let result = i32x4_trunc_sat_f32x4(f32x4_nearest(f32x4_add(f32x4_mul(existing, self.inv), add)));
        *p = i32x4_extract_lane::<0>(result) as u8;
        *p.add(1) = i32x4_extract_lane::<1>(result) as u8;
        *p.add(2) = i32x4_extract_lane::<2>(result) as u8;
        let t = self.tcov.get_unchecked_mut(idx);
        *t += self.opf * (1.0 - *t);
    }
}

impl<const SMOOTH: bool> FragmentSink for BlendSink<'_, SMOOTH> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let depth_v = Varyings::lerp_v(w, &self.d_v);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let wmask = i32x4_bitmask(v128_and(inside, f32x4_gt(depth_v, zbuf_v)));
        let [w0, w1, w2] = w;
        if wmask & 1 != 0 { self.blend(idx0, f32x4_extract_lane::<0>(w0), f32x4_extract_lane::<0>(w1), f32x4_extract_lane::<0>(w2)); }
        if wmask & 2 != 0 { self.blend(idx0 + 1, f32x4_extract_lane::<1>(w0), f32x4_extract_lane::<1>(w1), f32x4_extract_lane::<1>(w2)); }
        if wmask & 4 != 0 { self.blend(idx0 + 2, f32x4_extract_lane::<2>(w0), f32x4_extract_lane::<2>(w1), f32x4_extract_lane::<2>(w2)); }
        if wmask & 8 != 0 { self.blend(idx0 + 3, f32x4_extract_lane::<3>(w0), f32x4_extract_lane::<3>(w1), f32x4_extract_lane::<3>(w2)); }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        if w0s * self.d[0] + w1s * self.d[1] + w2s * self.d[2] > *self.zbuf.get_unchecked(idx) {
            self.blend(idx, w0s, w1s, w2s);
        }
    }
}
