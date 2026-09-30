//! Type-level tests for `reference` (moved from inline `mod type_tests`).

use super::*;
use std::path::PathBuf;

#[test]
fn tolerance_constants_are_distinct() {
    assert_eq!(Tolerance::EXACT.max_delta, 0);
    assert_eq!(Tolerance::EXACT.max_diff_fraction, 0.0);
    assert_eq!(Tolerance::TIER2.max_delta, 1);
    assert!((Tolerance::TIER2.max_diff_fraction - 0.001).abs() < f32::EPSILON);
    assert_eq!(Tolerance::TIER3.max_delta, 2);
    assert!((Tolerance::TIER3.max_diff_fraction - 0.005).abs() < f32::EPSILON);
}

#[test]
fn diff_report_display_contains_key_info() {
    let report = DiffReport {
        page_index: 0,
        width: 100,
        height: 50,
        mismatched_pixel_count: 42,
        first_mismatch: Some(PixelMismatch {
            x: 10,
            y: 20,
            expected: [255, 0, 0, 255],
            actual: [0, 255, 0, 255],
        }),
        actual_png_path: Some(PathBuf::from("/tmp/actual.png")),
        diff_png_path: Some(PathBuf::from("/tmp/diff.png")),
    };
    let s = format!("{report}");
    assert!(s.contains("page 0"), "missing page index: {s}");
    assert!(s.contains("100x50"), "missing dimensions: {s}");
    assert!(s.contains("42"), "missing mismatch count: {s}");
    assert!(s.contains("(10, 20)"), "missing first mismatch coords: {s}");
    assert!(s.contains("actual.png"), "missing actual path: {s}");
    assert!(s.contains("diff.png"), "missing diff path: {s}");
}

#[test]
fn fixture_error_is_error_trait() {
    fn assert_error<E: std::error::Error>() {}
    assert_error::<FixtureError>();
}

/// `Display` for the two post-open recheck variants. Constructed
/// directly (no filesystem I/O) since
/// these are pure formatting checks — the variants' actual construction
/// sites are pinned by `raikiri_traits::io`'s own
/// `check_open_handle_regular_*` and `check_open_handle_containment_*`
/// tests.
#[test]
fn fixture_error_not_regular_file_post_open_display_contains_path() {
    let err = FixtureError::NotRegularFilePostOpen {
        path: PathBuf::from("/tmp/evil.bin"),
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/evil.bin"), "missing path: {s}");
    assert!(s.contains("non-regular"), "missing kind description: {s}");
}

#[test]
fn fixture_error_path_escape_post_open_display_contains_paths() {
    let err = FixtureError::PathEscapePostOpen {
        canonical: PathBuf::from("/outside/leaf.bin"),
        root: PathBuf::from("/fixture/root"),
    };
    let s = err.to_string();
    assert!(s.contains("/outside/leaf.bin"), "missing canonical: {s}");
    assert!(s.contains("/fixture/root"), "missing root: {s}");
}
