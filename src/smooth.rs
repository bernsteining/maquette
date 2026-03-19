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

/// Compute per-vertex normals by averaging face normals at shared positions.
/// Returns unique vertex normals/positions and per-triangle index arrays,
/// enabling memoized shading (shade each unique vertex once).
pub fn compute_vertex_normals(triangles: &[Triangle]) -> SmoothData {
    let est_unique = triangles.len(); // good upper bound for closed meshes
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
