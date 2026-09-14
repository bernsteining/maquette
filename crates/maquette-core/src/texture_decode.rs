//! Decode image bytes (PNG / JPEG / WebP) into an RGBA8 pixel buffer.
//!
//! Shared by the maquette family: glTF base/MR/normal/occlusion/emissive maps
//! and OBJ/MTL `map_Kd` diffuse maps. Images can arrive with a MIME type
//! (`image/png`, `image/jpeg`, `image/webp`) or without one, in which case we
//! sniff the magic bytes. All output is RGBA8 — sRGB→linear conversion happens
//! later at sample time, so upstream code can pick the right space per texture
//! (base color is sRGB, MR/normal/AO are linear).

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

/// Decode an OBJ `map_Kd` texture: PNG, JPEG, TGA or BMP, chosen by magic bytes
/// with the `filename` extension as the tie-breaker for TGA (which has no
/// reliable header signature). Never calls `decode_webp`, so `image-webp`
/// (~200 KB of VP8/VP8L decoder) is dead-code-eliminated from this consumer's
/// wasm — OBJ textures are never WebP. TGA and BMP decode in-house (no
/// dependency), covering the legacy formats that still appear in OBJ/MTL.
pub fn decode_obj_texture(filename: &str, bytes: &[u8]) -> Result<DecodedImage, String> {
    match sniff(bytes) {
        Some(ImageKind::Png)  => return decode_png(bytes),
        Some(ImageKind::Jpeg) => return decode_jpeg(bytes),
        Some(ImageKind::Webp) =>
            return Err("WebP textures aren't supported for OBJ (use PNG, JPEG, TGA or BMP)".into()),
        None => {}
    }
    if looks_like_bmp(bytes) {
        return decode_bmp(bytes);
    }
    // TGA has no dependable magic; trust the extension (and fall through to it
    // for anything unrecognised so a mislabelled TGA still has a chance).
    let is_tga = filename.rsplit('.').next()
        .map(|e| e.eq_ignore_ascii_case("tga")).unwrap_or(false);
    if is_tga || looks_like_tga(bytes) {
        return decode_tga(bytes);
    }
    Err("unrecognised image format (OBJ textures support PNG, JPEG, TGA and BMP)".into())
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

/// Heuristic TGA check for when the extension is absent/misleading. TGA has no
/// header magic, but v2 files end with the `TRUEVISION-XFILE.\0` footer, and a
/// plausible header has a known colour-map-type + image-type and non-zero
/// dimensions. Conservative: only used as a fallback after PNG/JPEG sniffing.
fn looks_like_tga(b: &[u8]) -> bool {
    if b.len() >= 26 && &b[b.len() - 18..b.len() - 2] == b"TRUEVISION-XFILE" {
        return true;
    }
    if b.len() < 18 { return false; }
    let cmap_type = b[1];
    let img_type = b[2];
    let w = u16::from_le_bytes([b[12], b[13]]);
    let h = u16::from_le_bytes([b[14], b[15]]);
    (cmap_type == 0 || cmap_type == 1)
        && matches!(img_type, 1 | 2 | 3 | 9 | 10 | 11)
        && w != 0 && h != 0
}

/// BMP has a reliable `BM` magic; also require a known DIB-header size so a
/// stray `BM`-prefixed blob isn't mistaken for one.
fn looks_like_bmp(b: &[u8]) -> bool {
    b.len() >= 18 && &b[0..2] == b"BM"
        && matches!(u32::from_le_bytes([b[14], b[15], b[16], b[17]]), 12 | 40 | 52 | 56 | 108 | 124)
}

/// In-house Truevision TGA decoder — no dependency (TGA is a trivial header +
/// raw/RLE pixels, and it's the one texture format that shows up for OBJ but
/// not for glTF). Supports the formats that actually appear as textures:
///   - image type 2 / 10  — true-colour 24-bit (BGR) or 32-bit (BGRA), raw/RLE
///   - image type 3 / 11  — grayscale 8-bit, raw/RLE
/// Colour-mapped (1/9) and 16-bit true-colour are rare for textures and return
/// a clear error. Output is top-left-origin RGBA8; the image-descriptor origin
/// bit is honoured (bottom-left files are flipped).
fn decode_tga(b: &[u8]) -> Result<DecodedImage, String> {
    if b.len() < 18 { return Err("tga: header truncated".into()); }
    let id_len = b[0] as usize;
    let cmap_type = b[1];
    let img_type = b[2];
    let width = u16::from_le_bytes([b[12], b[13]]) as usize;
    let height = u16::from_le_bytes([b[14], b[15]]) as usize;
    let bpp = b[16];
    let descriptor = b[17];
    if cmap_type != 0 { return Err("tga: colour-mapped images unsupported (use true-colour or grayscale)".into()); }
    if width == 0 || height == 0 { return Err("tga: zero dimension".into()); }

    let (channels_in, is_gray, is_rle) = match img_type {
        2  => ((bpp / 8) as usize, false, false),
        10 => ((bpp / 8) as usize, false, true),
        3  => (1usize, true, false),
        11 => (1usize, true, true),
        1 | 9 => return Err("tga: colour-mapped images unsupported".into()),
        other => return Err(format!("tga: unsupported image type {}", other)),
    };
    if !is_gray && bpp != 24 && bpp != 32 {
        return Err(format!("tga: unsupported true-colour depth {} bpp (want 24 or 32)", bpp));
    }
    if is_gray && bpp != 8 {
        return Err(format!("tga: unsupported grayscale depth {} bpp (want 8)", bpp));
    }

    // Skip the ID field and (absent) colour map to reach pixel data.
    let mut pos = 18 + id_len;
    let npx = width * height;
    let mut rgba = vec![0u8; npx * 4];

    // Emit one source pixel (BGR/BGRA or gray) as RGBA into `rgba[i*4..]`.
    let put = |rgba: &mut [u8], i: usize, src: &[u8]| {
        if is_gray {
            let g = src[0];
            rgba[i * 4] = g; rgba[i * 4 + 1] = g; rgba[i * 4 + 2] = g; rgba[i * 4 + 3] = 255;
        } else {
            rgba[i * 4] = src[2];       // R ← B
            rgba[i * 4 + 1] = src[1];   // G
            rgba[i * 4 + 2] = src[0];   // B ← R
            rgba[i * 4 + 3] = if channels_in == 4 { src[3] } else { 255 };
        }
    };

    if is_rle {
        let mut i = 0usize;
        while i < npx {
            if pos >= b.len() { return Err("tga: RLE stream truncated (packet header)".into()); }
            let packet = b[pos]; pos += 1;
            let count = (packet & 0x7f) as usize + 1;
            if i + count > npx { return Err("tga: RLE run overruns image".into()); }
            if packet & 0x80 != 0 {
                // Run-length packet: one pixel repeated `count` times.
                if pos + channels_in > b.len() { return Err("tga: RLE run pixel truncated".into()); }
                let px = &b[pos..pos + channels_in];
                for k in 0..count { put(&mut rgba, i + k, px); }
                pos += channels_in;
            } else {
                // Raw packet: `count` literal pixels.
                if pos + count * channels_in > b.len() { return Err("tga: RLE raw run truncated".into()); }
                for k in 0..count {
                    put(&mut rgba, i + k, &b[pos + k * channels_in..pos + (k + 1) * channels_in]);
                }
                pos += count * channels_in;
            }
            i += count;
        }
    } else {
        if pos + npx * channels_in > b.len() { return Err("tga: pixel data truncated".into()); }
        for i in 0..npx {
            put(&mut rgba, i, &b[pos + i * channels_in..pos + (i + 1) * channels_in]);
        }
    }

    // Origin: descriptor bit 5 set = top-left (rows top→bottom); clear = the TGA
    // default bottom-left, which we flip to top-left to match the sampler.
    if descriptor & 0x20 == 0 {
        let stride = width * 4;
        for y in 0..height / 2 {
            let (top, bot) = (y * stride, (height - 1 - y) * stride);
            for x in 0..stride {
                rgba.swap(top + x, bot + x);
            }
        }
    }

    Ok(DecodedImage { width: width as u32, height: height as u32, rgba })
}

/// In-house uncompressed BMP decoder — no dependency, the other legacy format
/// (alongside TGA) that turns up in Windows-authored OBJ/MTL bundles. Supports
/// BITMAPINFOHEADER (40) and its V4/V5 supersets (108/124), which share field
/// offsets, in the forms that appear as textures:
///   - 24-bit BGR and 32-bit BGRX/BGRA, uncompressed (BI_RGB) — plus 32-bit
///     BI_BITFIELDS, treated as BGRX
///   - 8-bit palette-indexed (BI_RGB) with a BGRX colour table
/// RLE, 16/4/1-bit and the 12-byte OS/2 core header are rare for textures and
/// return a clear error. Output is top-left-origin RGBA8 (BMP rows are stored
/// bottom-up unless the height is negative); 32-bit alpha is forced opaque,
/// since BI_RGB leaves that byte undefined and many writers zero it.
fn decode_bmp(b: &[u8]) -> Result<DecodedImage, String> {
    if b.len() < 54 { return Err("bmp: header truncated".into()); }
    if &b[0..2] != b"BM" { return Err("bmp: bad magic".into()); }
    let data_offset = u32::from_le_bytes([b[10], b[11], b[12], b[13]]) as usize;
    let dib_size = u32::from_le_bytes([b[14], b[15], b[16], b[17]]) as usize;
    if dib_size < 40 {
        return Err("bmp: unsupported DIB header (need BITMAPINFOHEADER or newer)".into());
    }
    let width = i32::from_le_bytes([b[18], b[19], b[20], b[21]]);
    let height_raw = i32::from_le_bytes([b[22], b[23], b[24], b[25]]);
    let bpp = u16::from_le_bytes([b[28], b[29]]);
    let compression = u32::from_le_bytes([b[30], b[31], b[32], b[33]]);
    if width <= 0 || height_raw == 0 { return Err("bmp: bad dimensions".into()); }
    let top_down = height_raw < 0;
    let width = width as usize;
    let height = height_raw.unsigned_abs() as usize;

    // BI_RGB (0) everywhere; BI_BITFIELDS (3) accepted only for 32-bit (masks
    // are the standard BGRA layout in practice — we take BGR and force alpha).
    if compression != 0 && !(compression == 3 && bpp == 32) {
        return Err(format!("bmp: unsupported compression {} (only uncompressed)", compression));
    }

    // Rows are padded to a 4-byte boundary.
    let row_stride = ((bpp as usize * width + 31) / 32) * 4;
    if data_offset.checked_add(row_stride * height).map_or(true, |e| e > b.len()) {
        return Err("bmp: pixel data truncated".into());
    }
    let mut rgba = vec![0u8; width * height * 4];
    let dst_row = |y: usize| if top_down { y } else { height - 1 - y };

    match bpp {
        24 | 32 => {
            let bytespp = (bpp / 8) as usize;
            for y in 0..height {
                let src = data_offset + y * row_stride;
                let dy = dst_row(y);
                for x in 0..width {
                    let s = src + x * bytespp;
                    let di = (dy * width + x) * 4;
                    rgba[di]     = b[s + 2]; // R ← B
                    rgba[di + 1] = b[s + 1]; // G
                    rgba[di + 2] = b[s];     // B ← R
                    rgba[di + 3] = 255;      // opaque (BI_RGB alpha byte undefined)
                }
            }
        }
        8 => {
            // Palette (BGRX, 4 bytes/entry) sits between the DIB header and pixels.
            let colors_used = u32::from_le_bytes([b[46], b[47], b[48], b[49]]) as usize;
            let ncol = if colors_used == 0 { 256 } else { colors_used };
            let pal = 14 + dib_size;
            if pal + ncol * 4 > b.len() { return Err("bmp: palette truncated".into()); }
            for y in 0..height {
                let src = data_offset + y * row_stride;
                let dy = dst_row(y);
                for x in 0..width {
                    let idx = b[src + x] as usize;
                    if idx >= ncol { return Err("bmp: palette index out of range".into()); }
                    let p = pal + idx * 4;
                    let di = (dy * width + x) * 4;
                    rgba[di]     = b[p + 2]; // R ← palette B
                    rgba[di + 1] = b[p + 1];
                    rgba[di + 2] = b[p];
                    rgba[di + 3] = 255;
                }
            }
        }
        other => return Err(format!("bmp: unsupported {} bpp (want 8, 24 or 32)", other)),
    }
    Ok(DecodedImage { width: width as u32, height: height as u32, rgba })
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
