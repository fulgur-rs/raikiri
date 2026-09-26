//! raikiri-vrt — VRT test setup for raikiri (dev-only).
//!
//! Provides the reference-image fixture / diff / tolerance test setup via the
//! `reference` module, plus anyrender rasterize + rayon determinism tests
//! that guard the raster pipeline used by production PNG encoding.
//! Production PNG encoding itself lives in `raikiri::html_to_png` as an
//! inlined private helper.
//!
//! This crate is `publish = false` because `reference::Fixture` and friends
//! are test-scaffolding APIs, not production surface.
//!
//! Downstream consumers (all dev-only):
//! - hello-world VRT integration test — `raikiri/tests/hello_world_vrt.rs`
//! - determinism / rayon thread-count tests — this crate's own `#[cfg(test)]`

pub mod reference;

/// Encode a premultiplied RGBA8 buffer to PNG bytes via `tiny_skia::Pixmap`.
///
/// The buffer must be exactly `width * height * 4` bytes. Buffer format is
/// premultiplied RGBA8 — the `anyrender_vello_cpu` output convention.
/// `tiny_skia` stores pixmaps in the same format, so encoding is a direct
/// wrap-then-serialize.
///
/// Retained as a `pub(crate)` helper for internal callers (`reference::build_and_write_diff`).
/// Not part of the public API — production PNG encoding lives in `raikiri::html_to_png`.
///
/// # Panics
///
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization failure (tiny-skia never returns an error for a
///   well-formed pixmap in practice; treated as an invariant violation)
pub(crate) fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let pixmap = tiny_skia::Pixmap::from_vec(rgba.to_vec(), size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)");
    pixmap
        .encode_png()
        .expect("encode_png: tiny_skia::Pixmap::encode_png should not fail for a valid pixmap")
}

#[cfg(test)]
mod tests;
