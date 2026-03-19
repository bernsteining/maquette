/// sRGB ↔ linear conversion and color utilities.

/// Convert sRGB u8 to linear f32 [0, 1]. Uses a 256-entry lookup table.
#[inline]
pub fn srgb_to_linear(v: u8) -> f32 {
    // Safety: single-threaded WASM, init_color_luts() called before any rendering.
    unsafe { SRGB_LUT[v as usize] }
}

/// Convert linear f32 [0, 1] to sRGB u8. Uses a 4096-entry lookup table.
#[inline]
pub fn linear_to_srgb(v: f32) -> u8 {
    let c = if v < 0.0 { 0.0f32 } else if v > 1.0 { 1.0f32 } else { v };
    // Safety: single-threaded WASM, init_color_luts() called before any rendering.
    unsafe { LINEAR_TO_SRGB_LUT[(c * 4095.0) as usize] }
}

static mut SRGB_LUT: [f32; 256] = [0.0; 256];
static mut LINEAR_TO_SRGB_LUT: [u8; 4096] = [0; 4096];

/// Must be called once before rendering. Populates both sRGB LUTs.
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

/// Parse a hex color string like "#4488cc" into (r, g, b).
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

/// Interpolate two colors by parameter t ∈ [0, 1].
#[inline]
pub fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    (
        (a.0 as f64 + t * (b.0 as f64 - a.0 as f64)).round() as u8,
        (a.1 as f64 + t * (b.1 as f64 - a.1 as f64)).round() as u8,
        (a.2 as f64 + t * (b.2 as f64 - a.2 as f64)).round() as u8,
    )
}
