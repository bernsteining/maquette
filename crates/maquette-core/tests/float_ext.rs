#![allow(clippy::disallowed_methods)]

use maquette_core::math::FloatExt;

fn samples_f64() -> Vec<f64> {
    let mut v = vec![0.0, -0.0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.49999999999999994, -0.49999999999999994,
        4503599627370495.5, -4503599627370495.5, 4503599627370496.0, 9007199254740993.0, 1e300, -1e300,
        f64::MIN_POSITIVE, f64::EPSILON, f64::MAX, f64::MIN, f64::INFINITY, f64::NEG_INFINITY, f64::NAN];
    let mut x: u64 = 0x1234_5678_9abc_def1;
    for _ in 0..200_000 {
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        let f = f64::from_bits(x);
        if f.is_finite() { v.push(f); }
        v.push((x % 2_000_001) as f64 / 1000.0 - 1000.0);
        v.push(((x >> 11) % 4001) as f64 * 0.5 - 1000.0);
    }
    v
}

fn same(a: f64, b: f64) -> bool { a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) || (a == b) }

#[test]
fn fround_matches_std_round() {
    for x in samples_f64() {
        let (r, e) = (x.fround(), x.round());
        assert!(r.to_bits() == e.to_bits() || (r.is_nan() && e.is_nan()), "round({x:e}) = {r:e}, std {e:e}");
    }
}

#[test]
fn fround_f32_matches_std_round() {
    for x in samples_f64() {
        let x = x as f32;
        let (r, e) = (x.fround(), x.round());
        assert!(r.to_bits() == e.to_bits() || (r.is_nan() && e.is_nan()), "round({x:e}) = {r:e}, std {e:e}");
    }
}

#[test]
fn fmin_fmax_match_std() {
    let s = samples_f64();
    for w in s.windows(2) {
        let (a, b) = (w[0], w[1]);
        assert!(same(a.fmin(b), a.min(b)), "min({a:e},{b:e})");
        assert!(same(a.fmax(b), a.max(b)), "max({a:e},{b:e})");
        assert!(same(b.fmin(a), b.min(a)));
        assert!(same(b.fmax(a), b.max(a)));
        let (fa, fb) = (a as f32, b as f32);
        assert!(fa.fmin(fb) == fa.min(fb) || (fa.fmin(fb).is_nan() && fa.min(fb).is_nan()));
        assert!(fa.fmax(fb) == fa.max(fb) || (fa.fmax(fb).is_nan() && fa.max(fb).is_nan()));
    }
    assert_eq!(f64::NAN.fmin(1.0), 1.0);
    assert_eq!(1.0f64.fmin(f64::NAN), 1.0);
    assert_eq!(f64::NAN.fmax(1.0), 1.0);
    assert_eq!(1.0f64.fmax(f64::NAN), 1.0);
}
