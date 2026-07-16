//! Self-tests for raikiri_vrt::reference harness (m1.10).
//!
//! Uses synthetic PNG fixtures constructed via encode_png and tempfile-backed
//! directories. Verifies the harness independently of any real pipeline.
#![allow(unsafe_code)]

use raikiri_vrt::encode_png;
use raikiri_vrt::reference::{FixtureError, Tolerance, compare_png, load_fixture, run_and_compare};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

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

/// Build a synthetic fixture directory.
///
/// Writes `input.html` when `html` is Some, and `expected/page-{N:04}.png`
/// entries for each `(page_number, png_bytes)` in `pages`.
fn build_fixture(html: Option<&[u8]>, pages: &[(u32, Vec<u8>)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    if let Some(bytes) = html {
        fs::write(dir.path().join("input.html"), bytes).expect("write input.html");
    }
    if !pages.is_empty() {
        let expected_dir = dir.path().join("expected");
        fs::create_dir(&expected_dir).expect("mkdir expected");
        for (n, png) in pages {
            fs::write(expected_dir.join(format!("page-{n:04}.png")), png).expect("write page png");
        }
    }
    dir
}

fn tiny_png() -> Vec<u8> {
    encode_png(&solid([0, 0, 0, 255], 2, 2), 2, 2)
}

#[test]
fn test_load_fixture_missing_input() {
    let dir = build_fixture(None, &[(0, tiny_png())]);
    let err = load_fixture(dir.path()).expect_err("expected MissingInputHtml");
    match err {
        FixtureError::MissingInputHtml { fixture_dir } => {
            assert_eq!(fixture_dir, dir.path());
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn test_load_fixture_noncontiguous() {
    let dir = build_fixture(
        Some(b"<p>hi</p>"),
        &[(0, tiny_png()), (2, tiny_png())], // missing page 1
    );
    let err = load_fixture(dir.path()).expect_err("expected NonContiguousPages");
    match err {
        FixtureError::NonContiguousPages { found, .. } => {
            assert_eq!(found, vec![0, 2]);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn test_load_fixture_success_without_expected_dir() {
    // update-goldens mode: expected/ absent is allowed.
    let dir = build_fixture(Some(b"<p>hi</p>"), &[]);
    let fixture = load_fixture(dir.path()).expect("load should succeed");
    assert_eq!(fixture.input_html, b"<p>hi</p>");
    assert!(fixture.expected_pages.is_empty());
    assert_eq!(fixture.root, dir.path());
}

#[test]
fn test_load_fixture_success_with_pages() {
    let png_a = tiny_png();
    let png_b = tiny_png();
    let dir = build_fixture(
        Some(b"<p>hi</p>"),
        &[(0, png_a.clone()), (1, png_b.clone())],
    );
    let fixture = load_fixture(dir.path()).expect("load");
    assert_eq!(fixture.expected_pages.len(), 2);
    assert_eq!(fixture.expected_pages[0], png_a);
    assert_eq!(fixture.expected_pages[1], png_b);
}

/// Env-var name toggling golden update mode. Kept in sync with the value in
/// crates/raikiri-vrt/src/reference.rs.
const UPDATE_GOLDENS_ENV: &str = "RAIKIRI_UPDATE_GOLDENS";

/// Guard for RAIKIRI_UPDATE_GOLDENS across a scoped block.
/// The env var mutation is process-global; cargo runs integration tests
/// single-threaded per test crate by default when the harness is `libtest`,
/// but we guard the set/unset symmetrically to keep intent explicit.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: single-threaded test execution — cargo test integration
        // tests default to serial execution within a single binary.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, prev }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: same as set().
            Some(v) => unsafe {
                std::env::set_var(self.key, v);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

#[test]
fn test_run_and_compare_update_goldens() {
    // Empty expected/ + env set → expected regenerated from pipeline output.
    let png = encode_png(&solid([0, 128, 0, 255], 4, 4), 4, 4);
    let dir = build_fixture(Some(b"<p>hi</p>"), &[]);
    let pipeline_png = png.clone();

    {
        let _guard = EnvGuard::set(UPDATE_GOLDENS_ENV, "1");
        // No panic expected — update mode always succeeds if I/O succeeds.
        run_and_compare(dir.path(), Tolerance::EXACT, move |_html| {
            vec![pipeline_png]
        });
    }

    // After update, expected/page-0000.png exists and equals pipeline output.
    let written = fs::read(dir.path().join("expected/page-0000.png"))
        .expect("expected png should exist after update");
    assert_eq!(written, png);
}

#[test]
fn test_run_and_compare_success_removes_no_files() {
    // Success path: no target/ artifacts are written.
    let png = encode_png(&solid([200, 100, 50, 255], 4, 4), 4, 4);
    let dir = build_fixture(Some(b"<p>hi</p>"), &[(0, png.clone())]);

    // Determine what target dir the harness would use. CARGO_TARGET_TMPDIR is
    // only available via the compile-time `env!()` macro for test/bench
    // targets — it is NOT propagated into the runtime process environment
    // (verified: std::env::var("CARGO_TARGET_TMPDIR") is always Err at test
    // runtime, even under `cargo test`). Reading it at compile time here is
    // valid because this file is itself an integration test target.
    let target_tmp = env!("CARGO_TARGET_TMPDIR");
    let fixture_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let diff_dir = Path::new(target_tmp)
        .join("reference-diffs")
        .join(&fixture_name);

    // Ensure no stale diff dir from a prior run.
    let _ = fs::remove_dir_all(&diff_dir);

    let pipeline_png = png.clone();
    run_and_compare(dir.path(), Tolerance::EXACT, move |_html| {
        vec![pipeline_png]
    });

    // No artifacts should have been produced.
    assert!(
        !diff_dir.exists(),
        "success path should not create diff dir: {}",
        diff_dir.display()
    );
}
