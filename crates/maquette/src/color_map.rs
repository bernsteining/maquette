use crate::expr;
use crate::color::lerp_color;
use crate::math::{build_adjacency, quantize, fx_hashmap_cap, FxHashMap, Vec3};
use crate::parser::Triangle;
use crate::smooth::VertexKey;

/// Default height palette: blue -> cyan -> green -> yellow -> red.
const DEFAULT_PALETTE: [(u8, u8, u8); 5] = [
    (0, 0, 255),
    (0, 200, 255),
    (0, 200, 0),
    (255, 255, 0),
    (255, 0, 0),
];

fn resolve_palette(palette: &[(u8, u8, u8)]) -> &[(u8, u8, u8)] {
    if palette.is_empty() { &DEFAULT_PALETTE[..] } else { palette }
}

fn sample_palette(palette: &[(u8, u8, u8)], t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    if palette.len() == 1 {
        return palette[0];
    }
    let n = palette.len() - 1;
    let segment = (t * n as f64).min(n as f64 - 1e-9);
    let i = segment as usize;
    let frac = segment - i as f64;
    lerp_color(palette[i], palette[i + 1].min(palette[palette.len() - 1]), frac)
}

// ---------------------------------------------------------------------------
// Shared helpers: smoothing, vertex coloring
// ---------------------------------------------------------------------------

/// Laplacian smoothing of per-vertex scalar values.
fn smooth_values(
    values: &mut FxHashMap<VertexKey, f64>,
    adjacency: &FxHashMap<VertexKey, Vec<VertexKey>>,
    iterations: usize,
) {
    if iterations == 0 { return; }
    let mut buf = values.clone();
    for i in 0..iterations {
        let (read, write) = if i % 2 == 0 { (&*values, &mut buf) } else { (&buf, &mut *values) };
        for (key, &val) in read.iter() {
            let smoothed = if let Some(neighbors) = adjacency.get(key) {
                let mut sum = val;
                let mut count = 1.0;
                for nk in neighbors {
                    if let Some(&nv) = read.get(nk) {
                        sum += nv;
                        count += 1.0;
                    }
                }
                sum / count
            } else {
                val
            };
            write.insert(*key, smoothed);
        }
    }
    // After even iteration count: last write went to buf; odd: to values
    if iterations % 2 != 0 {
        *values = buf;
    }
}

/// Find (min, max) range of values. Returns None if range is degenerate.
fn value_range(values: &FxHashMap<VertexKey, f64>) -> Option<(f64, f64)> {
    let mut vmin = f64::INFINITY;
    let mut vmax = f64::NEG_INFINITY;
    for &v in values.values() {
        if v < vmin { vmin = v; }
        if v > vmax { vmax = v; }
    }
    if (vmax - vmin) < 1e-12 { None } else { Some((vmin, vmax)) }
}

/// Apply per-vertex colors from a scalar value map using a palette.
fn apply_vertex_colors(
    triangles: &mut [Triangle],
    values: &FxHashMap<VertexKey, f64>,
    vmin: f64,
    vrange: f64,
    palette: &[(u8, u8, u8)],
) {
    for tri in triangles.iter_mut() {
        let mut vcols = [(0u8, 0u8, 0u8); 3];
        let mut vals = [0.0; 3];
        for i in 0..3 {
            let key = quantize(tri.vertices[i]);
            vals[i] = values.get(&key).copied().unwrap_or(0.0);
            let t = ((vals[i] - vmin) / vrange).clamp(0.0, 1.0);
            vcols[i] = sample_palette(palette, t);
        }
        tri.vertex_colors = Some(vcols);
        let avg = (vals[0] + vals[1] + vals[2]) / 3.0;
        let t = ((avg - vmin) / vrange).clamp(0.0, 1.0);
        tri.color = Some(sample_palette(palette, t));
    }
}

/// Set all triangles to a uniform mid-palette color (for degenerate ranges).
fn apply_uniform_color(triangles: &mut [Triangle], palette: &[(u8, u8, u8)]) {
    let mid = sample_palette(palette, 0.5);
    for tri in triangles.iter_mut() {
        tri.color = Some(mid);
        tri.vertex_colors = Some([mid, mid, mid]);
    }
}

// ---------------------------------------------------------------------------
// Public color map functions
// ---------------------------------------------------------------------------

/// Overhang color mapping. Green for upward-facing, red for overhanging faces.
pub fn apply_overhang_map(triangles: &mut [Triangle], up: Vec3, threshold_deg: f64) {
    let up = up.normalized();
    let threshold_rad = threshold_deg.to_radians();

    for tri in triangles.iter_mut() {
        let n = tri.normal.normalized();
        let dot = n.dot(up);
        let angle = dot.clamp(-1.0, 1.0).acos();

        if angle <= threshold_rad {
            let t = angle / threshold_rad;
            tri.color = Some(lerp_color((0, 200, 0), (200, 200, 0), t));
        } else {
            let t = ((angle - threshold_rad) / (std::f64::consts::PI - threshold_rad)).min(1.0);
            tri.color = Some(lerp_color((200, 200, 0), (255, 0, 0), t));
        }
    }
}

/// Curvature-based color mapping. Maps angular difference between neighbor normals.
pub fn apply_curvature_map(triangles: &mut [Triangle], palette: &[(u8, u8, u8)], smooth_iterations: usize) {
    let pal = resolve_palette(palette);
    let normal_map = crate::smooth::build_vertex_normal_map(triangles);
    let adjacency = build_adjacency(triangles);

    // Compute curvature per vertex (average angle to neighbors)
    let mut curvature_map: FxHashMap<VertexKey, f64> = fx_hashmap_cap(normal_map.len());
    for (key, normal) in &normal_map {
        if let Some(neighbors) = adjacency.get(key) {
            let mut total_angle = 0.0;
            let mut count = 0;
            for nk in neighbors {
                if let Some(nn) = normal_map.get(nk) {
                    total_angle += normal.dot(*nn).clamp(-1.0, 1.0).acos();
                    count += 1;
                }
            }
            if count > 0 {
                curvature_map.insert(*key, total_angle / count as f64);
            }
        }
    }

    if smooth_iterations > 0 {
        smooth_values(&mut curvature_map, &adjacency, smooth_iterations);
    }

    match value_range(&curvature_map) {
        Some((vmin, vmax)) => apply_vertex_colors(triangles, &curvature_map, vmin, vmax - vmin, pal),
        None => apply_uniform_color(triangles, pal),
    }
}

/// Scalar function color mapping. Evaluates f(x,y,z) at each vertex.
pub fn apply_scalar_map(
    triangles: &mut [Triangle],
    function_str: &str,
    palette: &[(u8, u8, u8)],
    smooth_iterations: usize,
) -> Result<(), String> {
    let pal = resolve_palette(palette);
    let func = expr::parse(function_str)?;

    let mut vertex_values: FxHashMap<VertexKey, f64> = fx_hashmap_cap(triangles.len());
    for tri in triangles.iter() {
        for v in &tri.vertices {
            let key = quantize(*v);
            vertex_values.entry(key).or_insert_with(|| func(v.x, v.y, v.z));
        }
    }

    if smooth_iterations > 0 {
        let adjacency = build_adjacency(triangles);
        smooth_values(&mut vertex_values, &adjacency, smooth_iterations);
    }

    match value_range(&vertex_values) {
        Some((vmin, vmax)) => apply_vertex_colors(triangles, &vertex_values, vmin, vmax - vmin, pal),
        None => apply_uniform_color(triangles, pal),
    }

    Ok(())
}
