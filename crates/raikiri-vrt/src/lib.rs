//! raikiri-vrt — VRT harness (backend-agnostic anyrender rasterizer + tiny-skia PNG encoder).
//!
//! Thin wrapper providing the last-mile of the M1 render pipeline:
//!   `anyrender::PaintScene` write → RGBA8 buffer → PNG bytes.
//!
//! Backend: caller supplies any `anyrender::ImageRenderer` (M1 default =
//! `anyrender_vello_cpu`). Signature is generic over `R: ImageRenderer` so
//! future GPU backends can be swapped without API break.
//!
//! Downstream consumers:
//! - `raikiri` umbrella `html_to_png` (M1 end-to-end pipeline)
//! - hello-world VRT (m1.14)
//! - determinism test (m1.13)
//! - rayon thread-count test (m1.18)

use anyrender::ImageRenderer;

/// Rasterize an `anyrender` scene into an RGBA8 pixel buffer via the supplied
/// backend.
///
/// The `paint` closure receives `&mut R::ScenePainter` and issues draw
/// commands via the `anyrender::PaintScene` trait.
///
/// # Buffer layout
///
/// `width * height * 4` bytes, tightly packed rows, premultiplied RGBA8
/// (the `anyrender_vello_cpu` convention that `encode_png` accepts).
///
/// # Panics
///
/// Panics if the supplied renderer panics — e.g. zero dimensions passed
/// to `VelloCpuImageRenderer::new`. The wrapper adds no additional
/// validation; VRT usage prefers fail-fast over `Result`.
pub fn rasterize<R: ImageRenderer>(
    renderer: &mut R,
    paint: impl for<'a> FnOnce(&mut R::ScenePainter<'a>),
) -> Vec<u8> {
    let mut buf = Vec::new();
    renderer.render_to_vec(paint, &mut buf);
    buf
}

#[cfg(test)]
mod tests {
    use anyrender::{ImageRenderer, PaintScene};
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Rect};
    use peniko::{Color, Fill, color::palette::css};

    use super::rasterize;

    const W: u32 = 100;
    const H: u32 = 100;

    /// Deliberately trivial scene: a solid axis-aligned red rectangle.
    /// If this is not deterministic then nothing else in the pipeline will be.
    fn draw_red_rect<S: PaintScene>(scene: &mut S) {
        let color: Color = css::RED;
        let rect = Rect::new(10.0, 10.0, 90.0, 90.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }

    #[test]
    fn rasterize_produces_expected_buffer_shape() {
        let mut renderer = VelloCpuImageRenderer::new(W, H);
        let buf = rasterize(&mut renderer, |scene| draw_red_rect(scene));
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
    fn rasterize_is_byte_identical() {
        let mut r1 = VelloCpuImageRenderer::new(W, H);
        let mut r2 = VelloCpuImageRenderer::new(W, H);
        let a = rasterize(&mut r1, |scene| draw_red_rect(scene));
        let b = rasterize(&mut r2, |scene| draw_red_rect(scene));
        assert_eq!(a.len(), b.len());
        assert!(
            a == b,
            "independent renderers diverged: {} of {} bytes differ",
            a.iter().zip(&b).filter(|(x, y)| x != y).count(),
            a.len(),
        );
    }
}
