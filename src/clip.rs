use crate::color::lerp_color;
use crate::math::{quantize, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;

type Color3 = (u8, u8, u8);

/// Reserved `group_id` marking a clip cap face, so downstream rendering can
/// hatch the cross-section. `u32::MAX` is already taken by debug-light faces,
/// so caps use `MAX - 1`; real OBJ group ids are small and never collide.
pub const CAP_GID: u32 = u32::MAX - 1;

/// Get the effective color for vertex `i` of a triangle.
/// Prefers vertex_colors, falls back to face color, then the model's base color.
#[inline]
fn vertex_color(tri: &Triangle, i: usize, base: Color3) -> Color3 {
    if let Some(vc) = tri.vertex_colors {
        vc[i]
    } else if let Some(c) = tri.color {
        c
    } else {
        base
    }
}

/// A cap edge: two endpoints with their interpolated colors.
struct CapEdge {
    v0: Vec3,
    v1: Vec3,
    c0: Color3,
    c1: Color3,
}

/// Clip triangles against a plane `ax + by + cz + d = 0`.
/// Points where `ax + by + cz + d >= 0` are kept (inside).
/// When `cap` is true, generates cap triangles to close the cross-section.
pub fn clip_triangles(triangles: &[Triangle], plane: [f64; 4], cap: bool, base: Color3) -> Vec<Triangle> {
    let normal = Vec3::new(plane[0], plane[1], plane[2]);
    let d = plane[3];

    let mut result = Vec::with_capacity(triangles.len());
    let mut cap_edges: Vec<CapEdge> = Vec::new();

    for tri in triangles {
        let dists = [
            normal.dot(tri.vertices[0]) + d,
            normal.dot(tri.vertices[1]) + d,
            normal.dot(tri.vertices[2]) + d,
        ];

        let inside = [dists[0] >= 0.0, dists[1] >= 0.0, dists[2] >= 0.0];
        let count_inside = inside.iter().filter(|&&b| b).count();

        match count_inside {
            3 => result.push(*tri),
            0 => {} // fully clipped
            _ => clip_triangle(tri, &dists, &inside, &mut result, &mut cap_edges, base),
        }
    }

    // Generate cap faces to close the cross-section
    if cap && !cap_edges.is_empty() {
        let cap_normal = Vec3::new(-plane[0], -plane[1], -plane[2]).normalized();
        generate_cap(&cap_edges, cap_normal, &mut result);
    }

    result
}

fn clip_triangle(
    tri: &Triangle,
    dists: &[f64; 3],
    inside: &[bool; 3],
    out: &mut Vec<Triangle>,
    cap_edges: &mut Vec<CapEdge>,
    base: Color3,
) {
    if inside.iter().filter(|&&b| b).count() == 1 {
        // One vertex inside — produces 1 triangle
        let lone = unsafe { inside.iter().position(|&b| b).unwrap_unchecked() };
        let i0 = lone;
        let i1 = (lone + 1) % 3;
        let i2 = (lone + 2) % 3;

        let t1 = dists[i0] / (dists[i0] - dists[i1]);
        let t2 = dists[i0] / (dists[i0] - dists[i2]);

        let v0 = tri.vertices[i0];
        let v1 = intersect_at(tri.vertices[i0], tri.vertices[i1], t1);
        let v2 = intersect_at(tri.vertices[i0], tri.vertices[i2], t2);

        let c0 = vertex_color(tri, i0, base);
        let c1 = lerp_color(vertex_color(tri, i0, base), vertex_color(tri, i1, base), t1);
        let c2 = lerp_color(vertex_color(tri, i0, base), vertex_color(tri, i2, base), t2);

        out.push(Triangle {
            vertices: [v0, v1, v2],
            normal: tri.normal,
            color: tri.color,
            vertex_colors: Some([c0, c1, c2]),
            group_id: tri.group_id,
        });
        cap_edges.push(CapEdge { v0: v1, v1: v2, c0: c1, c1: c2 });
    } else {
        // Two vertices inside — produces 2 triangles (a quad)
        let lone = unsafe { inside.iter().position(|&b| !b).unwrap_unchecked() };
        let i0 = lone; // outside
        let i1 = (lone + 1) % 3; // inside
        let i2 = (lone + 2) % 3; // inside

        let t_a = dists[i0] / (dists[i0] - dists[i1]);
        let t_b = dists[i0] / (dists[i0] - dists[i2]);

        let a = intersect_at(tri.vertices[i0], tri.vertices[i1], t_a);
        let b = intersect_at(tri.vertices[i0], tri.vertices[i2], t_b);

        let c_a = lerp_color(vertex_color(tri, i0, base), vertex_color(tri, i1, base), t_a);
        let c_b = lerp_color(vertex_color(tri, i0, base), vertex_color(tri, i2, base), t_b);
        let c1 = vertex_color(tri, i1, base);
        let c2 = vertex_color(tri, i2, base);

        out.push(Triangle {
            vertices: [tri.vertices[i1], tri.vertices[i2], a],
            normal: tri.normal,
            color: tri.color,
            vertex_colors: Some([c1, c2, c_a]),
            group_id: tri.group_id,
        });
        out.push(Triangle {
            vertices: [tri.vertices[i2], b, a],
            normal: tri.normal,
            color: tri.color,
            vertex_colors: Some([c2, c_b, c_a]),
            group_id: tri.group_id,
        });
        cap_edges.push(CapEdge { v0: a, v1: b, c0: c_a, c1: c_b });
    }
}

/// Chain unordered edges into closed loops, then triangulate each loop.
fn generate_cap(edges: &[CapEdge], cap_normal: Vec3, out: &mut Vec<Triangle>) {
    for chain in &chain_edges(edges) {
        triangulate_loop(chain, cap_normal, out);
    }
}

#[inline]
fn push_cap_tri(out: &mut Vec<Triangle>, chain: &[(Vec3, Color3)], a: usize, b: usize, c: usize, cap_normal: Vec3) {
    out.push(Triangle {
        vertices: [chain[a].0, chain[b].0, chain[c].0],
        normal: cap_normal,
        color: None,
        vertex_colors: Some([chain[a].1, chain[b].1, chain[c].1]),
        group_id: Some(CAP_GID),
    });
}

/// Triangulate one closed cross-section loop by ear clipping. Unlike a
/// centroid fan this handles concave polygons (e.g. a bunny silhouette) without
/// spraying triangles outside the contour.
fn triangulate_loop(chain: &[(Vec3, Color3)], cap_normal: Vec3, out: &mut Vec<Triangle>) {
    let nv = chain.len();
    if nv < 3 { return; }

    // Project onto the cap plane using a right-handed basis (u, v, cap_normal).
    let (u, vv) = cap_normal.tangent_basis();
    let p: Vec<(f64, f64)> = chain.iter().map(|&(pt, _)| (pt.dot(u), pt.dot(vv))).collect();

    #[inline]
    fn cross2(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    }
    #[inline]
    fn in_tri(pt: (f64, f64), a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
        let d1 = cross2(a, b, pt); let d2 = cross2(b, c, pt); let d3 = cross2(c, a, pt);
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    }

    // Signed area → make the working index list counter-clockwise in (u, v).
    let mut signed = 0.0;
    for i in 0..nv { let j = (i + 1) % nv; signed += p[i].0 * p[j].1 - p[j].0 * p[i].1; }
    let mut idx: Vec<usize> = (0..nv).collect();
    if signed < 0.0 { idx.reverse(); }

    let mut guard = 0usize;
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ip, ic, inx) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a2, b2, c2) = (p[ip], p[ic], p[inx]);
            if cross2(a2, b2, c2) <= 0.0 { continue; } // reflex vertex — not an ear
            // Reject if any other vertex falls inside the candidate ear.
            if idx.iter().any(|&io| io != ip && io != ic && io != inx && in_tri(p[io], a2, b2, c2)) {
                continue;
            }
            push_cap_tri(out, chain, ip, ic, inx, cap_normal);
            idx.remove(k);
            clipped = true;
            break;
        }
        guard += 1;
        if !clipped || guard > nv * nv + 8 {
            // Degenerate/self-intersecting input: fan the remainder as a fallback.
            for k in 1..idx.len() - 1 { push_cap_tri(out, chain, idx[0], idx[k], idx[k + 1], cap_normal); }
            return;
        }
    }
    if idx.len() == 3 { push_cap_tri(out, chain, idx[0], idx[1], idx[2], cap_normal); }
}

/// Chain unordered edges into closed loops using quantized vertex matching.
/// Returns loops of (position, color) pairs.
fn chain_edges(edges: &[CapEdge]) -> Vec<Vec<(Vec3, Color3)>> {
    type VKey = (i64, i64, i64);

    // Build adjacency: quantized vertex → list of edge indices touching it
    let mut adj: FxHashMap<VKey, Vec<usize>> = fx_hashmap_cap(edges.len());
    let mut edge_data: Vec<(VKey, VKey, Vec3, Vec3, Color3, Color3)> = Vec::with_capacity(edges.len());
    for (i, e) in edges.iter().enumerate() {
        let ka = quantize(e.v0);
        let kb = quantize(e.v1);
        edge_data.push((ka, kb, e.v0, e.v1, e.c0, e.c1));
        adj.entry(ka).or_default().push(i);
        adj.entry(kb).or_default().push(i);
    }

    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();

    for start in 0..edges.len() {
        if used[start] { continue; }
        used[start] = true;
        let (ka, kb, va, vb, ca, cb) = edge_data[start];
        let mut chain: Vec<(Vec3, Color3)> = vec![(va, ca), (vb, cb)];
        let start_key = ka;
        let mut cur_key = kb;

        loop {
            // Check if loop is closed
            if chain.len() > 2 && cur_key == start_key {
                chain.pop();
                break;
            }

            // Find next unused edge at cur_key (O(1) lookup)
            let mut found = false;
            if let Some(neighbors) = adj.get(&cur_key) {
                for &ei in neighbors {
                    if used[ei] { continue; }
                    let (eka, ekb, eva, evb, eca, ecb) = edge_data[ei];
                    used[ei] = true;
                    if eka == cur_key {
                        chain.push((evb, ecb));
                        cur_key = ekb;
                    } else {
                        chain.push((eva, eca));
                        cur_key = eka;
                    }
                    found = true;
                    break;
                }
            }

            if !found { break; }
        }

        if chain.len() >= 3 {
            loops.push(chain);
        }
    }

    loops
}

/// Compute the intersection point between two vertices at parameter t.
#[inline]
fn intersect_at(v_in: Vec3, v_out: Vec3, t: f64) -> Vec3 {
    Vec3::new(
        v_in.x + t * (v_out.x - v_in.x),
        v_in.y + t * (v_out.y - v_in.y),
        v_in.z + t * (v_out.z - v_in.z),
    )
}
