//! Format-agnostic rendering primitives shared by the maquette plugin family.
//!
//! Consumers (currently `maquette-gltf`; eventually the STL/OBJ/PLY `maquette`
//! plugin too) provide a scene representation and shader; this crate provides
//! the render primitives that stay the same regardless of asset format:
//!
//!   * [`math`] — Vec3, Mat3, Mat4, FxHasher.
//!   * [`color`] — sRGB LUTs and colour helpers.
//!   * [`rasterizer`] — triangle scan-conversion with 4-pixel SIMD interior,
//!     scalar remainder, per-vertex attribute interp, z-buffer + WBOIT.
//!   * [`shadow`] — per-light depth maps with PCF + PCSS.
//!   * [`ssao`] — screen-space ambient occlusion (bilateral-blurred).
//!   * [`fxaa`] — FXAA 3.11.
//!   * [`ibl`] — procedural / photographic HDR IBL env with cosine-weighted
//!     diffuse pre-convolution and seam-aware octahedral sampling.
//!   * [`rgbe`] — Radiance HDR (.hdr) parser.
//!   * [`texture`] — 2D texture with wrap/filter/mipmaps.
//!   * [`texture_decode`] — JPEG/PNG glue over zune-jpeg/zune-png.
//!
//! Nothing in this crate references glTF, STL, PLY, or OBJ — pure geometry
//! + shading primitives.

pub mod color;
pub mod fxaa;
pub mod ibl;
pub mod light;
pub mod math;
pub mod rasterizer;
pub mod rgbe;
pub mod shadow;
pub mod ssao;
pub mod texture;
pub mod texture_decode;
