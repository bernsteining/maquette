use crate::math::FloatExt;
use crate::annotations;
use crate::cache;
use crate::clip;
use crate::color_map;
use crate::config::{LightKind, RenderConfig, ShadowConfig, GroupStyles};
use crate::decimate;
use crate::explode;
use crate::color::{linear_to_srgb, parse_hex_color, srgb_to_linear};
use crate::math::{quantize, fx_hashmap_cap, FxBuildHasher, FxHashMap, Mat4, Vec3, ViewMatSimd};
use crate::outline;
use crate::parser::Triangle;
use maquette_core::effects::{raw_raster, Bloom, Fog, Glow, PostEffects};
use maquette_core::rasterizer::PixelBuffer;
use maquette_core::ssao::SSAOParams;
use maquette_core::cache::{Global, KeyedCache};
use maquette_core::math::FxHasher;
use crate::smooth;
use crate::projection::*;
use crate::shading::*;
use crate::svg::*;
#[cfg(target_arch = "wasm32")] use std::arch::wasm32::*; #[cfg(not(target_arch = "wasm32"))] use maquette_core::simd::*;
use std::collections::HashSet;


const NO_SOURCE: u32 = u32::MAX;

/// Most supersampled samples a raster render fits in wasm32 memory (measured:
/// 400 M renders, 484 M runs out).
const MAX_RASTER_SAMPLES: u64 = 400_000_000;

struct ProjectedTri {
    pts: [(f64, f64); 3],
    depths: [f64; 3],
    depth: f64,
    r: u8,
    g: u8,
    b: u8,
    /// Per-vertex colors for smooth shading (None = flat shading).
    vertex_colors: Option<[(u8, u8, u8); 3]>,
    /// Group ID carried from Triangle, for per-group appearance lookup.
    group_id: Option<u32>,
    /// Opacity (0.0–1.0). 1.0 = fully opaque.
    opacity: f64,
    /// Index of the source triangle in the projected slice (for per-pixel
    /// shadows and textures), or `NO_SOURCE` for synthetic geometry.
    src: u32,
    splat: bool,
}


pub(crate) use maquette_core::math::{bbox_center, bbox_of, bbox_radius};

fn compute_bbox(triangles: &[Triangle]) -> (Vec3, Vec3) {
    let mut min = Vec3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut max = Vec3::new(f64::MIN, f64::MIN, f64::MIN);
    for t in triangles {
        for &v in &t.vertices { maquette_core::math::bbox_extend(&mut min, &mut max, v); }
    }
    (min, max)
}

/// Everything the shade paths need to apply cast shadows. `maps` has one entry
/// per light (None = non-caster); `factors` is the per-unique-vertex×light
/// attenuation for the smooth paths (None under flat shading, which samples the
/// maps per-face on the fly).
struct ShadowData {
    maps: Vec<Option<crate::shadow::LightShadow>>,
    bias: crate::shadow::BiasParams,
    strength: f32,
    softness: usize,
    factors: Option<Vec<f32>>,
    /// Per-pixel sampling active (PNG path only). When true, `factors` is None
    /// and the raster pass samples the maps per fragment via `ProjectedTri.pp`.
    per_pixel: bool,
    /// Shadow tint in linear-ish sRGB u8 (None = neutral).
    tint: Option<(u8, u8, u8)>,
    /// PCSS light size in world units, per light (0 = uniform PCF). Area lights
    /// use their own radius; other lights fall back to the global `light_size`.
    light_sizes: Vec<f64>,
    /// Ambient light fraction — the brightness a fully shadowed pixel keeps.
    ambient_keep_base: f32,
}

impl ShadowData {
    /// Lit multiplier (1 = lit, 0 = shadowed) for light `li` at a point/normal.
    /// Used by the flat-shading path, which has no precomputed vertex factors.
    #[inline]
    fn sample(&self, li: usize, p: Vec3, normal: Vec3) -> f32 {
        match &self.maps[li] {
            Some(map) => 1.0 - self.strength * (1.0 - map.lit(p, normal, &self.bias, self.softness)),
            None => 1.0,
        }
    }

    /// Aggregate geometric lit factor (0 = shadowed, 1 = lit) across all casting
    /// lights at a world point. Used by the per-pixel raster path.
    #[inline]
    fn pp_factor(&self, p: Vec3, normal: Vec3) -> f32 {
        let mut sum = 0.0f32;
        let mut n = 0u32;
        for (li, map) in self.maps.iter().enumerate() {
            if let Some(m) = map {
                sum += m.lit_sized(p, normal, &self.bias, self.softness, self.light_sizes[li]);
                n += 1;
            }
        }
        if n == 0 { 1.0 } else { sum / n as f32 }
    }

    /// Final per-pixel color: darken toward the (optionally tinted) ambient floor
    /// by the shadow factor at `p`.
    #[inline]
    fn pp_shade(&self, c: (u8, u8, u8), p: Vec3, normal: Vec3) -> (u8, u8, u8) {
        let t = self.pp_factor(p, normal);
        let keep = 1.0 - self.strength * (1.0 - self.ambient_keep_base);
        let (tr, tg, tb) = self.tint
            .map(|(r, g, b)| (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
            .unwrap_or((1.0, 1.0, 1.0));
        let chan = |cc: u8, tint: f32| {
            let m = keep * tint;
            let mul = m + t * (1.0 - m);
            (cc as f32 * mul).fround().clamp(0.0, 255.0) as u8
        };
        (chan(c.0, tr), chan(c.1, tg), chan(c.2, tb))
    }
}

/// Build shadow maps + per-vertex factors. Returns None when shadows are
/// disabled or there are no lights. Camera-independent, so it can be reused
/// across all views of a grid/turntable.
fn build_shadow_data(
    triangles: &[Triangle],
    lights: &[ResolvedLight],
    smooth: Option<&smooth::SmoothData>,
    config: &RenderConfig,
    group_styles: &GroupStyles,
    bc: Vec3,
    br: f64,
    allow_per_pixel: bool,
) -> Option<ShadowData> {
    let cfg = config.shadows.as_ref()?;
    if cfg.strength <= 0.0 || lights.is_empty() || config.shading == "unlit" {
        return None;
    }
    let per_pixel = cfg.per_pixel && allow_per_pixel;
    let tint = if cfg.color.is_empty() { None } else { Some(parse_hex_color(&cfg.color)) };
    let up = Vec3::from(config.up);
    let global_opacity = config.opacity;
    let is_occluder = |tri: &Triangle| -> bool {
        let o = tri.group_id
            .and_then(|gid| group_styles.get(&gid))
            .and_then(|a| a.opacity)
            .unwrap_or(global_opacity);
        o >= 0.5
    };
    let maps = crate::shadow::build_shadow_maps(triangles, lights, bc, br, up, cfg, &is_occluder);
    let bias = crate::shadow::BiasParams { bias: cfg.bias, normal_bias: cfg.normal_bias, slope_bias: cfg.slope_bias };
    let strength = cfg.strength as f32;

    let factors = if per_pixel {
        None
    } else {
        smooth.map(|sd| {
            let n_unique = sd.positions.len();
            let mut factors = vec![1.0f32; maps.len() * n_unique];
            for (li, map) in maps.iter().enumerate() {
                let Some(map) = map else { continue };
                let base = li * n_unique;
                for (vi, p) in sd.positions.iter().enumerate() {
                    let lit = map.lit(*p, sd.normals[vi], &bias, cfg.softness);
                    factors[base + vi] = 1.0 - strength * (1.0 - lit);
                }
            }
            factors
        })
    };

    let ambient_keep_base = (config.ambient.intensity as f32).clamp(0.0, 1.0);
    let light_sizes: Vec<f64> = lights.iter()
        .map(|l| if l.kind == LightKind::Area { l.size } else { cfg.light_size })
        .collect();
    Some(ShadowData { maps, bias, strength, softness: cfg.softness, factors, per_pixel, tint, light_sizes, ambient_keep_base })
}

fn shadow_cache_key(
    prep_base: u64,
    has_smooth: bool,
    lights: &[ResolvedLight],
    config: &RenderConfig,
    group_styles: &GroupStyles,
    allow_per_pixel: bool,
) -> Option<u64> {
    let cfg = config.shadows.as_ref()?;
    let mut h = FxHasher::seeded(prep_cache_key(prep_base, config)).mix(0x5AD0).mix(has_smooth as u64 | (allow_per_pixel as u64) << 1);
    for l in lights {
        h = h.mix(l.kind as u64);
        for v in [l.vector.x, l.vector.y, l.vector.z, l.size] { h = h.mix_f64(v); }
        h = h.mix(l.cast_shadow as u64);
    }
    for v in [cfg.bias, cfg.normal_bias, cfg.slope_bias, cfg.strength, cfg.light_size, config.opacity, config.ambient.intensity] {
        h = h.mix_f64(v);
    }
    for &u in &config.up { h = h.mix_f64(u); }
    h = h.mix(cfg.resolution as u64).mix(cfg.softness as u64).mix(cfg.per_pixel as u64 | (cfg.omni as u64) << 1).mix_str(&cfg.color);
    let groups = group_styles.iter().fold(0u64, |acc, (gid, g)| {
        acc ^ FxHasher::seeded(0xcbf2_9ce4_8422_2325).mix(*gid as u64).mix(g.opacity.map_or(u64::MAX, f64::to_bits)).key()
    });
    Some(h.mix(groups).key())
}

fn shadow_data_for<'a>(
    owned: &'a mut Option<ShadowData>,
    prep_key: Option<u64>,
    tris: &[Triangle],
    lights: &[ResolvedLight],
    smooth: Option<&smooth::SmoothData>,
    config: &RenderConfig,
    group_styles: &GroupStyles,
    bc: Vec3,
    br: f64,
    allow_per_pixel: bool,
) -> Option<&'a ShadowData> {
    static CACHE: Global<KeyedCache<Option<ShadowData>>> = Global::new(KeyedCache::new(2));
    if config.shading == "unlit" {
        return None;
    }
    let build = || build_shadow_data(tris, lights, smooth, config, group_styles, bc, br, allow_per_pixel);
    let Some(key) = prep_key.and_then(|p| shadow_cache_key(p, smooth.is_some(), lights, config, group_styles, allow_per_pixel)) else {
        *owned = build();
        return owned.as_ref();
    };
    CACHE.get().get_or_insert_with(key, build).as_ref()
}

/// `[(i / 255)^p for i in 0..256]`, memoized per exponent across calls: each
/// table costs 256 software `powf`, and a document re-renders with the same
/// shininess / fresnel / SSS exponents over and over.
fn area_spec_lut(sh: f32, spread: f64) -> [f32; 256] {
    if spread <= 0.0 { return pow_lut(sh); }
    let a = (2.0 / (sh as f64 + 2.0)).sqrt();
    let a2 = (a + spread).fmin(1.0);
    let sh2 = (2.0 / (a2 * a2) - 2.0) as f32;
    let k = ((a / a2) * (a / a2)) as f32;
    pow_lut(sh2).map(|x| x * k)
}

fn pow_lut(p: f32) -> [f32; 256] {
    static CACHE: Global<KeyedCache<[f32; 256]>> = Global::new(KeyedCache::new(32));
    *CACHE.get().get_or_insert_with(p.to_bits() as u64, || {
        let mut lut = [0.0f32; 256];
        for i in 0..256 { lut[i] = (i as f32 / 255.0).powf(p); }
        lut
    })
}

fn project_triangles(
    triangles: &[Triangle],
    smooth: Option<&smooth::SmoothData>,
    config: &RenderConfig,
    view: &ViewParams,
    vw: f64,
    vh: f64,
    br: f64,
    force_ortho: bool,
    group_styles: &GroupStyles,
    lights: &[ResolvedLight],
    shadow: Option<&ShadowData>,
) -> Vec<ProjectedTri> {
    let shadow_factors: Option<&[f32]> = shadow.and_then(|s| s.factors.as_deref());
    let proj = if force_ortho { Projection::Ortho } else { resolve_projection(&config.projection) };
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let view_simd = ViewMatSimd::from_mat4(&view_mat);
    let seam_sq = seam_limit(&proj_setup).map(|l| l * l);
    let (base_r, base_g, base_b) = parse_hex_color(&config.color);
    let is_wireframe = config.mode == "wireframe";
    let is_xray = config.mode == "x-ray";
    let skip_cull = matches!(proj, Projection::Cabinet | Projection::Cavalier | Projection::Military | Projection::TinyPlanet);
    let one_sided = config.cull_backface;
    let do_cull = config.cull_backface && !is_wireframe && !is_xray && !skip_cull && config.explode.abs() < 1e-12;

    let face_back: Vec<bool> = if do_cull || is_xray {
        let pv = if proj == Projection::Ortho { Some(view.camera.sub(view.center)) } else { None };
        triangles.iter().map(|tri| {
            let (dx, dy, dz) = if let Some(v) = pv {
                (v.x, v.y, v.z)
            } else {
                let sx = tri.vertices[0].x + tri.vertices[1].x + tri.vertices[2].x;
                let sy = tri.vertices[0].y + tri.vertices[1].y + tri.vertices[2].y;
                let sz = tri.vertices[0].z + tri.vertices[1].z + tri.vertices[2].z;
                (view.camera.x * 3.0 - sx, view.camera.y * 3.0 - sy, view.camera.z * 3.0 - sz)
            };
            tri.normal.x * dx + tri.normal.y * dy + tri.normal.z * dz <= 0.0
        }).collect()
    } else {
        Vec::new()
    };
    let tm = ToneMap::parse(&config.tone_mapping.method);
    let shading = match config.shading.as_str() {
        "gooch" => ShadingMode::Gooch, "cel" => ShadingMode::Cel,
        "flat" => ShadingMode::Flat, "normal" => ShadingMode::Normal, "unlit" => ShadingMode::Unlit, _ => ShadingMode::BlinnPhong,
    };
    let (gooch_warm, gooch_cool) = if shading == ShadingMode::Gooch {
        let [wr, wg, wb] = maquette_core::color::hex_to_linear(&config.gooch_warm);
        let [cr, cg, cb] = maquette_core::color::hex_to_linear(&config.gooch_cool);
        let (w, c) = ((wr, wg, wb), (cr, cg, cb));
        (w, c)
    } else {
        ((0.0f32, 0.0f32, 0.0f32), (0.0f32, 0.0f32, 0.0f32))
    };

    let fresnel_lut = if !is_wireframe && config.fresnel.intensity > 0.0 {
        pow_lut(config.fresnel.power as f32)
    } else {
        [0.0f32; 256]
    };
    let (sss_lut, sss_intensity, sss_dist) = if let Some(ref sc) = config.sss {
        (pow_lut(sc.power as f32), sc.intensity as f32, sc.distortion as f32)
    } else {
        ([0.0f32; 256], 0.0f32, 0.0f32)
    };
    let cfg_fresnel = config.fresnel.intensity as f32;
    let cfg_gamma = config.gamma_correction;
    let cfg_exposure = config.tone_mapping.exposure as f32;
    let cfg_cel_bands = config.cel_bands;
    let cfg_xray_opacity = config.xray_opacity;
    let cfg_ambient_intensity = config.ambient.intensity as f32;
    let (sky_r8, sky_g8, sky_b8) = parse_hex_color(&config.ambient.sky);
    let (gnd_r8, gnd_g8, gnd_b8) = parse_hex_color(&config.ambient.ground);
    let amb_sky = (
        sky_r8 as f32 / 255.0 * cfg_ambient_intensity,
        sky_g8 as f32 / 255.0 * cfg_ambient_intensity,
        sky_b8 as f32 / 255.0 * cfg_ambient_intensity,
    );
    let amb_gnd = (
        gnd_r8 as f32 / 255.0 * cfg_ambient_intensity,
        gnd_g8 as f32 / 255.0 * cfg_ambient_intensity,
        gnd_b8 as f32 / 255.0 * cfg_ambient_intensity,
    );
    let up_f32 = (config.up[0] as f32, config.up[1] as f32, config.up[2] as f32);
    let cfg_specular = config.specular as f32;
    let cfg_shininess = config.shininess as f32;
    let view_camera = view.camera;

    let area_spread: Vec<f64> = lights.iter().map(|l| {
        if l.kind == LightKind::Area && l.size > 0.0 {
            l.size / (2.0 * l.vector.sub(view.center).length().fmax(1e-3))
        } else { 0.0 }
    }).collect();
    let zero_luts = vec![[0.0f32; 256]; lights.len()];
    fn lut_ref<'a>(luts: &'a [(f32, Vec<[f32; 256]>)], zero: &'a [[f32; 256]], sh: f32) -> &'a [[f32; 256]] {
        luts.iter().find(|(s, _)| *s == sh).map(|(_, l)| &l[..]).unwrap_or(zero)
    }
    let spec_luts: Vec<(f32, Vec<[f32; 256]>)> = {
        let mut v: Vec<(f32, Vec<[f32; 256]>)> = Vec::new();
        if cfg_specular > 0.0 || group_styles.values().any(|a| a.specular.map_or(false, |s| s > 0.0)) {
            let mut add = |sh: f32| {
                if !v.iter().any(|(s, _)| *s == sh) {
                    v.push((sh, area_spread.iter().map(|&spread| area_spec_lut(sh, spread)).collect()));
                }
            };
            add(cfg_shininess);
            for a in group_styles.values() {
                if let Some(sh) = a.shininess { add(sh as f32); }
            }
        }
        v
    };
    let lights_f32: Vec<LightF32> = lights.iter().map(|l| LightF32 {
        kind: l.kind,
        dx: l.vector.x as f32, dy: l.vector.y as f32, dz: l.vector.z as f32,
        cr: l.color.0, cg: l.color.1, cb: l.color.2,
        ref_d2: l.ref_d2 as f32,
        fx: l.facing.x as f32, fy: l.facing.y as f32, fz: l.facing.z as f32,
    }).collect();

    #[inline(always)]
    fn hemi_ambient(n: Vec3, sky: (f32, f32, f32), gnd: (f32, f32, f32), up: (f32, f32, f32)) -> (f32, f32, f32) {
        let t = (n.x as f32 * up.0 + n.y as f32 * up.1 + n.z as f32 * up.2 + 1.0) * 0.5;
        (gnd.0 + (sky.0 - gnd.0) * t, gnd.1 + (sky.1 - gnd.1) * t, gnd.2 + (sky.2 - gnd.2) * t)
    }

    let shade_cache: Option<Vec<(u8, u8, u8)>> = if shading == ShadingMode::Unlit {
        None
    } else if let Some(sd) = smooth {
        let groups_uniform = group_styles.values().all(|a|
            a.specular.is_none() && a.shininess.is_none() && a.ambient.is_none());
        let can_memoize = !is_wireframe && !is_xray && groups_uniform
            && triangles.iter().all(|t| t.color.is_none() && t.vertex_colors.is_none() && t.tex.is_none());
        if can_memoize {
            let spec_lut = lut_ref(&spec_luts, &zero_luts, cfg_shininess);
            let one_minus_ambient = 1.0 - cfg_ambient_intensity;
            let n_unique = sd.normals.len();

            let use_simd = matches!(shading, ShadingMode::BlinnPhong | ShadingMode::Flat | ShadingMode::Cel | ShadingMode::Gooch);
            let simd_cel_bands = if shading == ShadingMode::Cel { cfg_cel_bands } else { 0 };
            let simd_gooch = shading == ShadingMode::Gooch;
            let cache = if use_simd && n_unique >= 4 {
                let (blr, blg, blb) = if cfg_gamma || simd_gooch {
                    (srgb_to_linear(base_r), srgb_to_linear(base_g), srgb_to_linear(base_b))
                } else {
                    (base_r as f32, base_g as f32, base_b as f32)
                };
                let ids: Vec<usize> = if do_cull && shadow_factors.is_none() {
                    let mut mask = vec![false; n_unique];
                    for (ti, &is_back) in face_back.iter().enumerate() {
                        if !is_back {
                            let [a, b2, c] = sd.tri_indices[ti];
                            mask[a] = true; mask[b2] = true; mask[c] = true;
                        }
                    }
                    (0..n_unique).filter(|&i| mask[i]).collect()
                } else {
                    (0..n_unique).collect()
                };
                let m = ids.len();
                let mut snx: Vec<f32> = Vec::with_capacity(m);
                let mut sny: Vec<f32> = Vec::with_capacity(m);
                let mut snz: Vec<f32> = Vec::with_capacity(m);
                let mut spx: Vec<f32> = Vec::with_capacity(m);
                let mut spy: Vec<f32> = Vec::with_capacity(m);
                let mut spz: Vec<f32> = Vec::with_capacity(m);
                for &i in &ids {
                    snx.push(sd.normals[i].x as f32);
                    sny.push(sd.normals[i].y as f32);
                    snz.push(sd.normals[i].z as f32);
                    spx.push(sd.positions[i].x as f32);
                    spy.push(sd.positions[i].y as f32);
                    spz.push(sd.positions[i].z as f32);
                }
                let cam_x = view_camera.x as f32;
                let cam_y = view_camera.y as f32;
                let cam_z = view_camera.z as f32;
                let n_batches = m / 4;
                let n_lights = lights_f32.len();
                let mut sh_scratch: Vec<v128> = vec![f32x4_splat(1.0); n_lights];
                let mut cache: Vec<(u8, u8, u8)> = vec![(0u8, 0u8, 0u8); n_unique];
                for bi in 0..n_batches {
                    let b = bi * 4;
                    let sh4: Option<&[v128]> = shadow_factors.map(|f| {
                        for li in 0..n_lights {
                            sh_scratch[li] = unsafe { v128_load(f.as_ptr().add(li * n_unique + b) as *const v128) };
                        }
                        &sh_scratch[..]
                    });
                    let nx4 = unsafe { v128_load(snx.as_ptr().add(b) as *const v128) };
                    let ny4 = unsafe { v128_load(sny.as_ptr().add(b) as *const v128) };
                    let nz4 = unsafe { v128_load(snz.as_ptr().add(b) as *const v128) };
                    let px4 = unsafe { v128_load(spx.as_ptr().add(b) as *const v128) };
                    let py4 = unsafe { v128_load(spy.as_ptr().add(b) as *const v128) };
                    let pz4 = unsafe { v128_load(spz.as_ptr().add(b) as *const v128) };
                    let colors = shade_batch_4(
                        nx4, ny4, nz4, px4, py4, pz4,
                        blr, blg, blb,
                        &lights_f32, cam_x, cam_y, cam_z,
                        amb_sky.0, amb_sky.1, amb_sky.2,
                        amb_gnd.0, amb_gnd.1, amb_gnd.2,
                        up_f32.0, up_f32.1, up_f32.2,
                        one_minus_ambient, cfg_specular, cfg_fresnel,
                        cfg_gamma, tm, cfg_exposure,
                        spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        simd_cel_bands,
                        one_sided,
                        simd_gooch, gooch_warm, gooch_cool,
                        sh4,
                    );
                    cache[ids[b]] = colors[0];
                    cache[ids[b + 1]] = colors[1];
                    cache[ids[b + 2]] = colors[2];
                    cache[ids[b + 3]] = colors[3];
                }
                for k in (n_batches * 4)..m {
                    let i = ids[k];
                    let amb = hemi_ambient(sd.normals[i], amb_sky, amb_gnd, up_f32);
                    cache[i] = shade_point(
                        sd.normals[i], sd.positions[i], (blr, blg, blb),
                        &lights_f32, view_camera, amb, one_minus_ambient, cfg_specular,
                        cfg_fresnel, cfg_gamma,
                        tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands, one_sided,
                        spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        shadow_factors.map(|f| (f, n_unique, i)),
                    );
                }
                cache
            } else {
                let (blr, blg, blb) = if cfg_gamma || shading == ShadingMode::Gooch {
                    (srgb_to_linear(base_r), srgb_to_linear(base_g), srgb_to_linear(base_b))
                } else {
                    (base_r as f32 / 255.0, base_g as f32 / 255.0, base_b as f32 / 255.0)
                };
                (0..n_unique).map(|i| {
                    let amb = hemi_ambient(sd.normals[i], amb_sky, amb_gnd, up_f32);
                    shade_point(
                        sd.normals[i], sd.positions[i], (blr, blg, blb),
                        &lights_f32, view_camera, amb, one_minus_ambient, cfg_specular,
                        cfg_fresnel, cfg_gamma,
                        tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands, one_sided,
                        spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        shadow_factors.map(|f| (f, n_unique, i)),
                    )
                }).collect()
            };
            Some(cache)
        } else { None }
    } else { None };

    let mut projected: Vec<ProjectedTri> = Vec::with_capacity(triangles.len());

    let memo_corners = shade_cache.is_none()
        && !is_wireframe
        && !is_xray
        && group_styles.values().all(|a| a.specular.is_none() && a.shininess.is_none() && a.ambient.is_none());
    let mut corner_memo: Vec<(u32, [u64; 3], (u8, u8, u8))> = match smooth {
        Some(sd) if memo_corners && sd.positions.len() < 3 * triangles.len() => vec![(0, [0; 3], (0, 0, 0)); sd.positions.len()],
        _ => Vec::new(),
    };

    let sh_n_lights = lights_f32.len();
    let sh_stride = smooth.map(|s| s.positions.len()).unwrap_or(0);
    let mut slow_sh_scratch: Vec<v128> = vec![f32x4_splat(1.0); sh_n_lights];
    let mut flat_sh_scratch: Vec<f32> = vec![1.0; sh_n_lights];

    for (ti, tri) in triangles.iter().enumerate() {
        let is_back_facing = if do_cull || is_xray { face_back[ti] } else { false };

        if do_cull && is_back_facing {
            continue;
        }

        let cam = view_simd.transform_tri(tri.vertices[0], tri.vertices[1], tri.vertices[2]);

        let (r, g, b, vertex_colors, opacity) = if is_wireframe {
            (0, 0, 0, None, 1.0)
        } else if shading == ShadingMode::Unlit {
            let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
            let mut opacity = ga.and_then(|a| a.opacity).unwrap_or(config.opacity);
            if is_xray {
                opacity = if is_back_facing { 1.0 } else { cfg_xray_opacity };
            }
            opacity *= tri.alpha.unwrap_or(1.0) as f64;
            let (r, g, b) = if tri.tex.is_some() { (255, 255, 255) } else { tri.color.unwrap_or((base_r, base_g, base_b)) };
            match tri.vertex_colors {
                Some(vc) => {
                    let (r, g, b) = crate::color::avg3(vc[0], vc[1], vc[2]);
                    (r, g, b, Some(vc), opacity)
                }
                None => (r, g, b, None, opacity),
            }
        } else if let Some(ref cache) = shade_cache {
            let sd = unsafe { smooth.unwrap_unchecked() };
            let [i0, i1, i2] = sd.tri_indices[ti];
            let vcols = [cache[i0], cache[i1], cache[i2]];
            let (r, g, b) = crate::color::avg3(vcols[0], vcols[1], vcols[2]);
            let opacity = tri.group_id.and_then(|gid| group_styles.get(&gid))
                .and_then(|a| a.opacity).unwrap_or(config.opacity)
                * tri.alpha.unwrap_or(1.0) as f64;
            (r, g, b, Some(vcols), opacity)
        } else {
            let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
            let grp_intensity = ga.and_then(|a| a.ambient).map(|v| v as f32).unwrap_or(cfg_ambient_intensity);
            let intensity_scale = if grp_intensity == cfg_ambient_intensity { 1.0 } else { grp_intensity / cfg_ambient_intensity.fmax(1e-6) };
            let grp_sky = (amb_sky.0 * intensity_scale, amb_sky.1 * intensity_scale, amb_sky.2 * intensity_scale);
            let grp_gnd = (amb_gnd.0 * intensity_scale, amb_gnd.1 * intensity_scale, amb_gnd.2 * intensity_scale);
            let one_minus_ambient = 1.0 - grp_intensity;
            let mut specular = ga.and_then(|a| a.specular).map(|v| v as f32).unwrap_or(cfg_specular);
            let shininess = ga.and_then(|a| a.shininess).map(|v| v as f32).unwrap_or(cfg_shininess);
            let mut opacity = ga.and_then(|a| a.opacity).unwrap_or(config.opacity);

            let spec_lut = lut_ref(&spec_luts, &zero_luts, shininess);

            if is_xray {
                if is_back_facing {
                    opacity = 1.0;
                    specular = 0.0;
                } else {
                    opacity = cfg_xray_opacity;
                }
            }

            opacity *= tri.alpha.unwrap_or(1.0) as f64;

            let (fr, fg, fb) = if tri.tex.is_some() {
                (255, 255, 255)
            } else {
                tri.color.unwrap_or((base_r, base_g, base_b))
            };

            if let Some(sd) = smooth {
                let [i0, i1, i2] = sd.tri_indices[ti];
                let vn = [sd.normals[i0], sd.normals[i1], sd.normals[i2]];

                let vcols = if tri.vertex_colors.is_none()
                    && matches!(shading, ShadingMode::BlinnPhong | ShadingMode::Flat | ShadingMode::Cel | ShadingMode::Gooch)
                {
                    let is_gooch = shading == ShadingMode::Gooch;
                    let (blr, blg, blb) = if cfg_gamma || is_gooch {
                        (srgb_to_linear(fr), srgb_to_linear(fg), srgb_to_linear(fb))
                    } else {
                        (fr as f32, fg as f32, fb as f32)
                    };
                    let nx4 = f32x4(vn[0].x as f32, vn[1].x as f32, vn[2].x as f32, 0.0);
                    let ny4 = f32x4(vn[0].y as f32, vn[1].y as f32, vn[2].y as f32, 0.0);
                    let nz4 = f32x4(vn[0].z as f32, vn[1].z as f32, vn[2].z as f32, 0.0);
                    let px4 = f32x4(tri.vertices[0].x as f32, tri.vertices[1].x as f32, tri.vertices[2].x as f32, 0.0);
                    let py4 = f32x4(tri.vertices[0].y as f32, tri.vertices[1].y as f32, tri.vertices[2].y as f32, 0.0);
                    let pz4 = f32x4(tri.vertices[0].z as f32, tri.vertices[1].z as f32, tri.vertices[2].z as f32, 0.0);
                    let sh4: Option<&[v128]> = shadow_factors.map(|f| {
                        for li in 0..sh_n_lights {
                            let base = li * sh_stride;
                            slow_sh_scratch[li] = f32x4(f[base + i0], f[base + i1], f[base + i2], 1.0);
                        }
                        &slow_sh_scratch[..]
                    });
                    let colors = shade_batch_4(
                        nx4, ny4, nz4, px4, py4, pz4,
                        blr, blg, blb,
                        &lights_f32, view_camera.x as f32, view_camera.y as f32, view_camera.z as f32,
                        grp_sky.0, grp_sky.1, grp_sky.2,
                        grp_gnd.0, grp_gnd.1, grp_gnd.2,
                        up_f32.0, up_f32.1, up_f32.2,
                        one_minus_ambient, specular, cfg_fresnel,
                        cfg_gamma, tm, cfg_exposure,
                        spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        if shading == ShadingMode::Cel { cfg_cel_bands } else { 0 },
                        one_sided,
                        is_gooch, gooch_warm, gooch_cool,
                        sh4,
                    );
                    [colors[0], colors[1], colors[2]]
                } else {
                    let gamma_or_gooch = cfg_gamma || shading == ShadingMode::Gooch;
                    let mut vcols = [(0u8, 0u8, 0u8); 3];
                    for i in 0..3 {
                        let (vr, vg, vb) = if let Some(vc) = tri.vertex_colors { vc[i] } else { (fr, fg, fb) };
                        let uidx = [i0, i1, i2][i];
                        let key = 0x0100_0000 | (vr as u32) << 16 | (vg as u32) << 8 | vb as u32;
                        let p = tri.vertices[i];
                        let pbits = [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
                        if let Some(m) = corner_memo.get(uidx) {
                            if m.0 == key && m.1 == pbits {
                                vcols[i] = m.2;
                                continue;
                            }
                        }
                        let base_lin = if gamma_or_gooch {
                            (srgb_to_linear(vr), srgb_to_linear(vg), srgb_to_linear(vb))
                        } else {
                            (vr as f32 / 255.0, vg as f32 / 255.0, vb as f32 / 255.0)
                        };
                        let amb = hemi_ambient(vn[i], grp_sky, grp_gnd, up_f32);
                        vcols[i] = shade_point(
                            vn[i], tri.vertices[i], base_lin,
                            &lights_f32, view_camera, amb, one_minus_ambient, specular,
                            cfg_fresnel, cfg_gamma,
                            tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands, one_sided,
                            spec_lut, &fresnel_lut,
                            sss_intensity, sss_dist, &sss_lut,
                            shadow_factors.map(|f| (f, sh_stride, uidx)),
                        );
                        if let Some(m) = corner_memo.get_mut(uidx) {
                            *m = (key, pbits, vcols[i]);
                        }
                    }
                    vcols
                };

                let (r, g, b) = crate::color::avg3(vcols[0], vcols[1], vcols[2]);
                (r, g, b, Some(vcols), opacity)
            } else {
                let centroid = Vec3::centroid(tri.vertices[0], tri.vertices[1], tri.vertices[2]);
                let amb = hemi_ambient(tri.normal, grp_sky, grp_gnd, up_f32);
                let base_lin = if cfg_gamma || shading == ShadingMode::Gooch {
                    (srgb_to_linear(fr), srgb_to_linear(fg), srgb_to_linear(fb))
                } else {
                    (fr as f32 / 255.0, fg as f32 / 255.0, fb as f32 / 255.0)
                };
                let flat_shadow = shadow.filter(|s| !s.per_pixel).map(|s| {
                    for li in 0..sh_n_lights {
                        flat_sh_scratch[li] = s.sample(li, centroid, tri.normal);
                    }
                    (&flat_sh_scratch[..], 1usize, 0usize)
                });
                let (r, g, b) = shade_point(
                    tri.normal, centroid, base_lin,
                    &lights_f32, view_camera, amb, one_minus_ambient, specular,
                    cfg_fresnel, cfg_gamma,
                    tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands, one_sided,
                    spec_lut, &fresnel_lut,
                    sss_intensity, sss_dist, &sss_lut,
                    flat_shadow,
                );
                (r, g, b, None, opacity)
            }
        };

        let pts = apply_projection(&proj_setup, &cam);
        if let Some(limit) = seam_sq {
            let len_sq = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0) * (a.0 - b.0) + (a.1 - b.1) * (a.1 - b.1);
            if len_sq(pts[0], pts[1]) > limit || len_sq(pts[1], pts[2]) > limit || len_sq(pts[2], pts[0]) > limit {
                continue;
            }
        }
        let depths = [cam[0].z, cam[1].z, cam[2].z];
        let depth = (depths[0] + depths[1] + depths[2]) / 3.0;
        projected.push(ProjectedTri { pts, depths, depth, r, g, b, vertex_colors, group_id: tri.group_id, opacity, src: ti as u32, splat: tri.splat });
    }

    projected
}

static mut RADIX_KEYS: Vec<u32> = Vec::new();
static mut RADIX_IDX: Vec<u32> = Vec::new();
static mut RADIX_OUT: Vec<ProjectedTri> = Vec::new();

/// Radix sort `projected` by depth.
/// `descending = false` → nearest last  (SVG painter's back-to-front).
/// `descending = true`  → nearest first (PNG z-buffer front-to-back).
fn radix_sort_by_depth(projected: &mut Vec<ProjectedTri>, descending: bool) {
    let n = projected.len();
    if n <= 1 { return; }

    let keys = unsafe { &mut *std::ptr::addr_of_mut!(RADIX_KEYS) };
    let idx = unsafe { &mut *std::ptr::addr_of_mut!(RADIX_IDX) };
    let out = unsafe { &mut *std::ptr::addr_of_mut!(RADIX_OUT) };

    keys.clear();
    keys.reserve(n);
    for tri in projected.iter() {
        let bits = (tri.depth as f32).to_bits();
        let k = if bits & 0x8000_0000 != 0 { !bits } else { bits ^ 0x8000_0000 };
        keys.push(if descending { !k } else { k });
    }

    let mut hist = [[0u32; 256]; 4];
    for &k in keys.iter() {
        hist[0][(k & 0xFF) as usize] += 1;
        hist[1][((k >> 8) & 0xFF) as usize] += 1;
        hist[2][((k >> 16) & 0xFF) as usize] += 1;
        hist[3][((k >> 24) & 0xFF) as usize] += 1;
    }
    for h in &mut hist {
        let mut sum = 0u32;
        for c in h.iter_mut() {
            let count = *c;
            *c = sum;
            sum += count;
        }
    }

    idx.clear();
    idx.resize(2 * n, 0);
    for i in 0..n { idx[i] = i as u32; }

    {
        let (a, b) = idx.split_at_mut(n);

        for &v in a.iter() {
            let bucket = (keys[v as usize] & 0xFF) as usize;
            b[hist[0][bucket] as usize] = v;
            hist[0][bucket] += 1;
        }
        for &v in b.iter() {
            let bucket = ((keys[v as usize] >> 8) & 0xFF) as usize;
            a[hist[1][bucket] as usize] = v;
            hist[1][bucket] += 1;
        }
        for &v in a.iter() {
            let bucket = ((keys[v as usize] >> 16) & 0xFF) as usize;
            b[hist[2][bucket] as usize] = v;
            hist[2][bucket] += 1;
        }
        for &v in b.iter() {
            let bucket = ((keys[v as usize] >> 24) & 0xFF) as usize;
            a[hist[3][bucket] as usize] = v;
            hist[3][bucket] += 1;
        }
    }

    out.clear();
    out.reserve(n);
    let ptr = projected.as_mut_ptr();
    for i in 0..n {
        out.push(unsafe { std::ptr::read(ptr.add(idx[i] as usize)) });
    }
    unsafe { projected.set_len(0); }
    std::mem::swap(projected, out);
}


fn project_shadow(
    triangles: &[Triangle],
    config: &RenderConfig,
    shadow_dir: Vec3,
    view: &ViewParams,
    vw: f64,
    vh: f64,
    br: f64,
    ground_z: f64,
    force_ortho: bool,
    shadow_color: &str,
) -> Vec<ProjectedTri> {
    let light_dir = shadow_dir;

    if light_dir.z <= 0.01 {
        return Vec::new();
    }

    let proj = if force_ortho { Projection::Ortho } else { resolve_projection(&config.projection) };
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let (sr, sg, sb) = parse_hex_color(shadow_color);

    let mut projected: Vec<ProjectedTri> = Vec::with_capacity(triangles.len());

    for tri in triangles {
        let mut sv = [Vec3::new(0.0, 0.0, 0.0); 3];
        for (i, v) in tri.vertices.iter().enumerate() {
            let t = (v.z - ground_z) / light_dir.z;
            sv[i] = Vec3::new(v.x - t * light_dir.x, v.y - t * light_dir.y, ground_z);
        }

        let cam = [
            view_mat.transform_point(sv[0]),
            view_mat.transform_point(sv[1]),
            view_mat.transform_point(sv[2]),
        ];

        let pts = apply_projection(&proj_setup, &cam);
        let depths = [cam[0].z, cam[1].z, cam[2].z];
        let depth = (depths[0] + depths[1] + depths[2]) / 3.0;
        projected.push(ProjectedTri { pts, depths, depth, r: sr, g: sg, b: sb, vertex_colors: None, group_id: None, opacity: 1.0, src: NO_SOURCE, splat: false });
    }

    projected
}


fn svg_open(svg: &mut String, w: f64, h: f64, bg: &str) {
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 ");
    push_f2(svg, w); svg.push(' '); push_f2(svg, h);
    svg.push_str("\">");
    if !bg.is_empty() {
        svg.push_str("<rect width=\""); push_f2(svg, w);
        svg.push_str("\" height=\""); push_f2(svg, h);
        svg.push_str("\" fill=\""); svg.push_str(bg);
        svg.push_str("\"/>");
    }
}

/// Resolve wireframe color. In overlay mode (solid+wireframe), default is black.
/// In wireframe-only mode, default is the model color.
fn resolve_wireframe_color<'a>(config: &'a RenderConfig, is_overlay: bool) -> &'a str {
    if !config.wireframe.color.is_empty() {
        &config.wireframe.color
    } else if is_overlay {
        "#000000"
    } else {
        &config.color
    }
}

/// Emit the `<defs>` section-hatch pattern for clip caps. `userSpaceOnUse`
/// keeps the lines continuous across triangulated cap faces; `rotate` sets the
/// section angle.
/// Half-length of a `crosses` plus-mark arm, as a fraction of the hatch spacing
/// (shared by the SVG and PNG paths so the two stay identical).
const HATCH_CROSS_ARM: f64 = 0.4;

/// Numeric style code passed to the rasterizer's hatch pass (0/1/2).
fn hatch_style_code(style: crate::config::HatchStyle) -> u8 {
    use crate::config::HatchStyle;
    match style { HatchStyle::Lines => 0, HatchStyle::Cross => 1, HatchStyle::Crosses => 2 }
}

fn push_hatch_defs(svg: &mut String, hc: &crate::config::HatchConfig) {
    use crate::config::HatchStyle;
    let s = hc.spacing;
    svg.push_str("<defs><pattern id=\"maq-hatch\" patternUnits=\"userSpaceOnUse\" width=\"");
    push_f2(svg, s);
    svg.push_str("\" height=\""); push_f2(svg, s);
    svg.push_str("\" patternTransform=\"rotate("); push_f2(svg, hc.angle);
    svg.push_str(")\">");
    let mut push_line = |x1: f64, y1: f64, x2: f64, y2: f64| {
        svg.push_str("<line x1=\""); push_f2(svg, x1);
        svg.push_str("\" y1=\""); push_f2(svg, y1);
        svg.push_str("\" x2=\""); push_f2(svg, x2);
        svg.push_str("\" y2=\""); push_f2(svg, y2);
        svg.push_str("\" stroke=\""); svg.push_str(&hc.color);
        svg.push_str("\" stroke-width=\""); push_f2(svg, hc.width);
        svg.push_str("\"/>");
    };
    match hc.style {
        HatchStyle::Lines => push_line(0.0, 0.0, 0.0, s),
        HatchStyle::Cross => {
            push_line(0.0, 0.0, 0.0, s);
            push_line(0.0, 0.0, s, 0.0);
        }
        HatchStyle::Crosses => {
            let (c, arm) = (s * 0.5, s * HATCH_CROSS_ARM);
            push_line(c, c - arm, c, c + arm);
            push_line(c - arm, c, c + arm, c);
        }
    }
    svg.push_str("</pattern></defs>");
}

fn push_edge_path(svg: &mut String, pts: &[(f64, f64); 3], mask: u8, color: &str, width: f64) {
    if mask == 0 { return; }
    svg.push_str("<path d=\"");
    for e in 0..3 {
        if (mask >> e) & 1 == 0 { continue; }
        let (a, b) = (pts[e], pts[(e + 1) % 3]);
        svg.push('M'); push_f2(svg, a.0); svg.push(' '); push_f2(svg, a.1);
        svg.push('L'); push_f2(svg, b.0); svg.push(' '); push_f2(svg, b.1);
    }
    svg.push_str("\" fill=\"none\" stroke=\"");
    svg.push_str(color);
    svg.push_str("\" stroke-width=\"");
    push_f2(svg, width);
    svg.push_str("\" stroke-linecap=\"round\"/>");
}

fn write_solid_polygon(svg: &mut String, tri: &ProjectedTri, global_stroke: Option<(&str, f64)>, group_styles: &GroupStyles, hatch: bool, mask: u8) {
    svg.push_str("<polygon points=\"");
    push_tri_points(svg, &tri.pts);
    svg.push_str("\" fill=\"");
    push_hex_color(svg, tri.r, tri.g, tri.b);
    svg.push('"');
    if tri.opacity < 1.0 {
        svg.push_str(" fill-opacity=\"");
        push_f2(svg, tri.opacity);
        svg.push('"');
    }
    if tri.group_id == Some(DEBUG_DISK_GID) {
        svg.push_str("/>");
        return;
    }
    if tri.group_id == Some(u32::MAX) {
        svg.push_str(" stroke=\"#333\" stroke-width=\"0.5\" stroke-linejoin=\"round\"/>");
        return;
    }
    let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
    let has_group_stroke = ga.map_or(false, |a| {
        a.stroke.as_deref().map_or(false, |s| s != "none") && a.stroke_width.unwrap_or(1.0) > 0.0
    });
    if mask != crate::tessellate::ALL_EDGES && (has_group_stroke || global_stroke.is_some()) {
        let (stroke, width) = if has_group_stroke {
            let a = unsafe { ga.unwrap_unchecked() };
            (unsafe { a.stroke.as_deref().unwrap_unchecked() }, a.stroke_width.unwrap_or(1.0))
        } else {
            unsafe { global_stroke.unwrap_unchecked() }
        };
        if tri.opacity < 1.0 {
            svg.push_str(" stroke=\"none\"/>");
        } else {
            svg.push_str(" stroke=\"");
            push_hex_color(svg, tri.r, tri.g, tri.b);
            svg.push_str("\" stroke-width=\"0.5\" stroke-linejoin=\"round\"/>");
        }
        push_edge_path(svg, &tri.pts, mask, stroke, width);
        return;
    }
    if has_group_stroke {
        let a = unsafe { ga.unwrap_unchecked() };
        svg.push_str(" stroke=\"");
        svg.push_str(unsafe { a.stroke.as_deref().unwrap_unchecked() });
        svg.push_str("\" stroke-width=\"");
        push_f2(svg, a.stroke_width.unwrap_or(1.0));
        svg.push_str("\" stroke-linejoin=\"round\"");
    } else if let Some((stroke, width)) = global_stroke {
        svg.push_str(" stroke=\"");
        svg.push_str(stroke);
        svg.push_str("\" stroke-width=\"");
        push_f2(svg, width);
        svg.push_str("\" stroke-linejoin=\"round\"");
    } else if tri.opacity < 1.0 {
        svg.push_str(" stroke=\"none\"");
    } else {
        svg.push_str(" stroke=\"");
        push_hex_color(svg, tri.r, tri.g, tri.b);
        svg.push_str("\" stroke-width=\"0.5\" stroke-linejoin=\"round\"");
    }
    svg.push_str("/>");
    if hatch && tri.group_id == Some(clip::CAP_GID) {
        svg.push_str("<polygon points=\"");
        push_tri_points(svg, &tri.pts);
        svg.push_str("\" fill=\"url(#maq-hatch)\" stroke=\"none\"/>");
    }
}

fn write_wireframe_polygon(svg: &mut String, tri: &ProjectedTri, color: &str, width: f64, mask: u8) {
    if mask != crate::tessellate::ALL_EDGES {
        push_edge_path(svg, &tri.pts, mask, color, width);
        return;
    }
    svg.push_str("<polygon points=\"");
    push_tri_points(svg, &tri.pts);
    svg.push_str("\" fill=\"none\" stroke=\"");
    svg.push_str(color);
    svg.push_str("\" stroke-width=\"");
    push_f2(svg, width);
    svg.push_str("\" stroke-linejoin=\"round\"/>");
}

fn write_shadow_polygon(svg: &mut String, tri: &ProjectedTri) {
    svg.push_str("<polygon points=\"");
    push_tri_points(svg, &tri.pts);
    svg.push_str("\" fill=\"");
    push_hex_color(svg, tri.r, tri.g, tri.b);
    svg.push_str("\" stroke=\"");
    push_hex_color(svg, tri.r, tri.g, tri.b);
    svg.push_str("\" stroke-width=\"0.5\" stroke-linejoin=\"round\"/>");
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => {
            let mut result = String::with_capacity(s.len());
            for ch in f.to_uppercase() { result.push(ch); }
            result.push_str(c.as_str());
            result
        }
    }
}


fn turntable_view(bc: Vec3, br: f64, azimuth: f64, elevation_deg: f64) -> ViewParams {
    let dist = br * 3.0;
    ViewParams {
        camera: spherical_camera(bc, dist, elevation_deg.to_radians(), azimuth),
        center: bc,
        up: Vec3::new(0.0, 0.0, 1.0),
    }
}

fn turntable_labels(n: usize) -> Vec<String> {
    let step = 360.0 / n as f64;
    (0..n).map(|i| {
        let mut s = String::with_capacity(6);
        push_i32(&mut s, (i as f64 * step).fround() as i32);
        s.push('°');
        s
    }).collect()
}


/// Resolve a `ClipConfig` to a concrete world-space plane `[a,b,c,d]` (keep the
/// `>= 0` half) plus the cap flag. For camera/axis/normal sources the plane's
/// normal is positioned along the model's extent by `depth`/`distance`.
fn resolve_clip(clip: &crate::config::ClipConfig, bmin: Vec3, bmax: Vec3, config: &RenderConfig) -> ([f64; 4], bool) {
    use crate::config::ClipSource;
    let n = match &clip.source {
        ClipSource::Plane(p) => return (*p, clip.cap),
        ClipSource::Camera => {
            let bc = bbox_center(bmin, bmax);
            let br = bbox_radius(bmin, bmax);
            let view = resolve_config_view(config, bc, br);
            view.center.sub(view.camera).normalized()
        }
        ClipSource::Axis(0) => Vec3::new(1.0, 0.0, 0.0),
        ClipSource::Axis(1) => Vec3::new(0.0, 1.0, 0.0),
        ClipSource::Axis(_) => Vec3::new(0.0, 0.0, 1.0),
        ClipSource::Normal(v) => Vec3::from(*v).normalized(),
    };
    let corners = [
        Vec3::new(bmin.x, bmin.y, bmin.z), Vec3::new(bmax.x, bmin.y, bmin.z),
        Vec3::new(bmin.x, bmax.y, bmin.z), Vec3::new(bmax.x, bmax.y, bmin.z),
        Vec3::new(bmin.x, bmin.y, bmax.z), Vec3::new(bmax.x, bmin.y, bmax.z),
        Vec3::new(bmin.x, bmax.y, bmax.z), Vec3::new(bmax.x, bmax.y, bmax.z),
    ];
    let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in corners { let t = n.dot(c); tmin = tmin.fmin(t); tmax = tmax.fmax(t); }
    let t = match clip.distance {
        Some(d) => tmin + d,
        None => tmin + clip.depth.clamp(0.0, 1.0) * (tmax - tmin),
    };
    let plane = if clip.keep_far {
        [n.x, n.y, n.z, -t]
    } else {
        [-n.x, -n.y, -n.z, t]
    };
    (plane, clip.cap)
}

fn preprocess(triangles: &[Triangle], config: &RenderConfig) -> (Vec<Triangle>, Vec3, Vec3) {
    let mut tris = triangles.to_vec();
    let (mut bmin, mut bmax) = compute_bbox(&tris);

    if config.decimate > 0.0 {
        tris = decimate::decimate(&tris, bmin, bmax, config.decimate);
        if !tris.is_empty() {
            let (new_min, new_max) = compute_bbox(&tris);
            bmin = new_min;
            bmax = new_max;
        }
    }

    if !config.color_map.is_empty() {
        match config.color_map.as_str() {
            "overhang" => {
                let up = Vec3::from(config.up);
                color_map::apply_overhang_map(&mut tris, up, config.overhang_angle);
            }
            "curvature" => {
                let palette: Vec<(u8, u8, u8)> = config.color_map_palette.iter()
                    .map(|s| parse_hex_color(s))
                    .collect();
                color_map::apply_curvature_map(&mut tris, &palette, config.vertex_smoothing);
            }
            "scalar" => {
                let palette: Vec<(u8, u8, u8)> = config.color_map_palette.iter()
                    .map(|s| parse_hex_color(s))
                    .collect();
                if let Err(e) = color_map::apply_scalar_map(&mut tris, &config.scalar_function, &palette, config.vertex_smoothing) {
                    eprintln!("Scalar function error: {}", e);
                }
            }
            "ply_scalar" => {
                let palette: Vec<(u8, u8, u8)> = config.color_map_palette.iter()
                    .map(|s| parse_hex_color(s))
                    .collect();
                color_map::apply_ply_scalar_map(&mut tris, &palette);
            }
            _ => {}
        }
    }

    if let Some(clip_cfg) = &config.clip {
        let (plane, cap) = resolve_clip(clip_cfg, bmin, bmax, config);
        let base = parse_hex_color(&config.color);
        tris = clip::clip_triangles(&tris, plane, cap, base);
    }

    if config.explode.abs() > 1e-12 {
        let bc = bbox_center(bmin, bmax);
        explode::explode_triangles(&mut tris, bc, config.explode);
    }

    if config.clip.is_some() || config.explode.abs() > 1e-12 {
        if !tris.is_empty() {
            let (new_min, new_max) = compute_bbox(&tris);
            bmin = new_min;
            bmax = new_max;
        }
    }
    for tri in &mut tris {
        tri.normal = tri.normal.normalized();
    }

    (tris, bmin, bmax)
}

/// Geometry hash for the smooth-normal cache: the model-data hash mixed with the
/// config that changes the *geometry* (and therefore the vertex normals). Color,
/// material, camera, lighting and shading are deliberately excluded so renders
/// varying only those reuse the cached normals.
/// Mix the clip configuration into a geometry cache key. A camera-relative clip
/// depends on the view, so the camera parameters are folded in for that source.
fn clip_key(h: u64, config: &RenderConfig) -> u64 {
    use crate::config::ClipSource;
    let c = match &config.clip { None => return FxHasher::seeded(h).mix(0x2).key(), Some(c) => c };
    let mut h = FxHasher::seeded(h).mix(0x1);
    match &c.source {
        ClipSource::Plane(p) => { h = h.mix(10); for &v in p { h = h.mix_f64(v); } }
        ClipSource::Camera => {
            h = h.mix(11);
            match config.camera { Some(cam) => for v in cam { h = h.mix_f64(v); }, None => h = h.mix(0x9E) }
            h = h.mix_f64(config.azimuth).mix_f64(config.elevation).mix_f64(config.distance.unwrap_or(0.0)).mix_str(&config.projection);
            for &u in &config.up { h = h.mix_f64(u); }
        }
        ClipSource::Axis(a) => { h = h.mix(12).mix(*a as u64); }
        ClipSource::Normal(n) => { h = h.mix(13); for &v in n { h = h.mix_f64(v); } }
    }
    h.mix_f64(c.depth).mix(c.distance.map(|d| d.to_bits()).unwrap_or(0xDEAD)).mix(c.keep_far as u64).mix(c.cap as u64).key()
}

fn smooth_geom_key(data_key: u64, config: &RenderConfig) -> u64 {
    clip_key(FxHasher::seeded(data_key).mix_f64(config.decimate).mix_f64(config.explode).mix_f64(config.point_size).key(), config)
}

/// Look up (or compute and cache) the smooth vertex normals for `tris`.
fn cached_smooth<'a>(
    data_key: u64,
    config: &RenderConfig,
    tris: &[Triangle],
) -> &'a smooth::SmoothData {
    cache::smooth(smooth_geom_key(data_key, config), || smooth::compute_vertex_normals(tris))
}

/// Cache key for the preprocessed mesh: the model-data hash mixed with EVERY
/// config field that `preprocess` reads, so a hit can only occur for inputs that
/// produce an identical mesh. Must be kept in lock-step with `preprocess`.
fn prep_cache_key(base: u64, config: &RenderConfig) -> u64 {
    let mut h = FxHasher::seeded(base).mix_str(&config.color_map);
    for s in &config.color_map_palette { h = h.mix_str(s); }
    h = h.mix_str(&config.scalar_function).mix_f64(config.overhang_angle).mix(config.vertex_smoothing as u64);
    for &u in &config.up { h = h.mix_f64(u); }
    let mut h = FxHasher::seeded(clip_key(h.key(), config));
    if config.clip.is_some() { h = h.mix_str(&config.color); }
    h.mix_f64(config.explode).mix_f64(config.decimate).key()
}

/// Run `preprocess` or reuse a cached result. When `prep_key` is None (PLY, or
/// OBJ with materials/highlight) the result is computed into `owned` and
/// borrowed from there; otherwise it is cached and borrowed from the cache.
fn cached_preprocess<'a>(
    triangles: &[Triangle],
    config: &RenderConfig,
    prep_key: Option<u64>,
    owned: &'a mut Option<(Vec<Triangle>, Vec3, Vec3)>,
) -> (&'a [Triangle], Vec3, Vec3) {
    if let Some(base) = prep_key {
        let e = cache::prep(prep_cache_key(base, config), || preprocess(triangles, config));
        return (&e.0, e.1, e.2);
    }
    *owned = Some(preprocess(triangles, config));
    let e = owned.as_ref().unwrap();
    (&e.0, e.1, e.2)
}


fn tessellate_for_view(tris: &[Triangle], config: &RenderConfig, view: &ViewParams, br: f64) -> Option<crate::tessellate::Tessellated> {
    let proj = resolve_projection(&config.projection);
    let clipped = if matches!(proj, Projection::Perspective | Projection::Curvilinear) {
        let forward = view.center.sub(view.camera).normalized();
        crate::tessellate::clip_near(tris, view.camera, forward, br * 1e-3)
    } else {
        None
    };
    if !matches!(proj, Projection::Fisheye | Projection::Stereographic | Projection::Curvilinear
        | Projection::Cylindrical | Projection::Pannini | Projection::TinyPlanet) {
        return clipped;
    }
    let (base, masks) = match &clipped {
        Some(c) => (&c.tris[..], Some(&c.edge_masks[..])),
        None => (tris, None),
    };
    crate::tessellate::subdivide_angular(base, masks, view.camera, 3.0f64.to_radians()).or(clipped)
}

fn make_point_projector(
    config: &RenderConfig,
    view: &ViewParams,
    vw: f64,
    vh: f64,
    br: f64,
) -> impl Fn(Vec3) -> (f64, f64) {
    let proj = resolve_projection(&config.projection);
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    move |p: Vec3| {
        let cam = view_mat.transform_point(p);
        let cam_arr = [cam, cam, cam];
        let pts = apply_projection(&proj_setup, &cam_arr);
        (pts[0].0, pts[0].1)
    }
}


pub fn render(triangles: &[Triangle], config: &RenderConfig, group_styles: &GroupStyles, data_key: Option<u64>, prep_key: Option<u64>) -> String {
    if triangles.is_empty() {
        return build_empty_svg(config);
    }

    let mut prep_owned: Option<(Vec<Triangle>, Vec3, Vec3)> = None;
    let (tris, bmin, bmax) = cached_preprocess(triangles, config, prep_key, &mut prep_owned);
    if tris.is_empty() {
        return build_empty_svg(config);
    }
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);

    if config.turntable.iterations >= 2 {
        let labels = turntable_labels(config.turntable.iterations);
        let mut views = Vec::with_capacity(config.turntable.iterations);
        for i in 0..config.turntable.iterations {
            let azimuth = 2.0 * std::f64::consts::PI * i as f64 / config.turntable.iterations as f64;
            views.push((turntable_view(bc, br, azimuth, config.turntable.elevation), labels[i].clone()));
        }
        return render_grid_svg(&tris, config, &views, br, bmin.z, group_styles);
    }

    if let Some(ref views) = config.views {
        if !views.is_empty() {
            let resolved: Vec<_> = views.iter().map(|n| (named_view(n, bc, br), capitalize(n))).collect();
            return render_grid_svg(&tris, config, &resolved, br, bmin.z, group_styles);
        }
    }

    let view = resolve_config_view(config, bc, br);
    let tess = tessellate_for_view(tris, config, &view, br);
    let (tris, edge_masks, data_key, prep_key) = match &tess {
        Some(t) => (&t.tris[..], Some(&t.edge_masks[..]), None, None),
        None => (tris, None, data_key, prep_key),
    };

    let needs_smooth = config.smooth
        && config.mode != "wireframe"
        && config.shading != "cel"
        && config.shading != "flat"
        && config.shading != "unlit";
    let owned_smooth: Option<smooth::SmoothData> = if needs_smooth && data_key.is_none() {
        Some(smooth::compute_vertex_normals(&tris))
    } else {
        None
    };
    let smooth_data: Option<&smooth::SmoothData> = if needs_smooth {
        match data_key {
            Some(k) => Some(cached_smooth(k, config, &tris)),
            None => owned_smooth.as_ref(),
        }
    } else {
        None
    };

    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";

    let lights = resolve_lights(config, bc, br);
    let mut shadow_owned = None;
    let shadow_data = shadow_data_for(&mut shadow_owned, prep_key, &tris, &lights, smooth_data, config, group_styles, bc, br, false);
    let mut projected = project_triangles(&tris, smooth_data, config, &view, config.width, config.height, br, false, group_styles, &lights, shadow_data);
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, config.width, config.height));
    }
    radix_sort_by_depth(&mut projected, false);

    let shadow_tris = if let Some(shadow) = &config.shadow {
        let mut s = project_shadow(&tris, config, shadow_light_dir(config, bc), &view, config.width, config.height, br, bmin.z, false, &shadow.color);
        radix_sort_by_depth(&mut s, false);
        s
    } else {
        Vec::new()
    };

    let outline_edges = if config.outline.is_some() && !is_wireframe {
        let view_dir = (view.center - view.camera).normalized();
        let projector = make_point_projector(config, &view, config.width, config.height, br);
        outline::find_silhouette_edges(&tris, view_dir, &projector)
    } else {
        Vec::new()
    };

    build_single_svg_full(
        &projected, &shadow_tris, &outline_edges, config,
        config.width, config.height, is_wireframe, is_solid_wireframe,
        &view, &tris, bmin, bmax, group_styles, edge_masks,
    )
}


#[allow(clippy::too_many_arguments)]
fn build_single_svg_full(
    tris: &[ProjectedTri],
    shadow_tris: &[ProjectedTri],
    outline_edges: &[outline::ScreenEdge],
    config: &RenderConfig,
    w: f64,
    h: f64,
    is_wireframe: bool,
    is_solid_wireframe: bool,
    view: &ViewParams,
    orig_tris: &[Triangle],
    bmin: Vec3,
    bmax: Vec3,
    group_styles: &GroupStyles,
    edge_masks: Option<&[u8]>,
) -> String {
    let mask_of = |t: &ProjectedTri| edge_masks.and_then(|m| m.get(t.src as usize)).copied().unwrap_or(crate::tessellate::ALL_EDGES);
    let estimated = tris.len() * 200 + shadow_tris.len() * 120 + outline_edges.len() * 80 + 512;
    let mut svg = String::with_capacity(estimated);
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 ");
    push_f2(&mut svg, w); svg.push(' '); push_f2(&mut svg, h);
    svg.push_str("\">");

    let hatch = config.clip.as_ref().and_then(|c| c.hatch.as_ref());
    if let Some(hc) = hatch { push_hatch_defs(&mut svg, hc); }

    if !config.background.is_empty() && config.background != "none" {
        svg.push_str("<rect width=\""); push_f2(&mut svg, w);
        svg.push_str("\" height=\""); push_f2(&mut svg, h);
        svg.push_str("\" fill=\""); svg.push_str(&config.background);
        svg.push_str("\"/>");
    }

    if !shadow_tris.is_empty() {
        svg.push_str("<g opacity=\""); push_f2(&mut svg, unsafe { config.shadow.as_ref().unwrap_unchecked() }.opacity); svg.push_str("\">");
        for tri in shadow_tris {
            write_shadow_polygon(&mut svg, tri);
        }
        svg.push_str("</g>");
    }

    if is_wireframe {
        let wire_color = resolve_wireframe_color(config, false);
        let wire_width = config.wireframe.width;
        for tri in tris {
            write_wireframe_polygon(&mut svg, tri, wire_color, wire_width, mask_of(tri));
        }
    } else {
        let global_stroke = if config.stroke.color != "none" && config.stroke.width > 0.0 {
            Some((config.stroke.color.as_str(), config.stroke.width))
        } else { None };
        let wire = is_solid_wireframe.then(|| (resolve_wireframe_color(config, true), config.wireframe.width));
        for tri in tris {
            write_solid_polygon(&mut svg, tri, global_stroke, group_styles, hatch.is_some(), mask_of(tri));
            if let Some((wire_color, wire_width)) = wire {
                write_wireframe_polygon(&mut svg, tri, wire_color, wire_width, mask_of(tri));
            }
        }
    }

    if !outline_edges.is_empty() {
        let ol = unsafe { config.outline.as_ref().unwrap_unchecked() };
        let ol_color = ol.color.as_str();
        let ol_width = ol.width;
        for edge in outline_edges {
            svg.push_str("<line x1=\""); push_f1(&mut svg, edge.v0.0);
            svg.push_str("\" y1=\""); push_f1(&mut svg, edge.v0.1);
            svg.push_str("\" x2=\""); push_f1(&mut svg, edge.v1.0);
            svg.push_str("\" y2=\""); push_f1(&mut svg, edge.v1.1);
            svg.push_str("\" stroke=\""); svg.push_str(ol_color);
            svg.push_str("\" stroke-width=\""); push_f2(&mut svg, ol_width);
            svg.push_str("\" stroke-linecap=\"round\"/>");
        }
    }

    if let Some(ref ann_cfg) = config.annotations {
        let centroids = compute_group_centroids(tris);
        let anns = annotations::compute_annotations(
            &centroids, group_styles, ann_cfg, (w / 2.0, h / 2.0), w, h,
        );
        annotations::write_annotations_svg(&mut svg, &anns, ann_cfg);
    }

    if config.debug {
        render_debug_light_lines(&mut svg, config, view, bmin, bmax, w, h);
        render_debug_overlay(&mut svg, w, h, orig_tris, bmin, bmax, view, config, "SVG");
    }

    svg.push_str("</svg>");
    svg
}

fn compute_group_centroids(tris: &[ProjectedTri]) -> FxHashMap<u32, (f64, f64)> {
    let mut sums: FxHashMap<u32, (f64, f64, usize)> = fx_hashmap_cap(16);
    for tri in tris {
        if let Some(gid) = tri.group_id {
            let cx = (tri.pts[0].0 + tri.pts[1].0 + tri.pts[2].0) / 3.0;
            let cy = (tri.pts[0].1 + tri.pts[1].1 + tri.pts[2].1) / 3.0;
            let entry = sums.entry(gid).or_insert((0.0, 0.0, 0));
            entry.0 += cx;
            entry.1 += cy;
            entry.2 += 1;
        }
    }
    sums.into_iter()
        .map(|(gid, (sx, sy, n))| (gid, (sx / n as f64, sy / n as f64)))
        .collect()
}

fn count_unique_vertices(triangles: &[Triangle]) -> usize {
    let mut set: HashSet<_, FxBuildHasher> =
        HashSet::with_capacity_and_hasher(triangles.len(), FxBuildHasher::default());
    for tri in triangles {
        for v in &tri.vertices {
            set.insert(quantize(*v));
        }
    }
    set.len()
}

/// Reserved `group_id` for debug area-light disks — filled with the light color
/// and (unlike the point-light octahedron, `u32::MAX`) drawn without edge strokes
/// so the triangle fan reads as a single clean disk.
const DEBUG_DISK_GID: u32 = u32::MAX - 2;

/// Generate debug light octahedrons as projected triangles for depth-sorted rendering.
fn make_debug_light_tris(
    config: &RenderConfig,
    view: &ViewParams,
    bmin: Vec3,
    bmax: Vec3,
    w: f64,
    h: f64,
) -> Vec<ProjectedTri> {
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);
    let lights = resolve_lights(config, bc, br);
    let proj = resolve_projection(&config.projection);
    let proj_setup = setup_projection(proj, config, view, w, h, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let size = br * 0.04;

    let faces: [(usize, usize, usize); 8] = [
        (0, 2, 4), (2, 1, 4), (1, 3, 4), (3, 0, 4),
        (2, 0, 5), (1, 2, 5), (3, 1, 5), (0, 3, 5),
    ];

    let mut out = Vec::new();
    for light in &lights {
        let pos = match light.kind {
            LightKind::Directional => bc + light.vector.scale(br * 2.0),
            LightKind::Positional | LightKind::Area => light.vector,
        };

        let r = linear_to_srgb(light.color.0.fmin(1.0f32));
        let g = linear_to_srgb(light.color.1.fmin(1.0f32));
        let b = linear_to_srgb(light.color.2.fmin(1.0f32));

        if light.kind == LightKind::Area && light.size > 0.0 {
            let n = {
                let d = bc.sub(pos);
                if d.length() > 1e-6 { d.normalized() } else { Vec3::new(0.0, 0.0, 1.0) }
            };
            let a = if n.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
            let u = n.cross(a).normalized();
            let vv = n.cross(u);
            const SEGS: usize = 24;
            let mut dv = Vec::with_capacity(SEGS + 1);
            dv.push(pos);
            for i in 0..SEGS {
                let ang = (i as f64) / (SEGS as f64) * std::f64::consts::TAU;
                dv.push(pos.add(u.scale(light.size * ang.cos())).add(vv.scale(light.size * ang.sin())));
            }
            let cam: Vec<Vec3> = dv.iter().map(|v| view_mat.transform_point(*v)).collect();
            let proj_pts: Vec<(f64, f64)> = cam.iter().map(|c| {
                let t = [*c, *c, *c];
                apply_projection(&proj_setup, &t)[0]
            }).collect();
            let cam_depths: Vec<f64> = cam.iter().map(|c| c.z).collect();
            for i in 0..SEGS {
                let (i1, i2) = (1 + i, 1 + (i + 1) % SEGS);
                let depth = (cam_depths[0] + cam_depths[i1] + cam_depths[i2]) / 3.0;
                out.push(ProjectedTri { splat: false,
                    pts: [proj_pts[0], proj_pts[i1], proj_pts[i2]],
                    depths: [cam_depths[0], cam_depths[i1], cam_depths[i2]],
                    depth,
                    r, g, b,
                    vertex_colors: None,
                    group_id: Some(DEBUG_DISK_GID),
                    opacity: 0.85,
                    src: NO_SOURCE,
                });
            }
            continue;
        }

        let verts = [
            Vec3::new(pos.x + size, pos.y, pos.z),
            Vec3::new(pos.x - size, pos.y, pos.z),
            Vec3::new(pos.x, pos.y + size, pos.z),
            Vec3::new(pos.x, pos.y - size, pos.z),
            Vec3::new(pos.x, pos.y, pos.z + size),
            Vec3::new(pos.x, pos.y, pos.z - size),
        ];

        let cam: Vec<Vec3> = verts.iter().map(|v| view_mat.transform_point(*v)).collect();
        let proj_pts: Vec<(f64, f64)> = (0..6).map(|i| {
            let c = [cam[i], cam[i], cam[i]];
            apply_projection(&proj_setup, &c)[0]
        }).collect();
        let cam_depths: Vec<f64> = cam.iter().map(|c| c.z).collect();

        for &(a, bi, c) in &faces {
            let depth = (cam_depths[a] + cam_depths[bi] + cam_depths[c]) / 3.0;
            out.push(ProjectedTri { splat: false,
                pts: [proj_pts[a], proj_pts[bi], proj_pts[c]],
                depths: [cam_depths[a], cam_depths[bi], cam_depths[c]],
                depth,
                r, g, b,
                vertex_colors: None,
                group_id: Some(u32::MAX),
                opacity: 0.85,
                src: NO_SOURCE,
            });
        }
    }
    out
}

/// Render directional light dashed lines as SVG overlay (always on top).
fn render_debug_light_lines(
    svg: &mut String,
    config: &RenderConfig,
    view: &ViewParams,
    bmin: Vec3,
    bmax: Vec3,
    w: f64,
    h: f64,
) {
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);
    let lights = resolve_lights(config, bc, br);
    let projector = make_point_projector(config, view, w, h, br);

    for light in &lights {
        if light.kind != LightKind::Directional { continue; }
        let pos = bc + light.vector.scale(br * 2.0);
        let line_end = bc + light.vector.scale(br * 1.5);
        let r = linear_to_srgb(light.color.0.fmin(1.0f32));
        let g = linear_to_srgb(light.color.1.fmin(1.0f32));
        let b = linear_to_srgb(light.color.2.fmin(1.0f32));
        let pp = projector(pos);
        let pe = projector(line_end);
        svg.push_str("<line x1=\""); push_f1(svg, pp.0);
        svg.push_str("\" y1=\""); push_f1(svg, pp.1);
        svg.push_str("\" x2=\""); push_f1(svg, pe.0);
        svg.push_str("\" y2=\""); push_f1(svg, pe.1);
        svg.push_str("\" stroke=\""); push_hex_color(svg, r, g, b);
        svg.push_str("\" stroke-width=\"1.5\" stroke-dasharray=\"4,3\" opacity=\"0.6\"/>");
    }
}

fn render_debug_overlay(
    svg: &mut String,
    w: f64,
    _h: f64,
    triangles: &[Triangle],
    bmin: Vec3,
    bmax: Vec3,
    view: &ViewParams,
    config: &RenderConfig,
    mode: &str,
) {
    let color = &config.debug_color;
    let font_size = 10.0;
    let line_height = font_size * 1.05;
    let pad = 8.0;
    let val_x = w - pad;
    let key_x = val_x - 120.0;
    let mut row = 0usize;

    let mut emit_row = |svg: &mut String, key: &str, val: &str| {
        let y = pad + font_size + row as f64 * line_height;
        svg.push_str("<text x=\""); push_f1(svg, key_x);
        svg.push_str("\" y=\""); push_f1(svg, y);
        svg.push_str("\" font-family=\"sans-serif\" font-size=\"");
        push_f1(svg, font_size);
        svg.push_str("\" font-weight=\"bold\" fill=\""); svg.push_str(color);
        svg.push_str("\" text-anchor=\"end\">"); svg.push_str(key);
        svg.push_str("</text><text x=\""); push_f1(svg, val_x);
        svg.push_str("\" y=\""); push_f1(svg, y);
        svg.push_str("\" font-family=\"sans-serif\" font-size=\"");
        push_f1(svg, font_size);
        svg.push_str("\" fill=\""); svg.push_str(color);
        svg.push_str("\" text-anchor=\"end\">"); svg.push_str(val);
        svg.push_str("</text>");
        row += 1;
    };

    emit_row(svg, "mode", mode);
    emit_row(svg, "projection", &config.projection);
    if config.projection == "perspective" {
        let mut buf = String::with_capacity(8);
        push_f2(&mut buf, config.fov); buf.push('\u{b0}');
        emit_row(svg, "fov", &buf);
    }
    if mode == "PNG" {
        let mut buf = String::with_capacity(16);
        push_usize(&mut buf, config.width as usize); buf.push('\u{d7}');
        push_usize(&mut buf, config.height as usize);
        emit_row(svg, "resolution", &buf);
    }
    { let mut buf = String::with_capacity(8); push_usize(&mut buf, triangles.len()); emit_row(svg, "triangles", &buf); }
    { let mut buf = String::with_capacity(8); push_usize(&mut buf, count_unique_vertices(triangles)); emit_row(svg, "vertices", &buf); }
    { let mut buf = String::with_capacity(8); push_f2(&mut buf, config.ambient.intensity); emit_row(svg, "ambient", &buf); }
    emit_row(svg, "smooth", if config.smooth { "on" } else { "off" });
    if config.decimate > 0.0 {
        let mut buf = String::with_capacity(8);
        push_f2(&mut buf, config.decimate);
        emit_row(svg, "decimate", &buf);
    }

    let mut effects_str = String::new();
    if config.outline.is_some() { effects_str.push_str("outline"); }
    if config.shadow.is_some() { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str("shadow"); }
    if config.clip.is_some() { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str("clip"); }
    if config.explode > 0.0 { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str("explode"); }
    if !config.color_map.is_empty() { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str(&config.color_map); }
    if !effects_str.is_empty() {
        emit_row(svg, "effects", &effects_str);
    }

    let mut buf = String::with_capacity(32);
    let mut vec3_row = |svg: &mut String, key: &str, v: Vec3| {
        buf.clear();
        buf.push('('); push_f2(&mut buf, v.x);
        buf.push_str(", "); push_f2(&mut buf, v.y);
        buf.push_str(", "); push_f2(&mut buf, v.z);
        buf.push(')');
        emit_row(svg, key, &buf);
    };
    vec3_row(svg, "camera", view.camera);
    vec3_row(svg, "center", view.center);
    vec3_row(svg, "bbox min", bmin);
    vec3_row(svg, "bbox max", bmax);

    buf.clear();
    push_f2(&mut buf, bmax.x - bmin.x); buf.push_str(" x ");
    push_f2(&mut buf, bmax.y - bmin.y); buf.push_str(" x ");
    push_f2(&mut buf, bmax.z - bmin.z);
    emit_row(svg, "size", &buf);
}

/// Write grid lines (shared by SVG grid and grid-label overlay).
fn write_grid_lines(svg: &mut String, cols: usize, rows: usize, cell_w: f64, cell_h: f64, w: f64, h: f64) {
    for c in 1..cols {
        let x = c as f64 * cell_w;
        svg.push_str("<line x1=\""); push_f2(svg, x);
        svg.push_str("\" y1=\"0\" x2=\""); push_f2(svg, x);
        svg.push_str("\" y2=\""); push_f2(svg, h);
        svg.push_str("\" stroke=\"#cccccc\" stroke-width=\"0.5\"/>");
    }
    for r in 1..rows {
        let y = r as f64 * cell_h;
        svg.push_str("<line x1=\"0\" y1=\""); push_f2(svg, y);
        svg.push_str("\" x2=\""); push_f2(svg, w);
        svg.push_str("\" y2=\""); push_f2(svg, y);
        svg.push_str("\" stroke=\"#cccccc\" stroke-width=\"0.5\"/>");
    }
}

/// Transparent SVG overlay with annotation leaders + labels.
fn overlay_annotations(
    w: f64,
    h: f64,
    centroids: &FxHashMap<u32, (f64, f64)>,
    group_styles: &GroupStyles,
    ann_cfg: &crate::config::AnnotationConfig,
) -> String {
    let mut svg = svg_overlay_open(w, h);
    let anns = annotations::compute_annotations(
        centroids, group_styles, ann_cfg, (w / 2.0, h / 2.0), w, h,
    );
    annotations::write_annotations_svg(&mut svg, &anns, ann_cfg);
    svg.push_str("</svg>");
    svg
}

/// Transparent SVG overlay with the debug light lines + text.
fn overlay_debug(
    w: f64,
    h: f64,
    triangles: &[Triangle],
    bmin: Vec3,
    bmax: Vec3,
    view: &ViewParams,
    config: &RenderConfig,
) -> String {
    let mut svg = svg_overlay_open(w, h);
    render_debug_light_lines(&mut svg, config, view, bmin, bmax, w, h);
    render_debug_overlay(&mut svg, w, h, triangles, bmin, bmax, view, config, "PNG");
    svg.push_str("</svg>");
    svg
}

/// Transparent SVG overlay with the grid's view labels + grid lines.
fn overlay_grid_labels(
    w: f64,
    h: f64,
    views: &[(ViewParams, String)],
) -> String {
    let mut svg = svg_overlay_open(w, h);

    let (cols, rows) = grid_layout(views.len());
    let cell_w = w / cols as f64;
    let cell_h = h / rows as f64;

    for (i, (_view, label)) in views.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = col as f64 * cell_w + cell_w / 2.0;
        let y = row as f64 * cell_h + 16.0;
        svg.push_str("<text x=\""); push_f2(&mut svg, x);
        svg.push_str("\" y=\""); push_f2(&mut svg, y);
        svg.push_str("\" font-family=\"sans-serif\" font-size=\"14\" fill=\"#666666\" text-anchor=\"middle\">");
        svg.push_str(label);
        svg.push_str("</text>");
    }

    if views.len() > 1 {
        write_grid_lines(&mut svg, cols, rows, cell_w, cell_h, w, h);
    }

    svg.push_str("</svg>");
    svg
}

fn build_empty_svg(config: &RenderConfig) -> String {
    let mut svg = String::new();
    svg_open(&mut svg, config.width, config.height, &config.background);
    svg.push_str("</svg>");
    svg
}


/// Compute grid layout: (cols, rows) from the number of views.
fn grid_layout(n: usize) -> (usize, usize) {
    let cols = if n <= 2 { n } else { 2 };
    let rows = (n + cols - 1) / cols;
    (cols, rows)
}

fn render_grid_svg(
    triangles: &[Triangle],
    config: &RenderConfig,
    views: &[(ViewParams, String)],
    br: f64,
    ground_z: f64,
    group_styles: &GroupStyles,
) -> String {
    let (cols, rows) = grid_layout(views.len());
    let cell_w = config.width / cols as f64;
    let cell_h = config.height / rows as f64;
    let label_h = if config.grid_labels { 24.0 } else { 0.0 };
    let is_wireframe = config.mode == "wireframe";

    let (gbmin, gbmax) = compute_bbox(triangles);
    let lights = resolve_lights(config, bbox_center(gbmin, gbmax), br);
    let shadow_data = build_shadow_data(triangles, &lights, None, config, group_styles, bbox_center(gbmin, gbmax), br, false);
    let estimated = triangles.len() * 200 * views.len() + 512;
    let mut svg = String::with_capacity(estimated);
    svg_open(&mut svg, config.width, config.height, &config.background);
    let hatch = config.clip.as_ref().and_then(|c| c.hatch.as_ref());
    if let Some(hc) = hatch { push_hatch_defs(&mut svg, hc); }

    for (i, (view, label)) in views.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = col as f64 * cell_w;
        let y = row as f64 * cell_h;
        let render_h = cell_h - label_h;

        let mut projected = project_triangles(triangles, None, config, view, cell_w, render_h, br, true, group_styles, &lights, shadow_data.as_ref());
        radix_sort_by_depth(&mut projected, false);

        if config.grid_labels {
            svg.push_str("<text x=\""); push_f2(&mut svg, x + cell_w / 2.0);
            svg.push_str("\" y=\""); push_f2(&mut svg, y + 16.0);
            svg.push_str("\" font-family=\"sans-serif\" font-size=\"14\" fill=\"#666666\" text-anchor=\"middle\">");
            svg.push_str(label);
            svg.push_str("</text>");
        }

        svg.push_str("<g transform=\"translate(");
        push_f2(&mut svg, x); svg.push_str(", "); push_f2(&mut svg, y + label_h);
        svg.push_str(")\">");

        if let Some(shadow_cfg) = &config.shadow {
            if !is_wireframe {
                let mut shadow = project_shadow(triangles, config, shadow_light_dir(config, bbox_center(gbmin, gbmax)), view, cell_w, render_h, br, ground_z, true, &shadow_cfg.color);
                radix_sort_by_depth(&mut shadow, false);
                svg.push_str("<g opacity=\""); push_f2(&mut svg, shadow_cfg.opacity); svg.push_str("\">");
                for tri in &shadow {
                    write_shadow_polygon(&mut svg, tri);
                }
                svg.push_str("</g>");
            }
        }

        if is_wireframe {
            let wire_color = resolve_wireframe_color(config, false);
            let wire_width = config.wireframe.width;
            for tri in &projected {
                write_wireframe_polygon(&mut svg, tri, wire_color, wire_width, crate::tessellate::ALL_EDGES);
            }
        } else {
            let global_stroke = if config.stroke.color != "none" && config.stroke.width > 0.0 {
                Some((config.stroke.color.as_str(), config.stroke.width))
            } else { None };
            let wire = (config.mode == "solid+wireframe").then(|| (resolve_wireframe_color(config, true), config.wireframe.width));
            for tri in &projected {
                write_solid_polygon(&mut svg, tri, global_stroke, group_styles, hatch.is_some(), crate::tessellate::ALL_EDGES);
                if let Some((wire_color, wire_width)) = wire {
                    write_wireframe_polygon(&mut svg, tri, wire_color, wire_width, crate::tessellate::ALL_EDGES);
                }
            }
        }

        svg.push_str("</g>");
    }

    if views.len() > 1 {
        write_grid_lines(&mut svg, cols, rows, cell_w, cell_h, config.width, config.height);
    }

    svg.push_str("</svg>");
    svg
}


/// The raw RGBA producer shared by both plain and overlay outputs: downsamples
/// (opaque SSAA) or composites z-buffer coverage (transparent) into straight
/// RGBA8 at output resolution. Returns `(width, height, rgba)`.
/// Raster blob, see [`raw_raster`]: tag `0x00` for a plain image, `0x02` for
/// an image followed by a transparent SVG overlay (labels, grid lines,
/// annotations, debug text) that the host layers on top in the raster's pixel
/// space. Either tag also distinguishes the blob from SVG output ('<').
fn encode_raster(out: &PixelBuffer, alpha: Option<&[u8]>, overlay: Option<&str>) -> Vec<u8> {
    let (w, h, rgba) = out.to_rgba8_alpha(alpha);
    match overlay {
        Some(svg) => raw_raster(0x02, w, h, &rgba, svg.as_bytes()),
        None => raw_raster(0x00, w, h, &rgba, &[]),
    }
}

/// Resolve a supersampled buffer with no post effects and encode it.
fn finish_raster(buf: PixelBuffer, aa: usize, transparent: bool, overlay: Option<&str>) -> Result<Vec<u8>, String> {
    let (out, alpha) = buf.resolve(aa, transparent, false);
    Ok(encode_raster(&out, alpha.as_deref(), overlay))
}

/// The configured post effects, for a frame resolved from `aa`× supersampling.
#[allow(clippy::too_many_arguments)]
fn post_effects(config: &RenderConfig, view: &ViewParams, vw: f64, vh: f64, br: f64, bg: (u8, u8, u8), aa: usize, fxaa: bool, is_wireframe: bool) -> PostEffects {
    let lit = !is_wireframe;
    PostEffects {
        ssao: config.ssao.as_ref().filter(|_| lit).map(|s| {
            let scene = (s.space != "screen").then(|| (depth_camera(config, view, vw, vh, br).downscaled(aa), br));
            SSAOParams::new(s.samples, s.radius, s.bias, s.strength, scene)
        }),
        fog: config.fog.as_ref().map(|f| {
            let color = if f.color.is_empty() { bg } else { parse_hex_color(&f.color) };
            Fog::molstar((view.camera - view.center).length(), br, f.intensity, color)
        }),
        bloom: config.bloom.as_ref().filter(|_| lit).map(|b| Bloom { threshold: b.threshold as f32, intensity: b.intensity as f32, radius: b.radius }),
        glow: config.glow.as_ref().filter(|_| lit).map(|g| Glow { color: parse_hex_color(&g.color), intensity: g.intensity as f32, radius: g.radius }),
        sharpen: config.sharpen.as_ref().map(|s| s.strength as f32),
        fxaa,
    }
}

/// Open a transparent overlay SVG sized to the raster's pixel space. Emits
/// explicit `width`/`height` (not just `viewBox`) so browsers give it an
/// intrinsic size — canvas `drawImage()` needs that to rasterize the SVG.
fn svg_overlay_open(w: f64, h: f64) -> String {
    let mut svg = String::with_capacity(256);
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"");
    push_f2(&mut svg, w);
    svg.push_str("\" height=\"");
    push_f2(&mut svg, h);
    svg.push_str("\" viewBox=\"0 0 ");
    push_f2(&mut svg, w);
    svg.push(' ');
    push_f2(&mut svg, h);
    svg.push_str("\">");
    svg
}

/// Rasterize to a bitmap. Plain images are a raw RGBA blob; the
/// annotation/debug/labelled-grid variants return a raster+overlay blob (see
/// [`encode_raster`]) so the host layers vector text over raw pixels — the
/// plugin never encodes an image format.
/// Per-triangle mip LOD from the screen-area / UV-area ratio. Constant over the
/// triangle (no per-pixel derivatives) — cheap and good enough for the mesh
/// path. Uses supersampled screen coords, so SSAA naturally sharpens then
/// filters. `lod` 0 = mip 0; `Texture::sample_lod` clamps to the chain.
#[inline]
fn triangle_texture_lod(pts: &[(f64, f64); 3], uvs: &[[f32; 2]; 3], tex: &maquette_core::texture::Texture) -> f32 {
    let sx1 = pts[1].0 - pts[0].0; let sy1 = pts[1].1 - pts[0].1;
    let sx2 = pts[2].0 - pts[0].0; let sy2 = pts[2].1 - pts[0].1;
    let screen_area = (sx1 * sy2 - sx2 * sy1).abs() as f32;
    let du1 = uvs[1][0] - uvs[0][0]; let dv1 = uvs[1][1] - uvs[0][1];
    let du2 = uvs[2][0] - uvs[0][0]; let dv2 = uvs[2][1] - uvs[0][1];
    let uv_area = (du1 * dv2 - du2 * dv1).abs();
    if screen_area <= 1e-9 || uv_area <= 1e-12 { return 0.0; }
    tex.lod_bias + 0.5 * (uv_area / screen_area).log2()
}

pub fn render_raster(triangles: &[Triangle], config: &RenderConfig, group_styles: &GroupStyles, data_key: Option<u64>, prep_key: Option<u64>, textures: &[maquette_core::texture::Texture]) -> Result<Vec<u8>, String> {
    let (aa, fxaa) = maquette_core::effects::antialias_mode(config.antialias)?;
    maquette_core::effects::check_raster_size(config.width as usize, config.height as usize, aa, MAX_RASTER_SAMPLES)?;
    let w = config.width as usize * aa;
    let h = config.height as usize * aa;
    let vw = config.width * aa as f64;
    let vh = config.height * aa as f64;
    let transparent = config.background.is_empty() || config.background == "none";
    let bg = if transparent {
        (255, 255, 255)
    } else {
        parse_hex_color(&config.background)
    };

    if triangles.is_empty() {
        let buf = PixelBuffer::new(config.width as usize, config.height as usize, bg);
        return finish_raster(buf, 1, transparent, None);
    }

    crate::prof::mark(10);
    let mut prep_owned: Option<(Vec<Triangle>, Vec3, Vec3)> = None;
    let (tris, bmin, bmax) = cached_preprocess(triangles, config, prep_key, &mut prep_owned);
    crate::prof::mark(11);
    if tris.is_empty() {
        let buf = PixelBuffer::new(config.width as usize, config.height as usize, bg);
        return finish_raster(buf, 1, transparent, None);
    }
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);

    if config.turntable.iterations >= 2 {
        let labels = turntable_labels(config.turntable.iterations);
        let mut views = Vec::with_capacity(config.turntable.iterations);
        for i in 0..config.turntable.iterations {
            let azimuth = 2.0 * std::f64::consts::PI * i as f64 / config.turntable.iterations as f64;
            views.push((turntable_view(bc, br, azimuth, config.turntable.elevation), labels[i].clone()));
        }
        let buf = render_grid_png_buf(&tris, config, &views, br, bmin.z, w, h, bg, group_styles);
        return if config.grid_labels {
            let overlay = overlay_grid_labels(config.width, config.height, &views);
            finish_raster(buf, aa, transparent, Some(&overlay))
        } else {
            finish_raster(buf, aa, transparent, None)
        };
    }

    if let Some(ref views) = config.views {
        if !views.is_empty() {
            let resolved: Vec<_> = views.iter().map(|n| (named_view(n, bc, br), capitalize(n))).collect();
            let buf = render_grid_png_buf(&tris, config, &resolved, br, bmin.z, w, h, bg, group_styles);
            return if config.grid_labels {
                let overlay = overlay_grid_labels(config.width, config.height, &resolved);
                finish_raster(buf, aa, transparent, Some(&overlay))
            } else {
                finish_raster(buf, aa, transparent, None)
            };
        }
    }

    let view = resolve_config_view(config, bc, br);
    let tess = tessellate_for_view(tris, config, &view, br);
    let (tris, edge_masks, data_key, prep_key) = match &tess {
        Some(t) => (&t.tris[..], Some(&t.edge_masks[..]), None, None),
        None => (tris, None, data_key, prep_key),
    };

    let needs_smooth = config.smooth
        && config.mode != "wireframe"
        && config.shading != "cel"
        && config.shading != "flat"
        && config.shading != "unlit";
    let owned_smooth: Option<smooth::SmoothData> = if needs_smooth && data_key.is_none() {
        Some(smooth::compute_vertex_normals(&tris))
    } else {
        None
    };
    let smooth_data: Option<&smooth::SmoothData> = if needs_smooth {
        match data_key {
            Some(k) => Some(cached_smooth(k, config, &tris)),
            None => owned_smooth.as_ref(),
        }
    } else {
        None
    };

    crate::prof::mark(12);
    let is_wireframe = config.mode == "wireframe";

    let lights = resolve_lights(config, bc, br);
    let mut shadow_owned = None;
    let shadow_data = shadow_data_for(&mut shadow_owned, prep_key, &tris, &lights, smooth_data, config, group_styles, bc, br, true);
    crate::prof::mark(13);
    let mut projected = project_triangles(&tris, smooth_data, config, &view, vw, vh, br, false, group_styles, &lights, shadow_data);
    crate::prof::mark(14);
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, vw, vh));
    }
    radix_sort_by_depth(&mut projected, true);
    crate::prof::mark(15);

    let mut buf = PixelBuffer::new(w, h, bg);
    crate::prof::mark(16);

    if let Some(shadow_cfg) = &config.shadow {
        if !is_wireframe {
            let shadow = project_shadow(&tris, config, shadow_light_dir(config, bc), &view, vw, vh, br, bmin.z, false, &shadow_cfg.color);
            rasterize_shadow_to_buf(&mut buf, &shadow, shadow_cfg);
        }
    }

    if !is_wireframe {
        let pp_shadow = shadow_data.filter(|s| s.per_pixel);
        let source = |i: u32| (i != NO_SOURCE).then(|| &tris[i as usize]);
        let pp_of = |t: &Triangle| {
            let n = if t.splat { t.vertex_normals.map_or(t.normal, |v| v[0]) } else { t.normal };
            (t.vertices, n)
        };
        for tri in &projected {
            if tri.opacity >= 1.0 {
                let src = source(tri.src);
                let pp = pp_shadow.zip(src).map(|(sd, t)| (sd, pp_of(t)));
                if tri.splat {
                    let mut c = tri.vertex_colors.map_or((tri.r, tri.g, tri.b), |c| c[0]);
                    if let Some((sd, (wp, normal))) = pp {
                        c = sd.pp_shade(c, wp[0], normal);
                    }
                    buf.rasterize_splat(&tri.pts, &tri.depths, c.0, c.1, c.2);
                    continue;
                }
                let max_d = tri.depths[0].fmax(tri.depths[1]).fmax(tri.depths[2]) as f32;
                if buf.hiz_can_skip(&tri.pts, max_d) { continue; }
                if let Some((ti, uvs)) = src.and_then(|t| t.tex.zip(t.uvs)) {
                    if let Some(tex) = textures.get(ti as usize) {
                        let light = tri.vertex_colors.unwrap_or([(tri.r, tri.g, tri.b); 3]);
                        let lod = triangle_texture_lod(&tri.pts, &uvs, tex);
                        buf.rasterize_triangle_textured(&tri.pts, &tri.depths, &uvs, &light, tex, lod);
                        continue;
                    }
                }
                match pp {
                    Some((sd, (wp, normal))) => {
                        let cols = tri.vertex_colors.unwrap_or([(tri.r, tri.g, tri.b); 3]);
                        let world = [[wp[0].x, wp[0].y, wp[0].z], [wp[1].x, wp[1].y, wp[1].z], [wp[2].x, wp[2].y, wp[2].z]];
                        buf.rasterize_triangle_shadowed(&tri.pts, &tri.depths, &cols, &world, |c, p| {
                            sd.pp_shade(c, Vec3::new(p[0], p[1], p[2]), normal)
                        });
                    }
                    _ => {
                        if let Some(vcols) = &tri.vertex_colors {
                            buf.rasterize_triangle_smooth(&tri.pts, &tri.depths, vcols);
                        } else {
                            buf.rasterize_triangle(&tri.pts, &tri.depths, tri.r, tri.g, tri.b);
                        }
                    }
                }
            }
        }
        for tri in projected.iter().rev() {
            if tri.opacity < 1.0 {
                if let Some(vcols) = &tri.vertex_colors {
                    buf.rasterize_triangle_smooth_blend(&tri.pts, &tri.depths, vcols, tri.opacity);
                } else {
                    buf.rasterize_triangle_blend(&tri.pts, &tri.depths, tri.r, tri.g, tri.b, tri.opacity);
                }
            }
        }
    }

    crate::prof::mark(17);
    if !is_wireframe {
        if let Some(hc) = config.clip.as_ref().and_then(|c| c.hatch.as_ref()) {
            let color = parse_hex_color(&hc.color);
            let ang = hc.angle.to_radians();
            let (cos_a, sin_a) = (ang.cos(), ang.sin());
            let spacing = (hc.spacing * aa as f64).fmax(0.5);
            let half_w = hc.width * aa as f64 * 0.5;
            let style = hatch_style_code(hc.style);
            let arm = spacing * HATCH_CROSS_ARM;
            for tri in &projected {
                if tri.group_id == Some(clip::CAP_GID) && tri.opacity >= 1.0 {
                    buf.hatch_triangle(&tri.pts, &tri.depths, spacing, half_w, cos_a, sin_a, style, arm, color);
                }
            }
        }
    }

    draw_edges(&mut buf, &projected, config, group_styles, edge_masks, br, (0.0, 0.0), EdgePass::Strokes);

    if config.debug {
        for tri in &projected {
            if tri.group_id == Some(u32::MAX) {
                buf.draw_triangle_edges_z(&tri.pts, &tri.depths, 0.0, 0x33, 0x33, 0x33);
            }
        }
    }

    draw_edges(&mut buf, &projected, config, group_styles, edge_masks, br, (0.0, 0.0), EdgePass::Wire);

    if let Some(ref outline) = config.outline {
        if !is_wireframe {
            let (or, og, ob) = parse_hex_color(&outline.color);
            let scene = outline.threshold.map(|t| (depth_camera(config, &view, vw, vh, br), t as f32));
            buf.apply_outline((or, og, ob), outline.width * aa as f64, scene);
        }
    }

    let effects = post_effects(config, &view, vw, vh, br, bg, aa, fxaa && !transparent, is_wireframe);
    crate::prof::mark(18);
    let (mut out, mut alpha) = buf.resolve(aa, transparent, effects.needs_depth());
    out.apply_post(&effects);
    if let Some(a) = alpha.as_mut() {
        out.merge_halo(a);
    }
    crate::prof::mark(19);

    let overlay = if let Some(ref ann_cfg) = config.annotations {
        let scale = 1.0 / aa as f64;
        let centroids: FxHashMap<u32, (f64, f64)> = compute_group_centroids(&projected)
            .into_iter()
            .map(|(gid, (x, y))| (gid, (x * scale, y * scale)))
            .collect();
        Some(overlay_annotations(config.width, config.height, &centroids, group_styles, ann_cfg))
    } else if config.debug {
        Some(overlay_debug(config.width, config.height, &tris, bmin, bmax, &view, config))
    } else {
        None
    };
    Ok(encode_raster(&out, alpha.as_deref(), overlay.as_deref()))
}

#[derive(Clone, Copy, PartialEq)]
enum EdgePass { Strokes, Wire }

#[allow(clippy::too_many_arguments)]
fn draw_edges(buf: &mut PixelBuffer, projected: &[ProjectedTri], config: &RenderConfig, group_styles: &GroupStyles,
              edge_masks: Option<&[u8]>, br: f64, off: (f64, f64), pass: EdgePass) {
    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";
    let bias = (br * 0.005) as f32;
    let edges = |buf: &mut PixelBuffer, tri: &ProjectedTri, (r, g, b): (u8, u8, u8), depth: bool| {
        let mask = edge_masks.and_then(|m| m.get(tri.src as usize)).copied().unwrap_or(crate::tessellate::ALL_EDGES);
        if !depth && mask == crate::tessellate::ALL_EDGES && off == (0.0, 0.0) {
            buf.draw_triangle_edges(&tri.pts, r, g, b);
            return;
        }
        for e in 0..3 {
            if (mask >> e) & 1 == 0 { continue; }
            let n = (e + 1) % 3;
            let (x0, y0, x1, y1) = (tri.pts[e].0 + off.0, tri.pts[e].1 + off.1, tri.pts[n].0 + off.0, tri.pts[n].1 + off.1);
            if depth {
                buf.draw_line_z(x0, y0, tri.depths[e] as f32, x1, y1, tri.depths[n] as f32, bias, r, g, b);
            } else {
                buf.draw_line(x0, y0, x1, y1, r, g, b);
            }
        }
    };
    match pass {
        EdgePass::Wire if is_wireframe || is_solid_wireframe => {
            let color = parse_hex_color(resolve_wireframe_color(config, is_solid_wireframe));
            for tri in projected {
                edges(buf, tri, color, is_solid_wireframe);
            }
        }
        EdgePass::Strokes if !is_wireframe => {
            let global = (config.stroke.color != "none" && config.stroke.width > 0.0).then(|| parse_hex_color(&config.stroke.color));
            if global.is_none() && !group_styles.values().any(|a| a.stroke.as_deref().is_some_and(|s| s != "none")) {
                return;
            }
            for tri in projected {
                let color = match tri.group_id.and_then(|gid| group_styles.get(&gid)) {
                    Some(a) if a.stroke_width.unwrap_or(config.stroke.width) <= 0.0 => None,
                    Some(a) => match a.stroke.as_deref() {
                        Some("none") => None,
                        Some(s) => Some(parse_hex_color(s)),
                        None => global,
                    },
                    None => global,
                };
                if let Some(c) = color {
                    edges(buf, tri, c, true);
                }
            }
        }
        _ => {}
    }
}

fn render_grid_png_buf(
    triangles: &[Triangle],
    config: &RenderConfig,
    views: &[(ViewParams, String)],
    br: f64,
    ground_z: f64,
    w: usize,
    h: usize,
    bg: (u8, u8, u8),
    group_styles: &GroupStyles,
) -> PixelBuffer {
    let (cols, rows) = grid_layout(views.len());
    let cell_w = w / cols;
    let cell_h = h / rows;
    let label_h = if config.grid_labels { (cell_h as f64 * 0.048).fround() as usize } else { 0 };
    let render_h = cell_h - label_h;
    let is_wireframe = config.mode == "wireframe";

    let (gbmin, gbmax) = compute_bbox(triangles);
    let lights = resolve_lights(config, bbox_center(gbmin, gbmax), br);
    let shadow_data = build_shadow_data(triangles, &lights, None, config, group_styles, bbox_center(gbmin, gbmax), br, false);
    let mut buf = PixelBuffer::new(w, h, bg);

    for (i, (view, _label)) in views.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let ox = (col * cell_w) as f64;
        let oy = (row * cell_h + label_h) as f64;

        let mut projected = project_triangles(
            triangles, None, config, view, cell_w as f64, render_h as f64, br, true, group_styles, &lights, shadow_data.as_ref(),
        );
        radix_sort_by_depth(&mut projected, true);

        if let Some(shadow_cfg) = &config.shadow {
            if !is_wireframe {
                let shadow = project_shadow(
                    triangles, config, shadow_light_dir(config, bbox_center(gbmin, gbmax)), view, cell_w as f64, render_h as f64, br, ground_z, true, &shadow_cfg.color,
                );
                let mut mask = vec![false; w * h];
                for tri in &shadow {
                    PixelBuffer::rasterize_shadow_mask_offset(&mut mask, w, h, &tri.pts, ox, oy);
                }
                let (sr, sg, sb) = parse_hex_color(&shadow_cfg.color);
                buf.apply_shadow(&mask, sr, sg, sb, shadow_cfg.opacity);
            }
        }

        if !is_wireframe {
            for tri in &projected {
                let pts_off = [
                    (tri.pts[0].0 + ox, tri.pts[0].1 + oy),
                    (tri.pts[1].0 + ox, tri.pts[1].1 + oy),
                    (tri.pts[2].0 + ox, tri.pts[2].1 + oy),
                ];
                let max_d = tri.depths[0].fmax(tri.depths[1]).fmax(tri.depths[2]) as f32;
                if buf.hiz_can_skip(&pts_off, max_d) { continue; }
                buf.rasterize_triangle_offset(&tri.pts, &tri.depths, tri.r, tri.g, tri.b, ox, oy);
            }
            if let Some(hc) = config.clip.as_ref().and_then(|c| c.hatch.as_ref()) {
                let color = parse_hex_color(&hc.color);
                let ang = hc.angle.to_radians();
                let (cos_a, sin_a) = (ang.cos(), ang.sin());
                let aa = (w / (config.width as usize).max(1)).max(1) as f64;
                let spacing = (hc.spacing * aa).fmax(0.5);
                let half_w = hc.width * aa * 0.5;
                let style = hatch_style_code(hc.style);
                let arm = spacing * HATCH_CROSS_ARM;
                for tri in &projected {
                    if tri.group_id == Some(clip::CAP_GID) && tri.opacity >= 1.0 {
                        let pts_off = [
                            (tri.pts[0].0 + ox, tri.pts[0].1 + oy),
                            (tri.pts[1].0 + ox, tri.pts[1].1 + oy),
                            (tri.pts[2].0 + ox, tri.pts[2].1 + oy),
                        ];
                        buf.hatch_triangle(&pts_off, &tri.depths, spacing, half_w, cos_a, sin_a, style, arm, color);
                    }
                }
            }
        }
        draw_edges(&mut buf, &projected, config, group_styles, None, br, (ox, oy), EdgePass::Strokes);
        draw_edges(&mut buf, &projected, config, group_styles, None, br, (ox, oy), EdgePass::Wire);
    }

    buf
}

/// Return JSON with model info for verbose/debug purposes.
/// Surface area, enclosed volume, and centre of mass of a triangle mesh.
///
/// Surface area is exact (sum of triangle areas). Volume and centroid use the
/// signed-tetrahedron (divergence) method: exact for a closed, consistently
/// wound surface, and still returned — but only approximate — for open or
/// non-manifold meshes. `volume` is reported as an absolute value so winding
/// direction doesn't flip its sign; `fallback_centroid` (the bbox centre) is
/// used when the mesh encloses ~no signed volume.
fn mesh_measures(triangles: &[Triangle], fallback_centroid: Vec3) -> (f64, f64, Vec3) {
    let mut area = 0.0;
    let mut vol6 = 0.0;
    let mut cacc = Vec3::new(0.0, 0.0, 0.0);
    for t in triangles {
        let (a, b, c) = (t.vertices[0], t.vertices[1], t.vertices[2]);
        area += 0.5 * b.sub(a).cross(c.sub(a)).length();
        let sv = a.dot(b.cross(c));
        vol6 += sv;
        cacc = cacc.add(a.add(b).add(c).scale(sv));
    }
    let volume = (vol6 / 6.0).abs();
    let centroid = if vol6.abs() > 1e-9 {
        cacc.scale(1.0 / (4.0 * vol6))
    } else {
        fallback_centroid
    };
    (area, volume, centroid)
}

pub fn get_info(triangles: &[Triangle], config: &RenderConfig) -> String {
    let decimated: Vec<Triangle>;
    let triangles: &[Triangle] = if config.decimate > 0.0 && !triangles.is_empty() {
        let (bmin, bmax) = compute_bbox(triangles);
        decimated = decimate::decimate(triangles, bmin, bmax, config.decimate);
        &decimated
    } else {
        triangles
    };

    let (bmin, bmax) = if triangles.is_empty() {
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0))
    } else {
        compute_bbox(triangles)
    };
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);
    let view = resolve_config_view(config, bc, br);
    let (surface_area, volume, centroid) = mesh_measures(triangles, bc);

    let mut s = String::with_capacity(320);
    s.push_str("{\"triangles\":"); push_usize(&mut s, triangles.len());
    s.push_str(",\"vertices\":"); push_usize(&mut s, count_unique_vertices(triangles));
    s.push_str(",\"bbox_min\":["); push_f4(&mut s, bmin.x); s.push(','); push_f4(&mut s, bmin.y); s.push(','); push_f4(&mut s, bmin.z);
    s.push_str("],\"bbox_max\":["); push_f4(&mut s, bmax.x); s.push(','); push_f4(&mut s, bmax.y); s.push(','); push_f4(&mut s, bmax.z);
    s.push_str("],\"bbox_center\":["); push_f4(&mut s, bc.x); s.push(','); push_f4(&mut s, bc.y); s.push(','); push_f4(&mut s, bc.z);
    s.push_str("],\"bbox_radius\":"); push_f4(&mut s, br);
    s.push_str(",\"size\":["); push_f4(&mut s, bmax.x - bmin.x); s.push(','); push_f4(&mut s, bmax.y - bmin.y); s.push(','); push_f4(&mut s, bmax.z - bmin.z);
    s.push_str("],\"surface_area\":"); push_f4(&mut s, surface_area);
    s.push_str(",\"volume\":"); push_f4(&mut s, volume);
    s.push_str(",\"centroid\":["); push_f4(&mut s, centroid.x); s.push(','); push_f4(&mut s, centroid.y); s.push(','); push_f4(&mut s, centroid.z);
    s.push_str("],\"camera\":["); push_f4(&mut s, view.camera.x); s.push(','); push_f4(&mut s, view.camera.y); s.push(','); push_f4(&mut s, view.camera.z);
    s.push_str("],\"center\":["); push_f4(&mut s, view.center.x); s.push(','); push_f4(&mut s, view.center.y); s.push(','); push_f4(&mut s, view.center.z);
    s.push_str("],\"projection\":\""); s.push_str(&config.projection);
    s.push_str("\",\"fov\":"); push_f2(&mut s, config.fov);
    s.push('}');
    s
}

fn rasterize_shadow_to_buf(buf: &mut PixelBuffer, shadow_tris: &[ProjectedTri], shadow: &ShadowConfig) {
    let mut mask = vec![false; buf.width * buf.height];
    for tri in shadow_tris {
        PixelBuffer::rasterize_shadow_mask(&mut mask, buf.width, buf.height, &tri.pts);
    }
    let (sr, sg, sb) = parse_hex_color(&shadow.color);
    buf.apply_shadow(&mask, sr, sg, sb, shadow.opacity);
}

