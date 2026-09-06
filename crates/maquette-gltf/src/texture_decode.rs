//! Decode glTF image bytes into an RGBA8 pixel buffer.
//!
//! glTF images can arrive with a MIME type (`image/png`, `image/jpeg`) or
//! without one, in which case we sniff the magic bytes. All output is RGBA8
//! — sRGB→linear conversion happens later at sample time, so upstream code
//! can pick the right space per texture (base color is sRGB, MR/normal/AO are
//! linear).

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA8, row-major, width·height·4 bytes.
    pub rgba: Vec<u8>,
}

pub fn decode(bytes: &[u8], mime: Option<&str>) -> Result<DecodedImage, String> {
    let kind = match mime {
        Some("image/png")  => ImageKind::Png,
        Some("image/jpeg") => ImageKind::Jpeg,
        Some("image/webp") => ImageKind::Webp,
        Some(other)        => return Err(format!("unsupported image MIME type: {}", other)),
        None => sniff(bytes).ok_or_else(|| "unrecognised image format".to_string())?,
    };
    match kind {
        ImageKind::Png  => decode_png(bytes),
        ImageKind::Jpeg => decode_jpeg(bytes),
        ImageKind::Webp => decode_webp(bytes),
    }
}

enum ImageKind { Png, Jpeg, Webp }

fn sniff(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" { return Some(ImageKind::Png); }
    if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff"    { return Some(ImageKind::Jpeg); }
    // WebP magic: `RIFF????WEBP` — 4 bytes RIFF, 4 bytes size, 4 bytes "WEBP".
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(ImageKind::Webp);
    }
    None
}

fn decode_png(bytes: &[u8]) -> Result<DecodedImage, String> {
    use zune_png::PngDecoder;
    use zune_png::zune_core::options::DecoderOptions;
    use zune_png::zune_core::colorspace::ColorSpace;

    let opts = DecoderOptions::default().png_set_add_alpha_channel(true);
    let mut decoder = PngDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
    let pixels = decoder.decode_raw()
        .map_err(|e| format!("png decode: {:?}", e))?;
    let (width, height) = decoder.dimensions()
        .ok_or("png: no dimensions after decode")?;
    let colorspace = decoder.colorspace()
        .ok_or("png: no colorspace after decode")?;

    let rgba = match colorspace {
        ColorSpace::RGBA => pixels,
        ColorSpace::RGB  => expand_rgb_to_rgba(&pixels),
        ColorSpace::Luma => expand_gray_to_rgba(&pixels),
        ColorSpace::LumaA => expand_gray_alpha_to_rgba(&pixels),
        other => return Err(format!("png: unexpected colorspace {:?}", other)),
    };

    Ok(DecodedImage { width: width as u32, height: height as u32, rgba })
}

fn decode_jpeg(bytes: &[u8]) -> Result<DecodedImage, String> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::options::DecoderOptions;
    use zune_jpeg::zune_core::colorspace::ColorSpace;

    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
    let pixels = decoder.decode()
        .map_err(|e| format!("jpeg decode: {:?}", e))?;
    let info = decoder.info().ok_or("jpeg: no info after decode")?;
    let rgba = expand_rgb_to_rgba(&pixels);
    Ok(DecodedImage {
        width:  info.width as u32,
        height: info.height as u32,
        rgba,
    })
}

/// Decode `EXT_texture_webp` payloads. `image-webp` is pure-Rust, supports
/// lossy (VP8) and lossless (VP8L) variants and returns straight RGB / RGBA
/// depending on whether the image has an alpha channel. We normalise to
/// RGBA8 for the sampler.
fn decode_webp(bytes: &[u8]) -> Result<DecodedImage, String> {
    use image_webp::WebPDecoder;
    let mut decoder = WebPDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("webp decode init: {:?}", e))?;
    let (width, height) = decoder.dimensions();
    let has_alpha = decoder.has_alpha();
    let out_size = decoder.output_buffer_size()
        .ok_or("webp: image too large for output buffer")?;
    let mut pixels = vec![0u8; out_size];
    decoder.read_image(&mut pixels)
        .map_err(|e| format!("webp decode: {:?}", e))?;
    let rgba = if has_alpha { pixels } else { expand_rgb_to_rgba(&pixels) };
    Ok(DecodedImage { width, height, rgba })
}

fn expand_rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let n = rgb.len() / 3;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        out.push(rgb[i * 3]);
        out.push(rgb[i * 3 + 1]);
        out.push(rgb[i * 3 + 2]);
        out.push(255);
    }
    out
}

fn expand_gray_to_rgba(gray: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(gray.len() * 4);
    for &v in gray {
        out.push(v); out.push(v); out.push(v); out.push(255);
    }
    out
}

fn expand_gray_alpha_to_rgba(ga: &[u8]) -> Vec<u8> {
    let n = ga.len() / 2;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let v = ga[i * 2];
        out.push(v); out.push(v); out.push(v); out.push(ga[i * 2 + 1]);
    }
    out
}
