use crate::annotations;
use crate::clip;
use crate::color_map;
use crate::config::{GroupAppearance, LightKind, RenderConfig, ShadowConfig};
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
            let arbitrary = if normal.x.abs() <= normal.y.abs() && normal.x.abs() <= normal.z.abs() {
                Vec3::new(1.0, 0.0, 0.0)
            } else if normal.y.abs() <= normal.z.abs() {
                Vec3::new(0.0, 1.0, 0.0)
            } else {
                Vec3::new(0.0, 0.0, 1.0)
            };
            let t1 = normal.cross(arbitrary).normalized();
            let t2 = normal.cross(t1);

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
            let n = pb.sub(pa).cross(pc.sub(pa));
            let len = n.length();
            if len < 1e-12 { continue; }
            n.scale(1.0 / len)
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
) -> Vec<ProjectedTri> {
    let proj = if force_ortho { Projection::Ortho } else { resolve_projection(&config.projection) };
    let proj_setup = setup_projection(proj, config, view, vw, vh, br);
    let view_mat = Mat4::look_at(view.camera, view.center, view.up);
    let view_simd = ViewMatSimd::from_mat4(&view_mat);
    let (base_r, base_g, base_b) = parse_hex_color(&config.color);
    let is_wireframe = config.mode == "wireframe";
    let is_xray = config.mode == "x-ray";
    let skip_cull = matches!(proj, Projection::Cabinet | Projection::Cavalier | Projection::TinyPlanet);
    let do_cull = config.cull_backface && !is_wireframe && !is_xray && !skip_cull && config.explode.abs() < 1e-12;
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
    let mut spec_lut = [0.0f32; 256];
    let mut spec_lut_exp = f32::NAN;  // track which exponent the LUT is built for

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
    let lights_f32: Vec<LightF32> = lights.iter().map(|l| LightF32 {
        kind: l.kind,
        dx: l.vector.x as f32, dy: l.vector.y as f32, dz: l.vector.z as f32,
        cr: l.color.0, cg: l.color.1, cb: l.color.2,
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
            if cfg_specular > 0.0 && cfg_shininess != spec_lut_exp {
                for i in 0..256 { spec_lut[i] = (i as f32 / 255.0).powf(cfg_shininess); }
                spec_lut_exp = cfg_shininess;
            }
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
                let mut snx: Vec<f32> = Vec::with_capacity(n_unique);
                let mut sny: Vec<f32> = Vec::with_capacity(n_unique);
                let mut snz: Vec<f32> = Vec::with_capacity(n_unique);
                let mut spx: Vec<f32> = Vec::with_capacity(n_unique);
                let mut spy: Vec<f32> = Vec::with_capacity(n_unique);
                let mut spz: Vec<f32> = Vec::with_capacity(n_unique);
                for i in 0..n_unique {
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
                let n_batches = n_unique / 4;
                let mut cache: Vec<(u8, u8, u8)> = Vec::with_capacity(n_unique);
                for i in 0..n_batches {
                    let b = i * 4;
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
                        &spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        simd_cel_bands,
                        simd_gooch, gooch_warm, gooch_cool,
                    );
                    cache.extend_from_slice(&colors);
                }
                for i in (n_batches * 4)..n_unique {
                    let amb = hemi_ambient(sd.normals[i], amb_sky, amb_gnd, up_f32);
                    cache.push(shade_point(
                        sd.normals[i], sd.positions[i], (blr, blg, blb),
                        &lights_f32, view_camera, amb, one_minus_ambient, cfg_specular,
                        cfg_fresnel, cfg_gamma,
                        tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                        &spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                    ));
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
                        &spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                    )
                }).collect()
            };
            Some(cache)
        } else { None }
    } else { None };

    let mut projected: Vec<ProjectedTri> = Vec::with_capacity(triangles.len());

    for (ti, tri) in triangles.iter().enumerate() {
        // World-space backface check — before vertex transforms to skip culled triangles early.
        // Sign of normal·(camera - centroid) determines facing; skip /3 by scaling camera×3.
        let is_back_facing = if do_cull || is_xray {
            let sx = tri.vertices[0].x + tri.vertices[1].x + tri.vertices[2].x;
            let sy = tri.vertices[0].y + tri.vertices[1].y + tri.vertices[2].y;
            let sz = tri.vertices[0].z + tri.vertices[1].z + tri.vertices[2].z;
            let dx = view.camera.x * 3.0 - sx;
            let dy = view.camera.y * 3.0 - sy;
            let dz = view.camera.z * 3.0 - sz;
            tri.normal.x * dx + tri.normal.y * dy + tri.normal.z * dz <= 0.0
        } else {
            false
        };

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
            let r = ((vcols[0].0 as u16 + vcols[1].0 as u16 + vcols[2].0 as u16) / 3) as u8;
            let g = ((vcols[0].1 as u16 + vcols[1].1 as u16 + vcols[2].1 as u16) / 3) as u8;
            let b = ((vcols[0].2 as u16 + vcols[1].2 as u16 + vcols[2].2 as u16) / 3) as u8;
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

            // Rebuild specular LUT when exponent changes (handles per-group overrides)
            if specular > 0.0 && shininess != spec_lut_exp {
                for i in 0..256 { spec_lut[i] = (i as f32 / 255.0).powf(shininess); }
                spec_lut_exp = shininess;
            }

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
                    let colors = shade_batch_4(
                        nx4, ny4, nz4, px4, py4, pz4,
                        blr, blg, blb,
                        &lights_f32, view_camera.x as f32, view_camera.y as f32, view_camera.z as f32,
                        grp_sky.0, grp_sky.1, grp_sky.2,
                        grp_gnd.0, grp_gnd.1, grp_gnd.2,
                        up_f32.0, up_f32.1, up_f32.2,
                        one_minus_ambient, specular, cfg_fresnel,
                        cfg_gamma, tm, cfg_exposure,
                        &spec_lut, &fresnel_lut,
                        sss_intensity, sss_dist, &sss_lut,
                        if shading == ShadingMode::Cel { cfg_cel_bands } else { 0 },
                        is_gooch, gooch_warm, gooch_cool,
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
                        vcols[i] = shade_point(
                            vn[i], tri.vertices[i], base_lin,
                            &lights_f32, view_camera, amb, one_minus_ambient, specular,
                            cfg_fresnel, cfg_gamma,
                            tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                            &spec_lut, &fresnel_lut,
                            sss_intensity, sss_dist, &sss_lut,
                        );
                    }
                    vcols
                };

                let r = ((vcols[0].0 as u16 + vcols[1].0 as u16 + vcols[2].0 as u16) / 3) as u8;
                let g = ((vcols[0].1 as u16 + vcols[1].1 as u16 + vcols[2].1 as u16) / 3) as u8;
                let b = ((vcols[0].2 as u16 + vcols[1].2 as u16 + vcols[2].2 as u16) / 3) as u8;
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
                let (r, g, b) = shade_point(
                    tri.normal, centroid, base_lin,
                    &lights_f32, view_camera, amb, one_minus_ambient, specular,
                    cfg_fresnel, cfg_gamma,
                    tm, cfg_exposure, shading, gooch_warm, gooch_cool, cfg_cel_bands,
                    &spec_lut, &fresnel_lut,
                    sss_intensity, sss_dist, &sss_lut,
                );
                (r, g, b, None, opacity)
            }
        };

        let pts = apply_projection(&proj_setup, &cam);
        let depths = [cam[0].z, cam[1].z, cam[2].z];
        let depth = (depths[0] + depths[1] + depths[2]) / 3.0;
        projected.push(ProjectedTri { pts, depths, depth, r, g, b, vertex_colors, group_id: tri.group_id, opacity });
    }

    projected
}

#[inline]
fn sort_by_depth(projected: &mut [ProjectedTri]) {
    projected.sort_unstable_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal));
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
        projected.push(ProjectedTri { pts, depths, depth, r: sr, g: sg, b: sb, vertex_colors: None, group_id: None, opacity: 1.0 });
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

fn write_solid_polygon(svg: &mut String, tri: &ProjectedTri, global_stroke: Option<(&str, f64)>, group_styles: &HashMap<u32, GroupAppearance>) {
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

fn preprocess(triangles: &[Triangle], config: &RenderConfig) -> (Vec<Triangle>, Vec3, Vec3) {
    let mut tris = triangles.to_vec();
    let (mut bmin, mut bmax) = compute_bbox(&tris);

    // 1. Color mapping
    if !config.color_map.is_empty() {
        match config.color_map.as_str() {
            "overhang" => {
                let up = Vec3::new(config.up[0], config.up[1], config.up[2]);
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
    if let Some(plane) = config.clip_plane {
        tris = clip::clip_triangles(&tris, plane, config.cull_backface);
    }

    // 3. Explode
    if config.explode.abs() > 1e-12 {
        let bc = bbox_center(bmin, bmax);
        explode::explode_triangles(&mut tris, bc, config.explode);
    }

    // 4. Recompute bbox after clipping/exploding
    if config.clip_plane.is_some() || config.explode.abs() > 1e-12 {
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

pub fn render(triangles: &[Triangle], config: &RenderConfig, group_styles: &HashMap<u32, GroupAppearance>) -> String {
    if triangles.is_empty() {
        return build_empty_svg(config);
    }

    // Preprocessing pipeline
    let (tris, bmin, bmax) = preprocess(triangles, config);
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
        && config.shading != "cel"
        && config.shading != "flat";
    let smooth_data = if needs_smooth {
        Some(smooth::compute_vertex_normals(&tris))
    } else {
        None
    };

    // Single view
    let view = resolve_config_view(config, bc, br);
    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";

    let lights = resolve_lights(config);
    let mut projected = project_triangles(&tris, smooth_data.as_ref(), config, &view, config.width, config.height, br, false, group_styles, &lights);
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, config.width, config.height));
    }
    sort_by_depth(&mut projected);

    let shadow_tris = if let Some(shadow) = &config.shadow {
        let mut s = project_shadow(&tris, config, shadow_light_dir(config), &view, config.width, config.height, br, bmin.z, false, &shadow.color);
        sort_by_depth(&mut s);
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
            write_solid_polygon(&mut svg, tri, global_stroke, group_styles);
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
    let mut set = HashSet::with_capacity(triangles.len());
    for tri in triangles {
        for v in &tri.vertices {
            set.insert(quantize(*v));
        }
    }
    set.len()
}

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
            LightKind::Positional => light.vector,
        };

        let r = linear_to_srgb(light.color.0.min(1.0f32));
        let g = linear_to_srgb(light.color.1.min(1.0f32));
        let b = linear_to_srgb(light.color.2.min(1.0f32));

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

    let mut effects_str = String::new();
    if config.outline.is_some() { effects_str.push_str("outline"); }
    if config.shadow.is_some() { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str("shadow"); }
    if config.clip_plane.is_some() { if !effects_str.is_empty() { effects_str.push_str(", "); } effects_str.push_str("clip"); }
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

/// Build SVG opening + image element for PNG-in-SVG wrappers.
fn svg_xlink_image_open(w: f64, h: f64, b64: &str) -> String {
    let mut svg = String::with_capacity(b64.len() + 256);
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 ");
    push_f2(&mut svg, w); svg.push(' '); push_f2(&mut svg, h);
    svg.push_str("\"><image width=\""); push_f2(&mut svg, w);
    svg.push_str("\" height=\""); push_f2(&mut svg, h);
    svg.push_str("\" href=\"data:image/png;base64,"); svg.push_str(b64);
    svg.push_str("\"/>");
    svg
}

/// Write grid lines (shared by SVG grid and PNG grid label wrapper).
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

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Wrap PNG bytes in an SVG with annotation overlays.
fn wrap_png_with_annotations(
    png_bytes: &[u8],
    w: f64,
    h: f64,
    centroids: &FxHashMap<u32, (f64, f64)>,
    group_styles: &HashMap<u32, GroupAppearance>,
    ann_cfg: &crate::config::AnnotationConfig,
) -> Vec<u8> {
    let b64 = base64_encode(png_bytes);
    let mut svg = svg_xlink_image_open(w, h, &b64);
    let anns = annotations::compute_annotations(
        centroids, group_styles, ann_cfg, (w / 2.0, h / 2.0), w, h,
    );
    annotations::write_annotations_svg(&mut svg, &anns, ann_cfg);
    svg.push_str("</svg>");
    svg.into_bytes()
}

/// Wrap PNG bytes in an SVG with a debug text overlay.
fn wrap_png_with_debug(
    png_bytes: &[u8],
    w: f64,
    h: f64,
    triangles: &[Triangle],
    bmin: Vec3,
    bmax: Vec3,
    view: &ViewParams,
    config: &RenderConfig,
) -> Vec<u8> {
    let b64 = base64_encode(png_bytes);
    let mut svg = svg_xlink_image_open(w, h, &b64);
    render_debug_light_lines(&mut svg, config, view, bmin, bmax, w, h);
    render_debug_overlay(&mut svg, w, h, triangles, bmin, bmax, view, config, "PNG");
    svg.push_str("</svg>");
    svg.into_bytes()
}

/// Wrap a grid PNG in SVG with text labels and grid lines (same technique as debug overlay).
fn wrap_png_with_grid_labels(
    png_bytes: &[u8],
    w: f64,
    h: f64,
    views: &[(ViewParams, String)],
) -> Vec<u8> {
    let b64 = base64_encode(png_bytes);
    let mut svg = svg_xlink_image_open(w, h, &b64);

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
    svg.into_bytes()
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
    let estimated = triangles.len() * 200 * views.len() + 512;
    let mut svg = String::with_capacity(estimated);
    svg_open(&mut svg, config.width, config.height, &config.background);

    for (i, (view, label)) in views.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = col as f64 * cell_w;
        let y = row as f64 * cell_h;
        let render_h = cell_h - label_h;

        let mut projected = project_triangles(triangles, None, config, view, cell_w, render_h, br, true, group_styles, &lights);
        sort_by_depth(&mut projected);

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
                sort_by_depth(&mut shadow);
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
                write_solid_polygon(&mut svg, tri, global_stroke, group_styles);
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

pub fn render_png(triangles: &[Triangle], config: &RenderConfig, group_styles: &HashMap<u32, GroupAppearance>) -> Result<Vec<u8>, String> {
    let aa = config.antialias.max(1).next_power_of_two();
    let w = config.width as usize * aa;
    let h = config.height as usize * aa;
    let vw = config.width * aa as f64;
    let vh = config.height * aa as f64;
    let bg = if config.background.is_empty() || config.background == "none" {
        (255, 255, 255)  // White background for transparent effect (PNG has no alpha)
    } else {
        parse_hex_color(&config.background)
    };

    if triangles.is_empty() {
        return PixelBuffer::new(config.width as usize, config.height as usize, bg).encode_png();
    }

    // Preprocessing pipeline
    let (tris, bmin, bmax) = preprocess(triangles, config);
    if tris.is_empty() {
        return PixelBuffer::new(config.width as usize, config.height as usize, bg).encode_png();
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
        let png = if aa > 1 { buf.downsample(aa) } else { buf }.encode_png()?;
        return if config.grid_labels {
            Ok(wrap_png_with_grid_labels(&png, config.width, config.height, &views))
        } else {
            Ok(png)
        };
    }

    // Grid mode
    if let Some(ref views) = config.views {
        if !views.is_empty() {
            let resolved: Vec<_> = views.iter().map(|n| (named_view(n, bc, br), capitalize(n))).collect();
            let buf = render_grid_png_buf(&tris, config, &resolved, br, bmin.z, w, h, bg, group_styles);
            let png = if aa > 1 { buf.downsample(aa) } else { buf }.encode_png()?;
            return if config.grid_labels {
                Ok(wrap_png_with_grid_labels(&png, config.width, config.height, &resolved))
            } else {
                Ok(png)
            };
        }
    }

    // Smooth normals (skip for cel shading which doesn't use per-vertex normals)
    let needs_smooth = config.smooth
        && config.shading != "cel"
        && config.shading != "flat";
    let smooth_data = if needs_smooth {
        Some(smooth::compute_vertex_normals(&tris))
    } else {
        None
    };

    // Single view
    let view = resolve_config_view(config, bc, br);
    let is_wireframe = config.mode == "wireframe";
    let is_solid_wireframe = config.mode == "solid+wireframe";

    let lights = resolve_lights(config);
    let mut projected = project_triangles(&tris, smooth_data.as_ref(), config, &view, vw, vh, br, false, group_styles, &lights);
    if config.debug {
        projected.append(&mut make_debug_light_tris(config, &view, bmin, bmax, vw, vh));
    }
    // Front-to-back sort: closer triangles fill z-buffer first, so farther
    // triangles' pixels fail z-test early (skipping color interpolation + writes).
    // Also correct for transparent pass which iterates in reverse (back-to-front).
    projected.sort_unstable_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));

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
                if let Some(vcols) = &tri.vertex_colors {
                    buf.rasterize_triangle_smooth(&tri.pts, &tri.depths, vcols);
                } else {
                    buf.rasterize_triangle(&tri.pts, &tri.depths, tri.r, tri.g, tri.b);
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

    // FXAA post-process (when not using SSAA; antialias:0 disables all AA)
    if config.fxaa && aa <= 1 && config.antialias != 0 {
        crate::fxaa::apply_fxaa(&mut buf.pixels, buf.width, buf.height);
    }

    let final_buf = if aa > 1 { buf.downsample(aa) } else { buf };
    let png_bytes = final_buf.encode_png()?;

    if let Some(ref ann_cfg) = config.annotations {
        // Scale centroids from supersampled space to output space
        let scale = 1.0 / aa as f64;
        let centroids: FxHashMap<u32, (f64, f64)> = compute_group_centroids(&projected)
            .into_iter()
            .map(|(gid, (x, y))| (gid, (x * scale, y * scale)))
            .collect();
        Ok(wrap_png_with_annotations(
            &png_bytes, config.width, config.height, &centroids, group_styles, ann_cfg,
        ))
    } else if config.debug {
        Ok(wrap_png_with_debug(
            &png_bytes,
            config.width,
            config.height,
            triangles,
            bmin,
            bmax,
            &view,
            config,
        ))
    } else {
        Ok(png_bytes)
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
    let mut buf = PixelBuffer::new(w, h, bg);

    for (i, (view, _label)) in views.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let ox = (col * cell_w) as f64;
        let oy = (row * cell_h + label_h) as f64;

        let mut projected = project_triangles(
            triangles, None, config, view, cell_w as f64, render_h as f64, br, true, group_styles, &lights,
        );
        projected.sort_unstable_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));

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
        }
    }

    buf
}

/// Return JSON with model info for verbose/debug purposes.
pub fn get_info(triangles: &[Triangle], config: &RenderConfig) -> String {
    let (bmin, bmax) = if triangles.is_empty() {
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0))
    } else {
        compute_bbox(triangles)
    };
    let bc = bbox_center(bmin, bmax);
    let br = bbox_radius(bmin, bmax);
    let view = resolve_config_view(config, bc, br);

    let mut s = String::with_capacity(256);
    s.push_str("{\"triangles\":"); push_usize(&mut s, triangles.len());
    s.push_str(",\"bbox_min\":["); push_f4(&mut s, bmin.x); s.push(','); push_f4(&mut s, bmin.y); s.push(','); push_f4(&mut s, bmin.z);
    s.push_str("],\"bbox_max\":["); push_f4(&mut s, bmax.x); s.push(','); push_f4(&mut s, bmax.y); s.push(','); push_f4(&mut s, bmax.z);
    s.push_str("],\"bbox_center\":["); push_f4(&mut s, bc.x); s.push(','); push_f4(&mut s, bc.y); s.push(','); push_f4(&mut s, bc.z);
    s.push_str("],\"bbox_radius\":"); push_f4(&mut s, br);
    s.push_str(",\"camera\":["); push_f4(&mut s, view.camera.x); s.push(','); push_f4(&mut s, view.camera.y); s.push(','); push_f4(&mut s, view.camera.z);
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

