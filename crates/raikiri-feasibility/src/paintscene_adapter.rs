//! anyrender PaintScene adapter compile-spike, nzv-spike for raikiri-spike-nzv.11
//!
//! # Goal
//!
//! Verify that the [`anyrender::PaintScene`] trait (from `anyrender 0.11`) and the
//! paint-consumer surface expected by `blitz-paint 0.3.0-beta.1` can be bridged by
//! a thin adapter (design doc M0 review 4 requirement).
//!
//! # Finding (summary)
//!
//! `blitz-paint 0.3.0-beta.1` does **not** define its own paint-consumer trait.
//! Its public entry point [`blitz_paint::paint_scene`] and all internal helpers
//! consume `&mut impl anyrender::PaintScene` directly (see the workspace crate
//! index; e.g. `blitz-paint/src/lib.rs` line 42, `blitz-paint/src/render.rs`
//! line 113 `BlitzDomPainter::paint_scene(scene: &mut impl PaintScene)`).
//!
//! That means the two surfaces are **1:1 identical**: the blitz-paint side is
//! literally the anyrender side. A `RaikiriPaintAdapter<S: PaintScene>` is
//! therefore a pure delegating wrapper — no divergence, no missing methods,
//! no unsafe, no `mem::transmute`. Any type that implements `anyrender::PaintScene`
//! (e.g. the vello_cpu / vello / vello_hybrid backends) is directly usable as
//! the paint consumer for blitz-paint.
//!
//! # What this spike shows
//!
//! 1. A generic newtype `RaikiriPaintAdapter<S>` that implements
//!    [`anyrender::RenderContext`] and [`anyrender::PaintScene`] by delegating
//!    to an inner `S: PaintScene`.
//! 2. A `const _: fn() = ...` compile-time assertion that
//!    `RaikiriPaintAdapter<S>` satisfies the `PaintScene` bound expected by
//!    `blitz_paint::paint_scene` (i.e. `impl PaintScene`).
//! 3. A runtime smoke test wiring `RaikiriPaintAdapter<Scene>` (the recording
//!    backend built into anyrender) and issuing a `fill` command, proving the
//!    delegation compiles and executes.
//!
//! # Implication for the design doc / M1
//!
//! Design-doc review 4 asked whether a bridge would be needed between two
//! distinct trait surfaces. It is not: `blitz-paint` reuses `anyrender::PaintScene`
//! as its consumer contract, so raikiri's paint path can hand a blitz-paint
//! call any `impl PaintScene` (including a raikiri-owned newtype for
//! instrumentation / paged-media routing) without any translation layer.

use std::sync::Arc;

use anyrender::{
    Filter, Glyph, NormalizedCoord, PaintRef, PaintScene, RegisterResourceError, RenderContext,
    ResourceId, Scene,
};
use kurbo::{Affine, Rect, Shape, Stroke};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};

/// Thin delegating adapter over any `S: anyrender::PaintScene`.
///
/// Exists to prove that no translation layer is required between anyrender 0.11
/// and blitz-paint 0.3.0-beta.1. Every method forwards 1:1 to `self.inner`.
pub struct RaikiriPaintAdapter<S: PaintScene> {
    /// The wrapped paint sink. Any `anyrender::PaintScene` impl works —
    /// in production this would be a backend such as `anyrender_vello_cpu`'s
    /// scene painter, in tests it can be `anyrender::Scene` (the recording backend).
    pub inner: S,
}

impl<S: PaintScene> RaikiriPaintAdapter<S> {
    /// Wrap an existing paint sink.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }

    /// Recover the wrapped paint sink.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

// --- RenderContext (super-trait of PaintScene) ---

impl<S: PaintScene> RenderContext for RaikiriPaintAdapter<S> {
    fn try_register_custom_resource(
        &mut self,
        resource: Box<dyn std::any::Any>,
    ) -> Result<ResourceId, RegisterResourceError> {
        self.inner.try_register_custom_resource(resource)
    }

    fn unregister_resource(&mut self, resource_id: ResourceId) {
        self.inner.unregister_resource(resource_id)
    }

    fn renderer_specific_context(&self) -> Option<Box<dyn std::any::Any>> {
        self.inner.renderer_specific_context()
    }
}

// --- PaintScene (the surface blitz-paint drives) ---

impl<S: PaintScene> PaintScene for RaikiriPaintAdapter<S> {
    fn reset(&mut self) {
        self.inner.reset()
    }

    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        filter: Option<Arc<Filter>>,
        backdrop_filter: Option<Arc<Filter>>,
    ) {
        self.inner
            .push_layer(blend, alpha, transform, clip, filter, backdrop_filter)
    }

    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        self.inner.push_clip_layer(transform, clip)
    }

    fn pop_layer(&mut self) {
        self.inner.pop_layer()
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.inner
            .stroke(style, transform, brush, brush_transform, shape)
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.inner
            .fill(style, transform, brush, brush_transform, shape)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        embolden: kurbo::Vec2,
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph> + Clone,
    ) {
        self.inner.draw_glyphs(
            font,
            font_size,
            hint,
            normalized_coords,
            embolden,
            style,
            brush,
            brush_alpha,
            transform,
            glyph_transform,
            glyphs,
        )
    }

    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.inner
            .draw_box_shadow(transform, rect, brush, radius, std_dev)
    }
}

// --- Compile-time assertion: the adapter is usable wherever `blitz-paint`
// expects an `impl anyrender::PaintScene`. Because `blitz-paint 0.3.0-beta.1`
// reuses `anyrender::PaintScene` verbatim (see `blitz_paint::paint_scene`
// signature `scene: &mut impl PaintScene`), satisfying the anyrender bound
// is equivalent to satisfying the blitz-paint bound. ---

const _ASSERT_ADAPTER_IS_PAINTSCENE: fn() = || {
    fn takes_paint_scene<T: PaintScene>() {}
    // Same bound blitz_paint::paint_scene requires of its `scene` argument.
    takes_paint_scene::<RaikiriPaintAdapter<Scene>>();
};

// --- Runtime evidence ---

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;
    use peniko::{Color, Fill};

    /// Sanity check: `Scene` (anyrender's recording backend) implements
    /// `PaintScene`, so `RaikiriPaintAdapter<Scene>` does too, and forwarded
    /// calls hit the inner sink without panicking.
    #[test]
    fn adapter_forwards_fill_to_inner() {
        let mut adapter = RaikiriPaintAdapter::new(Scene::new());
        adapter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(255, 0, 0),
            None,
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        // Non-empty command buffer proves the delegation reached the inner sink.
        let inner = adapter.into_inner();
        assert!(
            !inner.commands.is_empty(),
            "adapter.fill(...) should have appended one RenderCommand to the inner Scene"
        );
    }

    /// Sanity check for the layer-stack methods (push then pop).
    #[test]
    fn adapter_forwards_clip_layer_push_pop() {
        let mut adapter = RaikiriPaintAdapter::new(Scene::new());
        adapter.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 5.0, 5.0));
        adapter.pop_layer();
        let inner = adapter.into_inner();
        assert_eq!(
            inner.commands.len(),
            2,
            "push_clip_layer + pop_layer should record 2 commands"
        );
    }

    /// Compile-time confirmation from inside the test module too, so a
    /// regression in the trait shape shows up as a test-build failure and not
    /// only as a library-build failure.
    #[test]
    fn adapter_satisfies_paintscene_bound() {
        fn assert_paint_scene<T: PaintScene>() {}
        assert_paint_scene::<RaikiriPaintAdapter<Scene>>();
    }
}
