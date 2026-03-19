// ---------------------------------------------------------------------------
// Minimal number→string helpers (replaces core::fmt for SVG output)
// ---------------------------------------------------------------------------

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[inline]
pub(crate) fn push_u64(s: &mut String, mut v: u64) {
    if v == 0 { s.push('0'); return; }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while v > 0 { i -= 1; buf[i] = b'0' + (v % 10) as u8; v /= 10; }
    unsafe { s.push_str(std::str::from_utf8_unchecked(&buf[i..])); }
}

#[inline]
pub(crate) fn push_usize(s: &mut String, v: usize) { push_u64(s, v as u64); }

#[inline]
pub(crate) fn push_i32(s: &mut String, v: i32) {
    if v < 0 { s.push('-'); push_u64(s, (-(v as i64)) as u64); }
    else { push_u64(s, v as u64); }
}

/// Append f64 with N decimal places (N=1,2,4).
#[inline]
fn push_fn(s: &mut String, v: f64, decimals: u32) {
    if v.is_sign_negative() && v != 0.0 { s.push('-'); push_fn(s, -v, decimals); return; }
    let mul = 10u64.pow(decimals) as f64;
    let scaled = (v * mul).round() as u64;
    let int_part = scaled / mul as u64;
    let frac_part = scaled % mul as u64;
    push_u64(s, int_part);
    s.push('.');
    // Zero-pad fractional part
    let frac_digits = decimals as usize;
    let mut buf = [b'0'; 4];
    let mut f = frac_part;
    for j in (0..frac_digits).rev() { buf[j] = b'0' + (f % 10) as u8; f /= 10; }
    unsafe { s.push_str(std::str::from_utf8_unchecked(&buf[..frac_digits])); }
}

#[inline] pub(crate) fn push_f1(s: &mut String, v: f64) { push_fn(s, v, 1); }
#[inline] pub(crate) fn push_f2(s: &mut String, v: f64) { push_fn(s, v, 2); }
#[inline] pub(crate) fn push_f4(s: &mut String, v: f64) { push_fn(s, v, 4); }

#[inline]
pub(crate) fn push_hex(s: &mut String, v: u8) {
    s.push(HEX_DIGITS[(v >> 4) as usize] as char);
    s.push(HEX_DIGITS[(v & 0xF) as usize] as char);
}

#[inline]
pub(crate) fn push_hex_color(s: &mut String, r: u8, g: u8, b: u8) {
    s.push('#'); push_hex(s, r); push_hex(s, g); push_hex(s, b);
}

/// Write u64 into byte buffer, return bytes written.
#[inline]
fn write_u64(buf: &mut [u8], mut v: u64) -> usize {
    if v == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 20];
    let mut i = 20;
    while v > 0 { i -= 1; tmp[i] = b'0' + (v % 10) as u8; v /= 10; }
    let len = 20 - i;
    buf[..len].copy_from_slice(&tmp[i..]);
    len
}

/// Write f64 with 2 decimal places into byte buffer, return bytes written.
#[inline]
fn write_f2(buf: &mut [u8], v: f64) -> usize {
    let mut p = 0;
    if v.is_sign_negative() && v != 0.0 {
        buf[p] = b'-'; p += 1;
        return p + write_f2(&mut buf[p..], -v);
    }
    let scaled = (v * 100.0).round() as u64;
    let int_part = scaled / 100;
    let frac_part = scaled % 100;
    p += write_u64(&mut buf[p..], int_part);
    buf[p] = b'.'; p += 1;
    buf[p] = b'0' + (frac_part / 10) as u8; p += 1;
    buf[p] = b'0' + (frac_part % 10) as u8; p += 1;
    p
}

/// Append triangle points: "x0,y0 x1,y1 x2,y2" — single push_str.
#[inline]
pub(crate) fn push_tri_points(s: &mut String, pts: &[(f64, f64); 3]) {
    let mut buf = [0u8; 80];
    let mut p = 0;
    for i in 0..3 {
        if i > 0 { buf[p] = b' '; p += 1; }
        p += write_f2(&mut buf[p..], pts[i].0);
        buf[p] = b','; p += 1;
        p += write_f2(&mut buf[p..], pts[i].1);
    }
    unsafe { s.push_str(std::str::from_utf8_unchecked(&buf[..p])); }
}
