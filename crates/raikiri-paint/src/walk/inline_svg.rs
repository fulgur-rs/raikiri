//! The painting of an inline `<svg>` root, rasterized from its subtree.

use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use raikiri_dom::Document;
use raikiri_dom::inline_svg_cache::InlineSvgRaster;
use raikiri_dom::inline_svg_style::InlineSvgElementDeclarations;
use raikiri_layout_geometry::Edges;
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::{DecodedImage, NodeId, RenderWarning, ResourceKind, WarningKind};
use std::sync::Arc;

/// One inline SVG root, painted as a raster of its subtree in the content
/// box of the element.
///
/// A root that is not visible still shows the descendants that set
/// `visibility` themselves, so its subtree is rasterized anyway. The
/// element's own opacity, backgrounds and borders belong to the outer paint
/// group, not to this raster.
pub(super) struct InlineSvg<'a> {
    document: &'a Document,
    cascade: &'a CascadeResult,
    cv: &'a ComputedValues,
    node_id: usize,
    content: ContentBox,
}

/// The content box of the element in page pixels.
#[derive(Clone, Copy)]
struct ContentBox {
    x: f64,
    y: f64,
    width: f32,
    height: f32,
}

impl<'a> InlineSvg<'a> {
    /// The inline SVG root `node_id`, whose border box is `width` × `height`
    /// at (`abs_x`, `abs_y`) with the given `padding`.
    pub(super) fn new(
        document: &'a Document,
        cascade: &'a CascadeResult,
        cv: &'a ComputedValues,
        node_id: usize,
        (width, height): (f32, f32),
        (abs_x, abs_y): (f32, f32),
        padding: &Edges<f32>,
    ) -> Self {
        let bl = cv.border.left.width().px();
        let bt = cv.border.top.width().px();
        let br = cv.border.right.width().px();
        let bb = cv.border.bottom.width().px();
        Self {
            document,
            cascade,
            cv,
            node_id,
            content: ContentBox {
                x: (abs_x + bl + padding.left) as f64,
                y: (abs_y + bt + padding.top) as f64,
                width: (width - bl - br - padding.left - padding.right).max(0.0),
                height: (height - bt - bb - padding.top - padding.bottom).max(0.0),
            },
        }
    }

    /// Paints the root. A root with nothing to show paints nothing, and a
    /// visible one that cannot be rasterized leaves a warning. The result is
    /// always `true`: the element's content counts as painted either way.
    pub(super) fn paint(
        &self,
        scene: &mut impl PaintScene,
        warnings: &mut Vec<RenderWarning>,
    ) -> bool {
        let raster = (!self.is_blank())
            .then(|| self.rasterize(warnings))
            .flatten();
        if let Some(raster) = raster {
            self.draw(scene, raster);
        }
        true
    }

    /// Whether the root is transparent or its content box has no area.
    fn is_blank(&self) -> bool {
        self.cv.opacity <= 0.0 || self.content.width <= 0.0 || self.content.height <= 0.0
    }

    /// The raster of the subtree, or `None` when nothing is to be drawn.
    fn rasterize(&self, warnings: &mut Vec<RenderWarning>) -> Option<InlineSvgRaster> {
        let visible = !raikiri_dom::paint_rules::hides_own_box(self.cv);

        let source = self
            .document
            .cached_svg_subtree(self.node_id)
            .ok()
            .flatten();
        // The declarations the page's stylesheets give to the descendants, which
        // the serialized source does not carry. A subtree whose declarations
        // outgrow the size bound is drawn unstyled, as it always was.
        let descendants = raikiri_dom::inline_svg_style::inline_svg_element_declarations(
            self.document,
            self.cascade,
            self.node_id,
        )
        .unwrap_or_default();
        // A root that is not visible shows only the descendants that set
        // `visibility` themselves, so a source that never mentions it paints
        // nothing.
        if !visible && !mentions_visibility(source.as_deref(), &descendants) {
            return None;
        }
        // Compare all current rasterizer inputs exactly. Public cascade values
        // are mutable without changing their generation, so styles are collected
        // again on every paint rather than trusting a generation-only key.
        let style_key = format!(
            "{:?}|{:?}|{:?}",
            self.root_style(visible),
            (
                self.cv.font_size.0.to_bits(),
                self.cv.font_weight.to_bits(),
                self.cv.font_style.as_css_str(),
                raikiri_dom::inline_svg_style::svg_font_family(self.cv)
            ),
            descendants
        );
        let viewport_key = (self.content.width.to_bits(), self.content.height.to_bits());
        if let Some(raster) =
            self.document
                .cached_inline_svg_raster(self.node_id, viewport_key, &style_key)
        {
            return Some(raster);
        }
        let Some(decoded) = source.and_then(|source| self.decode(&source, &descendants, visible))
        else {
            // Nothing was expected to show from a root that is not visible.
            if visible {
                warnings.push(fallback_warning(
                    self.node_id,
                    "inline SVG could not be parsed or rasterized",
                ));
            }
            return None;
        };

        // A root that is not visible draws only the descendants that are
        // visible themselves, so most of these paint nothing at all.
        let decoded = require_pixels(decoded, self.node_id, warnings)
            .filter(|decoded| visible || has_ink(decoded))?;
        let raster = InlineSvgRaster {
            width: decoded.width,
            height: decoded.height,
            rgba: Arc::new(decoded.rgba),
        };
        self.document.cache_inline_svg_raster(
            self.node_id,
            viewport_key,
            style_key,
            raster.clone(),
        );
        Some(raster)
    }

    /// Parses `source` and rasterizes it at the size of the content box, with
    /// the styles the cascade resolved for the root and its descendants.
    fn decode(
        &self,
        source: &str,
        descendants: &[InlineSvgElementDeclarations],
        visible: bool,
    ) -> Option<DecodedImage> {
        let svg = raikiri_svg::SvgDocument::parse(source.as_bytes()).ok()?;
        let elements: Vec<_> = descendants
            .iter()
            .map(|descendant| raikiri_svg::SvgElementStyle {
                element_index: descendant.element_index,
                declarations: &descendant.declarations,
            })
            .collect();
        svg.rasterize_at_css_pixel_scale_with_resolved_styles(
            raikiri_svg::SvgViewport {
                width: self.content.width,
                height: self.content.height,
            },
            self.root_style(visible),
            raikiri_svg::SvgResolvedStyles {
                root_color: self.inherited_color(),
                font: raikiri_svg::SvgRootFont {
                    size: self.cv.font_size.0,
                    weight: self.cv.font_weight,
                    style: self.cv.font_style.as_css_str(),
                    family: &raikiri_dom::inline_svg_style::svg_font_family(self.cv),
                },
                elements: &elements,
            },
            None,
        )
        .ok()
    }

    /// The style of the SVG root that the host element decides.
    fn root_style(&self, visible: bool) -> raikiri_svg::SvgRootStyle {
        raikiri_svg::SvgRootStyle {
            inherited_color: self.inherited_color(),
            // The host opacity remains the SVG root's inherited value
            // while the rasterizer removes its group alpha. The outer
            // paint group applies it with backgrounds and borders.
            opacity: self.cv.opacity,
            neutralize_root_opacity: self
                .cascade
                .opacity_specified
                .get(self.node_id)
                .copied()
                .unwrap_or(false),
            host_controls_root_background: self
                .cascade
                .background_color_specified
                .get(self.node_id)
                .copied()
                .unwrap_or(false),
            visible,
        }
    }

    /// The inherited CSS `color` of the root, in straight RGBA8.
    fn inherited_color(&self) -> [u8; 4] {
        let color = self.cv.color;
        [color.r, color.g, color.b, color.a]
    }

    /// Fills the content box with `decoded`, clipped to its exact extent.
    fn draw(&self, scene: &mut impl PaintScene, decoded: InlineSvgRaster) {
        let ContentBox {
            x: content_x,
            y: content_y,
            width: content_w,
            height: content_h,
        } = self.content;
        let raster_w = f64::from(decoded.width);
        let raster_h = f64::from(decoded.height);
        let image_data = peniko::ImageData {
            data: peniko::Blob::new(decoded.rgba),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: decoded.width,
            height: decoded.height,
        };
        let brush = peniko::ImageBrush::new(image_data);
        let clip = Rect::new(
            content_x,
            content_y,
            content_x + f64::from(content_w),
            content_y + f64::from(content_h),
        );
        scene.push_clip_layer(Affine::IDENTITY, &clip);
        scene.fill(
            peniko::Fill::NonZero,
            // SVG pixels retain the CSS pixel grid. The buffer is rounded up,
            // while the clip above keeps the exact fractional viewport extent.
            Affine::translate((content_x, content_y)),
            brush.as_ref(),
            None,
            &Rect::new(0.0, 0.0, raster_w, raster_h),
        );
        scene.pop_layer();
    }
}

/// Whether the source or the declarations of a descendant mention
/// `visibility`.
fn mentions_visibility(source: Option<&str>, descendants: &[InlineSvgElementDeclarations]) -> bool {
    source.is_some_and(|source| source.contains("visibility"))
        || descendants
            .iter()
            .any(|descendant| descendant.declarations.contains("visibility"))
}

/// The warning for an image that the root `node_id` could not show.
fn fallback_warning(node_id: usize, details: &str) -> RenderWarning {
    RenderWarning::new(
        WarningKind::ResourceFallback {
            kind: ResourceKind::Image,
            url: None,
        },
        details.to_owned(),
    )
    .node_id(Some(NodeId::new(node_id as u64)))
}

/// `decoded` when it has pixels. Otherwise it is dropped, with a warning for
/// the root `node_id`.
fn require_pixels(
    decoded: DecodedImage,
    node_id: usize,
    warnings: &mut Vec<RenderWarning>,
) -> Option<DecodedImage> {
    if decoded.width > 0 && decoded.height > 0 {
        return Some(decoded);
    }
    warnings.push(fallback_warning(
        node_id,
        "inline SVG produced an empty raster size",
    ));
    None
}

/// Whether any pixel of the raster is not fully transparent.
fn has_ink(raster: &DecodedImage) -> bool {
    raster.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0)
}

#[cfg(test)]
mod tests;
