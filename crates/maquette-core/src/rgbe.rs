//! Radiance RGBE (.hdr) parser. Produces linear f32 RGB from HDR bytes.
//!
//! The format is very old (Radiance 1.4, ~1988) but is still the standard
//! interchange for HDR environments — Poly Haven and other libraries all ship
//! .hdr files. The parser here handles the common subset:
//!
//! * ASCII header (`#?RADIANCE` magic, `FORMAT=32-bit_rle_rgbe`, `-Y H +X W`)
//! * Both new (per-scanline RLE) and old (whole-image RLE / uncompressed) codings
//! * Big-endian scanline size
//!
//! We don't support flipped orientations (`+Y`, `-X`, etc.) — every .hdr in
//! the wild uses `-Y +X`. If we hit a rotated one, error and let the caller
//! reproject upstream.
//!
//! Output is linear-space RGB f32 (unbounded). One texel = `[R, G, B]`.
//! Storage is row-major.
//!
//! One texel decode:
//!   `(R, G, B) = (rm, gm, bm) · 2^(e − 128) / 255`.
//! Zero exponent = all-zero output.

/// Parse a .hdr file → (linear RGB f32 pixels, width, height).
pub fn parse(bytes: &[u8]) -> Result<(Vec<f32>, u32, u32), String> {
    // ------ Header -------------------------------------------------------
    // Header is ASCII terminated by a blank line, then a resolution line.
    let mut i = 0usize;
    // Optional Radiance magic.
    if !bytes.starts_with(b"#?RADIANCE") && !bytes.starts_with(b"#?RGBE") {
        return Err("HDR: not a Radiance file".into());
    }
    // Walk to the blank line separating header from resolution.
    while i < bytes.len() {
        let end = bytes[i..].iter().position(|&b| b == b'\n').ok_or("HDR: truncated header")?;
        let line = &bytes[i..i + end];
        i += end + 1;
        if line.is_empty() { break; }
        // FORMAT= line — only 32-bit_rle_rgbe is supported.
        if line.starts_with(b"FORMAT=") {
            let val = &line[7..];
            if val != b"32-bit_rle_rgbe" {
                return Err(format!("HDR: unsupported FORMAT ({})", String::from_utf8_lossy(val)));
            }
        }
    }
    // Resolution line: e.g. "-Y 512 +X 1024\n"
    let end = bytes[i..].iter().position(|&b| b == b'\n').ok_or("HDR: missing resolution")?;
    let res = std::str::from_utf8(&bytes[i..i + end]).map_err(|_| "HDR: non-ASCII resolution")?;
    i += end + 1;
    let (w, h) = parse_resolution(res)?;

    // ------ Scanlines ----------------------------------------------------
    let mut out = vec![0.0f32; (w * h * 3) as usize];
    let mut scanline = vec![0u8; (w * 4) as usize];
    for row in 0..h as usize {
        decode_scanline(&bytes, &mut i, &mut scanline, w)?;
        let dst_base = row * w as usize * 3;
        for x in 0..w as usize {
            let (r, g, b, e) = (
                scanline[x] as u32,
                scanline[w as usize + x] as u32,
                scanline[w as usize * 2 + x] as u32,
                scanline[w as usize * 3 + x] as u32,
            );
            let (fr, fg, fb) = rgbe_to_linear(r, g, b, e);
            let o = dst_base + x * 3;
            out[o]     = fr;
            out[o + 1] = fg;
            out[o + 2] = fb;
        }
    }
    Ok((out, w, h))
}

/// `-Y H +X W` — the canonical orientation. Anything else = error.
fn parse_resolution(res: &str) -> Result<(u32, u32), String> {
    let parts: Vec<&str> = res.split_whitespace().collect();
    if parts.len() != 4 || parts[0] != "-Y" || parts[2] != "+X" {
        return Err(format!("HDR: unsupported orientation ({})", res));
    }
    let h: u32 = parts[1].parse().map_err(|_| "HDR: bad height")?;
    let w: u32 = parts[3].parse().map_err(|_| "HDR: bad width")?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(format!("HDR: dims out of range ({}x{})", w, h));
    }
    Ok((w, h))
}

/// Decode one scanline into planar RGBE bytes: `[R0..Rw-1 | G0..Gw-1 | B0..Bw-1 | E0..Ew-1]`.
/// Handles both new-style adaptive RLE and old-style flat RLE.
fn decode_scanline(bytes: &[u8], i: &mut usize, out: &mut [u8], w: u32) -> Result<(), String> {
    if *i + 4 > bytes.len() { return Err("HDR: truncated scanline".into()); }
    // Sniff for new-style header: (0x02, 0x02, hi, lo).
    if w >= 8 && w <= 0x7fff && bytes[*i] == 2 && bytes[*i + 1] == 2 && (bytes[*i + 2] & 0x80) == 0 {
        let stated_w = ((bytes[*i + 2] as u32) << 8) | (bytes[*i + 3] as u32);
        if stated_w != w {
            return Err(format!("HDR: scanline width mismatch ({} vs {})", stated_w, w));
        }
        *i += 4;
        // Per-channel RLE. Runs: byte b > 128 = run of length (b − 128) of next byte;
        // b ≤ 128 = literal run of b bytes.
        for ch in 0..4 {
            let ch_base = ch * w as usize;
            let mut x = 0usize;
            while x < w as usize {
                if *i >= bytes.len() { return Err("HDR: RLE truncated".into()); }
                let b = bytes[*i]; *i += 1;
                if b > 128 {
                    if *i >= bytes.len() { return Err("HDR: RLE truncated".into()); }
                    let v = bytes[*i]; *i += 1;
                    let n = (b - 128) as usize;
                    if x + n > w as usize { return Err("HDR: RLE overflow".into()); }
                    out[ch_base + x..ch_base + x + n].fill(v);
                    x += n;
                } else {
                    let n = b as usize;
                    if n == 0 { return Err("HDR: RLE zero literal".into()); }
                    if x + n > w as usize { return Err("HDR: RLE overflow".into()); }
                    if *i + n > bytes.len() { return Err("HDR: RLE truncated".into()); }
                    out[ch_base + x..ch_base + x + n].copy_from_slice(&bytes[*i..*i + n]);
                    *i += n;
                    x += n;
                }
            }
        }
        return Ok(());
    }
    // Fall-back: interleaved RGBE with either old-style RLE (marker 0x01) or
    // uncompressed. Convert to planar as we decode.
    let mut x = 0usize;
    let mut prev = [0u8; 4];
    let mut rlcount: u32 = 0;
    while x < w as usize {
        if *i + 4 > bytes.len() { return Err("HDR: truncated pixels".into()); }
        let px = [bytes[*i], bytes[*i + 1], bytes[*i + 2], bytes[*i + 3]];
        *i += 4;
        if px[0] == 1 && px[1] == 1 && px[2] == 1 {
            // Old-style RLE: repeat previous pixel `px[3] << rlcount` times.
            let n = (px[3] as u32) << rlcount;
            for _ in 0..n {
                if x >= w as usize { return Err("HDR: old RLE overflow".into()); }
                out[x] = prev[0];
                out[w as usize + x] = prev[1];
                out[w as usize * 2 + x] = prev[2];
                out[w as usize * 3 + x] = prev[3];
                x += 1;
            }
            rlcount += 8;
        } else {
            out[x] = px[0];
            out[w as usize + x] = px[1];
            out[w as usize * 2 + x] = px[2];
            out[w as usize * 3 + x] = px[3];
            prev = px;
            x += 1;
            rlcount = 0;
        }
    }
    Ok(())
}

/// Standard Radiance decode: multiply mantissa by `2^(e − 128) / 255`.
#[inline]
fn rgbe_to_linear(r: u32, g: u32, b: u32, e: u32) -> (f32, f32, f32) {
    if e == 0 { return (0.0, 0.0, 0.0); }
    // 2^(e − 128 − 8). `−8` bakes in the `/256` that pairs with treating
    // mantissa as u8 (so we skip a divide per texel).
    let f = ldexpf(1.0, e as i32 - 128 - 8);
    (r as f32 * f, g as f32 * f, b as f32 * f)
}

/// `x · 2^n`. No libm on wasm; hand-code via the exponent bits.
#[inline]
fn ldexpf(x: f32, n: i32) -> f32 {
    // Clamp to safe range — normal f32 exponent bias is 127.
    let n = n.clamp(-125, 128);
    let bits = ((n + 127) as u32) << 23;
    x * f32::from_bits(bits)
}
