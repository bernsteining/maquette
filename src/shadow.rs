// Shadow mapping — one depth map per light, rendered from the light's viewpoint.
// At shade time each vertex's world position is projected into every light's map
// and depth-compared to decide whether that light is occluded (in shadow).

use crate::config::ShadowMapConfig;
use crate::math::{Mat4, Vec3};
use crate::parser::Triangle;
use crate::shading::ResolvedLight;
use crate::config::LightKind;

const EMPTY: f32 = f32::MAX;

/// Depth-comparison bias parameters (see `ShadowMapConfig`).
pub struct BiasParams {
    pub bias: f64,
    pub normal_bias: f64,
    pub slope_bias: f64,
}

/// A single light's depth map plus the transform to project world → its texel grid.
pub struct ShadowMap {
    view: Mat4,          // world → light view space (look_at)
    ortho: bool,         // true: directional (orthographic), false: positional (perspective)
    half_extent: f64,    // ortho: half-size of the covered square, in world units
    tan_half_fov: f64,   // perspective: tan(fov/2)
    near: f64,           // depth-normalization range (distance along light forward)
    far: f64,
    res: usize,
    depth: Vec<f32>,     // res*res, min normalized depth per texel (EMPTY = nothing)
    forward: Vec3,       // unit direction the light travels (scene is in front)
    eye: Vec3,           // light position (positional) or camera behind the ortho box
}

impl ShadowMap {
    /// Project a world point into this map. Returns (texel_x, texel_y, normalized_depth),
    /// or None if the point is behind the light (outside a perspective frustum).
    #[inline]
    fn project(&self, p: Vec3) -> Option<(f64, f64, f64)> {
        let v = self.view.transform_point(p);
        let fwd = -v.z; // distance in front of the light (view looks along -z)
        if fwd <= 1e-6 {
            return None;
        }
        let (ndc_x, ndc_y) = if self.ortho {
            (v.x / self.half_extent, v.y / self.half_extent)
        } else {
            let inv = 1.0 / fwd;
            (v.x * inv / self.tan_half_fov, v.y * inv / self.tan_half_fov)
        };
        let sx = (ndc_x * 0.5 + 0.5) * self.res as f64;
        let sy = (ndc_y * 0.5 + 0.5) * self.res as f64;
        let depth = ((fwd - self.near) / (self.far - self.near)).clamp(0.0, 1.0);
        Some((sx, sy, depth))
    }

    /// World-space size of one shadow texel near point `p` (for normal-offset).
    #[inline]
    fn texel_world(&self, p: Vec3) -> f64 {
        if self.ortho {
            2.0 * self.half_extent / self.res as f64
        } else {
            let fwd = (p - self.eye).dot(self.forward).max(self.near);
            2.0 * self.tan_half_fov * fwd / self.res as f64
        }
    }

    /// Unit direction from `p` toward the light.
    #[inline]
    fn light_dir(&self, p: Vec3) -> Vec3 {
        if self.ortho {
            self.forward.scale(-1.0)
        } else {
            (self.eye - p).normalized()
        }
    }

    /// Lit fraction ∈ [0,1] for a world point with surface `normal`.
    /// Applies normal-offset + slope-scaled bias, then PCF-filters.
    pub fn lit(&self, p: Vec3, normal: Vec3, b: &BiasParams, softness: usize) -> f32 {
        // Normal-offset: push the sample toward the light along the surface normal.
        let sample = if b.normal_bias > 0.0 {
            p.add(normal.scale(b.normal_bias * self.texel_world(p)))
        } else {
            p
        };
        // Slope-scaled depth bias: grow bias at grazing light angles.
        let ndotl = normal.dot(self.light_dir(p)).abs().max(0.15);
        let tan_theta = ((1.0 - ndotl * ndotl).max(0.0)).sqrt() / ndotl;
        let bias = b.bias * (1.0 + b.slope_bias * tan_theta.min(6.0));

        let (sx, sy, depth) = match self.project(sample) {
            Some(v) => v,
            None => return 1.0, // behind the light → not shadowed by this map
        };
        let cx = sx.floor() as i64;
        let cy = sy.floor() as i64;
        let r = softness as i64;
        let mut lit = 0u32;
        let mut total = 0u32;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx + dx;
                let y = cy + dy;
                total += 1;
                if x < 0 || y < 0 || x >= self.res as i64 || y >= self.res as i64 {
                    lit += 1; // outside the map → treat as lit
                    continue;
                }
                let stored = self.depth[y as usize * self.res + x as usize];
                if stored == EMPTY || depth <= stored as f64 + bias {
                    lit += 1;
                }
            }
        }
        lit as f32 / total as f32
    }

    #[inline]
    fn stored(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x >= self.res as i64 || y >= self.res as i64 {
            EMPTY
        } else {
            self.depth[y as usize * self.res + x as usize]
        }
    }

    /// Contact-hardening soft shadow (PCSS). Searches for blockers to estimate a
    /// world-space penumbra, then filters with a proportional PCF kernel: sharp
    /// where the occluder is close, soft where it's far. `light_size` is the
    /// emitter's world-space size. Bounded sampling keeps the per-pixel cost sane.
    pub fn lit_pcss(&self, p: Vec3, normal: Vec3, b: &BiasParams, base_softness: usize, light_size: f64) -> f32 {
        let sample = if b.normal_bias > 0.0 {
            p.add(normal.scale(b.normal_bias * self.texel_world(p)))
        } else {
            p
        };
        let ndotl = normal.dot(self.light_dir(p)).abs().max(0.15);
        let tan_theta = ((1.0 - ndotl * ndotl).max(0.0)).sqrt() / ndotl;
        let bias = b.bias * (1.0 + b.slope_bias * tan_theta.min(6.0));
        let (sx, sy, depth) = match self.project(sample) {
            Some(v) => v,
            None => return 1.0,
        };
        let cx = sx.floor() as i64;
        let cy = sy.floor() as i64;
        let tw = self.texel_world(p).max(1e-9);
        let light_texels = (light_size / tw).clamp(1.0, 24.0);

        // 1) Blocker search: average depth of texels closer than the receiver.
        let search = light_texels.ceil() as i64;
        let sstep = (search / 4).max(1);
        let mut bsum = 0.0f64;
        let mut bn = 0u32;
        let mut dy = -search;
        while dy <= search {
            let mut dx = -search;
            while dx <= search {
                let s = self.stored(cx + dx, cy + dy);
                if s != EMPTY && (s as f64) < depth - bias {
                    bsum += s as f64;
                    bn += 1;
                }
                dx += sstep;
            }
            dy += sstep;
        }
        if bn == 0 {
            return 1.0; // no blockers → fully lit
        }
        let avg_blocker = bsum / bn as f64;

        // 2) Penumbra ∝ (receiver − blocker) / blocker × light size.
        let penumbra = ((depth - avg_blocker) / avg_blocker).max(0.0);
        let radius = ((penumbra * light_texels * 8.0).max(base_softness as f64)).clamp(1.0, 12.0) as i64;

        // 3) PCF over the penumbra-sized kernel (bounded taps).
        let pstep = (radius / 6).max(1);
        let mut lit = 0u32;
        let mut total = 0u32;
        let mut dy = -radius;
        while dy <= radius {
            let mut dx = -radius;
            while dx <= radius {
                let s = self.stored(cx + dx, cy + dy);
                if s == EMPTY || depth <= s as f64 + bias {
                    lit += 1;
                }
                total += 1;
                dx += pstep;
            }
            dy += pstep;
        }
        lit as f32 / total as f32
    }

    /// Rasterize occluder depth (no culling) into this map.
    fn render(&mut self, triangles: &[Triangle], is_occluder: &dyn Fn(&Triangle) -> bool) {
        let res = self.res;
        for tri in triangles {
            if !is_occluder(tri) {
                continue;
            }
            let mut sp = [(0.0f64, 0.0f64, 0.0f64); 3];
            let mut behind = false;
            for (i, v) in tri.vertices.iter().enumerate() {
                match self.project(*v) {
                    Some(p) => sp[i] = p,
                    None => {
                        behind = true;
                        break;
                    }
                }
            }
            if behind {
                continue; // triangle crosses the light plane; skip (rare)
            }
            rasterize_depth(&mut self.depth, res, &sp);
        }
    }
}

/// A light's shadow: either a single frustum (directional / external point) or
/// a 6-face cube map (omnidirectional point light among the geometry).
pub enum LightShadow {
    Single(ShadowMap),
    Cube(Box<[ShadowMap; 6]>),
}

impl LightShadow {
    #[inline]
    pub fn lit(&self, p: Vec3, normal: Vec3, b: &BiasParams, softness: usize) -> f32 {
        match self {
            LightShadow::Single(m) => m.lit(p, normal, b, softness),
            LightShadow::Cube(f) => f[cube_face(p, f[0].eye)].lit(p, normal, b, softness),
        }
    }
    #[inline]
    pub fn lit_pcss(&self, p: Vec3, normal: Vec3, b: &BiasParams, softness: usize, light_size: f64) -> f32 {
        match self {
            LightShadow::Single(m) => m.lit_pcss(p, normal, b, softness, light_size),
            LightShadow::Cube(f) => f[cube_face(p, f[0].eye)].lit_pcss(p, normal, b, softness, light_size),
        }
    }
    fn render(&mut self, triangles: &[Triangle], is_occluder: &dyn Fn(&Triangle) -> bool) {
        match self {
            LightShadow::Single(m) => m.render(triangles, is_occluder),
            LightShadow::Cube(f) => f.iter_mut().for_each(|m| m.render(triangles, is_occluder)),
        }
    }
}

/// Pick the cube face (0:+X 1:−X 2:+Y 3:−Y 4:+Z 5:−Z) for a point, by the
/// dominant axis of the direction from the light to it.
#[inline]
fn cube_face(p: Vec3, eye: Vec3) -> usize {
    let d = p.sub(eye);
    let (ax, ay, az) = (d.x.abs(), d.y.abs(), d.z.abs());
    if ax >= ay && ax >= az {
        if d.x > 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if d.y > 0.0 { 2 } else { 3 }
    } else if d.z > 0.0 { 4 } else { 5 }
}

fn cube_face_map(eye: Vec3, forward: Vec3, up: Vec3, near: f64, far: f64, res: usize) -> ShadowMap {
    ShadowMap {
        view: Mat4::look_at(eye, eye.add(forward), up),
        ortho: false,
        half_extent: 0.0,
        tan_half_fov: 1.0, // 90° fov per face
        near,
        far,
        res,
        depth: vec![EMPTY; res * res],
        forward,
        eye,
    }
}

fn build_cube(eye: Vec3, br: f64, res: usize) -> Box<[ShadowMap; 6]> {
    let near = (br * 0.02).max(1e-4);
    let far = br * 3.5;
    let z = Vec3::new(0.0, 0.0, 1.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    Box::new([
        cube_face_map(eye, Vec3::new(1.0, 0.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(-1.0, 0.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, 1.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, -1.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, 0.0, 1.0), y, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, 0.0, -1.0), y, near, far, res),
    ])
}

/// Build one shadow (single frustum or cube) per light — None for lights that
/// don't cast. `bc`/`br` frame each view; `is_occluder` filters caster tris.
pub fn build_shadow_maps(
    triangles: &[Triangle],
    lights: &[ResolvedLight],
    bc: Vec3,
    br: f64,
    up: Vec3,
    cfg: &ShadowMapConfig,
    is_occluder: &dyn Fn(&Triangle) -> bool,
) -> Vec<Option<LightShadow>> {
    let res = cfg.resolution.clamp(64, 4096);
    lights
        .iter()
        .map(|light| {
            if !light.cast_shadow {
                return None;
            }
            let mut ls = if cfg.omni && light.kind == LightKind::Positional {
                LightShadow::Cube(build_cube(light.vector, br, res))
            } else {
                LightShadow::Single(build_one(light, bc, br, up, res))
            };
            ls.render(triangles, is_occluder);
            Some(ls)
        })
        .collect()
}

fn build_one(light: &ResolvedLight, bc: Vec3, br: f64, up: Vec3, res: usize) -> ShadowMap {
    // Pick an up vector for the light view that isn't parallel to its forward axis.
    let forward = if light.kind == LightKind::Directional {
        // `vector` points toward the light; the light shines along -vector.
        light.vector.normalized().scale(-1.0)
    } else {
        (bc - light.vector).normalized()
    };
    let up_aux = if forward.cross(up).length() > 1e-3 {
        up
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };

    if light.kind == LightKind::Directional {
        let eye = bc - forward.scale(br * 2.0);
        let view = Mat4::look_at(eye, bc, up_aux);
        let eye_dist = br * 2.0;
        ShadowMap {
            view,
            ortho: true,
            half_extent: br * 1.05,
            tan_half_fov: 0.0,
            near: (eye_dist - br * 1.2).max(1e-4),
            far: eye_dist + br * 1.2,
            res,
            depth: vec![EMPTY; res * res],
            forward,
            eye,
        }
    } else {
        let eye = light.vector;
        let dist = (bc - eye).length().max(br * 0.1);
        let view = Mat4::look_at(eye, bc, up_aux);
        ShadowMap {
            view,
            ortho: false,
            half_extent: 0.0,
            tan_half_fov: (br * 1.1 / dist).clamp(0.05, 10.0),
            near: (dist - br * 1.2).max(dist * 0.01),
            far: dist + br * 1.2,
            res,
            depth: vec![EMPTY; res * res],
            forward,
            eye,
        }
    }
}

/// Fill a triangle into the depth buffer, keeping the nearest (min) depth per texel.
fn rasterize_depth(depth: &mut [f32], res: usize, p: &[(f64, f64, f64); 3]) {
    let (x0, y0, z0) = p[0];
    let (x1, y1, z1) = p[1];
    let (x2, y2, z2) = p[2];
    let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if area.abs() < 1e-12 {
        return;
    }
    let inv_area = 1.0 / area;
    let min_x = x0.min(x1).min(x2).floor().max(0.0) as usize;
    let max_x = (x0.max(x1).max(x2).ceil() as i64).clamp(0, res as i64) as usize;
    let min_y = y0.min(y1).min(y2).floor().max(0.0) as usize;
    let max_y = (y0.max(y1).max(y2).ceil() as i64).clamp(0, res as i64) as usize;
    for y in min_y..max_y {
        let py = y as f64 + 0.5;
        for x in min_x..max_x {
            let px = x as f64 + 0.5;
            // Barycentric weights via edge functions.
            let w0 = ((x1 - px) * (y2 - py) - (x2 - px) * (y1 - py)) * inv_area;
            let w1 = ((x2 - px) * (y0 - py) - (x0 - px) * (y2 - py)) * inv_area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let d = (w0 * z0 + w1 * z1 + w2 * z2) as f32;
            let idx = y * res + x;
            if d < depth[idx] {
                depth[idx] = d;
            }
        }
    }
}
