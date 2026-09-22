use crate::math::{quantize, fx_hashmap, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;

pub type VertexKey = (i64, i64, i64);

/// Smooth shading data: unique vertex normals/positions + per-triangle indices.
pub struct SmoothData {
    /// Smoothed normal per unique vertex.
    pub normals: Vec<Vec3>,
    /// World position per unique vertex.
    pub positions: Vec<Vec3>,
    /// Per-triangle: indices into normals/positions for each of the 3 vertices.
    pub tri_indices: Vec<[usize; 3]>,
}

/// Build a map of vertex position -> accumulated (smoothed) normal.
pub fn build_vertex_normal_map(triangles: &[Triangle]) -> FxHashMap<VertexKey, Vec3> {
    let mut normal_map: FxHashMap<VertexKey, Vec3> = fx_hashmap();

    for tri in triangles {
        let n = tri.normal;
        for v in &tri.vertices {
            let key = quantize(*v);
            let entry = normal_map.entry(key).or_insert(Vec3::new(0.0, 0.0, 0.0));
            *entry = *entry + n;
        }
    }

    for n in normal_map.values_mut() {
        *n = n.normalized();
    }

    normal_map
}

/// Quantize a normal to an integer bucket. Coarser than the position grid —
/// authoring tools round to 6 decimals or fewer, so 1e-3 is enough to
/// coalesce numerical drift while keeping calculate_normals crease splits
/// (which sit ≥ several degrees apart) distinct.
#[inline]
fn quantize_normal(n: Vec3) -> (i32, i32, i32) {
    let s = 1000.0;
    ((n.x * s).round() as i32, (n.y * s).round() as i32, (n.z * s).round() as i32)
}

/// Compute per-vertex normals for smooth shading, deciding per face.
///
/// A face keeps its authored `vertex_normals` (e.g. from a PLY's `nx/ny/nz` or
/// Manifold's `calculate_normals`) — quantized by `(position, normal)` so
/// crease-split vertices don't get re-merged.
///
/// A face is instead recomputed when it has no authored normals, or when its
/// authored normals are *flat* (all three within ~2.5° of each other) yet it
/// belongs to an OBJ smoothing group. That last case is a self-contradictory
/// export — per-face normals say "faceted" while `s N` says "smooth" — so the
/// flat normals carry no information and the smoothing group wins. Recomputed
/// faces average face normals at each shared position, keyed by smoothing group
/// (a face with no group, e.g. `s off`, is left flat).
///
/// The return is per-unique-vertex, enabling memoized shading (shade each unique
/// vertex once, index from triangles).
pub fn compute_vertex_normals(triangles: &[Triangle]) -> SmoothData {
    let est_unique = triangles.len();

    #[derive(PartialEq, Eq, Hash)]
    enum NKey {
        Authored((i64, i64, i64), (i32, i32, i32)),
        Recompute((i64, i64, i64), i64),
    }

    let mut index_map: FxHashMap<NKey, usize> = fx_hashmap_cap(est_unique);
    let mut normals: Vec<Vec3> = Vec::with_capacity(est_unique);
    let mut positions: Vec<Vec3> = Vec::with_capacity(est_unique);
    let mut tri_indices: Vec<[usize; 3]> = Vec::with_capacity(triangles.len());

    for (ti, tri) in triangles.iter().enumerate() {
        let recompute = match tri.vertex_normals {
            None => true,
            Some([a, b, c]) => {
                let eq = |x: Vec3, y: Vec3| x.normalized().dot(y.normalized()) > 0.999;
                eq(a, b) && eq(b, c) && tri.smoothing_group.is_some()
            }
        };
        let mut indices = [0usize; 3];
        for i in 0..3 {
            let v = tri.vertices[i];
            let (key, add) = if recompute {
                let g = tri.smoothing_group.map(|g| g as i64).unwrap_or(-(1 + ti as i64));
                (NKey::Recompute(quantize(v), g), tri.normal)
            } else {
                let n = tri.vertex_normals.unwrap()[i];
                (NKey::Authored(quantize(v), quantize_normal(n)), n)
            };
            let len = normals.len();
            let idx = *index_map.entry(key).or_insert_with(|| {
                normals.push(Vec3::new(0.0, 0.0, 0.0));
                positions.push(v);
                len
            });
            normals[idx] = normals[idx] + add;
            indices[i] = idx;
        }
        tri_indices.push(indices);
    }

    for n in &mut normals {
        *n = n.normalized();
    }

    SmoothData { normals, positions, tri_indices }
}
