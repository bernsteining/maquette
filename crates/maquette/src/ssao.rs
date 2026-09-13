//! Mesh-side SSAO kernel.
//!
//! The shared pieces — `SSAOParams`, `SampleOffset`, and the SIMD separable
//! bilateral blur — live in `maquette-core` and are re-exported here so the
//! rasterizer's `crate::ssao::…` paths are unchanged. Only the kernel
//! precompute stays local: the mesh renderer keeps the **16-rotation tiled**
//! kernel (indexed per pixel by `(y & 3) * 4 + (x & 3)` in the rasterizer),
//! which reads cleaner than the 256-hash variant on maquette's *static* output
//! — the hash's advantage (no shimmering pattern) only pays off in motion/TAA.

pub use maquette_core::ssao::{bilateral_blur_separable, SSAOParams, SampleOffset};

/// Pre-compute all sample offsets for 16 noise rotations x N kernel samples.
/// Returns Vec of 16 Vecs, indexed by noise pattern `(y & 3) * 4 + (x & 3)`.
/// Folds kernel generation, noise rotation, radius scaling, and int rounding
/// into a single precomputation so the per-pixel loop needs only integer adds
/// and a depth comparison.
pub fn precompute_sample_offsets(
    samples: usize,
    radius_px: f32,
    bias_scaled: f32,
) -> Vec<Vec<SampleOffset>> {
    use std::f64::consts::{PI, TAU};

    // Generate hemisphere kernel (same math as before, stored as tuples)
    let mut kernel = Vec::with_capacity(samples);
    for i in 0..samples {
        let u = (i as f64 + 0.5) / samples as f64;
        let angle = 2.0 * PI * u;
        let r = ((i + 1) as f64 / samples as f64).sqrt();
        let x = angle.cos() * r;
        let y = angle.sin() * r;
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        let scale = (i as f64 / samples as f64).powi(2) * 0.9 + 0.1;
        kernel.push(((x * scale) as f32, (y * scale) as f32, (z * scale) as f32));
    }

    // 16 noise rotations (4x4 tiled pattern)
    const PERM: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];

    let mut offsets = Vec::with_capacity(16);
    for n in 0..16 {
        let angle = (PERM[n] as f64 / 16.0) * TAU;
        let (sin_a, cos_a) = angle.sin_cos();
        let cos_f = cos_a as f32;
        let sin_f = sin_a as f32;

        let mut pattern = Vec::with_capacity(samples);
        for &(kx, ky, kz) in &kernel {
            let rx = kx * cos_f - ky * sin_f;
            let ry = kx * sin_f + ky * cos_f;
            pattern.push(SampleOffset {
                dx: (rx * radius_px + 0.5) as i32,
                dy: (ry * radius_px + 0.5) as i32,
                z_bias: kz * bias_scaled,
            });
        }
        offsets.push(pattern);
    }
    offsets
}
