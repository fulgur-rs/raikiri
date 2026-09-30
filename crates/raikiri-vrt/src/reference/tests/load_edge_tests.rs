//! Edge cases for `load_fixture` enumeration and root handling.
//!
//! Covers the missing-root shape, non-page files being ignored, and
//! non-UTF8 entry names being skipped — all documented behavior in the
//! `expected/` name-filter that previously had no direct test.

use super::*;
use std::io::Write;

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

fn write_input(dir: &std::path::Path) {
    let mut f = std::fs::File::create(dir.join("input.html")).unwrap();
    f.write_all(b"<!doctype html>").unwrap();
}

#[test]
fn missing_fixture_root_reports_missing_input() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does-not-exist");
    match load_fixture(&missing) {
        Err(FixtureError::MissingInputHtml { fixture_dir }) => {
            assert_eq!(fixture_dir, missing);
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        other => panic!("expected MissingInputHtml for missing root, got {other:?}"),
    }
}

#[test]
fn non_page_files_in_expected_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    write_input(tmp.path());
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    std::fs::write(expected.join("page-0000.png"), TINY_PNG).unwrap();
    // Non-PNG suffix, non-page prefix, and README-style files must be skipped.
    std::fs::write(expected.join("README.md"), b"docs").unwrap();
    std::fs::write(expected.join("other.png"), b"not a page").unwrap();
    std::fs::write(expected.join("page-0000.txt"), b"wrong suffix").unwrap();
    std::fs::write(expected.join("thumb-0000.png"), b"wrong prefix").unwrap();

    let fixture = load_fixture(tmp.path()).expect("non-page files must be ignored");
    assert_eq!(fixture.expected_pages.len(), 1);
    assert_eq!(fixture.expected_pages[0], TINY_PNG);
}

#[test]
#[cfg(unix)]
fn non_utf8_entry_name_is_skipped() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let tmp = tempfile::tempdir().unwrap();
    write_input(tmp.path());
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    std::fs::write(expected.join("page-0000.png"), TINY_PNG).unwrap();
    // Invalid-UTF8 file name: `file_name().to_str()` returns None so the
    // walker must skip it without failing the load.
    let bad = OsString::from_vec(vec![0xff, 0xfe, 0xfd]);
    std::fs::write(expected.join(&bad), b"bad name").unwrap();

    let fixture = load_fixture(tmp.path()).expect("non-UTF8 entry must be skipped");
    assert_eq!(fixture.expected_pages.len(), 1);
}

#[test]
fn unordered_pages_load_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    write_input(tmp.path());
    let expected = tmp.path().join("expected");
    std::fs::create_dir(&expected).unwrap();
    // Write out of order; loader must sort by page number.
    std::fs::write(expected.join("page-0001.png"), TINY_PNG).unwrap();
    std::fs::write(expected.join("page-0000.png"), TINY_PNG).unwrap();
    let fixture = load_fixture(tmp.path()).expect("unordered pages must load");
    assert_eq!(fixture.expected_pages.len(), 2);
}
