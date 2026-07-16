//! Self-tests for raikiri_vrt::reference harness (m1.10).
//!
//! Uses synthetic PNG fixtures constructed via encode_png and tempfile-backed
//! directories. Verifies the harness independently of any real pipeline.

use raikiri_vrt::encode_png;
use raikiri_vrt::reference::{Tolerance, compare_png};

/// Solid-color RGBA8 buffer for a given size.
fn solid(color: [u8; 4], w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for _ in 0..(w * h) {
        buf.extend_from_slice(&color);
    }
    buf
}

#[test]
fn test_compare_png_exact_match() {
    let rgba = solid([255, 0, 0, 255], 10, 10);
    let png = encode_png(&rgba, 10, 10);
    // Compare a PNG against a bytewise copy of itself.
    assert!(compare_png(&png, &png, Tolerance::EXACT).is_ok());
}

#[test]
fn test_compare_png_diff_detected() {
    let mut rgba_a = solid([255, 0, 0, 255], 10, 10);
    let rgba_b = solid([255, 0, 0, 255], 10, 10);
    // Flip pixel (3, 4) to green in image A.
    let idx = (4 * 10 + 3) * 4;
    rgba_a[idx..idx + 4].copy_from_slice(&[0, 255, 0, 255]);
    let png_a = encode_png(&rgba_a, 10, 10);
    let png_b = encode_png(&rgba_b, 10, 10);

    let err =
        compare_png(&png_a, &png_b, Tolerance::EXACT).expect_err("expected diff to be detected");
    assert_eq!(err.width, 10);
    assert_eq!(err.height, 10);
    assert_eq!(err.mismatched_pixel_count, 1);
    let m = err
        .first_mismatch
        .expect("first_mismatch should be populated");
    assert_eq!((m.x, m.y), (3, 4));
    assert_eq!(m.actual, [0, 255, 0, 255]);
    assert_eq!(m.expected, [255, 0, 0, 255]);
    // page_index defaults to 0 when compare_png is called directly (no page context).
    assert_eq!(err.page_index, 0);
    // No paths set — compare_png does no I/O.
    assert!(err.actual_png_path.is_none());
    assert!(err.diff_png_path.is_none());
}
