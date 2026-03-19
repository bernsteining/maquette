// ---------------------------------------------------------------------------
// Lighting and shading pipeline
// ---------------------------------------------------------------------------

use crate::config::{LightKind, RenderConfig};
use crate::color::linear_to_srgb;
use crate::math::Vec3;
use std::arch::wasm32::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ShadingMode { BlinnPhong, Gooch, Cel, Flat, Normal }

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ToneMapMethod { None, Reinhard, Aces }

pub(crate) struct ResolvedLight {
    pub(crate) kind: LightKind,
    pub(crate) vector: Vec3,
    pub(crate) color: (f32, f32, f32),
}

#[derive(Clone, Copy)]
pub(crate) struct LightF32 {
    pub(crate) kind: LightKind,
    pub(crate) dx: f32, pub(crate) dy: f32, pub(crate) dz: f32,
    pub(crate) cr: f32, pub(crate) cg: f32, pub(crate) cb: f32,
}

// ---------------------------------------------------------------------------
// Light resolution
// ---------------------------------------------------------------------------

pub(crate) fn resolve_lights(config: &RenderConfig) -> Vec<ResolvedLight> {
    if config.lights.is_empty() {
        vec![ResolvedLight {
            kind: LightKind::Directional,
            vector: Vec3::new(config.light_dir[0], config.light_dir[1], config.light_dir[2]).normalized(),
            color: (1.0f32, 1.0f32, 1.0f32),
        }]
    } else {
        let lights = &config.lights;
        lights.iter().map(|l| {
            let intensity = l.intensity as f32;
            ResolvedLight {
                kind: l.kind,
                vector: if l.kind == LightKind::Directional {
                    Vec3::new(l.vector[0], l.vector[1], l.vector[2]).normalized()
                } else {
                    Vec3::new(l.vector[0], l.vector[1], l.vector[2])
                },
                color: (l.color.0 * intensity, l.color.1 * intensity, l.color.2 * intensity),
            }
        }).collect()
    }
}

pub(crate) fn shadow_light_dir(config: &RenderConfig) -> Vec3 {
    if config.lights.is_empty() {
        Vec3::new(config.light_dir[0], config.light_dir[1], config.light_dir[2]).normalized()
    } else {
        config.lights.iter()
            .find(|l| l.kind == LightKind::Directional)
            .map(|l| Vec3::new(l.vector[0], l.vector[1], l.vector[2]).normalized())
            .unwrap_or(Vec3::new(0.0, 0.0, 1.0))
    }
}

// ---------------------------------------------------------------------------
// Scalar shading helpers
// ---------------------------------------------------------------------------

#[inline(always)]
pub(crate) fn get_light_dir(light: &LightF32, wpx: f32, wpy: f32, wpz: f32) -> (f32, f32, f32) {
    if light.kind == LightKind::Directional {
        (light.dx, light.dy, light.dz)
    } else {
        let dx = light.dx - wpx;
        let dy = light.dy - wpy;
        let dz = light.dz - wpz;
        let inv = 1.0f32 / (dx * dx + dy * dy + dz * dz).sqrt();
        (dx * inv, dy * inv, dz * inv)
    }
}

#[inline(always)]
pub(crate) fn specular_contrib(
    ldx: f32, ldy: f32, ldz: f32,
    vdx: f32, vdy: f32, vdz: f32,
    onx: f32, ony: f32, onz: f32,
    spec_lut: &[f32; 256], specular: f32,
) -> f32 {
    let hx = ldx + vdx;
    let hy = ldy + vdy;
    let hz = ldz + vdz;
    let ndoth_unnorm = onx * hx + ony * hy + onz * hz;
    if ndoth_unnorm > 0.0 {
        let h_inv = 1.0f32 / (hx * hx + hy * hy + hz * hz).sqrt();
        let ndoth = (ndoth_unnorm * h_inv).min(1.0);
        spec_lut[(ndoth * 255.0) as usize] * specular
    } else {
        0.0
    }
}

#[inline(always)]
pub(crate) fn finalize_color(
    lr: f32, lg: f32, lb: f32,
    ambient: (f32, f32, f32),
    diff: (f32, f32, f32),
    spec: (f32, f32, f32),
    rim: f32,
    gamma_correction: bool,
    tm: ToneMapMethod,
    exposure: f32,
) -> (u8, u8, u8) {
    let (hr, hg, hb) = tone_map(
        lr * (ambient.0 + diff.0) + spec.0 + rim,
        lg * (ambient.1 + diff.1) + spec.1 + rim,
        lb * (ambient.2 + diff.2) + spec.2 + rim,
        tm, exposure,
    );
    if gamma_correction {
        (linear_to_srgb(hr), linear_to_srgb(hg), linear_to_srgb(hb))
    } else {
        (
            (hr * 255.0).min(255.0).round() as u8,
            (hg * 255.0).min(255.0).round() as u8,
            (hb * 255.0).min(255.0).round() as u8,
        )
    }
}

// ---------------------------------------------------------------------------
// SIMD batched shade_point: 4 vertices per iteration
// ---------------------------------------------------------------------------

/// Helper: tone-map a single f32x4 channel (4 vertices' worth of one channel).
#[inline(always)]
fn tone_map_4(v: v128, method: ToneMapMethod, exp4: v128) -> v128 {
    match method {
        ToneMapMethod::None => v,
        ToneMapMethod::Reinhard => {
            let ve = f32x4_mul(v, exp4);
            f32x4_div(ve, f32x4_add(f32x4_splat(1.0), ve))
        }
        _ => { // ACES
            let ve = f32x4_mul(v, exp4);
            let a = f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.51), ve), f32x4_splat(0.03)));
            let b = f32x4_add(f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.43), ve), f32x4_splat(0.59))), f32x4_splat(0.14));
            f32x4_min(f32x4_max(f32x4_div(a, b), f32x4_splat(0.0)), f32x4_splat(1.0))
        }
    }
}

/// Helper: extract 4 f32 lanes, look up in a 256-entry LUT (index = val * 255),
/// return results as f32x4.
#[inline(always)]
fn lut_lookup_4(vals: v128, lut: &[f32; 256]) -> v128 {
    let i0 = unsafe { *lut.get_unchecked((f32x4_extract_lane::<0>(vals) * 255.0) as usize) };
    let i1 = unsafe { *lut.get_unchecked((f32x4_extract_lane::<1>(vals) * 255.0) as usize) };
    let i2 = unsafe { *lut.get_unchecked((f32x4_extract_lane::<2>(vals) * 255.0) as usize) };
    let i3 = unsafe { *lut.get_unchecked((f32x4_extract_lane::<3>(vals) * 255.0) as usize) };
    f32x4_replace_lane::<3>(f32x4_replace_lane::<2>(f32x4_replace_lane::<1>(f32x4_splat(i0), i1), i2), i3)
}

#[inline(never)]
pub(crate) fn shade_batch_4(
    nx4: v128, ny4: v128, nz4: v128,
    px4: v128, py4: v128, pz4: v128,
    base_lin_r: f32, base_lin_g: f32, base_lin_b: f32,
    lights: &[LightF32],
    cam_x: f32, cam_y: f32, cam_z: f32,
    sky_r: f32, sky_g: f32, sky_b: f32,
    gnd_r: f32, gnd_g: f32, gnd_b: f32,
    up_x: f32, up_y: f32, up_z: f32,
    one_minus_ambient: f32, specular: f32, fresnel: f32,
    gamma_correction: bool,
    tm: ToneMapMethod, exposure: f32,
    spec_lut: &[f32; 256], fresnel_lut: &[f32; 256],
    sss_intensity: f32, sss_distortion: f32, sss_lut: &[f32; 256],
    cel_bands: usize,
    is_gooch: bool, gooch_warm: (f32, f32, f32), gooch_cool: (f32, f32, f32),
) -> [(u8, u8, u8); 4] {
    let zero = f32x4_splat(0.0);
    let one = f32x4_splat(1.0);
    let eps = f32x4_splat(1e-10);
    let is_cel = cel_bands > 0;

    // View direction: cam - pos, normalized
    let vdx = f32x4_sub(f32x4_splat(cam_x), px4);
    let vdy = f32x4_sub(f32x4_splat(cam_y), py4);
    let vdz = f32x4_sub(f32x4_splat(cam_z), pz4);
    let v_len_sq = f32x4_add(f32x4_add(f32x4_mul(vdx, vdx), f32x4_mul(vdy, vdy)), f32x4_mul(vdz, vdz));
    let v_inv = f32x4_div(one, f32x4_sqrt(f32x4_add(v_len_sq, eps)));
    let vdx = f32x4_mul(vdx, v_inv);
    let vdy = f32x4_mul(vdy, v_inv);
    let vdz = f32x4_mul(vdz, v_inv);

    // Hemisphere ambient: t = (dot(n, up) + 1) * 0.5; lerp ground→sky
    let t = f32x4_mul(f32x4_add(f32x4_add(f32x4_add(
        f32x4_mul(nx4, f32x4_splat(up_x)),
        f32x4_mul(ny4, f32x4_splat(up_y))),
        f32x4_mul(nz4, f32x4_splat(up_z))),
        one), f32x4_splat(0.5));
    let amb_r = f32x4_add(f32x4_splat(gnd_r), f32x4_mul(f32x4_splat(sky_r - gnd_r), t));
    let amb_g = f32x4_add(f32x4_splat(gnd_g), f32x4_mul(f32x4_splat(sky_g - gnd_g), t));
    let amb_b = f32x4_add(f32x4_splat(gnd_b), f32x4_mul(f32x4_splat(sky_b - gnd_b), t));

    // n_dot_v and orient normal
    let n_dot_v = f32x4_add(f32x4_add(f32x4_mul(nx4, vdx), f32x4_mul(ny4, vdy)), f32x4_mul(nz4, vdz));
    let neg_mask = f32x4_lt(n_dot_v, zero);
    let onx = v128_bitselect(f32x4_neg(nx4), nx4, neg_mask);
    let ony = v128_bitselect(f32x4_neg(ny4), ny4, neg_mask);
    let onz = v128_bitselect(f32x4_neg(nz4), nz4, neg_mask);

    // --- Gooch early path: single light, cool/warm lerp, monochromatic specular ---
    if is_gooch {
        let light = &lights[0];
        let (ldx, ldy, ldz) = if light.kind == LightKind::Directional {
            (f32x4_splat(light.dx), f32x4_splat(light.dy), f32x4_splat(light.dz))
        } else {
            let dx = f32x4_sub(f32x4_splat(light.dx), px4);
            let dy = f32x4_sub(f32x4_splat(light.dy), py4);
            let dz = f32x4_sub(f32x4_splat(light.dz), pz4);
            let inv = f32x4_div(one, f32x4_sqrt(f32x4_add(f32x4_add(
                f32x4_mul(dx, dx), f32x4_mul(dy, dy)), f32x4_add(f32x4_mul(dz, dz), eps))));
            (f32x4_mul(dx, inv), f32x4_mul(dy, inv), f32x4_mul(dz, inv))
        };

        // Signed ndotl (original normal, not oriented)
        let ndotl = f32x4_add(f32x4_add(
            f32x4_mul(nx4, ldx), f32x4_mul(ny4, ldy)), f32x4_mul(nz4, ldz));
        let t4 = f32x4_mul(f32x4_add(ndotl, one), f32x4_splat(0.5));

        // cool = gooch_cool * 0.5 + base_lin * 0.5; warm = gooch_warm * 0.5 + base_lin * 0.5
        let half = f32x4_splat(0.5);
        let lr4 = f32x4_splat(base_lin_r);
        let lg4 = f32x4_splat(base_lin_g);
        let lb4 = f32x4_splat(base_lin_b);
        let cool_r = f32x4_add(f32x4_mul(f32x4_splat(gooch_cool.0), half), f32x4_mul(lr4, half));
        let cool_g = f32x4_add(f32x4_mul(f32x4_splat(gooch_cool.1), half), f32x4_mul(lg4, half));
        let cool_b = f32x4_add(f32x4_mul(f32x4_splat(gooch_cool.2), half), f32x4_mul(lb4, half));
        let warm_r = f32x4_add(f32x4_mul(f32x4_splat(gooch_warm.0), half), f32x4_mul(lr4, half));
        let warm_g = f32x4_add(f32x4_mul(f32x4_splat(gooch_warm.1), half), f32x4_mul(lg4, half));
        let warm_b = f32x4_add(f32x4_mul(f32x4_splat(gooch_warm.2), half), f32x4_mul(lb4, half));

        let mut hr = f32x4_add(cool_r, f32x4_mul(t4, f32x4_sub(warm_r, cool_r)));
        let mut hg = f32x4_add(cool_g, f32x4_mul(t4, f32x4_sub(warm_g, cool_g)));
        let mut hb = f32x4_add(cool_b, f32x4_mul(t4, f32x4_sub(warm_b, cool_b)));

        // Monochromatic specular (oriented normal)
        if specular > 0.0 {
            let hx = f32x4_add(ldx, vdx);
            let hy = f32x4_add(ldy, vdy);
            let hz = f32x4_add(ldz, vdz);
            let ndoth_unnorm = f32x4_add(f32x4_add(
                f32x4_mul(onx, hx), f32x4_mul(ony, hy)), f32x4_mul(onz, hz));
            let h_len_sq = f32x4_add(f32x4_add(f32x4_mul(hx, hx), f32x4_mul(hy, hy)), f32x4_mul(hz, hz));
            let h_inv = f32x4_div(one, f32x4_sqrt(f32x4_add(h_len_sq, eps)));
            let ndoth = f32x4_min(f32x4_max(f32x4_mul(ndoth_unnorm, h_inv), zero), one);
            let s = f32x4_mul(lut_lookup_4(ndoth, spec_lut), f32x4_splat(specular));
            hr = f32x4_add(hr, s);
            hg = f32x4_add(hg, s);
            hb = f32x4_add(hb, s);
        }

        // Tone map + sRGB (Gooch always uses linear pipeline)
        let exp4 = f32x4_splat(exposure);
        let hr = tone_map_4(hr, tm, exp4);
        let hg = tone_map_4(hg, tm, exp4);
        let hb = tone_map_4(hb, tm, exp4);
        return [
            (linear_to_srgb(f32x4_extract_lane::<0>(hr)), linear_to_srgb(f32x4_extract_lane::<0>(hg)), linear_to_srgb(f32x4_extract_lane::<0>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<1>(hr)), linear_to_srgb(f32x4_extract_lane::<1>(hg)), linear_to_srgb(f32x4_extract_lane::<1>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<2>(hr)), linear_to_srgb(f32x4_extract_lane::<2>(hg)), linear_to_srgb(f32x4_extract_lane::<2>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<3>(hr)), linear_to_srgb(f32x4_extract_lane::<3>(hg)), linear_to_srgb(f32x4_extract_lane::<3>(hb))),
        ];
    }

    // Accumulate diffuse and specular across lights
    let mut diff_r = zero;
    let mut diff_g = zero;
    let mut diff_b = zero;
    let mut spec_r = zero;
    let mut spec_g = zero;
    let mut spec_b = zero;

    let oma4 = f32x4_splat(one_minus_ambient);

    for light in lights {
        // Light direction
        let (ldx, ldy, ldz) = if light.kind == LightKind::Directional {
            (f32x4_splat(light.dx), f32x4_splat(light.dy), f32x4_splat(light.dz))
        } else {
            let dx = f32x4_sub(f32x4_splat(light.dx), px4);
            let dy = f32x4_sub(f32x4_splat(light.dy), py4);
            let dz = f32x4_sub(f32x4_splat(light.dz), pz4);
            let inv = f32x4_div(one, f32x4_sqrt(f32x4_add(f32x4_add(
                f32x4_mul(dx, dx), f32x4_mul(dy, dy)), f32x4_add(f32x4_mul(dz, dz), eps))));
            (f32x4_mul(dx, inv), f32x4_mul(dy, inv), f32x4_mul(dz, inv))
        };

        // ndotl = abs(dot(n, l))
        let ndotl_raw = f32x4_abs(f32x4_add(f32x4_add(
            f32x4_mul(nx4, ldx), f32x4_mul(ny4, ldy)), f32x4_mul(nz4, ldz)));

        // Cel banding: floor(ndotl * bands) / bands
        let ndotl = if is_cel {
            let bands4 = f32x4_splat(cel_bands as f32);
            let inv_bands4 = f32x4_splat(1.0 / cel_bands as f32);
            f32x4_mul(f32x4_floor(f32x4_mul(ndotl_raw, bands4)), inv_bands4)
        } else { ndotl_raw };

        // Diffuse: one_minus_ambient * ndotl * light_color
        let df = f32x4_mul(oma4, ndotl);
        diff_r = f32x4_add(diff_r, f32x4_mul(f32x4_splat(light.cr), df));
        diff_g = f32x4_add(diff_g, f32x4_mul(f32x4_splat(light.cg), df));
        diff_b = f32x4_add(diff_b, f32x4_mul(f32x4_splat(light.cb), df));

        // Specular: half-vector, ndoth, LUT lookup
        if specular > 0.0 {
            let hx = f32x4_add(ldx, vdx);
            let hy = f32x4_add(ldy, vdy);
            let hz = f32x4_add(ldz, vdz);
            let ndoth_unnorm = f32x4_add(f32x4_add(
                f32x4_mul(onx, hx), f32x4_mul(ony, hy)), f32x4_mul(onz, hz));
            let h_len_sq = f32x4_add(f32x4_add(f32x4_mul(hx, hx), f32x4_mul(hy, hy)), f32x4_mul(hz, hz));
            let h_inv = f32x4_div(one, f32x4_sqrt(f32x4_add(h_len_sq, eps)));
            // Clamp to [0,1] — negative ndoth naturally maps to spec_lut[0]=0
            let ndoth = f32x4_min(f32x4_max(f32x4_mul(ndoth_unnorm, h_inv), zero), one);
            let spec_raw = f32x4_mul(lut_lookup_4(ndoth, spec_lut), f32x4_splat(specular));
            // Cel threshold: spec > 0.5 → 1.0, else 0.0
            let spec_val = if is_cel {
                let half = f32x4_splat(0.5);
                v128_bitselect(one, zero, f32x4_gt(spec_raw, half))
            } else { spec_raw };
            spec_r = f32x4_add(spec_r, f32x4_mul(f32x4_splat(light.cr), spec_val));
            spec_g = f32x4_add(spec_g, f32x4_mul(f32x4_splat(light.cg), spec_val));
            spec_b = f32x4_add(spec_b, f32x4_mul(f32x4_splat(light.cb), spec_val));
        }

        // SSS
        if sss_intensity > 0.0 {
            let lx = f32x4_add(f32x4_neg(ldx), f32x4_mul(onx, f32x4_splat(sss_distortion)));
            let ly = f32x4_add(f32x4_neg(ldy), f32x4_mul(ony, f32x4_splat(sss_distortion)));
            let lz = f32x4_add(f32x4_neg(ldz), f32x4_mul(onz, f32x4_splat(sss_distortion)));
            let vdotl = f32x4_min(f32x4_max(f32x4_add(f32x4_add(
                f32x4_mul(vdx, lx), f32x4_mul(vdy, ly)), f32x4_mul(vdz, lz)), zero), one);
            let sss = f32x4_mul(lut_lookup_4(vdotl, sss_lut), f32x4_splat(sss_intensity));
            diff_r = f32x4_add(diff_r, f32x4_mul(f32x4_splat(light.cr), sss));
            diff_g = f32x4_add(diff_g, f32x4_mul(f32x4_splat(light.cg), sss));
            diff_b = f32x4_add(diff_b, f32x4_mul(f32x4_splat(light.cb), sss));
        }
    }

    // Fresnel rim
    let rim4 = if fresnel > 0.0 {
        let base = f32x4_sub(one, f32x4_abs(f32x4_add(f32x4_add(
            f32x4_mul(onx, vdx), f32x4_mul(ony, vdy)), f32x4_mul(onz, vdz))));
        let clamped = f32x4_min(f32x4_max(base, zero), one);
        let rim_raw = f32x4_mul(lut_lookup_4(clamped, fresnel_lut), f32x4_splat(fresnel));
        // Cel threshold: rim > 0.5 → 1.0, else 0.0
        if is_cel {
            let half = f32x4_splat(0.5);
            v128_bitselect(one, zero, f32x4_gt(rim_raw, half))
        } else { rim_raw }
    } else { zero };

    // Finalize: base_lin * (ambient + diffuse) + specular + rim
    let exp4 = f32x4_splat(exposure);
    if gamma_correction {
        let lr4 = f32x4_splat(base_lin_r);
        let lg4 = f32x4_splat(base_lin_g);
        let lb4 = f32x4_splat(base_lin_b);
        let hr = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lr4, f32x4_add(amb_r, diff_r)), spec_r), rim4), tm, exp4);
        let hg = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lg4, f32x4_add(amb_g, diff_g)), spec_g), rim4), tm, exp4);
        let hb = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lb4, f32x4_add(amb_b, diff_b)), spec_b), rim4), tm, exp4);
        // Extract lanes and convert to sRGB
        [
            (linear_to_srgb(f32x4_extract_lane::<0>(hr)), linear_to_srgb(f32x4_extract_lane::<0>(hg)), linear_to_srgb(f32x4_extract_lane::<0>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<1>(hr)), linear_to_srgb(f32x4_extract_lane::<1>(hg)), linear_to_srgb(f32x4_extract_lane::<1>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<2>(hr)), linear_to_srgb(f32x4_extract_lane::<2>(hg)), linear_to_srgb(f32x4_extract_lane::<2>(hb))),
            (linear_to_srgb(f32x4_extract_lane::<3>(hr)), linear_to_srgb(f32x4_extract_lane::<3>(hg)), linear_to_srgb(f32x4_extract_lane::<3>(hb))),
        ]
    } else {
        let inv255 = f32x4_splat(1.0 / 255.0);
        let lr4 = f32x4_mul(f32x4_splat(base_lin_r), inv255);
        let lg4 = f32x4_mul(f32x4_splat(base_lin_g), inv255);
        let lb4 = f32x4_mul(f32x4_splat(base_lin_b), inv255);
        let hr = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lr4, f32x4_add(amb_r, diff_r)), spec_r), rim4), tm, exp4);
        let hg = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lg4, f32x4_add(amb_g, diff_g)), spec_g), rim4), tm, exp4);
        let hb = tone_map_4(f32x4_add(f32x4_add(f32x4_mul(lb4, f32x4_add(amb_b, diff_b)), spec_b), rim4), tm, exp4);
        let s255 = f32x4_splat(255.0);
        let hr = f32x4_nearest(f32x4_min(f32x4_mul(hr, s255), s255));
        let hg = f32x4_nearest(f32x4_min(f32x4_mul(hg, s255), s255));
        let hb = f32x4_nearest(f32x4_min(f32x4_mul(hb, s255), s255));
        [
            (f32x4_extract_lane::<0>(hr) as u8, f32x4_extract_lane::<0>(hg) as u8, f32x4_extract_lane::<0>(hb) as u8),
            (f32x4_extract_lane::<1>(hr) as u8, f32x4_extract_lane::<1>(hg) as u8, f32x4_extract_lane::<1>(hb) as u8),
            (f32x4_extract_lane::<2>(hr) as u8, f32x4_extract_lane::<2>(hg) as u8, f32x4_extract_lane::<2>(hb) as u8),
            (f32x4_extract_lane::<3>(hr) as u8, f32x4_extract_lane::<3>(hg) as u8, f32x4_extract_lane::<3>(hb) as u8),
        ]
    }
}

// ---------------------------------------------------------------------------
// Scalar tone mapping and shading
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn tone_map(r: f32, g: f32, b: f32, method: ToneMapMethod, exposure: f32) -> (f32, f32, f32) {
    if method == ToneMapMethod::None { return (r, g, b); }
    let (r, g, b) = (r * exposure, g * exposure, b * exposure);
    match method {
        ToneMapMethod::Reinhard => (r / (1.0 + r), g / (1.0 + g), b / (1.0 + b)),
        _ => {
            #[inline]
            fn aces(x: f32) -> f32 {
                let a = x * (2.51 * x + 0.03);
                let b = x * (2.43 * x + 0.59) + 0.14;
                (a / b).max(0.0).min(1.0)
            }
            (aces(r), aces(g), aces(b))
        }
    }
}

#[inline]
pub(crate) fn shade_point(
    normal: Vec3,
    world_pos: Vec3,
    base_lin: (f32, f32, f32),
    lights: &[LightF32],
    view_camera: Vec3,
    ambient: (f32, f32, f32),
    one_minus_ambient: f32,
    specular: f32,
    fresnel: f32,
    gamma_correction: bool,
    tone_map_method: ToneMapMethod,
    exposure: f32,
    shading: ShadingMode,
    gooch_warm: (f32, f32, f32),
    gooch_cool: (f32, f32, f32),
    cel_bands: usize,
    spec_lut: &[f32; 256],
    fresnel_lut: &[f32; 256],
    sss_intensity: f32,
    sss_distortion: f32,
    sss_lut: &[f32; 256],
) -> (u8, u8, u8) {
    let nx = normal.x as f32;
    let ny = normal.y as f32;
    let nz = normal.z as f32;
    let wpx = world_pos.x as f32;
    let wpy = world_pos.y as f32;
    let wpz = world_pos.z as f32;

    let vdx = (view_camera.x - world_pos.x) as f32;
    let vdy = (view_camera.y - world_pos.y) as f32;
    let vdz = (view_camera.z - world_pos.z) as f32;
    let v_inv = 1.0f32 / (vdx * vdx + vdy * vdy + vdz * vdz).sqrt();
    let vdx = vdx * v_inv;
    let vdy = vdy * v_inv;
    let vdz = vdz * v_inv;

    let n_dot_v = nx * vdx + ny * vdy + nz * vdz;
    let (onx, ony, onz) = if n_dot_v < 0.0 { (-nx, -ny, -nz) } else { (nx, ny, nz) };

    if shading == ShadingMode::Normal {
        return (
            ((onx * 0.5 + 0.5) * 255.0) as u8,
            ((ony * 0.5 + 0.5) * 255.0) as u8,
            ((onz * 0.5 + 0.5) * 255.0) as u8,
        );
    }

    if shading == ShadingMode::Gooch {
        let light = &lights[0];
        let (ldx, ldy, ldz) = get_light_dir(light, wpx, wpy, wpz);
        let ndotl = nx * ldx + ny * ldy + nz * ldz;
        let t = (1.0 + ndotl) * 0.5;
        let kd = 0.5f32;

        let (lr, lg, lb) = base_lin;

        let cool_r = gooch_cool.0 * 0.5 + lr * kd;
        let cool_g = gooch_cool.1 * 0.5 + lg * kd;
        let cool_b = gooch_cool.2 * 0.5 + lb * kd;
        let warm_r = gooch_warm.0 * 0.5 + lr * kd;
        let warm_g = gooch_warm.1 * 0.5 + lg * kd;
        let warm_b = gooch_warm.2 * 0.5 + lb * kd;

        let mut hr = cool_r + t * (warm_r - cool_r);
        let mut hg = cool_g + t * (warm_g - cool_g);
        let mut hb = cool_b + t * (warm_b - cool_b);

        if specular > 0.0 {
            let s = specular_contrib(ldx, ldy, ldz, vdx, vdy, vdz, onx, ony, onz, spec_lut, specular);
            hr += s;
            hg += s;
            hb += s;
        }

        let (hr, hg, hb) = tone_map(hr, hg, hb, tone_map_method, exposure);
        return (linear_to_srgb(hr), linear_to_srgb(hg), linear_to_srgb(hb));
    }

    let mut diff_r = 0.0f32;
    let mut diff_g = 0.0f32;
    let mut diff_b = 0.0f32;
    let mut spec_r = 0.0f32;
    let mut spec_g = 0.0f32;
    let mut spec_b = 0.0f32;

    for light in lights {
        let (ldx, ldy, ldz) = get_light_dir(light, wpx, wpy, wpz);

        let ndotl = (nx * ldx + ny * ldy + nz * ldz).abs();
        let band = if shading == ShadingMode::Cel && cel_bands > 0 {
            (ndotl * cel_bands as f32).floor() / cel_bands as f32
        } else { ndotl };
        let diffuse_factor = one_minus_ambient * band;
        diff_r += light.cr * diffuse_factor;
        diff_g += light.cg * diffuse_factor;
        diff_b += light.cb * diffuse_factor;

        if specular > 0.0 {
            let raw = specular_contrib(ldx, ldy, ldz, vdx, vdy, vdz, onx, ony, onz, spec_lut, specular);
            let s = if shading == ShadingMode::Cel { if raw > 0.5 { 1.0f32 } else { 0.0f32 } } else { raw };
            spec_r += light.cr * s;
            spec_g += light.cg * s;
            spec_b += light.cb * s;
        }

        if sss_intensity > 0.0 {
            let lx = -ldx + onx * sss_distortion;
            let ly = -ldy + ony * sss_distortion;
            let lz = -ldz + onz * sss_distortion;
            let vdotl = (vdx * lx + vdy * ly + vdz * lz).max(0.0).min(1.0);
            let sss = sss_lut[(vdotl * 255.0) as usize] * sss_intensity;
            diff_r += light.cr * sss;
            diff_g += light.cg * sss;
            diff_b += light.cb * sss;
        }
    }

    let rim = if fresnel > 0.0 {
        let base = 1.0 - (onx * vdx + ony * vdy + onz * vdz).abs();
        let r = fresnel_lut[(base.max(0.0) * 255.0).min(255.0) as usize] * fresnel;
        if shading == ShadingMode::Cel { if r > 0.5 { 1.0f32 } else { 0.0f32 } } else { r }
    } else { 0.0f32 };

    finalize_color(
        base_lin.0, base_lin.1, base_lin.2,
        ambient, (diff_r, diff_g, diff_b), (spec_r, spec_g, spec_b),
        rim, gamma_correction, tone_map_method, exposure,
    )
}
