/// Software triangle rasterizer with z-buffer.
///
/// Lean port of maquette's rasterizer keeping the SIMD scanline fill and
/// z-test. Deferred to v3 (copy from maquette when needed): Hi-Z tiles,
/// wireframe/shadow-mask rasterization, SSAO, FXAA, downsample-with-coverage.
/// The scanline SIMD core is the piece that took work to get right, so it's
/// preserved verbatim.
///
/// Two rasterization paths:
///   * `rasterize_triangle` — flat color, kept for future wireframe/shadow-mask work.
///   * `rasterize_triangle_shaded` — per-vertex attributes (position, normal, UV)
///     interpolated perspective-correctly. Shader returns `Option<[f32; 4]>`
///     per pixel; `None` discards (mask alpha), `Some` writes with the
///     requested `BlendMode`. SIMD handles coverage + depth test; the shader
///     call itself runs scalar per pixel for now — vectorising the BRDF body
///     is a v3 concern (Cook-Torrance's transcendentals need polynomial
///     approximations to SIMD well).

use crate::math::Vec3;

use std::arch::wasm32::*;
use std::cell::RefCell;

// Reusable scratch buffers for SSAO. Sized on demand each render, but the
// backing allocation is retained across calls — so an animation scrub with
// N frames does one alloc + fill instead of N. Also applies to the harness
// bench loop (--bench=5 sees ~4 fewer 1MB allocations). thread_local rather
// than static-mut so this stays sound if we ever host multi-threaded wasm.
thread_local! {
    static AO_BUFFER:    RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static FLAT_OFFSETS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static Z_BIASES:     RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

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
    fn shade_scalar(&self, pos: Vec3, normal: Vec3, uv: [f32; 2], uv1: [f32; 2], color: [f32; 4], tangent: [f32; 4]) -> Option<[f32; 4]>;
}

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
}

impl PixelBuffer {
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
            if reveal >= 1.0 { continue; } // nothing accumulated here
            let ao = idx * 4;
            let (ar, ag, ab, aa) = unsafe {
                (
                    *self.oit_accum.get_unchecked(ao),
                    *self.oit_accum.get_unchecked(ao + 1),
                    *self.oit_accum.get_unchecked(ao + 2),
                    *self.oit_accum.get_unchecked(ao + 3),
                )
            };
            let inv_a = 1.0 / aa.max(1e-5);
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
    pub fn draw_line(
        &mut self,
        a: (f64, f64),
        b: (f64, f64),
        za: f32,
        zb: f32,
        rgba_a: [f32; 4],
        rgba_b: [f32; 4],
    ) {
        let (mut x0, mut y0) = (a.0, a.1);
        let (mut x1, mut y1) = (b.0, b.1);
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        let steps = dx.max(dy).ceil() as i32;
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
        // Suppress unused warnings.
        let _ = (x1, y1);
    }

    pub fn new(width: usize, height: usize, bg: (u8, u8, u8)) -> Self {
        let n = width * height;
        let pixel = [bg.0, bg.1, bg.2];
        Self {
            width,
            height,
            pixels: pixel.repeat(n),
            zbuf: vec![f32::NEG_INFINITY; n],
            oit_accum: Vec::new(),
            oit_reveal: Vec::new(),
            oit_used: false,
        }
    }

    /// Rasterize a filled triangle with z-buffer depth testing.
    /// Scanline clipping + f32x4 SIMD (4 pixels per iteration).
    ///
    /// Kept for future wireframe / shadow-mask work; the PBR path uses
    /// `rasterize_triangle_shaded` and this method is otherwise unused.
    #[allow(dead_code)]
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
        colors: &[[f32; 4]; 3],
        tangents: &[[f32; 4]; 3],
        blend: BlendMode,
        shader: &S,
    ) {
        let setup = match TriSetup::new(pts, self.width, self.height) {
            Some(s) => s,
            None => return,
        };
        let width = self.width;
        // `depths` is the per-vertex 1/w used for perspective-correct attribute
        // interpolation. `zbuf_depths` is the per-vertex z-buffer key (larger =
        // closer). For perspective they're equal (both -1/v.z). For orthographic
        // they differ: `depths = [1, 1, 1]` (no perspective correction — plain
        // barycentric interp) and `zbuf_depths = [-v.z, -v.z, -v.z]` (linear).
        let d0 = depths[0] as f32;
        let d1 = depths[1] as f32;
        let d2 = depths[2] as f32;
        let zk0 = zbuf_depths[0] as f32;
        let zk1 = zbuf_depths[1] as f32;
        let zk2 = zbuf_depths[2] as f32;

        // Precompute attribute·depth per vertex — the "over-w" values that
        // make perspective-correct interpolation just barycentric math.
        // Stored as f32 scalars first, then splatted to v128 once for the
        // SIMD interior. Also kept as f64 tuples for the scalar remainder,
        // where the extra precision costs nothing measurable.
        let pd_pos = [
            (positions[0].x as f32 * d0, positions[0].y as f32 * d0, positions[0].z as f32 * d0),
            (positions[1].x as f32 * d1, positions[1].y as f32 * d1, positions[1].z as f32 * d1),
            (positions[2].x as f32 * d2, positions[2].y as f32 * d2, positions[2].z as f32 * d2),
        ];
        let pd_n = [
            (normals[0].x as f32 * d0, normals[0].y as f32 * d0, normals[0].z as f32 * d0),
            (normals[1].x as f32 * d1, normals[1].y as f32 * d1, normals[1].z as f32 * d1),
            (normals[2].x as f32 * d2, normals[2].y as f32 * d2, normals[2].z as f32 * d2),
        ];
        let pd_uv = [
            (uvs[0][0] * d0, uvs[0][1] * d0),
            (uvs[1][0] * d1, uvs[1][1] * d1),
            (uvs[2][0] * d2, uvs[2][1] * d2),
        ];
        let pd_uv1 = [
            (uvs1[0][0] * d0, uvs1[0][1] * d0),
            (uvs1[1][0] * d1, uvs1[1][1] * d1),
            (uvs1[2][0] * d2, uvs1[2][1] * d2),
        ];
        let pd_col = [
            (colors[0][0] * d0, colors[0][1] * d0, colors[0][2] * d0, colors[0][3] * d0),
            (colors[1][0] * d1, colors[1][1] * d1, colors[1][2] * d1, colors[1][3] * d1),
            (colors[2][0] * d2, colors[2][1] * d2, colors[2][2] * d2, colors[2][3] * d2),
        ];
        let pd_tan = [
            (tangents[0][0] * d0, tangents[0][1] * d0, tangents[0][2] * d0),
            (tangents[1][0] * d1, tangents[1][1] * d1, tangents[1][2] * d1),
            (tangents[2][0] * d2, tangents[2][1] * d2, tangents[2][2] * d2),
        ];
        // Bitangent handedness is a per-triangle constant; splat once and pass
        // through unchanged (barycentric interp of ±1 is well-defined as ±1
        // only when all three vertices agree, which is our per-triangle case).
        let tan_w_splat = tangents[0][3];

        // Lazy-init OIT buffers on the first WBOIT triangle so opaque-only
        // renders don't pay the memory/init cost. Once initialised the buffers
        // survive for the rest of the render and are composited at the end.
        if matches!(blend, BlendMode::WBOIT) && !self.oit_used {
            let n = self.width * self.height;
            self.oit_accum = vec![0.0f32; n * 4];
            self.oit_reveal = vec![1.0f32; n];
            self.oit_used = true;
        }

        let zbuf = &mut self.zbuf;
        let pixels = &mut self.pixels;
        let oit_accum = &mut self.oit_accum;
        let oit_reveal = &mut self.oit_reveal;

        unsafe {
            let d0v = f32x4_splat(d0);
            let d1v = f32x4_splat(d1);
            let d2v = f32x4_splat(d2);
            let zk0v = f32x4_splat(zk0);
            let zk1v = f32x4_splat(zk1);
            let zk2v = f32x4_splat(zk2);
            let zero = f32x4_splat(0.0);
            let one = f32x4_splat(1.0);

            // Splat per-vertex attribute·depth values once — reused for every
            // 4-pixel batch inside this triangle. 24 f32x4 splats total.
            let pdv_pos_x = [f32x4_splat(pd_pos[0].0), f32x4_splat(pd_pos[1].0), f32x4_splat(pd_pos[2].0)];
            let pdv_pos_y = [f32x4_splat(pd_pos[0].1), f32x4_splat(pd_pos[1].1), f32x4_splat(pd_pos[2].1)];
            let pdv_pos_z = [f32x4_splat(pd_pos[0].2), f32x4_splat(pd_pos[1].2), f32x4_splat(pd_pos[2].2)];
            let pdv_n_x   = [f32x4_splat(pd_n[0].0),   f32x4_splat(pd_n[1].0),   f32x4_splat(pd_n[2].0)];
            let pdv_n_y   = [f32x4_splat(pd_n[0].1),   f32x4_splat(pd_n[1].1),   f32x4_splat(pd_n[2].1)];
            let pdv_n_z   = [f32x4_splat(pd_n[0].2),   f32x4_splat(pd_n[1].2),   f32x4_splat(pd_n[2].2)];
            let pdv_uv_u  = [f32x4_splat(pd_uv[0].0),  f32x4_splat(pd_uv[1].0),  f32x4_splat(pd_uv[2].0)];
            let pdv_uv_v  = [f32x4_splat(pd_uv[0].1),  f32x4_splat(pd_uv[1].1),  f32x4_splat(pd_uv[2].1)];
            let pdv_uv1_u = [f32x4_splat(pd_uv1[0].0), f32x4_splat(pd_uv1[1].0), f32x4_splat(pd_uv1[2].0)];
            let pdv_uv1_v = [f32x4_splat(pd_uv1[0].1), f32x4_splat(pd_uv1[1].1), f32x4_splat(pd_uv1[2].1)];
            let pdv_col_r = [f32x4_splat(pd_col[0].0), f32x4_splat(pd_col[1].0), f32x4_splat(pd_col[2].0)];
            let pdv_col_g = [f32x4_splat(pd_col[0].1), f32x4_splat(pd_col[1].1), f32x4_splat(pd_col[2].1)];
            let pdv_col_b = [f32x4_splat(pd_col[0].2), f32x4_splat(pd_col[1].2), f32x4_splat(pd_col[2].2)];
            let pdv_col_a = [f32x4_splat(pd_col[0].3), f32x4_splat(pd_col[1].3), f32x4_splat(pd_col[2].3)];
            let pdv_tan_x = [f32x4_splat(pd_tan[0].0), f32x4_splat(pd_tan[1].0), f32x4_splat(pd_tan[2].0)];
            let pdv_tan_y = [f32x4_splat(pd_tan[0].1), f32x4_splat(pd_tan[1].1), f32x4_splat(pd_tan[2].1)];
            let pdv_tan_z = [f32x4_splat(pd_tan[0].2), f32x4_splat(pd_tan[1].2), f32x4_splat(pd_tan[2].2)];
            let tan_w_v   = f32x4_splat(tan_w_splat);

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
                            // Perspective-interp weight (`Σ w_i · d_i`, where
                            // d_i is 1/w for perspective, 1 for ortho).
                            let depth_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, d0v), f32x4_mul(w1v, d1v)), f32x4_mul(w2v, d2v));
                            // Z-buffer key (`Σ w_i · zk_i`). For perspective
                            // this equals `depth_v` and LLVM folds the compute;
                            // for ortho zk_i = -v.z (linear depth).
                            let zbuf_key_v = f32x4_add(f32x4_add(
                                f32x4_mul(w0v, zk0v), f32x4_mul(w1v, zk1v)), f32x4_mul(w2v, zk2v));
                            let idx0 = row_base + px;
                            let zbuf_v = v128_load(zbuf.as_ptr().add(idx0) as *const v128);
                            let pass = v128_and(inside, f32x4_gt(zbuf_key_v, zbuf_v));
                            let wmask = i32x4_bitmask(pass);

                            if wmask != 0 {
                                let inv_depth_v = f32x4_div(one, depth_v);

                                // Perspective-correct barycentric interp of each attribute:
                                //   A = Σ(w_i · A_i · depth_i) / Σ(w_i · depth_i)
                                //     = Σ(w_i · pd_i) · inv_depth
                                let interp = |a: &[v128; 3]| -> v128 {
                                    let num = f32x4_add(
                                        f32x4_add(f32x4_mul(w0v, a[0]), f32x4_mul(w1v, a[1])),
                                        f32x4_mul(w2v, a[2]),
                                    );
                                    f32x4_mul(num, inv_depth_v)
                                };
                                let pos_x = interp(&pdv_pos_x);
                                let pos_y = interp(&pdv_pos_y);
                                let pos_z = interp(&pdv_pos_z);
                                let nx_raw = interp(&pdv_n_x);
                                let ny_raw = interp(&pdv_n_y);
                                let nz_raw = interp(&pdv_n_z);
                                let uv_u  = interp(&pdv_uv_u);
                                let uv_v  = interp(&pdv_uv_v);
                                let uv1_u = interp(&pdv_uv1_u);
                                let uv1_v = interp(&pdv_uv1_v);
                                let col_r = interp(&pdv_col_r);
                                let col_g = interp(&pdv_col_g);
                                let col_b = interp(&pdv_col_b);
                                let col_a = interp(&pdv_col_a);
                                let tan_x = interp(&pdv_tan_x);
                                let tan_y = interp(&pdv_tan_y);
                                let tan_z = interp(&pdv_tan_z);

                                // Normalize the interpolated normal in SIMD.
                                let n_len2 = f32x4_add(
                                    f32x4_add(f32x4_mul(nx_raw, nx_raw), f32x4_mul(ny_raw, ny_raw)),
                                    f32x4_mul(nz_raw, nz_raw),
                                );
                                let n_inv_len = f32x4_div(one, f32x4_sqrt(f32x4_max(n_len2, f32x4_splat(1e-14))));
                                let n_x = f32x4_mul(nx_raw, n_inv_len);
                                let n_y = f32x4_mul(ny_raw, n_inv_len);
                                let n_z = f32x4_mul(nz_raw, n_inv_len);

                                let out = shader.shade4(ShadeIn4 {
                                    pos_x, pos_y, pos_z,
                                    n_x, n_y, n_z,
                                    uv_u, uv_v,
                                    uv1_u, uv1_v,
                                    col_r, col_g, col_b, col_a,
                                    tan_x, tan_y, tan_z, tan_w: tan_w_v,
                                });

                                write_lane_masked(pixels, zbuf, oit_accum, oit_reveal, idx0, wmask, zbuf_key_v, &out, blend);
                            }
                        }

                        w0v = f32x4_add(w0v, dw0_dx4);
                        w1v = f32x4_add(w1v, dw1_dx4);
                        w2v = f32x4_add(w2v, dw2_dx4);
                        px += 4;
                    }

                    // Scalar remainder — shade one at a time via shade_scalar.
                    let mut w0s = f32x4_extract_lane::<0>(w0v);
                    let mut w1s = f32x4_extract_lane::<0>(w1v);
                    let mut w2s = f32x4_extract_lane::<0>(w2v);
                    while px <= xr {
                        if w0s >= 0.0 && w1s >= 0.0 && w2s >= 0.0 {
                            let depth = w0s * d0 + w1s * d1 + w2s * d2;
                            let zbuf_key = w0s * zk0 + w1s * zk1 + w2s * zk2;
                            let idx = row_base + px;
                            if zbuf_key > *zbuf.get_unchecked(idx) {
                                let inv_depth = 1.0 / depth;
                                let pos = Vec3::new(
                                    ((w0s * pd_pos[0].0 + w1s * pd_pos[1].0 + w2s * pd_pos[2].0) * inv_depth) as f64,
                                    ((w0s * pd_pos[0].1 + w1s * pd_pos[1].1 + w2s * pd_pos[2].1) * inv_depth) as f64,
                                    ((w0s * pd_pos[0].2 + w1s * pd_pos[1].2 + w2s * pd_pos[2].2) * inv_depth) as f64,
                                );
                                let n_raw = Vec3::new(
                                    ((w0s * pd_n[0].0 + w1s * pd_n[1].0 + w2s * pd_n[2].0) * inv_depth) as f64,
                                    ((w0s * pd_n[0].1 + w1s * pd_n[1].1 + w2s * pd_n[2].1) * inv_depth) as f64,
                                    ((w0s * pd_n[0].2 + w1s * pd_n[1].2 + w2s * pd_n[2].2) * inv_depth) as f64,
                                );
                                let normal = n_raw.normalized();
                                let uv = [
                                    (w0s * pd_uv[0].0 + w1s * pd_uv[1].0 + w2s * pd_uv[2].0) * inv_depth,
                                    (w0s * pd_uv[0].1 + w1s * pd_uv[1].1 + w2s * pd_uv[2].1) * inv_depth,
                                ];
                                let uv1 = [
                                    (w0s * pd_uv1[0].0 + w1s * pd_uv1[1].0 + w2s * pd_uv1[2].0) * inv_depth,
                                    (w0s * pd_uv1[0].1 + w1s * pd_uv1[1].1 + w2s * pd_uv1[2].1) * inv_depth,
                                ];
                                let color = [
                                    (w0s * pd_col[0].0 + w1s * pd_col[1].0 + w2s * pd_col[2].0) * inv_depth,
                                    (w0s * pd_col[0].1 + w1s * pd_col[1].1 + w2s * pd_col[2].1) * inv_depth,
                                    (w0s * pd_col[0].2 + w1s * pd_col[1].2 + w2s * pd_col[2].2) * inv_depth,
                                    (w0s * pd_col[0].3 + w1s * pd_col[1].3 + w2s * pd_col[2].3) * inv_depth,
                                ];
                                let tan = [
                                    (w0s * pd_tan[0].0 + w1s * pd_tan[1].0 + w2s * pd_tan[2].0) * inv_depth,
                                    (w0s * pd_tan[0].1 + w1s * pd_tan[1].1 + w2s * pd_tan[2].1) * inv_depth,
                                    (w0s * pd_tan[0].2 + w1s * pd_tan[1].2 + w2s * pd_tan[2].2) * inv_depth,
                                    tan_w_splat,
                                ];
                                if let Some(rgba) = shader.shade_scalar(pos, normal, uv, uv1, color, tan) {
                                    write_pixel(pixels, zbuf, oit_accum, oit_reveal, idx, zbuf_key as f32, rgba, blend);
                                }
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

    /// Screen-Space Ambient Occlusion. Modulates the RGB pixel buffer by a
    /// per-pixel AO term derived from the depth buffer + bilateral blur.
    /// Ported from maquette (SSAO is format-agnostic — depth+RGB in, RGB out).
    pub fn apply_ssao(&mut self, params: &crate::ssao::SSAOParams) {
        let w = self.width;
        let h = self.height;
        let w_i32 = w as i32;
        let h_i32 = h as i32;

        let mut zmin = f32::MAX;
        let mut zmax = f32::MIN;
        for &d in &self.zbuf {
            if d != f32::NEG_INFINITY {
                if d < zmin { zmin = d; }
                if d > zmax { zmax = d; }
            }
        }
        let depth_range = (zmax - zmin).max(0.001);

        let radius_px = (params.radius * w.min(h) as f64) as f32;
        let bias_scaled = params.bias as f32 * depth_range;
        let strength = params.strength as f32;
        let offsets = crate::ssao::precompute_sample_offsets(params.samples, radius_px, bias_scaled);

        let num_samples = offsets[0].len();
        let batches = num_samples / 4;
        let n_rot = crate::ssao::NOISE_ROTATIONS;
        // Take ownership of the reusable buffers out of the thread-locals for
        // the duration of this render, then put them back at the end. Keeps
        // the backing allocation alive across renders (saves ~1 MB alloc + fill
        // per 512×512 render for `ao_buffer`, plus a couple of small ones for
        // the offset tables). Safe: apply_ssao has no early returns / `?`, so
        // the swap-back at the bottom always runs.
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
        let zbuf_ptr = self.zbuf.as_ptr();

        let mut ao_buffer = AO_BUFFER.with(|c| std::mem::take(&mut *c.borrow_mut()));
        ao_buffer.clear();
        ao_buffer.resize(w * h, 1.0);

        macro_rules! ssao_scalar_pixel {
            ($x:expr, $y:expr, $idx:expr) => {
                let depth = unsafe { *self.zbuf.get_unchecked($idx) };
                if depth != f32::NEG_INFINITY {
                    let pattern = &offsets[crate::ssao::noise_index($x, $y)];
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
                    let depth = unsafe { *self.zbuf.get_unchecked(idx) };
                    if depth == f32::NEG_INFINITY { continue; }
                    let pi = crate::ssao::noise_index(x, y);
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
                        unsafe { *ao_buffer.get_unchecked_mut(idx) =
                            (1.0 - (occluded as f32 / valid as f32 * strength).min(1.0)).max(0.0) };
                    }
                }
                for x in interior_x_end..w_i32 {
                    let idx = row + x as usize;
                    ssao_scalar_pixel!(x, y, idx);
                }
            }
        }

        let blurred = crate::ssao::bilateral_blur_separable(&ao_buffer, &self.zbuf, w, h, 4);

        // Apply AO by darkening pixels.
        unsafe {
            for i in 0..w * h {
                let ao = *blurred.get_unchecked(i);
                let p = self.pixels.as_mut_ptr().add(i * 3);
                *p        = (*p        as f32 * ao + 0.5) as u8;
                *p.add(1) = (*p.add(1) as f32 * ao + 0.5) as u8;
                *p.add(2) = (*p.add(2) as f32 * ao + 0.5) as u8;
            }
        }

        // Return the scratch Vecs to the thread-locals so the next call reuses
        // the backing allocations. `blurred` is a fresh Vec — dropped here.
        AO_BUFFER   .with(|c| *c.borrow_mut() = ao_buffer);
        FLAT_OFFSETS.with(|c| *c.borrow_mut() = flat_offsets);
        Z_BIASES    .with(|c| *c.borrow_mut() = z_biases);
    }

    /// Downsample by an integer factor via box filter — SIMD-accelerated for
    /// factor=2 and factor=4 (two passes of 2×). Used for SSAA. Copied
    /// verbatim from maquette (format-agnostic).
    pub fn downsample(&self, factor: usize) -> Self {
        if factor <= 1 { return Self {
            width: self.width, height: self.height,
            pixels: self.pixels.clone(),
            zbuf: self.zbuf.clone(),
            // OIT buffers are consumed by `composite_oit` before downsample —
            // downsampled buffer never needs its own.
            oit_accum: Vec::new(), oit_reveal: Vec::new(), oit_used: false,
        }; }
        if factor == 2 { return self.downsample_2x(); }
        if factor == 4 { return self.downsample_2x().downsample_2x(); }
        // Generic scalar path for other factors.
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
        // Rebuild a partial z-buffer via the same box average — matters for
        // the transparent-background path (pixels with zbuf == −∞ are treated
        // as background). Any lane hit becomes non-∞.
        let mut zbuf = vec![f32::NEG_INFINITY; nw * nh];
        for ny in 0..nh {
            for nx in 0..nw {
                let mut z = f32::NEG_INFINITY;
                for sy in 0..factor {
                    for sx in 0..factor {
                        let si = (ny * factor + sy) * src_w + nx * factor + sx;
                        let sz = self.zbuf[si];
                        if sz != f32::NEG_INFINITY && sz > z { z = sz; }
                    }
                }
                zbuf[ny * nw + nx] = z;
            }
        }
        Self { width: nw, height: nh, pixels, zbuf,
            oit_accum: Vec::new(), oit_reveal: Vec::new(), oit_used: false }
    }

    /// SIMD 2× downsample: processes 4 output pixels per iteration using
    /// byte shuffles to deinterleave RGB, u16 widening for accumulation,
    /// and re-interleave for output. Ported verbatim from maquette.
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
                    nx += 1;
                }
            }
        }

        // Coverage-preserving zbuf: any covered subpixel means output is
        // covered. Use max (front-most in our +z=closer convention).
        let mut zbuf = vec![f32::NEG_INFINITY; nw * nh];
        for ny in 0..nh {
            for nx in 0..nw {
                let mut z = f32::NEG_INFINITY;
                let s0 = ny * 2 * self.width + nx * 2;
                for off in [0, 1, self.width, self.width + 1] {
                    let sz = self.zbuf[s0 + off];
                    if sz != f32::NEG_INFINITY && sz > z { z = sz; }
                }
                zbuf[ny * nw + nx] = z;
            }
        }

        Self { width: nw, height: nh, pixels: out, zbuf,
            oit_accum: Vec::new(), oit_reveal: Vec::new(), oit_used: false }
    }

    /// Expand opaque RGB → straight RGBA8 (alpha = 255). Vectorised: each
    /// iteration reads 16 bytes (4 RGB pixels + 4 padding bytes shuffled
    /// out), interleaves 0xFF alphas via `i8x16_shuffle`, and writes 16
    /// bytes of RGBA. The tail is handled scalar to avoid a 16-byte read
    /// past `pixels.len()`.
    pub fn to_rgba8(&self) -> (u32, u32, Vec<u8>) {
        let n = self.width * self.height;
        let mut rgba = vec![0u8; n * 4];
        let src = self.pixels.as_ptr();
        let dst = rgba.as_mut_ptr();
        // A batch of 4 pixels reads `[p*3 .. p*3 + 16)` and writes 16 bytes.
        // Safe while `p*3 + 16 <= 3n`, i.e. `p <= n - 6` (integer).
        let simd_end_p = if n >= 6 { (n - 5) & !3 } else { 0 };
        unsafe {
            let ff = u8x16_splat(0xff);
            let mut p = 0usize;
            while p < simd_end_p {
                let v = v128_load(src.add(p * 3) as *const v128);
                // Lane picks: [R0,G0,B0, α, R1,G1,B1, α, R2,G2,B2, α, R3,G3,B3, α]
                // 0-15 come from `v`; 16 comes from `ff` (splat, so any lane of
                // `ff` is 0xFF — pick 16).
                let out = i8x16_shuffle::<
                    0, 1, 2, 16,
                    3, 4, 5, 16,
                    6, 7, 8, 16,
                    9, 10, 11, 16,
                >(v, ff);
                v128_store(dst.add(p * 4) as *mut v128, out);
                p += 4;
            }
            // Tail: remaining `n - p` pixels scalar.
            while p < n {
                *dst.add(p * 4)     = *src.add(p * 3);
                *dst.add(p * 4 + 1) = *src.add(p * 3 + 1);
                *dst.add(p * 4 + 2) = *src.add(p * 3 + 2);
                *dst.add(p * 4 + 3) = 0xff;
                p += 1;
            }
        }
        (self.width as u32, self.height as u32, rgba)
    }

    /// RGBA8 where pixels never touched by the rasterizer (zbuf = −∞) become
    /// fully transparent. Used when the config wants a transparent background.
    pub fn to_rgba8_transparent(&self) -> (u32, u32, Vec<u8>) {
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
            // WBOIT accumulate: (rgb·a·w, a·w) into accum, (1−a) into reveal.
            // Do NOT touch the pixel buffer or z-buffer — the opaque render is
            // still visible where the translucents get composited later.
            // Weight `w = a`: simple form that works well for non-interpenetrating
            // translucents. Can be swapped for a depth-weighted variant later
            // if needed.
            let a = rgba[3].clamp(0.0, 1.0);
            if a <= 0.0 { return; }
            // Depth-test against the opaque z-buffer: skip fragments occluded by
            // opaque geometry (per WBOIT spec — translucents behind opaque are
            // never seen). `depth` is the interp weight in perspective mode; the
            // opaque zbuf stores the same units, so `>` means "closer".
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

// ---------------------------------------------------------------------------
// Triangle setup for scanline + SIMD rasterization
// ---------------------------------------------------------------------------

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

        let inv = |d: f64| if d.abs() < 1e-12 { 0.0 } else { 1.0 / d };
        Some(Self {
            min_x, max_x, min_y, max_y,
            dw0_dx, dw0_dy, dw1_dx, dw1_dy, dw2_dx, dw2_dy,
            inv_dw0_dx: inv(dw0_dx), inv_dw1_dx: inv(dw1_dx), inv_dw2_dx: inv(dw2_dx),
            row_w0, row_w1, row_w2,
        })
    }

    #[inline]
    fn scanline(&self, row_w0: f64, row_w1: f64, row_w2: f64) -> Option<(usize, usize)> {
        let mut left = self.min_x as f64;
        let mut right = self.max_x as f64;

        for &(w, dw, inv_dw) in &[
            (row_w0, self.dw0_dx, self.inv_dw0_dx),
            (row_w1, self.dw1_dx, self.inv_dw1_dx),
            (row_w2, self.dw2_dx, self.inv_dw2_dx),
        ] {
            if dw.abs() < 1e-12 {
                if w < -1e-9 { return None; }
            } else {
                let x_cross = self.min_x as f64 - w * inv_dw;
                if dw > 0.0 {
                    left = left.max(x_cross);
                } else {
                    right = right.min(x_cross);
                }
            }
        }

        let xl = ((left - 1.0).max(self.min_x as f64)) as usize;
        let xr = (((right + 1.0) as usize).min(self.max_x)).min(self.max_x);
        if xl > xr { return None; }
        Some((xl, xr))
    }
}
