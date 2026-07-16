//! Self-tests for raikiri_vrt::reference harness (m1.10).
//!
//! Uses synthetic PNG fixtures constructed via encode_png and tempfile-backed
//! directories. Verifies the harness independently of any real pipeline.
#![allow(unsafe_code)]

use raikiri_vrt::encode_png;
use raikiri_vrt::reference::{FixtureError, Tolerance, compare_png, load_fixture, run_and_compare};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};
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

/// Global lock serializing every test that reads or mutates the process
/// environment. libtest runs integration tests within one binary in parallel
/// by default (per Rust's rationale for making env::set_var unsafe in edition
/// 2024). Every test that touches RAIKIRI_UPDATE_GOLDENS — set, unset, OR read
/// through run_and_compare — must hold this lock for its full duration.
fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        // A poisoned lock here would only mean a previous env-touching test
        // panicked while holding it — recovering is safe for our purposes,
        // and this makes independent test failures visible.
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Guard for RAIKIRI_UPDATE_GOLDENS across a scoped block. Holds env_lock()
/// for its lifetime so no other env-touching test can observe an
/// intermediate state.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
    // _lock is held through Drop: EnvGuard::drop() restores the env var
    // while _lock is still alive, then field drop-glue releases _lock.
    // (Rust guarantees Drop::drop() runs before any field is auto-dropped.)
    _lock: MutexGuard<'static, ()>,
}
impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let lock = env_lock();
        let prev = std::env::var(key).ok();
        // SAFETY: env_lock() serializes all env access in this test binary,
        // so no other thread can call getenv/setenv concurrently.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key,
            prev,
            _lock: lock,
        }
    }

    /// Acquire the env lock without mutating any env var. Use when a test
    /// calls into code that reads RAIKIRI_UPDATE_GOLDENS but the test itself
    /// doesn't set it — prevents a concurrent set from another test leaking
    /// into this test's read.
    fn read() -> Self {
        let lock = env_lock();
        Self {
            key: "",
            prev: None,
            _lock: lock,
        }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        if self.key.is_empty() {
            return; // read-only guard: no env restoration needed
        }
        match &self.prev {
            // SAFETY: same as set() — holding env_lock().
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
    let _guard = EnvGuard::read(); // exclude concurrent env-setter tests
    // Success path: no target/ artifacts are written.
    let png = encode_png(&solid([200, 100, 50, 255], 4, 4), 4, 4);
    let dir = build_fixture(Some(b"<p>hi</p>"), &[(0, png.clone())]);

    // run_and_compare's diff_dir_root() reads CARGO_TARGET_TMPDIR /
    // CARGO_TARGET_DIR from the *runtime* process environment, where neither
    // is set (those are cargo-supplied compile-time vars for the test/bench
    // target, not propagated to the running binary's env) — so it falls
    // through to the relative "target/reference-diffs", resolved against
    // the process's current directory, which `cargo test` sets to the
    // package's manifest directory. CARGO_MANIFEST_DIR (compile-time) gives
    // us that same directory to compute the actual on-disk location.
    let fixture_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let diff_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
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

#[test]
fn test_run_and_compare_writes_artifacts_on_diff() {
    use std::panic;
    // Compare mismatch → panics with a DiffReport, and both actual/diff
    // PNGs land under target/reference-diffs/<fixture>/.
    let _guard = EnvGuard::read(); // exclude concurrent env-setter tests
    let expected = encode_png(&solid([0, 0, 255, 255], 4, 4), 4, 4); // blue
    let actual = encode_png(&solid([255, 0, 0, 255], 4, 4), 4, 4); // red
    let dir = build_fixture(Some(b"<p>hi</p>"), &[(0, expected)]);

    // run_and_compare's diff_dir_root() reads CARGO_TARGET_TMPDIR /
    // CARGO_TARGET_DIR from the *runtime* process environment, where neither
    // is set (those are cargo-supplied compile-time vars for the test/bench
    // target, not propagated to the running binary's env) — so it falls
    // through to the relative "target/reference-diffs", resolved against
    // the process's current directory, which `cargo test` sets to the
    // package's manifest directory. CARGO_MANIFEST_DIR (compile-time) gives
    // us that same directory to compute the actual on-disk location.
    let fixture_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let diff_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("reference-diffs")
        .join(&fixture_name);
    let _ = fs::remove_dir_all(&diff_dir);

    // catch_unwind because run_and_compare panics on diff. AssertUnwindSafe
    // needed because our fixture TempDir and paths are captured by ref.
    let actual_clone = actual.clone();
    let dir_path = dir.path().to_path_buf();
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        run_and_compare(&dir_path, Tolerance::EXACT, move |_html| vec![actual_clone]);
    }));
    assert!(result.is_err(), "expected run_and_compare to panic on diff");

    // Both artifacts should have been written.
    let actual_path = diff_dir.join("page-0000-actual.png");
    let diff_path = diff_dir.join("page-0000-diff.png");
    assert!(
        actual_path.exists(),
        "actual PNG not written to {}",
        actual_path.display()
    );
    assert!(
        diff_path.exists(),
        "diff PNG not written to {}",
        diff_path.display()
    );

    // Actual PNG bytes exactly equal the pipeline output.
    let written_actual = fs::read(&actual_path).expect("read actual.png");
    assert_eq!(written_actual, actual);

    // Diff PNG decodes to same dimensions and its non-transparent pixels
    // include magenta (255, 0, 255, 255) because every pixel differed.
    let diff_pixmap = tiny_skia::Pixmap::decode_png(&fs::read(&diff_path).expect("read diff.png"))
        .expect("decode diff.png");
    assert_eq!(diff_pixmap.width(), 4);
    assert_eq!(diff_pixmap.height(), 4);
    let data = diff_pixmap.data();
    let all_magenta = data.chunks_exact(4).all(|px| px == [255, 0, 255, 255]);
    assert!(
        all_magenta,
        "expected every pixel in diff.png to be magenta"
    );
}

#[test]
fn test_compare_png_dimension_mismatch() {
    // compare_png:207-217 branch: actual と expected の decode dim が異なる場合、
    // DiffReport::{width, height} は expected 側の dim を、mismatched_pixel_count
    // は ew * eh を報告する。色を変えているのは EXACT byte-eq shortcut を
    // 偶然通っていないことが視覚的に読めるようにするため (実際には dim check が
    // 先に走るので shortcut に到達しない)。
    let actual_png = encode_png(&solid([255, 0, 0, 255], 10, 10), 10, 10); // 10x10 red
    let expected_png = encode_png(&solid([0, 0, 255, 255], 5, 5), 5, 5); // 5x5 blue

    let err = compare_png(&actual_png, &expected_png, Tolerance::EXACT)
        .expect_err("dim mismatch should be detected");

    assert_eq!(err.width, 5, "width should be expected dim");
    assert_eq!(err.height, 5, "height should be expected dim");
    assert_eq!(err.mismatched_pixel_count, 25, "should equal ew * eh");
    assert!(
        err.first_mismatch.is_none(),
        "dim mismatch has no pixel-level info"
    );
    assert!(err.actual_png_path.is_none(), "compare_png does no I/O");
    assert!(err.diff_png_path.is_none(), "compare_png does no I/O");
    assert_eq!(err.page_index, 0, "compare_png sets page_index to 0");
}

#[test]
fn test_run_and_compare_page_count_mismatch() {
    use std::panic;

    // run_and_compare:395-402 branch: pipeline output page count が fixture の
    // expected_pages.len() と一致しないとき、compare_png loop の手前で panic!。
    // 副作用として diff artifacts は書かれない (loop に入らないため)。
    let _guard = EnvGuard::read(); // exclude concurrent env-setter tests

    let png_a = encode_png(&solid([255, 0, 0, 255], 4, 4), 4, 4);
    let png_b = encode_png(&solid([0, 255, 0, 255], 4, 4), 4, 4);
    // Fixture: 1 expected page. Pipeline は 2 page 返す → mismatch。
    let dir = build_fixture(Some(b"<p>hi</p>"), &[(0, png_a.clone())]);

    let fixture_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let diff_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("reference-diffs")
        .join(&fixture_name);
    let _ = fs::remove_dir_all(&diff_dir);

    let png_a_clone = png_a.clone();
    let png_b_clone = png_b.clone();
    let dir_path = dir.path().to_path_buf();
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        run_and_compare(&dir_path, Tolerance::EXACT, move |_html| {
            vec![png_a_clone, png_b_clone]
        });
    }));

    let payload = result.expect_err("run_and_compare should panic on page count mismatch");
    // Panic payload は run_and_compare の panic!("...") が生成する String。
    let msg = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&'static str>().copied())
        .unwrap_or("");
    assert!(
        msg.contains("page count mismatch"),
        "expected 'page count mismatch' in panic payload, got: {msg}"
    );
    assert!(
        msg.contains("expected 1"),
        "expected 'expected 1' in panic payload, got: {msg}"
    );
    assert!(
        msg.contains("actual 2"),
        "expected 'actual 2' in panic payload, got: {msg}"
    );

    // panic は compare_png loop の手前で走るので diff artifacts は生成されない。
    assert!(
        !diff_dir.exists(),
        "page-count panic path must not create diff dir: {}",
        diff_dir.display()
    );
}
