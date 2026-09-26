use super::*;

use anyrender::PaintScene;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use kurbo::{Affine, Rect};
use peniko::{Color, Fill, color::palette::css};

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
    let buf =
        anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(|scene| draw_red_rect(scene), W, H);
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
    let a =
        anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(|scene| draw_red_rect(scene), W, H);
    #[allow(clippy::redundant_closure)]
    let b =
        anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(|scene| draw_red_rect(scene), W, H);
    assert_eq!(a.len(), b.len());
    assert!(
        a == b,
        "independent renderers diverged: {} of {} bytes differ",
        a.iter().zip(&b).filter(|(x, y)| x != y).count(),
        a.len(),
    );
}

/// Pixels outside the drawn shape must be transparent-black; the historical
/// `rasterize(&mut R, ...)` API (since removed) could leak
/// prior renders' pixels into that region across calls sharing one renderer.
/// `anyrender::render_to_buffer` constructs a fresh renderer per call, so
/// the leaked-pixel scenario is structurally impossible at this API surface.
/// This test pins that property at the integration boundary raikiri-vrt
/// consumes.
#[test]
fn render_to_buffer_leaves_untouched_pixels_transparent() {
    fn draw_blue_rect<S: PaintScene>(scene: &mut S) {
        let color: Color = css::BLUE;
        let rect = Rect::new(0.0, 0.0, 50.0, 50.0);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }

    #[allow(clippy::redundant_closure)]
    let blue_buf = anyrender::render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| draw_blue_rect(scene),
        W,
        H,
    );

    // Pixel (75, 75) is outside blue rect `[0..50, 0..50]` — must be
    // transparent-black. A non-transparent value would indicate the renderer
    // started from a non-empty state.
    let idx_outside = (75 * (W as usize) + 75) * 4;
    let outside = &blue_buf[idx_outside..idx_outside + 4];
    assert_eq!(
        outside,
        &[0, 0, 0, 0],
        "blue_buf(75,75) not transparent — untouched region carried non-empty pixels. Got: {outside:?}",
    );

    // Sanity: pixel (25, 25) is inside blue rect — must be non-transparent.
    let idx_inside = (25 * (W as usize) + 25) * 4;
    let inside = &blue_buf[idx_inside..idx_inside + 4];
    assert_ne!(
        inside,
        &[0, 0, 0, 0],
        "blue_buf(25,25) transparent — blue rect did not render at all",
    );
}

/// Spec §5.2 — pinning rayon worker count and verifying byte-identity.
///
/// `anyrender_vello_cpu::VelloCpuImageRenderer` internally constructs
/// `vello_cpu::RenderContext::new(w, h)` which uses `RenderSettings::default()`
/// (num_threads = `min(available_parallelism - 1, 8)`). To check the worker count
/// we bypass `VelloCpuImageRenderer::new` and construct `RenderContext::new_with`
/// directly, then wrap it in `VelloCpuScenePainter` (whose fields are `pub`) to
/// stay on the exact same production render path (`draw_fn → flush →
/// render(PixmapMut)`) that `VelloCpuImageRenderer::render` uses
/// (`render` uses default `RasterizerSettings`, i.e. `OptimizeSpeed`).
///
/// Both `num_threads: 1` and `num_threads: 4` route through
/// `MultiThreadedDispatcher` (only `num_threads == 0` selects
/// `SingleThreadedDispatcher`), so this test verifies rayon work-stealing
/// determinism, not scalar-vs-rayon parity.
fn render_with_threads(num_threads: u16) -> Vec<u8> {
    use anyrender_vello_cpu::VelloCpuScenePainter;
    use vello_cpu::{PixmapMut, RenderContext, RenderSettings, Resources};

    let settings = RenderSettings {
        num_threads,
        ..Default::default()
    };
    let render_ctx = RenderContext::new_with(W as u16, H as u16, settings);
    let mut scene = VelloCpuScenePainter {
        render_ctx,
        resources: Resources::new(),
    };
    draw_red_rect(&mut scene);
    scene.render_ctx.flush();
    let mut buf = vec![0u8; (W as usize) * (H as usize) * 4];
    let (w, h) = (scene.render_ctx.width(), scene.render_ctx.height());
    scene.render_ctx.render(
        PixmapMut::new(w, h, &mut buf).unwrap(),
        &mut scene.resources,
    );
    buf
}

/// Sanity: pinned-thread render still produces a non-empty buffer with the
/// red rectangle drawn. Guards against silent no-op regressions in the
/// pinned path (e.g. flush order, dispatcher initialization).
#[test]
fn rayon_thread_pin_produces_expected_buffer_shape() {
    let buf = render_with_threads(4);
    assert_eq!(buf.len(), (W as usize) * (H as usize) * 4);
    let idx = (50 * (W as usize) + 50) * 4;
    assert_ne!(
        &buf[idx..idx + 4],
        &[0, 0, 0, 0],
        "pinned num_threads=4 render left (50,50) transparent — scene did not render",
    );
}

/// spec §5.2 — pinning rayon at 1 and 4 workers must give byte-identical
/// output. Both paths use `MultiThreadedDispatcher`; a divergence would mean
/// rayon scheduling non-determinism leaks into pixel output, invalidating
/// the VRT/T1 pixel-exact baseline (design doc §12.8).
#[test]
fn rayon_thread_pin_1_vs_4_is_byte_identical() {
    let a = render_with_threads(1);
    let b = render_with_threads(4);
    assert_eq!(a.len(), b.len());
    assert!(
        a == b,
        "rayon 1 vs 4 threads diverged: {} of {} bytes differ",
        a.iter().zip(&b).filter(|(x, y)| x != y).count(),
        a.len(),
    );
}

#[test]
fn encode_png_roundtrips_rgba_buffer() {
    let rgba = vec![255, 0, 0, 255, 0, 0, 255, 255];
    let png = encode_png(&rgba, 2, 1);
    let pixmap = tiny_skia::Pixmap::decode_png(&png).expect("decode own output");
    assert_eq!((pixmap.width(), pixmap.height()), (2, 1));
    assert_eq!(pixmap.data(), rgba.as_slice());
}

#[test]
#[should_panic(expected = "expected 4 bytes")]
fn encode_png_rejects_short_buffer() {
    encode_png(&[0u8; 3], 1, 1);
}

#[test]
#[should_panic(expected = "width/height must be > 0")]
fn encode_png_rejects_zero_dimensions() {
    encode_png(&[], 0, 1);
}
