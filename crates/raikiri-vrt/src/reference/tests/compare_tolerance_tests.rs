//! Tolerance-fraction path for `compare_png`.
//!
//! Existing tier tests pin the `mismatched == 0` fast path (delta within
//! `max_delta` passes without consulting the fraction) and the over-fraction
//! reject path. The remaining production branch is `mismatched > 0` but
//! `fraction <= max_diff_fraction` returning `Ok` — small absolute diffs in
//! a large image that tolerance forgives.

use super::*;

fn solid_rgba(color: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for _ in 0..(w * h) {
        buf.extend_from_slice(&color);
    }
    buf
}

fn png_of(color: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    crate::encode_png(&solid_rgba(color, w, h), w, h)
}

#[test]
fn tier2_forgives_few_over_delta_pixels_in_large_image() {
    // 100x100 = 10_000 px. Flip 5 pixels by 2 (> TIER2 max_delta=1).
    // 5/10_000 = 0.05% <= 0.1% (TIER2) so compare must pass via the
    // fraction-allow branch; EXACT must still fail.
    let (w, h) = (100, 100);
    let mut a = solid_rgba([10, 20, 30, 255], w, h);
    let b = solid_rgba([10, 20, 30, 255], w, h);
    for n in 0..5u32 {
        let idx = (n as usize) * 4;
        a[idx] = 12;
    }
    let png_a = crate::encode_png(&a, w, h);
    let png_b = crate::encode_png(&b, w, h);
    compare_png(&png_a, &png_b, Tolerance::EXACT).expect_err("delta 2 must fail EXACT");
    assert!(
        compare_png(&png_a, &png_b, Tolerance::TIER2).is_ok(),
        "5/10000 = 0.05% should pass TIER2 0.1%"
    );
}

#[test]
fn tier2_rejects_just_over_fraction_with_over_delta_pixels() {
    // 100x100 = 10_000 px. Flip 11 pixels by 2: 0.11% > 0.1% must fail
    // TIER2 but pass TIER3 (0.5%).
    let (w, h) = (100, 100);
    let mut a = solid_rgba([10, 20, 30, 255], w, h);
    let b = solid_rgba([10, 20, 30, 255], w, h);
    for n in 0..11u32 {
        let idx = (n as usize) * 4;
        a[idx] = 12;
    }
    let png_a = crate::encode_png(&a, w, h);
    let png_b = crate::encode_png(&b, w, h);
    compare_png(&png_a, &png_b, Tolerance::TIER2).expect_err("0.11% must fail TIER2");
    assert!(
        compare_png(&png_a, &png_b, Tolerance::TIER3).is_ok(),
        "0.11% should pass TIER3 0.5%"
    );
}

#[test]
fn exact_passes_for_identical_large_image() {
    let png = png_of([200, 100, 50, 255], 32, 32);
    assert!(compare_png(&png, &png, Tolerance::EXACT).is_ok());
}
