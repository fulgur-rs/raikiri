//! Diff-artifact helpers: `diff_dir_root` precedence,
//! `write_diff_artifacts` failure shapes, and `build_and_write_diff`
//! mixed-content rendering.
//!
//! `diff_dir_root` reads `CARGO_TARGET_TMPDIR` / `CARGO_TARGET_DIR` from the
//! runtime environment (usually unset under `cargo test`, falling back to
//! `target/`). These tests pin the precedence with a process-wide lock so
//! parallel tests cannot observe intermediate env states.

#![allow(unsafe_code)]

use super::*;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[test]
fn diff_dir_root_prefers_tmpdir_over_target_dir() {
    let _l = env_lock();
    // SAFETY: holding the lock; restore manually after assertions.
    let prev_tmp = std::env::var("CARGO_TARGET_TMPDIR").ok();
    let prev_dir = std::env::var("CARGO_TARGET_DIR").ok();
    unsafe {
        std::env::set_var("CARGO_TARGET_TMPDIR", "/tmp/vrt-tmp");
        std::env::set_var("CARGO_TARGET_DIR", "/tmp/vrt-target");
    }
    let root = diff_dir_root();
    // Restore before asserting so a failure cannot leak env state.
    unsafe {
        match prev_tmp {
            Some(v) => std::env::set_var("CARGO_TARGET_TMPDIR", v),
            None => std::env::remove_var("CARGO_TARGET_TMPDIR"),
        }
        match prev_dir {
            Some(v) => std::env::set_var("CARGO_TARGET_DIR", v),
            None => std::env::remove_var("CARGO_TARGET_DIR"),
        }
    }
    assert_eq!(root, PathBuf::from("/tmp/vrt-tmp").join("reference-diffs"));
}

#[test]
fn diff_dir_root_falls_back_to_target_dir_then_relative() {
    let _l = env_lock();
    let prev_tmp = std::env::var("CARGO_TARGET_TMPDIR").ok();
    let prev_dir = std::env::var("CARGO_TARGET_DIR").ok();
    unsafe {
        std::env::remove_var("CARGO_TARGET_TMPDIR");
        std::env::set_var("CARGO_TARGET_DIR", "/tmp/vrt-target2");
    }
    let via_target_dir = diff_dir_root();
    unsafe {
        std::env::remove_var("CARGO_TARGET_DIR");
    }
    let via_relative = diff_dir_root();
    unsafe {
        match prev_tmp {
            Some(v) => std::env::set_var("CARGO_TARGET_TMPDIR", v),
            None => std::env::remove_var("CARGO_TARGET_TMPDIR"),
        }
        match prev_dir {
            Some(v) => std::env::set_var("CARGO_TARGET_DIR", v),
            None => std::env::remove_var("CARGO_TARGET_DIR"),
        }
    }
    assert_eq!(
        via_target_dir,
        PathBuf::from("/tmp/vrt-target2").join("reference-diffs")
    );
    assert_eq!(
        via_relative,
        PathBuf::from("target").join("reference-diffs")
    );
}

#[test]
fn write_diff_artifacts_fails_closed_when_dir_uncreatable() {
    // Parent is a regular file so `create_dir_all(dir)` must fail and the
    // helper returns `(None, None)` instead of panicking.
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, b"not a dir").unwrap();
    let dir = blocker.join("subdir");
    let actual = crate::encode_png(&[0, 0, 0, 255, 0, 0, 0, 255], 2, 1);
    let expected = actual.clone();
    let (a, d) = write_diff_artifacts(&dir, 0, &actual, &expected);
    assert!(
        a.is_none(),
        "actual path must be None on dir-create failure"
    );
    assert!(d.is_none(), "diff path must be None on dir-create failure");
}

#[test]
fn build_and_write_diff_rejects_corrupt_and_mismatched_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let good = crate::encode_png(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1);
    let corrupt = b"not a png".to_vec();
    assert!(
        !build_and_write_diff(&tmp.path().join("a.png"), &corrupt, &good),
        "corrupt actual must return false"
    );
    assert!(
        !build_and_write_diff(&tmp.path().join("b.png"), &good, &corrupt),
        "corrupt expected must return false"
    );
    let other_size = crate::encode_png(&[0, 0, 0, 255], 1, 1);
    assert!(
        !build_and_write_diff(&tmp.path().join("c.png"), &good, &other_size),
        "dimension mismatch must return false"
    );
}

#[test]
fn build_and_write_diff_renders_mixed_match_and_diff() {
    // 2x1: pixel 0 identical, pixel 1 differs. Output must contain one dim
    // grey pixel (matching branch) and one magenta pixel (diff branch),
    // covering both arms of the per-pixel loop.
    let actual_rgba = [10, 20, 30, 255, 255, 0, 0, 255];
    let expected_rgba = [10, 20, 30, 255, 0, 0, 255, 255];
    let actual = crate::encode_png(&actual_rgba, 2, 1);
    let expected = crate::encode_png(&expected_rgba, 2, 1);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("diff.png");
    assert!(build_and_write_diff(&out, &actual, &expected));
    let pix = tiny_skia::Pixmap::decode_png(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((pix.width(), pix.height()), (2, 1));
    let data = pix.data();
    // Pixel 1 (differing) must be magenta.
    assert_eq!(&data[4..8], &[255, 0, 255, 255]);
    // Pixel 0 (matching) must be dim grey, not magenta, fully opaque.
    assert_ne!(&data[0..4], &[255, 0, 255, 255]);
    assert_eq!(data[3], 255);
    assert_eq!(data[0], data[1]);
    assert_eq!(data[1], data[2]);
}

#[test]
fn write_diff_artifacts_returns_none_for_unwritable_actual() {
    // Pre-create a directory at the would-be `*-actual.png` path so the
    // actual write fails but the diff write still succeeds. Covers the
    // `actual_written == false` (None) arm while keeping `diff_ok == true`.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir(dir.join("page-0000-actual.png")).unwrap();
    let actual_rgba = [10, 20, 30, 255, 255, 0, 0, 255];
    let expected_rgba = [10, 20, 30, 255, 0, 0, 255, 255];
    let actual = crate::encode_png(&actual_rgba, 2, 1);
    let expected = crate::encode_png(&expected_rgba, 2, 1);
    let (a, d) = write_diff_artifacts(&dir, 0, &actual, &expected);
    assert!(a.is_none(), "blocked actual write must yield None");
    assert!(d.is_some(), "diff write should still succeed");
}
