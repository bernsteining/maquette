use crate::math::{quantize, edge_key, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;

/// A screen-space edge to draw.
pub struct ScreenEdge {
    pub v0: (f64, f64),
    pub v1: (f64, f64),
}

/// Stack-allocated pair of adjacent faces (an edge has at most 2 in a manifold mesh).
struct EdgeFaces {
    data: [(usize, bool); 2],
    len: u8,
}

impl EdgeFaces {
    fn new() -> Self { Self { data: [(0, false); 2], len: 0 } }
    fn push(&mut self, val: (usize, bool)) {
        if (self.len as usize) < 2 { self.data[self.len as usize] = val; }
        self.len += 1;
    }
}

/// Find silhouette edges given world-space triangles and a view direction.
/// `view_dir` should point from the camera toward the scene center.
/// `project_fn` projects a world-space point to screen coordinates.
pub fn find_silhouette_edges(
    triangles: &[Triangle],
    view_dir: Vec3,
    project_fn: impl Fn(Vec3) -> (f64, f64),
) -> Vec<ScreenEdge> {
    // Build edge adjacency and screen-space vertex lookup in a single pass
    let mut edge_faces: FxHashMap<
        ((i64, i64, i64), (i64, i64, i64)),
        EdgeFaces,
    > = fx_hashmap_cap(triangles.len() * 3 / 2);
    let mut screen_map: FxHashMap<(i64, i64, i64), (f64, f64)> = fx_hashmap_cap(triangles.len());

    for (fi, tri) in triangles.iter().enumerate() {
        let front = tri.normal.dot(view_dir) < 0.0;
        let vq: [_; 3] = [
            quantize(tri.vertices[0]),
            quantize(tri.vertices[1]),
            quantize(tri.vertices[2]),
        ];
        for e in 0..3 {
            let key = edge_key(vq[e], vq[(e + 1) % 3]);
            edge_faces.entry(key).or_insert_with(EdgeFaces::new).push((fi, front));
            screen_map.entry(vq[e]).or_insert_with(|| project_fn(tri.vertices[e]));
        }
    }

    let mut edges = Vec::new();

    for (key, faces) in &edge_faces {
        let is_silhouette = if faces.len == 1 {
            faces.data[0].1
        } else {
            faces.data[0].1 != faces.data[1].1
        };

        if is_silhouette {
            if let (Some(&v0), Some(&v1)) = (screen_map.get(&key.0), screen_map.get(&key.1)) {
                edges.push(ScreenEdge { v0, v1 });
            }
        }
    }

    edges
}
