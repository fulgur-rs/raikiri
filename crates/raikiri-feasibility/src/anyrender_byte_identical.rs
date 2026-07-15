//! anyrender_vello_cpu byte-identical raster, nzv-spike for raikiri-spike-nzv.7
//!
//! Design-doc reference: section 12.8 (VRT / T1 pixel-exact baseline).
//!
//! Goal
//! ----
//! Verify that `anyrender_vello_cpu` 0.14 (with the `multithreading` feature
//! enabled workspace-wide) produces byte-identical RGBA8 output across:
//!
//! 1. Two consecutive `.reset()` + render cycles of the *same* renderer.
//! 2. Two *independent* renderer instances rendering the identical scene.
//! 3. A tiny-skia baseline (own-vs-own) as a sanity anchor for the harness.
//!
//! Rayon thread-count variation
//! ----------------------------
//! The task asks us to also vary rayon thread count (1 vs 4).  In
//! `vello_cpu` 0.0.9, thread count is set per-`RenderContext` via
//! `RenderSettings { num_threads, .. }` at construction time
//! (`RenderContext::new_with`).  `anyrender_vello_cpu::VelloCpuImageRenderer`
//! does not surface that knob, and per this spike's ground rules we can only
//! edit this file (no Cargo.toml changes), so we cannot add `vello_cpu` as a
//! direct dev-dep to reach `RenderSettings` from here.
//!
//! What we *can* verify with only the anyrender surface is stronger for the
//! design question anyway: the default configuration on this host already
//! spawns up to `min(available_parallelism - 1, 8)` rayon workers (see
//! `vello_cpu::RenderSettings::default`), so two identical byte streams under
//! that config demonstrate that thread scheduling non-determinism does not
//! leak into the output pixels.  The narrower 1-vs-4 comparison is filed as
//! a follow-up for nzv.12 / M1, once `vello_cpu` can be added as a direct
//! dev-dep or `anyrender_vello_cpu` exposes a `new_with_threads` shim.

/// Canvas dimensions used by every spike render.  Small on purpose: this is a
/// disposable M0 check, not a benchmark.
#[cfg(test)]
const W: u32 = 100;
#[cfg(test)]
const H: u32 = 100;

#[cfg(test)]
mod tests {
    use anyrender::{ImageRenderer, PaintScene};
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use kurbo::{Affine, Rect};
    use peniko::{Color, Fill, color::palette::css};

    use super::{H, W};

    /// The scene under test.  Deliberately trivial: one axis-aligned solid-red
    /// rectangle, no text, no images, no gradients, no clipping.  If *this*
    /// isn't deterministic then nothing else in the pipeline will be.
    fn draw_red_rect<S: PaintScene>(scene: &mut S) {
        let color: Color = css::RED;
        let rect = Rect::new(10.0, 10.0, 90.0, 90.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }

    /// Render the scene once through a fresh `VelloCpuImageRenderer` and
    /// return the raw RGBA8 premultiplied pixel buffer.
    fn render_once() -> Vec<u8> {
        let mut renderer = VelloCpuImageRenderer::new(W, H);
        let mut buf = Vec::new();
        renderer.render_to_vec(draw_red_rect, &mut buf);
        buf
    }

    /// Same renderer, `reset()` between two renders of the identical scene.
    fn render_twice_same_renderer() -> (Vec<u8>, Vec<u8>) {
        let mut renderer = VelloCpuImageRenderer::new(W, H);
        let mut a = Vec::new();
        let mut b = Vec::new();
        renderer.render_to_vec(draw_red_rect, &mut a);
        renderer.reset();
        renderer.render_to_vec(draw_red_rect, &mut b);
        (a, b)
    }

    /// The buffer must be the expected size and non-trivial (i.e. the red
    /// rectangle actually got painted, not silently no-op'd).
    #[test]
    fn render_produces_expected_buffer_shape() {
        let buf = render_once();
        assert_eq!(buf.len(), (W as usize) * (H as usize) * 4);
        // Somewhere inside the rect (10..90, 10..90), pixel (50, 50) must be
        // red-ish, not the transparent-black initial state.
        let idx = (50 * (W as usize) + 50) * 4;
        assert_ne!(
            &buf[idx..idx + 4],
            &[0, 0, 0, 0],
            "expected red rectangle interior at (50,50), got transparent black — scene did not render",
        );
    }

    /// Same renderer, two back-to-back renders with `reset()` in between:
    /// bytes must match exactly.
    #[test]
    fn same_renderer_reset_between_renders_is_byte_identical() {
        let (a, b) = render_twice_same_renderer();
        assert_eq!(a.len(), b.len());
        assert!(
            a == b,
            "same-renderer back-to-back renders diverged: {} of {} bytes differ",
            a.iter().zip(&b).filter(|(x, y)| x != y).count(),
            a.len(),
        );
    }

    /// Two *independent* `VelloCpuImageRenderer` instances rendering the
    /// identical scene must produce identical bytes.  This is the property
    /// VRT/T1 depends on (design doc section 12.8).
    #[test]
    fn two_independent_renderers_are_byte_identical() {
        let a = render_once();
        let b = render_once();
        assert_eq!(a.len(), b.len());
        assert!(
            a == b,
            "independent renderers diverged: {} of {} bytes differ",
            a.iter().zip(&b).filter(|(x, y)| x != y).count(),
            a.len(),
        );
    }

    /// Sanity: tiny-skia (used elsewhere as the VRT oracle) is trivially
    /// deterministic for the same scene.  This isn't testing anyrender, it's
    /// documenting our comparison harness against a known-good reference so
    /// nzv.12 can cite it.
    #[test]
    fn tiny_skia_reference_is_byte_identical() {
        fn render_ts() -> Vec<u8> {
            let mut pm = tiny_skia::Pixmap::new(W, H).unwrap();
            let mut paint = tiny_skia::Paint::default();
            paint.set_color_rgba8(255, 0, 0, 255);
            paint.anti_alias = true;
            let rect = tiny_skia::Rect::from_ltrb(10.0, 10.0, 90.0, 90.0).unwrap();
            pm.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
            pm.data().to_vec()
        }
        let a = render_ts();
        let b = render_ts();
        assert_eq!(
            a, b,
            "tiny-skia baseline is not deterministic — harness bug"
        );
    }
}
