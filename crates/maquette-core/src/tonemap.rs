//! Tone mapping of linear HDR colour into `[0, 1]`, shared by every renderer.

use crate::math::FloatExt;
#[cfg(target_arch = "wasm32")] use std::arch::wasm32::*; #[cfg(not(target_arch = "wasm32"))] use crate::simd::*;

/// Tone-mapping operator. `None` = raw linear (clamped at gamma encode);
/// `Reinhard` = simple `x / (1 + x)`; `Aces` = ACES-fitted rational
/// approximation. Both non-None operators multiply by `exposure` first, so
/// they work like a virtual camera EV setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToneMap { None, Reinhard, Aces }

impl ToneMap {
    /// Operator for a config name: `"reinhard"`, `"aces"`, anything else none.
    pub fn parse(name: &str) -> Self {
        match name {
            "reinhard" => ToneMap::Reinhard,
            "aces" => ToneMap::Aces,
            _ => ToneMap::None,
        }
    }
}

#[inline]
pub fn tone_map(r: f32, g: f32, b: f32, method: ToneMap, exposure: f32) -> (f32, f32, f32) {
    if method == ToneMap::None { return (r, g, b); }
    let (r, g, b) = (r * exposure, g * exposure, b * exposure);
    match method {
        ToneMap::Reinhard => (r / (1.0 + r), g / (1.0 + g), b / (1.0 + b)),
        _ => {
            #[inline]
            fn aces(x: f32) -> f32 {
                let a = x * (2.51 * x + 0.03);
                let b = x * (2.43 * x + 0.59) + 0.14;
                (a / b).fmax(0.0).fmin(1.0)
            }
            (aces(r), aces(g), aces(b))
        }
    }
}

/// [`tone_map`] on four values of one channel; `exp4` is the splatted exposure.
#[inline(always)]
pub fn tone_map_4(v: v128, method: ToneMap, exp4: v128) -> v128 {
    match method {
        ToneMap::None => v,
        ToneMap::Reinhard => {
            let ve = f32x4_mul(v, exp4);
            f32x4_div(ve, f32x4_add(f32x4_splat(1.0), ve))
        }
        ToneMap::Aces => {
            let ve = f32x4_mul(v, exp4);
            let a = f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.51), ve), f32x4_splat(0.03)));
            let b = f32x4_add(f32x4_mul(ve, f32x4_add(f32x4_mul(f32x4_splat(2.43), ve), f32x4_splat(0.59))), f32x4_splat(0.14));
            f32x4_min(f32x4_max(f32x4_div(a, b), f32x4_splat(0.0)), f32x4_splat(1.0))
        }
    }
}
