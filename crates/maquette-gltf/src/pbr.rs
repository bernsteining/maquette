//! Cook-Torrance GGX metallic-roughness shader.
//!
//! `PbrContext` carries the invariant part of the shading state (camera, one
//! directional light, ambient stand-in). `MaterialShader` binds a PbrContext
//! to a specific `Material` and its textures for one triangle. It implements
//! the rasterizer's `PixelShader` trait — SIMD `shade4` for the 4-pixel inner
//! loop and scalar `shade_scalar` for the scanline remainder.
//!
//! Texture sampling runs scalar per lane (wasm SIMD has no gather), then the
//! sampled values are packed into `f32x4` for the BRDF math. baseColor and
//! emissive samples go through the sRGB→linear LUT; MR, occlusion, and normal
//! samples stay linear per glTF spec.
//!
//! Everything happens in linear space; the rasterizer gamma-encodes the
//! returned RGB to sRGB u8 (LUT-based, per lane extract).
//!
//! v2.x plans:
//! - Normal texture + tangent-space transform (phase 2g).
//! - KHR_lights_punctual multi-light dispatch (point + spot + distance falloff).
//! - IBL from a baked cubemap for real ambient reflection.

use maquette_core::color::srgb_to_linear_f01;
use maquette_core::math::Vec3;
use maquette_core::rasterizer::{PixelShader, ShadeIn4, ShadeOut4};
use crate::scene::{Material, TextureTransform};
use maquette_core::texture::Texture;
use std::arch::wasm32::*;

/// Pre-splatted punctual light — ready for the SIMD shader loop. Position
/// and direction are in world space. `attenuation_kind` picks the falloff
/// path (directional = no falloff; point/spot = inverse-square with range
/// cutoff; spot additionally applies cone smoothstep).
#[derive(Clone, Copy)]
pub struct SplattedLight {
    pub kind: crate::scene::LightKind,
    pub px: v128, pub py: v128, pub pz: v128,
    pub dx: v128, pub dy: v128, pub dz: v128,
    pub cr: v128, pub cg: v128, pub cb: v128,
    /// `1/range²` for point/spot with a finite range, `0` for infinite or
    /// directional. Used inside the smoothstep cutoff.
    pub range_inv: v128,
    pub range: v128,
    /// Precomputed spot-cone coefficients: `scale = 1/(inner_cos − outer_cos)`
    /// and `offset = -outer_cos · scale`. Applied as
    /// `saturate(cos_theta · scale + offset)`. Zero for non-spot lights.
    pub cone_scale: v128,
    pub cone_offset: v128,
}

impl SplattedLight {
    pub fn from_light(l: &crate::scene::PunctualLight) -> Self {
        use crate::scene::LightKind;
        let (cone_scale, cone_offset) = if l.kind == LightKind::Spot {
            let denom = (l.inner_cone_cos - l.outer_cone_cos).max(1e-4);
            let scale = 1.0 / denom;
            (scale, -l.outer_cone_cos * scale)
        } else { (0.0, 0.0) };
        Self {
            kind: l.kind,
            px: f32x4_splat(l.position.x as f32),
            py: f32x4_splat(l.position.y as f32),
            pz: f32x4_splat(l.position.z as f32),
            dx: f32x4_splat(l.direction.x as f32),
            dy: f32x4_splat(l.direction.y as f32),
            dz: f32x4_splat(l.direction.z as f32),
            cr: f32x4_splat(l.color[0]),
            cg: f32x4_splat(l.color[1]),
            cb: f32x4_splat(l.color[2]),
            range_inv: f32x4_splat(if l.range > 0.0 { 1.0 / l.range } else { 0.0 }),
            range: f32x4_splat(l.range),
            cone_scale: f32x4_splat(cone_scale),
            cone_offset: f32x4_splat(cone_offset),
        }
    }

    /// Fallback single-directional built from the plain-config `light_dir` +
    /// `light_color`. Used when the glTF scene declares no lights.
    pub fn fallback_directional(dir: maquette_core::math::Vec3, color: [f32; 3]) -> Self {
        Self {
            kind: crate::scene::LightKind::Directional,
            px: f32x4_splat(0.0), py: f32x4_splat(0.0), pz: f32x4_splat(0.0),
            // glTF directional light shines toward its -Z; our config `light_dir`
            // is FROM surface TO light. So the "toward" direction is -light_dir.
            dx: f32x4_splat(-dir.x as f32),
            dy: f32x4_splat(-dir.y as f32),
            dz: f32x4_splat(-dir.z as f32),
            cr: f32x4_splat(color[0]),
            cg: f32x4_splat(color[1]),
            cb: f32x4_splat(color[2]),
            range_inv: f32x4_splat(0.0),
            range:     f32x4_splat(0.0),
            cone_scale: f32x4_splat(0.0),
            cone_offset: f32x4_splat(0.0),
        }
    }
}

/// Tone-mapping operator. `None` = raw linear (clamped at gamma encode);
/// `Reinhard` = simple `x / (1 + x)`; `Aces` = ACES-fitted rational
/// approximation. Both non-None operators multiply by `exposure` first, so
/// they work like a virtual camera EV setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToneMap { None, Reinhard, Aces }

/// Everything a shader call needs that doesn't vary per pixel.
pub struct PbrContext {
    pub light_dir: Vec3,        // world-space, points FROM surface TO light (unit) — fallback only
    pub light_color: [f32; 3],  // linear RGB — fallback only
    pub ambient: [f32; 3],      // linear RGB (constant sky ambient — fallback when IBL is None)
    pub camera_pos: Vec3,
    pub tone_map: ToneMap,
    pub exposure: f32,

    /// Optional image-based lighting. `Some` supersedes `ambient` — analytical
    /// hemispheric irradiance + Karis polynomial BRDF for specular reflection.
    pub ibl: Option<IblContext>,
    /// Baked environment map (procedural, generated from `ibl` at render start).
    /// When `Some`, the IBL branch samples this instead of the analytical hemi.
    pub ibl_env: Option<&'static maquette_core::ibl::IblEnvironment>,
    /// World-space up direction — used for hemisphere orientation when IBL is
    /// enabled. Should match the config's `up`.
    pub world_up: Vec3,

    /// Pre-splatted lights, built once per render. Never empty — when the
    /// glTF has no KHR_lights_punctual, this contains one fallback
    /// directional built from `light_dir` / `light_color`.
    pub lights: Vec<SplattedLight>,
    /// Parallel to `lights`. Empty means no shadow pass. Non-empty: an
    /// `Option<LightShadow>` per light — `None` means that specific light
    /// doesn't cast shadows (fully lit as if no map was built).
    pub shadows: Vec<Option<maquette_core::shadow::LightShadow>>,
    /// PCF bias/softness parameters, meaningful only when `shadows` is non-empty.
    pub shadow_bias: maquette_core::shadow::BiasParams,
    pub shadow_softness: usize,
    /// PCSS emitter size. `0.0` = plain PCF (no blocker search).
    pub shadow_pcss_light_size: f32,
}

/// IBL scene constants (per render, not per material).
#[derive(Clone, Copy)]
pub struct IblContext {
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub intensity: f32,
}

impl PbrContext {
    /// Scalar shade. Kept as an executable reference for the SIMD `shade4`;
    /// also used by the scalar scanline remainder. `base`, `metallic`,
    /// `roughness`, `emissive`, `occlusion` are the post-sample material
    /// values (factor × texture where applicable). `normal` is the final
    /// world-space normal after any normal-map perturbation.
    #[allow(clippy::too_many_arguments)]
    pub fn shade_pixel(
        &self,
        world_pos: Vec3,
        normal: Vec3,
        base: [f32; 3],
        alpha: f32,
        metallic: f32,
        roughness: f32,
        emissive: [f32; 3],
        occlusion: f32,
        material: &Material,
    ) -> [f32; 4] {
        if material.unlit {
            return [base[0], base[1], base[2], alpha];
        }

        let alpha_g = (roughness * roughness).max(0.001);
        // Dielectric F0 from IOR (glTF KHR_materials_ior), tinted + scaled by
        // KHR_materials_specular. Metals still use base color as F0.
        let ior_f0 = {
            let n = material.ior;
            let x = (n - 1.0) / (n + 1.0);
            x * x
        };
        let dielectric_f0 = [
            (ior_f0 * material.specular_color[0] * material.specular_factor).min(1.0),
            (ior_f0 * material.specular_color[1] * material.specular_factor).min(1.0),
            (ior_f0 * material.specular_color[2] * material.specular_factor).min(1.0),
        ];
        let f0 = mix3(dielectric_f0, base, metallic);
        let diffuse = scale3(base, 1.0 - metallic);

        let n = [normal.x as f32, normal.y as f32, normal.z as f32];
        let mut v = [
            (self.camera_pos.x - world_pos.x) as f32,
            (self.camera_pos.y - world_pos.y) as f32,
            (self.camera_pos.z - world_pos.z) as f32,
        ];
        normalize_in_place(&mut v);
        let n_dot_v = dot3(n, v).max(0.0);

        // Accumulate direct-lighting + per-light clearcoat + per-light sheen
        // across every splatted light in the scene (fallback path in render.rs
        // ensures at least one entry).
        let mut r = 0.0_f32;
        let mut g = 0.0_f32;
        let mut b = 0.0_f32;
        let alpha_cc = (material.clearcoat_roughness * material.clearcoat_roughness).max(0.001);
        let has_clearcoat = material.clearcoat_factor > 0.0;
        let has_sheen = material.sheen_color.iter().any(|c| *c > 0.0);
        let alpha_s = (material.sheen_roughness * material.sheen_roughness).max(1e-4);

        for (light_idx, light) in self.lights.iter().enumerate() {
            let (l, atten) = resolve_light_scalar(light, world_pos);
            let n_dot_l = dot3(n, l).max(0.0);
            if n_dot_l <= 0.0 || n_dot_v <= 0.0 { continue; }
            let shadow = self.shadows.get(light_idx).and_then(|s| s.as_ref())
                .map(|sh| {
                    if self.shadow_pcss_light_size > 0.0 {
                        sh.lit_pcss(world_pos, normal, &self.shadow_bias, self.shadow_softness, self.shadow_pcss_light_size as f64)
                    } else {
                        sh.lit(world_pos, normal, &self.shadow_bias, self.shadow_softness)
                    }
                })
                .unwrap_or(1.0);
            let atten = [atten[0] * shadow, atten[1] * shadow, atten[2] * shadow];

            let mut h = [v[0] + l[0], v[1] + l[1], v[2] + l[2]];
            normalize_in_place(&mut h);
            let n_dot_h = dot3(n, h).max(0.0);
            let v_dot_h = dot3(v, h).max(0.0);

            let d = ggx_d(n_dot_h, alpha_g);
            let v_geom = smith_v(n_dot_v, n_dot_l, alpha_g);
            let f = fresnel_schlick(v_dot_h, f0);

            let specular = [d * v_geom * f[0], d * v_geom * f[1], d * v_geom * f[2]];
            let k_d = [(1.0 - f[0]) * (1.0 - metallic),
                       (1.0 - f[1]) * (1.0 - metallic),
                       (1.0 - f[2]) * (1.0 - metallic)];
            let inv_pi = std::f32::consts::FRAC_1_PI;
            let diffuse_term = [
                k_d[0] * diffuse[0] * inv_pi,
                k_d[1] * diffuse[1] * inv_pi,
                k_d[2] * diffuse[2] * inv_pi,
            ];
            let mut dr = (diffuse_term[0] + specular[0]) * atten[0] * n_dot_l;
            let mut dg = (diffuse_term[1] + specular[1]) * atten[1] * n_dot_l;
            let mut db = (diffuse_term[2] + specular[2]) * atten[2] * n_dot_l;

            // KHR_materials_clearcoat — layered on top of this light's base
            // contribution. Attenuates base then adds clearcoat spec.
            if has_clearcoat {
                let d_cc = ggx_d(n_dot_h, alpha_cc);
                let v_cc = smith_v(n_dot_v, n_dot_l, alpha_cc);
                let f_cc = 0.04 + (1.0 - 0.04) * (1.0 - v_dot_h).max(0.0).powi(5);
                let spec_cc = d_cc * v_cc * f_cc * n_dot_l;
                let cc = material.clearcoat_factor;
                let cc_atten = 1.0 - cc * f_cc;
                dr = dr * cc_atten + spec_cc * cc * atten[0];
                dg = dg * cc_atten + spec_cc * cc * atten[1];
                db = db * cc_atten + spec_cc * cc * atten[2];
            }

            // KHR_materials_sheen — additive per light.
            if has_sheen {
                let inv_alpha = 1.0 / alpha_s;
                let sin2h = (1.0 - n_dot_h * n_dot_h).max(0.0);
                let d_s = (2.0 + inv_alpha) / (2.0 * std::f32::consts::PI) * sin2h.powf(inv_alpha * 0.5);
                let v_s = 1.0 / (4.0 * (n_dot_l + n_dot_v - n_dot_l * n_dot_v));
                let s_common = d_s * v_s * n_dot_l;
                let sc = material.sheen_color;
                dr += sc[0] * s_common * atten[0];
                dg += sc[1] * s_common * atten[1];
                db += sc[2] * s_common * atten[2];
            }

            r += dr; g += dg; b += db;
        }

        // Ambient + emissive (light-independent) go on top.
        let inv_pi = std::f32::consts::FRAC_1_PI;
        let rough_atten = 1.0 - 0.5 * roughness;
        r += self.ambient[0] * diffuse[0] * inv_pi * occlusion
           + self.ambient[0] * f0[0] * rough_atten * occlusion
           + emissive[0];
        g += self.ambient[1] * diffuse[1] * inv_pi * occlusion
           + self.ambient[1] * f0[1] * rough_atten * occlusion
           + emissive[1];
        b += self.ambient[2] * diffuse[2] * inv_pi * occlusion
           + self.ambient[2] * f0[2] * rough_atten * occlusion
           + emissive[2];

        let (r, g, b) = tone_map_scalar(r, g, b, self.tone_map, self.exposure);
        [r, g, b, alpha]
    }
}

/// Scalar analog of `resolve_light_dir_and_atten`. Extracts lane 0 of the
/// splatted values and applies the same kind-dependent falloff. Called only
/// from the scalar remainder path — total pixel count is small so the
/// per-lane extract overhead is negligible in aggregate.
fn resolve_light_scalar(light: &SplattedLight, world_pos: Vec3) -> ([f32; 3], [f32; 3]) {
    use crate::scene::LightKind;
    let cr = f32x4_extract_lane::<0>(light.cr);
    let cg = f32x4_extract_lane::<0>(light.cg);
    let cb = f32x4_extract_lane::<0>(light.cb);
    let dx = f32x4_extract_lane::<0>(light.dx);
    let dy = f32x4_extract_lane::<0>(light.dy);
    let dz = f32x4_extract_lane::<0>(light.dz);
    match light.kind {
        LightKind::Directional => ([-dx, -dy, -dz], [cr, cg, cb]),
        LightKind::Point | LightKind::Spot => {
            let px = f32x4_extract_lane::<0>(light.px);
            let py = f32x4_extract_lane::<0>(light.py);
            let pz = f32x4_extract_lane::<0>(light.pz);
            let tx = px - world_pos.x as f32;
            let ty = py - world_pos.y as f32;
            let tz = pz - world_pos.z as f32;
            let dist2 = (tx*tx + ty*ty + tz*tz).max(1e-8);
            let inv_dist = 1.0 / dist2.sqrt();
            let lx = tx * inv_dist;
            let ly = ty * inv_dist;
            let lz = tz * inv_dist;
            let dist_atten = 1.0 / dist2.max(1e-4);
            let range_inv = f32x4_extract_lane::<0>(light.range_inv);
            let dr = dist2.sqrt() * range_inv;
            let dr4 = dr * dr * dr * dr;
            let cutoff = (1.0 - dr4).clamp(0.0, 1.0);
            let mut atten = dist_atten * cutoff * cutoff;
            if light.kind == LightKind::Spot {
                let cos_scale = f32x4_extract_lane::<0>(light.cone_scale);
                let cos_off = f32x4_extract_lane::<0>(light.cone_offset);
                let cos_theta = (-(dx * lx + dy * ly + dz * lz)).max(0.0);
                let cone = (cos_theta * cos_scale + cos_off).clamp(0.0, 1.0);
                atten *= cone;
            }
            ([lx, ly, lz], [cr * atten, cg * atten, cb * atten])
        }
    }
}

#[inline]
fn tone_map_scalar(r: f32, g: f32, b: f32, method: ToneMap, exposure: f32) -> (f32, f32, f32) {
    if method == ToneMap::None { return (r, g, b); }
    let (r, g, b) = (r * exposure, g * exposure, b * exposure);
    match method {
        ToneMap::Reinhard => (r / (1.0 + r), g / (1.0 + g), b / (1.0 + b)),
        ToneMap::Aces => {
            #[inline]
            fn aces(x: f32) -> f32 {
                let a = x * (2.51 * x + 0.03);
                let b = x * (2.43 * x + 0.59) + 0.14;
                (a / b).clamp(0.0, 1.0)
            }
            (aces(r), aces(g), aces(b))
        }
        ToneMap::None => unreachable!(),
    }
}

#[inline(always)]
fn tone_map_4(v: v128, method: ToneMap, exp4: v128) -> v128 {
    match method {
        ToneMap::None => v,
        ToneMap::Reinhard => {
            let ve = f32x4_mul(v, exp4);
            f32x4_div(ve, f32x4_add(f32x4_splat(1.0), ve))
        }
        ToneMap::Aces => {
            let ve = f32x4_mul(v, exp4);
            let a = f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.51), ve), f32x4_splat(0.03)));
            let b = f32x4_add(
                f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.43), ve), f32x4_splat(0.59))),
                f32x4_splat(0.14),
            );
            f32x4_min(f32x4_max(f32x4_div(a, b), f32x4_splat(0.0)), f32x4_splat(1.0))
        }
    }
}

// ---------------------------------------------------------------------------
// MaterialShader: per-triangle precomputed splats + texture refs
// ---------------------------------------------------------------------------

pub struct MaterialShader<'a> {
    ctx: &'a PbrContext,
    material: &'a Material,
    mask_cutoff: Option<f32>,

    // Optional textures. `None` = use the corresponding factor alone.
    base_tex: Option<&'a Texture>,
    mr_tex: Option<&'a Texture>,
    emissive_tex: Option<&'a Texture>,
    occlusion_tex: Option<&'a Texture>,
    normal_tex: Option<&'a Texture>,
    normal_scale: v128,

    // Precomputed per-triangle LOD per texture — passed straight to
    // `Texture::sample_lod`. Only meaningful when the texture is bound.
    lod_base: f32,
    lod_mr: f32,
    lod_emissive: f32,
    lod_occlusion: f32,
    lod_normal: f32,

    // Factors — splatted once per triangle so shade4 can multiply per-lane
    // sample × factor without re-splatting inside the inner loop.
    base_r_f: v128, base_g_f: v128, base_b_f: v128, alpha_f: v128,
    metallic_f: v128, roughness_f: v128,
    emissive_r_f: v128, emissive_g_f: v128, emissive_b_f: v128,
    occlusion_strength: v128,

    // Splatted scene constants.
    cam_x: v128, cam_y: v128, cam_z: v128,
    ambient_r: v128, ambient_g: v128, ambient_b: v128,

    /// Reference to the scene's pre-splatted light list. Never empty.
    lights: &'a [SplattedLight],
    /// Reference to the procedural env map (when IBL is enabled).
    ibl_env: Option<&'a maquette_core::ibl::IblEnvironment>,
    /// Reference to per-light shadow maps. Same length as `lights` when
    /// shadows are enabled; empty when disabled (skip the shadow sample path).
    /// `None` per-slot means that specific light doesn't cast.
    shadows: &'a [Option<maquette_core::shadow::LightShadow>],
    shadow_bias: maquette_core::shadow::BiasParams,
    shadow_softness: usize,
    shadow_pcss_light_size: f32,

    tone_map: ToneMap,
    exp4: v128,

    // IBL — splatted sky/ground constants + world up. `has_ibl=false` runs
    // the legacy fake-ambient path.
    has_ibl: bool,
    ibl_sky_r: v128, ibl_sky_g: v128, ibl_sky_b: v128,
    ibl_ground_r: v128, ibl_ground_g: v128, ibl_ground_b: v128,
    ibl_intensity: v128,
    up_x: v128, up_y: v128, up_z: v128,

    // KHR_materials_clearcoat — dielectric layer on top of base.
    clearcoat_factor: v128,      // splatted; 0 = disabled path skipped
    clearcoat_alpha: v128,       // roughness²
    has_clearcoat: bool,
    clearcoat_normal_tex: Option<&'a Texture>,
    clearcoat_normal_scale: v128,
    lod_clearcoat_normal: f32,

    // KHR_materials_sheen — Charlie D + Neubelt V.
    sheen_r: v128, sheen_g: v128, sheen_b: v128,
    sheen_inv_alpha: v128,       // 1/roughness² (scalar-per-lane pow uses this)
    has_sheen: bool,

    // KHR_materials_ior + KHR_materials_specular — precomputed dielectric F0.
    // Metals still use base color as F0 via the metallic-blended mix below.
    dielectric_f0_r: v128, dielectric_f0_g: v128, dielectric_f0_b: v128,

    // KHR_materials_transmission — dielectric transmission through a thin wall.
    // When `has_transmission`, we sample the IBL env at the refraction direction
    // and blend into the ambient/diffuse channel. Attenuated by base color and
    // Beer-Lambert (from KHR_materials_volume) before blending.
    transmission_tex: Option<&'a Texture>,
    lod_transmission: f32,
    transmission_factor: v128,   // splatted
    has_transmission: bool,
    // KHR_materials_volume — Beer-Lambert σ per channel (`-ln(color) / distance`).
    // We precompute `-σ · thickness` here so shade4 does a single `exp` per lane.
    volume_attenuation_r: v128,
    volume_attenuation_g: v128,
    volume_attenuation_b: v128,
    /// IOR ratio (surface / air = 1.0). Used to compute the refraction direction
    /// for transmission sampling. Duplicates the `dielectric_f0` derivation but
    /// keeps the raw IOR handy for Snell's law.
    ior_ratio: v128,
    /// KHR_materials_dispersion — per-RGB IOR ratio for chromatic separation
    /// during transmission. When `has_dispersion`, we do three refractions
    /// (one per channel) and sample the env three times. Zero factor = all
    /// three ratios equal to `ior_ratio` (no-op).
    ior_ratio_r: v128,
    ior_ratio_b: v128,
    has_dispersion: bool,

    // KHR_materials_iridescence — thin-film interference on the specular F0.
    // Belcour & Barla 2017 Fourier fit. When `has_iridescence`, we compute a
    // wavelength-dependent Fresnel term and mix it into F0 based on `factor`.
    iridescence_tex: Option<&'a Texture>,
    iridescence_thickness_tex: Option<&'a Texture>,
    lod_iridescence: f32,
    lod_iridescence_thickness: f32,
    iridescence_factor:      v128,
    iridescence_ior:         v128,   // splatted, film IOR
    iridescence_thickness_min: v128,
    iridescence_thickness_max: v128,
    has_iridescence: bool,

    // KHR_materials_anisotropy — directional roughness. Splits α into
    // `α_t` (tangent) and `α_b` (bitangent) for anisotropic GGX D+V.
    // Rotation rotates the tangent basis around N.
    anisotropy_tex: Option<&'a Texture>,
    lod_anisotropy: f32,
    anisotropy_strength: v128,
    anisotropy_cos_rot: v128,
    anisotropy_sin_rot: v128,
    has_anisotropy: bool,

    // KHR_materials_diffuse_transmission — matte back-side transmission (thin
    // cloth, backlit leaves). Adds a lambertian lobe on `max(0, -N·L)` tinted
    // by `dt_color`. Textures modulate the factor / color per pixel. Zero
    // factor → skip the whole lobe.
    diffuse_transmission_tex:       Option<&'a Texture>,
    diffuse_transmission_color_tex: Option<&'a Texture>,
    lod_diffuse_transmission:       f32,
    lod_diffuse_transmission_color: f32,
    diffuse_transmission_factor: v128,
    dt_color_r: v128, dt_color_g: v128, dt_color_b: v128,
    has_diffuse_transmission: bool,

    unlit: bool,
    /// glTF `doubleSided` — the shader flips the interpolated normal on
    /// back-facing lanes so lighting works either side. Culling still happens
    /// upstream when the material is single-sided.
    double_sided: bool,
}

impl<'a> MaterialShader<'a> {
    pub fn new(
        ctx: &'a PbrContext,
        material: &'a Material,
        textures: &'a [Texture],
        mask_cutoff: Option<f32>,
        lod_scale: f32,
    ) -> Self {
        let tex = |i: Option<u32>| i.and_then(|idx| textures.get(idx as usize));
        let base_tex = tex(material.base_color_texture);
        let mr_tex   = tex(material.metallic_roughness_texture);
        let em_tex   = tex(material.emissive_texture);
        let oc_tex   = tex(material.occlusion_texture);
        let n_tex    = tex(material.normal_texture);
        // Per-triangle LOD: `0.5 · log₂(lod_scale · w · h)`. Splits into
        // `0.5 · log₂(lod_scale) + Texture::lod_bias` (the second term is
        // precomputed at texture load — same value for every triangle that
        // samples this texture). The `.max(1.0)` clamp on the product would
        // matter only for lod_scale × texels < 1 (extreme minification into
        // sub-texel territory) — in that regime the caller clamps to
        // `[0, max_mip_level]` anyway, so a `.max(0.0)` on the sum below
        // is enough.
        let half_log_scale = 0.5 * (lod_scale.max(1e-30)).log2();
        let lod_for = |t: Option<&Texture>| -> f32 {
            let Some(t) = t else { return 0.0; };
            (half_log_scale + t.lod_bias).max(0.0)
        };
        Self {
            ctx,
            material,
            mask_cutoff,
            base_tex,
            mr_tex,
            emissive_tex:  em_tex,
            occlusion_tex: oc_tex,
            normal_tex:    n_tex,
            normal_scale:  f32x4_splat(material.normal_scale),
            lod_base:      lod_for(base_tex),
            lod_mr:        lod_for(mr_tex),
            lod_emissive:  lod_for(em_tex),
            lod_occlusion: lod_for(oc_tex),
            lod_normal:    lod_for(n_tex),

            base_r_f: f32x4_splat(material.base_color[0]),
            base_g_f: f32x4_splat(material.base_color[1]),
            base_b_f: f32x4_splat(material.base_color[2]),
            alpha_f:  f32x4_splat(material.base_color[3]),
            metallic_f:  f32x4_splat(material.metallic),
            roughness_f: f32x4_splat(material.roughness),
            emissive_r_f: f32x4_splat(material.emissive[0]),
            emissive_g_f: f32x4_splat(material.emissive[1]),
            emissive_b_f: f32x4_splat(material.emissive[2]),
            occlusion_strength: f32x4_splat(material.occlusion_strength),

            cam_x: f32x4_splat(ctx.camera_pos.x as f32),
            cam_y: f32x4_splat(ctx.camera_pos.y as f32),
            cam_z: f32x4_splat(ctx.camera_pos.z as f32),
            ambient_r: f32x4_splat(ctx.ambient[0]),
            ambient_g: f32x4_splat(ctx.ambient[1]),
            ambient_b: f32x4_splat(ctx.ambient[2]),
            lights: &ctx.lights,
            ibl_env: ctx.ibl_env.map(|e| e as &_),
            shadows: &ctx.shadows,
            shadow_bias: ctx.shadow_bias,
            shadow_softness: ctx.shadow_softness,
            shadow_pcss_light_size: ctx.shadow_pcss_light_size,

            tone_map: ctx.tone_map,
            exp4: f32x4_splat(ctx.exposure),

            has_ibl: ctx.ibl.is_some(),
            ibl_sky_r:    f32x4_splat(ctx.ibl.map(|c| c.sky[0]).unwrap_or(0.0)),
            ibl_sky_g:    f32x4_splat(ctx.ibl.map(|c| c.sky[1]).unwrap_or(0.0)),
            ibl_sky_b:    f32x4_splat(ctx.ibl.map(|c| c.sky[2]).unwrap_or(0.0)),
            ibl_ground_r: f32x4_splat(ctx.ibl.map(|c| c.ground[0]).unwrap_or(0.0)),
            ibl_ground_g: f32x4_splat(ctx.ibl.map(|c| c.ground[1]).unwrap_or(0.0)),
            ibl_ground_b: f32x4_splat(ctx.ibl.map(|c| c.ground[2]).unwrap_or(0.0)),
            ibl_intensity: f32x4_splat(ctx.ibl.map(|c| c.intensity).unwrap_or(1.0)),
            up_x: f32x4_splat(ctx.world_up.x as f32),
            up_y: f32x4_splat(ctx.world_up.y as f32),
            up_z: f32x4_splat(ctx.world_up.z as f32),

            clearcoat_factor: f32x4_splat(material.clearcoat_factor),
            clearcoat_alpha:  f32x4_splat((material.clearcoat_roughness * material.clearcoat_roughness).max(0.001)),
            has_clearcoat:    material.clearcoat_factor > 0.0,
            clearcoat_normal_tex: tex(material.clearcoat_normal_texture),
            clearcoat_normal_scale: f32x4_splat(material.clearcoat_normal_scale),
            lod_clearcoat_normal: lod_for(tex(material.clearcoat_normal_texture)),
            sheen_r:          f32x4_splat(material.sheen_color[0]),
            sheen_g:          f32x4_splat(material.sheen_color[1]),
            sheen_b:          f32x4_splat(material.sheen_color[2]),
            // All per-material precomputes (`1/α²`, dielectric F0, volume
            // attenuation via powf, IOR reciprocals) are baked at scene
            // flatten via `MaterialPrecomp::from_material` — this reads the
            // cached scalars and pays only the splat cost per triangle.
            sheen_inv_alpha:  f32x4_splat(material.precomp.sheen_inv_alpha),
            has_sheen:        material.sheen_color.iter().any(|c| *c > 0.0),
            dielectric_f0_r:  f32x4_splat(material.precomp.dielectric_f0[0]),
            dielectric_f0_g:  f32x4_splat(material.precomp.dielectric_f0[1]),
            dielectric_f0_b:  f32x4_splat(material.precomp.dielectric_f0[2]),

            transmission_tex: tex(material.transmission_texture),
            lod_transmission: lod_for(tex(material.transmission_texture)),
            transmission_factor: f32x4_splat(material.transmission_factor),
            has_transmission: material.transmission_factor > 0.0,
            volume_attenuation_r: f32x4_splat(material.precomp.volume_attenuation[0]),
            volume_attenuation_g: f32x4_splat(material.precomp.volume_attenuation[1]),
            volume_attenuation_b: f32x4_splat(material.precomp.volume_attenuation[2]),
            ior_ratio:   f32x4_splat(material.precomp.ior_ratio),
            ior_ratio_r: f32x4_splat(material.precomp.ior_ratio_r),
            ior_ratio_b: f32x4_splat(material.precomp.ior_ratio_b),
            has_dispersion: material.dispersion > 0.0 && material.transmission_factor > 0.0,

            iridescence_tex:              tex(material.iridescence_texture),
            iridescence_thickness_tex:    tex(material.iridescence_thickness_texture),
            lod_iridescence:              lod_for(tex(material.iridescence_texture)),
            lod_iridescence_thickness:    lod_for(tex(material.iridescence_thickness_texture)),
            iridescence_factor:           f32x4_splat(material.iridescence_factor),
            iridescence_ior:              f32x4_splat(material.iridescence_ior),
            iridescence_thickness_min:    f32x4_splat(material.iridescence_thickness_min),
            iridescence_thickness_max:    f32x4_splat(material.iridescence_thickness_max),
            has_iridescence:              material.iridescence_factor > 0.0,

            anisotropy_tex:               tex(material.anisotropy_texture),
            lod_anisotropy:               lod_for(tex(material.anisotropy_texture)),
            anisotropy_strength:          f32x4_splat(material.anisotropy_strength),
            anisotropy_cos_rot:           f32x4_splat(material.precomp.anisotropy_cos_rot),
            anisotropy_sin_rot:           f32x4_splat(material.precomp.anisotropy_sin_rot),

            diffuse_transmission_tex:       tex(material.diffuse_transmission_texture),
            diffuse_transmission_color_tex: tex(material.diffuse_transmission_color_texture),
            lod_diffuse_transmission:       lod_for(tex(material.diffuse_transmission_texture)),
            lod_diffuse_transmission_color: lod_for(tex(material.diffuse_transmission_color_texture)),
            diffuse_transmission_factor:    f32x4_splat(material.diffuse_transmission_factor),
            dt_color_r: f32x4_splat(material.diffuse_transmission_color[0]),
            dt_color_g: f32x4_splat(material.diffuse_transmission_color[1]),
            dt_color_b: f32x4_splat(material.diffuse_transmission_color[2]),
            has_diffuse_transmission:       material.diffuse_transmission_factor > 0.0,
            has_anisotropy:               material.anisotropy_strength > 0.0,

            unlit: material.unlit,
            double_sided: material.double_sided,
        }
    }

    /// Per-lane scalar texture gather for the four attribute vectors.
    /// Returns four `f32x4` bundles (base, mr, emissive, occlusion) already
    /// converted to linear where appropriate. Each texture slot picks TEXCOORD_0
    /// or TEXCOORD_1 based on its `texcoord_*` field in the material.
    #[inline]
    fn sample_textures4(&self, uv_u: v128, uv_v: v128, uv1_u: v128, uv1_v: v128, uv2_u: v128, uv2_v: v128) -> Samples4 {
        let mut base = [[1.0f32; 4]; 4];
        let mut mr   = [[0.0f32; 4]; 4];
        let mut emit = [[0.0f32; 4]; 4];
        let mut occ  = [1.0f32; 4];
        let mut nrm  = [[0.0f32; 3]; 4];
        let mut trans = [1.0f32; 4];
        // Diffuse-transmission per-pixel modulators. `dt_fac[i]` samples the
        // `diffuseTransmissionTexture` alpha channel (spec §KHR_materials_
        // diffuse_transmission: "the alpha component is multiplied by the
        // factor"). `dt_col[i]` samples the `diffuseTransmissionColorTexture`
        // RGB (linear, since it's a color modulator not a display colour).
        // Both default to 1.0 so materials without textures get factor+color
        // straight through.
        let mut dt_fac = [1.0f32; 4];
        let mut dt_col = [[1.0f32; 3]; 4];

        macro_rules! lane { ($i:tt) => { {
            let uv0  = [f32x4_extract_lane::<$i>(uv_u),  f32x4_extract_lane::<$i>(uv_v)];
            let uv1  = [f32x4_extract_lane::<$i>(uv1_u), f32x4_extract_lane::<$i>(uv1_v)];
            let uv2  = [f32x4_extract_lane::<$i>(uv2_u), f32x4_extract_lane::<$i>(uv2_v)];
            let pick = |n: u8| -> [f32; 2] {
                match n { 2 => uv2, 1 => uv1, _ => uv0 }
            };
            if let Some(t) = self.base_tex {
                let s = t.sample_lod(self.material.xform_base.apply(pick(self.material.texcoord_base)), self.lod_base);
                base[$i] = [
                    srgb_to_linear_f01(s[0]),
                    srgb_to_linear_f01(s[1]),
                    srgb_to_linear_f01(s[2]),
                    s[3],
                ];
            }
            if let Some(t) = self.mr_tex {
                mr[$i] = t.sample_lod(self.material.xform_mr.apply(pick(self.material.texcoord_mr)), self.lod_mr);
            }
            if let Some(t) = self.emissive_tex {
                let s = t.sample_lod(self.material.xform_emissive.apply(pick(self.material.texcoord_emissive)), self.lod_emissive);
                emit[$i] = [
                    srgb_to_linear_f01(s[0]),
                    srgb_to_linear_f01(s[1]),
                    srgb_to_linear_f01(s[2]),
                    0.0,
                ];
            }
            if let Some(t) = self.occlusion_tex {
                occ[$i] = t.sample_lod(self.material.xform_occlusion.apply(pick(self.material.texcoord_occlusion)), self.lod_occlusion)[0];
            }
            if let Some(t) = self.normal_tex {
                let s = t.sample_lod(self.material.xform_normal.apply(pick(self.material.texcoord_normal)), self.lod_normal);
                nrm[$i] = [s[0] * 2.0 - 1.0, s[1] * 2.0 - 1.0, s[2] * 2.0 - 1.0];
            }
            if let Some(t) = self.transmission_tex {
                trans[$i] = t.sample_lod(self.material.xform_transmission.apply(pick(self.material.texcoord_transmission)), self.lod_transmission)[0];
            }
            if let Some(t) = self.diffuse_transmission_tex {
                // Spec: only the alpha channel of the DT texture modulates the factor.
                let s = t.sample_lod(self.material.xform_diffuse_transmission.apply(pick(self.material.texcoord_diffuse_transmission)), self.lod_diffuse_transmission);
                dt_fac[$i] = s[3];
            }
            if let Some(t) = self.diffuse_transmission_color_tex {
                // Spec: color texture is sRGB, converted to linear for shading.
                let s = t.sample_lod(self.material.xform_diffuse_transmission_color.apply(pick(self.material.texcoord_diffuse_transmission_color)), self.lod_diffuse_transmission_color);
                dt_col[$i] = [
                    srgb_to_linear_f01(s[0]),
                    srgb_to_linear_f01(s[1]),
                    srgb_to_linear_f01(s[2]),
                ];
            }
        } } }
        lane!(0); lane!(1); lane!(2); lane!(3);

        Samples4 {
            base_r: f32x4(base[0][0], base[1][0], base[2][0], base[3][0]),
            base_g: f32x4(base[0][1], base[1][1], base[2][1], base[3][1]),
            base_b: f32x4(base[0][2], base[1][2], base[2][2], base[3][2]),
            base_a: f32x4(base[0][3], base[1][3], base[2][3], base[3][3]),
            roughness_tex: f32x4(mr[0][1], mr[1][1], mr[2][1], mr[3][1]),
            metallic_tex:  f32x4(mr[0][2], mr[1][2], mr[2][2], mr[3][2]),
            emit_r: f32x4(emit[0][0], emit[1][0], emit[2][0], emit[3][0]),
            emit_g: f32x4(emit[0][1], emit[1][1], emit[2][1], emit[3][1]),
            emit_b: f32x4(emit[0][2], emit[1][2], emit[2][2], emit[3][2]),
            occlusion: f32x4(occ[0], occ[1], occ[2], occ[3]),
            normal_lx: f32x4(nrm[0][0], nrm[1][0], nrm[2][0], nrm[3][0]),
            normal_ly: f32x4(nrm[0][1], nrm[1][1], nrm[2][1], nrm[3][1]),
            normal_lz: f32x4(nrm[0][2], nrm[1][2], nrm[2][2], nrm[3][2]),
            transmission: f32x4(trans[0], trans[1], trans[2], trans[3]),
            dt_factor_tex:  f32x4(dt_fac[0], dt_fac[1], dt_fac[2], dt_fac[3]),
            dt_color_r_tex: f32x4(dt_col[0][0], dt_col[1][0], dt_col[2][0], dt_col[3][0]),
            dt_color_g_tex: f32x4(dt_col[0][1], dt_col[1][1], dt_col[2][1], dt_col[3][1]),
            dt_color_b_tex: f32x4(dt_col[0][2], dt_col[1][2], dt_col[2][2], dt_col[3][2]),
        }
    }
}

struct Samples4 {
    base_r: v128, base_g: v128, base_b: v128, base_a: v128,
    // roughness_tex / metallic_tex are `1.0` when no texture is bound, so
    // multiplying with the factor works either way.
    roughness_tex: v128, metallic_tex: v128,
    emit_r: v128, emit_g: v128, emit_b: v128,
    /// occlusion sample is scalar per lane; `1.0` when no texture is bound.
    occlusion: v128,
    /// Tangent-space normal in `[-1, 1]`. Zero-vector when no normal texture
    /// is bound (shader falls back to interpolated geometric normal).
    normal_lx: v128, normal_ly: v128, normal_lz: v128,
    /// KHR_materials_transmission per-pixel scale (R channel × factor). `1.0`
    /// when no transmission texture — the factor multiply happens outside.
    transmission: v128,
    /// KHR_materials_diffuse_transmission per-pixel modulators. Alpha of the
    /// dt texture scales the factor; RGB of the color texture (linearised)
    /// scales the color. All default to 1.0 so materials without textures
    /// get factor+color straight through.
    dt_factor_tex: v128,
    dt_color_r_tex: v128, dt_color_g_tex: v128, dt_color_b_tex: v128,
}

impl<'a> PixelShader for MaterialShader<'a> {
    // Hot inner loop of the rasterizer — called once per 4-pixel batch, per
    // triangle. `#[inline]` (not `always`) — with `always`, wasmi's translator
    // panics on the resulting fused cmp+branch pattern (`cmp+branch fusion must
    // succeed`, wasmi 1.0.9 mod.rs:1704). Plain `#[inline]` gives LLVM enough
    // license to inline through the trait call without producing that shape.
    #[inline]
    fn shade4(&self, in_: ShadeIn4) -> ShadeOut4 {
        let zero = f32x4_splat(0.0);
        let one  = f32x4_splat(1.0);
        let default_keep = i32x4_splat(-1i32);

        // Sample all textures once per 4-pixel batch (scalar gather, packed
        // back to SIMD lanes). Textures without a binding yield the identity
        // for the multiply that follows.
        let s = self.sample_textures4(in_.uv_u, in_.uv_v, in_.uv1_u, in_.uv1_v, in_.uv2_u, in_.uv2_v);

        // Post-sample material values. glTF spec: `base = baseColorFactor
        // · baseColorTexture · COLOR_0`. COLOR_0 splats to 1.0 when absent
        // so this multiply is a no-op for meshes without vertex colours.
        let base_r = if self.base_tex.is_some() { f32x4_mul(s.base_r, self.base_r_f) } else { self.base_r_f };
        let base_g = if self.base_tex.is_some() { f32x4_mul(s.base_g, self.base_g_f) } else { self.base_g_f };
        let base_b = if self.base_tex.is_some() { f32x4_mul(s.base_b, self.base_b_f) } else { self.base_b_f };
        let alpha  = if self.base_tex.is_some() { f32x4_mul(s.base_a, self.alpha_f)  } else { self.alpha_f  };
        let base_r = f32x4_mul(base_r, in_.col_r);
        let base_g = f32x4_mul(base_g, in_.col_g);
        let base_b = f32x4_mul(base_b, in_.col_b);
        let alpha  = f32x4_mul(alpha,  in_.col_a);

        if self.unlit {
            return ShadeOut4 {
                r: base_r, g: base_g, b: base_b, a: alpha,
                keep: apply_mask_cutoff(alpha, self.mask_cutoff, default_keep),
            };
        }

        let metallic  = if self.mr_tex.is_some() { f32x4_mul(s.metallic_tex,  self.metallic_f)  } else { self.metallic_f  };
        let roughness = if self.mr_tex.is_some() { f32x4_mul(s.roughness_tex, self.roughness_f) } else { self.roughness_f };
        let emit_r    = if self.emissive_tex.is_some() { f32x4_mul(s.emit_r, self.emissive_r_f) } else { self.emissive_r_f };
        let emit_g    = if self.emissive_tex.is_some() { f32x4_mul(s.emit_g, self.emissive_g_f) } else { self.emissive_g_f };
        let emit_b    = if self.emissive_tex.is_some() { f32x4_mul(s.emit_b, self.emissive_b_f) } else { self.emissive_b_f };
        // Occlusion strength: mix(1, sampled, strength) per glTF.
        let occlusion = if self.occlusion_tex.is_some() {
            f32x4_add(one, f32x4_mul(self.occlusion_strength, f32x4_sub(s.occlusion, one)))
        } else { one };

        // F0 (per lane, since base varies per pixel now). Dielectric F0
        // comes from IOR + specular tint; metals blend to base color.
        let one_minus_metallic = f32x4_sub(one, metallic);
        let f0_r = f32x4_add(f32x4_mul(self.dielectric_f0_r, one_minus_metallic), f32x4_mul(base_r, metallic));
        let f0_g = f32x4_add(f32x4_mul(self.dielectric_f0_g, one_minus_metallic), f32x4_mul(base_g, metallic));
        let f0_b = f32x4_add(f32x4_mul(self.dielectric_f0_b, one_minus_metallic), f32x4_mul(base_b, metallic));

        // Diffuse albedo = base × (1 − metallic).
        let diffuse_r = f32x4_mul(base_r, one_minus_metallic);
        let diffuse_g = f32x4_mul(base_g, one_minus_metallic);
        let diffuse_b = f32x4_mul(base_b, one_minus_metallic);

        // Normal perturbation from normal map (identity when no normal_tex).
        //   n_world = T · lx + B · ly + N · lz, with B = (N × T) · w.
        let (nx_final, ny_final, nz_final) = if self.normal_tex.is_some() {
            // Scale xy per glTF; z stays untouched then we renormalise.
            let lx = f32x4_mul(s.normal_lx, self.normal_scale);
            let ly = f32x4_mul(s.normal_ly, self.normal_scale);
            let lz = s.normal_lz;
            let bx = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_y, in_.tan_z), f32x4_mul(in_.n_z, in_.tan_y)), in_.tan_w);
            let by = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_z, in_.tan_x), f32x4_mul(in_.n_x, in_.tan_z)), in_.tan_w);
            let bz = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_x, in_.tan_y), f32x4_mul(in_.n_y, in_.tan_x)), in_.tan_w);
            let nx = f32x4_add(f32x4_add(f32x4_mul(in_.tan_x, lx), f32x4_mul(bx, ly)), f32x4_mul(in_.n_x, lz));
            let ny = f32x4_add(f32x4_add(f32x4_mul(in_.tan_y, lx), f32x4_mul(by, ly)), f32x4_mul(in_.n_y, lz));
            let nz = f32x4_add(f32x4_add(f32x4_mul(in_.tan_z, lx), f32x4_mul(bz, ly)), f32x4_mul(in_.n_z, lz));
            normalize_v3(nx, ny, nz)
        } else {
            (in_.n_x, in_.n_y, in_.n_z)
        };

        // View direction V = normalize(camera − pos). N·V is per-pixel so
        // do it once; per-light L / H / N·L / N·H / V·H come from the loop.
        let vx_raw = f32x4_sub(self.cam_x, in_.pos_x);
        let vy_raw = f32x4_sub(self.cam_y, in_.pos_y);
        let vz_raw = f32x4_sub(self.cam_z, in_.pos_z);
        let (vx, vy, vz) = normalize_v3(vx_raw, vy_raw, vz_raw);
        // glTF spec: doubleSided materials MUST light back faces with the
        // *flipped* normal. Detect back-face per lane via `raw_n_dot_v < 0`
        // and negate the normal there. Single-sided materials are back-face
        // culled upstream so this only kicks in for the flag = true case.
        let (nx_final, ny_final, nz_final) = if self.double_sided {
            let raw = dot_v3(nx_final, ny_final, nz_final, vx, vy, vz);
            let back = f32x4_lt(raw, zero);
            // bitselect(a, b, mask): mask bits pick from a, else b. We want
            // −n where back is true, so pass (−n) as `a` and `n` as `b`.
            (
                v128_bitselect(f32x4_sub(zero, nx_final), nx_final, back),
                v128_bitselect(f32x4_sub(zero, ny_final), ny_final, back),
                v128_bitselect(f32x4_sub(zero, nz_final), nz_final, back),
            )
        } else {
            (nx_final, ny_final, nz_final)
        };
        let n_dot_v = f32x4_max(zero, dot_v3(nx_final, ny_final, nz_final, vx, vy, vz));

        // α = roughness²; α² used inside D and V terms.
        let alpha_g = f32x4_max(f32x4_mul(roughness, roughness), f32x4_splat(0.001));
        let a2 = f32x4_mul(alpha_g, alpha_g);
        let one_minus_a2 = f32x4_sub(one, a2);
        let inv_pi = f32x4_splat(std::f32::consts::FRAC_1_PI);
        let pi_v = f32x4_splat(std::f32::consts::PI);

        // Per-triangle constants for the clearcoat + sheen lobes.
        let a2_cc = if self.has_clearcoat {
            f32x4_mul(self.clearcoat_alpha, self.clearcoat_alpha)
        } else { zero };
        let one_minus_a2_cc = f32x4_sub(one, a2_cc);
        let sheen_inv_alpha = self.sheen_inv_alpha;
        let sheen_inv_alpha_half = f32x4_mul(sheen_inv_alpha, f32x4_splat(0.5));

        // Clearcoat's normal (per glTF spec: independent of base normal
        // perturbation). Precomputed once — used by every light inside the
        // loop when the clearcoat lobe is active.
        let (nx_cc, ny_cc, nz_cc) = if self.has_clearcoat {
            compute_clearcoat_normal(self, &in_)
        } else {
            (in_.n_x, in_.n_y, in_.n_z)
        };
        let ndv_cc = if self.has_clearcoat {
            f32x4_max(zero, dot_v3(nx_cc, ny_cc, nz_cc, vx, vy, vz))
        } else { zero };

        // Sum direct + clearcoat + sheen contributions across all lights.
        // Per glTF layered-material math, clearcoat Fresnel attenuates the
        // base direct term for each light (not the sum), so both belong inside
        // the loop. Sheen is additive per light.
        let mut direct_r = zero;
        let mut direct_g = zero;
        let mut direct_b = zero;
        // Unused since ambient clearcoat now uses N·V Fresnel; kept as a
        // placeholder in case future code wants it back.
        let primary_f_cc = f32x4_splat(0.04);

        for (light_idx, light) in self.lights.iter().enumerate() {
            let (lx, ly, lz, atten_r, atten_g, atten_b) = resolve_light_dir_and_atten(
                light, in_.pos_x, in_.pos_y, in_.pos_z,
            );
            let n_dot_l = f32x4_max(zero, dot_v3(nx_final, ny_final, nz_final, lx, ly, lz));

            let hx_raw = f32x4_add(vx, lx);
            let hy_raw = f32x4_add(vy, ly);
            let hz_raw = f32x4_add(vz, lz);
            let (hx, hy, hz) = normalize_v3(hx_raw, hy_raw, hz_raw);
            let n_dot_h = f32x4_max(zero, dot_v3(nx_final, ny_final, nz_final, hx, hy, hz));
            let v_dot_h = f32x4_max(zero, dot_v3(vx, vy, vz, hx, hy, hz));

            // GGX D — isotropic OR anisotropic (KHR_materials_anisotropy).
            // Anisotropic path: split α into (α_t, α_b) per glTF spec:
            //   α_t = mix(α, 1, strength²)   (stretched in tangent dir)
            //   α_b = α                       (unchanged in bitangent)
            // then D_aniso = 1 / (π · α_t · α_b · ((h·t/α_t)² + (h·b/α_b)² + (h·n)²)²)
            let (d, alpha_t2, alpha_b2, ta_x, ta_y, ta_z, ba_x, ba_y, ba_z) = if self.has_anisotropy {
                // Rotate tangent basis around N by anisotropy_rotation.
                let bx = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_y, in_.tan_z), f32x4_mul(in_.n_z, in_.tan_y)), in_.tan_w);
                let by = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_z, in_.tan_x), f32x4_mul(in_.n_x, in_.tan_z)), in_.tan_w);
                let bz = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_x, in_.tan_y), f32x4_mul(in_.n_y, in_.tan_x)), in_.tan_w);
                let cs = self.anisotropy_cos_rot;
                let sn = self.anisotropy_sin_rot;
                let tax = f32x4_add(f32x4_mul(cs, in_.tan_x), f32x4_mul(sn, bx));
                let tay = f32x4_add(f32x4_mul(cs, in_.tan_y), f32x4_mul(sn, by));
                let taz = f32x4_add(f32x4_mul(cs, in_.tan_z), f32x4_mul(sn, bz));
                // Bitangent = N × T_rotated (keeps orthonormality without needing
                // a second rotation).
                let bax = f32x4_sub(f32x4_mul(ny_final, taz), f32x4_mul(nz_final, tay));
                let bay = f32x4_sub(f32x4_mul(nz_final, tax), f32x4_mul(nx_final, taz));
                let baz = f32x4_sub(f32x4_mul(nx_final, tay), f32x4_mul(ny_final, tax));
                let s2 = f32x4_mul(self.anisotropy_strength, self.anisotropy_strength);
                let at2 = f32x4_add(f32x4_mul(a2, f32x4_sub(one, s2)), s2);
                let ab2 = a2;
                let ht = dot_v3(hx, hy, hz, tax, tay, taz);
                let hb = dot_v3(hx, hy, hz, bax, bay, baz);
                let a_x = f32x4_div(ht, f32x4_max(at2, f32x4_splat(1e-7)));
                let a_y = f32x4_div(hb, f32x4_max(ab2, f32x4_splat(1e-7)));
                let sum = f32x4_add(f32x4_add(f32x4_mul(a_x, a_x), f32x4_mul(a_y, a_y)), f32x4_mul(n_dot_h, n_dot_h));
                let denom_a = f32x4_max(f32x4_mul(pi_v, f32x4_mul(f32x4_mul(at2, ab2), f32x4_mul(sum, sum))), f32x4_splat(1e-7));
                let d_a = f32x4_div(one, denom_a);
                (d_a, at2, ab2, tax, tay, taz, bax, bay, baz)
            } else {
                let denom = f32x4_add(f32x4_mul(f32x4_mul(n_dot_h, n_dot_h), f32x4_sub(a2, one)), one);
                let d_iso = f32x4_div(a2, f32x4_max(f32x4_mul(pi_v, f32x4_mul(denom, denom)), f32x4_splat(1e-7)));
                (d_iso, a2, a2, zero, zero, zero, zero, zero, zero)
            };

            // Smith V — anisotropic-aware. For anisotropic, Λ uses (α_t, α_b)
            // scaled dot components in the tangent frame. For isotropic, falls
            // back to the standard height-correlated Smith with a single α.
            let v_geom = if self.has_anisotropy {
                // Λ(v) = 0.5 · (-1 + √(1 + (α_t² · v·t² + α_b² · v·b²) / v·n²))
                let vt = dot_v3(vx, vy, vz, ta_x, ta_y, ta_z);
                let vb = dot_v3(vx, vy, vz, ba_x, ba_y, ba_z);
                let lt = dot_v3(lx, ly, lz, ta_x, ta_y, ta_z);
                let lb = dot_v3(lx, ly, lz, ba_x, ba_y, ba_z);
                let ndv_safe = f32x4_max(n_dot_v, f32x4_splat(1e-7));
                let ndl_safe = f32x4_max(n_dot_l, f32x4_splat(1e-7));
                let lam_v = f32x4_mul(ndl_safe, f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(vt, vt), alpha_t2), f32x4_add(f32x4_mul(f32x4_mul(vb, vb), alpha_b2), f32x4_mul(ndv_safe, ndv_safe)))));
                let lam_l = f32x4_mul(ndv_safe, f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(lt, lt), alpha_t2), f32x4_add(f32x4_mul(f32x4_mul(lb, lb), alpha_b2), f32x4_mul(ndl_safe, ndl_safe)))));
                f32x4_div(f32x4_splat(0.5), f32x4_max(f32x4_add(lam_v, lam_l), f32x4_splat(1e-7)))
            } else {
                let ggx_v_t = f32x4_mul(n_dot_l,
                    f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(n_dot_v, n_dot_v), one_minus_a2), a2)));
                let ggx_l_t = f32x4_mul(n_dot_v,
                    f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(n_dot_l, n_dot_l), one_minus_a2), a2)));
                f32x4_div(f32x4_splat(0.5), f32x4_max(f32x4_add(ggx_v_t, ggx_l_t), f32x4_splat(1e-7)))
            };

            // Iridescence — KHR_materials_iridescence. Modulates the specular
            // F0 with a wavelength-dependent Fresnel from thin-film interference.
            // We use a simplified sinusoidal-phase approximation over 3 RGB
            // wavelengths (685/532/465 nm). Not physically-exact — that needs
            // Belcour-Barla's Fourier fit ~40 lines — but produces the
            // characteristic angle-dependent rainbow shift that iridescent
            // materials are recognisable by.
            let (f0_ir_r, f0_ir_g, f0_ir_b) = if self.has_iridescence {
                let cos_i = f32x4_max(v_dot_h, f32x4_splat(1e-4));
                // Refract into film (Snell): sin²θ_2 = (1/η)² · (1 − cos²θ_1)
                let inv_ir_ior = f32x4_div(one, self.iridescence_ior);
                let sin_i2 = f32x4_sub(one, f32x4_mul(cos_i, cos_i));
                let sin_t2 = f32x4_mul(f32x4_mul(inv_ir_ior, inv_ir_ior), sin_i2);
                let cos_t = f32x4_sqrt(f32x4_max(f32x4_sub(one, sin_t2), zero));
                // Thickness — texture lookup gated behind has_iridescence_thickness_tex;
                // for MVP use midpoint of [min, max]. Per-pixel thickness tex is a
                // future refinement.
                let thickness = f32x4_mul(f32x4_add(self.iridescence_thickness_min, self.iridescence_thickness_max), f32x4_splat(0.5));
                // Optical path difference (nanometres).
                let opd = f32x4_mul(f32x4_mul(f32x4_splat(2.0), self.iridescence_ior), f32x4_mul(thickness, cos_t));
                let two_pi = f32x4_splat(2.0 * std::f32::consts::PI);
                let phase_r = f32x4_mul(two_pi, f32x4_div(opd, f32x4_splat(685.0)));
                let phase_g = f32x4_mul(two_pi, f32x4_div(opd, f32x4_splat(532.0)));
                let phase_b = f32x4_mul(two_pi, f32x4_div(opd, f32x4_splat(465.0)));
                let (cr, cg, cb) = (per_lane_cos(phase_r), per_lane_cos(phase_g), per_lane_cos(phase_b));
                let tint_r = f32x4_add(f32x4_splat(0.5), f32x4_mul(f32x4_splat(0.5), cr));
                let tint_g = f32x4_add(f32x4_splat(0.5), f32x4_mul(f32x4_splat(0.5), cg));
                let tint_b = f32x4_add(f32x4_splat(0.5), f32x4_mul(f32x4_splat(0.5), cb));
                // Modulate F0 by tint, then mix with base F0 by factor.
                let ir_r = f32x4_mul(f0_r, f32x4_mul(f32x4_splat(2.0), tint_r));
                let ir_g = f32x4_mul(f0_g, f32x4_mul(f32x4_splat(2.0), tint_g));
                let ir_b = f32x4_mul(f0_b, f32x4_mul(f32x4_splat(2.0), tint_b));
                let mix_f = self.iridescence_factor;
                let inv_mix = f32x4_sub(one, mix_f);
                (
                    f32x4_add(f32x4_mul(f0_r, inv_mix), f32x4_mul(ir_r, mix_f)),
                    f32x4_add(f32x4_mul(f0_g, inv_mix), f32x4_mul(ir_g, mix_f)),
                    f32x4_add(f32x4_mul(f0_b, inv_mix), f32x4_mul(ir_b, mix_f)),
                )
            } else {
                (f0_r, f0_g, f0_b)
            };

            // Schlick F using (possibly iridescence-shifted) F0.
            let x = f32x4_max(zero, f32x4_sub(one, v_dot_h));
            let x2 = f32x4_mul(x, x);
            let x4 = f32x4_mul(x2, x2);
            let x5 = f32x4_mul(x4, x);
            let f_r = f32x4_add(f0_ir_r, f32x4_mul(f32x4_sub(one, f0_ir_r), x5));
            let f_g = f32x4_add(f0_ir_g, f32x4_mul(f32x4_sub(one, f0_ir_g), x5));
            let f_b = f32x4_add(f0_ir_b, f32x4_mul(f32x4_sub(one, f0_ir_b), x5));

            let dv = f32x4_mul(d, v_geom);
            let spec_r = f32x4_mul(dv, f_r);
            let spec_g = f32x4_mul(dv, f_g);
            let spec_b = f32x4_mul(dv, f_b);

            let kd_r = f32x4_mul(f32x4_sub(one, f_r), one_minus_metallic);
            let kd_g = f32x4_mul(f32x4_sub(one, f_g), one_minus_metallic);
            let kd_b = f32x4_mul(f32x4_sub(one, f_b), one_minus_metallic);
            let diff_r = f32x4_mul(f32x4_mul(kd_r, diffuse_r), inv_pi);
            let diff_g = f32x4_mul(f32x4_mul(kd_g, diffuse_g), inv_pi);
            let diff_b = f32x4_mul(f32x4_mul(kd_b, diffuse_b), inv_pi);

            let lit_mask = v128_and(f32x4_gt(n_dot_l, zero), f32x4_gt(n_dot_v, zero));

            // Per-lane shadow factor from this light's map. Scalar sample per
            // lane (wasm SIMD has no gather); cheap given typical PCF kernels
            // are 3×3–5×5 and cost is bounded per pixel.
            let shadow_v = match self.shadows.get(light_idx).and_then(|s| s.as_ref()) {
                Some(sh) => {
                    let pcss = self.shadow_pcss_light_size as f64;
                    let s0 = shadow_lane::<0>(&in_, sh, self.shadow_bias, self.shadow_softness, pcss);
                    let s1 = shadow_lane::<1>(&in_, sh, self.shadow_bias, self.shadow_softness, pcss);
                    let s2 = shadow_lane::<2>(&in_, sh, self.shadow_bias, self.shadow_softness, pcss);
                    let s3 = shadow_lane::<3>(&in_, sh, self.shadow_bias, self.shadow_softness, pcss);
                    f32x4(s0, s1, s2, s3)
                }
                None => one,
            };

            // Base direct for this light: (diffuse + spec) · light_atten · N·L.
            // Shadow factor scales the whole per-light contribution (direct +
            // clearcoat + sheen) uniformly — occlusion of ONE light shouldn't
            // dim the others.
            let mut base_r = f32x4_mul(f32x4_mul(f32x4_mul(f32x4_add(diff_r, spec_r), atten_r), n_dot_l), shadow_v);
            let mut base_g = f32x4_mul(f32x4_mul(f32x4_mul(f32x4_add(diff_g, spec_g), atten_g), n_dot_l), shadow_v);
            let mut base_b = f32x4_mul(f32x4_mul(f32x4_mul(f32x4_add(diff_b, spec_b), atten_b), n_dot_l), shadow_v);

            // KHR_materials_diffuse_transmission — matte back-lit lambertian.
            // Uses the flipped normal (max(0, -N·L)) so backlit surfaces glow
            // even when the visible face is in shadow. Tinted by
            // `diffuse_transmission_color` × sampled color texture; the
            // front-side kd already includes `(1 - metallic)` so the
            // transmitted term does too — an all-metal surface has no
            // transmission per spec. Textures modulate per-pixel: alpha
            // channel scales factor, RGB scales color (both default to 1
            // when the texture isn't bound).
            if self.has_diffuse_transmission {
                let ndl_back = f32x4_max(zero, f32x4_neg(dot_v3(nx_final, ny_final, nz_final, lx, ly, lz)));
                let dt_f = f32x4_mul(self.diffuse_transmission_factor, s.dt_factor_tex);
                let dt_cr = f32x4_mul(self.dt_color_r, s.dt_color_r_tex);
                let dt_cg = f32x4_mul(self.dt_color_g, s.dt_color_g_tex);
                let dt_cb = f32x4_mul(self.dt_color_b, s.dt_color_b_tex);
                let scale = f32x4_mul(
                    f32x4_mul(one_minus_metallic, dt_f),
                    f32x4_mul(inv_pi, f32x4_mul(ndl_back, shadow_v)));
                base_r = f32x4_add(base_r, f32x4_mul(f32x4_mul(dt_cr, atten_r), scale));
                base_g = f32x4_add(base_g, f32x4_mul(f32x4_mul(dt_cg, atten_g), scale));
                base_b = f32x4_add(base_b, f32x4_mul(f32x4_mul(dt_cb, atten_b), scale));
            }

            // KHR_materials_clearcoat — per-light layered attenuation + spec.
            // Uses clearcoat's own N + independent light dot products.
            if self.has_clearcoat {
                let ndl_cc = f32x4_max(zero, dot_v3(nx_cc, ny_cc, nz_cc, lx, ly, lz));
                let ndh_cc = f32x4_max(zero, dot_v3(nx_cc, ny_cc, nz_cc, hx, hy, hz));
                let denom_cc = f32x4_add(f32x4_mul(f32x4_mul(ndh_cc, ndh_cc), f32x4_sub(a2_cc, one)), one);
                let d_cc = f32x4_div(a2_cc, f32x4_max(
                    f32x4_mul(pi_v, f32x4_mul(denom_cc, denom_cc)), f32x4_splat(1e-7)));
                let ggx_v_cc = f32x4_mul(ndl_cc,
                    f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(ndv_cc, ndv_cc), one_minus_a2_cc), a2_cc)));
                let ggx_l_cc = f32x4_mul(ndv_cc,
                    f32x4_sqrt(f32x4_add(f32x4_mul(f32x4_mul(ndl_cc, ndl_cc), one_minus_a2_cc), a2_cc)));
                let v_cc = f32x4_div(f32x4_splat(0.5),
                    f32x4_max(f32x4_add(ggx_v_cc, ggx_l_cc), f32x4_splat(1e-7)));
                let f0_cc = f32x4_splat(0.04);
                let f_cc = f32x4_add(f0_cc, f32x4_mul(f32x4_sub(one, f0_cc), x5));
                let spec_cc = f32x4_mul(f32x4_mul(d_cc, v_cc), f_cc);
                let cc = self.clearcoat_factor;
                let cc_atten = f32x4_sub(one, f32x4_mul(cc, f_cc));
                // Clearcoat spec also gets shadowed.
                let cc_add = f32x4_mul(f32x4_mul(f32x4_mul(spec_cc, cc), ndl_cc), shadow_v);
                base_r = f32x4_add(f32x4_mul(base_r, cc_atten), f32x4_mul(cc_add, atten_r));
                base_g = f32x4_add(f32x4_mul(base_g, cc_atten), f32x4_mul(cc_add, atten_g));
                base_b = f32x4_add(f32x4_mul(base_b, cc_atten), f32x4_mul(cc_add, atten_b));
                let _ = light_idx;
            }

            // KHR_materials_sheen — additive per light (Charlie D + Neubelt V).
            if self.has_sheen {
                let sin2h = f32x4_max(f32x4_sub(one, f32x4_mul(n_dot_h, n_dot_h)), zero);
                let pow_result = {
                    let bs = [
                        f32x4_extract_lane::<0>(sin2h),
                        f32x4_extract_lane::<1>(sin2h),
                        f32x4_extract_lane::<2>(sin2h),
                        f32x4_extract_lane::<3>(sin2h),
                    ];
                    let es = [
                        f32x4_extract_lane::<0>(sheen_inv_alpha_half),
                        f32x4_extract_lane::<1>(sheen_inv_alpha_half),
                        f32x4_extract_lane::<2>(sheen_inv_alpha_half),
                        f32x4_extract_lane::<3>(sheen_inv_alpha_half),
                    ];
                    f32x4(bs[0].powf(es[0]), bs[1].powf(es[1]), bs[2].powf(es[2]), bs[3].powf(es[3]))
                };
                let two_pi = f32x4_splat(2.0 * std::f32::consts::PI);
                let d_sheen = f32x4_mul(
                    f32x4_div(f32x4_add(f32x4_splat(2.0), sheen_inv_alpha), two_pi), pow_result);
                let denom_v = f32x4_sub(f32x4_add(n_dot_l, n_dot_v), f32x4_mul(n_dot_l, n_dot_v));
                let v_sheen = f32x4_div(f32x4_splat(1.0),
                    f32x4_max(f32x4_mul(f32x4_splat(4.0), denom_v), f32x4_splat(1e-7)));
                let common = f32x4_mul(f32x4_mul(f32x4_mul(d_sheen, v_sheen), n_dot_l), shadow_v);
                base_r = f32x4_add(base_r, f32x4_mul(f32x4_mul(self.sheen_r, common), atten_r));
                base_g = f32x4_add(base_g, f32x4_mul(f32x4_mul(self.sheen_g, common), atten_g));
                base_b = f32x4_add(base_b, f32x4_mul(f32x4_mul(self.sheen_b, common), atten_b));
            }

            direct_r = f32x4_add(direct_r, v128_and(base_r, lit_mask));
            direct_g = f32x4_add(direct_g, v128_and(base_g, lit_mask));
            direct_b = f32x4_add(direct_b, v128_and(base_b, lit_mask));
        }

        // Ambient / IBL path. Three tiers:
        //   * `ibl_env` present → real env-map sampling (procedural cubemap-like).
        //   * `has_ibl` (analytical hemispheric config only) → old fallback.
        //   * Neither → constant `ambient` fake.
        let (amb_diff_r, amb_diff_g, amb_diff_b, amb_spec_r, amb_spec_g, amb_spec_b) =
        if let Some(env) = self.ibl_env {
            // Reflection direction R = 2·(N·V)·N − V.
            let n_dot_v_full = dot_v3(nx_final, ny_final, nz_final, vx, vy, vz);
            let two_ndv = f32x4_mul(f32x4_splat(2.0), n_dot_v_full);
            let rx = f32x4_sub(f32x4_mul(two_ndv, nx_final), vx);
            let ry = f32x4_sub(f32x4_mul(two_ndv, ny_final), vy);
            let rz = f32x4_sub(f32x4_mul(two_ndv, nz_final), vz);

            let max_lod = env.max_lod;
            let mut diff = [[0.0f32; 3]; 4];
            let mut spec = [[0.0f32; 3]; 4];
            // Back-facing diffuse sample for KHR_materials_diffuse_transmission's
            // IBL contribution — the env at -N models light passing through the
            // surface from the far side. Only populated when the material
            // actually uses DT (otherwise the sample is wasted).
            let mut diff_back = [[0.0f32; 3]; 4];
            let dt_on = self.has_diffuse_transmission;
            macro_rules! lane { ($i:tt) => { {
                let nx = f32x4_extract_lane::<$i>(nx_final);
                let ny = f32x4_extract_lane::<$i>(ny_final);
                let nz = f32x4_extract_lane::<$i>(nz_final);
                let rxl = f32x4_extract_lane::<$i>(rx);
                let ryl = f32x4_extract_lane::<$i>(ry);
                let rzl = f32x4_extract_lane::<$i>(rz);
                let rough = f32x4_extract_lane::<$i>(roughness);
                diff[$i] = env.sample_diffuse(nx, ny, nz);
                spec[$i] = env.sample_dir(rxl, ryl, rzl, rough * max_lod);
                if dt_on { diff_back[$i] = env.sample_diffuse(-nx, -ny, -nz); }
            } } }
            lane!(0); lane!(1); lane!(2); lane!(3);
            let irr_r = f32x4(diff[0][0], diff[1][0], diff[2][0], diff[3][0]);
            let irr_g = f32x4(diff[0][1], diff[1][1], diff[2][1], diff[3][1]);
            let irr_b = f32x4(diff[0][2], diff[1][2], diff[2][2], diff[3][2]);
            let env_spec_r = f32x4(spec[0][0], spec[1][0], spec[2][0], spec[3][0]);
            let env_spec_g = f32x4(spec[0][1], spec[1][1], spec[2][1], spec[3][1]);
            let env_spec_b = f32x4(spec[0][2], spec[1][2], spec[2][2], spec[3][2]);

            let mut ad_r = f32x4_mul(f32x4_mul(irr_r, diffuse_r), occlusion);
            let mut ad_g = f32x4_mul(f32x4_mul(irr_g, diffuse_g), occlusion);
            let mut ad_b = f32x4_mul(f32x4_mul(irr_b, diffuse_b), occlusion);

            // KHR_materials_diffuse_transmission — env contribution. Same
            // `(1 - metallic) · factor · color` scaling as the direct-light
            // lobe, but here the "light" is the env sampled at -N.
            if dt_on {
                let irr_back_r = f32x4(diff_back[0][0], diff_back[1][0], diff_back[2][0], diff_back[3][0]);
                let irr_back_g = f32x4(diff_back[0][1], diff_back[1][1], diff_back[2][1], diff_back[3][1]);
                let irr_back_b = f32x4(diff_back[0][2], diff_back[1][2], diff_back[2][2], diff_back[3][2]);
                let dt_f  = f32x4_mul(self.diffuse_transmission_factor, s.dt_factor_tex);
                let dt_cr = f32x4_mul(self.dt_color_r, s.dt_color_r_tex);
                let dt_cg = f32x4_mul(self.dt_color_g, s.dt_color_g_tex);
                let dt_cb = f32x4_mul(self.dt_color_b, s.dt_color_b_tex);
                let scale = f32x4_mul(f32x4_mul(one_minus_metallic, dt_f), occlusion);
                ad_r = f32x4_add(ad_r, f32x4_mul(f32x4_mul(irr_back_r, dt_cr), scale));
                ad_g = f32x4_add(ad_g, f32x4_mul(f32x4_mul(irr_back_g, dt_cg), scale));
                ad_b = f32x4_add(ad_b, f32x4_mul(f32x4_mul(irr_back_b, dt_cb), scale));
            }

            // Karis split-sum polynomial (full form with exp2 grazing term).
            // Returns (scale, bias) that combine F0 and roughness.
            // a004 = min(r.x², exp2(-9.28·NoV)) · r.x + r.y
            let r_x = f32x4_sub(one, roughness);
            let r_y = f32x4_add(f32x4_mul(roughness, f32x4_splat(-0.0275)), f32x4_splat(0.0425));
            let r_z = f32x4_add(f32x4_mul(roughness, f32x4_splat(-0.572)), f32x4_splat(1.04));
            let r_w = f32x4_add(f32x4_mul(roughness, f32x4_splat(0.022)),  f32x4_splat(-0.04));
            // SIMD exp2 approximation for the grazing-term argument
            // `-9.28·NoV`, x ∈ roughly [-9.28, 0]. Was: extract 4 lanes,
            // scalar `.exp2()` per lane, repack — 8 lane ops + 4 libcalls per
            // pixel batch. Now: pure SIMD, ~15 ops, no lane extracts.
            let exp_x = simd_exp2_grazing(f32x4_mul(f32x4_splat(-9.28), n_dot_v));
            let rx2 = f32x4_mul(r_x, r_x);
            let a004 = f32x4_add(f32x4_mul(f32x4_min(rx2, exp_x), r_x), r_y);
            let scale = f32x4_add(f32x4_mul(f32x4_splat(-1.04), a004), r_z);
            let bias  = f32x4_add(f32x4_mul(f32x4_splat( 1.04), a004), r_w);

            let as_r = f32x4_mul(f32x4_mul(env_spec_r, f32x4_add(f32x4_mul(f0_r, scale), bias)), occlusion);
            let as_g = f32x4_mul(f32x4_mul(env_spec_g, f32x4_add(f32x4_mul(f0_g, scale), bias)), occlusion);
            let as_b = f32x4_mul(f32x4_mul(env_spec_b, f32x4_add(f32x4_mul(f0_b, scale), bias)), occlusion);
            (ad_r, ad_g, ad_b, as_r, as_g, as_b)
        } else {
            let rough_atten = f32x4_sub(one, f32x4_mul(f32x4_splat(0.5), roughness));
            let ad_r = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_r, diffuse_r), inv_pi), occlusion);
            let ad_g = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_g, diffuse_g), inv_pi), occlusion);
            let ad_b = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_b, diffuse_b), inv_pi), occlusion);
            let as_r = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_r, f0_r), rough_atten), occlusion);
            let as_g = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_g, f0_g), rough_atten), occlusion);
            let as_b = f32x4_mul(f32x4_mul(f32x4_mul(self.ambient_b, f0_b), rough_atten), occlusion);
            (ad_r, ad_g, ad_b, as_r, as_g, as_b)
        };

        // Apply clearcoat attenuation to ambient contributions using
        // view-dependent Fresnel (Schlick with F0 = 0.04 and N·V), matching
        // how a viewer sees ambient light through the clearcoat. This is
        // more correct than the earlier primary-light approximation —
        // per-view ambient attenuation follows the same law that IBL would
        // apply if we had prefiltered clearcoat probes.
        let _ = primary_f_cc; // superseded by the N·V-based term below
        let (amb_diff_r, amb_diff_g, amb_diff_b, amb_spec_r, amb_spec_g, amb_spec_b) =
        if self.has_clearcoat {
            let cc = self.clearcoat_factor;
            // F_cc(N·V) using Schlick, F0 = 0.04 (dielectric clearcoat).
            let x = f32x4_max(zero, f32x4_sub(one, n_dot_v));
            let x2 = f32x4_mul(x, x);
            let x5 = f32x4_mul(f32x4_mul(x2, x2), x);
            let f_cc_view = f32x4_add(f32x4_splat(0.04),
                f32x4_mul(f32x4_sub(one, f32x4_splat(0.04)), x5));
            let atten = f32x4_sub(one, f32x4_mul(cc, f_cc_view));
            (
                f32x4_mul(amb_diff_r, atten), f32x4_mul(amb_diff_g, atten), f32x4_mul(amb_diff_b, atten),
                f32x4_mul(amb_spec_r, atten), f32x4_mul(amb_spec_g, atten), f32x4_mul(amb_spec_b, atten),
            )
        } else {
            (amb_diff_r, amb_diff_g, amb_diff_b, amb_spec_r, amb_spec_g, amb_spec_b)
        };

        let r = f32x4_add(f32x4_add(f32x4_add(direct_r, amb_diff_r), amb_spec_r), emit_r);
        let g = f32x4_add(f32x4_add(f32x4_add(direct_g, amb_diff_g), amb_spec_g), emit_g);
        let b = f32x4_add(f32x4_add(f32x4_add(direct_b, amb_diff_b), amb_spec_b), emit_b);

        // KHR_materials_transmission: sample the IBL env at the refraction
        // direction and blend it in based on `transmission_factor * base_alpha`.
        // Uses Snell's law with `ior_ratio = 1.0 / ior` (assumes air→material).
        // Attenuated by base color (glTF spec: transmitted light picks up the
        // base color's tint) and volume Beer-Lambert. Diffuse contribution is
        // replaced by the transmission when the factor is 1 — matches glTF
        // spec's "diffuse light is transmitted" formulation.
        //
        // We also DIM the material's own alpha by (1 − transmission factor) so
        // opaque geometry BEHIND the transmissive surface (coals under a
        // heat-dome, wine in a glass) stays visible when render.rs routes
        // transmissive OPAQUE materials into the WBOIT queue.
        let mut transmission_alpha_scale = f32x4_splat(1.0);
        let (r, g, b) = if self.has_transmission {
            if let Some(env) = self.ibl_env {
                let max_lod = env.max_lod;
                let (trans_r, trans_g, trans_b) = if self.has_dispersion {
                    // KHR_materials_dispersion: three refractions (one per RGB
                    // channel with its own IOR) and pick each channel from the
                    // corresponding env sample. Costs 3× the base transmission
                    // per pixel; gated by `has_dispersion` so opaque paths pay
                    // nothing.
                    let (rfx_r, rfy_r, rfz_r) = refract_v3(vx, vy, vz, nx_final, ny_final, nz_final, self.ior_ratio_r);
                    let (rfx_g, rfy_g, rfz_g) = refract_v3(vx, vy, vz, nx_final, ny_final, nz_final, self.ior_ratio);
                    let (rfx_b, rfy_b, rfz_b) = refract_v3(vx, vy, vz, nx_final, ny_final, nz_final, self.ior_ratio_b);
                    let mut tr_r = [0.0f32; 4];
                    let mut tr_g = [0.0f32; 4];
                    let mut tr_b = [0.0f32; 4];
                    macro_rules! lane_d { ($i:tt) => { {
                        let rough = f32x4_extract_lane::<$i>(roughness);
                        let lod = rough * max_lod;
                        tr_r[$i] = env.sample_dir(
                            f32x4_extract_lane::<$i>(rfx_r),
                            f32x4_extract_lane::<$i>(rfy_r),
                            f32x4_extract_lane::<$i>(rfz_r), lod)[0];
                        tr_g[$i] = env.sample_dir(
                            f32x4_extract_lane::<$i>(rfx_g),
                            f32x4_extract_lane::<$i>(rfy_g),
                            f32x4_extract_lane::<$i>(rfz_g), lod)[1];
                        tr_b[$i] = env.sample_dir(
                            f32x4_extract_lane::<$i>(rfx_b),
                            f32x4_extract_lane::<$i>(rfy_b),
                            f32x4_extract_lane::<$i>(rfz_b), lod)[2];
                    } } }
                    lane_d!(0); lane_d!(1); lane_d!(2); lane_d!(3);
                    (
                        f32x4(tr_r[0], tr_r[1], tr_r[2], tr_r[3]),
                        f32x4(tr_g[0], tr_g[1], tr_g[2], tr_g[3]),
                        f32x4(tr_b[0], tr_b[1], tr_b[2], tr_b[3]),
                    )
                } else {
                    let (rfx, rfy, rfz) = refract_v3(vx, vy, vz, nx_final, ny_final, nz_final, self.ior_ratio);
                    let mut tr = [[0.0f32; 3]; 4];
                    macro_rules! lane_t { ($i:tt) => { {
                        let rx = f32x4_extract_lane::<$i>(rfx);
                        let ry = f32x4_extract_lane::<$i>(rfy);
                        let rz = f32x4_extract_lane::<$i>(rfz);
                        let rough = f32x4_extract_lane::<$i>(roughness);
                        tr[$i] = env.sample_dir(rx, ry, rz, rough * max_lod);
                    } } }
                    lane_t!(0); lane_t!(1); lane_t!(2); lane_t!(3);
                    (
                        f32x4(tr[0][0], tr[1][0], tr[2][0], tr[3][0]),
                        f32x4(tr[0][1], tr[1][1], tr[2][1], tr[3][1]),
                        f32x4(tr[0][2], tr[1][2], tr[2][2], tr[3][2]),
                    )
                };
                // Tint by base color (dielectric transmission takes the base
                // color) and Beer-Lambert attenuation.
                let tinted_r = f32x4_mul(f32x4_mul(trans_r, base_r), self.volume_attenuation_r);
                let tinted_g = f32x4_mul(f32x4_mul(trans_g, base_g), self.volume_attenuation_g);
                let tinted_b = f32x4_mul(f32x4_mul(trans_b, base_b), self.volume_attenuation_b);
                // Effective factor per pixel: material factor × texture-R
                // sample × (1 − metallic) — metals never transmit.
                let factor = f32x4_mul(f32x4_mul(self.transmission_factor, s.transmission), one_minus_metallic);
                let inv_factor = f32x4_sub(one, factor);
                transmission_alpha_scale = inv_factor;
                (
                    f32x4_add(f32x4_mul(r, inv_factor), f32x4_mul(tinted_r, factor)),
                    f32x4_add(f32x4_mul(g, inv_factor), f32x4_mul(tinted_g, factor)),
                    f32x4_add(f32x4_mul(b, inv_factor), f32x4_mul(tinted_b, factor)),
                )
            } else { (r, g, b) }
        } else { (r, g, b) };

        let r = tone_map_4(r, self.tone_map, self.exp4);
        let g = tone_map_4(g, self.tone_map, self.exp4);
        let b = tone_map_4(b, self.tone_map, self.exp4);

        let alpha = f32x4_mul(alpha, transmission_alpha_scale);
        ShadeOut4 {
            r, g, b, a: alpha,
            keep: apply_mask_cutoff(alpha, self.mask_cutoff, default_keep),
        }
    }

    /// Scalar entry point — used by the rasterizer for the 0-3 scanline
    /// remainder pixels that don't fill a 4-pixel SIMD batch. Previously
    /// this had its own math implementation (via `PbrContext::shade_pixel`
    /// and per-slot scalar helpers), which diverged from `shade4` enough
    /// to leave a **visible right-edge scanline seam on every triangle**
    /// — the "hex-grid / wireframe" artifact that appeared only under IBL
    /// (bright ambient amplified the tiny per-pixel divergence).
    ///
    /// Fix: splat the scalar inputs into all 4 lanes and call `shade4`,
    /// then extract lane 0. Guarantees pixel-perfect consistency between
    /// SIMD and scalar paths.
    fn shade_scalar(&self, pos: Vec3, normal: Vec3, uv: [f32; 2], uv1: [f32; 2], uv2: [f32; 2], color: [f32; 4], tangent: [f32; 4]) -> Option<[f32; 4]> {
        let in4 = ShadeIn4 {
            pos_x: f32x4_splat(pos.x as f32),
            pos_y: f32x4_splat(pos.y as f32),
            pos_z: f32x4_splat(pos.z as f32),
            n_x:   f32x4_splat(normal.x as f32),
            n_y:   f32x4_splat(normal.y as f32),
            n_z:   f32x4_splat(normal.z as f32),
            uv_u:  f32x4_splat(uv[0]),
            uv_v:  f32x4_splat(uv[1]),
            uv1_u: f32x4_splat(uv1[0]),
            uv1_v: f32x4_splat(uv1[1]),
            uv2_u: f32x4_splat(uv2[0]),
            uv2_v: f32x4_splat(uv2[1]),
            col_r: f32x4_splat(color[0]),
            col_g: f32x4_splat(color[1]),
            col_b: f32x4_splat(color[2]),
            col_a: f32x4_splat(color[3]),
            tan_x: f32x4_splat(tangent[0]),
            tan_y: f32x4_splat(tangent[1]),
            tan_z: f32x4_splat(tangent[2]),
            tan_w: f32x4_splat(tangent[3]),
        };
        let out = self.shade4(in4);
        let keep = i32x4_extract_lane::<0>(out.keep);
        if keep == 0 { return None; }
        Some([
            f32x4_extract_lane::<0>(out.r),
            f32x4_extract_lane::<0>(out.g),
            f32x4_extract_lane::<0>(out.b),
            f32x4_extract_lane::<0>(out.a),
        ])
    }
}

/// Scalar counterpart to `refract_v3`. See it for algorithm.
#[inline]
fn refract_scalar(v: Vec3, n: Vec3, eta: f64) -> (f64, f64, f64) {
    let ndv = n.dot(v).max(0.0);
    let k = 1.0 - eta * eta * (1.0 - ndv * ndv);
    if k < 0.0 {
        return (-v.x, -v.y, -v.z);
    }
    let coef = eta * ndv - k.sqrt();
    let tx = eta * -v.x + coef * n.x;
    let ty = eta * -v.y + coef * n.y;
    let tz = eta * -v.z + coef * n.z;
    let len = (tx * tx + ty * ty + tz * tz).sqrt().max(1e-12);
    (tx / len, ty / len, tz / len)
}

/// Apply the material's alpha cutoff to a lane mask.
#[inline]
fn apply_mask_cutoff(alpha_v: v128, cutoff: Option<f32>, default_keep: v128) -> v128 {
    match cutoff {
        None => default_keep,
        Some(c) => f32x4_ge(alpha_v, f32x4_splat(c)),
    }
}

// ---------------------------------------------------------------------------
// SIMD vec3 helpers
// ---------------------------------------------------------------------------

#[inline(always)]
fn dot_v3(ax: v128, ay: v128, az: v128, bx: v128, by: v128, bz: v128) -> v128 {
    f32x4_add(f32x4_add(f32x4_mul(ax, bx), f32x4_mul(ay, by)), f32x4_mul(az, bz))
}

#[inline(always)]
fn mix4(a: v128, b: v128, t: v128) -> v128 {
    f32x4_add(a, f32x4_mul(f32x4_sub(b, a), t))
}

/// Extract one lane's world-space `(pos, normal)` and sample the given
/// shadow map. Called from the SIMD light loop — a scalar path per lane,
/// which is fine given PCF cost dominates.
#[inline(always)]
fn shadow_lane<const L: usize>(
    in_: &ShadeIn4,
    shadow: &maquette_core::shadow::LightShadow,
    bias: maquette_core::shadow::BiasParams,
    softness: usize,
    pcss_light_size: f64,
) -> f32
where std::arch::wasm32::v128: Sized,
{
    let pos = Vec3::new(
        f32x4_extract_lane::<L>(in_.pos_x) as f64,
        f32x4_extract_lane::<L>(in_.pos_y) as f64,
        f32x4_extract_lane::<L>(in_.pos_z) as f64,
    );
    let normal = Vec3::new(
        f32x4_extract_lane::<L>(in_.n_x) as f64,
        f32x4_extract_lane::<L>(in_.n_y) as f64,
        f32x4_extract_lane::<L>(in_.n_z) as f64,
    );
    if pcss_light_size > 0.0 {
        shadow.lit_pcss(pos, normal, &bias, softness, pcss_light_size)
    } else {
        shadow.lit(pos, normal, &bias, softness)
    }
}

/// Clearcoat's tangent-space normal, in world space. Uses geometric N when
/// there's no clearcoat-normal texture (spec: clearcoat does NOT inherit the
/// base normal-map perturbation).
#[inline]
fn compute_clearcoat_normal(sh: &MaterialShader, in_: &ShadeIn4) -> (v128, v128, v128) {
    let Some(t) = sh.clearcoat_normal_tex else {
        return (in_.n_x, in_.n_y, in_.n_z);
    };
    let mut cc = [[0.0f32; 3]; 4];
    macro_rules! lane { ($i:tt) => { {
        let uv0 = [f32x4_extract_lane::<$i>(in_.uv_u),  f32x4_extract_lane::<$i>(in_.uv_v)];
        let uv1 = [f32x4_extract_lane::<$i>(in_.uv1_u), f32x4_extract_lane::<$i>(in_.uv1_v)];
        let uv2 = [f32x4_extract_lane::<$i>(in_.uv2_u), f32x4_extract_lane::<$i>(in_.uv2_v)];
        let uv_raw = match sh.material.texcoord_clearcoat_normal { 2 => uv2, 1 => uv1, _ => uv0 };
        let s = t.sample_lod(sh.material.xform_clearcoat_normal.apply(uv_raw), sh.lod_clearcoat_normal);
        cc[$i] = [s[0] * 2.0 - 1.0, s[1] * 2.0 - 1.0, s[2] * 2.0 - 1.0];
    } } }
    lane!(0); lane!(1); lane!(2); lane!(3);
    let ccx = f32x4(cc[0][0], cc[1][0], cc[2][0], cc[3][0]);
    let ccy = f32x4(cc[0][1], cc[1][1], cc[2][1], cc[3][1]);
    let ccz = f32x4(cc[0][2], cc[1][2], cc[2][2], cc[3][2]);
    let lx = f32x4_mul(ccx, sh.clearcoat_normal_scale);
    let ly = f32x4_mul(ccy, sh.clearcoat_normal_scale);
    let lz = ccz;
    let bx = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_y, in_.tan_z), f32x4_mul(in_.n_z, in_.tan_y)), in_.tan_w);
    let by = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_z, in_.tan_x), f32x4_mul(in_.n_x, in_.tan_z)), in_.tan_w);
    let bz = f32x4_mul(f32x4_sub(f32x4_mul(in_.n_x, in_.tan_y), f32x4_mul(in_.n_y, in_.tan_x)), in_.tan_w);
    let nx = f32x4_add(f32x4_add(f32x4_mul(in_.tan_x, lx), f32x4_mul(bx, ly)), f32x4_mul(in_.n_x, lz));
    let ny = f32x4_add(f32x4_add(f32x4_mul(in_.tan_y, lx), f32x4_mul(by, ly)), f32x4_mul(in_.n_y, lz));
    let nz = f32x4_add(f32x4_add(f32x4_mul(in_.tan_z, lx), f32x4_mul(bz, ly)), f32x4_mul(in_.n_z, lz));
    normalize_v3(nx, ny, nz)
}

/// Returns per-pixel `(L_x, L_y, L_z, atten_r, atten_g, atten_b)` for a
/// splatted light. Directional lights → constant direction, colour used as
/// atten. Point/Spot → position-based direction + inverse-square falloff +
/// range smoothstep + (for Spot) cone smoothstep.
#[inline(always)]
fn resolve_light_dir_and_atten(
    light: &SplattedLight,
    px: v128, py: v128, pz: v128,
) -> (v128, v128, v128, v128, v128, v128) {
    use crate::scene::LightKind;
    let zero = f32x4_splat(0.0);
    let one  = f32x4_splat(1.0);
    match light.kind {
        LightKind::Directional => {
            // L = -direction (surface toward light).
            (
                f32x4_sub(zero, light.dx),
                f32x4_sub(zero, light.dy),
                f32x4_sub(zero, light.dz),
                light.cr, light.cg, light.cb,
            )
        }
        LightKind::Point | LightKind::Spot => {
            // to_light = light.pos - surface_pos
            let tx = f32x4_sub(light.px, px);
            let ty = f32x4_sub(light.py, py);
            let tz = f32x4_sub(light.pz, pz);
            let dist2 = f32x4_add(f32x4_add(f32x4_mul(tx, tx), f32x4_mul(ty, ty)), f32x4_mul(tz, tz));
            let inv_dist = f32x4_div(one, f32x4_sqrt(f32x4_max(dist2, f32x4_splat(1e-8))));
            let lx = f32x4_mul(tx, inv_dist);
            let ly = f32x4_mul(ty, inv_dist);
            let lz = f32x4_mul(tz, inv_dist);
            // Inverse-square with soft range cutoff (glTF recommendation):
            // atten = 1/dist² · smoothstep(1, 0, (dist·inv_range)^4)^2
            let dist_atten = f32x4_div(one, f32x4_max(dist2, f32x4_splat(0.01 * 0.01)));
            // Range cutoff: fade out as distance approaches range.
            let dr = f32x4_mul(f32x4_sqrt(dist2), light.range_inv);
            let dr4 = f32x4_mul(f32x4_mul(dr, dr), f32x4_mul(dr, dr));
            let cutoff = f32x4_max(zero, f32x4_min(one, f32x4_sub(one, dr4)));
            // If range=0 (light.range_inv=0), dr=0, cutoff=1 → no cutoff.
            let mut atten = f32x4_mul(dist_atten, f32x4_mul(cutoff, cutoff));

            if light.kind == LightKind::Spot {
                // cos_theta = dot(-L, spot_direction) = dot(direction, from_light_to_surface)
                //           = dot(direction, -L)
                let cos_theta = f32x4_max(zero, f32x4_sub(zero,
                    f32x4_add(f32x4_add(
                        f32x4_mul(light.dx, lx), f32x4_mul(light.dy, ly)), f32x4_mul(light.dz, lz))));
                let cone = f32x4_max(zero, f32x4_min(one,
                    f32x4_add(f32x4_mul(cos_theta, light.cone_scale), light.cone_offset)));
                atten = f32x4_mul(atten, cone);
            }

            (
                lx, ly, lz,
                f32x4_mul(light.cr, atten),
                f32x4_mul(light.cg, atten),
                f32x4_mul(light.cb, atten),
            )
        }
    }
}

/// Snell's law refraction: `T = eta·I + (eta·(N·I) − √(1 − eta²·(1 − (N·I)²)))·N`
/// where I is the *incident* direction (into the surface) and N points *out*.
/// Callers pass V (view = eye − pos) so we negate to get I = −V. Returns the
/// unit refraction direction, or reflection when total internal reflection
/// would occur (√ negative). SIMD, 4 lanes.
#[inline(always)]
fn refract_v3(vx: v128, vy: v128, vz: v128, nx: v128, ny: v128, nz: v128, eta: v128) -> (v128, v128, v128) {
    let one = f32x4_splat(1.0);
    let zero = f32x4_splat(0.0);
    // I = −V, so N·I = −N·V. Keep as +N·V for max(0, ·) shape then negate.
    let ndv = dot_v3(nx, ny, nz, vx, vy, vz);
    let cos_i = f32x4_max(zero, ndv); // clamp to avoid domain wander on TIR edges
    // k = 1 − eta² · (1 − cos_i²)
    let k = f32x4_sub(one, f32x4_mul(f32x4_mul(eta, eta), f32x4_sub(one, f32x4_mul(cos_i, cos_i))));
    let tir = f32x4_lt(k, zero);
    let sqrt_k = f32x4_sqrt(f32x4_max(k, zero));
    // T = eta · I + (eta · cos_i − √k) · N,   with I = −V
    let coef = f32x4_sub(f32x4_mul(eta, cos_i), sqrt_k);
    let tx = f32x4_add(f32x4_mul(eta, f32x4_sub(zero, vx)), f32x4_mul(coef, nx));
    let ty = f32x4_add(f32x4_mul(eta, f32x4_sub(zero, vy)), f32x4_mul(coef, ny));
    let tz = f32x4_add(f32x4_mul(eta, f32x4_sub(zero, vz)), f32x4_mul(coef, nz));
    // On TIR, fall back to −V (straight-through — visually acceptable for
    // typical thin-walled glass and cheaper than a reflection compute).
    let out_x = v128_bitselect(f32x4_sub(zero, vx), tx, tir);
    let out_y = v128_bitselect(f32x4_sub(zero, vy), ty, tir);
    let out_z = v128_bitselect(f32x4_sub(zero, vz), tz, tir);
    normalize_v3(out_x, out_y, out_z)
}

/// Per-lane scalar cosine — wasm SIMD has no vector cos. Only called from
/// the iridescence branch which is off by default, so the scalar fallback
/// hit rate is near zero.
#[inline]
fn per_lane_cos(v: v128) -> v128 {
    f32x4(
        f32x4_extract_lane::<0>(v).cos(),
        f32x4_extract_lane::<1>(v).cos(),
        f32x4_extract_lane::<2>(v).cos(),
        f32x4_extract_lane::<3>(v).cos(),
    )
}

/// SIMD `2^x` for `x` in roughly `[-10, 0]` — the range the Karis split-sum
/// grazing term hits (`x = -9.28 · NoV`, `NoV ∈ [0, 1]`). Was a per-lane
/// scalar `.exp2()` (extract, libcall, repack — 4 lane ops + 4 transcendental
/// calls per 4-pixel batch, hot path in every IBL pixel). Now: pure SIMD via
/// exponent-bit assembly + 5th-degree polynomial for the fractional part.
///
/// Error bound over the hit range is < 1e-5 vs the libm `exp2` — well below
/// the 8-bit sRGB quantization the shader writes to, and orders below the
/// tone-map's own polynomial error.
///
/// Polynomial coefficients: minimax fit for `2^x` on `[0, 1]` (Chebyshev
/// truncation of the Taylor series in `ln 2`; degrees 0-5).
#[inline]
fn simd_exp2_grazing(x: v128) -> v128 {
    let ix_f  = f32x4_floor(x);
    let fx    = f32x4_sub(x, ix_f);
    // 2^ix — reinterpret `(ix + 127) << 23` as f32 (IEEE 754 bias trick).
    // For our x-range `ix ∈ [-10, 0]`, `ix + 127 ∈ [117, 127]` — safely inside
    // the exponent field, no NaN/denormal edge cases.
    let ix_i    = i32x4_trunc_sat_f32x4(ix_f);
    let biased  = i32x4_add(ix_i, i32x4_splat(127));
    let pow_int = i32x4_shl(biased, 23);   // v128 layout matches f32x4
    // 2^fx via Horner on the fractional part in [0, 1].
    let c0 = f32x4_splat(1.0);
    let c1 = f32x4_splat(0.693_147_2);
    let c2 = f32x4_splat(0.240_226_5);
    let c3 = f32x4_splat(0.055_504_1);
    let c4 = f32x4_splat(0.009_681_2);
    let c5 = f32x4_splat(0.001_333_5);
    let p = c5;
    let p = f32x4_add(c4, f32x4_mul(p, fx));
    let p = f32x4_add(c3, f32x4_mul(p, fx));
    let p = f32x4_add(c2, f32x4_mul(p, fx));
    let p = f32x4_add(c1, f32x4_mul(p, fx));
    let p = f32x4_add(c0, f32x4_mul(p, fx));
    f32x4_mul(pow_int, p)
}

#[inline(always)]
fn normalize_v3(vx: v128, vy: v128, vz: v128) -> (v128, v128, v128) {
    let len2 = f32x4_add(f32x4_add(f32x4_mul(vx, vx), f32x4_mul(vy, vy)), f32x4_mul(vz, vz));
    let inv = f32x4_div(f32x4_splat(1.0), f32x4_sqrt(f32x4_max(len2, f32x4_splat(1e-14))));
    (f32x4_mul(vx, inv), f32x4_mul(vy, inv), f32x4_mul(vz, inv))
}

// Volume/Beer-Lambert attenuation now lives on `Material::precomp` — filled
// at scene flatten via `MaterialPrecomp::from_material` (see scene.rs).

// Scalar BRDF terms (used by PbrContext::shade_pixel)
// ---------------------------------------------------------------------------

#[inline]
fn ggx_d(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * denom * denom).max(1e-7)
}

#[inline]
fn smith_v(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let ggx_v = n_dot_l * ((n_dot_v * n_dot_v * (1.0 - a2)) + a2).sqrt();
    let ggx_l = n_dot_v * ((n_dot_l * n_dot_l * (1.0 - a2)) + a2).sqrt();
    0.5 / (ggx_v + ggx_l).max(1e-7)
}

#[inline]
fn fresnel_schlick(v_dot_h: f32, f0: [f32; 3]) -> [f32; 3] {
    let x = (1.0 - v_dot_h).max(0.0);
    let x5 = x * x * x * x * x;
    [
        f0[0] + (1.0 - f0[0]) * x5,
        f0[1] + (1.0 - f0[1]) * x5,
        f0[2] + (1.0 - f0[2]) * x5,
    ]
}

// ---------------------------------------------------------------------------
// Scalar vec3 helpers
// ---------------------------------------------------------------------------

#[inline] fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline] fn normalize_in_place(v: &mut [f32; 3]) {
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if len2 > 1e-14 {
        let inv = len2.sqrt().recip();
        v[0] *= inv; v[1] *= inv; v[2] *= inv;
    }
}

#[inline] fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + t * (b[0] - a[0]),
     a[1] + t * (b[1] - a[1]),
     a[2] + t * (b[2] - a[2])]
}

#[inline] fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
