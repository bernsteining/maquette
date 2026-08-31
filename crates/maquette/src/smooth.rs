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

/// Compute per-vertex normals for smooth shading.
///
/// If every input triangle carries `vertex_normals` (e.g. from a PLY with
/// `nx/ny/nz` or from Manifold's `calculate_normals`), the authored per-corner
/// normals are used directly — quantized by `(position, normal)` so
/// crease-split vertices from creased authoring don't get re-merged. Otherwise
/// the classic fallback runs: face normals averaged at each shared position.
///
/// Either way the return is per-unique-vertex, enabling memoized shading
/// (shade each unique vertex once, index from triangles).
pub fn compute_vertex_normals(triangles: &[Triangle]) -> SmoothData {
    let est_unique = triangles.len();

    // Fast path: fully authored per-vertex normals — use them verbatim.
    if !triangles.is_empty() && triangles.iter().all(|t| t.vertex_normals.is_some()) {
        type Key = ((i64, i64, i64), (i32, i32, i32));
        let mut index_map: FxHashMap<Key, usize> = fx_hashmap_cap(est_unique);
        let mut normals: Vec<Vec3> = Vec::with_capacity(est_unique);
        let mut positions: Vec<Vec3> = Vec::with_capacity(est_unique);
        let mut tri_indices: Vec<[usize; 3]> = Vec::with_capacity(triangles.len());
        for tri in triangles {
            let vns = tri.vertex_normals.unwrap();
            let mut indices = [0usize; 3];
            for i in 0..3 {
                let v = tri.vertices[i];
                let n = vns[i];
                let key: Key = (quantize(v), quantize_normal(n));
                let len = normals.len();
                let idx = *index_map.entry(key).or_insert_with(|| {
                    normals.push(n);
                    positions.push(v);
                    len
                });
                indices[i] = idx;
            }
            tri_indices.push(indices);
        }
        // Renormalise in case the input carried unnormalised normals — cheap
        // insurance, one loop over `unique` verts (usually ≪ triangle count).
        for n in &mut normals { *n = n.normalized(); }
        return SmoothData { normals, positions, tri_indices };
    }

    // Fallback: average face normals at each shared position.
    let mut index_map: FxHashMap<VertexKey, usize> = fx_hashmap_cap(est_unique);
    let mut normals: Vec<Vec3> = Vec::with_capacity(est_unique);
    let mut positions: Vec<Vec3> = Vec::with_capacity(est_unique);
    let mut tri_indices: Vec<[usize; 3]> = Vec::with_capacity(triangles.len());

    for tri in triangles {
        let n = tri.normal;
        let mut indices = [0usize; 3];
        for (i, v) in tri.vertices.iter().enumerate() {
            let key = quantize(*v);
            let len = normals.len();
            let idx = *index_map.entry(key).or_insert_with(|| {
                normals.push(Vec3::new(0.0, 0.0, 0.0));
                positions.push(*v);
                len
            });
            normals[idx] = normals[idx] + n;
            indices[i] = idx;
        }
        tri_indices.push(indices);
    }

    for n in &mut normals {
        *n = n.normalized();
    }

    SmoothData { normals, positions, tri_indices }
}
