//! raikiri-vrt — VRT harness (tiny-skia PNG encoder for anyrender pipelines).
//!
//! Thin wrapper providing the last-mile of the M1 render pipeline:
//!   RGBA8 buffer → PNG bytes.
//!
//! Scene rasterization is handled by `anyrender::render_to_buffer` (fresh
//! renderer per call, matches blitz-paint's caller-owns-reset convention).
//! Backend selection: caller passes the concrete `ImageRenderer` type via
//! turbofish. M1 default = `anyrender_vello_cpu::VelloCpuImageRenderer`.
//!
//! Downstream consumers:
//! - `raikiri` umbrella `html_to_png` (M1 end-to-end pipeline)
//! - hello-world VRT (m1.14)
//! - determinism test (m1.13)
//! - rayon thread-count test (m1.18)

/// Encode a premultiplied RGBA8 buffer to PNG bytes via `tiny_skia::Pixmap`.
///
/// The buffer must be exactly `width * height * 4` bytes. Buffer format is
/// premultiplied RGBA8 — the `anyrender_vello_cpu` output convention.
/// `tiny_skia` stores pixmaps in the same format, so encoding is a direct
/// wrap-then-serialize.
///
/// # Panics
///
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization failure (tiny-skia never returns an error for a
///   well-formed pixmap in practice; treated as an invariant violation)
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
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
mod tests {
    use anyrender::PaintScene;
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Rect};
    use peniko::{Color, Fill, color::palette::css};

    use super::encode_png;

    const W: u32 = 100;
    const H: u32 = 100;

    /// Deliberately trivial scene: a solid axis-aligned red rectangle.
    /// If this is not deterministic then nothing else in the pipeline will be.
    ///
    /// Call sites wrap this in a closure (`|scene| draw_red_rect(scene)`) —
    /// necessary because Rust cannot infer `S` from a bare fn-item argument
    /// against `render_to_buffer`'s `FnOnce(&mut R::ScenePainter<'_>)` HRTB.
    /// The closure unifies its parameter type directly per call, so the
    /// accompanying `#[allow(clippy::redundant_closure)]` attributes are
    /// load-bearing.
    fn draw_red_rect<S: PaintScene>(scene: &mut S) {
        let color: Color = css::RED;
        let rect = Rect::new(10.0, 10.0, 90.0, 90.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }

    #[test]
    fn render_helper_produces_expected_buffer_shape() {
        #[allow(clippy::redundant_closure)]
        let buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_red_rect(scene),
            W,
            H,
        );
        assert_eq!(buf.len(), (W as usize) * (H as usize) * 4);
        // Pixel (50, 50) is inside the red rect; must not still be transparent-black.
        let idx = (50 * (W as usize) + 50) * 4;
        assert_ne!(
            &buf[idx..idx + 4],
            &[0, 0, 0, 0],
            "expected non-transparent pixel at (50,50); scene did not render",
        );
    }

    #[test]
    fn render_helper_is_byte_identical() {
        #[allow(clippy::redundant_closure)]
        let a = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_red_rect(scene),
            W,
            H,
        );
        #[allow(clippy::redundant_closure)]
        let b = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_red_rect(scene),
            W,
            H,
        );
        assert_eq!(a.len(), b.len());
        assert!(
            a == b,
            "independent renderers diverged: {} of {} bytes differ",
            a.iter().zip(&b).filter(|(x, y)| x != y).count(),
            a.len(),
        );
    }

    #[test]
    fn encode_png_produces_png_signature() {
        #[allow(clippy::redundant_closure)]
        let rgba = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_red_rect(scene),
            W,
            H,
        );
        let png = encode_png(&rgba, W, H);
        // PNG magic bytes: \x89 P N G \r \n \x1A \n
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'],
            "PNG magic bytes mismatch — encoder produced non-PNG output",
        );
    }

    #[test]
    fn two_different_scenes_render_independently() {
        fn draw_blue_rect<S: PaintScene>(scene: &mut S) {
            let color: Color = css::BLUE;
            let rect = Rect::new(0.0, 0.0, 50.0, 50.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        }

        // Render red rect first, then blue rect via a separate fresh render. Under
        // the m1.8-era `rasterize(&mut R, ...)` bug (renderer state accumulation),
        // the blue buffer would still contain the red rect from the prior render.
        // With `anyrender::render_to_buffer`'s fresh-renderer-per-call semantics,
        // that state leak is structurally impossible — this test documents that
        // invariant.
        #[allow(clippy::redundant_closure)]
        let _red_buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_red_rect(scene),
            W,
            H,
        );
        #[allow(clippy::redundant_closure)]
        let blue_buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| draw_blue_rect(scene),
            W,
            H,
        );

        // Pixel (75, 75) is inside red rect [10..90, 10..90] but OUTSIDE blue rect
        // [0..50, 0..50]. Under bug: blue_buf would still contain red rect at
        // (75,75) → non-transparent. Under fix: blue_buf is a fresh render of blue
        // only → transparent-black.
        let idx = (75 * (W as usize) + 75) * 4;
        let pixel = &blue_buf[idx..idx + 4];
        assert_eq!(
            pixel,
            &[0, 0, 0, 0],
            "blue_buf(75,75) not transparent — red rect from prior render leaked (state accumulation bug regression). Got: {pixel:?}",
        );

        // Sanity: pixel (25, 25) is inside blue rect — must be non-transparent.
        let idx2 = (25 * (W as usize) + 25) * 4;
        let pixel2 = &blue_buf[idx2..idx2 + 4];
        assert_ne!(
            pixel2,
            &[0, 0, 0, 0],
            "blue_buf(25,25) transparent — blue rect did not render at all",
        );
    }
}
