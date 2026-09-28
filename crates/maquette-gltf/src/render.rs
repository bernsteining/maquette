/// Top-level render pipeline: scene → pixels.
///
/// Pipeline:
///   1. Resolve camera (spherical: azimuth/elevation/distance, auto-fit to bbox).
///   2. Build view matrix.
///   3. For each triangle:
///      - Look up material.
///      - Backface cull (unless material.double_sided).
///      - Transform view + project.
///      - Rasterize via `rasterize_triangle_shaded` with a PBR closure.
///   4. Read back the pixel buffer as raw RGBA prefixed with a marker byte.
///
/// The Typst wrapper feeds the byte stream straight into `image(...,
/// encoding: "rgba8")`, so there's no PNG encode/decode round-trip.
///
/// Deferred: SSAO / FXAA / tone mapping (phase 3), texture sampling (phase 2),
/// animations (phase 6).

use maquette_core::math::FloatExt;
use crate::config::RenderConfig;
use maquette_core::math::{Mat4, Vec3};
use crate::pbr::{IblContext, MaterialShader, PbrContext, SplattedLight, ToneMap};
use maquette_core::rasterizer::{BlendMode, PixelBuffer};
use crate::scene::{AlphaMode, Material, Scene, Triangle, Vertex};
use maquette_core::effects::{raw_raster, PostEffects};
use maquette_core::ssao::{DepthCamera, SSAOParams};

pub fn render(scene: &Scene, scene_key: u64, config: &RenderConfig) -> Vec<u8> {
    let width = config.width.max(1);
    let height = config.height.max(1);
    let factor = config.antialias.clamp(1, 4);
    let (bg, transparent) = resolve_background(&config.background);

    let (y0, y1) = match config.band {
        Some((a, b)) => (a.min(height), b.clamp(a.min(height), height)),
        None => (0, height),
    };
    let pad = if config.band.is_some() && config.fxaa { BAND_PAD } else { 0 };
    let (top, bottom) = (y0.saturating_sub(pad), (y1 + pad).min(height));
    let mut buffer = PixelBuffer::new(width * factor, (bottom - top).max(1) * factor, bg);
    crate::prof::mark(10);

    let camera = if !scene.triangles.is_empty() || !scene.lines.is_empty() || !scene.points.is_empty() {
        let frame = Frame { width: (width * factor) as f64, height: (height * factor) as f64, y_off: (top * factor) as f64 };
        Some(rasterize_scene(&mut buffer, scene, scene_key, config, frame).downscaled(factor))
    } else {
        None
    };

    crate::prof::mark(14);
    buffer.composite_oit();
    crate::prof::mark(15);

    let ssao = config.ssao.as_ref().map(|s| {
        let scene_space = if s.scene_space { camera.map(|c| (c, scene.bounds().1)) } else { None };
        SSAOParams::new(s.samples, s.radius, s.bias, s.strength, scene_space)
    });
    let (mut buffer, _) = buffer.resolve(factor, false, transparent || ssao.is_some());
    crate::prof::mark(16);
    if let Some(cam) = ssao.as_ref().and_then(|p| p.camera) {
        depth_to_view_z(&mut buffer.zbuf, &cam);
    }
    let post = Post { width, height, y0, y1, top, band: config.band.is_some(), fxaa: config.fxaa, transparent };
    match ssao {
        Some(params) if post.band => {
            let rows = &buffer.zbuf[(y0 - top) * width..(y1 - top) * width];
            let mut depth = Vec::with_capacity(rows.len() * 4);
            for d in rows { depth.extend_from_slice(&d.to_le_bytes()); }
            PENDING_BANDS.with(|p| {
                let mut p = p.borrow_mut();
                p.retain(|b| b.post.y0 != y0);
                if p.len() >= MAX_PENDING_BANDS { p.remove(0); }
                p.push(PendingBand { buffer, params, post });
            });
            depth
        }
        ssao => {
            crate::prof::mark(17);
            post.finish(buffer, ssao)
        }
    }
}

/// Complete a banded render deferred by SSAO: `render` with both `band` and
/// `ssao` returns the band's output-resolution depth rows (little-endian
/// `f32`) and keeps its pixels; once the caller has gathered every band's rows
/// into `full_depth` (the whole frame, top to bottom), this applies SSAO
/// against it and returns the band's raw RGBA, identical to those rows of a
/// whole-frame render.
pub fn finish_band(y0: usize, full_depth: &[u8]) -> Result<Vec<u8>, String> {
    let PendingBand { mut buffer, params, post } = PENDING_BANDS
        .with(|p| {
            let mut p = p.borrow_mut();
            let i = p.iter().position(|b| b.post.y0 == y0)?;
            Some(p.remove(i))
        })
        .ok_or_else(|| format!("no pending band at row {y0}"))?;
    if full_depth.len() != post.width * post.height * 4 {
        return Err(format!("full depth is {} bytes, expected {}", full_depth.len(), post.width * post.height * 4));
    }
    let depth: Vec<f32> = full_depth.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    buffer.apply_ssao_band(&params, &depth, post.width, post.height, post.top);
    Ok(post.finish(buffer, None))
}

const MAX_PENDING_BANDS: usize = 8;

struct PendingBand {
    buffer: PixelBuffer,
    params: SSAOParams,
    post: Post,
}

thread_local! {
    static PENDING_BANDS: std::cell::RefCell<Vec<PendingBand>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy)]
struct Post {
    width: usize,
    height: usize,
    y0: usize,
    y1: usize,
    top: usize,
    band: bool,
    fxaa: bool,
    transparent: bool,
}

impl Post {
    fn finish(self, mut buffer: PixelBuffer, ssao: Option<SSAOParams>) -> Vec<u8> {
        buffer.apply_post(&PostEffects { ssao, fxaa: self.fxaa, ..Default::default() });
        crate::prof::mark(18);
        let (w, h, mut rgba) = if self.transparent {
            buffer.to_rgba8_transparent_by_depth()
        } else {
            buffer.to_rgba8()
        };
        let (w, h) = if self.band {
            let row = w as usize * 4;
            let keep = (self.y0 - self.top)..(self.y1 - self.top);
            rgba = rgba[keep.start * row..keep.end * row].to_vec();
            (w, (self.y1 - self.y0) as u32)
        } else {
            (w, h)
        };
        let out = raw_raster(0x00, w, h, &rgba, &[]);
        crate::prof::mark(19);
        out
    }
}

const BAND_PAD: usize = 24;
const STATIC_PASS_MAX_SAMPLES: usize = 8 << 20;

#[derive(Clone, Copy)]
struct Frame {
    width: f64,
    height: f64,
    y_off: f64,
}

fn shadow_key(scene_key: u64, lights: &[crate::scene::PunctualLight], up: Vec3, radius: f64, resolution: usize) -> u64 {
    use std::hash::Hasher;
    use crate::scene::LightKind;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u64(scene_key);
    for l in lights {
        h.write_u8(match l.kind { LightKind::Directional => 0, LightKind::Point => 1, LightKind::Spot => 2 });
        for v in [l.position.x, l.position.y, l.position.z, l.direction.x, l.direction.y, l.direction.z] { h.write_u64(v.to_bits()); }
        for v in [l.color[0], l.color[1], l.color[2], l.range, l.inner_cone_cos, l.outer_cone_cos] { h.write_u32(v.to_bits()); }
        h.write_u8(l.cast_shadow as u8);
    }
    for v in [up.x, up.y, up.z, radius] { h.write_u64(v.to_bits()); }
    h.write_u64(resolution as u64);
    h.finish()
}

fn rasterize_scene(buffer: &mut PixelBuffer, scene: &Scene, scene_key: u64, config: &RenderConfig, frame: Frame) -> DepthCamera {
    let (center, radius) = scene.bounds();

    let (camera_pos, view, projection, znear, zfar) = if let Some(sc) = pick_glb_camera(scene, config) {
        let view = Mat4::look_at(sc.position, sc.target, sc.up);
        let proj = match sc.fov_y_deg {
            Some(fov) => Projection::Perspective { fov_deg: fov },
            None => Projection::Orthographic {
                half_h: sc.ortho_half_height.fmax(1e-6),
                half_w: sc.ortho_half_width.fmax(1e-6),
            },
        };
        (sc.position, view, proj, sc.znear.unwrap_or(1e-4), sc.zfar)
    } else {
        let pos = camera_position(config, center, radius);
        let view = build_view_matrix(config, center, radius, pos);
        (pos, view, Projection::Perspective { fov_deg: config.fov }, 1e-4, None)
    };

    let (width_f, height_f, y_off) = (frame.width, frame.height, frame.y_off);
    let focal = match projection {
        Projection::Perspective { fov_deg } => (height_f * 0.5) / (fov_deg.to_radians() * 0.5).tan(),
        Projection::Orthographic { .. } => 0.0,
    };

    let raw_lights: Vec<crate::scene::PunctualLight> = if scene.lights.is_empty() {
        let d = Vec3::from(config.light_dir).normalized();
        vec![crate::scene::PunctualLight {
            kind: crate::scene::LightKind::Directional,
            position: Vec3::new(0.0, 0.0, 0.0),
            direction: d.scale(-1.0),
            color: [1.0, 1.0, 1.0],
            range: 0.0,
            inner_cone_cos: 0.0,
            outer_cone_cos: 0.0,
            cast_shadow: true,
        }]
    } else {
        scene.lights.clone()
    };
    let lights: Vec<SplattedLight> = if scene.lights.is_empty() {
        vec![SplattedLight::fallback_directional(
            Vec3::from(config.light_dir).normalized(),
            [1.0, 1.0, 1.0],
        )]
    } else {
        raw_lights.iter().map(SplattedLight::from_light).collect()
    };

    let (ground_tris, ground_material) = build_ground(scene, config);

    let moving_tris: usize = scene.dynamic.iter().map(|&(a, b)| b - a).sum();
    let split_static = !scene.dynamic.is_empty() && scene.static_key != 0 && moving_tris * 2 <= scene.triangles.len();
    let (shadows, base_shadows, base_shadow_key, shadow_bias, shadow_softness, shadow_pcss_light_size) = if let Some(sh_cfg) = config.shadows {
        let (bc, br) = scene.bounds();
        let up = Vec3::from(config.up);
        let effective_br = if let Some(g) = &config.ground {
            br * g.size_scale as f64
        } else { br };
        let caster = |t: &crate::scene::Triangle| [t.vertices[0].position, t.vertices[1].position, t.vertices[2].position];
        let bias = maquette_core::shadow::BiasParams {
            bias: sh_cfg.bias as f64, normal_bias: sh_cfg.normal_bias as f64, slope_bias: sh_cfg.slope_bias as f64,
        };
        let frame_key = shadow_key(scene_key, &raw_lights, up, effective_br, sh_cfg.resolution);
        if !split_static {
            let maps = crate::cache::shadows_for(frame_key, || {
                let caster_tris: Vec<[Vec3; 3]> = scene.triangles.iter().map(caster).collect();
                maquette_core::shadow::build_shadow_maps(&caster_tris, &raw_lights, bc, effective_br, up, sh_cfg.resolution)
            });
            (maps, None, 0, bias, sh_cfg.softness, sh_cfg.pcss_light_size)
        } else {
            let mut skey = scene.static_key;
            for v in [bc.x, bc.y, bc.z] { skey = (skey ^ v.to_bits()).wrapping_mul(0x100000001b3); }
            let base_key = shadow_key(skey, &raw_lights, up, effective_br, sh_cfg.resolution);
            let base = crate::cache::static_shadows_for(base_key, || {
                let mut fixed: Vec<[Vec3; 3]> = Vec::with_capacity(scene.triangles.len());
                let mut at = 0;
                for &(a, b) in &scene.dynamic {
                    fixed.extend(scene.triangles[at..a].iter().map(caster));
                    at = b;
                }
                fixed.extend(scene.triangles[at..].iter().map(caster));
                maquette_core::shadow::build_shadow_maps(&fixed, &raw_lights, bc, effective_br, up, sh_cfg.resolution)
            });
            let moving = || {
                let mut moving: Vec<[Vec3; 3]> = Vec::new();
                for &(a, b) in &scene.dynamic { moving.extend(scene.triangles[a..b].iter().map(caster)); }
                moving
            };
            let maps: &'static [Option<maquette_core::shadow::LightShadow>] = if base.iter().flatten().all(|l| l.single().is_some()) {
                patch_work_shadows(base, base_key, frame_key, &moving())
            } else {
                crate::cache::shadows_for(frame_key, || {
                    let moving = moving();
                    let mut maps = base.to_vec();
                    for m in maps.iter_mut().flatten() { m.add_casters(&moving); }
                    maps
                })
            };
            (maps, Some(base), base_key, bias, sh_cfg.softness, sh_cfg.pcss_light_size)
        }
    } else {
        (&[][..], None, 0, maquette_core::shadow::BiasParams { bias: 0.0, normal_bias: 0.0, slope_bias: 0.0 }, 0, 0.0)
    };

    let factor = config.antialias.clamp(1, 4);
    let make_pbr = |shadows: &'static [Option<maquette_core::shadow::LightShadow>]| PbrContext {
        lod_scale_mul: (factor * factor) as f32,
        light_dir: Vec3::from(config.light_dir).normalized(),
        light_color: [1.0, 1.0, 1.0],
        ambient: {
            let a = config.ambient as f32;
            [a, a, a]
        },
        camera_pos,
        tone_map: ToneMap::parse(&config.tone_mapping),
        exposure: config.exposure as f32,
        ibl: config.ibl.as_ref().map(|c| IblContext { sky: c.sky, ground: c.ground, intensity: c.intensity }),
        ibl_env: config.ibl.as_ref().and_then(|c| {
            if let Some(hdr) = c.hdr_bytes.as_ref() {
                match crate::cache::ibl_for_hdr(hdr, c.intensity, c.rotation) {
                    Ok(env) => Some(env),
                    Err(_) => Some(crate::cache::ibl_for(
                        c.sky, c.ground, c.intensity,
                        Vec3::from(config.light_dir).normalized(),
                    )),
                }
            } else {
                Some(crate::cache::ibl_for(
                    c.sky, c.ground, c.intensity,
                    Vec3::from(config.light_dir).normalized(),
                ))
            }
        }),
        world_up: Vec3::from(config.up),
        lights: lights.clone(),
        shadows,
        shadow_bias,
        shadow_softness,
        shadow_pcss_light_size,
    };
    let pbr = make_pbr(shadows);

    crate::prof::mark(11);
    let n_scene = scene.triangles.len();
    let total = n_scene + ground_tris.len();
    let tris = Tris { scene, ground: &ground_tris, ground_material: ground_material.as_deref() };
    let cam = Cam { pos: camera_pos, view, projection, znear, zfar, focal, width: width_f, height: height_f, y_off, cull: config.cull_backface };
    let mut out = DepthPassOut { deferred: Vec::new(), deferred_idx: Vec::new(), blend: Vec::new() };
    buffer.begin_deferred(total);

    let shade_reusable: Option<&'static [Option<maquette_core::shadow::LightShadow>]> = match base_shadows {
        None if config.shadows.is_none() => Some(&[][..]),
        Some(base) if base.iter().flatten().all(|l| l.single().is_some()) => Some(base),
        _ => None,
    };
    let small_enough = buffer.width * buffer.height <= STATIC_PASS_MAX_SAMPLES;
    let pass_key = (split_static && small_enough && !scene.materials_animated).then(|| {
        static_pass_key(scene.static_key, &cam, &ground_tris, config.shading_key, base_shadow_key, &raw_lights, shade_reusable.is_some())
    });
    let cacheable = pass_key.is_some_and(|k| crate::cache::static_pass(k).is_some() || crate::cache::static_pass_requested(k));
    let shade_static = if cacheable { shade_reusable } else { None };
    let reach = {
        let pcf = shadow_softness as i64;
        let pcss = if shadow_pcss_light_size > 0.0 { 24 } else { 0 };
        pcf.max(pcss) + shadow_bias.normal_bias.ceil() as i64 + 2
    };
    let mut static_count = 0usize;
    let mut texels: &[u32] = &[];
    let mut snapshot_blocks: Vec<(usize, u32)> = Vec::new();
    let mut reduced: &[(usize, u32)] = &[];
    if cacheable {
        let mut moving = vec![false; n_scene];
        for &(a, b) in &scene.dynamic { moving[a..b].iter_mut().for_each(|m| *m = true); }
        let key = pass_key.unwrap();
        let sp = match crate::cache::static_pass(key) {
            Some(sp) => {
                buffer.restore_depth_state(&sp.zbuf, &sp.vis);
                if let Some(px) = &sp.pixels { buffer.pixels.copy_from_slice(px); }
                for e in &sp.entries {
                    let tri = tris.tri(e.index);
                    let prepared = PreparedTriangle {
                        tri, pts: e.pts, depths: e.depths, zbuf_depths: e.zbuf_depths,
                        view_center_z: e.view_center_z, textures: &scene.textures,
                    };
                    if e.blend {
                        out.blend.push((e.index, prepared));
                    } else {
                        out.deferred.push((prepared, tris.material(tri)));
                        out.deferred_idx.push(e.index);
                    }
                }
                sp
            }
            None => {
                for i in 0..total {
                    if i < n_scene && moving[i] { continue; }
                    depth_pass_tri(i, &tris, &cam, &pbr, buffer, &mut out);
                }
                buffer.resolve_owners();
                let mut map = vec![u32::MAX; out.deferred.len()];
                let mut kept = 0u32;
                for (id, m) in map.iter_mut().enumerate() {
                    if buffer.owns_pixels(id) { *m = kept; kept += 1; }
                }
                buffer.remap_owners(&map);
                let mut id = 0;
                out.deferred.retain(|_| { id += 1; map[id - 1] != u32::MAX });
                let mut id = 0;
                out.deferred_idx.retain(|_| { id += 1; map[id - 1] != u32::MAX });
                let (pixels, rects) = if let Some(base) = shade_static {
                    let pbr_static = make_pbr(base);
                    let blocks = buffer.reduce_shading_rate(factor);
                    buffer.resolve_owners();
                    for (id, (prepared, material)) in out.deferred.iter().enumerate() {
                        if buffer.owns_pixels(id) {
                            shade_triangle_deferred(buffer, &pbr_static, material, prepared, id);
                        }
                    }
                    buffer.fill_shading_rate(factor, &blocks);
                    snapshot_blocks = blocks;
                    let maps: Vec<Option<&maquette_core::shadow::ShadowMap>> = base.iter().map(|l| l.as_ref().and_then(|l| l.single())).collect();
                    let (zbuf, vis) = buffer.depth_state();
                    let (w, half_w, half_h) = (buffer.width, width_f * 0.5, height_f * 0.5);
                    let mut texels = vec![TEXEL_NONE; vis.len() * maps.len()];
                    for (i, &v) in vis.iter().enumerate() {
                        if v == 0 || maps.is_empty() { continue; }
                        let (sx, sy) = ((i % w) as f64 + 0.5, (i / w) as f64 + 0.5 + y_off);
                        let k = zbuf[i] as f64;
                        let vp = match projection {
                            Projection::Perspective { .. } => {
                                let z = -1.0 / k;
                                Vec3::new((sx - half_w) * -z / focal, (half_h - sy) * -z / focal, z)
                            }
                            Projection::Orthographic { half_w: ow, half_h: oh } => {
                                Vec3::new((sx - half_w) / half_w * ow, (half_h - sy) / half_h * oh, -k)
                            }
                        };
                        let world = rigid_inverse_apply(&view, vp);
                        for (m, map) in maps.iter().enumerate() {
                            texels[i * maps.len() + m] = match map {
                                None => TEXEL_NONE,
                                Some(map) => match map.texel_of(world) {
                                    Some((x, y)) if (0..TEXEL_LIMIT).contains(&x) && (0..TEXEL_LIMIT).contains(&y) => (y as u32) << 16 | x as u32,
                                    _ => TEXEL_ALWAYS,
                                },
                            };
                        }
                    }
                    let n_maps = maps.len();
                    for &(top, _) in &snapshot_blocks {
                        for dy in 0..factor {
                            for dx in 0..factor {
                                let i = top + dy * w + dx;
                                if i == top || n_maps == 0 { continue; }
                                let (src, dst) = (top * n_maps, i * n_maps);
                                texels.copy_within(src..src + n_maps, dst);
                            }
                        }
                    }
                    (Some(buffer.pixels.clone()), texels)
                } else {
                    (None, Vec::new())
                };
                let entry = |index: usize, p: &PreparedTriangle, blend: bool| crate::cache::StaticEntry {
                    index, pts: p.pts, depths: p.depths, zbuf_depths: p.zbuf_depths, view_center_z: p.view_center_z, blend,
                };
                let mut entries: Vec<crate::cache::StaticEntry> = out.deferred.iter().zip(&out.deferred_idx).map(|((p, _), &i)| entry(i, p, false)).collect();
                entries.extend(out.blend.iter().map(|(i, p)| entry(*i, p, true)));
                let (zbuf, vis) = buffer.depth_state();
                crate::cache::put_static_pass(key, crate::cache::StaticPass { zbuf, vis, pixels, texels: rects, blocks: std::mem::take(&mut snapshot_blocks), entries });
                crate::cache::static_pass(key).unwrap()
            }
        };
        if sp.pixels.is_some() {
            static_count = out.deferred.len();
            texels = &sp.texels;
            reduced = &sp.blocks;
        }
        for i in 0..n_scene {
            if moving[i] { depth_pass_tri(i, &tris, &cam, &pbr, buffer, &mut out); }
        }
        out.blend.sort_by_key(|(i, _)| *i);
    } else {
        for i in 0..total {
            depth_pass_tri(i, &tris, &cam, &pbr, buffer, &mut out);
        }
    }
    let DepthPassOut { deferred, blend: blend_queue, .. } = out;
    if static_count > 0 {
        let dirty = match crate::cache::work_shadows() {
            Some(w) if std::ptr::eq(w.maps.as_slice(), shadows) => DirtyTiles { maps: w.dirty.clone() },
            _ => DirtyTiles::between(shadows, shade_static.unwrap_or(&[])),
        }.dilated(reach + 1);
        let n_maps = shadows.len();
        let w = buffer.width;
        let mut broken = vec![false; if reduced.is_empty() { 0 } else { buffer.width * buffer.height }];
        {
            let owners = buffer.owners();
            for &(top, owner) in reduced {
                let intact = (0..factor).all(|dy| owners[top + dy * w..top + dy * w + factor].iter().all(|&v| v == owner));
                if intact { continue; }
                for dy in 0..factor {
                    broken[top + dy * w..top + dy * w + factor].iter_mut().for_each(|b| *b = true);
                }
            }
        }
        buffer.retain_owners(|i, owner| {
            owner >= static_count
                || broken.get(i).copied().unwrap_or(false)
                || texels[i * n_maps..(i + 1) * n_maps].iter().enumerate().any(|(m, &t)| dirty.hit(m, t))
        });
    }
    let blocks = buffer.reduce_shading_rate(factor);
    buffer.resolve_owners();
    for (id, (prepared, material)) in deferred.iter().enumerate() {
        if buffer.owns_pixels(id) {
            shade_triangle_deferred(buffer, &pbr, material, prepared, id);
        }
    }
    buffer.fill_shading_rate(factor, &blocks);
    buffer.end_deferred();

    crate::prof::mark(12);
    for (_, prepared) in &blend_queue {
        let material = &scene.materials[prepared.tri.material_id as usize];
        shade_triangle(buffer, &pbr, material, prepared, BlendMode::WBOIT);
    }

    crate::prof::mark(13);
    for pt in &scene.points {
        let vp = view.transform_point(pt.p.position);
        if vp.z >= -znear { continue; }
        if let Some(f) = zfar { if vp.z <= -f { continue; } }
        let (sx, sy) = match projection {
            Projection::Perspective { .. } => project(vp, focal, width_f, height_f),
            Projection::Orthographic { half_w, half_h } => project_ortho(vp, half_w, half_h, width_f, height_f),
        };
        let x = sx as i32;
        let y = sy as i32 - y_off as i32;
        if x < 0 || y < 0 || x >= buffer.width as i32 || y >= buffer.height as i32 || (sy as i32) < 0 { continue; }
        let material = if pt.material_id == u32::MAX {
            ground_material.as_ref().expect("ground material required")
        } else {
            &scene.materials[pt.material_id as usize]
        };
        let base = material.base_color;
        let c = pt.p.color;
        let rgba = [base[0] * c[0], base[1] * c[1], base[2] * c[2], base[3] * c[3]];
        let zbuf_key = match projection {
            Projection::Perspective { .. } => -1.0 / vp.z,
            Projection::Orthographic { .. } => -vp.z,
        };
        buffer.write_point((x as usize, y as usize), zbuf_key as f32, rgba);
    }

    for ln in &scene.lines {
        let va = view.transform_point(ln.a.position);
        let vb = view.transform_point(ln.b.position);
        if va.z >= -znear && vb.z >= -znear { continue; }
        if let Some(f) = zfar { if va.z <= -f && vb.z <= -f { continue; } }
        let (ax, ay) = match projection {
            Projection::Perspective { .. } => project(va, focal, width_f, height_f),
            Projection::Orthographic { half_w, half_h } => project_ortho(va, half_w, half_h, width_f, height_f),
        };
        let (bx, by) = match projection {
            Projection::Perspective { .. } => project(vb, focal, width_f, height_f),
            Projection::Orthographic { half_w, half_h } => project_ortho(vb, half_w, half_h, width_f, height_f),
        };
        let material = if ln.material_id == u32::MAX {
            ground_material.as_ref().expect("ground material required")
        } else {
            &scene.materials[ln.material_id as usize]
        };
        let base = material.base_color;
        let ca = ln.a.color;
        let cb = ln.b.color;
        let rgba_a = [base[0] * ca[0], base[1] * ca[1], base[2] * ca[2], base[3] * ca[3]];
        let rgba_b = [base[0] * cb[0], base[1] * cb[1], base[2] * cb[2], base[3] * cb[3]];
        let (za, zb) = match projection {
            Projection::Perspective { .. } => (-1.0 / va.z, -1.0 / vb.z),
            Projection::Orthographic { .. } => (-va.z, -vb.z),
        };
        buffer.draw_line_depth((ax, ay), (bx, by), za as f32, zb as f32, rgba_a, rgba_b, y_off as usize);
    }
    let (cx, cy) = ((width_f * 0.5) as f32, (height_f * 0.5) as f32);
    match projection {
        Projection::Perspective { .. } => DepthCamera::Perspective { fx: focal as f32, fy: focal as f32, cx, cy },
        Projection::Orthographic { half_w, half_h } => DepthCamera::Ortho { sx: cx / half_w as f32, sy: cy / half_h as f32, cx, cy },
    }
}

fn depth_to_view_z(zbuf: &mut [f32], camera: &DepthCamera) {
    for d in zbuf.iter_mut().filter(|d| **d != f32::NEG_INFINITY) {
        *d = match camera {
            DepthCamera::Perspective { .. } => -1.0 / *d,
            DepthCamera::Ortho { .. } => -*d,
        };
    }
}

/// Projection kind resolved from glTF camera or user config.
#[derive(Copy, Clone)]
enum Projection {
    Perspective { fov_deg: f64 },
    Orthographic { half_w: f64, half_h: f64 },
}

enum Pass {
    Immediate(BlendMode),
    MaskDepth,
    Deferred,
}

struct Tris<'a> {
    scene: &'a Scene,
    ground: &'a [Triangle],
    ground_material: Option<&'a Material>,
}

impl<'a> Tris<'a> {
    fn tri(&self, i: usize) -> &'a Triangle {
        let n = self.scene.triangles.len();
        if i < n { &self.scene.triangles[i] } else { &self.ground[i - n] }
    }

    fn material(&self, tri: &Triangle) -> &'a Material {
        if tri.material_id == u32::MAX {
            self.ground_material.expect("ground material required when ground_tris present")
        } else {
            &self.scene.materials[tri.material_id as usize]
        }
    }
}

struct Cam {
    pos: Vec3,
    view: Mat4,
    projection: Projection,
    znear: f64,
    zfar: Option<f64>,
    focal: f64,
    width: f64,
    height: f64,
    y_off: f64,
    cull: bool,
}

struct DepthPassOut<'a> {
    deferred: Vec<(PreparedTriangle<'a>, &'a Material)>,
    deferred_idx: Vec<usize>,
    blend: Vec<(usize, PreparedTriangle<'a>)>,
}

fn static_pass_key(static_key: u64, cam: &Cam, ground: &[Triangle], shading_key: u64, shadow_key: u64, lights: &[crate::scene::PunctualLight], shaded: bool) -> u64 {
    use std::hash::Hasher;
    let mut h = maquette_core::math::FxHasher::default();
    h.write_u64(static_key);
    h.write_u64(shading_key);
    h.write_u64(shadow_key);
    h.write_u8(shaded as u8);
    for l in lights {
        for v in [l.position.x, l.position.y, l.position.z, l.direction.x, l.direction.y, l.direction.z] { h.write_u64(v.to_bits()); }
        for v in [l.color[0], l.color[1], l.color[2], l.range, l.inner_cone_cos, l.outer_cone_cos] { h.write_u32(v.to_bits()); }
        h.write_u8(l.cast_shadow as u8);
    }
    for v in [cam.pos.x, cam.pos.y, cam.pos.z, cam.znear, cam.zfar.unwrap_or(-1.0), cam.focal, cam.width, cam.height, cam.y_off] {
        h.write_u64(v.to_bits());
    }
    for v in cam.view.0.iter().flatten() { h.write_u64(v.to_bits()); }
    match cam.projection {
        Projection::Perspective { fov_deg } => { h.write_u8(0); h.write_u64(fov_deg.to_bits()); }
        Projection::Orthographic { half_w, half_h } => { h.write_u8(1); h.write_u64(half_w.to_bits()); h.write_u64(half_h.to_bits()); }
    }
    h.write_u8(cam.cull as u8);
    h.write_u64(ground.len() as u64);
    for t in ground {
        for v in &t.vertices {
            for c in [v.position.x, v.position.y, v.position.z] { h.write_u64(c.to_bits()); }
        }
    }
    h.finish()
}

const DIRTY_TILE: i64 = 4;

#[derive(Default)]
struct DirtyTiles {
    maps: Vec<Option<(i64, Vec<bool>)>>,
}

impl DirtyTiles {
    fn between(full: &[Option<maquette_core::shadow::LightShadow>], base: &[Option<maquette_core::shadow::LightShadow>]) -> Self {
        let maps = full.iter().zip(base).map(|(f, b)| {
            let (f, b) = (f.as_ref()?.single()?, b.as_ref()?.single()?);
            let res = f.resolution();
            let tiles = (res as i64 + DIRTY_TILE - 1) / DIRTY_TILE;
            let mut dirty = vec![false; (tiles * tiles) as usize];
            let (ft, bt) = (f.texels(), b.texels());
            for y in 0..res {
                let (fr, br) = (&ft[y * res..(y + 1) * res], &bt[y * res..(y + 1) * res]);
                let trow = (y as i64 / DIRTY_TILE * tiles) as usize;
                for x in 0..res {
                    if fr[x].to_bits() != br[x].to_bits() { dirty[trow + x / DIRTY_TILE as usize] = true; }
                }
            }
            Some((tiles, dirty))
        }).collect();
        DirtyTiles { maps }
    }

    fn dilated(mut self, reach_texels: i64) -> Self {
        let r = (reach_texels + DIRTY_TILE - 1) / DIRTY_TILE;
        for m in self.maps.iter_mut().flatten() {
            let (tiles, dirty) = (m.0, &m.1);
            let t = tiles as usize;
            let mut rows = vec![false; dirty.len()];
            for y in 0..t {
                for x in 0..t {
                    if !dirty[y * t + x] { continue; }
                    let (x0, x1) = ((x as i64 - r).max(0) as usize, ((x as i64 + r) as usize).min(t - 1));
                    rows[y * t + x0..=y * t + x1].iter_mut().for_each(|d| *d = true);
                }
            }
            let mut out = vec![false; dirty.len()];
            for y in 0..t {
                for x in 0..t {
                    if !rows[y * t + x] { continue; }
                    let (y0, y1) = ((y as i64 - r).max(0) as usize, ((y as i64 + r) as usize).min(t - 1));
                    for yy in y0..=y1 { out[yy * t + x] = true; }
                }
            }
            m.1 = out;
        }
        self
    }

    #[inline]
    fn hit(&self, map: usize, texel: u32) -> bool {
        match texel {
            TEXEL_NONE => false,
            TEXEL_ALWAYS => true,
            t => match &self.maps[map] {
                None => false,
                Some((tiles, dirty)) => {
                    let (x, y) = ((t & 0xFFFF) as i64 / DIRTY_TILE, (t >> 16) as i64 / DIRTY_TILE);
                    x >= *tiles || y >= *tiles || dirty[(y * tiles + x) as usize]
                }
            },
        }
    }
}

fn patch_work_shadows(
    base: &'static [Option<maquette_core::shadow::LightShadow>],
    base_key: u64,
    frame_key: u64,
    moving: &[[Vec3; 3]],
) -> &'static [Option<maquette_core::shadow::LightShadow>] {
    let slot = crate::cache::work_shadows();
    if !matches!(slot, Some(w) if w.base_key == base_key) {
        *slot = Some(crate::cache::WorkShadows {
            base_key,
            frame_key: 0,
            maps: base.to_vec(),
            dirty: base.iter().map(|l| l.as_ref().and_then(|l| l.single()).map(|m| {
                let tiles = (m.resolution() as i64 + DIRTY_TILE - 1) / DIRTY_TILE;
                (tiles, vec![false; (tiles * tiles) as usize])
            })).collect(),
        });
    }
    let w = slot.as_mut().unwrap();
    if w.frame_key != frame_key {
        for ((work, b), d) in w.maps.iter_mut().zip(base).zip(w.dirty.iter_mut()) {
            let (Some(work), Some(b), Some((tiles, dirty))) = (work.as_mut().and_then(|l| l.single_mut()), b.as_ref().and_then(|l| l.single()), d.as_mut()) else { continue };
            let res = b.resolution();
            let t = *tiles as usize;
            let src = b.texels();
            let dst = work.texels_mut();
            for ty in 0..t {
                for tx in 0..t {
                    if !dirty[ty * t + tx] { continue; }
                    dirty[ty * t + tx] = false;
                    let (x0, x1) = (tx * DIRTY_TILE as usize, ((tx + 1) * DIRTY_TILE as usize).min(res));
                    for y in ty * DIRTY_TILE as usize..((ty + 1) * DIRTY_TILE as usize).min(res) {
                        dst[y * res + x0..y * res + x1].copy_from_slice(&src[y * res + x0..y * res + x1]);
                    }
                }
            }
            let mut r = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
            let mut everywhere = false;
            for tri in moving {
                match work.texel_rect(tri) {
                    Some(q) => r = [r[0].min(q[0]), r[1].min(q[1]), r[2].max(q[2]), r[3].max(q[3])],
                    None => everywhere = true,
                }
            }
            for tri in moving { work.splat_tri(*tri); }
            if everywhere { r = [0, 0, res as i64 - 1, res as i64 - 1]; }
            if r[0] > r[2] { continue; }
            let (x0, y0) = ((r[0] - 1).clamp(0, res as i64 - 1) as usize, (r[1] - 1).clamp(0, res as i64 - 1) as usize);
            let (x1, y1) = ((r[2] + 1).clamp(0, res as i64 - 1) as usize, (r[3] + 1).clamp(0, res as i64 - 1) as usize);
            let dst = work.texels();
            for y in y0..=y1 {
                let trow = y / DIRTY_TILE as usize * t;
                for x in x0..=x1 {
                    if dst[y * res + x].to_bits() != src[y * res + x].to_bits() { dirty[trow + x / DIRTY_TILE as usize] = true; }
                }
            }
        }
        w.frame_key = frame_key;
    }
    unsafe { std::mem::transmute::<&[Option<maquette_core::shadow::LightShadow>], &'static [Option<maquette_core::shadow::LightShadow>]>(w.maps.as_slice()) }
}

const TEXEL_NONE: u32 = u32::MAX;
const TEXEL_ALWAYS: u32 = u32::MAX - 1;
const TEXEL_LIMIT: i64 = 0xFFFF;

fn rigid_inverse_apply(view: &Mat4, v: Vec3) -> Vec3 {
    let m = view.0;
    let p = Vec3::new(v.x - m[0][3], v.y - m[1][3], v.z - m[2][3]);
    Vec3::new(
        m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z,
        m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z,
        m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z,
    )
}

fn depth_pass_tri<'a>(i: usize, tris: &Tris<'a>, cam: &Cam, pbr: &PbrContext, buffer: &mut PixelBuffer, out: &mut DepthPassOut<'a>) {
    let tri = tris.tri(i);
    let material = tris.material(tri);

    if !material.double_sided && cam.cull {
        let face_normal = tri.vertices[0].normal;
        let to_camera = (cam.pos - tri.vertices[0].position).normalized();
        if face_normal.dot(to_camera) <= 0.0 { return; }
    }

    let v0 = cam.view.transform_point(tri.vertices[0].position);
    let v1 = cam.view.transform_point(tri.vertices[1].position);
    let v2 = cam.view.transform_point(tri.vertices[2].position);

    let near = -cam.znear;
    if v0.z >= near || v1.z >= near || v2.z >= near { return; }
    if let Some(f) = cam.zfar {
        let far = -f;
        if v0.z <= far && v1.z <= far && v2.z <= far { return; }
    }

    let (focal, width_f, height_f) = (cam.focal, cam.width, cam.height);
    let (pts, depths, zbuf_depths) = match cam.projection {
        Projection::Perspective { .. } => (
            [
                project(v0, focal, width_f, height_f),
                project(v1, focal, width_f, height_f),
                project(v2, focal, width_f, height_f),
            ],
            [-1.0 / v0.z, -1.0 / v1.z, -1.0 / v2.z],
            [-1.0 / v0.z, -1.0 / v1.z, -1.0 / v2.z],
        ),
        Projection::Orthographic { half_w, half_h } => (
            [
                project_ortho(v0, half_w, half_h, width_f, height_f),
                project_ortho(v1, half_w, half_h, width_f, height_f),
                project_ortho(v2, half_w, half_h, width_f, height_f),
            ],
            [1.0, 1.0, 1.0],
            [-v0.z, -v1.z, -v2.z],
        ),
    };
    let pts = pts.map(|(x, y)| (x, y - cam.y_off));
    let prepared = PreparedTriangle {
        tri,
        pts,
        depths,
        zbuf_depths,
        view_center_z: (v0.z + v1.z + v2.z) / 3.0,
        textures: &tris.scene.textures,
    };

    let transmissive = material.transmission_factor > 0.0;
    match material.alpha_mode {
        AlphaMode::Blend => out.blend.push((i, prepared)),
        AlphaMode::Opaque | AlphaMode::Mask if transmissive => out.blend.push((i, prepared)),
        AlphaMode::Mask => {
            mask_depth(buffer, pbr, material, &prepared, out.deferred.len());
            out.deferred.push((prepared, material));
            out.deferred_idx.push(i);
        }
        AlphaMode::Opaque => {
            buffer.rasterize_depth_id(&prepared.pts, &prepared.zbuf_depths, out.deferred.len());
            out.deferred.push((prepared, material));
            out.deferred_idx.push(i);
        }
    }
}

struct PreparedTriangle<'a> {
    tri: &'a Triangle,
    pts: [(f64, f64); 3],
    /// Per-vertex 1/w for perspective-correct interp. Constant 1 in ortho.
    depths: [f64; 3],
    /// Per-vertex z-buffer key. Equal to `depths` in perspective; linear
    /// `-v.z` in ortho.
    zbuf_depths: [f64; 3],
    view_center_z: f64,
    textures: &'a [maquette_core::texture::Texture],
}

fn shade_triangle(
    buffer: &mut PixelBuffer,
    pbr: &PbrContext,
    material: &crate::scene::Material,
    prepared: &PreparedTriangle,
    blend: BlendMode,
) {
    shade_with(buffer, pbr, material, prepared, Pass::Immediate(blend), 0);
}

fn mask_depth(
    buffer: &mut PixelBuffer,
    pbr: &PbrContext,
    material: &crate::scene::Material,
    prepared: &PreparedTriangle,
    id: usize,
) {
    shade_with(buffer, pbr, material, prepared, Pass::MaskDepth, id);
}

fn shade_triangle_deferred(
    buffer: &mut PixelBuffer,
    pbr: &PbrContext,
    material: &crate::scene::Material,
    prepared: &PreparedTriangle,
    id: usize,
) {
    shade_with(buffer, pbr, material, prepared, Pass::Deferred, id);
}

fn shade_with(
    buffer: &mut PixelBuffer,
    pbr: &PbrContext,
    material: &crate::scene::Material,
    prepared: &PreparedTriangle,
    pass: Pass,
    deferred_id: usize,
) {
    let tri = prepared.tri;
    let positions = [
        tri.vertices[0].position, tri.vertices[1].position, tri.vertices[2].position,
    ];
    let normals = [
        tri.vertices[0].normal, tri.vertices[1].normal, tri.vertices[2].normal,
    ];
    let uvs = [
        tri.vertices[0].uv, tri.vertices[1].uv, tri.vertices[2].uv,
    ];
    let uvs1 = [
        tri.vertices[0].uv1, tri.vertices[1].uv1, tri.vertices[2].uv1,
    ];
    let uvs2 = [
        tri.vertices[0].uv2, tri.vertices[1].uv2, tri.vertices[2].uv2,
    ];
    let colors = [
        tri.vertices[0].color, tri.vertices[1].color, tri.vertices[2].color,
    ];
    let tangents = [
        tri.vertices[0].tangent, tri.vertices[1].tangent, tri.vertices[2].tangent,
    ];

    let mask_cutoff = if material.alpha_mode == AlphaMode::Mask {
        Some(material.alpha_cutoff)
    } else {
        None
    };

    let xform_area_scale = {
        let s = material.xform_base.scale;
        (s[0] * s[1]).abs()
    };
    let rate = if matches!(pass, Pass::Deferred) { pbr.lod_scale_mul } else { 1.0 };
    let lod_scale = compute_lod_scale(&prepared.pts, tri) as f32 * xform_area_scale * rate;

    let shader = MaterialShader::new(pbr, material, &prepared.textures, mask_cutoff, lod_scale);
    match pass {
        Pass::Immediate(blend) => buffer.rasterize_triangle_shaded(
            &prepared.pts, &prepared.depths, &prepared.zbuf_depths,
            &positions, &normals, &uvs, &uvs1, &uvs2, &colors, &tangents,
            blend, &shader,
        ),
        Pass::MaskDepth => buffer.rasterize_mask_depth_id(
            deferred_id, &prepared.pts, &prepared.depths, &prepared.zbuf_depths,
            &positions, &normals, &uvs, &uvs1, &uvs2, &colors, &tangents,
            &shader,
        ),
        Pass::Deferred => buffer.shade_deferred(
            deferred_id, &prepared.pts, &prepared.depths, &prepared.zbuf_depths,
            &positions, &normals, &uvs, &uvs1, &uvs2, &colors, &tangents,
            &shader,
        ),
    }
}

#[inline]
fn project(v: Vec3, focal: f64, width: f64, height: f64) -> (f64, f64) {
    let inv_z = -1.0 / v.z;
    (
        width  * 0.5 + v.x * focal * inv_z,
        height * 0.5 - v.y * focal * inv_z,
    )
}

/// Orthographic projection: linear scale from view-space (x, y) to screen
/// pixels using the glTF-authored half-extents `xmag`/`ymag`. No z-dependency
/// (parallel projection). Follows the same y-flip as `project`.
#[inline]
fn project_ortho(v: Vec3, half_w: f64, half_h: f64, width: f64, height: f64) -> (f64, f64) {
    (
        width  * 0.5 + (v.x / half_w) * (width  * 0.5),
        height * 0.5 - (v.y / half_h) * (height * 0.5),
    )
}

fn camera_position(config: &RenderConfig, center: Vec3, radius: f64) -> Vec3 {
    if let Some(cam) = config.camera {
        return Vec3::from(cam);
    }
    let dist = config.distance.filter(|&d| d > 0.0)
        .unwrap_or_else(|| default_distance(config.fov, radius));
    let up = Vec3::from(config.up);
    let az = config.azimuth.to_radians();
    let el = config.elevation.to_radians();

    let arbitrary = if up.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let right   = up.cross(arbitrary).normalized();
    let forward = right.cross(up).normalized();
    let offset  = right.scale(el.cos() * az.cos())
        .add(forward.scale(el.cos() * az.sin()))
        .add(up.scale(el.sin()));
    center.add(offset.scale(dist))
}

fn build_view_matrix(config: &RenderConfig, center: Vec3, _radius: f64, camera: Vec3) -> Mat4 {
    let look_center = if config.auto_center { center } else { Vec3::from(config.center) };
    Mat4::look_at(camera, look_center, Vec3::from(config.up))
}

fn default_distance(fov_deg: f64, radius: f64) -> f64 {
    let half_fov = (fov_deg * 0.5).to_radians();
    radius / half_fov.sin() * 1.15
}

/// Build the ground-plane triangles and material from config, sized to
/// the scene's bounding sphere. Returns `(triangles, material)` — both empty
/// / None when `config.ground` is None. Triangles use `material_id =
/// u32::MAX` as a sentinel so the shade loop can dispatch to the standalone
/// ground material.
fn build_ground(scene: &Scene, config: &RenderConfig) -> (Vec<Triangle>, Option<Box<Material>>) {
    let Some(g) = &config.ground else { return (Vec::new(), None); };
    if scene.triangles.is_empty() { return (Vec::new(), None); }
    let (bc, br) = scene.bounds();
    let up = Vec3::from(config.up).normalized();
    let arbitrary = if up.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let axis_a = up.cross(arbitrary).normalized();
    let axis_b = up.cross(axis_a).normalized();
    let half = br * g.size_scale as f64;

    let y_ground = match g.y {
        Some(y) => y as f64,
        None => {
            let corner = scene.bbox_min;
            corner.dot(up)
        }
    };
    let center_on_ground = bc.sub(up.scale(bc.dot(up) - y_ground));
    const N: usize = 8;
    let cell = (half * 2.0) / N as f64;
    let n = up;
    let t = [axis_a.x as f32, axis_a.y as f32, axis_a.z as f32, 1.0];
    let mk_vertex = |p: Vec3, u: f32, v: f32| Vertex {
        position: p, normal: n, uv: [u, v], uv1: [u, v], uv2: [u, v], color: [1.0, 1.0, 1.0, 1.0], tangent: t,
    };
    let mut tris = Vec::with_capacity(N * N * 2);
    let origin = center_on_ground.sub(axis_a.scale(half)).sub(axis_b.scale(half));
    for j in 0..N {
        for i in 0..N {
            let p00 = origin.add(axis_a.scale(cell * i as f64)).add(axis_b.scale(cell * j as f64));
            let p10 = origin.add(axis_a.scale(cell * (i + 1) as f64)).add(axis_b.scale(cell * j as f64));
            let p01 = origin.add(axis_a.scale(cell * i as f64)).add(axis_b.scale(cell * (j + 1) as f64));
            let p11 = origin.add(axis_a.scale(cell * (i + 1) as f64)).add(axis_b.scale(cell * (j + 1) as f64));
            let (u0, u1) = (i as f32 / N as f32, (i + 1) as f32 / N as f32);
            let (v0, v1) = (j as f32 / N as f32, (j + 1) as f32 / N as f32);
            tris.push(Triangle {
                vertices: [mk_vertex(p00, u0, v0), mk_vertex(p10, u1, v0), mk_vertex(p11, u1, v1)],
                material_id: u32::MAX,
            });
            tris.push(Triangle {
                vertices: [mk_vertex(p00, u0, v0), mk_vertex(p11, u1, v1), mk_vertex(p01, u0, v1)],
                material_id: u32::MAX,
            });
        }
    }

    let mut mat = Material::default_gltf();
    mat.base_color = [g.color[0], g.color[1], g.color[2], 1.0];
    mat.metallic = 0.0;
    mat.roughness = g.roughness;
    mat.double_sided = true;
    mat.recompute_precomp();
    (tris, Some(Box::new(mat)))
}

/// Resolve `config.camera_name` / `camera_index` to a `SceneCamera` from
/// the glTF, if present. Name lookup wins over index. Returns None when the
/// user didn't ask or the target isn't found.
fn pick_glb_camera<'a>(scene: &'a Scene, config: &RenderConfig) -> Option<&'a crate::scene::SceneCamera> {
    if let Some(name) = &config.camera_name {
        if let Some(c) = scene.cameras.iter().find(|c| c.name.as_deref() == Some(name.as_str())) {
            return Some(c);
        }
    }
    if let Some(idx) = config.camera_index {
        return scene.cameras.get(idx);
    }
    if config.camera_auto_use && scene.cameras.first().is_some() && !user_overrode_framing(config) {
        return scene.cameras.first();
    }
    None
}

/// Did the caller explicitly override camera framing? If so, `pick_glb_camera`
/// shouldn't silently override with a glTF-authored camera on top.
fn user_overrode_framing(cfg: &RenderConfig) -> bool {
    cfg.camera.is_some()
        || cfg.azimuth != 0.0
        || cfg.elevation != 0.0
        || cfg.distance.is_some()
}

/// Per-triangle LOD scale: `|Δuv × Δuv| / |Δp × Δp|` — the ratio of the
/// triangle's UV area (in unit-square²) to its screen area (in pixel²).
/// Zero when either area is degenerate (falls back to mip 0 via clamp).
fn compute_lod_scale(pts: &[(f64, f64); 3], tri: &Triangle) -> f64 {
    let (u0, u1, u2) = (tri.vertices[0].uv, tri.vertices[1].uv, tri.vertices[2].uv);
    let duv1 = [(u1[0] - u0[0]) as f64, (u1[1] - u0[1]) as f64];
    let duv2 = [(u2[0] - u0[0]) as f64, (u2[1] - u0[1]) as f64];
    let uv_area = (duv1[0] * duv2[1] - duv1[1] * duv2[0]).abs();
    let e1 = (pts[1].0 - pts[0].0, pts[1].1 - pts[0].1);
    let e2 = (pts[2].0 - pts[0].0, pts[2].1 - pts[0].1);
    let screen_area = (e1.0 * e2.1 - e1.1 * e2.0).abs();
    if screen_area < 1e-6 { 0.0 } else { uv_area / screen_area }
}

fn resolve_background(bg: &str) -> ((u8, u8, u8), bool) {
    if bg.is_empty() {
        return ((0, 0, 0), true);
    }
    (maquette_core::color::parse_hex_color(bg), false)
}


