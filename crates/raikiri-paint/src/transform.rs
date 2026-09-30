//! Paint-only affine context. Geometry, brushes, glyphs and clips share the
//! same matrix; layout coordinates and the backend contract stay unchanged.

use anyrender::{Filter, Glyph, NormalizedCoord, PaintRef, PaintScene, RenderContext};
use kurbo::{Affine, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};
use std::{any::Any, sync::Arc};

pub(crate) struct TransformScene<'a, S> {
    pub scene: &'a mut S,
    pub transform: Affine,
}

impl<S: RenderContext> RenderContext for TransformScene<'_, S> {
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

impl<S: PaintScene> PaintScene for TransformScene<'_, S> {
    fn reset(&mut self) {
        self.scene.reset();
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
        self.scene.push_layer(
            blend,
            alpha,
            self.transform * transform,
            clip,
            filter,
            backdrop_filter,
        );
    }
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        self.scene.push_clip_layer(self.transform * transform, clip);
    }
    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }
    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.scene.stroke(
            style,
            self.transform * transform,
            brush,
            brush_transform,
            shape,
        );
    }
    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.scene.fill(
            style,
            self.transform * transform,
            brush,
            brush_transform,
            shape,
        );
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
        self.scene.draw_glyphs(
            font,
            font_size,
            hint,
            normalized_coords,
            embolden,
            style,
            brush,
            brush_alpha,
            self.transform * transform,
            glyph_transform,
            glyphs,
        );
    }
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.scene
            .draw_box_shadow(self.transform * transform, rect, brush, radius, std_dev);
    }
}

#[cfg(test)]
mod tests;
