use crate::math::{FloatExt, Vec3};
use crate::parser::Triangle;

pub(crate) const ALL_EDGES: u8 = 0b111;

const MAX_DEPTH: u32 = 7;

pub(crate) struct Tessellated {
    pub(crate) tris: Vec<Triangle>,
    pub(crate) edge_masks: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Corner {
    p: Vec3,
    b: [f64; 3],
}

fn mid(a: Corner, c: Corner) -> Corner {
    Corner {
        p: Vec3::new((a.p.x + c.p.x) * 0.5, (a.p.y + c.p.y) * 0.5, (a.p.z + c.p.z) * 0.5),
        b: [(a.b[0] + c.b[0]) * 0.5, (a.b[1] + c.b[1]) * 0.5, (a.b[2] + c.b[2]) * 0.5],
    }
}

fn lerp_u8(v: [(u8, u8, u8); 3], b: [f64; 3]) -> (u8, u8, u8) {
    let ch = |f: fn(&(u8, u8, u8)) -> u8| {
        (f(&v[0]) as f64 * b[0] + f(&v[1]) as f64 * b[1] + f(&v[2]) as f64 * b[2]).fround().clamp(0.0, 255.0) as u8
    };
    (ch(|c| c.0), ch(|c| c.1), ch(|c| c.2))
}

fn sub_triangle(src: &Triangle, c: [Corner; 3]) -> Triangle {
    let bs = [c[0].b, c[1].b, c[2].b];
    Triangle {
        vertices: [c[0].p, c[1].p, c[2].p],
        normal: src.normal,
        color: src.color,
        vertex_colors: src.vertex_colors.map(|v| bs.map(|b| lerp_u8(v, b))),
        group_id: src.group_id,
        alpha: src.alpha,
        vertex_normals: src.vertex_normals.map(|n| bs.map(|b| {
            n[0].scale(b[0]).add(n[1].scale(b[1])).add(n[2].scale(b[2])).normalized()
        })),
        smoothing_group: src.smoothing_group,
        vertex_scalars: src.vertex_scalars.map(|s| bs.map(|b| s[0] * b[0] + s[1] * b[1] + s[2] * b[2])),
        uvs: src.uvs.map(|uv| bs.map(|b| {
            let (b0, b1, b2) = (b[0] as f32, b[1] as f32, b[2] as f32);
            [uv[0][0] * b0 + uv[1][0] * b1 + uv[2][0] * b2, uv[0][1] * b0 + uv[1][1] * b1 + uv[2][1] * b2]
        })),
        tex: src.tex,
        splat: src.splat,
    }
}

fn corners(t: &Triangle) -> [Corner; 3] {
    [
        Corner { p: t.vertices[0], b: [1.0, 0.0, 0.0] },
        Corner { p: t.vertices[1], b: [0.0, 1.0, 0.0] },
        Corner { p: t.vertices[2], b: [0.0, 0.0, 1.0] },
    ]
}

/// Clip triangles against the plane a distance `near` in front of `eye` along
/// `forward`, so no vertex reaches the perspective divide behind the camera.
/// Returns `None` when every triangle already lies in front of the plane.
pub(crate) fn clip_near(tris: &[Triangle], eye: Vec3, forward: Vec3, near: f64) -> Option<Tessellated> {
    let dist = |p: Vec3| p.sub(eye).dot(forward) - near;
    if tris.iter().all(|t| t.splat || t.vertices.iter().all(|&p| dist(p) >= 0.0)) {
        return None;
    }
    let mut out = Tessellated { tris: Vec::with_capacity(tris.len()), edge_masks: Vec::with_capacity(tris.len()) };
    for t in tris {
        let d = t.vertices.map(dist);
        if t.splat || d.iter().all(|&x| x >= 0.0) {
            out.tris.push(*t);
            out.edge_masks.push(ALL_EDGES);
            continue;
        }
        if d.iter().all(|&x| x < 0.0) {
            continue;
        }
        let c = corners(t);
        let mut poly: Vec<(Corner, bool)> = Vec::with_capacity(4);
        for i in 0..3 {
            let j = (i + 1) % 3;
            if d[i] >= 0.0 {
                poly.push((c[i], true));
            }
            if (d[i] >= 0.0) != (d[j] >= 0.0) {
                let s = d[i] / (d[i] - d[j]);
                let p = c[i].p.add(c[j].p.sub(c[i].p).scale(s));
                let b = [0, 1, 2].map(|k| c[i].b[k] + (c[j].b[k] - c[i].b[k]) * s);
                poly.push((Corner { p, b }, d[i] < 0.0));
            }
        }
        for k in 1..poly.len() - 1 {
            let mut mask = 0u8;
            if k == 1 && poly[0].1 { mask |= 1; }
            if poly[k].1 { mask |= 2; }
            if k + 2 == poly.len() && poly[k + 1].1 { mask |= 4; }
            out.tris.push(sub_triangle(t, [poly[0].0, poly[k].0, poly[k + 1].0]));
            out.edge_masks.push(mask);
        }
    }
    Some(out)
}

/// Split triangles until every edge spans at most `max_angle` radians as seen
/// from `eye`, so curved projections bend long edges. Each edge's split depends
/// only on its endpoints, so neighbours agree and no cracks open. Returns `None`
/// when no edge needs splitting.
pub(crate) fn subdivide_angular(tris: &[Triangle], masks: Option<&[u8]>, eye: Vec3, max_angle: f64) -> Option<Tessellated> {
    let cos_max = max_angle.cos();
    let long = |a: Vec3, b: Vec3| {
        let (u, v) = (a.sub(eye), b.sub(eye));
        u.dot(v) < cos_max * (u.dot(u) * v.dot(v)).sqrt()
    };
    let needs = |t: &Triangle| !t.splat && (0..3).any(|i| long(t.vertices[i], t.vertices[(i + 1) % 3]));
    if !tris.iter().any(needs) {
        return None;
    }
    let mut out = Tessellated { tris: Vec::with_capacity(tris.len() * 2), edge_masks: Vec::with_capacity(tris.len() * 2) };
    for (ti, t) in tris.iter().enumerate() {
        let mask = masks.map_or(ALL_EDGES, |m| m[ti]);
        if !needs(t) {
            out.tris.push(*t);
            out.edge_masks.push(mask);
            continue;
        }
        split(t, corners(t), mask, 0, &long, &mut out);
    }
    Some(out)
}

fn split(src: &Triangle, c: [Corner; 3], mask: u8, depth: u32, long: &impl Fn(Vec3, Vec3) -> bool, out: &mut Tessellated) {
    let s = if depth >= MAX_DEPTH { [false; 3] } else { [0, 1, 2].map(|i| long(c[i].p, c[(i + 1) % 3].p)) };
    let n = s.iter().filter(|&&x| x).count();
    if n == 0 {
        out.tris.push(sub_triangle(src, c));
        out.edge_masks.push(mask);
        return;
    }
    let bit = |e: usize| (mask >> e) & 1;
    let rot = match n {
        1 => (0..3).find(|&i| s[i]).unwrap(),
        2 => (0..3).find(|&i| !s[(i + 2) % 3]).unwrap(),
        _ => 0,
    };
    let (p0, p1, p2) = (c[rot], c[(rot + 1) % 3], c[(rot + 2) % 3]);
    let (e0, e1, e2) = (bit(rot), bit((rot + 1) % 3), bit((rot + 2) % 3));
    let m = |a: u8, b: u8, c: u8| a | (b << 1) | (c << 2);
    let d = depth + 1;
    let m0 = mid(p0, p1);
    match n {
        1 => {
            split(src, [p0, m0, p2], m(e0, 0, e2), d, long, out);
            split(src, [m0, p1, p2], m(e0, e1, 0), d, long, out);
        }
        2 => {
            let m1 = mid(p1, p2);
            split(src, [m0, p1, m1], m(e0, e1, 0), d, long, out);
            split(src, [p0, m0, m1], m(e0, 0, 0), d, long, out);
            split(src, [p0, m1, p2], m(0, e1, e2), d, long, out);
        }
        _ => {
            let m1 = mid(p1, p2);
            let m2 = mid(p2, p0);
            split(src, [p0, m0, m2], m(e0, 0, e2), d, long, out);
            split(src, [m0, p1, m1], m(e0, e1, 0), d, long, out);
            split(src, [m2, m1, p2], m(0, e1, e2), d, long, out);
            split(src, [m0, m1, m2], 0, d, long, out);
        }
    }
}
