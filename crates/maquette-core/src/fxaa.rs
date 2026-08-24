/// FXAA 3.11 post-process filter (Timothy Lottes, NVIDIA).
///
/// Two-component: edge walk + sub-pixel blend.
/// SIMD batch early-exit (16 pixels) + all-integer scalar processing + LUT.
/// Two-row ring buffer instead of full pixel buffer clone.

use std::arch::wasm32::*;

const EDGE_MIN: u8 = 8;

/// Variable step sizes: 6 iterations covering 18 pixels (vs 10 iterations for 10 pixels).
const WALK_STEPS: [i32; 6] = [1, 1, 2, 2, 4, 8];

/// Precomputed sub-pixel blend: smoothstep(x/255)^2 * 0.75, scaled to 0-96 (128 = 1.0).
const SUBPIX_LUT: [u8; 256] = {
    let mut lut = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let x = i as f64 / 255.0;
        let ss = x * x * (3.0 - 2.0 * x);
        lut[i] = (ss * ss * 96.0 + 0.5) as u8;
        i += 1;
    }
    lut
};

/// Compute per-pixel luma buffer. SIMD: 4 pixels per iteration.
/// Rec. 601 fixed-point: luma = (77*R + 150*G + 29*B) >> 8.
fn compute_luma(pixels: &[u8], n: usize) -> Vec<u8> {
    let mut luma = vec![0u8; n];

    unsafe {
        let coeff_r = i16x8_splat(77);
        let coeff_g = i16x8_splat(150);
        let coeff_b = i16x8_splat(29);

        let mut i = 0usize;
        while i * 3 + 16 <= pixels.len() {
            let si = i * 3;
            let v = v128_load(pixels.as_ptr().add(si) as *const v128);

            let deint = i8x16_shuffle::<
                0, 3, 6, 9, 1, 4, 7, 10, 2, 5, 8, 11, 0, 0, 0, 0
            >(v, v);

            let rg = u16x8_extend_low_u8x16(deint);
            let b = u16x8_extend_high_u8x16(deint);

            let g_shifted = i8x16_shuffle::<
                8, 9, 10, 11, 12, 13, 14, 15, 0, 1, 0, 1, 0, 1, 0, 1
            >(rg, rg);

            let wr = i16x8_mul(rg, coeff_r);
            let wg = i16x8_mul(g_shifted, coeff_g);
            let wb = i16x8_mul(b, coeff_b);
            let sum = i16x8_add(i16x8_add(wr, wg), wb);
            let luma_u16 = u16x8_shr(sum, 8);

            let luma_u8 = u8x16_narrow_i16x8(luma_u16, i16x8_splat(0));
            let out_ptr = luma.as_mut_ptr().add(i);
            (out_ptr as *mut u32).write_unaligned(i32x4_extract_lane::<0>(luma_u8) as u32);

            i += 4;
        }

        while i < n {
            let si = i * 3;
            *luma.get_unchecked_mut(i) = ((*pixels.get_unchecked(si) as u16 * 77
                      + *pixels.get_unchecked(si + 1) as u16 * 150
                      + *pixels.get_unchecked(si + 2) as u16 * 29) >> 8) as u8;
            i += 1;
        }
    }

    luma
}

/// Process a single edge pixel. All arithmetic is integer (i16/u32).
/// Reads neighbor RGB from row buffers (prev_row/curr_row) when the neighbor
/// was already processed, or directly from pixels when not yet processed.
#[inline(always)]
fn fxaa_pixel(idx: usize, x: usize, y: usize, w: usize, h: usize,
              luma: &[u8], pixels: &mut [u8],
              prev_row: &[u8], curr_row: &[u8]) {
    unsafe {
    let m = *luma.get_unchecked(idx) as i16;
    let ln = *luma.get_unchecked(idx - w) as i16;
    let ls = *luma.get_unchecked(idx + w) as i16;
    let lw = *luma.get_unchecked(idx - 1) as i16;
    let le = *luma.get_unchecked(idx + 1) as i16;

    let mx = m.max(ln).max(ls).max(lw).max(le);
    let range = mx - m.min(ln).min(ls).min(lw).min(le);

    let lnw = *luma.get_unchecked(idx - w - 1) as i16;
    let lne = *luma.get_unchecked(idx - w + 1) as i16;
    let lsw = *luma.get_unchecked(idx + w - 1) as i16;
    let lse = *luma.get_unchecked(idx + w + 1) as i16;

    // Edge direction: vertical 2nd derivs → horizontal edge
    let eh = (ln + ls - 2 * m).abs() * 2
           + (lnw + lsw - 2 * lw).abs()
           + (lne + lse - 2 * le).abs();
    let ev = (lw + le - 2 * m).abs() * 2
           + (lnw + lne - 2 * ln).abs()
           + (lsw + lse - 2 * ls).abs();
    let horiz = eh >= ev;

    // Perpendicular gradient
    let (l_neg, l_pos) = if horiz { (ln, ls) } else { (lw, le) };
    let g_neg = (l_neg - m).abs();
    let g_pos = (l_pos - m).abs();
    let neg_side = g_neg >= g_pos;

    // Edge boundary luma ×2 (avoids /2), gradient threshold = g/2 (×2 scale)
    let l_edge2 = if neg_side { l_neg + m } else { l_pos + m };
    let g_thr = ((if neg_side { g_neg } else { g_pos }) + 1) >> 1;

    // Walk offsets as linear indices
    let wi = w as i32;
    let along = if horiz { 1i32 } else { wi };
    let perp = if horiz {
        if neg_side { -wi } else { wi }
    } else {
        if neg_side { -1i32 } else { 1i32 }
    };

    // Max safe walk distance
    let (max_n, max_p) = if horiz {
        (x as i32, w as i32 - 1 - x as i32)
    } else {
        (y as i32, h as i32 - 1 - y as i32)
    };

    // Edge walk with variable step sizes
    let start = idx as i32;
    let mut dn = 1i32;
    let mut dp = 1i32;
    let mut en = 0i16;
    let mut ep = 0i16;
    let mut done_n = false;
    let mut done_p = false;
    let mut cum_n = 0i32;
    let mut cum_p = 0i32;

    for &step in &WALK_STEPS {
        if !done_n {
            cum_n += step;
            if cum_n > max_n {
                done_n = true;
            } else {
                let pos = (start - along * cum_n) as usize;
                en = *luma.get_unchecked(pos) as i16 + *luma.get_unchecked((pos as i32 + perp) as usize) as i16 - l_edge2;
                done_n = en.abs() >= g_thr;
                dn = cum_n;
            }
        }
        if !done_p {
            cum_p += step;
            if cum_p > max_p {
                done_p = true;
            } else {
                let pos = (start + along * cum_p) as usize;
                ep = *luma.get_unchecked(pos) as i16 + *luma.get_unchecked((pos as i32 + perp) as usize) as i16 - l_edge2;
                done_p = ep.abs() >= g_thr;
                dp = cum_p;
            }
        }
        if done_n && done_p { break; }
    }

    // Edge blend: (0.5 - min/span) scaled to 0-64 (in 0-128 space)
    let span = dn + dp;
    let dmin = dn.min(dp);
    let e_raw = ((span - 2 * dmin) as u32 * 64 / span as u32) as u16;

    // Good span check
    let closer = if dn < dp { en } else { ep };
    let m_side = 2 * m - l_edge2;
    let good = (closer < 0) != (m_side < 0);
    let e_blend = if good { e_raw } else { 0 };

    // Sub-pixel blend via LUT
    let avg12 = 2 * (ln + ls + lw + le) + lnw + lne + lsw + lse;
    let sub_num = (avg12 - 12 * m).unsigned_abs() as u32;
    let sub_den = (12 * range) as u32;
    let sub_idx = ((sub_num * 255 + sub_den / 2) / sub_den).min(255) as usize;
    let s_blend = SUBPIX_LUT[sub_idx] as u16;

    let blend = e_blend.max(s_blend);
    if blend == 0 { return; }

    // Read neighbor RGB from the correct source:
    // neg_side = true → neighbor already processed → saved row buffer
    // neg_side = false → neighbor not yet processed → pixels directly
    let (nr, ng, nb) = if neg_side {
        if horiz {
            let o = x * 3; // north neighbor, same x, row y-1
            (*prev_row.get_unchecked(o), *prev_row.get_unchecked(o + 1), *prev_row.get_unchecked(o + 2))
        } else {
            let o = (x - 1) * 3; // west neighbor, x-1, same row
            (*curr_row.get_unchecked(o), *curr_row.get_unchecked(o + 1), *curr_row.get_unchecked(o + 2))
        }
    } else {
        let n3 = (idx as i32 + perp) as usize * 3;
        (*pixels.get_unchecked(n3), *pixels.get_unchecked(n3 + 1), *pixels.get_unchecked(n3 + 2))
    };

    // Center pixel not yet modified — read directly, then write blend
    let p = pixels.as_mut_ptr().add(idx * 3);
    let iv = 128 - blend;
    *p     = ((*p as u16 * iv + nr as u16 * blend + 64) >> 7) as u8;
    *p.add(1) = ((*p.add(1) as u16 * iv + ng as u16 * blend + 64) >> 7) as u8;
    *p.add(2) = ((*p.add(2) as u16 * iv + nb as u16 * blend + 64) >> 7) as u8;
    } // unsafe
}

/// Apply FXAA 3.11 to an RGB pixel buffer in-place.
pub fn apply_fxaa(pixels: &mut [u8], width: usize, height: usize) {
    let n = width * height;
    let luma = compute_luma(pixels, n);

    let w = width;
    let h = height;
    let row_bytes = w * 3;

    // Two-row ring buffer: original RGB for already-processed rows
    // prev_row: originals of row y-1, curr_row: originals of row y
    let mut prev_row = vec![0u8; row_bytes];
    let mut curr_row = vec![0u8; row_bytes];

    // Row 0 is never modified (loop starts at y=1), save for north neighbor reads
    prev_row.copy_from_slice(&pixels[0..row_bytes]);

    for y in 1..h - 1 {
        let row_start = y * row_bytes;
        let row = y * w;
        let mut x = 1usize;
        let mut row_saved = false;

        // SIMD batch: check 16 pixels at once with both thresholds
        unsafe {
            let edge_thr = u8x16_splat(EDGE_MIN - 1);
            let zero = u8x16_splat(0);
            let lp = luma.as_ptr();

            while x + 17 <= w {
                let idx = row + x;

                let c = v128_load(lp.add(idx) as *const v128);
                let north = v128_load(lp.add(idx - w) as *const v128);
                let south = v128_load(lp.add(idx + w) as *const v128);
                let west = v128_load(lp.add(idx - 1) as *const v128);
                let east = v128_load(lp.add(idx + 1) as *const v128);

                let vmin = u8x16_min(u8x16_min(c, north), u8x16_min(south, u8x16_min(west, east)));
                let vmax = u8x16_max(u8x16_max(c, north), u8x16_max(south, u8x16_max(west, east)));
                let range = u8x16_sub(vmax, vmin);

                // Absolute threshold: range >= EDGE_MIN
                let above_abs = u8x16_sub_sat(range, edge_thr);
                if !v128_any_true(above_abs) {
                    x += 16;
                    continue;
                }

                // Relative threshold: range >= vmax >> 3
                let thr_lo = u16x8_shr(u16x8_extend_low_u8x16(vmax), 3);
                let thr_hi = u16x8_shr(u16x8_extend_high_u8x16(vmax), 3);
                let thr_rel = u8x16_narrow_i16x8(thr_lo, thr_hi);
                let below_rel = u8x16_sub_sat(thr_rel, range);
                let pass_rel = i8x16_eq(below_rel, zero);

                let pass_abs = v128_not(i8x16_eq(above_abs, zero));
                let is_edge = v128_and(pass_abs, pass_rel);

                if !v128_any_true(is_edge) {
                    x += 16;
                    continue;
                }

                // Lazy row copy: only save originals when first edge is found
                if !row_saved {
                    curr_row.copy_from_slice(&pixels[row_start..row_start + row_bytes]);
                    row_saved = true;
                }

                let mut mask = i8x16_bitmask(is_edge) as u32;
                while mask != 0 {
                    let lane = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    fxaa_pixel(row + x + lane, x + lane, y, w, h,
                               &luma, pixels, &prev_row, &curr_row);
                }

                x += 16;
            }
        }

        // Scalar remainder
        while x < w - 1 {
            let idx = row + x;
            let m = unsafe { *luma.get_unchecked(idx) };
            let ln = unsafe { *luma.get_unchecked(idx - w) };
            let ls = unsafe { *luma.get_unchecked(idx + w) };
            let lw = unsafe { *luma.get_unchecked(idx - 1) };
            let le = unsafe { *luma.get_unchecked(idx + 1) };

            let mx = m.max(ln).max(ls).max(lw).max(le);
            let r = mx - m.min(ln).min(ls).min(lw).min(le);
            if r >= EDGE_MIN && r >= (mx >> 3) {
                if !row_saved {
                    curr_row.copy_from_slice(&pixels[row_start..row_start + row_bytes]);
                    row_saved = true;
                }
                fxaa_pixel(idx, x, y, w, h, &luma, pixels, &prev_row, &curr_row);
            }

            x += 1;
        }

        // Rotate: current row becomes previous for next iteration
        if row_saved {
            std::mem::swap(&mut prev_row, &mut curr_row);
        } else {
            // No edges on this row — prev_row for next iteration is the
            // unmodified pixels row, copy directly
            prev_row.copy_from_slice(&pixels[row_start..row_start + row_bytes]);
        }
    }
}
