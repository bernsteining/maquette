use crate::color::lerp_color;
use crate::math::{quantize, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;

type Color3 = (u8, u8, u8);

/// Get the effective color for vertex `i` of a triangle.
/// Prefers vertex_colors, falls back to face color, then default gray.
#[inline]
fn vertex_color(tri: &Triangle, i: usize) -> Color3 {
    if let Some(vc) = tri.vertex_colors {
        vc[i]
    } else if let Some(c) = tri.color {
        c
    } else {
        (128, 128, 128)
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
pub fn clip_triangles(triangles: &[Triangle], plane: [f64; 4], cap: bool) -> Vec<Triangle> {
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
            _ => clip_triangle(tri, &dists, &inside, &mut result, &mut cap_edges),
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

        let c0 = vertex_color(tri, i0);
        let c1 = lerp_color(vertex_color(tri, i0), vertex_color(tri, i1), t1);
        let c2 = lerp_color(vertex_color(tri, i0), vertex_color(tri, i2), t2);

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

        let c_a = lerp_color(vertex_color(tri, i0), vertex_color(tri, i1), t_a);
        let c_b = lerp_color(vertex_color(tri, i0), vertex_color(tri, i2), t_b);
        let c1 = vertex_color(tri, i1);
        let c2 = vertex_color(tri, i2);

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

/// Chain unordered edges into closed loops, then fan-triangulate each loop.
fn generate_cap(edges: &[CapEdge], cap_normal: Vec3, out: &mut Vec<Triangle>) {
    let loops = chain_edges(edges);

    for chain in &loops {
        if chain.len() < 3 {
            continue;
        }

        // Compute centroid
        let n = chain.len() as f64;
        let center = Vec3::new(
            chain.iter().map(|&(v, _)| v.x).sum::<f64>() / n,
            chain.iter().map(|&(v, _)| v.y).sum::<f64>() / n,
            chain.iter().map(|&(v, _)| v.z).sum::<f64>() / n,
        );
        let center_color = {
            let n = chain.len() as f64;
            let (sr, sg, sb) = chain.iter().fold((0.0, 0.0, 0.0), |(r, g, b), &(_, c)| {
                (r + c.0 as f64, g + c.1 as f64, b + c.2 as f64)
            });
            ((sr / n).round() as u8, (sg / n).round() as u8, (sb / n).round() as u8)
        };

        // Check winding via signed area relative to cap_normal
        let mut area = 0.0;
        for i in 0..chain.len() {
            let j = (i + 1) % chain.len();
            let vi = chain[i].0 - center;
            let vj = chain[j].0 - center;
            area += vi.cross(vj).dot(cap_normal);
        }

        // Fan triangulate from centroid
        let reversed = area < 0.0;
        let len = chain.len();
        for i in 0..len {
            let j = (i + 1) % len;
            let (a, b) = if reversed { (j, i) } else { (i, j) };
            out.push(Triangle {
                vertices: [center, chain[a].0, chain[b].0],
                normal: cap_normal,
                color: None,
                vertex_colors: Some([center_color, chain[a].1, chain[b].1]),
                group_id: None,
            });
        }
    }
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
