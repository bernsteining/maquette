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

use crate::config::{GroundCfg, RenderConfig};
use maquette_core::math::{Mat4, Vec3};
use crate::pbr::{IblContext, MaterialShader, PbrContext, SplattedLight, ToneMap};
use maquette_core::rasterizer::{BlendMode, PixelBuffer};
use crate::scene::{AlphaMode, Material, Scene, Triangle, Vertex};
use maquette_core::ssao::SSAOParams;

pub fn render(scene: &Scene, config: &RenderConfig) -> Vec<u8> {
    let width = config.width.max(1);
    let height = config.height.max(1);
    let factor = config.antialias.clamp(1, 4);
    let (bg, transparent) = resolve_background(&config.background);

    // SSAA: render at supersize, then downsample to target. Focal length
    // scales naturally since `focal = height * 0.5 / tan(fov/2)` — bigger
    // buffer height means proportionally larger focal, keeping FOV identical.
    let mut buffer = PixelBuffer::new(width * factor, height * factor, bg);

    if !scene.triangles.is_empty() || !scene.lines.is_empty() || !scene.points.is_empty() {
        rasterize_scene(&mut buffer, scene, config);
    }

    // Composite WBOIT translucent accumulation over the opaque pixel buffer.
    // No-op when no AlphaMode::Blend triangles were drawn.
    buffer.composite_oit();

    // SSAO runs on the hi-res depth buffer (better sample distribution).
    if let Some(ssao) = &config.ssao {
        buffer.apply_ssao(&SSAOParams {
            samples: ssao.samples,
            radius: ssao.radius,
            bias: ssao.bias,
            strength: ssao.strength,
        });
    }

    // Downsample to target size (no-op when factor == 1).
    let mut buffer = buffer.downsample(factor);

    // FXAA on target-size buffer — SSAA handles the bulk of edge cleanup;
    // FXAA polishes the remaining sub-pixel-ratio edges.
    if config.fxaa {
        maquette_core::fxaa::apply_fxaa(&mut buffer.pixels, buffer.width, buffer.height);
    }

    let (w, h, rgba) = if transparent {
        buffer.to_rgba8_transparent()
    } else {
        buffer.to_rgba8()
    };
    encode_raw_rgba(w, h, rgba)
}

fn rasterize_scene(buffer: &mut PixelBuffer, scene: &Scene, config: &RenderConfig) {
    let (center, radius) = scene.bounds();

    // Resolve camera. glTF-authored wins (via `camera_name`/`camera_index`);
    // else the config's Cartesian/spherical perspective setup applies.
    let (camera_pos, view, projection, znear, zfar) = if let Some(sc) = pick_glb_camera(scene, config) {
        let view = Mat4::look_at(sc.position, sc.target, sc.up);
        let proj = match sc.fov_y_deg {
            Some(fov) => Projection::Perspective { fov_deg: fov },
            None => Projection::Orthographic {
                half_h: sc.ortho_half_height.max(1e-6),
                half_w: sc.ortho_half_width.max(1e-6),
            },
        };
        (sc.position, view, proj, sc.znear.unwrap_or(1e-4), sc.zfar)
    } else {
        let pos = camera_position(config, center, radius);
        let view = build_view_matrix(config, center, radius, pos);
        (pos, view, Projection::Perspective { fov_deg: config.fov }, 1e-4, None)
    };

    let width_f = buffer.width as f64;
    let height_f = buffer.height as f64;
    // Perspective focal: `height/2 / tan(fov/2)`. Unused for orthographic
    // (projection() handles both).
    let focal = match projection {
        Projection::Perspective { fov_deg } => (height_f * 0.5) / (fov_deg.to_radians() * 0.5).tan(),
        Projection::Orthographic { .. } => 0.0,
    };

    // The config's light_dir is a *from-surface-toward-light* vector, matching
    // shader convention (positive N·L when lit).
    // Splat lights: prefer glTF's KHR_lights_punctual list; fall back to the
    // config's single directional when the file declares none. We also keep
    // the un-splatted `PunctualLight` list for shadow-map construction.
    let raw_lights: Vec<crate::scene::PunctualLight> = if scene.lights.is_empty() {
        // Build a fallback directional in the world "away from light" sense:
        // `direction` is "toward light" per glTF, so config.light_dir (which
        // is FROM surface TO light) needs to be negated for the light's
        // outbound direction.
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

    // Optional ground plane — two triangles at model bottom. Included in
    // both the shadow-caster pass (so ground can occlude itself under
    // grazing angles) and the shading pass (so shadows land on it).
    let (ground_tris, ground_material) = build_ground(scene, config);

    // Shadow maps: one per light when enabled. Built from all scene triangles
    // (the model) plus optional ground triangles.
    let (shadows, shadow_bias, shadow_softness, shadow_pcss_light_size) = if let Some(sh_cfg) = config.shadows {
        let (bc, br) = scene.bounds();
        let up = Vec3::from(config.up);
        // Grow the shadow bbox radius to cover the ground so the
        // directional-light ortho frustum reaches ground extents. Ground
        // itself is a RECEIVER only, not a caster — including it in the
        // shadow map produces subtle self-shadowing bands at triangle
        // boundaries where PCF taps read slightly different depths.
        let effective_br = if let Some(g) = &config.ground {
            br * g.size_scale as f64
        } else { br };
        // Project the glTF `Triangle` → 3-tuple of world positions that the
        // shadow builder wants. Cheap; ground isn't included per the roadmap
        // note (avoids self-shadow banding on the grid).
        let caster_tris: Vec<[Vec3; 3]> = scene.triangles.iter().map(|t| {
            [t.vertices[0].position, t.vertices[1].position, t.vertices[2].position]
        }).collect();
        let maps = maquette_core::shadow::build_shadow_maps(
            &caster_tris, &raw_lights, bc, effective_br, up, sh_cfg.resolution,
        );
        let bias = maquette_core::shadow::BiasParams {
            bias: sh_cfg.bias, normal_bias: sh_cfg.normal_bias, slope_bias: sh_cfg.slope_bias,
        };
        (maps, bias, sh_cfg.softness, sh_cfg.pcss_light_size)
    } else {
        (Vec::new(), maquette_core::shadow::BiasParams { bias: 0.0, normal_bias: 0.0, slope_bias: 0.0 }, 0, 0.0)
    };

    let pbr = PbrContext {
        light_dir: Vec3::from(config.light_dir).normalized(),
        light_color: [1.0, 1.0, 1.0],
        ambient: {
            let a = config.ambient as f32;
            [a, a, a]
        },
        camera_pos,
        tone_map: match config.tone_mapping.as_str() {
            "reinhard" => ToneMap::Reinhard,
            "aces" => ToneMap::Aces,
            _ => ToneMap::None,
        },
        exposure: config.exposure as f32,
        ibl: config.ibl.as_ref().map(|c| IblContext { sky: c.sky, ground: c.ground, intensity: c.intensity }),
        // Bake the env map once per render if IBL is on. HDR bytes (if present)
        // win over procedural sky/ground — same slot, different colour source.
        // On HDR parse error, fall back to procedural silently rather than
        // failing the whole render.
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
        lights,
        shadows,
        shadow_bias,
        shadow_softness,
        shadow_pcss_light_size,
    };

    // Cull + project each triangle once. Opaque + mask go straight to the
    // rasterizer with Overwrite blend; blend triangles queue up so they're
    // rasterized AFTER every opaque/mask primitive has settled the z-buffer.
    // No back-to-front sort needed — the queue drains through WBOIT
    // (Weighted Blended OIT, McGuire & Bavoil 2013), which is inherently
    // order-independent: each translucent fragment contributes into an
    // accumulation buffer weighted by depth, then a single composite pass
    // over the opaque frame produces the final image.
    let mut blend_queue: Vec<PreparedTriangle> = Vec::new();

    // Iterate scene triangles then ground triangles. Ground material lives
    // outside `scene.materials` so we branch on a sentinel id.
    let all_tris = scene.triangles.iter().chain(ground_tris.iter());
    for tri in all_tris {
        let material: &Material = if tri.material_id == u32::MAX {
            ground_material.as_ref().expect("ground material required when ground_tris present")
        } else {
            &scene.materials[tri.material_id as usize]
        };

        if !material.double_sided && config.cull_backface {
            let face_normal = tri.vertices[0].normal;
            let to_camera = (camera_pos - tri.vertices[0].position).normalized();
            if face_normal.dot(to_camera) <= 0.0 { continue; }
        }

        let v0 = view.transform_point(tri.vertices[0].position);
        let v1 = view.transform_point(tri.vertices[1].position);
        let v2 = view.transform_point(tri.vertices[2].position);

        // Near / far clip. glTF's camera znear/zfar are in view-space distance
        // (positive), so translate to our view-space z (camera looks along -Z).
        let near = -znear;
        if v0.z >= near || v1.z >= near || v2.z >= near { continue; }
        if let Some(f) = zfar {
            let far = -f;
            if v0.z <= far && v1.z <= far && v2.z <= far { continue; }
        }

        let (pts, depths, zbuf_depths) = match projection {
            Projection::Perspective { .. } => (
                [
                    project(v0, focal, width_f, height_f),
                    project(v1, focal, width_f, height_f),
                    project(v2, focal, width_f, height_f),
                ],
                // Perspective-correct interp weight = -1/v.z (also serves as
                // the hyperbolic z-buffer key — larger = closer).
                [-1.0 / v0.z, -1.0 / v1.z, -1.0 / v2.z],
                [-1.0 / v0.z, -1.0 / v1.z, -1.0 / v2.z],
            ),
            Projection::Orthographic { half_w, half_h } => (
                [
                    project_ortho(v0, half_w, half_h, width_f, height_f),
                    project_ortho(v1, half_w, half_h, width_f, height_f),
                    project_ortho(v2, half_w, half_h, width_f, height_f),
                ],
                // Orthographic: plain barycentric interp (no perspective
                // correction — attrs scale linearly across screen space).
                [1.0, 1.0, 1.0],
                // Z-buffer key = linear view-space depth (-v.z; larger = closer
                // since v.z is negative in front of the camera).
                [-v0.z, -v1.z, -v2.z],
            ),
        };
        let prepared = PreparedTriangle {
            tri,
            pts,
            depths,
            zbuf_depths,
            view_center_z: (v0.z + v1.z + v2.z) / 3.0,
            textures: &scene.textures,
        };

        match material.alpha_mode {
            AlphaMode::Blend => blend_queue.push(prepared),
            AlphaMode::Opaque | AlphaMode::Mask => {
                shade_triangle(buffer, &pbr, material, &prepared, BlendMode::Overwrite);
            }
        }
    }

    // Translucent (AlphaMode::Blend) triangles go through Weighted Blended OIT
    // — accumulate `(rgb·a·w, a·w)` into an off-screen buffer and multiply
    // `(1−a)` into a revealage buffer, composited into the pixel buffer after
    // the loop by `composite_oit`. Order-independent, so no back-to-front sort
    // is needed and interpenetrating translucents render correctly.
    for prepared in &blend_queue {
        let material = &scene.materials[prepared.tri.material_id as usize];
        shade_triangle(buffer, &pbr, material, prepared, BlendMode::WBOIT);
    }

    // Points and lines. glTF primitives with mode POINTS / LINES / LINE_STRIP
    // / LINE_LOOP get rendered as unlit 1-pixel screen-space primitives with
    // depth-testing. No PBR shading — the spec doesn't define BRDFs for
    // point/line topology, so we just use the material's base color × vertex
    // colour × cheap N·L for lit lines (giving some depth cue). Points get a
    // flat base × vertex colour.
    for pt in &scene.points {
        let vp = view.transform_point(pt.p.position);
        // Near/far clip.
        if vp.z >= -znear { continue; }
        if let Some(f) = zfar { if vp.z <= -f { continue; } }
        let (sx, sy) = match projection {
            Projection::Perspective { .. } => project(vp, focal, width_f, height_f),
            Projection::Orthographic { half_w, half_h } => project_ortho(vp, half_w, half_h, width_f, height_f),
        };
        let x = sx as i32;
        let y = sy as i32;
        if x < 0 || y < 0 || x >= buffer.width as i32 || y >= buffer.height as i32 { continue; }
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
        buffer.draw_line((ax, ay), (bx, by), za as f32, zb as f32, rgba_a, rgba_b);
    }
}

/// Projection kind resolved from glTF camera or user config.
#[derive(Copy, Clone)]
enum Projection {
    Perspective { fov_deg: f64 },
    Orthographic { half_w: f64, half_h: f64 },
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

    // Per-triangle LOD: ratio of UV area (unit²) to screen area (pixel²).
    // Each texture then converts this to its own mip level as
    // `0.5 · log2(lod_scale · width · height)`. Big screen area / small UV
    // means "oversampled — use mip 0"; the reverse means "use a smaller mip".
    //
    // KHR_texture_transform correction: the raw UV area assumes UVs live in
    // roughly `[0, 1]`. Under KHR_mesh_quantization + gltfpack the vertex UV
    // may be a raw 12-bit integer (0..~4095) meant to be dequantized by the
    // material's `KHR_texture_transform.scale`. Without the correction below
    // the raw UV area is ~4095² × the true area, driving LOD into the
    // highest mip and blurring texture detail catastrophically. We fold the
    // base-color transform's scale in — assets almost always use one
    // transform across all textures, so this correction is applied uniformly.
    let xform_area_scale = {
        let s = material.xform_base.scale;
        (s[0] * s[1]).abs()
    };
    let lod_scale = compute_lod_scale(&prepared.pts, tri) as f32 * xform_area_scale;

    let shader = MaterialShader::new(pbr, material, &prepared.textures, mask_cutoff, lod_scale);
    buffer.rasterize_triangle_shaded(
        &prepared.pts,
        &prepared.depths,
        &prepared.zbuf_depths,
        &positions,
        &normals,
        &uvs,
        &uvs1,
        &uvs2,
        &colors,
        &tangents,
        blend,
        &shader,
    );
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
    // Pick two orthogonal axes in the ground plane by seeding from up.
    let arbitrary = if up.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let axis_a = up.cross(arbitrary).normalized();
    let axis_b = up.cross(axis_a).normalized();
    let half = br * g.size_scale as f64;

    // Ground origin: scene center projected onto the ground plane at
    // configured Y (or `bbox_min.y - small epsilon` when auto).
    let y_ground = match g.y {
        Some(y) => y as f64,
        None => {
            // Project bbox_min onto up axis. For up=+Y that's just bbox_min.y.
            // General: distance along -up from bc to touch the bbox lower edge.
            let corner = scene.bbox_min;
            corner.dot(up)
        }
    };
    // Place ground centered under the model in the plane perpendicular to `up`.
    let center_on_ground = bc.sub(up.scale(bc.dot(up) - y_ground));
    // Subdivide the ground into a grid so triangles crossing the near plane
    // only cull the affected cells, not the whole plane. `N` = 8 gives 128
    // triangles — cheap, and preserves most of the ground even when the
    // camera is close.
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

    // Ground material: matte dielectric — takes ambient IBL + cast shadows.
    let mut mat = Material::default_gltf();
    mat.base_color = [g.color[0], g.color[1], g.color[2], 1.0];
    mat.metallic = 0.0;
    mat.roughness = g.roughness;
    mat.double_sided = true; // Grazing views shouldn't cull ground.
    // Reflect the mutations above in the cached precomputes — dielectric F0,
    // volume attenuation etc. depend on fields we may have touched.
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
    // Auto-pick the first authored camera when the caller didn't ask for a
    // specific one and hasn't overridden the framing (position, azimuth,
    // distance, fov). Matches viewer convention — an asset that ships a
    // camera almost always expects you to use it. Callers that want the
    // orbit fallback pass `camera: [x, y, z]` or a spherical override; the
    // check on `camera_auto_use` (default true) is the opt-out.
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

/// Wire format for the Typst wrapper:
///   `[0x00][w u32 LE][h u32 LE][rgba8...]`
fn encode_raw_rgba(w: u32, h: u32, mut rgba: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + rgba.len());
    out.push(0x00);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.append(&mut rgba);
    out
}

