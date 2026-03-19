use crate::math::{quantize, edge_key, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
        }
    }
}

/// Move connected components apart from the model center.
///
/// If triangles have OBJ group IDs, those are used directly as components.
/// Otherwise, triangles sharing edges are grouped via union-find.
/// Each component is offset uniformly by `factor * (component_centroid - center)`.
pub fn explode_triangles(triangles: &mut [Triangle], center: Vec3, factor: f64) {
    if factor.abs() < 1e-12 || triangles.is_empty() {
        return;
    }

    // Check if triangles have OBJ group IDs
    let has_groups = triangles.iter().any(|t| t.group_id.is_some());

    let groups: FxHashMap<usize, Vec<usize>> = if has_groups {
        // Use OBJ groups directly: group_id → list of triangle indices
        let mut g: FxHashMap<u32, Vec<usize>> = fx_hashmap_cap(16);
        for (ti, tri) in triangles.iter().enumerate() {
            let gid = tri.group_id.unwrap_or(u32::MAX);
            g.entry(gid).or_default().push(ti);
        }
        // Re-key as usize for uniform handling
        let mut out = fx_hashmap_cap(g.len());
        for (k, v) in g { out.insert(k as usize, v); }
        out
    } else {
        // Fall back to union-find on shared edges
        let n = triangles.len();
        let mut uf = UnionFind::new(n);
        let mut edge_map: FxHashMap<((i64, i64, i64), (i64, i64, i64)), usize> = fx_hashmap_cap(triangles.len() * 3 / 2);

        for (ti, tri) in triangles.iter().enumerate() {
            let vq = [
                quantize(tri.vertices[0]),
                quantize(tri.vertices[1]),
                quantize(tri.vertices[2]),
            ];
            for e in 0..3 {
                let key = edge_key(vq[e], vq[(e + 1) % 3]);
                if let Some(&other_ti) = edge_map.get(&key) {
                    uf.union(ti, other_ti);
                } else {
                    edge_map.insert(key, ti);
                }
            }
        }

        let mut g: FxHashMap<usize, Vec<usize>> = fx_hashmap_cap(16);
        for ti in 0..n {
            let root = uf.find(ti);
            g.entry(root).or_default().push(ti);
        }
        g
    };

    // Compute per-component centroid and apply offset
    for (_comp, indices) in &groups {
        let mut sum = Vec3::new(0.0, 0.0, 0.0);
        let mut count = 0usize;
        for &ti in indices {
            for v in &triangles[ti].vertices {
                sum = sum + *v;
                count += 1;
            }
        }
        let comp_centroid = sum.scale(1.0 / count as f64);
        let offset = (comp_centroid - center).scale(factor);
        for &ti in indices {
            for v in triangles[ti].vertices.iter_mut() {
                *v = *v + offset;
            }
        }
    }
}
