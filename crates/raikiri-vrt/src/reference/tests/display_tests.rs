//! Display + `Error::source` pins for `FixtureError` and `DiffReport`.
//!
//! Covers the `Display` arms that existing happy-path / integration tests
//! never format (each variant's message shape), plus `source()` which only
//! returns `Some` for `IoError`.

use super::*;
use std::path::PathBuf;

#[test]
fn io_error_display_names_path() {
    let err = FixtureError::IoError {
        path: PathBuf::from("/tmp/fixture/input.html"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture/input.html"), "missing path: {s}");
    assert!(s.contains("IO error"), "missing kind: {s}");
}

#[test]
fn symlink_rejected_display_names_path() {
    let err = FixtureError::SymlinkRejected {
        path: PathBuf::from("/tmp/fixture/input.html"),
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture/input.html"), "missing path: {s}");
    assert!(s.contains("symlink"), "missing kind: {s}");
}

#[test]
fn not_regular_file_display_names_path() {
    let err = FixtureError::NotRegularFile {
        path: PathBuf::from("/tmp/fixture/input.html"),
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture/input.html"), "missing path: {s}");
    assert!(s.contains("not a regular file"), "missing kind: {s}");
}

#[test]
fn oversized_fixture_display_pins_numbers() {
    let err = FixtureError::OversizedFixture {
        path: PathBuf::from("/tmp/fixture/page-0000.png"),
        size: 101,
        cap: 100,
    };
    let s = err.to_string();
    assert!(s.contains("page-0000.png"), "missing path: {s}");
    assert!(s.contains("101"), "missing size: {s}");
    assert!(s.contains("100"), "missing cap: {s}");
}

#[test]
fn path_escape_display_names_both_paths() {
    let err = FixtureError::PathEscape {
        canonical: PathBuf::from("/outside/leaf.png"),
        root: PathBuf::from("/fixture/root"),
    };
    let s = err.to_string();
    assert!(s.contains("/outside/leaf.png"), "missing canonical: {s}");
    assert!(s.contains("/fixture/root"), "missing root: {s}");
}

#[test]
fn too_many_pages_display_pins_cap() {
    let err = FixtureError::TooManyExpectedPages {
        fixture_dir: PathBuf::from("/tmp/fixture"),
        cap: 1024,
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture"), "missing dir: {s}");
    assert!(s.contains("1024"), "missing cap: {s}");
}

#[test]
fn oversized_aggregate_display_pins_numbers() {
    let err = FixtureError::OversizedFixtureAggregate {
        fixture_dir: PathBuf::from("/tmp/fixture"),
        total: 300,
        cap: 256,
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture"), "missing dir: {s}");
    assert!(s.contains("300"), "missing total: {s}");
    assert!(s.contains("256"), "missing cap: {s}");
}

#[test]
fn missing_input_display_names_dir() {
    let err = FixtureError::MissingInputHtml {
        fixture_dir: PathBuf::from("/tmp/fixture"),
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture"), "missing dir: {s}");
    assert!(s.contains("input.html"), "missing file name: {s}");
}

#[test]
fn non_contiguous_display_names_gap() {
    let err = FixtureError::NonContiguousPages {
        fixture_dir: PathBuf::from("/tmp/fixture"),
        found: vec![0, 2],
    };
    let s = err.to_string();
    assert!(s.contains("/tmp/fixture"), "missing dir: {s}");
    assert!(s.contains("non-contiguous"), "missing kind: {s}");
}

#[test]
fn error_source_is_some_only_for_io() {
    use std::error::Error;
    let io = FixtureError::IoError {
        path: PathBuf::from("/tmp/a"),
        source: std::io::Error::other("x"),
    };
    assert!(io.source().is_some(), "IoError must expose source");
    let other = FixtureError::MissingInputHtml {
        fixture_dir: PathBuf::from("/tmp/a"),
    };
    assert!(other.source().is_none(), "non-Io variant must return None");
    let symlink = FixtureError::SymlinkRejected {
        path: PathBuf::from("/tmp/a"),
    };
    assert!(
        symlink.source().is_none(),
        "non-Io variant must return None"
    );
}

#[test]
fn diff_report_display_without_optional_sections() {
    let report = DiffReport {
        page_index: 2,
        width: 10,
        height: 5,
        mismatched_pixel_count: 0,
        first_mismatch: None,
        actual_png_path: None,
        diff_png_path: None,
    };
    let s = format!("{report}");
    assert!(s.contains("page 2"), "missing page: {s}");
    assert!(s.contains("10x5"), "missing dims: {s}");
    assert!(
        !s.contains("first mismatch"),
        "unexpected mismatch section: {s}"
    );
    assert!(!s.contains("actual PNG"), "unexpected actual section: {s}");
    assert!(!s.contains("diff PNG"), "unexpected diff section: {s}");
}
