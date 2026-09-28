//! Screen-space post-processing settings shared by every renderer, and the
//! raw raster wire format they all emit.

use crate::ssao::SSAOParams;

/// Effects applied to a resolved, output-resolution frame by
/// [`PixelBuffer::apply_post`](crate::rasterizer::PixelBuffer::apply_post),
/// in this order: ambient occlusion, depth fog, bloom, glow, sharpening, FXAA.
#[derive(Clone, Default)]
pub struct PostEffects {
    pub ssao: Option<SSAOParams>,
    pub fog: Option<Fog>,
    pub bloom: Option<Bloom>,
    pub glow: Option<Glow>,
    /// Unsharp-mask strength.
    pub sharpen: Option<f32>,
    pub fxaa: bool,
}

impl PostEffects {
    /// Whether any effect reads the depth buffer.
    pub fn needs_depth(&self) -> bool {
        self.ssao.is_some() || self.fog.is_some() || self.glow.is_some()
    }
}

/// Depth cueing: covered pixels blend toward `color` by
/// `smoothstep(near, far, distance)`, distance measured along the view axis.
#[derive(Clone, Copy)]
pub struct Fog {
    pub near: f32,
    pub far: f32,
    pub color: (u8, u8, u8),
}

impl Fog {
    /// Mol*'s fog range for a model of bounding radius `radius` viewed from
    /// `camera_distance`: the fade starts between the sphere's back
    /// (`intensity` 0) and front (100), midway at 50, and completes at its back.
    pub fn molstar(camera_distance: f64, radius: f64, intensity: f64, color: (u8, u8, u8)) -> Self {
        let near = camera_distance + radius * (50.0 - intensity) / 50.0;
        Fog { near: near as f32, far: (camera_distance + radius) as f32, color }
    }
}

/// Bright-pass bloom.
#[derive(Clone, Copy)]
pub struct Bloom {
    pub threshold: f32,
    pub intensity: f32,
    pub radius: usize,
}

/// Coloured halo around everything rendered.
#[derive(Clone, Copy)]
pub struct Glow {
    pub color: (u8, u8, u8),
    pub intensity: f32,
    pub radius: usize,
}

/// Raw raster blob: `[tag][width u32 LE][height u32 LE][rgba8…][trailer…]`.
/// Tag `0x00` is a plain image; plugins may define other tags for images
/// followed by a trailer (such as an SVG overlay).
pub fn raw_raster(tag: u8, width: u32, height: u32, rgba: &[u8], trailer: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + rgba.len() + trailer.len());
    out.push(tag);
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(rgba);
    out.extend_from_slice(trailer);
    out
}
