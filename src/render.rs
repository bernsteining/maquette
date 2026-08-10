use crate::annotations;
use crate::cache;
use crate::clip;
use crate::color_map;
use crate::config::{GroupAppearance, LightKind, RenderConfig, ShadowConfig};
use crate::decimate;
use crate::explode;
use crate::color::{linear_to_srgb, parse_hex_color, srgb_to_linear};
use crate::math::{quantize, fx_hashmap_cap, FxBuildHasher, FxHashMap, Mat4, Vec3, ViewMatSimd};
use crate::outline;
use crate::parser::Triangle;
use crate::rasterizer::PixelBuffer;
use crate::smooth;
use crate::projection::*;
use crate::shading::*;
use crate::svg::*;
use std::arch::wasm32::*;
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

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
    /// Per-pixel shadow data (world vertex positions + face normal). Set only
    /// when per-pixel shadows are active; the raster pass samples the maps here.
    pp: Option<([Vec3; 3], Vec3)>,
}

// ---------------------------------------------------------------------------
// Bounding box
// ---------------------------------------------------------------------------

fn bbox_of(iter: impl Iterator<Item = Vec3>) -> (Vec3, Vec3) {
    let mut min = Vec3::new(f64::MAX, f64::MAX, f64::MAX);
    let mut max = Vec3::new(f64::MIN, f64::MIN, f64::MIN);
    for v in iter {
        min.x = min.x.min(v.x);
        min.y = min.y.min(v.y);
        min.z = min.z.min(v.z);
        max.x = max.x.max(v.x);
        max.y = max.y.max(v.y);
        max.z = max.z.max(v.z);
    }
    (min, max)
}

fn compute_bbox(triangles: &[Triangle]) -> (Vec3, Vec3) {
    bbox_of(triangles.iter().flat_map(|t| t.vertices.iter().copied()))
}

pub(crate) fn bbox_center(min: Vec3, max: Vec3) -> Vec3 {
    Vec3::new(
        (min.x + max.x) / 2.0,
        (min.y + max.y) / 2.0,
        (min.z + max.z) / 2.0,
    )
}

pub(crate) fn bbox_radius(min: Vec3, max: Vec3) -> f64 {
    Vec3::new(max.x - min.x, max.y - min.y, max.z - min.z).length() / 2.0
}

/// Returns 0-3 for the quadrant of a 2D vector (used for atan2-free angle sorting)
#[inline(always)]
fn angle_quadrant(u: f64, v: f64) -> u8 {
    if u >= 0.0 { if v >= 0.0 { 0 } else { 3 } }
    else { if v >= 0.0 { 1 } else { 2 } }
}

/// Sort 3 u32 values in-place (branchless-friendly, avoids sort_unstable overhead)
#[inline(always)]
fn sort3(a: &mut u32, b: &mut u32, c: &mut u32) {
    if *a > *b { core::mem::swap(a, b); }
    if *b > *c { core::mem::swap(b, c); }
    if *a > *b { core::mem::swap(a, b); }
}

pub(crate) fn pointcloud_to_triangles(
    cloud: &crate::ply_parser::PointCloud,
    config: &RenderConfig,
) -> Vec<Triangle> {
    let n = cloud.positions.len();
    if n < 3 { return Vec::new(); }

    let positions = &cloud.positions;
    let (bmin, bmax) = bbox_of(positions.iter().copied());
    let diag = bmax.sub(bmin).length();
    if diag < 1e-12 { return Vec::new(); }

    let has_normals = cloud.normals.len() == n;
    let has_colors = cloud.colors.len() == n;

    let radius = if config.point_size > 0.0 {
        config.point_size
    } else {
        diag / (n as f64).cbrt() * 1.5
    };
    let rsq_f32 = (radius * radius) as f32;

    // f32 SOA for SIMD-accelerated distance checks
    let xs: Vec<f32> = positions.iter().map(|p| p.x as f32).collect();
    let ys: Vec<f32> = positions.iter().map(|p| p.y as f32).collect();
    let zs: Vec<f32> = positions.iter().map(|p| p.z as f32).collect();

    // Build spatial hash with FxHasher — also store cell coords to avoid recomputing
    let inv_cell_f32 = (1.0 / radius) as f32;
    let mut grid: HashMap<(i32, i32, i32), Vec<u32>, FxBuildHasher> =
        HashMap::with_hasher(FxBuildHasher::default());
    for i in 0..n {
        let cx = (xs[i] * inv_cell_f32).floor() as i32;
        let cy = (ys[i] * inv_cell_f32).floor() as i32;
        let cz = (zs[i] * inv_cell_f32).floor() as i32;
        grid.entry((cx, cy, cz)).or_default().push(i as u32);
    }

    let max_neighbors: usize = 12;
    let mut tri_set: HashSet<(u32, u32, u32), FxBuildHasher> =
        HashSet::with_hasher(FxBuildHasher::default());

    // Fallback normal if no normals provided: use camera direction
    let fallback_normal = if !has_normals {
        let bc = bbox_center(bmin, bmax);
        let br = bbox_radius(bmin, bmax);
        let view = resolve_config_view(config, bc, br);
        view.camera.sub(view.center).normalized()
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };

    // Reusable buffers with pre-reserved capacity
    let mut candidates: Vec<u32> = Vec::with_capacity(64);
    let mut neighbors: Vec<(u32, f32)> = Vec::with_capacity(max_neighbors + 4);
    let mut sorted: Vec<(u32, f64, f64)> = Vec::with_capacity(max_neighbors);

    // Iterate cell-by-cell: 27 HashMap lookups amortized across all points in each cell
    let cell_keys: Vec<(i32, i32, i32)> = grid.keys().copied().collect();
    for cell in &cell_keys {
        // Collect candidates from 27 neighbors — done once per cell, shared by all points
        candidates.clear();
        for dz in -1i32..=1 {
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if let Some(bucket) = grid.get(&(cell.0 + dx, cell.1 + dy, cell.2 + dz)) {
                        candidates.extend_from_slice(bucket);
                    }
                }
            }
        }

        let rsq4 = f32x4_splat(rsq_f32);

        for &ii in &grid[cell] {
            let i = ii as usize;
            let normal = if has_normals { cloud.normals[i] } else { fallback_normal };

            // SIMD f32x4 distance check — 4 candidates per iteration
            neighbors.clear();
            let px4 = f32x4_splat(xs[i]);
            let py4 = f32x4_splat(ys[i]);
            let pz4 = f32x4_splat(zs[i]);
            let self_idx = i32x4_splat(ii as i32);

            let mut k = 0;
            let len = candidates.len();
            while k + 4 <= len {
                let j0 = candidates[k] as usize;
                let j1 = candidates[k + 1] as usize;
                let j2 = candidates[k + 2] as usize;
                let j3 = candidates[k + 3] as usize;

                let jv = i32x4(candidates[k] as i32, candidates[k + 1] as i32,
                               candidates[k + 2] as i32, candidates[k + 3] as i32);
                let not_self = v128_not(i32x4_eq(jv, self_idx));

                let dx = f32x4_sub(f32x4(xs[j0], xs[j1], xs[j2], xs[j3]), px4);
                let dy = f32x4_sub(f32x4(ys[j0], ys[j1], ys[j2], ys[j3]), py4);
                let dz = f32x4_sub(f32x4(zs[j0], zs[j1], zs[j2], zs[j3]), pz4);
                let dsq = f32x4_add(f32x4_add(f32x4_mul(dx, dx), f32x4_mul(dy, dy)),
                                    f32x4_mul(dz, dz));

                let pass = v128_and(not_self, f32x4_lt(dsq, rsq4));
                let mask = i32x4_bitmask(pass);

                if mask & 1 != 0 { neighbors.push((candidates[k],   f32x4_extract_lane::<0>(dsq))); }
                if mask & 2 != 0 { neighbors.push((candidates[k+1], f32x4_extract_lane::<1>(dsq))); }
                if mask & 4 != 0 { neighbors.push((candidates[k+2], f32x4_extract_lane::<2>(dsq))); }
                if mask & 8 != 0 { neighbors.push((candidates[k+3], f32x4_extract_lane::<3>(dsq))); }

                k += 4;
            }
            while k < len {
                let j = candidates[k];
                if j != ii {
                    let dx = xs[j as usize] - xs[i];
                    let dy = ys[j as usize] - ys[i];
                    let dz = zs[j as usize] - zs[i];
                    let dsq = dx * dx + dy * dy + dz * dz;
                    if dsq < rsq_f32 { neighbors.push((j, dsq)); }
                }
                k += 1;
            }

            if neighbors.len() < 2 { continue; }

            // Keep only closest max_neighbors
            if neighbors.len() > max_neighbors {
                neighbors.select_nth_unstable_by(max_neighbors, |a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                neighbors.truncate(max_neighbors);
            }

            // Build tangent frame from normal
            let (t1, t2) = normal.tangent_basis();

            // Project neighbors onto tangent plane using f64x2 (u and v simultaneously)
            sorted.clear();
            let t1x_t2x = f64x2(t1.x, t2.x);
            let t1y_t2y = f64x2(t1.y, t2.y);
            let t1z_t2z = f64x2(t1.z, t2.z);
            let p = positions[i];
            for &(j, _) in &neighbors {
                let q = positions[j as usize];
                let uv = f64x2_add(f64x2_add(
                    f64x2_mul(f64x2_splat(q.x - p.x), t1x_t2x),
                    f64x2_mul(f64x2_splat(q.y - p.y), t1y_t2y)),
                    f64x2_mul(f64x2_splat(q.z - p.z), t1z_t2z));
                sorted.push((j, f64x2_extract_lane::<0>(uv), f64x2_extract_lane::<1>(uv)));
            }

            // Sort by angle around normal: quadrant + cross product (no atan2)
            sorted.sort_unstable_by(|a, b| {
                let qa = angle_quadrant(a.1, a.2);
                let qb = angle_quadrant(b.1, b.2);
                if qa != qb { return qa.cmp(&qb); }
                let cross = a.1 * b.2 - a.2 * b.1;
                if cross > 0.0 { core::cmp::Ordering::Less }
                else if cross < 0.0 { core::cmp::Ordering::Greater }
                else { core::cmp::Ordering::Equal }
            });

            // Fan-triangulate: create triangle (i, neighbor[k], neighbor[k+1])
            let nn = sorted.len();
            for k in 0..nn {
                let ja = sorted[k].0;
                let jb = sorted[(k + 1) % nn].0;
                if ja == jb { continue; }

                let (mut a, mut b, mut c) = (ii, ja, jb);
                sort3(&mut a, &mut b, &mut c);
                tri_set.insert((a, b, c));
            }
        }
    }

    // Convert deduplicated triangles to Triangle structs
    let mut triangles = Vec::with_capacity(tri_set.len());
    for &(a, b, c) in &tri_set {
        let (ia, ib, ic) = (a as usize, b as usize, c as usize);
        let (pa, pb, pc) = (positions[ia], positions[ib], positions[ic]);
        let normal = if has_normals {
            cloud.normals[ia].add(cloud.normals[ib]).add(cloud.normals[ic]).normalized()
        } else {
            match Vec3::face_normal(pa, pb, pc) {
                Some(n) => n,
                None => continue,
            }
        };
        let color = if has_colors { Some(cloud.colors[ia]) } else { None };
        let vertex_colors = if has_colors {
            Some([cloud.colors[ia], cloud.colors[ib], cloud.colors[ic]])
        } else {
            None
        };
        triangles.push(Triangle {
            vertices: [pa, pb, pc],
            normal,
            color,
            vertex_colors,
            group_id: None,
        });
    }

    triangles
}

// ---------------------------------------------------------------------------
// Core triangle projection
// ---------------------------------------------------------------------------

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
                let ls = self.light_sizes[li];
                sum += if ls > 0.0 {
                    m.lit_pcss(p, normal, &self.bias, self.softness, ls)
                } else {
                    m.lit(p, normal, &self.bias, self.softness)
                };
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
            let m = keep * tint;          // shadow floor for this channel
            let mul = m + t * (1.0 - m);  // lerp floor→1 by lit factor
            (cc as f32 * mul).round().clamp(0.0, 255.0) as u8
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
    group_styles: &HashMap<u32, GroupAppearance>,
    bc: Vec3,
    br: f64,
    allow_per_pixel: bool,
) -> Option<ShadowData> {
    let cfg = config.shadows.as_ref()?;
    if cfg.strength <= 0.0 || lights.is_empty() {
        return None;
    }
    // Per-pixel is PNG-only; SVG callers pass allow_per_pixel = false.
    let per_pixel = cfg.per_pixel && allow_per_pixel;
    let tint = if cfg.color.is_empty() { None } else { Some(parse_hex_color(&cfg.color)) };
    let up = Vec3::from(config.up);
    // Occluder filter: skip triangles that are (nearly) transparent so a glassy
    // or x-ray part doesn't cast a solid shadow.
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

    // Precompute per-unique-vertex factors for the smooth paths (per-vertex
    // mode only; per-pixel samples the maps during rasterization instead).
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
    // Per-light PCSS radius: an area light uses its own `size`; every other
    // light falls back to the global `shadows.light_size` (backward compatible).
    let light_sizes: Vec<f64> = lights.iter()
        .map(|l| if l.kind == LightKind::Area { l.size } else { cfg.light_size })
        .collect();
    Some(ShadowData { maps, bias, strength, softness: cfg.softness, factors, per_pixel, tint, light_sizes, ambient_keep_base })
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
    group_styles: &HashMap<u32, GroupAppearance>,
    lights: &[ResolvedLight],
    shadow: Option<&ShadowData>,
) -> Vec<ProjectedTri> {
    // Smooth paths consume the precomputed per-vertex factors; the flat path
    // (below) samples the maps directly, so it needs the full bundle.
    let shadow_factors: Option<&[f32]> = shadow.and_then(|s| s.factors.as_deref());
    let proj = if force_ortho { Projection::Ortho } else { resolve_projection(&config.projection) };
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let view_simd = ViewMatSimd::from_mat4(&view_mat);
    let (base_r, base_g, base_b) = parse_hex_color(&config.color);
    let is_wireframe = config.mode == "wireframe";
    let is_xray = config.mode == "x-ray";
    let skip_cull = matches!(proj, Projection::Cabinet | Projection::Cavalier | Projection::TinyPlanet);
    let do_cull = config.cull_backface && !is_wireframe && !is_xray && !skip_cull && config.explode.abs() < 1e-12;

    // Back-face test computed ONCE and reused by the shade-skip and the cull
    // loop below (avoids computing it twice). `true` = back-facing.
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
    let tm = match config.tone_mapping.method.as_str() { "reinhard" => ToneMapMethod::Reinhard, "aces" => ToneMapMethod::Aces, _ => ToneMapMethod::None };
    let shading = match config.shading.as_str() {
        "gooch" => ShadingMode::Gooch, "cel" => ShadingMode::Cel,
        "flat" => ShadingMode::Flat, "normal" => ShadingMode::Normal, _ => ShadingMode::BlinnPhong,
    };
    let (gooch_warm, gooch_cool) = if shading == ShadingMode::Gooch {
        let w = { let (r, g, b) = parse_hex_color(&config.gooch_warm); (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)) };
        let c = { let (r, g, b) = parse_hex_color(&config.gooch_cool); (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)) };
        (w, c)
    } else {
        ((0.0f32, 0.0f32, 0.0f32), (0.0f32, 0.0f32, 0.0f32))
    };

    // Pre-compute power LUTs: x^shininess and x^fresnel_power for x ∈ [0,1].
    // Rebuilds only when exponent changes (handles per-group shininess overrides).
    let fresnel_lut = if !is_wireframe && config.fresnel.intensity > 0.0 {
        let mut lut = [0.0f32; 256];
        let fp = config.fresnel.power as f32;
        for i in 0..256 { lut[i] = (i as f32 / 255.0).powf(fp); }
        lut
    } else {
        [0.0f32; 256]
    };
    let (sss_lut, sss_intensity, sss_dist) = if let Some(ref sc) = config.sss {
        let mut lut = [0.0f32; 256];
        let p = sc.power as f32;
        for i in 0..256 { lut[i] = (i as f32 / 255.0).powf(p); }
        (lut, sc.intensity as f32, sc.distortion as f32)
    } else {
        ([0.0f32; 256], 0.0f32, 0.0f32)
    };
    let cfg_fresnel = config.fresnel.intensity as f32;
    let cfg_gamma = config.gamma_correction;
    let cfg_exposure = config.tone_mapping.exposure as f32;
    let cfg_cel_bands = config.cel_bands;
    let cfg_xray_opacity = config.xray_opacity;
    let cfg_ambient_intensity = config.ambient.intensity as f32;
    // Pre-parse hemisphere sky/ground colors (pre-multiplied by intensity)
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

    // Pre-build one specular LUT per distinct shininess (global + group overrides)
    // so the per-triangle slow path looks up a ready table instead of rebuilding
    // 256 powf() values whenever interleaved groups change shininess.
    fn lut_ref<'a>(luts: &'a [(f32, [f32; 256])], sh: f32) -> &'a [f32; 256] {
        const ZERO: [f32; 256] = [0.0f32; 256];
        luts.iter().find(|(s, _)| *s == sh).map(|(_, l)| l).unwrap_or(&ZERO)
    }
    let spec_luts: Vec<(f32, [f32; 256])> = {
        let mut v: Vec<(f32, [f32; 256])> = Vec::new();
        if cfg_specular > 0.0 || group_styles.values().any(|a| a.specular.map_or(false, |s| s > 0.0)) {
            let mut add = |sh: f32| {
                if !v.iter().any(|(s, _)| *s == sh) {
                    let mut lut = [0.0f32; 256];
                    for i in 0..256 { lut[i] = (i as f32 / 255.0).powf(sh); }
                    v.push((sh, lut));
                }
            };
            add(cfg_shininess);
            for a in group_styles.values() {
                if let Some(sh) = a.shininess { add(sh as f32); }
            }
        }
        v
    };
    let lights_f32: Vec<LightF32> = lights.iter().map(|l| {
        // Disk area light: subdue the highlight in proportion to the light's
        // angular radius (size / distance-to-subject). A cheap stand-in for a
        // true broadened lobe — big soft lights don't produce a tight glint.
        let spec_scale = if l.kind == LightKind::Area && l.size > 0.0 {
            let d = l.vector.sub(view.center).length().max(1e-3);
            (1.0 / (1.0 + 4.0 * (l.size / d))) as f32
        } else { 1.0 };
        LightF32 {
            kind: l.kind,
            dx: l.vector.x as f32, dy: l.vector.y as f32, dz: l.vector.z as f32,
            cr: l.color.0, cg: l.color.1, cb: l.color.2,
            scr: l.color.0 * spec_scale, scg: l.color.1 * spec_scale, scb: l.color.2 * spec_scale,
        }
    }).collect();

    // Hemisphere ambient blend: lerp sky↔ground based on normal·up
    #[inline(always)]
    fn hemi_ambient(n: Vec3, sky: (f32, f32, f32), gnd: (f32, f32, f32), up: (f32, f32, f32)) -> (f32, f32, f32) {
        let t = (n.x as f32 * up.0 + n.y as f32 * up.1 + n.z as f32 * up.2 + 1.0) * 0.5;
        (gnd.0 + (sky.0 - gnd.0) * t, gnd.1 + (sky.1 - gnd.1) * t, gnd.2 + (sky.2 - gnd.2) * t)
    }

    // Memoized smooth shading: shade each unique vertex once when possible.
    // Valid when base color is uniform (no per-tri color, no vertex_colors)
    // and shading params are uniform (no per-group material overrides, not x-ray).
    let shade_cache: Option<Vec<(u8, u8, u8)>> = if let Some(sd) = smooth {
        let groups_uniform = group_styles.values().all(|a|
            a.specular.is_none() && a.shininess.is_none() && a.ambient.is_none());
        let can_memoize = !is_wireframe && !is_xray && groups_uniform
            && triangles.iter().all(|t| t.color.is_none() && t.vertex_colors.is_none());
        if can_memoize {
            let spec_lut = lut_ref(&spec_luts, cfg_shininess);
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
                // Shade only vertices touching a front-facing (un-culled) triangle
                // when culling is on and there are no cast shadows (whose dense
                // per-vertex factors would also need compacting). Back-facing-only
                // vertices are never rasterized — byte-identical, fewer shade calls.
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
                // Reused per-light shadow-factor lanes for the current batch of 4.
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
                        tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
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
                        tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                        spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        shadow_factors.map(|f| (f, n_unique, i)),
                    )
                }).collect()
            };
            Some(cache)
        } else { None }
    } else { None };

    // Back-face culling direction. Perspective uses the per-triangle vector to
    // the camera *point* (a converging eye). Parallel projections (orthographic
    // plus the axonometric family — all Projection::Ortho here) instead need a
    // single constant view direction; using the per-triangle camera vector there
    // over-culls silhouette faces and punches holes when the camera is close.
    // Kept normal-based (not screen-space winding) so meshes whose winding is
    // inconsistent with their normals — e.g. point-cloud reconstructions — stay
    // correct.
    let mut projected: Vec<ProjectedTri> = Vec::with_capacity(triangles.len());

    // Slow-path shadow scratch: gathered per-light lanes for a triangle's 3 verts.
    let sh_n_lights = lights_f32.len();
    let sh_stride = smooth.map(|s| s.positions.len()).unwrap_or(0);
    let mut slow_sh_scratch: Vec<v128> = vec![f32x4_splat(1.0); sh_n_lights];
    // Flat-path shadow scratch: one factor per light, sampled at the face centroid.
    let mut flat_sh_scratch: Vec<f32> = vec![1.0; sh_n_lights];

    for (ti, tri) in triangles.iter().enumerate() {
        let is_back_facing = if do_cull || is_xray { face_back[ti] } else { false };

        if do_cull && is_back_facing {
            continue;
        }

        // SIMD f32 batch transform: 3 vertices at once (9 SIMD mul-adds vs 27 scalar)
        let cam = view_simd.transform_tri(tri.vertices[0], tri.vertices[1], tri.vertices[2]);

        // Wireframe mode: skip all shading, only need projection
        let (r, g, b, vertex_colors, opacity) = if is_wireframe {
            (0, 0, 0, None, 1.0)
        } else if let Some(ref cache) = shade_cache {
            // Fast path: look up pre-computed vertex colors from cache
            let sd = unsafe { smooth.unwrap_unchecked() };
            let [i0, i1, i2] = sd.tri_indices[ti];
            let vcols = [cache[i0], cache[i1], cache[i2]];
            let (r, g, b) = crate::color::avg3(vcols[0], vcols[1], vcols[2]);
            let opacity = tri.group_id.and_then(|gid| group_styles.get(&gid))
                .and_then(|a| a.opacity).unwrap_or(config.opacity);
            (r, g, b, Some(vcols), opacity)
        } else {
            // Per-group appearance overrides
            let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
            // Per-group ambient override scales intensity; sky/ground colors stay global
            let grp_intensity = ga.and_then(|a| a.ambient).map(|v| v as f32).unwrap_or(cfg_ambient_intensity);
            let intensity_scale = if grp_intensity == cfg_ambient_intensity { 1.0 } else { grp_intensity / cfg_ambient_intensity.max(1e-6) };
            let grp_sky = (amb_sky.0 * intensity_scale, amb_sky.1 * intensity_scale, amb_sky.2 * intensity_scale);
            let grp_gnd = (amb_gnd.0 * intensity_scale, amb_gnd.1 * intensity_scale, amb_gnd.2 * intensity_scale);
            let one_minus_ambient = 1.0 - grp_intensity;
            let mut specular = ga.and_then(|a| a.specular).map(|v| v as f32).unwrap_or(cfg_specular);
            let shininess = ga.and_then(|a| a.shininess).map(|v| v as f32).unwrap_or(cfg_shininess);
            let mut opacity = ga.and_then(|a| a.opacity).unwrap_or(config.opacity);

            // Look up the pre-built specular LUT for this shininess (no rebuild).
            let spec_lut = lut_ref(&spec_luts, shininess);

            // X-ray mode: set opacity based on face orientation
            if is_xray {
                if is_back_facing {
                    opacity = 1.0;
                    specular = 0.0;
                } else {
                    opacity = cfg_xray_opacity;
                }
            }

            let (fr, fg, fb) = tri.color.unwrap_or((base_r, base_g, base_b));

            if let Some(sd) = smooth {
                // Smooth shading: per-vertex lighting (slow path with per-tri overrides)
                let [i0, i1, i2] = sd.tri_indices[ti];
                let vn = [sd.normals[i0], sd.normals[i1], sd.normals[i2]];

                // SIMD batch path: 3 vertices in one shade_batch_4 call
                // Valid when base color is uniform (no vertex colors)
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
                        is_gooch, gooch_warm, gooch_cool,
                        sh4,
                    );
                    [colors[0], colors[1], colors[2]]
                } else {
                    // Scalar fallback: per-vertex colors or Normal
                    let gamma_or_gooch = cfg_gamma || shading == ShadingMode::Gooch;
                    let mut vcols = [(0u8, 0u8, 0u8); 3];
                    for i in 0..3 {
                        let (vr, vg, vb) = if let Some(vc) = tri.vertex_colors { vc[i] } else { (fr, fg, fb) };
                        let base_lin = if gamma_or_gooch {
                            (srgb_to_linear(vr), srgb_to_linear(vg), srgb_to_linear(vb))
                        } else {
                            (vr as f32 / 255.0, vg as f32 / 255.0, vb as f32 / 255.0)
                        };
                        let amb = hemi_ambient(vn[i], grp_sky, grp_gnd, up_f32);
                        let uidx = [i0, i1, i2][i];
                        vcols[i] = shade_point(
                            vn[i], tri.vertices[i], base_lin,
                            &lights_f32, view_camera, amb, one_minus_ambient, specular,
                            cfg_fresnel, cfg_gamma,
                            tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                            spec_lut, &fresnel_lut,
                            sss_intensity, sss_dist, &sss_lut,
                            shadow_factors.map(|f| (f, sh_stride, uidx)),
                        );
                    }
                    vcols
                };

                let (r, g, b) = crate::color::avg3(vcols[0], vcols[1], vcols[2]);
                (r, g, b, Some(vcols), opacity)
            } else {
                // Flat shading: single face normal
                let centroid = Vec3::centroid(tri.vertices[0], tri.vertices[1], tri.vertices[2]);
                let amb = hemi_ambient(tri.normal, grp_sky, grp_gnd, up_f32);
                let base_lin = if cfg_gamma || shading == ShadingMode::Gooch {
                    (srgb_to_linear(fr), srgb_to_linear(fg), srgb_to_linear(fb))
                } else {
                    (fr as f32 / 255.0, fg as f32 / 255.0, fb as f32 / 255.0)
                };
                // Flat shading has no smooth vertices, so sample the shadow maps
                // per-face at the centroid (stride 1, index 0 → f[li]).
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
                    tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                    spec_lut, &fresnel_lut,
                    sss_intensity, sss_dist, &sss_lut,
                    flat_shadow,
                );
                (r, g, b, None, opacity)
            }
        };

        let pts = apply_projection(&proj_setup, &cam);
        let depths = [cam[0].z, cam[1].z, cam[2].z];
        let depth = (depths[0] + depths[1] + depths[2]) / 3.0;
        let pp = if shadow.map_or(false, |s| s.per_pixel) {
            Some((tri.vertices, tri.normal))
        } else {
            None
        };
        projected.push(ProjectedTri { pts, depths, depth, r, g, b, vertex_colors, group_id: tri.group_id, opacity, pp });
    }

    projected
}

// Reusable scratch for the depth radix sort. WASM is single-threaded and the
// sort is non-reentrant, so static buffers avoid re-allocating keys/indices and
// the gathered output on every call (every render sorts at least once; grid and
// turntable sort many times). Same safety justification as the model cache.
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

    // Quantize f64 depths to u32 sort keys.
    // Flip bits so that IEEE-754 ordering becomes unsigned ordering.
    keys.clear();
    keys.reserve(n);
    for tri in projected.iter() {
        let bits = (tri.depth as f32).to_bits();
        let k = if bits & 0x8000_0000 != 0 { !bits } else { bits ^ 0x8000_0000 };
        keys.push(if descending { !k } else { k });
    }

    // Compute all 4 byte-histograms in a single pass, then prefix-sum them.
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

    // Two index buffers in a single allocation; ping-pong via split_at_mut.
    idx.clear();
    idx.resize(2 * n, 0);
    for i in 0..n { idx[i] = i as u32; }

    {
        let (a, b) = idx.split_at_mut(n);

        // Pass 1: a → b (byte 0)
        for &v in a.iter() {
            let bucket = (keys[v as usize] & 0xFF) as usize;
            b[hist[0][bucket] as usize] = v;
            hist[0][bucket] += 1;
        }
        // Pass 2: b → a (byte 1)
        for &v in b.iter() {
            let bucket = ((keys[v as usize] >> 8) & 0xFF) as usize;
            a[hist[1][bucket] as usize] = v;
            hist[1][bucket] += 1;
        }
        // Pass 3: a → b (byte 2)
        for &v in a.iter() {
            let bucket = ((keys[v as usize] >> 16) & 0xFF) as usize;
            b[hist[2][bucket] as usize] = v;
            hist[2][bucket] += 1;
        }
        // Pass 4: b → a (byte 3) — result lands in a = idx[0..n]
        for &v in b.iter() {
            let bucket = ((keys[v as usize] >> 24) & 0xFF) as usize;
            a[hist[3][bucket] as usize] = v;
            hist[3][bucket] += 1;
        }
    }

    // Gather into the reusable output buffer. ptr::read moves each large struct
    // exactly once; set_len(0) then prevents `projected` from dropping the moved
    // elements. Swapping hands the sorted buffer to the caller and recycles the
    // now-empty old buffer for the next call.
    out.clear();
    out.reserve(n);
    let ptr = projected.as_mut_ptr();
    for i in 0..n {
        out.push(unsafe { std::ptr::read(ptr.add(idx[i] as usize)) });
    }
    unsafe { projected.set_len(0); }
    std::mem::swap(projected, out);
}

// ---------------------------------------------------------------------------
// Ground shadow projection
// ---------------------------------------------------------------------------

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

    // No shadow if light is at or below ground level
    if light_dir.z <= 0.01 {
        return Vec::new();
    }

    let proj = if force_ortho { Projection::Ortho } else { resolve_projection(&config.projection) };
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let (sr, sg, sb) = parse_hex_color(shadow_color);

    let mut projected: Vec<ProjectedTri> = Vec::with_capacity(triangles.len());

    for tri in triangles {
        // Project each vertex onto the ground plane along the light direction
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
        projected.push(ProjectedTri { pts, depths, depth, r: sr, g: sg, b: sb, vertex_colors: None, group_id: None, opacity: 1.0, pp: None });
    }

    projected
}

// ---------------------------------------------------------------------------
// SVG building helpers
// ---------------------------------------------------------------------------

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
        // A single family of vertical lines (tiled → parallel section lines).
        HatchStyle::Lines => push_line(0.0, 0.0, 0.0, s),
        // Vertical + horizontal lines → a cross-hatch grid.
        HatchStyle::Cross => {
            push_line(0.0, 0.0, 0.0, s);
            push_line(0.0, 0.0, s, 0.0);
        }
        // A `+` mark centred in each cell → a grid of plus signs.
        HatchStyle::Crosses => {
            let (c, arm) = (s * 0.5, s * HATCH_CROSS_ARM);
            push_line(c, c - arm, c, c + arm);
            push_line(c - arm, c, c + arm, c);
        }
    }
    svg.push_str("</pattern></defs>");
}

fn write_solid_polygon(svg: &mut String, tri: &ProjectedTri, global_stroke: Option<(&str, f64)>, group_styles: &HashMap<u32, GroupAppearance>, hatch: bool) {
    svg.push_str("<polygon points=\"");
    push_tri_points(svg, &tri.pts);
    svg.push_str("\" fill=\"");
    push_hex_color(svg, tri.r, tri.g, tri.b);
    svg.push('"');
    // Per-group opacity
    if tri.opacity < 1.0 {
        svg.push_str(" fill-opacity=\"");
        push_f2(svg, tri.opacity);
        svg.push('"');
    }
    // Debug area-light disk: clean fill, no edge strokes (would show fan spokes).
    if tri.group_id == Some(DEBUG_DISK_GID) {
        svg.push_str("/>");
        return;
    }
    // Debug light octahedron faces
    if tri.group_id == Some(u32::MAX) {
        svg.push_str(" stroke=\"#333\" stroke-width=\"0.5\" stroke-linejoin=\"round\"/>");
        return;
    }
    // Per-group stroke overrides
    let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
    let has_group_stroke = ga.map_or(false, |a| {
        a.stroke.as_deref().map_or(false, |s| s != "none") && a.stroke_width.unwrap_or(1.0) > 0.0
    });
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
    // Section hatching: overlay the cap face with the hatch pattern, in paint
    // order right after its solid fill so nearer geometry still occludes it.
    if hatch && tri.group_id == Some(clip::CAP_GID) {
        svg.push_str("<polygon points=\"");
        push_tri_points(svg, &tri.pts);
        svg.push_str("\" fill=\"url(#maq-hatch)\" stroke=\"none\"/>");
    }
}

fn write_wireframe_polygon(svg: &mut String, tri: &ProjectedTri, color: &str, width: f64) {
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

// ---------------------------------------------------------------------------
// Turntable views
// ---------------------------------------------------------------------------

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
        push_i32(&mut s, (i as f64 * step).round() as i32);
        s.push('°');
        s
    }).collect()
}

// ---------------------------------------------------------------------------
// Preprocessing pipeline
// ---------------------------------------------------------------------------

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
            view.center.sub(view.camera).normalized() // forward: camera → scene
        }
        ClipSource::Axis(0) => Vec3::new(1.0, 0.0, 0.0),
        ClipSource::Axis(1) => Vec3::new(0.0, 1.0, 0.0),
        ClipSource::Axis(_) => Vec3::new(0.0, 0.0, 1.0),
        ClipSource::Normal(v) => Vec3::from(*v).normalized(),
    };
    // Extent of the model projected onto the normal.
    let corners = [
        Vec3::new(bmin.x, bmin.y, bmin.z), Vec3::new(bmax.x, bmin.y, bmin.z),
        Vec3::new(bmin.x, bmax.y, bmin.z), Vec3::new(bmax.x, bmax.y, bmin.z),
        Vec3::new(bmin.x, bmin.y, bmax.z), Vec3::new(bmax.x, bmin.y, bmax.z),
        Vec3::new(bmin.x, bmax.y, bmax.z), Vec3::new(bmax.x, bmax.y, bmax.z),
    ];
    let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in corners { let t = n.dot(c); tmin = tmin.min(t); tmax = tmax.max(t); }
    // Position of the cut along the normal (measured from the near side).
    let t = match clip.distance {
        Some(d) => tmin + d,
        None => tmin + clip.depth.clamp(0.0, 1.0) * (tmax - tmin),
    };
    // keep_far → keep `n·x >= t`; keep_near → flip the normal so `-n·x >= -t`.
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

    // 0. Decimation (vertex clustering) — runs first so every later stage and
    //    the shading itself operate on the reduced mesh.
    if config.decimate > 0.0 {
        tris = decimate::decimate(&tris, bmin, bmax, config.decimate);
        if !tris.is_empty() {
            let (new_min, new_max) = compute_bbox(&tris);
            bmin = new_min;
            bmax = new_max;
        }
    }

    // 1. Color mapping
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
                    // If parsing fails, skip scalar mapping
                    eprintln!("Scalar function error: {}", e);
                }
            }
            _ => {}
        }
    }

    // 2. Clipping
    if let Some(clip_cfg) = &config.clip {
        let (plane, cap) = resolve_clip(clip_cfg, bmin, bmax, config);
        // Fall back to the model's base color for plain (uncolored) meshes, so the
        // clipped surface and cap inherit `color` instead of a hardcoded gray.
        let base = parse_hex_color(&config.color);
        tris = clip::clip_triangles(&tris, plane, cap, base);
    }

    // 3. Explode
    if config.explode.abs() > 1e-12 {
        let bc = bbox_center(bmin, bmax);
        explode::explode_triangles(&mut tris, bc, config.explode);
    }

    // 4. Recompute bbox after clipping/exploding
    if config.clip.is_some() || config.explode.abs() > 1e-12 {
        if !tris.is_empty() {
            let (new_min, new_max) = compute_bbox(&tris);
            bmin = new_min;
            bmax = new_max;
        }
    }

    // 5. Normalize face normals (avoids per-triangle normalize in shade_point)
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
    #[inline]
    fn m(h: u64, x: u64) -> u64 { (h ^ x).wrapping_mul(0x100000001b3) }
    let c = match &config.clip { None => return m(h, 0x2), Some(c) => c };
    let mut h = m(h, 0x1);
    match &c.source {
        ClipSource::Plane(p) => { h = m(h, 10); for &v in p { h = m(h, v.to_bits()); } }
        ClipSource::Camera => {
            h = m(h, 11);
            match config.camera { Some(cam) => for v in cam { h = m(h, v.to_bits()); }, None => h = m(h, 0x9E) }
            h = m(h, config.azimuth.to_bits());
            h = m(h, config.elevation.to_bits());
            h = m(h, config.distance.unwrap_or(0.0).to_bits());
            for &b in config.projection.as_bytes() { h = m(h, b as u64); }
            for &u in &config.up { h = m(h, u.to_bits()); }
        }
        ClipSource::Axis(a) => { h = m(h, 12); h = m(h, *a as u64); }
        ClipSource::Normal(n) => { h = m(h, 13); for &v in n { h = m(h, v.to_bits()); } }
    }
    h = m(h, c.depth.to_bits());
    h = m(h, c.distance.map(|d| d.to_bits()).unwrap_or(0xDEAD));
    h = m(h, c.keep_far as u64);
    m(h, c.cap as u64)
}

fn smooth_geom_key(data_key: u64, config: &RenderConfig) -> u64 {
    #[inline]
    fn mix(h: u64, x: f64) -> u64 {
        (h ^ x.to_bits()).wrapping_mul(0x100000001b3)
    }
    let mut h = mix(data_key, config.decimate);
    h = mix(h, config.explode);
    h = mix(h, config.point_size);
    clip_key(h, config)
}

/// Look up (or compute and cache) the smooth vertex normals for `tris`.
fn cached_smooth<'a>(
    data_key: u64,
    config: &RenderConfig,
    tris: &[Triangle],
) -> &'a smooth::SmoothData {
    let key = smooth_geom_key(data_key, config);
    if let Some(s) = cache::get_smooth(key) {
        return s;
    }
    cache::put_smooth(key, smooth::compute_vertex_normals(tris));
    cache::get_smooth(key).unwrap()
}

/// Cache key for the preprocessed mesh: the model-data hash mixed with EVERY
/// config field that `preprocess` reads, so a hit can only occur for inputs that
/// produce an identical mesh. Must be kept in lock-step with `preprocess`.
fn prep_cache_key(base: u64, config: &RenderConfig) -> u64 {
    #[inline]
    fn m(h: u64, x: u64) -> u64 {
        (h ^ x).wrapping_mul(0x100000001b3)
    }
    let mut h = base;
    // Color mapping (sets vertex colors).
    for &b in config.color_map.as_bytes() { h = m(h, b as u64); }
    h = m(h, 0xF1);
    for s in &config.color_map_palette {
        for &b in s.as_bytes() { h = m(h, b as u64); }
        h = m(h, 0xF2);
    }
    for &b in config.scalar_function.as_bytes() { h = m(h, b as u64); }
    h = m(h, 0xF3);
    h = m(h, config.overhang_angle.to_bits());
    h = m(h, config.vertex_smoothing as u64);
    for &u in &config.up { h = m(h, u.to_bits()); }
    // Clipping (geometry) + explode + decimate.
    h = clip_key(h, config);
    // Clipping bakes the model base color into the cap/clipped vertex colors for
    // plain meshes, so the preprocessed mesh depends on `color` when clip is on.
    if config.clip.is_some() {
        for &b in config.color.as_bytes() { h = m(h, b as u64); }
        h = m(h, 0xF4);
    }
    h = m(h, config.explode.to_bits());
    h = m(h, config.decimate.to_bits());
    h
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
        let key = prep_cache_key(base, config);
        if let Some(e) = cache::get_prep(key) {
            return (&e.0, e.1, e.2);
        }
        cache::put_prep(key, preprocess(triangles, config));
        let e = cache::get_prep(key).unwrap();
        return (&e.0, e.1, e.2);
    }
    *owned = Some(preprocess(triangles, config));
    let e = owned.as_ref().unwrap();
    (&e.0, e.1, e.2)
}

// ---------------------------------------------------------------------------
// Point projection helper (for dimensions/outlines)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

pub fn render(triangles: &[Triangle], config: &RenderConfig, group_styles: &HashMap<u32, GroupAppearance>, data_key: Option<u64>, prep_key: Option<u64>) -> String {
    if triangles.is_empty() {
        return build_empty_svg(config);
    }

    // Preprocessing pipeline (cached when the mesh geometry/colors are unchanged)
    let mut prep_owned: Option<(Vec<Triangle>, Vec3, Vec3)> = None;
    let (tris, bmin, bmax) = cached_preprocess(triangles, config, prep_key, &mut prep_owned);
    if tris.is_empty() {
        return build_empty_svg(config);
    }
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);

    // Turntable mode
    if config.turntable.iterations >= 2 {
        let labels = turntable_labels(config.turntable.iterations);
        let mut views = Vec::with_capacity(config.turntable.iterations);
        for i in 0..config.turntable.iterations {
            let azimuth = 2.0 * std::f64::consts::PI * i as f64 / config.turntable.iterations as f64;
            views.push((turntable_view(bc, br, azimuth, config.turntable.elevation), labels[i].clone()));
        }
        return render_grid_svg(&tris, config, &views, br, bmin.z, group_styles);
    }

    // Grid mode
    if let Some(ref views) = config.views {
        if !views.is_empty() {
            let resolved: Vec<_> = views.iter().map(|n| (named_view(n, bc, br), capitalize(n))).collect();
            return render_grid_svg(&tris, config, &resolved, br, bmin.z, group_styles);
        }
    }

    // Smooth normals (skip for cel shading which doesn't use per-vertex normals)
    let needs_smooth = config.smooth
        && config.mode != "wireframe"
        && config.shading != "cel"
        && config.shading != "flat";
    // Owned fallback for the no-cache path (e.g. PLY, whose point-cloud
    // reconstruction can be camera-dependent); cached for STL/OBJ.
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

    // Single view
    let view = resolve_config_view(config, bc, br);
    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";

    let lights = resolve_lights(config);
    let shadow_data = build_shadow_data(&tris, &lights, smooth_data, config, group_styles, bc, br, false);
    let mut projected = project_triangles(&tris, smooth_data, config, &view, config.width, config.height, br, false, group_styles, &lights, shadow_data.as_ref());
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, config.width, config.height));
    }
    radix_sort_by_depth(&mut projected, false);

    let shadow_tris = if let Some(shadow) = &config.shadow {
        let mut s = project_shadow(&tris, config, shadow_light_dir(config), &view, config.width, config.height, br, bmin.z, false, &shadow.color);
        radix_sort_by_depth(&mut s, false);
        s
    } else {
        Vec::new()
    };

    // Outline edges
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
        &view, &tris, bmin, bmax, group_styles,
    )
}

// ---------------------------------------------------------------------------
// Single-view SVG
// ---------------------------------------------------------------------------

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
    group_styles: &HashMap<u32, GroupAppearance>,
) -> String {
    let estimated = tris.len() * 200 + shadow_tris.len() * 120 + outline_edges.len() * 80 + 512;
    let mut svg = String::with_capacity(estimated);
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 ");
    push_f2(&mut svg, w); svg.push(' '); push_f2(&mut svg, h);
    svg.push_str("\">");

    let hatch = config.clip.as_ref().and_then(|c| c.hatch.as_ref());
    if let Some(hc) = hatch { push_hatch_defs(&mut svg, hc); }

    // Background rect
    if !config.background.is_empty() && config.background != "none" {
        svg.push_str("<rect width=\""); push_f2(&mut svg, w);
        svg.push_str("\" height=\""); push_f2(&mut svg, h);
        svg.push_str("\" fill=\""); svg.push_str(&config.background);
        svg.push_str("\"/>");
    }

    // Shadow pass
    if !shadow_tris.is_empty() {
        svg.push_str("<g opacity=\""); push_f2(&mut svg, unsafe { config.shadow.as_ref().unwrap_unchecked() }.opacity); svg.push_str("\">");
        for tri in shadow_tris {
            write_shadow_polygon(&mut svg, tri);
        }
        svg.push_str("</g>");
    }

    // Model pass
    if is_wireframe {
        let wire_color = resolve_wireframe_color(config, false);
        let wire_width = config.wireframe.width;
        for tri in tris {
            write_wireframe_polygon(&mut svg, tri, wire_color, wire_width);
        }
    } else {
        let global_stroke = if config.stroke.color != "none" && config.stroke.width > 0.0 {
            Some((config.stroke.color.as_str(), config.stroke.width))
        } else { None };
        for tri in tris {
            write_solid_polygon(&mut svg, tri, global_stroke, group_styles, hatch.is_some());
        }
    }

    // Wireframe overlay (solid+wireframe mode)
    if is_solid_wireframe {
        let wire_color = resolve_wireframe_color(config, true);
        let wire_width = config.wireframe.width;
        for tri in tris {
            write_wireframe_polygon(&mut svg, tri, wire_color, wire_width);
        }
    }

    // Silhouette outlines
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

    // Annotations
    if let Some(ref ann_cfg) = config.annotations {
        let centroids = compute_group_centroids(tris);
        let anns = annotations::compute_annotations(
            &centroids, group_styles, ann_cfg, (w / 2.0, h / 2.0), w, h,
        );
        annotations::write_annotations_svg(&mut svg, &anns, ann_cfg);
    }

    // Debug overlay
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
    let lights = resolve_lights(config);
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

        let r = linear_to_srgb(light.color.0.min(1.0f32));
        let g = linear_to_srgb(light.color.1.min(1.0f32));
        let b = linear_to_srgb(light.color.2.min(1.0f32));

        // Area (disk) lights render as a flat disk of the light color, sized to
        // the light's physical radius and facing the model center — distinct from
        // the point-light marker octahedron.
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
            dv.push(pos); // center
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
                out.push(ProjectedTri {
                    pts: [proj_pts[0], proj_pts[i1], proj_pts[i2]],
                    depths: [cam_depths[0], cam_depths[i1], cam_depths[i2]],
                    depth,
                    r, g, b,
                    vertex_colors: None,
                    group_id: Some(DEBUG_DISK_GID),
                    opacity: 0.85,
                    pp: None,
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

        // Transform to camera space and project
        let cam: Vec<Vec3> = verts.iter().map(|v| view_mat.transform_point(*v)).collect();
        let proj_pts: Vec<(f64, f64)> = (0..6).map(|i| {
            let c = [cam[i], cam[i], cam[i]];
            apply_projection(&proj_setup, &c)[0]
        }).collect();
        let cam_depths: Vec<f64> = cam.iter().map(|c| c.z).collect();

        for &(a, bi, c) in &faces {
            let depth = (cam_depths[a] + cam_depths[bi] + cam_depths[c]) / 3.0;
            out.push(ProjectedTri {
                pts: [proj_pts[a], proj_pts[bi], proj_pts[c]],
                depths: [cam_depths[a], cam_depths[bi], cam_depths[c]],
                depth,
                r, g, b,
                vertex_colors: None,
                group_id: Some(u32::MAX),
                opacity: 0.85,
                pp: None,
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
    let lights = resolve_lights(config);
    let projector = make_point_projector(config, view, w, h, br);

    for light in &lights {
        if light.kind != LightKind::Directional { continue; }
        let pos = bc + light.vector.scale(br * 2.0);
        let line_end = bc + light.vector.scale(br * 1.5);
        let r = linear_to_srgb(light.color.0.min(1.0f32));
        let g = linear_to_srgb(light.color.1.min(1.0f32));
        let b = linear_to_srgb(light.color.2.min(1.0f32));
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

    // Emit one key-value debug row
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
    // Helper for Vec3 rows
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
    group_styles: &HashMap<u32, GroupAppearance>,
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

    // Labels
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

    // Grid lines
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

// ---------------------------------------------------------------------------
// Grid (multi-view) rendering
// ---------------------------------------------------------------------------

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
    group_styles: &HashMap<u32, GroupAppearance>,
) -> String {
    let (cols, rows) = grid_layout(views.len());
    let cell_w = config.width / cols as f64;
    let cell_h = config.height / rows as f64;
    let label_h = if config.grid_labels { 24.0 } else { 0.0 };
    let is_wireframe = config.mode == "wireframe";

    let lights = resolve_lights(config);
    // Shadow maps are camera-independent → build once, reuse for every view.
    let (gbmin, gbmax) = compute_bbox(triangles);
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
                let mut shadow = project_shadow(triangles, config, shadow_light_dir(config), view, cell_w, render_h, br, ground_z, true, &shadow_cfg.color);
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
                write_wireframe_polygon(&mut svg, tri, wire_color, wire_width);
            }
        } else {
            let global_stroke = if config.stroke.color != "none" && config.stroke.width > 0.0 {
                Some((config.stroke.color.as_str(), config.stroke.width))
            } else { None };
            for tri in &projected {
                write_solid_polygon(&mut svg, tri, global_stroke, group_styles, hatch.is_some());
            }
        }

        svg.push_str("</g>");
    }

    // Grid lines
    if views.len() > 1 {
        write_grid_lines(&mut svg, cols, rows, cell_w, cell_h, config.width, config.height);
    }

    svg.push_str("</svg>");
    svg
}

// ---------------------------------------------------------------------------
// PNG rendering
// ---------------------------------------------------------------------------

/// The raw RGBA producer shared by both plain and overlay outputs: downsamples
/// (opaque SSAA) or composites z-buffer coverage (transparent) into straight
/// RGBA8 at output resolution. Returns `(width, height, rgba)`.
fn raster_rgba(buf: &PixelBuffer, aa: usize, transparent: bool) -> (u32, u32, Vec<u8>) {
    if transparent {
        buf.to_rgba8_transparent(aa)
    } else if aa > 1 {
        buf.downsample(aa).to_rgba8()
    } else {
        buf.to_rgba8()
    }
}

/// Plain raster blob: `[0x00][width u32 LE][height u32 LE][rgba8…]`. The leading
/// 0x00 distinguishes it from SVG output ('<' = 0x3C) and a raster+overlay blob
/// (0x02); the host slices the 9-byte header and embeds the pixels via
/// `image(px, format: (encoding: "rgba8", width, height))`.
fn finish_raster(buf: &PixelBuffer, aa: usize, transparent: bool) -> Result<Vec<u8>, String> {
    let (w, h, rgba) = raster_rgba(buf, aa, transparent);
    let mut out = Vec::with_capacity(9 + rgba.len());
    out.push(0x00);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&rgba);
    Ok(out)
}

/// Raster + vector overlay blob: `[0x02][w u32 LE][h u32 LE][rgba8 w*h*4][svg…]`.
/// The raster is drawn as raw pixels and the transparent SVG (labels, grid
/// lines, annotations, debug text) is layered on top by the host — no image
/// encoding, so the plugin needs no PNG. The overlay's coordinates share the
/// raster's pixel space (`viewBox = 0 0 w h`).
fn pack_raster_overlay(buf: &PixelBuffer, aa: usize, transparent: bool, overlay: &str) -> Vec<u8> {
    let (w, h, rgba) = raster_rgba(buf, aa, transparent);
    let mut out = Vec::with_capacity(9 + rgba.len() + overlay.len());
    out.push(0x02);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&rgba);
    out.extend_from_slice(overlay.as_bytes());
    out
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

/// Rasterize to a bitmap. Plain images are a raw RGBA blob (see
/// [`finish_raster`]); the annotation/debug/labelled-grid variants return a
/// raster+overlay blob (see [`pack_raster_overlay`]) so the host layers vector
/// text over raw pixels — the plugin never encodes an image format.
pub fn render_raster(triangles: &[Triangle], config: &RenderConfig, group_styles: &HashMap<u32, GroupAppearance>, data_key: Option<u64>, prep_key: Option<u64>) -> Result<Vec<u8>, String> {
    let aa = config.antialias.max(1).next_power_of_two();
    let w = config.width as usize * aa;
    let h = config.height as usize * aa;
    let vw = config.width * aa as f64;
    let vh = config.height * aa as f64;
    // Transparent output: emit RGBA, using the z-buffer as model coverage. The
    // buffer is still filled with white so the opaque rasterization is unchanged;
    // the white only shows through where the model doesn't cover, and there it is
    // made transparent at encode time (and excluded from edge colour averaging).
    let transparent = config.background.is_empty() || config.background == "none";
    let bg = if transparent {
        (255, 255, 255)
    } else {
        parse_hex_color(&config.background)
    };

    if triangles.is_empty() {
        let buf = PixelBuffer::new(config.width as usize, config.height as usize, bg);
        return finish_raster(&buf, 1, transparent);
    }

    // Preprocessing pipeline (cached when the mesh geometry/colors are unchanged)
    let mut prep_owned: Option<(Vec<Triangle>, Vec3, Vec3)> = None;
    let (tris, bmin, bmax) = cached_preprocess(triangles, config, prep_key, &mut prep_owned);
    if tris.is_empty() {
        let buf = PixelBuffer::new(config.width as usize, config.height as usize, bg);
        return finish_raster(&buf, 1, transparent);
    }
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);

    // Turntable mode
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
            Ok(pack_raster_overlay(&buf, aa, transparent, &overlay))
        } else {
            finish_raster(&buf, aa, transparent)
        };
    }

    // Grid mode
    if let Some(ref views) = config.views {
        if !views.is_empty() {
            let resolved: Vec<_> = views.iter().map(|n| (named_view(n, bc, br), capitalize(n))).collect();
            let buf = render_grid_png_buf(&tris, config, &resolved, br, bmin.z, w, h, bg, group_styles);
            return if config.grid_labels {
                let overlay = overlay_grid_labels(config.width, config.height, &resolved);
                Ok(pack_raster_overlay(&buf, aa, transparent, &overlay))
            } else {
                finish_raster(&buf, aa, transparent)
            };
        }
    }

    // Smooth normals (skip for cel shading which doesn't use per-vertex normals)
    let needs_smooth = config.smooth
        && config.mode != "wireframe"
        && config.shading != "cel"
        && config.shading != "flat";
    // Owned fallback for the no-cache path (PLY); cached for STL/OBJ.
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

    // Single view
    let view = resolve_config_view(config, bc, br);
    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";

    let lights = resolve_lights(config);
    let shadow_data = build_shadow_data(&tris, &lights, smooth_data, config, group_styles, bc, br, true);
    let mut projected = project_triangles(&tris, smooth_data, config, &view, vw, vh, br, false, group_styles, &lights, shadow_data.as_ref());
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, vw, vh));
    }
    // Front-to-back sort: closer triangles fill z-buffer first, so farther
    // triangles' pixels fail z-test early (skipping color interpolation + writes).
    // Also correct for transparent pass which iterates in reverse (back-to-front).
    radix_sort_by_depth(&mut projected, true);

    let mut buf = PixelBuffer::new(w, h, bg);

    // Shadow pass
    if let Some(shadow_cfg) = &config.shadow {
        if !is_wireframe {
            let shadow = project_shadow(&tris, config, shadow_light_dir(config), &view, vw, vh, br, bmin.z, false, &shadow_cfg.color);
            rasterize_shadow_to_buf(&mut buf, &shadow, shadow_cfg);
        }
    }

    // Model pass: opaque triangles first (z-buffer write + test), then transparent (blend only)
    if !is_wireframe {
        // Opaque pass (front-to-back: closer triangles fill z-buffer first,
        // so farther triangles' pixels fail z-test early, skipping color interpolation).
        // Hi-Z: skip entire triangles whose closest point is behind all overlapping tiles.
        for tri in &projected {
            if tri.opacity >= 1.0 {
                let max_d = tri.depths[0].max(tri.depths[1]).max(tri.depths[2]) as f32;
                if buf.hiz_can_skip(&tri.pts, max_d) { continue; }
                match (shadow_data.as_ref(), tri.pp) {
                    // Per-pixel shadows: sample the maps at each fragment's world pos.
                    (Some(sd), Some((wp, normal))) => {
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
                buf.hiz_update(&tri.pts);
            }
        }
        // Transparent pass (back-to-front via reverse iteration for correct alpha blending)
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

    // Section hatching over clip caps (PNG). The SVG path fills the cap with a
    // <pattern>; here we overlay anti-aliased section lines on the visible cap
    // fragments so cross-sections read the same in PNG output.
    if !is_wireframe {
        if let Some(hc) = config.clip.as_ref().and_then(|c| c.hatch.as_ref()) {
            let color = parse_hex_color(&hc.color);
            let ang = hc.angle.to_radians();
            let (cos_a, sin_a) = (ang.cos(), ang.sin());
            let spacing = (hc.spacing * aa as f64).max(0.5);
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

    // Per-triangle stroke (global config.stroke or per-group overrides) for PNG
    if !is_wireframe {
        let global_has_stroke = config.stroke.color != "none" && config.stroke.width > 0.0;
        if global_has_stroke && group_styles.is_empty() {
            // Fast path: uniform stroke, no per-group overrides
            let (sr, sg, sb) = parse_hex_color(&config.stroke.color);
            for tri in &projected {
                buf.draw_triangle_edges(&tri.pts, sr, sg, sb);
            }
        } else if global_has_stroke || !group_styles.is_empty() {
            // Slow path: per-group stroke overrides
            let global_color = if global_has_stroke { Some(parse_hex_color(&config.stroke.color)) } else { None };
            let default_stroke_width = config.stroke.width;
            for tri in &projected {
                let ga = tri.group_id.and_then(|gid| group_styles.get(&gid));
                if let Some(a) = ga {
                    let sw = a.stroke_width.unwrap_or(default_stroke_width);
                    if sw > 0.0 {
                        if let Some(s) = a.stroke.as_deref() {
                            if s != "none" {
                                let (sr, sg, sb) = parse_hex_color(s);
                                buf.draw_triangle_edges(&tri.pts, sr, sg, sb);
                                continue;
                            }
                        } else if let Some((sr, sg, sb)) = global_color {
                            buf.draw_triangle_edges(&tri.pts, sr, sg, sb);
                            continue;
                        }
                    }
                } else if let Some((sr, sg, sb)) = global_color {
                    buf.draw_triangle_edges(&tri.pts, sr, sg, sb);
                }
            }
        }
    }

    // Debug light octahedron edges (z-tested so they hide behind model)
    if config.debug {
        for tri in &projected {
            if tri.group_id == Some(u32::MAX) {
                buf.draw_triangle_edges_z(&tri.pts, &tri.depths, 0x33, 0x33, 0x33);
            }
        }
    }

    // Wireframe overlay for PNG
    if is_solid_wireframe || is_wireframe {
        let (wr, wg, wb) = parse_hex_color(resolve_wireframe_color(config, is_solid_wireframe));
        for tri in &projected {
            buf.draw_triangle_edges(&tri.pts, wr, wg, wb);
        }
    }

    // Screen-space outline detection for PNG
    if let Some(ref outline) = config.outline {
        if !is_wireframe {
            let (or, og, ob) = parse_hex_color(&outline.color);
            buf.apply_outline((or, og, ob), outline.width * aa as f64);
        }
    }

    // Apply SSAO (screen-space ambient occlusion) if enabled
    if let Some(ref ssao) = config.ssao {
        if !is_wireframe {
            let ssao_params = crate::ssao::SSAOParams {
                samples: ssao.samples,
                radius: ssao.radius,
                bias: ssao.bias,
                strength: ssao.strength,
            };
            buf.apply_ssao(&ssao_params);
        }
    }

    // Bloom post-process
    if let Some(ref bloom) = config.bloom {
        if !is_wireframe {
            buf.apply_bloom(bloom.threshold as f32, bloom.intensity as f32, bloom.radius);
        }
    }

    // Glow post-process
    if let Some(ref glow) = config.glow {
        if !is_wireframe {
            let gc = parse_hex_color(&glow.color);
            buf.apply_glow(gc, glow.intensity as f32, glow.radius);
        }
    }

    // Sharpen post-process
    if let Some(ref sharpen) = config.sharpen {
        buf.apply_sharpen(sharpen.strength as f32);
    }

    // FXAA post-process. `antialias` is the single AA control: 0 = none,
    // 1 = FXAA, 2/4 = SSAA (aa > 1, handled by the supersample downsample), so
    // FXAA runs only at antialias == 1. Skipped for transparent output: FXAA
    // blends edges in RGB only, which would fringe against the white fill while
    // alpha stays hard — use SSAA for smooth transparent edges instead.
    if config.antialias == 1 && !transparent {
        crate::fxaa::apply_fxaa(&mut buf.pixels, buf.width, buf.height);
    }

    if let Some(ref ann_cfg) = config.annotations {
        // Scale centroids from supersampled space to output space
        let scale = 1.0 / aa as f64;
        let centroids: FxHashMap<u32, (f64, f64)> = compute_group_centroids(&projected)
            .into_iter()
            .map(|(gid, (x, y))| (gid, (x * scale, y * scale)))
            .collect();
        let overlay = overlay_annotations(config.width, config.height, &centroids, group_styles, ann_cfg);
        Ok(pack_raster_overlay(&buf, aa, transparent, &overlay))
    } else if config.debug {
        let overlay = overlay_debug(config.width, config.height, &tris, bmin, bmax, &view, config);
        Ok(pack_raster_overlay(&buf, aa, transparent, &overlay))
    } else {
        finish_raster(&buf, aa, transparent)
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
    group_styles: &HashMap<u32, GroupAppearance>,
) -> PixelBuffer {
    let (cols, rows) = grid_layout(views.len());
    let cell_w = w / cols;
    let cell_h = h / rows;
    // Reserve space for labels (proportional to cell height, matching SVG's 24px at 500px)
    let label_h = if config.grid_labels { (cell_h as f64 * 0.048).round() as usize } else { 0 };
    let render_h = cell_h - label_h;
    let is_wireframe = config.mode == "wireframe";

    let lights = resolve_lights(config);
    // Shadow maps are camera-independent → build once, reuse for every view.
    let (gbmin, gbmax) = compute_bbox(triangles);
    // Grid uses the per-vertex/flat sampling path (its raster loop has no
    // per-pixel branch), so disable per-pixel here regardless of config.
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
                    triangles, config, shadow_light_dir(config), view, cell_w as f64, render_h as f64, br, ground_z, true, &shadow_cfg.color,
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
                let max_d = tri.depths[0].max(tri.depths[1]).max(tri.depths[2]) as f32;
                if buf.hiz_can_skip(&pts_off, max_d) { continue; }
                buf.rasterize_triangle_offset(&tri.pts, &tri.depths, tri.r, tri.g, tri.b, ox, oy);
                buf.hiz_update(&pts_off);
            }
            // Section hatching over clip caps (matches the SVG grid <pattern>).
            if let Some(hc) = config.clip.as_ref().and_then(|c| c.hatch.as_ref()) {
                let color = parse_hex_color(&hc.color);
                let ang = hc.angle.to_radians();
                let (cos_a, sin_a) = (ang.cos(), ang.sin());
                let aa = (w / (config.width as usize).max(1)).max(1) as f64;
                let spacing = (hc.spacing * aa).max(0.5);
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
    let mut vol6 = 0.0; // 6 × signed volume
    let mut cacc = Vec3::new(0.0, 0.0, 0.0); // Σ  sv · (a + b + c)
    for t in triangles {
        let (a, b, c) = (t.vertices[0], t.vertices[1], t.vertices[2]);
        area += 0.5 * b.sub(a).cross(c.sub(a)).length();
        let sv = a.dot(b.cross(c)); // 6 × signed volume of tetra (origin, a, b, c)
        vol6 += sv;
        cacc = cacc.add(a.add(b).add(c).scale(sv));
    }
    let volume = (vol6 / 6.0).abs();
    // Centre of mass = Σ(vol_i · tetra_centroid_i) / V, with tetra_centroid = (a+b+c)/4.
    let centroid = if vol6.abs() > 1e-9 {
        cacc.scale(1.0 / (4.0 * vol6))
    } else {
        fallback_centroid
    };
    (area, volume, centroid)
}

pub fn get_info(triangles: &[Triangle], config: &RenderConfig) -> String {
    // Apply decimation so the reported counts match what render() produces.
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

