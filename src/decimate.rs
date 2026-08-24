use crate::math::Vec3;
use crate::parser::Triangle;
use std::arch::wasm32::*;

/// Grid vertex-clustering decimation (sort-based, SIMD-accelerated).
///
/// Format-agnostic: operates on the unified triangle list, so STL/OBJ/PLY are
/// all handled identically. A uniform `res³` grid is laid over the bounding box;
/// every vertex falling in a cell collapses to that cell's representative (the
/// average of the vertices that landed there), and triangles whose three corners
/// collapse into fewer than three distinct cells are dropped as degenerate.
///
/// Hot-path design (chosen for measured throughput in the wasm runtime):
///   1. Each vertex's `(i, j, k)` cell is packed into a 24-bit key with SIMD.
///   2. Vertex *indices* are LSD-radix-sorted by that key — a 4-byte payload, and
///      since keys are ≤ 24 bits only **3 byte-passes** are needed (the top byte
///      is always zero). Sorting the full 8-byte `(key|index)` word was measurably
///      slower here: the doubled memory traffic outweighs the saved key gathers.
///   3. A linear scan over the sorted runs averages each cluster, reading from a
///      compact f32 SOA (not the ~100-byte `Triangle`) and scattering the
///      representative back to each member vertex for O(1) lookup at emission.
///
/// `strength` is in `(0, 1]` (0 = off): higher means a coarser grid and a more
/// aggressive reduction. Quality is intentionally simple (blocky); it pairs well
/// with smooth shading, which re-derives vertex normals afterward.
pub fn decimate(triangles: &[Triangle], bmin: Vec3, bmax: Vec3, strength: f64) -> Vec<Triangle> {
    if strength <= 0.0 || triangles.len() < 2 {
        return triangles.to_vec();
    }
    let s = strength.min(1.0);

    // Geometric map strength → grid resolution along the longest axis.
    // s→0 ≈ 256 cells (near no-op), s = 1 ≈ 6 cells (aggressive). Capped at 256
    // so the packed key stays within 24 bits (256³ = 2²⁴) → only 3 radix passes.
    const N_FINE: f64 = 256.0;
    const N_COARSE: f64 = 6.0;
    let res = (N_FINE * (N_COARSE / N_FINE).powf(s)).round().clamp(2.0, 256.0) as i32;

    let longest = (bmax.x - bmin.x).max(bmax.y - bmin.y).max(bmax.z - bmin.z);
    if longest < 1e-12 {
        return triangles.to_vec();
    }
    let inv = res as f32 / longest as f32;
    let has_vcolors = triangles.iter().any(|t| t.vertex_colors.is_some());

    let nv = triangles.len() * 3;

    // --- Compact f32 SOA of every vertex: feeds the SIMD key pack AND the
    //     cluster averaging (4 bytes/coord vs gathering the fat Triangle). ---
    let mut xs: Vec<f32> = Vec::with_capacity(nv);
    let mut ys: Vec<f32> = Vec::with_capacity(nv);
    let mut zs: Vec<f32> = Vec::with_capacity(nv);
    // Optional compact color SOA, only when the mesh carries per-vertex colors.
    let (mut cr8, mut cg8, mut cb8) = (Vec::new(), Vec::new(), Vec::new());
    if has_vcolors {
        cr8 = Vec::with_capacity(nv);
        cg8 = Vec::with_capacity(nv);
        cb8 = Vec::with_capacity(nv);
    }
    for tri in triangles {
        for (i, v) in tri.vertices.iter().enumerate() {
            xs.push(v.x as f32);
            ys.push(v.y as f32);
            zs.push(v.z as f32);
            if has_vcolors {
                let (r, g, b) = tri.vertex_colors.map(|vc| vc[i]).unwrap_or((0, 0, 0));
                cr8.push(r);
                cg8.push(g);
                cb8.push(b);
            }
        }
    }

    // --- SIMD: pack each vertex into a cell key kx + ky*res + kz*res² (≤ 2²⁴). ---
    let mut keys = vec![0u32; nv];
    let bx = bmin.x as f32;
    let by = bmin.y as f32;
    let bz = bmin.z as f32;
    let max_idx = (res - 1) as f32;

    let inv4 = f32x4_splat(inv);
    let bx4 = f32x4_splat(bx);
    let by4 = f32x4_splat(by);
    let bz4 = f32x4_splat(bz);
    let zero4 = f32x4_splat(0.0);
    let maxi4 = f32x4_splat(max_idx);
    let res4 = i32x4_splat(res);
    let res2_4 = i32x4_splat(res * res);

    // cell index per axis, clamped to [0, res-1]: floor(clamp((v - b) * inv))
    #[inline(always)]
    fn cell_axis(v: v128, b: v128, inv: v128, zero: v128, maxi: v128) -> v128 {
        let f = f32x4_mul(f32x4_sub(v, b), inv);
        let f = f32x4_floor(f32x4_min(f32x4_max(f, zero), maxi));
        i32x4_trunc_sat_f32x4(f)
    }

    let mut b = 0usize;
    while b + 4 <= nv {
        let x4 = unsafe { v128_load(xs.as_ptr().add(b) as *const v128) };
        let y4 = unsafe { v128_load(ys.as_ptr().add(b) as *const v128) };
        let z4 = unsafe { v128_load(zs.as_ptr().add(b) as *const v128) };
        let kx = cell_axis(x4, bx4, inv4, zero4, maxi4);
        let ky = cell_axis(y4, by4, inv4, zero4, maxi4);
        let kz = cell_axis(z4, bz4, inv4, zero4, maxi4);
        let key = i32x4_add(kx, i32x4_add(i32x4_mul(ky, res4), i32x4_mul(kz, res2_4)));
        unsafe { v128_store(keys.as_mut_ptr().add(b) as *mut v128, key) };
        b += 4;
    }
    let clamp_axis = |c: f32| (((c).max(0.0)).min(max_idx)).floor() as i32;
    for v in b..nv {
        let kx = clamp_axis((xs[v] - bx) * inv);
        let ky = clamp_axis((ys[v] - by) * inv);
        let kz = clamp_axis((zs[v] - bz) * inv);
        keys[v] = (kx + ky * res + kz * res * res) as u32;
    }

    // --- Radix-sort vertex indices by key (3 passes; keys are ≤ 24 bits). ---
    let order = radix_sort_indices(&keys);

    // --- Linear scan over sorted runs: average each cluster (from the f32 SOA),
    //     scatter the representative back to every member vertex. ---
    let mut rep_x = vec![0f32; nv];
    let mut rep_y = vec![0f32; nv];
    let mut rep_z = vec![0f32; nv];
    let mut rep_col: Vec<(u8, u8, u8)> = if has_vcolors {
        vec![(0, 0, 0); nv]
    } else {
        Vec::new()
    };

    let mut i = 0usize;
    while i < nv {
        let key = keys[order[i] as usize];
        let mut j = i;
        let (mut sx, mut sy, mut sz) = (0f32, 0f32, 0f32);
        let (mut cr, mut cg, mut cb) = (0u32, 0u32, 0u32);
        while j < nv && keys[order[j] as usize] == key {
            let v = order[j] as usize;
            sx += xs[v];
            sy += ys[v];
            sz += zs[v];
            if has_vcolors {
                cr += cr8[v] as u32;
                cg += cg8[v] as u32;
                cb += cb8[v] as u32;
            }
            j += 1;
        }
        let cnt = (j - i) as u32;
        let inv_n = 1.0 / cnt as f32;
        let (ax, ay, az) = (sx * inv_n, sy * inv_n, sz * inv_n);
        let col = ((cr / cnt) as u8, (cg / cnt) as u8, (cb / cnt) as u8);
        for &o in &order[i..j] {
            let v = o as usize;
            rep_x[v] = ax;
            rep_y[v] = ay;
            rep_z[v] = az;
            if has_vcolors {
                rep_col[v] = col;
            }
        }
        i = j;
    }

    // --- Emit: drop degenerate triangles, rebuild surviving ones. ---
    let mut out = Vec::with_capacity(triangles.len());
    for (t, tri) in triangles.iter().enumerate() {
        let (a, b2, c) = (3 * t, 3 * t + 1, 3 * t + 2);
        // Two or more corners in the same cell → collapsed to a sliver/point.
        if keys[a] == keys[b2] || keys[b2] == keys[c] || keys[a] == keys[c] {
            continue;
        }

        let pa = Vec3::new(rep_x[a] as f64, rep_y[a] as f64, rep_z[a] as f64);
        let pb = Vec3::new(rep_x[b2] as f64, rep_y[b2] as f64, rep_z[b2] as f64);
        let pc = Vec3::new(rep_x[c] as f64, rep_y[c] as f64, rep_z[c] as f64);

        let normal = match Vec3::face_normal(pa, pb, pc) {
            Some(n) => n,
            None => continue, // collinear after clustering
        };

        let vertex_colors = if has_vcolors {
            Some([rep_col[a], rep_col[b2], rep_col[c]])
        } else {
            None
        };

        out.push(Triangle {
            vertices: [pa, pb, pc],
            normal,
            color: tri.color,
            vertex_colors,
            group_id: tri.group_id,
            alpha: tri.alpha,
        });
    }

    out
}

/// LSD radix sort of `0..keys.len()` by the `u32` value at each index. Keys are
/// ≤ 24 bits, so only the low 3 bytes need sorting — 3 passes, ping-ponging
/// between two index buffers; an odd swap count leaves the result in `idx`.
fn radix_sort_indices(keys: &[u32]) -> Vec<u32> {
    let n = keys.len();
    let mut idx: Vec<u32> = (0..n as u32).collect();
    if n <= 1 {
        return idx;
    }
    let mut tmp = vec![0u32; n];

    let mut hist = [[0u32; 256]; 3];
    for &k in keys {
        hist[0][(k & 0xFF) as usize] += 1;
        hist[1][((k >> 8) & 0xFF) as usize] += 1;
        hist[2][((k >> 16) & 0xFF) as usize] += 1;
    }
    for h in &mut hist {
        let mut sum = 0u32;
        for c in h.iter_mut() {
            let count = *c;
            *c = sum;
            sum += count;
        }
    }

    for (pass, off) in hist.iter_mut().enumerate() {
        let shift = pass * 8;
        for &i in idx.iter() {
            let bucket = ((keys[i as usize] >> shift) & 0xFF) as usize;
            tmp[off[bucket] as usize] = i;
            off[bucket] += 1;
        }
        std::mem::swap(&mut idx, &mut tmp);
    }
    idx
}
