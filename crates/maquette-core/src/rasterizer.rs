//! Software triangle rasterizer shared by the maquette plugin family: one
//! z-buffered RGB `PixelBuffer` with the flat/smooth/textured/hatch fills,
//! the shaded and deferred paths, and the post passes.

use crate::math::{FloatExt, Vec3};

#[cfg(target_arch = "wasm32")] use std::arch::wasm32::*; #[cfg(not(target_arch = "wasm32"))] use crate::simd::*;

mod fill;
mod post;

const HIZ_SHIFT: usize = 4;
const HIZ_SIZE: usize = 1 << HIZ_SHIFT;

/// Per-pixel write behaviour once the shader has produced a linear RGBA sample.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// Overwrite RGB, ignore alpha. Used for opaque and mask materials.
    Overwrite,
    /// Straight src-over: `dst = src.rgb · src.a + dst.rgb · (1 − src.a)`.
    /// For correct translucency, callers must sort triangles back-to-front.
    /// z-buffer is still written so the transparent-background path counts
    /// blend pixels as covered. Kept for reference; new code uses WBOIT.
    #[allow(dead_code)]
    SrcOver,
    /// Weighted Blended Order-Independent Transparency (McGuire & Bavoil,
    /// 2013). Accumulates `(rgb·a·w, a·w)` into an f32 accum buffer and
    /// multiplies `(1−a)` into a revealage buffer. Composited into the
    /// main pixel buffer by `PixelBuffer::composite_oit` after all triangles
    /// are drawn. Order-independent — no back-to-front sort needed and
    /// correct for interpenetrating translucent geometry (which the centroid
    /// sort renders wrong).
    WBOIT,
}

/// 4-pixel SIMD input to a `PixelShader`. Each `v128` is 4×f32, one per pixel.
#[allow(dead_code)]
pub struct ShadeIn4 {
    pub pos_x: v128, pub pos_y: v128, pub pos_z: v128,
    pub n_x:   v128, pub n_y:   v128, pub n_z:   v128,
    pub uv_u:  v128, pub uv_v:  v128,
    /// TEXCOORD_1. Zero when the primitive lacks it — materials whose slots
    /// select `texcoord = 1` will sample at the origin, which is harmless
    /// (matches glTF's implicit-zero fallback).
    pub uv1_u: v128, pub uv1_v: v128,
    /// TEXCOORD_2. Same fallback semantics as `uv1_*`. Materials referencing
    /// TEXCOORD_3+ still collapse to slot 2 (see `clamp_texcoord`).
    pub uv2_u: v128, pub uv2_v: v128,
    /// COLOR_0 (linear RGBA). Splatted 1.0 when the primitive lacks it.
    /// glTF spec: multiplied with baseColor before shading.
    pub col_r: v128, pub col_g: v128, pub col_b: v128, pub col_a: v128,
    pub tan_x: v128, pub tan_y: v128, pub tan_z: v128,
    /// Bitangent handedness: `±1`. Splatted per pixel (invariant across
    /// interpolation because handedness is a per-triangle constant when the
    /// tangent generator agrees within a UV shell).
    pub tan_w: v128,
}

/// 4-pixel SIMD output from a `PixelShader`. Linear-space RGBA per lane, plus
/// a per-lane `keep` mask (`0xFFFFFFFF` = write, `0x00000000` = discard for
/// mask alpha).
pub struct ShadeOut4 {
    pub r: v128, pub g: v128, pub b: v128, pub a: v128,
    pub keep: v128,
}

/// Per-pixel shader with SIMD and scalar paths. The rasterizer takes any
/// implementer and monomorphises, so there's no vtable dispatch — the
/// PBR closure is inlined into the scanline loop.
pub trait PixelShader {
    /// Shade 4 pixels at once (called from the SIMD inner loop).
    fn shade4(&self, in_: ShadeIn4) -> ShadeOut4;
    /// Shade one pixel (called from the scalar scanline remainder). Returns
    /// `None` to discard the fragment (mask alpha).
    fn shade_scalar(&self, pos: Vec3, normal: Vec3, uv: [f32; 2], uv1: [f32; 2], uv2: [f32; 2], color: [f32; 4], tangent: [f32; 4]) -> Option<[f32; 4]>;
    /// The `keep` mask `shade4` would return for these inputs, computed
    /// without the rest of the shading. Lets the deferred depth pass resolve
    /// alpha-masked coverage exactly. Default: keep every lane.
    fn alpha_keep4(&self, _in: &ShadeIn4) -> v128 {
        i32x4_splat(-1)
    }
}

#[derive(Default)]
pub struct PixelBuffer {
    pub width: usize,
    pub height: usize,
    /// RGB, width*height*3 bytes.
    pub pixels: Vec<u8>,
    /// f32 depth, width*height. Initialized to −∞ (nothing rendered yet).
    pub zbuf: Vec<f32>,
    /// WBOIT accumulator: 4 channels (rgb·a·w, a·w) per pixel, f32. Empty
    /// until the first WBOIT write; lazy-init keeps opaque-only renders free.
    pub oit_accum: Vec<f32>,
    /// WBOIT revealage: single channel, initialized to 1.0. Empty until first
    /// WBOIT write. `reveal *= (1 - a)` per fragment; final composite mixes
    /// `bg × reveal + oit × (1 − reveal)`.
    pub oit_reveal: Vec<f32>,
    /// Set to true on the first WBOIT write. `composite_oit` short-circuits
    /// when false.
    pub oit_used: bool,
    vis: Vec<u32>,
    owned: Vec<u32>,
    tcov: Vec<f32>,
    bg: [f32; 3],
    /// Hi-Z: conservative lower bound on min zbuf per 16×16 tile.
    hiz: Vec<f32>,
    hiz_tiles_x: usize,
    /// Per tile still at −∞: how many pixels (row-major within the tile) are
    /// already known covered, and the minimum depth seen over them, so
    /// `hiz_resume` continues where the last scan stopped.
    hiz_scan: Vec<(u16, f32)>,
    /// Per incomplete tile: zbuf index of the empty pixel that stopped the last
    /// scan. While it stays empty the tile can't be complete, which the skip
    /// test checks inline without resuming the scan.
    hiz_block: Vec<u32>,
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
            bg: [bg.0 as f32, bg.1 as f32, bg.2 as f32],
            hiz: vec![f32::NEG_INFINITY; hiz_tiles_x * hiz_tiles_y],
            hiz_tiles_x,
            hiz_scan: vec![(0, f32::INFINITY); hiz_tiles_x * hiz_tiles_y],
            hiz_block: (0..hiz_tiles_x * hiz_tiles_y)
                .map(|t| (((t / hiz_tiles_x) << HIZ_SHIFT) * width + ((t % hiz_tiles_x) << HIZ_SHIFT)) as u32)
                .collect(),
            ..Default::default()
        }
    }

    /// Composite the WBOIT accum + reveal buffers over the opaque pixel
    /// buffer. Call once, after all triangles/points/lines have been
    /// rasterized. No-op when no translucent geometry was drawn.
    ///
    /// Per pixel:
    ///   avg_color = accum.rgb / max(accum.a, 1e-5)
    ///   out       = mix(bg, avg_color, 1 - reveal)
    /// The final blend runs in linear space (un-gamma the sRGB bg via LUT,
    /// re-gamma the result).
    pub fn composite_oit(&mut self) {
        if !self.oit_used { return; }
        let n = self.width * self.height;
        for idx in 0..n {
            let reveal = unsafe { *self.oit_reveal.get_unchecked(idx) };
            if reveal >= 1.0 { continue; }
            let ao = idx * 4;
            let (ar, ag, ab, aa) = unsafe {
                (
                    *self.oit_accum.get_unchecked(ao),
                    *self.oit_accum.get_unchecked(ao + 1),
                    *self.oit_accum.get_unchecked(ao + 2),
                    *self.oit_accum.get_unchecked(ao + 3),
                )
            };
            let inv_a = 1.0 / aa.fmax(1e-5);
            let avg_r = ar * inv_a;
            let avg_g = ag * inv_a;
            let avg_b = ab * inv_a;
            let po = idx * 3;
            let bg_r = crate::color::srgb_to_linear(unsafe { *self.pixels.get_unchecked(po) });
            let bg_g = crate::color::srgb_to_linear(unsafe { *self.pixels.get_unchecked(po + 1) });
            let bg_b = crate::color::srgb_to_linear(unsafe { *self.pixels.get_unchecked(po + 2) });
            let coverage = 1.0 - reveal;
            let r = avg_r * coverage + bg_r * reveal;
            let g = avg_g * coverage + bg_g * reveal;
            let b = avg_b * coverage + bg_b * reveal;
            unsafe {
                *self.pixels.get_unchecked_mut(po)     = crate::color::linear_to_srgb(r);
                *self.pixels.get_unchecked_mut(po + 1) = crate::color::linear_to_srgb(g);
                *self.pixels.get_unchecked_mut(po + 2) = crate::color::linear_to_srgb(b);
            }
        }
    }

    /// Depth-tested single-pixel write. Used by glTF POINTS primitives.
    /// `zbuf_key` follows the same convention as triangle rasterization:
    /// larger = closer.
    pub fn write_point(&mut self, xy: (usize, usize), zbuf_key: f32, rgba_linear: [f32; 4]) {
        let (x, y) = xy;
        if x >= self.width || y >= self.height { return; }
        let idx = y * self.width + x;
        if zbuf_key <= unsafe { *self.zbuf.get_unchecked(idx) } { return; }
        let (r, g, b) = crate::color::linear_rgb_to_srgb(
            rgba_linear[0].clamp(0.0, 1.0),
            rgba_linear[1].clamp(0.0, 1.0),
            rgba_linear[2].clamp(0.0, 1.0),
        );
        let po = idx * 3;
        unsafe {
            *self.pixels.get_unchecked_mut(po)     = r;
            *self.pixels.get_unchecked_mut(po + 1) = g;
            *self.pixels.get_unchecked_mut(po + 2) = b;
            *self.zbuf.get_unchecked_mut(idx)      = zbuf_key;
        }
    }

    /// Draw a 1-pixel-wide screen-space line with z-buffer test + per-pixel
    /// linear interp of z and colour. Used by glTF LINES / LINE_STRIP /
    /// LINE_LOOP primitives. Bresenham-ish stepping via DDA — good enough
    /// for 1-pixel unlit primitives, no anti-aliasing.
    pub fn draw_line_depth(
        &mut self,
        a: (f64, f64),
        b: (f64, f64),
        za: f32,
        zb: f32,
        rgba_a: [f32; 4],
        rgba_b: [f32; 4],
    ) {
        let (mut x0, mut y0) = (a.0, a.1);
        let (x1, y1) = (b.0, b.1);
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let steps = dx.fmax(dy).ceil() as i32;
        if steps == 0 {
            self.write_point((x0 as usize, y0 as usize), za, rgba_a);
            return;
        }
        let inv_steps = 1.0 / steps as f64;
        let sx = (x1 - x0) * inv_steps;
        let sy = (y1 - y0) * inv_steps;
        let sz = (zb - za) as f64 * inv_steps;
        let scr = (rgba_b[0] - rgba_a[0]) as f64 * inv_steps;
        let scg = (rgba_b[1] - rgba_a[1]) as f64 * inv_steps;
        let scb = (rgba_b[2] - rgba_a[2]) as f64 * inv_steps;
        let sca = (rgba_b[3] - rgba_a[3]) as f64 * inv_steps;
        let mut zc = za as f64;
        let mut cr = rgba_a[0] as f64;
        let mut cg = rgba_a[1] as f64;
        let mut cb = rgba_a[2] as f64;
        let mut ca = rgba_a[3] as f64;
        for _ in 0..=steps {
            if x0 >= 0.0 && y0 >= 0.0 && (x0 as usize) < self.width && (y0 as usize) < self.height {
                self.write_point(
                    (x0 as usize, y0 as usize),
                    zc as f32,
                    [cr as f32, cg as f32, cb as f32, ca as f32],
                );
            }
            x0 += sx; y0 += sy; zc += sz;
            cr += scr; cg += scg; cb += scb; ca += sca;
        }
        let _ = (x1, y1);
    }

    /// Rasterize a triangle with per-vertex attributes (position, normal, UV)
    /// interpolated perspective-correctly, calling a `PixelShader` (SIMD for
    /// 4-pixel batches, scalar for the remainder).
    ///
    /// `depths[i]` is `-1 / view_z_i`, matching `rasterize_triangle`. That's
    /// exactly `1/w` in the OpenGL sense, so attribute A at a pixel is
    /// `Σ(b_i · A_i · depth_i) / Σ(b_i · depth_i)` — perspective correction
    /// with no per-vertex bookkeeping.
    ///
    /// `blend` picks the write path (`Overwrite` for opaque/mask, `SrcOver`
    /// for blend materials — which must be pre-sorted back-to-front by
    /// centroid depth for correct translucency).
    pub fn rasterize_triangle_shaded<S: PixelShader>(
        &mut self,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        zbuf_depths: &[f64; 3],
        positions: &[Vec3; 3],
        normals: &[Vec3; 3],
        uvs: &[[f32; 2]; 3],
        uvs1: &[[f32; 2]; 3],
        uvs2: &[[f32; 2]; 3],
        colors: &[[f32; 4]; 3],
        tangents: &[[f32; 4]; 3],
        blend: BlendMode,
        shader: &S,
    ) {
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let varyings = Varyings::new(depths, zbuf_depths, positions, normals, uvs, uvs1, uvs2, colors, tangents);
        if matches!(blend, BlendMode::WBOIT) && !self.oit_used {
            let n = self.width * self.height;
            self.oit_accum = vec![0.0f32; n * 4];
            self.oit_reveal = vec![1.0f32; n];
            self.oit_used = true;
        }
        let mut sink = ImmediateSink {
            varyings: &varyings,
            shader,
            blend,
            pixels: &mut self.pixels,
            zbuf: &mut self.zbuf,
            oit_accum: &mut self.oit_accum,
            oit_reveal: &mut self.oit_reveal,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
    }

    /// Start a deferred opaque pass for up to `triangles` triangles: until
    /// [`end_deferred`](Self::end_deferred), [`rasterize_depth_id`](Self::rasterize_depth_id)
    /// and [`rasterize_mask_depth_id`](Self::rasterize_mask_depth_id) record which
    /// triangle owns each pixel; [`resolve_owners`](Self::resolve_owners) then
    /// tells which triangles won any pixel at all.
    pub fn begin_deferred(&mut self, triangles: usize) {
        self.vis = vec![0; self.width * self.height];
        self.owned = vec![0; triangles];
    }

    pub fn resolve_owners(&mut self) {
        for &v in &self.vis {
            if v != 0 { self.owned[v as usize - 1] = 1; }
        }
    }

    /// Copy of the depth and ownership buffers mid deferred pass, to restore
    /// with [`restore_depth_state`](Self::restore_depth_state) on a later
    /// render that starts from the same geometry.
    pub fn depth_state(&self) -> (Vec<f32>, Vec<u32>) {
        (self.zbuf.clone(), self.vis.clone())
    }

    /// Overwrite the depth and ownership buffers after `begin_deferred`. Both
    /// slices must match this buffer's pixel count.
    pub fn restore_depth_state(&mut self, zbuf: &[f32], vis: &[u32]) {
        self.zbuf.copy_from_slice(zbuf);
        self.vis.copy_from_slice(vis);
    }

    /// Renumber the owners recorded in the visibility buffer through `map`
    /// (old id → new id) and clear the ownership flags, so a caller can drop
    /// deferred triangles that own no pixel. Call `resolve_owners` again after.
    pub fn remap_owners(&mut self, map: &[u32]) {
        for v in &mut self.vis {
            if *v != 0 { *v = map[*v as usize - 1] + 1; }
        }
        self.owned.iter_mut().for_each(|o| *o = 0);
    }

    /// Clear the owner of every pixel for which `keep(pixel, owner_id)` is
    /// false, so deferred shading leaves those pixels as they are, and clear
    /// the ownership flags. Call `resolve_owners` again after.
    pub fn retain_owners(&mut self, mut keep: impl FnMut(usize, usize) -> bool) {
        for (i, v) in self.vis.iter_mut().enumerate() {
            if *v != 0 && !keep(i, *v as usize - 1) { *v = 0; }
        }
        self.owned.iter_mut().for_each(|o| *o = 0);
    }

    pub fn end_deferred(&mut self) {
        self.vis = Vec::new();
        self.owned = Vec::new();
    }

    /// Depth-only pass for opaque triangle `id`: the same coverage, depth key
    /// and strict z-test as `rasterize_triangle_shaded`, but instead of shading
    /// it records `id` as the owner of every pixel it wins.
    pub fn rasterize_depth_id(&mut self, pts: &[(f64, f64); 3], zbuf_depths: &[f64; 3], id: usize) {
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let zk = [zbuf_depths[0] as f32, zbuf_depths[1] as f32, zbuf_depths[2] as f32];
        let mut sink = DepthSink {
            zk,
            zk_v: [f32x4_splat(zk[0]), f32x4_splat(zk[1]), f32x4_splat(zk[2])],
            owner: id as u32 + 1,
            zbuf: &mut self.zbuf,
            vis: &mut self.vis,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
    }

    /// Depth pass for alpha-masked triangle `id`: like
    /// [`rasterize_depth_id`](Self::rasterize_depth_id), but a fragment only
    /// counts when the shader's alpha test ([`PixelShader::alpha_keep4`]) keeps
    /// it, exactly as immediate shading would have discarded it.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_mask_depth_id<S: PixelShader>(
        &mut self,
        id: usize,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        zbuf_depths: &[f64; 3],
        positions: &[Vec3; 3],
        normals: &[Vec3; 3],
        uvs: &[[f32; 2]; 3],
        uvs1: &[[f32; 2]; 3],
        uvs2: &[[f32; 2]; 3],
        colors: &[[f32; 4]; 3],
        tangents: &[[f32; 4]; 3],
        shader: &S,
    ) {
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let varyings = Varyings::new(depths, zbuf_depths, positions, normals, uvs, uvs1, uvs2, colors, tangents);
        let mut sink = MaskDepthSink { varyings: &varyings, shader, owner: id as u32 + 1, zbuf: &mut self.zbuf, vis: &mut self.vis };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
    }

    pub fn owns_pixels(&self, id: usize) -> bool {
        self.owned.get(id).is_some_and(|&n| n > 0)
    }

    /// Shade the pixels opaque triangle `id` still owns after the depth pass,
    /// with the exact inputs the immediate path would have used for them, packed
    /// four per shader call. Produces the same pixels as shading every triangle
    /// immediately in submission order, running the shader once per visible
    /// pixel instead of once per passing fragment.
    #[allow(clippy::too_many_arguments)]
    pub fn shade_deferred<S: PixelShader>(
        &mut self,
        id: usize,
        pts: &[(f64, f64); 3],
        depths: &[f64; 3],
        zbuf_depths: &[f64; 3],
        positions: &[Vec3; 3],
        normals: &[Vec3; 3],
        uvs: &[[f32; 2]; 3],
        uvs1: &[[f32; 2]; 3],
        uvs2: &[[f32; 2]; 3],
        colors: &[[f32; 4]; 3],
        tangents: &[[f32; 4]; 3],
        shader: &S,
    ) {
        let Some(setup) = TriSetup::new(pts, self.width, self.height) else { return };
        let varyings = Varyings::new(depths, zbuf_depths, positions, normals, uvs, uvs1, uvs2, colors, tangents);
        let mut sink = DeferredSink {
            varyings: &varyings,
            shader,
            owner: id as u32 + 1,
            vis: &self.vis,
            pixels: &mut self.pixels,
            lanes: [[0.0; 20]; 4],
            idx: [0; 4],
            len: 0,
        };
        unsafe { for_each_fragment(&setup, self.width, &mut sink) };
        sink.flush();
    }
}

/// Extract SIMD lanes into scalar pixel writes, gated by `wmask` (the SIMD
/// coverage+depth mask, 1 bit per lane) AND `out.keep` (the shader's per-lane
/// alpha-discard mask). Lanes that fail either don't touch the pixel/z buffers.
#[inline]
unsafe fn write_lane_masked(
    pixels: &mut [u8],
    zbuf: &mut [f32],
    oit_accum: &mut [f32],
    oit_reveal: &mut [f32],
    idx0: usize,
    wmask: u8,
    depth_v: v128,
    out: &ShadeOut4,
    blend: BlendMode,
) {
    let keep_bits = i32x4_bitmask(out.keep);
    let mask = wmask & keep_bits;
    macro_rules! do_lane {
        ($bit:literal, $lane:literal, $off:expr) => {
            if mask & $bit != 0 {
                write_pixel(pixels, zbuf, oit_accum, oit_reveal, idx0 + $off,
                    f32x4_extract_lane::<$lane>(depth_v),
                    [f32x4_extract_lane::<$lane>(out.r), f32x4_extract_lane::<$lane>(out.g),
                     f32x4_extract_lane::<$lane>(out.b), f32x4_extract_lane::<$lane>(out.a)],
                    blend);
            }
        };
    }
    do_lane!(1, 0, 0);
    do_lane!(2, 1, 1);
    do_lane!(4, 2, 2);
    do_lane!(8, 3, 3);
}

/// Common per-pixel write: gamma-encode + write pixel + update z-buffer.
/// Kept as a free function so both the SIMD-lane and scalar-remainder branches
/// share the exact same behaviour without an extra layer of closures.
#[inline]
unsafe fn write_pixel(
    pixels: &mut [u8],
    zbuf: &mut [f32],
    oit_accum: &mut [f32],
    oit_reveal: &mut [f32],
    idx: usize,
    depth: f32,
    rgba: [f32; 4],
    blend: BlendMode,
) {
    let p = pixels.as_mut_ptr().add(idx * 3);
    match blend {
        BlendMode::Overwrite => {
            *p         = crate::color::linear_to_srgb(rgba[0]);
            *p.add(1)  = crate::color::linear_to_srgb(rgba[1]);
            *p.add(2)  = crate::color::linear_to_srgb(rgba[2]);
            *zbuf.get_unchecked_mut(idx) = depth;
        }
        BlendMode::SrcOver => {
            let a = rgba[3].clamp(0.0, 1.0);
            let dr = crate::color::srgb_to_linear(*p);
            let dg = crate::color::srgb_to_linear(*p.add(1));
            let db = crate::color::srgb_to_linear(*p.add(2));
            let r = rgba[0] * a + dr * (1.0 - a);
            let g = rgba[1] * a + dg * (1.0 - a);
            let b = rgba[2] * a + db * (1.0 - a);
            *p        = crate::color::linear_to_srgb(r);
            *p.add(1) = crate::color::linear_to_srgb(g);
            *p.add(2) = crate::color::linear_to_srgb(b);
            *zbuf.get_unchecked_mut(idx) = depth;
        }
        BlendMode::WBOIT => {
            let a = rgba[3].clamp(0.0, 1.0);
            if a <= 0.0 { return; }
            if depth <= *zbuf.get_unchecked(idx) { return; }
            let w = a;
            let ao = idx * 4;
            *oit_accum.get_unchecked_mut(ao)     += rgba[0] * a * w;
            *oit_accum.get_unchecked_mut(ao + 1) += rgba[1] * a * w;
            *oit_accum.get_unchecked_mut(ao + 2) += rgba[2] * a * w;
            *oit_accum.get_unchecked_mut(ao + 3) += a * w;
            *oit_reveal.get_unchecked_mut(idx)   *= 1.0 - a;
        }
    }
}

#[inline(always)]
fn clip_edge(w: f64, dw: f64, inv_dw: f64, min_x: f64, left: &mut f64, right: &mut f64) -> bool {
    if dw.abs() < 1e-12 {
        return w >= -1e-9 || w.is_nan();
    }
    let x_cross = min_x - w * inv_dw;
    if dw > 0.0 {
        if x_cross > *left { *left = x_cross; }
    } else if x_cross < *right {
        *right = x_cross;
    }
    true
}

#[inline(always)]
fn min_max3(a: f64, b: f64, c: f64) -> (f64, f64) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    (if c < lo { c } else { lo }, if c > hi { c } else { hi })
}

/// Signed edge function: positive if P is to the left of edge A→B.
#[inline]
fn edge(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
}

struct TriSetup {
    min_x: usize, max_x: usize,
    min_y: usize, max_y: usize,
    dw0_dx: f64, dw0_dy: f64,
    dw1_dx: f64, dw1_dy: f64,
    dw2_dx: f64, dw2_dy: f64,
    inv_dw0_dx: f64, inv_dw1_dx: f64, inv_dw2_dx: f64,
    row_w0: f64, row_w1: f64, row_w2: f64,
}

impl TriSetup {
    #[inline]
    fn new(pts: &[(f64, f64); 3], width: usize, height: usize) -> Option<Self> {
        let area = edge(pts[0], pts[1], pts[2]);
        if area.abs() < 1e-6 || area.is_nan() { return None; }
        let inv_area = 1.0 / area;

        let (lo_x, hi_x) = min_max3(pts[0].0, pts[1].0, pts[2].0);
        let (lo_y, hi_y) = min_max3(pts[0].1, pts[1].1, pts[2].1);
        let (w1, h1) = ((width - 1) as f64, (height - 1) as f64);
        let min_x = (if lo_x > 0.0 { lo_x } else { 0.0 }) as usize;
        let max_x = ((if hi_x < w1 { hi_x } else { w1 }) as usize).min(width - 1);
        let min_y = (if lo_y > 0.0 { lo_y } else { 0.0 }) as usize;
        let max_y = ((if hi_y < h1 { hi_y } else { h1 }) as usize).min(height - 1);

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

        let inv = |d: f64| if d.abs() < 1e-12 { 0.0 } else { 1.0 / d };
        Some(Self {
            min_x, max_x, min_y, max_y,
            dw0_dx, dw0_dy, dw1_dx, dw1_dy, dw2_dx, dw2_dy,
            inv_dw0_dx: inv(dw0_dx), inv_dw1_dx: inv(dw1_dx), inv_dw2_dx: inv(dw2_dx),
            row_w0, row_w1, row_w2,
        })
    }

    /// Compute conservative scanline X range for the current row weights.
    /// Returns None if the row has no interior pixels.
    #[inline]
    fn scanline(&self, row_w0: f64, row_w1: f64, row_w2: f64) -> Option<(usize, usize)> {
        let min_x = self.min_x as f64;
        let mut left = min_x;
        let mut right = self.max_x as f64;
        if !clip_edge(row_w0, self.dw0_dx, self.inv_dw0_dx, min_x, &mut left, &mut right)
            || !clip_edge(row_w1, self.dw1_dx, self.inv_dw1_dx, min_x, &mut left, &mut right)
            || !clip_edge(row_w2, self.dw2_dx, self.inv_dw2_dx, min_x, &mut left, &mut right)
        {
            return None;
        }

        let xl = (if left - 1.0 > min_x { left - 1.0 } else { min_x }) as usize;
        let xr = (((right + 1.0) as usize).min(self.max_x)).min(self.max_x);
        if xl > xr { return None; }
        Some((xl, xr))
    }
}

struct ScalarIn {
    zbuf_key: f32,
    pos: Vec3,
    normal: Vec3,
    uv: [f32; 2],
    uv1: [f32; 2],
    uv2: [f32; 2],
    color: [f32; 4],
    tan: [f32; 4],
}

struct Varyings {
    d: [f32; 3],
    zk: [f32; 3],
    pd_pos: [(f32, f32, f32); 3],
    pd_n: [(f32, f32, f32); 3],
    pd_uv: [(f32, f32); 3],
    pd_uv1: [(f32, f32); 3],
    pd_uv2: [(f32, f32); 3],
    pd_col: [(f32, f32, f32, f32); 3],
    pd_tan: [(f32, f32, f32); 3],
    tan_w: f32,
    d_v: [v128; 3],
    zk_v: [v128; 3],
    v: [[v128; 3]; 19],
    tan_w_v: v128,
}

impl Varyings {
    #[allow(clippy::too_many_arguments)]
    fn new(
        depths: &[f64; 3], zbuf_depths: &[f64; 3], positions: &[Vec3; 3], normals: &[Vec3; 3],
        uvs: &[[f32; 2]; 3], uvs1: &[[f32; 2]; 3], uvs2: &[[f32; 2]; 3], colors: &[[f32; 4]; 3], tangents: &[[f32; 4]; 3],
    ) -> Self {
        let d = [depths[0] as f32, depths[1] as f32, depths[2] as f32];
        let zk = [zbuf_depths[0] as f32, zbuf_depths[1] as f32, zbuf_depths[2] as f32];
        let pd_pos = [0, 1, 2].map(|i| (positions[i].x as f32 * d[i], positions[i].y as f32 * d[i], positions[i].z as f32 * d[i]));
        let pd_n = [0, 1, 2].map(|i| (normals[i].x as f32 * d[i], normals[i].y as f32 * d[i], normals[i].z as f32 * d[i]));
        let pd_uv = [0, 1, 2].map(|i| (uvs[i][0] * d[i], uvs[i][1] * d[i]));
        let pd_uv1 = [0, 1, 2].map(|i| (uvs1[i][0] * d[i], uvs1[i][1] * d[i]));
        let pd_uv2 = [0, 1, 2].map(|i| (uvs2[i][0] * d[i], uvs2[i][1] * d[i]));
        let pd_col = [0, 1, 2].map(|i| (colors[i][0] * d[i], colors[i][1] * d[i], colors[i][2] * d[i], colors[i][3] * d[i]));
        let pd_tan = [0, 1, 2].map(|i| (tangents[i][0] * d[i], tangents[i][1] * d[i], tangents[i][2] * d[i]));
        let splat3 = |f: &dyn Fn(usize) -> f32| [f32x4_splat(f(0)), f32x4_splat(f(1)), f32x4_splat(f(2))];
        let v = [
            splat3(&|i| pd_pos[i].0), splat3(&|i| pd_pos[i].1), splat3(&|i| pd_pos[i].2),
            splat3(&|i| pd_n[i].0), splat3(&|i| pd_n[i].1), splat3(&|i| pd_n[i].2),
            splat3(&|i| pd_uv[i].0), splat3(&|i| pd_uv[i].1),
            splat3(&|i| pd_uv1[i].0), splat3(&|i| pd_uv1[i].1),
            splat3(&|i| pd_uv2[i].0), splat3(&|i| pd_uv2[i].1),
            splat3(&|i| pd_col[i].0), splat3(&|i| pd_col[i].1), splat3(&|i| pd_col[i].2), splat3(&|i| pd_col[i].3),
            splat3(&|i| pd_tan[i].0), splat3(&|i| pd_tan[i].1), splat3(&|i| pd_tan[i].2),
        ];
        Self {
            d, zk, pd_pos, pd_n, pd_uv, pd_uv1, pd_uv2, pd_col, pd_tan,
            tan_w: tangents[0][3],
            d_v: [f32x4_splat(d[0]), f32x4_splat(d[1]), f32x4_splat(d[2])],
            zk_v: [f32x4_splat(zk[0]), f32x4_splat(zk[1]), f32x4_splat(zk[2])],
            v,
            tan_w_v: f32x4_splat(tangents[0][3]),
        }
    }

    #[inline(always)]
    fn lerp_v(w: [v128; 3], a: &[v128; 3]) -> v128 {
        f32x4_add(f32x4_add(f32x4_mul(w[0], a[0]), f32x4_mul(w[1], a[1])), f32x4_mul(w[2], a[2]))
    }

    #[inline(always)]
    fn depth_v(&self, w: [v128; 3]) -> v128 { Self::lerp_v(w, &self.d_v) }

    #[inline(always)]
    fn zbuf_key_v(&self, w: [v128; 3]) -> v128 { Self::lerp_v(w, &self.zk_v) }

    #[inline(always)]
    fn inputs4(&self, w: [v128; 3], depth_v: v128) -> ShadeIn4 {
        let one = f32x4_splat(1.0);
        let inv_depth_v = f32x4_div(one, depth_v);
        let interp = |a: &[v128; 3]| f32x4_mul(Self::lerp_v(w, a), inv_depth_v);
        let v = &self.v;
        let (nx_raw, ny_raw, nz_raw) = (interp(&v[3]), interp(&v[4]), interp(&v[5]));
        let n_len2 = f32x4_add(f32x4_add(f32x4_mul(nx_raw, nx_raw), f32x4_mul(ny_raw, ny_raw)), f32x4_mul(nz_raw, nz_raw));
        let n_inv_len = f32x4_div(one, f32x4_sqrt(f32x4_max(n_len2, f32x4_splat(1e-14))));
        ShadeIn4 {
            pos_x: interp(&v[0]), pos_y: interp(&v[1]), pos_z: interp(&v[2]),
            n_x: f32x4_mul(nx_raw, n_inv_len), n_y: f32x4_mul(ny_raw, n_inv_len), n_z: f32x4_mul(nz_raw, n_inv_len),
            uv_u: interp(&v[6]), uv_v: interp(&v[7]),
            uv1_u: interp(&v[8]), uv1_v: interp(&v[9]),
            uv2_u: interp(&v[10]), uv2_v: interp(&v[11]),
            col_r: interp(&v[12]), col_g: interp(&v[13]), col_b: interp(&v[14]), col_a: interp(&v[15]),
            tan_x: interp(&v[16]), tan_y: interp(&v[17]), tan_z: interp(&v[18]), tan_w: self.tan_w_v,
        }
    }

    #[inline(always)]
    fn depth_s(&self, w0s: f32, w1s: f32, w2s: f32) -> f32 { w0s * self.d[0] + w1s * self.d[1] + w2s * self.d[2] }

    #[inline(always)]
    fn zbuf_key_s(&self, w0s: f32, w1s: f32, w2s: f32) -> f32 { w0s * self.zk[0] + w1s * self.zk[1] + w2s * self.zk[2] }

    #[inline(always)]
    fn inputs1(&self, w0s: f32, w1s: f32, w2s: f32, zbuf_key: f32) -> ScalarIn {
        let inv_depth = 1.0 / self.depth_s(w0s, w1s, w2s);
        let l = |a: f32, b: f32, c: f32| (w0s * a + w1s * b + w2s * c) * inv_depth;
        let (p, n, uv, uv1, uv2, col, tan) = (&self.pd_pos, &self.pd_n, &self.pd_uv, &self.pd_uv1, &self.pd_uv2, &self.pd_col, &self.pd_tan);
        ScalarIn {
            zbuf_key,
            pos: Vec3::new(l(p[0].0, p[1].0, p[2].0) as f64, l(p[0].1, p[1].1, p[2].1) as f64, l(p[0].2, p[1].2, p[2].2) as f64),
            normal: Vec3::new(l(n[0].0, n[1].0, n[2].0) as f64, l(n[0].1, n[1].1, n[2].1) as f64, l(n[0].2, n[1].2, n[2].2) as f64).normalized(),
            uv: [l(uv[0].0, uv[1].0, uv[2].0), l(uv[0].1, uv[1].1, uv[2].1)],
            uv1: [l(uv1[0].0, uv1[1].0, uv1[2].0), l(uv1[0].1, uv1[1].1, uv1[2].1)],
            uv2: [l(uv2[0].0, uv2[1].0, uv2[2].0), l(uv2[0].1, uv2[1].1, uv2[2].1)],
            color: [l(col[0].0, col[1].0, col[2].0), l(col[0].1, col[1].1, col[2].1), l(col[0].2, col[1].2, col[2].2), l(col[0].3, col[1].3, col[2].3)],
            tan: [l(tan[0].0, tan[1].0, tan[2].0), l(tan[0].1, tan[1].1, tan[2].1), l(tan[0].2, tan[1].2, tan[2].2), self.tan_w],
        }
    }
}

trait FragmentSink {
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128);
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32);
}

#[inline(always)]
unsafe fn for_each_fragment<K: FragmentSink>(setup: &TriSetup, width: usize, sink: &mut K) {
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
            while px + 3 <= xr {
                let inside = v128_and(v128_and(f32x4_ge(w0v, zero), f32x4_ge(w1v, zero)), f32x4_ge(w2v, zero));
                if i32x4_bitmask(inside) != 0 {
                    sink.group(row_base + px, [w0v, w1v, w2v], inside);
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
                    sink.single(row_base + px, w0s, w1s, w2s);
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

struct ImmediateSink<'a, S: PixelShader> {
    varyings: &'a Varyings,
    shader: &'a S,
    blend: BlendMode,
    pixels: &'a mut [u8],
    zbuf: &'a mut [f32],
    oit_accum: &'a mut [f32],
    oit_reveal: &'a mut [f32],
}

impl<S: PixelShader> FragmentSink for ImmediateSink<'_, S> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let depth_v = self.varyings.depth_v(w);
        let zbuf_key_v = self.varyings.zbuf_key_v(w);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let pass = v128_and(inside, f32x4_gt(zbuf_key_v, zbuf_v));
        let wmask = i32x4_bitmask(pass);
        if wmask == 0 { return; }
        let out = self.shader.shade4(self.varyings.inputs4(w, depth_v));
        write_lane_masked(self.pixels, self.zbuf, self.oit_accum, self.oit_reveal, idx0, wmask, zbuf_key_v, &out, self.blend);
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let zbuf_key = self.varyings.zbuf_key_s(w0s, w1s, w2s);
        if zbuf_key > *self.zbuf.get_unchecked(idx) {
            let i = self.varyings.inputs1(w0s, w1s, w2s, zbuf_key);
            if let Some(rgba) = self.shader.shade_scalar(i.pos, i.normal, i.uv, i.uv1, i.uv2, i.color, i.tan) {
                write_pixel(self.pixels, self.zbuf, self.oit_accum, self.oit_reveal, idx, i.zbuf_key, rgba, self.blend);
            }
        }
    }
}

struct DepthSink<'a> {
    zk: [f32; 3],
    zk_v: [v128; 3],
    owner: u32,
    zbuf: &'a mut [f32],
    vis: &'a mut [u32],
}

#[inline(always)]
unsafe fn claim4(zbuf: &mut [f32], vis: &mut [u32], idx0: usize, zbuf_key_v: v128, zbuf_v: v128, pass: v128, owner: u32) {
    v128_store(zbuf.as_mut_ptr().add(idx0) as *mut v128, v128_bitselect(zbuf_key_v, zbuf_v, pass));
    let vp = vis.as_mut_ptr().add(idx0) as *mut v128;
    v128_store(vp, v128_bitselect(i32x4_splat(owner as i32), v128_load(vp as *const v128), pass));
}

impl FragmentSink for DepthSink<'_> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let zbuf_key_v = Varyings::lerp_v(w, &self.zk_v);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let pass = v128_and(inside, f32x4_gt(zbuf_key_v, zbuf_v));
        if i32x4_bitmask(pass) != 0 { claim4(self.zbuf, self.vis, idx0, zbuf_key_v, zbuf_v, pass, self.owner); }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let zbuf_key = w0s * self.zk[0] + w1s * self.zk[1] + w2s * self.zk[2];
        if zbuf_key > *self.zbuf.get_unchecked(idx) {
            *self.zbuf.get_unchecked_mut(idx) = zbuf_key;
            *self.vis.get_unchecked_mut(idx) = self.owner;
        }
    }
}

fn splat_inputs(i: &ScalarIn) -> ShadeIn4 {
    ShadeIn4 {
        pos_x: f32x4_splat(i.pos.x as f32), pos_y: f32x4_splat(i.pos.y as f32), pos_z: f32x4_splat(i.pos.z as f32),
        n_x: f32x4_splat(i.normal.x as f32), n_y: f32x4_splat(i.normal.y as f32), n_z: f32x4_splat(i.normal.z as f32),
        uv_u: f32x4_splat(i.uv[0]), uv_v: f32x4_splat(i.uv[1]),
        uv1_u: f32x4_splat(i.uv1[0]), uv1_v: f32x4_splat(i.uv1[1]),
        uv2_u: f32x4_splat(i.uv2[0]), uv2_v: f32x4_splat(i.uv2[1]),
        col_r: f32x4_splat(i.color[0]), col_g: f32x4_splat(i.color[1]), col_b: f32x4_splat(i.color[2]), col_a: f32x4_splat(i.color[3]),
        tan_x: f32x4_splat(i.tan[0]), tan_y: f32x4_splat(i.tan[1]), tan_z: f32x4_splat(i.tan[2]), tan_w: f32x4_splat(i.tan[3]),
    }
}

struct MaskDepthSink<'a, S: PixelShader> {
    varyings: &'a Varyings,
    shader: &'a S,
    owner: u32,
    zbuf: &'a mut [f32],
    vis: &'a mut [u32],
}

impl<S: PixelShader> FragmentSink for MaskDepthSink<'_, S> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let depth_v = self.varyings.depth_v(w);
        let zbuf_key_v = self.varyings.zbuf_key_v(w);
        let zbuf_v = v128_load(self.zbuf.as_ptr().add(idx0) as *const v128);
        let pass = v128_and(inside, f32x4_gt(zbuf_key_v, zbuf_v));
        if i32x4_bitmask(pass) == 0 { return; }
        let pass = v128_and(pass, self.shader.alpha_keep4(&self.varyings.inputs4(w, depth_v)));
        if i32x4_bitmask(pass) != 0 { claim4(self.zbuf, self.vis, idx0, zbuf_key_v, zbuf_v, pass, self.owner); }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        let zbuf_key = self.varyings.zbuf_key_s(w0s, w1s, w2s);
        if zbuf_key > *self.zbuf.get_unchecked(idx) {
            let keep = self.shader.alpha_keep4(&splat_inputs(&self.varyings.inputs1(w0s, w1s, w2s, zbuf_key)));
            if i32x4_extract_lane::<0>(keep) != 0 {
                *self.zbuf.get_unchecked_mut(idx) = zbuf_key;
                *self.vis.get_unchecked_mut(idx) = self.owner;
            }
        }
    }
}

struct DeferredSink<'a, S: PixelShader> {
    varyings: &'a Varyings,
    shader: &'a S,
    owner: u32,
    vis: &'a [u32],
    pixels: &'a mut [u8],
    lanes: [[f32; 20]; 4],
    idx: [usize; 4],
    len: usize,
}

impl<S: PixelShader> DeferredSink<'_, S> {
    #[inline(always)]
    fn push(&mut self, idx: usize, values: [f32; 20]) {
        self.lanes[self.len] = values;
        self.idx[self.len] = idx;
        self.len += 1;
        if self.len == 4 { self.flush(); }
    }

    fn flush(&mut self) {
        if self.len == 0 { return; }
        for l in self.len..4 { self.lanes[l] = self.lanes[0]; }
        let a = |k: usize| f32x4(self.lanes[0][k], self.lanes[1][k], self.lanes[2][k], self.lanes[3][k]);
        let out = self.shader.shade4(ShadeIn4 {
            pos_x: a(0), pos_y: a(1), pos_z: a(2), n_x: a(3), n_y: a(4), n_z: a(5),
            uv_u: a(6), uv_v: a(7), uv1_u: a(8), uv1_v: a(9), uv2_u: a(10), uv2_v: a(11),
            col_r: a(12), col_g: a(13), col_b: a(14), col_a: a(15),
            tan_x: a(16), tan_y: a(17), tan_z: a(18), tan_w: a(19),
        });
        let mut rgb = [[0.0f32; 4]; 3];
        for (c, v) in [out.r, out.g, out.b].into_iter().enumerate() {
            unsafe { v128_store(rgb[c].as_mut_ptr() as *mut v128, v) };
        }
        for l in 0..self.len {
            let p = self.idx[l] * 3;
            self.pixels[p] = crate::color::linear_to_srgb(rgb[0][l]);
            self.pixels[p + 1] = crate::color::linear_to_srgb(rgb[1][l]);
            self.pixels[p + 2] = crate::color::linear_to_srgb(rgb[2][l]);
        }
        self.len = 0;
    }
}

#[inline(always)]
fn lane_values(i: &ShadeIn4, lane: usize) -> [f32; 20] {
    let mut out = [0.0f32; 20];
    let fields = [i.pos_x, i.pos_y, i.pos_z, i.n_x, i.n_y, i.n_z, i.uv_u, i.uv_v, i.uv1_u, i.uv1_v, i.uv2_u, i.uv2_v,
        i.col_r, i.col_g, i.col_b, i.col_a, i.tan_x, i.tan_y, i.tan_z, i.tan_w];
    for (k, f) in fields.into_iter().enumerate() {
        let mut lanes = [0.0f32; 4];
        unsafe { v128_store(lanes.as_mut_ptr() as *mut v128, f) };
        out[k] = lanes[lane];
    }
    out
}

impl<S: PixelShader> FragmentSink for DeferredSink<'_, S> {
    #[inline(always)]
    unsafe fn group(&mut self, idx0: usize, w: [v128; 3], inside: v128) {
        let inside = i32x4_bitmask(inside);
        let mut mine = 0u8;
        for lane in 0..4 {
            if inside & (1 << lane) != 0 && *self.vis.get_unchecked(idx0 + lane) == self.owner { mine |= 1 << lane; }
        }
        if mine == 0 { return; }
        let inputs = self.varyings.inputs4(w, self.varyings.depth_v(w));
        for lane in 0..4 {
            if mine & (1 << lane) != 0 { self.push(idx0 + lane, lane_values(&inputs, lane)); }
        }
    }

    #[inline(always)]
    unsafe fn single(&mut self, idx: usize, w0s: f32, w1s: f32, w2s: f32) {
        if *self.vis.get_unchecked(idx) != self.owner { return; }
        let i = self.varyings.inputs1(w0s, w1s, w2s, 0.0);
        self.push(idx, [
            i.pos.x as f32, i.pos.y as f32, i.pos.z as f32, i.normal.x as f32, i.normal.y as f32, i.normal.z as f32,
            i.uv[0], i.uv[1], i.uv1[0], i.uv1[1], i.uv2[0], i.uv2[1],
            i.color[0], i.color[1], i.color[2], i.color[3], i.tan[0], i.tan[1], i.tan[2], i.tan[3],
        ]);
    }
}
