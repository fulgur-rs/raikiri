//! The painting of one laid-out page-margin box: its background color and
//! images, borders, then its text and shadows.

use super::{
    ImageBrushCache, clip_inset_rect, css_color, origin_inset_rect, paint_background_from_source,
    push_overflow_clip, redacted_image_url, warn_unpainted_background_layers,
};
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use raikiri_dom::MarginBox;
use raikiri_style::property::VisualBox;
use raikiri_traits::{
    ImagePixelSource, PageBox, PaintRect, RenderWarning, ResourceKind, WarningKind,
};

/// Paint one laid-out page-margin box: background color, background image,
/// borders, then its text and shadows clipped when its overflow clips.
pub(super) fn paint_margin_box(
    image_cache: &ImageBrushCache,
    scene: &mut impl PaintScene,
    margin_box: &MarginBox,
    page_box: &PageBox,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let Some(painter) = MarginBoxPaint::new(margin_box) else {
        return;
    };
    painter.paint_color(scene, warnings);
    // The layout already dropped the layers under the painted ones.
    warn_unpainted_background_layers(margin_box.background_layer_count, warnings);
    painter.paint_images(image_cache, scene, pixel_source, warnings);
    painter.paint_borders(scene);
    // Text and its shadows overflow the box unless `overflow` clips them
    // (CSS Overflow 3 §3: the initial `visible` does not clip).
    let clipped = margin_box.overflow_clip.is_some_and(|clip| {
        push_overflow_clip(
            &mut crate::transform::TransformScene {
                scene: &mut *scene,
                transform: Affine::IDENTITY,
            },
            clip,
            page_box,
        );
        true
    });
    painter.paint_text(scene);
    painter.paint_content_image(scene);
    if clipped {
        scene.pop_layer();
    }
}

/// A margin box, and the geometry its background and borders are worked out
/// from.
struct MarginBoxPaint<'a> {
    margin_box: &'a MarginBox,
    /// The border box.
    rect: Rect,
    /// The border widths `(left, top, right, bottom)`.
    border: (f64, f64, f64, f64),
    /// The padding `(left, top, right, bottom)`.
    padding: (f64, f64, f64, f64),
}

impl<'a> MarginBoxPaint<'a> {
    /// The geometry of `margin_box`, or `None` for a box without an area.
    fn new(margin_box: &'a MarginBox) -> Option<Self> {
        let PaintRect {
            x,
            y,
            width,
            height,
            ..
        } = margin_box.rect;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let borders = margin_box.border_widths();
        Some(Self {
            margin_box,
            rect: Rect::new(x as f64, y as f64, (x + width) as f64, (y + height) as f64),
            border: (
                borders.left as f64,
                borders.top as f64,
                borders.right as f64,
                borders.bottom as f64,
            ),
            padding: (
                margin_box.padding.left as f64,
                margin_box.padding.top as f64,
                margin_box.padding.right as f64,
                margin_box.padding.bottom as f64,
            ),
        })
    }

    /// Paint the background color. The bottom layer's clip also clips the
    /// color (CSS Backgrounds 3 §2.1). The paint area honors
    /// `background-clip` like an element's does (the border box is `rect`),
    /// and an unsupported `text` clip warns and skips the color instead of
    /// silently omitting pixels.
    fn paint_color(&self, scene: &mut impl PaintScene, warnings: &mut Vec<RenderWarning>) {
        let margin_box = self.margin_box;
        let (rect, border, padding) = (self.rect, self.border, self.padding);
        let Some(color) = margin_box.background_color else {
            return;
        };
        match clip_inset_rect(rect, margin_box.background_clip, border, padding) {
            Some(painting) => {
                if painting.x1 > painting.x0 && painting.y1 > painting.y0 {
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        css_color(color),
                        None,
                        &painting,
                    );
                }
            }
            // `border-area` paints the color in the border ring only, as
            // the element path does.
            None if margin_box.background_clip == VisualBox::BorderArea => {
                let inner = Rect::new(
                    rect.x0 + border.0,
                    rect.y0 + border.1,
                    rect.x1 - border.2,
                    rect.y1 - border.3,
                );
                let mut ring = kurbo::Shape::to_path(&rect, 0.1);
                if inner.x1 > inner.x0 && inner.y1 > inner.y0 {
                    ring.extend(kurbo::Shape::path_elements(&inner, 0.1));
                }
                scene.fill(
                    Fill::EvenOdd,
                    Affine::IDENTITY,
                    css_color(color),
                    None,
                    &ring,
                );
            }
            None => warnings.push(RenderWarning {
                kind: WarningKind::ResourceFallback {
                    kind: ResourceKind::Image,
                    url: None,
                },
                node_id: None,
                details:
                    "margin-box background-clip has an unsupported visual box; the color was skipped"
                        .into(),
            }),
        }
    }

    /// Paint the background image layers, the first on top (CSS Backgrounds
    /// 3 §2.1), or the green placeholder of a box that has no image source.
    /// Each layer honors `background-origin` and `background-clip` like an
    /// element's does; an unsupported `text` or `border-area` clip warns and
    /// skips the image.
    fn paint_images(
        &self,
        image_cache: &ImageBrushCache,
        scene: &mut impl PaintScene,
        pixel_source: Option<&dyn ImagePixelSource>,
        warnings: &mut Vec<RenderWarning>,
    ) {
        let margin_box = self.margin_box;
        let (rect, border, padding) = (self.rect, self.border, self.padding);
        if let Some(source) = pixel_source.filter(|_| !margin_box.background_layers.is_empty()) {
            let layers = margin_box
                .background_layers
                .iter()
                .rev()
                .filter_map(|image| Some((image, image_cache.image_url(&image.url)?)));
            for (image, url) in layers {
                let positioning =
                    origin_inset_rect(rect, image.origin, border, padding, Some(&url), warnings);
                match clip_inset_rect(rect, image.clip, border, padding) {
                    Some(painting) => {
                        if painting.x1 > painting.x0 && painting.y1 > painting.y0 {
                            paint_background_from_source(
                                image_cache,
                                scene,
                                &url,
                                positioning,
                                painting,
                                &image.size,
                                &image.position,
                                &image.repeat,
                                None,
                                source,
                                warnings,
                            );
                        }
                    }
                    None => {
                        warnings.push(RenderWarning {
                            kind: WarningKind::ResourceFallback {
                                kind: ResourceKind::Image,
                                url: Some(redacted_image_url(&url)),
                            },
                            node_id: None,
                            details:
                                "margin-box background-clip has an unsupported visual box; the image was skipped"
                                    .into(),
                        });
                    }
                }
            }
        } else if margin_box.lime_background {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(0, 255, 0, 255),
                None,
                &rect,
            );
        }
    }

    /// Paint the solid borders, in the order top, right, bottom, left.
    fn paint_borders(&self, scene: &mut impl PaintScene) {
        let PaintRect {
            x,
            y,
            width,
            height,
            ..
        } = self.margin_box.rect;
        for (side, border) in self.margin_box.borders.iter().enumerate() {
            let Some(border) = border else {
                continue;
            };
            let border_width = border.width;
            let border_rect = match side {
                0 => Rect::new(
                    x as f64,
                    y as f64,
                    (x + width) as f64,
                    (y + border_width) as f64,
                ),
                1 => Rect::new(
                    (x + width - border_width) as f64,
                    y as f64,
                    (x + width) as f64,
                    (y + height) as f64,
                ),
                2 => Rect::new(
                    x as f64,
                    (y + height - border_width) as f64,
                    (x + width) as f64,
                    (y + height) as f64,
                ),
                _ => Rect::new(
                    x as f64,
                    y as f64,
                    (x + border_width) as f64,
                    (y + height) as f64,
                ),
            };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                css_color(border.color),
                None,
                &border_rect,
            );
        }
    }

    /// Paint the text and its shadows.
    fn paint_text(&self, scene: &mut impl PaintScene) {
        let margin_box = self.margin_box;
        if let Some(text) = &margin_box.text {
            let (text_x, text_y) = text.origin();
            let shaped = text.shaped();
            let container = shaped.container();
            let text_bounds = Rect::new(
                f64::from(text_x),
                f64::from(text_y),
                f64::from(text_x + shaped.width().max(container.width)),
                f64::from(text_y + shaped.height().max(container.height)),
            );
            crate::standalone_text::draw_shadows(
                scene,
                shaped,
                text_x,
                text_y,
                &margin_box.text_shadows,
                text_bounds,
            );
            crate::standalone_text::draw(
                scene,
                shaped,
                text_x,
                text_y,
                css_color(margin_box.color),
            );
        }
    }

    /// Paint the green placeholder of the content image.
    fn paint_content_image(&self, scene: &mut impl PaintScene) {
        if let Some(image) = self.margin_box.lime_content_image {
            let image_rect = Rect::new(
                image.x as f64,
                image.y as f64,
                (image.x + image.width) as f64,
                (image.y + image.height) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(0, 255, 0, 255),
                None,
                &image_rect,
            );
        }
    }
}
