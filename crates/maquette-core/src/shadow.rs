//! Shadow mapping — depth pass per light, PCF-filtered at shade time.
//!
//! Format-agnostic: the caster set is `&[[Vec3; 3]]` (per-triangle world-space
//! positions) and lights come from [`crate::light::PunctualLight`]. Callers
//! project their own triangle type into `[Vec3; 3]` before invoking
//! [`build_shadow_maps`] — cheap for any mesh representation.
//!
//! Directional lights get a single orthographic frustum sized to the scene
//! bounding sphere. Spot lights get a single perspective frustum aimed along
//! the light's direction. Point lights get a 6-face cube of 90°-fov perspective
//! frustums to cover the omnidirectional case.

use crate::math::FloatExt;
use crate::light::{LightKind, PunctualLight};
use crate::math::{Mat4, Vec3};

/// Alias for the caster input — one world-space triangle. Kept as a plain
/// array so callers don't pay for a wrapper type.
pub type CasterTri = [Vec3; 3];

const EMPTY: f32 = f32::MAX;

#[derive(Clone, Copy)]
pub struct BiasParams {
    pub bias: f64,
    pub normal_bias: f64,
    pub slope_bias: f64,
}

/// Single-frustum depth map + light-view projection.
#[derive(Clone)]
pub struct ShadowMap {
    view: Mat4,
    ortho: bool,
    half_extent: f64,
    tan_half_fov: f64,
    near: f64,
    far: f64,
    res: usize,
    depth: Vec<f32>,
    forward: Vec3,
    eye: Vec3,
}

impl ShadowMap {
    /// Construct an empty depth map for a frustum. Per-format shadow builders
    /// (which frame the light view differently) create the map, then fill it
    /// via `splat_tri`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        view: Mat4,
        ortho: bool,
        half_extent: f64,
        tan_half_fov: f64,
        near: f64,
        far: f64,
        res: usize,
        forward: Vec3,
        eye: Vec3,
    ) -> Self {
        ShadowMap {
            view,
            ortho,
            half_extent,
            tan_half_fov,
            near,
            far,
            res,
            depth: vec![EMPTY; res * res],
            forward,
            eye,
        }
    }

    #[inline(always)]
    fn project(&self, p: Vec3) -> Option<(f64, f64, f64)> {
        let v = self.view.transform_point(p);
        let fwd = -v.z;
        if fwd <= 1e-6 { return None; }
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

    /// Texel-space bounds `[x0, y0, x1, y1]` (inclusive, unclamped) of a
    /// world-space triangle's projection, or `None` when a corner is behind
    /// the light. Every point inside the triangle projects inside it.
    pub fn texel_rect(&self, tri: &[Vec3; 3]) -> Option<[i64; 4]> {
        let mut r = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
        for v in tri {
            let (x, y, _) = self.project(*v)?;
            let (fx, fy) = (x.floor() as i64, y.floor() as i64);
            r = [r[0].min(fx), r[1].min(fy), r[2].max(fx), r[3].max(fy)];
        }
        Some(r)
    }

    /// Texel containing the projection of `p`, or `None` behind the light.
    pub fn texel_of(&self, p: Vec3) -> Option<(i64, i64)> {
        let (x, y, _) = self.project(p)?;
        Some((x.floor() as i64, y.floor() as i64))
    }

    /// Side length of the square depth map, in texels.
    pub fn resolution(&self) -> usize {
        self.res
    }

    /// Row-major depth texels (`resolution²`).
    pub fn texels(&self) -> &[f32] {
        &self.depth
    }

    /// Mutable depth texels, for callers that patch a map in place (e.g.
    /// restoring regions from another map of the same frustum).
    pub fn texels_mut(&mut self) -> &mut [f32] {
        &mut self.depth
    }

    #[inline(always)]
    fn texel_world(&self, p: Vec3) -> f64 {
        if self.ortho {
            2.0 * self.half_extent / self.res as f64
        } else {
            let fwd = (p - self.eye).dot(self.forward).fmax(self.near);
            2.0 * self.tan_half_fov * fwd / self.res as f64
        }
    }

    #[inline(always)]
    fn light_dir(&self, p: Vec3) -> Vec3 {
        if self.ortho {
            self.forward.scale(-1.0)
        } else {
            (self.eye - p).normalized()
        }
    }

    /// Lit fraction ∈ [0, 1] for a world point with surface `normal`.
    /// Applies normal-offset + slope-scaled bias, then PCF-filters.
    pub fn lit(&self, p: Vec3, normal: Vec3, b: &BiasParams, softness: usize) -> f32 {
        let sample = if b.normal_bias > 0.0 {
            p.add(normal.scale(b.normal_bias * self.texel_world(p)))
        } else {
            p
        };
        let ndotl = normal.dot(self.light_dir(p)).abs().fmax(0.15);
        let tan_theta = ((1.0 - ndotl * ndotl).fmax(0.0)).sqrt() / ndotl;
        let bias = b.bias * (1.0 + b.slope_bias * tan_theta.fmin(6.0));

        let (sx, sy, depth) = match self.project(sample) {
            Some(v) => v,
            None => return 1.0,
        };
        let cx = sx.floor() as i64;
        let cy = sy.floor() as i64;
        let r = softness as i64;
        if self.window_inside(cx, cy, r) {
            let (res, n) = (self.res, (2 * r + 1) as usize);
            let cutoff = lit_cutoff(depth, bias);
            let mut lit = 0u32;
            for y in cy - r..=cy + r {
                lit += count_lit(&self.depth[y as usize * res..][..res], (cx - r) as usize, n, 1, cutoff, depth, bias);
            }
            return lit as f32 / (n * n) as f32;
        }
        let mut lit = 0u32;
        let mut total = 0u32;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx + dx;
                let y = cy + dy;
                total += 1;
                if x < 0 || y < 0 || x >= self.res as i64 || y >= self.res as i64 {
                    lit += 1;
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

    /// Contact-hardening soft shadow (PCSS). Searches for blockers to estimate
    /// a world-space penumbra, then filters with a proportional PCF kernel:
    /// sharp where the occluder is close, soft where it's far. `light_size`
    /// is the emitter's world-space size. Bounded sampling keeps per-pixel
    /// cost sane. Ported from maquette.
    pub fn lit_pcss(&self, p: Vec3, normal: Vec3, b: &BiasParams, base_softness: usize, light_size: f64) -> f32 {
        let sample = if b.normal_bias > 0.0 {
            p.add(normal.scale(b.normal_bias * self.texel_world(p)))
        } else {
            p
        };
        let ndotl = normal.dot(self.light_dir(p)).abs().fmax(0.15);
        let tan_theta = ((1.0 - ndotl * ndotl).fmax(0.0)).sqrt() / ndotl;
        let bias = b.bias * (1.0 + b.slope_bias * tan_theta.fmin(6.0));
        let (sx, sy, depth) = match self.project(sample) {
            Some(v) => v,
            None => return 1.0,
        };
        let cx = sx.floor() as i64;
        let cy = sy.floor() as i64;
        let tw = self.texel_world(p).fmax(1e-9);
        let light_texels = (light_size / tw).clamp(1.0, 24.0);

        let search = light_texels.ceil() as i64;
        let sstep = (search / 4).max(1);
        let mut bsum = 0.0f64;
        let mut bn = 0u32;
        let threshold = depth - bias;
        let blocker_cutoff = below_cutoff(threshold);
        if self.window_inside(cx, cy, search) {
            let res = self.res;
            let mut dy = -search;
            while dy <= search {
                let row = &self.depth[(cy + dy) as usize * res..][..res];
                let mut x = (cx - search) as usize;
                let x_end = (cx + search) as usize;
                while x <= x_end {
                    let s = row[x];
                    if s < blocker_cutoff {
                        bsum += s as f64;
                        bn += 1;
                    }
                    x += sstep as usize;
                }
                dy += sstep;
            }
        } else {
            let mut dy = -search;
            while dy <= search {
                let mut dx = -search;
                while dx <= search {
                    let s = self.stored(cx + dx, cy + dy);
                    if s != EMPTY && (s as f64) < threshold {
                        bsum += s as f64;
                        bn += 1;
                    }
                    dx += sstep;
                }
                dy += sstep;
            }
        }
        if bn == 0 {
            return 1.0;
        }
        let avg_blocker = bsum / bn as f64;

        let penumbra = ((depth - avg_blocker) / avg_blocker).fmax(0.0);
        let radius = ((penumbra * light_texels * 8.0).fmax(base_softness as f64)).clamp(1.0, 12.0) as i64;

        let pstep = (radius / 6).max(1);
        let mut lit = 0u32;
        let mut total = 0u32;
        if self.window_inside(cx, cy, radius) {
            let res = self.res;
            let n = (2 * radius / pstep + 1) as usize;
            let cutoff = lit_cutoff(depth, bias);
            let mut dy = -radius;
            while dy <= radius {
                let row = &self.depth[(cy + dy) as usize * res..][..res];
                lit += count_lit(row, (cx - radius) as usize, n, pstep as usize, cutoff, depth, bias);
                total += n as u32;
                dy += pstep;
            }
        } else {
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
        }
        lit as f32 / total as f32
    }

    #[inline(always)]
    fn window_inside(&self, cx: i64, cy: i64, r: i64) -> bool {
        let res = self.res as i64;
        cx - r >= 0 && cy - r >= 0 && cx + r < res && cy + r < res
    }

    #[inline]
    fn stored(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x >= self.res as i64 || y >= self.res as i64 {
            EMPTY
        } else {
            self.depth[y as usize * self.res + x as usize]
        }
    }

    /// Rasterise scene triangles (no material filter — everything casts) into
    /// this map's depth buffer, keeping the nearest depth per texel.
    /// Project + rasterise one world-space triangle into this map, keeping the
    /// nearest depth per texel. Public so per-format builders can drive their
    /// own caster iteration (e.g. with an occluder filter).
    pub fn splat_tri(&mut self, tri: CasterTri) {
        let res = self.res;
        let mut sp = [(0.0f64, 0.0f64, 0.0f64); 3];
        for (i, v) in tri.iter().enumerate() {
            match self.project(*v) {
                Some(p) => sp[i] = p,
                None => return,
            }
        }
        rasterize_depth(&mut self.depth, res, &sp);
    }

    fn render(&mut self, triangles: &[CasterTri]) {
        for tri in triangles {
            self.splat_tri(*tri);
        }
    }
}

/// A light's shadow: single frustum (directional / spot / external point) or
/// a 6-face cube (omnidirectional point). Spot could use a tighter cone but
/// a single frustum covers the outer cone adequately.
#[derive(Clone)]
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
    /// The single frustum, or `None` for an omnidirectional cube.
    pub fn single(&self) -> Option<&ShadowMap> {
        match self {
            LightShadow::Single(m) => Some(m),
            LightShadow::Cube(_) => None,
        }
    }

    /// Mutable single frustum, or `None` for an omnidirectional cube.
    pub fn single_mut(&mut self) -> Option<&mut ShadowMap> {
        match self {
            LightShadow::Single(m) => Some(m),
            LightShadow::Cube(_) => None,
        }
    }

    /// Rasterise more casters into an existing shadow. Depth keeps the nearest
    /// value per texel, so splitting casters across calls gives the same map.
    pub fn add_casters(&mut self, triangles: &[CasterTri]) {
        self.render(triangles);
    }

    fn render(&mut self, triangles: &[CasterTri]) {
        match self {
            LightShadow::Single(m) => m.render(triangles),
            LightShadow::Cube(f) => f.iter_mut().for_each(|m| m.render(triangles)),
        }
    }
}

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
        tan_half_fov: 1.0,
        near,
        far,
        res,
        depth: vec![EMPTY; res * res],
        forward,
        eye,
    }
}

pub fn build_cube(eye: Vec3, br: f64, res: usize) -> Box<[ShadowMap; 6]> {
    let near = (br * 0.02).fmax(1e-4);
    let far = br * 3.5;
    let z = Vec3::new(0.0, 0.0, 1.0);
    let y = Vec3::new(0.0, 1.0, 0.0);
    Box::new([
        cube_face_map(eye, Vec3::new( 1.0, 0.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(-1.0, 0.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0,  1.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, -1.0, 0.0), z, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, 0.0,  1.0), y, near, far, res),
        cube_face_map(eye, Vec3::new(0.0, 0.0, -1.0), y, near, far, res),
    ])
}

/// Build one shadow (single frustum or cube) per light, then rasterise scene
/// triangles into each. `bc`/`br` frame each view.
pub fn build_shadow_maps(
    triangles: &[CasterTri],
    lights: &[PunctualLight],
    bc: Vec3,
    br: f64,
    up: Vec3,
    resolution: usize,
) -> Vec<Option<LightShadow>> {
    lights.iter().map(|light| {
        if !light.cast_shadow { return None; }
        let mut ls = match light.kind {
            LightKind::Point => LightShadow::Cube(build_cube(light.position, br, resolution)),
            _ => LightShadow::Single(build_single(light, bc, br, up, resolution)),
        };
        ls.render(triangles);
        Some(ls)
    }).collect()
}

fn build_single(light: &PunctualLight, bc: Vec3, br: f64, up: Vec3, res: usize) -> ShadowMap {
    let forward = light.direction.normalized();
    let up_aux = if forward.cross(up).length() > 1e-3 {
        up
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };

    match light.kind {
        LightKind::Directional => {
            let eye = bc.sub(forward.scale(br * 2.0));
            let eye_dist = br * 2.0;
            ShadowMap {
                view: Mat4::look_at(eye, bc, up_aux),
                ortho: true,
                half_extent: br * 1.05,
                tan_half_fov: 0.0,
                near: (eye_dist - br * 1.2).fmax(1e-4),
                far: eye_dist + br * 1.2,
                res,
                depth: vec![EMPTY; res * res],
                forward,
                eye,
            }
        }
        _ => {
            let eye = light.position;
            let dist = (bc - eye).length().fmax(br * 0.1);
            let tan_half_fov: f64 = if light.kind == LightKind::Spot {
                let outer = light.outer_cone_cos.acos();
                ((outer * 1.05).tan() as f64).fmax(0.05)
            } else {
                (br * 1.1 / dist).clamp(0.05, 10.0)
            };
            ShadowMap {
                view: Mat4::look_at(eye, eye.add(forward), up_aux),
                ortho: false,
                half_extent: 0.0,
                tan_half_fov,
                near: (dist - br * 1.2).fmax(dist * 0.01),
                far: dist + br * 1.2,
                res,
                depth: vec![EMPTY; res * res],
                forward,
                eye,
            }
        }
    }
}

/// Fill a triangle into the depth buffer, keeping the nearest (min) depth per texel.
fn rasterize_depth(depth: &mut [f32], res: usize, p: &[(f64, f64, f64); 3]) {
    let (x0, y0, z0) = p[0];
    let (x1, y1, z1) = p[1];
    let (x2, y2, z2) = p[2];
    let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if area.abs() < 1e-12 { return; }
    let inv_area = 1.0 / area;
    let min_x = x0.fmin(x1).fmin(x2).floor().fmax(0.0) as usize;
    let max_x = (x0.fmax(x1).fmax(x2).ceil() as i64).clamp(0, res as i64) as usize;
    let min_y = y0.fmin(y1).fmin(y2).floor().fmax(0.0) as usize;
    let max_y = (y0.fmax(y1).fmax(y2).ceil() as i64).clamp(0, res as i64) as usize;
    let edges = edge_crossings(p, inv_area);
    for y in min_y..max_y {
        let py = y as f64 + 0.5;
        let (xa, xb) = row_span(&edges, py, min_x, max_x);
        for x in xa..xb {
            let px = x as f64 + 0.5;
            let w0 = ((x1 - px) * (y2 - py) - (x2 - px) * (y1 - py)) * inv_area;
            let w1 = ((x2 - px) * (y0 - py) - (x0 - px) * (y2 - py)) * inv_area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
            let d = (w0 * z0 + w1 * z1 + w2 * z2) as f32;
            let idx = y * res + x;
            if d < depth[idx] { depth[idx] = d; }
        }
    }
}

fn edge_crossings(p: &[(f64, f64, f64); 3], inv_area: f64) -> [(i8, f64, f64); 3] {
    let mut out = [(0i8, 0.0, 0.0); 3];
    for (i, e) in out.iter_mut().enumerate() {
        let (xa, ya, _) = p[(i + 1) % 3];
        let (xb, yb, _) = p[(i + 2) % 3];
        let slope = (ya - yb) * inv_area;
        if slope.abs() > 1e-6 {
            let inv_dy = 1.0 / (yb - ya);
            *e = (if slope > 0.0 { 1 } else { -1 }, (xa * yb - xb * ya) * inv_dy - 0.5, (xb - xa) * inv_dy);
        }
    }
    out
}

fn row_span(edges: &[(i8, f64, f64); 3], py: f64, min_x: usize, max_x: usize) -> (usize, usize) {
    let (mut lo, mut hi) = (min_x as f64, max_x as f64);
    for &(dir, c0, c1) in edges {
        let cross = c0 + c1 * py;
        if dir > 0 {
            let c = (cross - 1.0).floor();
            if c > lo { lo = c; }
        } else if dir < 0 {
            let c = (cross + 2.0).ceil();
            if c < hi { hi = c; }
        }
    }
    if lo >= hi { return (0, 0); }
    (lo as usize, hi as usize)
}

fn count_lit(row: &[f32], x0: usize, n: usize, step: usize, cutoff: Option<f32>, depth: f64, bias: f64) -> u32 {
    let mut lit = 0u32;
    let mut x = x0;
    match cutoff {
        Some(c) => for _ in 0..n {
            lit += (row[x] >= c) as u32;
            x += step;
        },
        None => for _ in 0..n {
            let s = row[x];
            lit += (s == EMPTY || depth <= s as f64 + bias) as u32;
            x += step;
        },
    }
    lit
}

/// Smallest finite `f32` `s` with `depth <= s as f64 + bias`. The sum is
/// monotonic in `s`, so the PCF lit test reduces to `s >= cutoff` (and the
/// `EMPTY` sentinel, being `f32::MAX`, always passes). `None` if not found in
/// a few ulps, in which case callers fall back to the exact test.
fn lit_cutoff(depth: f64, bias: f64) -> Option<f32> {
    let lit = |s: f32| depth <= s as f64 + bias;
    let mut t = (depth - bias) as f32;
    if !t.is_finite() { return None; }
    for _ in 0..8 {
        if lit(t) {
            let d = t.next_down();
            if !lit(d) { return Some(t); }
            t = d;
        } else {
            t = t.next_up();
            if !t.is_finite() { return None; }
        }
    }
    None
}

/// Smallest `f32` `c` (capped at `EMPTY`) with `c as f64 >= threshold`, so
/// `s != EMPTY && (s as f64) < threshold` reduces to `s < c`.
fn below_cutoff(threshold: f64) -> f32 {
    let mut c = threshold as f32;
    if (c as f64) < threshold { c = c.next_up(); }
    if c > EMPTY { EMPTY } else { c }
}
