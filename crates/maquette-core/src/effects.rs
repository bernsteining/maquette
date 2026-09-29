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

/// Refuse a `width`×`height` output supersampled `factor`× per axis when it
/// needs more than `max_samples` samples, the most a plugin's buffers fit in
/// wasm32 memory, with an error the user can act on.
pub fn check_raster_size(width: usize, height: usize, factor: usize, max_samples: u64) -> Result<(), String> {
    let samples = width as u64 * height as u64 * (factor * factor) as u64;
    if samples <= max_samples {
        return Ok(());
    }
    let side = |f: u64| ((max_samples / (f * f)) as f64).sqrt() as u64;
    let ssaa = if factor > 1 { format!(" with {factor}× supersampling") } else { String::new() };
    Err(format!(
        "image too large: {width}×{height}{ssaa} is {:.0} M samples, the limit is {} M (about {s1}×{s1}, or {s2}×{s2} with 2× supersampling); lower width/height or antialias",
        samples as f64 / 1e6,
        max_samples / 1_000_000,
        s1 = side(1),
        s2 = side(2),
    ))
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

/// A decoded [`raw_raster`] blob.
pub struct RawRaster<'a> {
    pub tag: u8,
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    pub trailer: &'a [u8],
}

/// Decode a raster blob with tag `0x00` or `0x02`; `None` for any other
/// output (such as SVG, which starts with `<`).
pub fn parse_raw_raster(blob: &[u8]) -> Result<Option<RawRaster<'_>>, String> {
    let tag = *blob.first().ok_or("empty render output")?;
    if tag != 0x00 && tag != 0x02 { return Ok(None); }
    if blob.len() < 9 { return Err("truncated raster header".into()); }
    let width = u32::from_le_bytes(blob[1..5].try_into().unwrap());
    let height = u32::from_le_bytes(blob[5..9].try_into().unwrap());
    let end = 9 + width as usize * height as usize * 4;
    let rgba = blob.get(9..end).ok_or("truncated raster pixels")?;
    Ok(Some(RawRaster { tag, width, height, rgba, trailer: &blob[end..] }))
}

/// PNG file of a raster blob's pixels (any overlay trailer is dropped);
/// SVG output (`<`) passes through unchanged.
#[cfg(feature = "png")]
pub fn raw_raster_to_png(blob: &[u8]) -> Result<Vec<u8>, String> {
    let Some(r) = parse_raw_raster(blob)? else {
        return if blob[0] == b'<' { Ok(blob.to_vec()) } else { Err(format!("unexpected render marker 0x{:02x}", blob[0])) };
    };
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, r.width, r.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| format!("png: {e}"))?;
        writer.write_image_data(r.rgba).map_err(|e| format!("png: {e}"))?;
    }
    Ok(out)
}
