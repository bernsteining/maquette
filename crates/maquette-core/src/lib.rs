//! Format-agnostic rendering primitives shared by the maquette plugin family.
//!
//! Consumers — `maquette-gltf` and the STL/OBJ/PLY `maquette` plugin —
//! provide a scene representation and shader; this crate provides the render
//! primitives that stay the same regardless of asset format:
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
//!   * [`texture`] — 2D texture with wrap/filter/mipmaps;
//!     [`texture_decode`] — JPEG/PNG/WebP decoding.
//!   * [`tonemap`] — ACES / Reinhard tone mapping.
//!   * [`effects`] — post-effect settings and the raw raster wire format.
//!   * [`cache`] — small keyed caches; [`prof`] — profiling marks;
//!     [`panic`] — panic capture for plugins.
//!
//! Nothing in this crate references glTF, STL, PLY, or OBJ — pure geometry
//! + shading primitives.

pub mod bundle;
pub mod cache;
pub mod panic;
pub mod prof;
pub mod color;
pub mod effects;
mod color_lut;
pub mod fxaa;
pub mod ibl;
pub mod light;
pub mod math;
pub mod rasterizer;
pub mod rgbe;
pub mod shadow; #[cfg(not(target_arch = "wasm32"))] pub mod simd;
pub mod ssao;
pub mod texture;
pub mod texture_decode;
pub mod tonemap;

/// Stand-ins for the host calls that `#[wasm_func]` expands to, so a plugin
/// crate also builds for native targets (CLI, bindings, tests). Invoke once
/// at the plugin's crate root.
#[macro_export]
macro_rules! native_protocol {
    () => {
        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn __write_args_to_buffer(_ptr: *mut u8) {}
        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn __send_result_to_host(_ptr: *const u8, _len: usize) {}
        #[cfg(not(target_arch = "wasm32"))]
        trait __ToResult {
            type Ok: ::core::convert::AsRef<[u8]>;
            type Err: ::core::fmt::Display;
            fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err>;
        }
        #[cfg(not(target_arch = "wasm32"))]
        impl __ToResult for Vec<u8> {
            type Ok = Self;
            type Err = ::core::convert::Infallible;
            fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        impl __ToResult for Box<[u8]> {
            type Ok = Self;
            type Err = ::core::convert::Infallible;
            fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        impl<'a> __ToResult for &'a [u8] {
            type Ok = Self;
            type Err = ::core::convert::Infallible;
            fn to_result(self) -> ::core::result::Result<Self::Ok, Self::Err> { Ok(self) }
        }
        #[cfg(not(target_arch = "wasm32"))]
        impl<T: ::core::convert::AsRef<[u8]>, E: ::core::fmt::Display> __ToResult
            for ::core::result::Result<T, E>
        {
            type Ok = T;
            type Err = E;
            fn to_result(self) -> Self { self }
        }
    };
}
