#![allow(dead_code)] // srgb_to_linear + linear_rgb_to_srgb land with PBR/texture sampling in v2.

//! sRGB ↔ linear conversion and color utilities. Ported verbatim from maquette.

#[inline]
pub fn srgb_to_linear(v: u8) -> f32 {
    unsafe { SRGB_LUT[v as usize] }
}

/// sRGB→linear for a normalised f32 value in `[0, 1]`. Quantises through the
/// existing u8 LUT — good enough for texture samples (which came from u8
/// pixels quantised themselves) and avoids `powf` in the per-pixel shader.
#[inline]
pub fn srgb_to_linear_f01(v: f32) -> f32 {
    let idx = (v * 255.0 + 0.5).clamp(0.0, 255.0) as usize;
    unsafe { SRGB_LUT[idx] }
}

#[inline]
pub fn linear_to_srgb(v: f32) -> u8 {
    let c = if v < 0.0 { 0.0f32 } else if v > 1.0 { 1.0f32 } else { v };
    unsafe { LINEAR_TO_SRGB_LUT[(c * 4095.0) as usize] }
}

static mut SRGB_LUT: [f32; 256] = [0.0; 256];
static mut LINEAR_TO_SRGB_LUT: [u8; 4096] = [0; 4096];

pub fn init_color_luts() {
    static mut DONE: bool = false;
    unsafe {
        if DONE { return; }
        for i in 0..256u16 {
            let s = i as f32 / 255.0;
            SRGB_LUT[i as usize] = if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            };
        }
        for i in 0..4096u16 {
            let c = i as f32 / 4095.0;
            let s = if c <= 0.0031308 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            LINEAR_TO_SRGB_LUT[i as usize] = (s * 255.0 + 0.5) as u8;
        }
        DONE = true;
    }
}

pub fn parse_hex_color(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(128);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(128);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(128);
        (r, g, b)
    } else {
        (128, 128, 128)
    }
}

/// linear f32 [0,1] triple → sRGB u8 triple.
#[inline]
pub fn linear_rgb_to_srgb(r: f32, g: f32, b: f32) -> (u8, u8, u8) {
    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

/// Average three RGB colors (per-channel mean), used to collapse per-vertex
/// colors to a single flat face color.
#[inline]
pub fn avg3(a: (u8, u8, u8), b: (u8, u8, u8), c: (u8, u8, u8)) -> (u8, u8, u8) {
    (
        ((a.0 as u16 + b.0 as u16 + c.0 as u16) / 3) as u8,
        ((a.1 as u16 + b.1 as u16 + c.1 as u16) / 3) as u8,
        ((a.2 as u16 + b.2 as u16 + c.2 as u16) / 3) as u8,
    )
}

/// Interpolate two colors by parameter t ∈ [0, 1].
#[inline]
pub fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    (
        (a.0 as f64 + t * (b.0 as f64 - a.0 as f64)).round() as u8,
        (a.1 as f64 + t * (b.1 as f64 - a.1 as f64)).round() as u8,
        (a.2 as f64 + t * (b.2 as f64 - a.2 as f64)).round() as u8,
    )
}
