#![allow(dead_code)] // FxHash + transform_vector are kept for v2 (vertex de-dup, normal transforms).

//! Minimal 3D vector math and utilities — no external dependency needed.
//!
//! Ported from maquette (format-agnostic); the `build_adjacency` helper that
//! depended on maquette's `Triangle` type is dropped — glTF meshes come with
//! explicit index buffers, so we don't need to rediscover topology.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// FxHash — fast, non-cryptographic hash for integer-like keys.
pub struct FxHasher(u64);

const SEED: u64 = 0x517cc1b727220a95;

impl Default for FxHasher {
    fn default() -> Self { Self(0) }
}

impl Hasher for FxHasher {
    #[inline]
    fn finish(&self) -> u64 { self.0 }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(SEED);
        }
    }
    #[inline]
    fn write_u64(&mut self, i: u64) { self.0 = (self.0 ^ i).wrapping_mul(SEED); }
    #[inline]
    fn write_i64(&mut self, i: i64) { self.write_u64(i as u64); }
    #[inline]
    fn write_u32(&mut self, i: u32) { self.write_u64(i as u64); }
    #[inline]
    fn write_usize(&mut self, i: usize) { self.write_u64(i as u64); }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

#[derive(Clone, Copy, Debug)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    #[inline(always)]
    pub const fn new(x: f64, y: f64, z: f64) -> Self { Self { x, y, z } }

    #[inline(always)]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline(always)]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    #[inline(always)]
    pub fn sub(self, other: Self) -> Self {
        Self { x: self.x - other.x, y: self.y - other.y, z: self.z - other.z }
    }

    #[inline(always)]
    pub fn add(self, other: Self) -> Self {
        Self { x: self.x + other.x, y: self.y + other.y, z: self.z + other.z }
    }

    #[inline(always)]
    pub fn scale(self, s: f64) -> Self {
        Self { x: self.x * s, y: self.y * s, z: self.z * s }
    }

    #[inline(always)]
    pub fn length(self) -> f64 { self.dot(self).sqrt() }

    #[inline(always)]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len < 1e-12 { Self::new(0.0, 0.0, 0.0) } else { self.scale(1.0 / len) }
    }

    #[inline]
    pub fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Option<Vec3> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-12 { None } else { Some(n.scale(1.0 / len)) }
    }
}

impl From<[f64; 3]> for Vec3 {
    #[inline]
    fn from(a: [f64; 3]) -> Self { Vec3::new(a[0], a[1], a[2]) }
}

impl From<[f32; 3]> for Vec3 {
    #[inline]
    fn from(a: [f32; 3]) -> Self { Vec3::new(a[0] as f64, a[1] as f64, a[2] as f64) }
}

impl std::ops::Sub for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn sub(self, rhs: Vec3) -> Vec3 { Vec3::sub(self, rhs) }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    #[inline(always)]
    fn add(self, rhs: Vec3) -> Vec3 { Vec3::add(self, rhs) }
}

/// 3×3 matrix stored as [row][col]. Used for normal transforms, where the
/// inverse-transpose of the world 3×3 handles non-uniform scale correctly.
#[derive(Clone, Copy, Debug)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    #[inline]
    pub fn identity() -> Self {
        Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    }

    #[inline(always)]
    pub fn transform_vector(self, v: Vec3) -> Vec3 {
        let m = self.0;
        Vec3 {
            x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        }
    }
}

/// 4x4 matrix stored as [row][col].
#[derive(Clone, Copy, Debug)]
pub struct Mat4(pub [[f64; 4]; 4]);

impl Mat4 {
    #[inline]
    pub fn identity() -> Self {
        Mat4([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// Look-at view matrix (world → camera space).
    #[inline]
    pub fn look_at(camera: Vec3, center: Vec3, up: Vec3) -> Self {
        let f = (center - camera).normalized();
        let r = f.cross(up).normalized();
        let u = r.cross(f);
        Mat4([
            [r.x, r.y, r.z, -r.dot(camera)],
            [u.x, u.y, u.z, -u.dot(camera)],
            [-f.x, -f.y, -f.z, f.dot(camera)],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    #[inline(always)]
    pub fn transform_point(self, p: Vec3) -> Vec3 {
        let m = self.0;
        Vec3 {
            x: m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
            y: m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
            z: m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
        }
    }

    /// Transform a direction (ignores translation column).
    #[inline(always)]
    pub fn transform_vector(self, v: Vec3) -> Vec3 {
        let m = self.0;
        Vec3 {
            x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        }
    }

    /// Row-major 4×4 matrix multiply: `self * rhs`.
    #[inline]
    pub fn mul(self, rhs: Mat4) -> Mat4 {
        let a = self.0;
        let b = rhs.0;
        let mut out = [[0.0f64; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                out[i][j] = a[i][0] * b[0][j]
                    + a[i][1] * b[1][j]
                    + a[i][2] * b[2][j]
                    + a[i][3] * b[3][j];
            }
        }
        Mat4(out)
    }

    /// Build a matrix from a glTF column-major 4×4 array.
    #[inline]
    pub fn from_gltf_column_major(m: [[f32; 4]; 4]) -> Mat4 {
        Mat4([
            [m[0][0] as f64, m[1][0] as f64, m[2][0] as f64, m[3][0] as f64],
            [m[0][1] as f64, m[1][1] as f64, m[2][1] as f64, m[3][1] as f64],
            [m[0][2] as f64, m[1][2] as f64, m[2][2] as f64, m[3][2] as f64],
            [m[0][3] as f64, m[1][3] as f64, m[2][3] as f64, m[3][3] as f64],
        ])
    }

    /// Inverse-transpose of the upper-left 3×3, for transforming normals under
    /// non-uniform scale. Returns identity when the matrix is (near-)singular
    /// — safe fallback for degenerate transforms.
    ///
    /// glTF spec: normals are transformed by the inverse-transpose of the
    /// world matrix's upper-left 3×3. For rigid + uniform-scale transforms
    /// this equals the original 3×3 (so the whole thing is a no-op); it only
    /// matters when the scene has non-uniform scale, which the wild does hit.
    pub fn normal_matrix_3x3(&self) -> Mat3 {
        let m = self.0;
        let a = m[0][0]; let b = m[0][1]; let c = m[0][2];
        let d = m[1][0]; let e = m[1][1]; let f = m[1][2];
        let g = m[2][0]; let h = m[2][1]; let i = m[2][2];
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if det.abs() < 1e-12 {
            return Mat3::identity();
        }
        let inv_det = 1.0 / det;
        // Cofactor matrix; the transpose of the inverse equals the cofactor
        // matrix divided by the determinant.
        Mat3([
            [ (e * i - f * h) * inv_det, -(d * i - f * g) * inv_det,  (d * h - e * g) * inv_det],
            [-(b * i - c * h) * inv_det,  (a * i - c * g) * inv_det, -(a * h - b * g) * inv_det],
            [ (b * f - c * e) * inv_det, -(a * f - c * d) * inv_det,  (a * e - b * d) * inv_det],
        ])
    }

    /// TRS decomposition: translation × rotation-from-quaternion × scale.
    /// Quaternion in glTF order [x, y, z, w].
    pub fn from_trs(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> Mat4 {
        let (qx, qy, qz, qw) = (r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64);
        let (sx, sy, sz) = (s[0] as f64, s[1] as f64, s[2] as f64);

        let xx = qx * qx; let yy = qy * qy; let zz = qz * qz;
        let xy = qx * qy; let xz = qx * qz; let yz = qy * qz;
        let wx = qw * qx; let wy = qw * qy; let wz = qw * qz;

        Mat4([
            [sx * (1.0 - 2.0 * (yy + zz)), sy * (2.0 * (xy - wz)),       sz * (2.0 * (xz + wy)),       t[0] as f64],
            [sx * (2.0 * (xy + wz)),       sy * (1.0 - 2.0 * (xx + zz)), sz * (2.0 * (yz - wx)),       t[1] as f64],
            [sx * (2.0 * (xz - wy)),       sy * (2.0 * (yz + wx)),       sz * (1.0 - 2.0 * (xx + yy)), t[2] as f64],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }
}
