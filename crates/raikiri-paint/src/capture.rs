//! A paint target that can record a stretch of paint apart, so the painter
//! can place it later, such as inside an opacity group that is composited
//! once the rest of its content is known.

use anyrender::{Glyph, NormalizedCoord, PaintRef, PaintScene, RenderContext, Scene};
use kurbo::{Affine, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};
use std::{any::Any, sync::Arc};

pub(crate) struct CaptureScene<'a, S> {
    scene: &'a mut S,
    /// Open captures, innermost last. Paint goes to the innermost one, or
    /// to `scene` when none is open.
    captures: Vec<Scene>,
}

impl<'a, S> CaptureScene<'a, S> {
    pub(crate) fn new(scene: &'a mut S) -> Self {
        Self {
            scene,
            captures: Vec::new(),
        }
    }

    /// Record the following paint apart until [`Self::end_capture`].
    pub(crate) fn begin_capture(&mut self) {
        self.captures.push(Scene::new());
    }

    /// The paint recorded since the matching [`Self::begin_capture`].
    pub(crate) fn end_capture(&mut self) -> Scene {
        self.captures.pop().unwrap_or_default()
    }
}

/// Call `$method` on the innermost capture, or on the wrapped scene.
macro_rules! forward {
    ($self:ident.$method:ident($($arg:expr),* $(,)?)) => {
        match $self.captures.last_mut() {
            Some(capture) => capture.$method($($arg),*),
            None => $self.scene.$method($($arg),*),
        }
    };
}

impl<S: RenderContext> RenderContext for CaptureScene<'_, S> {
    fn try_register_custom_resource(
        &mut self,
        resource: Box<dyn Any>,
    ) -> Result<anyrender::ResourceId, anyrender::RegisterResourceError> {
        self.scene.try_register_custom_resource(resource)
    }
    fn unregister_resource(&mut self, resource_id: anyrender::ResourceId) {
        self.scene.unregister_resource(resource_id);
    }
    fn renderer_specific_context(&self) -> Option<Box<dyn Any>> {
        self.scene.renderer_specific_context()
    }
}

impl<S: PaintScene> PaintScene for CaptureScene<'_, S> {
    fn reset(&mut self) {
        self.captures.clear();
        self.scene.reset();
    }
    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        filter: Option<Arc<anyrender::Filter>>,
        backdrop_filter: Option<Arc<anyrender::Filter>>,
    ) {
        forward!(self.push_layer(blend, alpha, transform, clip, filter, backdrop_filter));
    }
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        forward!(self.push_clip_layer(transform, clip));
    }
    fn pop_layer(&mut self) {
        forward!(self.pop_layer());
    }
    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        forward!(self.stroke(style, transform, brush, brush_transform, shape));
    }
    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        forward!(self.fill(style, transform, brush, brush_transform, shape));
    }
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        embolden: Vec2,
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph> + Clone,
    ) {
        forward!(self.draw_glyphs(
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
        ));
    }
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        forward!(self.draw_box_shadow(transform, rect, brush, radius, std_dev));
    }
    fn append_scene(&mut self, scene: Scene, scene_transform: Affine) {
        forward!(self.append_scene(scene, scene_transform));
    }
}

#[cfg(test)]
mod tests;
