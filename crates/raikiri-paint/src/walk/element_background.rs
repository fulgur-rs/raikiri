//! The painting of one background layer of an element: the color, and the
//! image or gradient over it, positioned by `background-origin` and clipped by
//! `background-clip`.

use super::{
    BackgroundEnv, ImageBrushCache, background_color_with_image_source, fill_rounded_background,
    gradient, paint_background_from_source, paint_conic_gradient, paint_element_border_rounded,
    redacted_image_url, rounded_background_path, used_border_radii,
};
use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Rect};
use peniko::{BlendMode, Color, Compose, Fill, Mix};
use raikiri_style::property::{
    BackgroundImage, BackgroundRepeat, BorderColor, BorderStyle, CssColor, Gradient, Sides,
    VisualBox,
};
use raikiri_style::resolve::ComputedBorder;
use raikiri_style::{
    ComputedBackgroundSize, ComputedBorderRadius, ComputedCssPosition, ComputedLength,
};
use raikiri_traits::{ImagePixelSource, RenderWarning, ResourceKind, WarningKind};

/// Four distances `(left, top, right, bottom)` from the sides of a box.
type Inset = (f64, f64, f64, f64);

/// One background layer of an element, and the box and surroundings it is
/// painted in. The box is the border box at (`abs_x`, `abs_y`) of size
/// `width` by `height`, which are positive: the caller paints no layer for an
/// empty box. `border` and `padding` are the widths that `background-origin`
/// and `background-clip` take off it.
#[derive(Clone, Copy)]
pub(super) struct ElementBackgroundLayer<'a> {
    pub(super) image_cache: &'a ImageBrushCache,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) abs_x: f32,
    pub(super) abs_y: f32,
    /// The background color the layer fills with.
    pub(super) bg: CssColor,
    pub(super) image: &'a BackgroundImage,
    pub(super) current_color: CssColor,
    pub(super) clip: VisualBox,
    pub(super) origin: VisualBox,
    pub(super) border_radius: &'a ComputedBorderRadius,
    pub(super) border: &'a Sides<ComputedBorder>,
    pub(super) padding: &'a taffy::Rect<f32>,
    pub(super) env: BackgroundEnv,
    pub(super) size: &'a ComputedBackgroundSize,
    pub(super) position: &'a ComputedCssPosition,
    pub(super) repeat: &'a BackgroundRepeat,
    pub(super) pixel_source: Option<&'a dyn ImagePixelSource>,
    /// The part of the painting area the image is limited to.
    pub(super) image_slice: Option<Rect>,
    /// Without the image-derived fallback, the color is painted as declared.
    pub(super) image_color_fallback: bool,
}

/// What a layer paints: the URL of its image, the color it fills with, and
/// whether its image is a gradient.
struct Visible {
    url: Option<url::Url>,
    color: CssColor,
    has_gradient: bool,
}

/// The border box of the element, and the widths taken off it, as `f64`s.
#[derive(Clone, Copy)]
struct Edges {
    /// The border box `(x0, y0, x1, y1)`, snapped to whole pixels.
    border_box: (f64, f64, f64, f64),
    /// The size of the snapped border box: what `border-radius` percentages
    /// resolve against before any origin or clip inset is applied.
    radius_reference: (f64, f64),
    border: Inset,
    padding: Inset,
}

/// Where a layer paints: the area its position and size resolve against, the
/// area that is painted, and the curve its corners follow.
struct Areas {
    positioning: Rect,
    painting: Rect,
    /// The curve of a rounded painting area, absent for a square one.
    rounded: Option<BezPath>,
    /// How far the corner curve is pulled in from the border box.
    radius_inset: Inset,
    radius_reference: (f64, f64),
}

impl Edges {
    fn new(slice: &ElementBackgroundLayer<'_>) -> Self {
        let ElementBackgroundLayer {
            width,
            height,
            abs_x,
            abs_y,
            border,
            padding,
            ..
        } = *slice;
        let (bx0, by0, bx1, by1) = (
            (abs_x as f64).round(),
            (abs_y as f64).round(),
            ((abs_x + width) as f64).round(),
            ((abs_y + height) as f64).round(),
        );
        Self {
            border_box: (bx0, by0, bx1, by1),
            radius_reference: (bx1 - bx0, by1 - by0),
            border: (
                border.left.width().px() as f64,
                border.top.width().px() as f64,
                border.right.width().px() as f64,
                border.bottom.width().px() as f64,
            ),
            padding: (
                padding.left as f64,
                padding.top as f64,
                padding.right as f64,
                padding.bottom as f64,
            ),
        }
    }

    /// Whether the border area is a ring around a padding box. With zero
    /// borders it is equivalent to the border box.
    fn has_ring(&self) -> bool {
        let (bx0, by0, bx1, by1) = self.border_box;
        let (bl, bt, br, bb) = self.border;
        !(bx1 - br <= bx0 + bl
            || by1 - bb <= by0 + bt
            || (bl == 0.0 && bt == 0.0 && br == 0.0 && bb == 0.0))
    }

    /// The painting area from `background-clip` (CSS Backgrounds 3 section
    /// 2.7), and how far its corner curve is pulled in from the border box.
    /// A `border-area` clip without a ring paints the border box.
    fn painting_area(&self, clip: VisualBox, bleed_inset: Option<f64>) -> (Rect, Inset) {
        let (bx0, by0, bx1, by1) = self.border_box;
        let (bl, bt, br, bb) = self.border;
        let (pl, pt, pr, pb) = self.padding;
        match clip {
            VisualBox::PaddingBox => (
                Rect::new(
                    (bx0 + bl).round(),
                    (by0 + bt).round(),
                    (bx1 - br).round(),
                    (by1 - bb).round(),
                ),
                (bl, bt, br, bb),
            ),
            VisualBox::ContentBox => (
                Rect::new(
                    (bx0 + bl + pl).round(),
                    (by0 + bt + pt).round(),
                    (bx1 - br - pr).round(),
                    (by1 - bb - pb).round(),
                ),
                (bl + pl, bt + pt, br + pr, bb + pb),
            ),
            // `border-box`, and a `border-area` clip that has no ring. The
            // `text` clips are skipped before the painting area is worked out.
            _ => match bleed_inset {
                Some(fraction) => {
                    let inset = (bl * fraction, bt * fraction, br * fraction, bb * fraction);
                    (
                        Rect::new(bx0 + inset.0, by0 + inset.1, bx1 - inset.2, by1 - inset.3),
                        inset,
                    )
                }
                None => (Rect::new(bx0, by0, bx1, by1), (0.0, 0.0, 0.0, 0.0)),
            },
        }
    }
}

impl ElementBackgroundLayer<'_> {
    /// Paint the layer: its color, then its image or gradient over it.
    pub(super) fn paint(&self, scene: &mut impl PaintScene, warnings: &mut Vec<RenderWarning>) {
        let Some(visible) = self.visible() else {
            return;
        };
        if self.skipped_as_text_clip(&visible, warnings) {
            return;
        }
        let edges = Edges::new(self);
        let bleed_inset = self.bleed_inset(&edges);
        let clip = if self.border_box_union(&visible) {
            VisualBox::BorderBox
        } else {
            self.clip
        };
        let positioning = self.positioning(&edges, visible.url.as_ref(), warnings);
        if matches!(clip, VisualBox::BorderArea) && edges.has_ring() {
            self.paint_border_ring(scene, warnings, &edges);
            return;
        }
        let (painting, radius_inset) = edges.painting_area(clip, bleed_inset);
        // Guard against negative or inverted rect after inset (e.g. border larger than box)
        if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
            return;
        }
        let areas = Areas {
            positioning,
            painting,
            rounded: rounded_background_path(
                painting.x0,
                painting.y0,
                painting.x1,
                painting.y1,
                self.border_radius,
                radius_inset,
                edges.radius_reference,
            ),
            radius_inset,
            radius_reference: edges.radius_reference,
        };
        if let BackgroundImage::Gradient(gradient) = self.image {
            self.paint_gradient(scene, gradient, &areas);
        } else {
            self.paint_color_and_image(scene, warnings, &visible, &areas);
        }
    }

    /// What the layer paints, or `None` when it paints nothing.
    fn visible(&self) -> Option<Visible> {
        let url = match self.image {
            BackgroundImage::Url(raw_url) => self.image_cache.resolve_url(raw_url),
            _ => None,
        };
        let color = if self.image_color_fallback {
            background_color_with_image_source(
                self.bg,
                self.image,
                self.current_color,
                self.pixel_source,
            )
        } else {
            (self.bg.a != 0).then_some(self.bg)
        }
        .unwrap_or(CssColor::TRANSPARENT);
        // A gradient paints even when its first stop is transparent.
        let has_gradient = matches!(self.image, BackgroundImage::Gradient(_));
        (color.a != 0 || url.is_some() || has_gradient).then_some(Visible {
            url,
            color,
            has_gradient,
        })
    }

    /// Whether the layer is skipped for `background-clip: text`, which trims
    /// to glyph shapes (CSS Backgrounds 4 section 2.6). Glyph-path clipping is
    /// not implemented; warn and skip so the gap is visible instead of
    /// silently mispainting an opaque rect. `border-area text` clips to the
    /// union of the border area and the glyphs and needs the same glyph
    /// clipping.
    fn skipped_as_text_clip(&self, visible: &Visible, warnings: &mut Vec<RenderWarning>) -> bool {
        if !matches!(self.clip, VisualBox::Text | VisualBox::BorderAreaText) {
            return false;
        }
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: visible.url.as_ref().map(redacted_image_url),
            },
            node_id: None,
            details:
                "background-clip: text for CSS backgrounds is not yet implemented; the background was skipped"
                    .into(),
        });
        true
    }

    /// Whether the background and the borders are painted as one border-box
    /// curve. When an opaque background and all border sides use the same
    /// color, the visible union is the border-box curve. Painting that union
    /// once avoids a one-pixel seam where an inner clipped fill meets its
    /// border ring. A gradient is not one color, so it keeps its own clip.
    fn border_box_union(&self, visible: &Visible) -> bool {
        let border = self.border;
        matches!(self.clip, VisualBox::PaddingBox | VisualBox::ContentBox)
            && visible.color.a != 0
            && !visible.has_gradient
            && [&border.top, &border.right, &border.bottom, &border.left]
                .iter()
                .all(|side| {
                    side.style() == BorderStyle::Solid
                        && side.width().px() > 0.0
                        && matches!(side.color, BorderColor::Resolved(c) if c == visible.color)
                })
    }

    /// The fraction of the border width a rounded border-box background is
    /// pulled in from the border edge, when the border hides that edge. This
    /// keeps the anti-aliased corner pixels from blending the background into
    /// the border's fringe. The inset is half the border width, or a sixth
    /// when a double border has a gap to keep covered (the shrink Chrome
    /// applies to avoid background bleed). Positioning is unaffected: only the
    /// painted area shrinks.
    fn bleed_inset(&self, edges: &Edges) -> Option<f64> {
        let border = self.border;
        let (width, height) = edges.radius_reference;
        (self.env.border_hides_edge
            && matches!(self.clip, VisualBox::BorderBox)
            && used_border_radii(self.border_radius, width, height)
                .iter()
                .flatten()
                .any(|value| *value > 0.0))
        .then(|| {
            let double = [&border.top, &border.right, &border.bottom, &border.left]
                .iter()
                .any(|side| side.style() == BorderStyle::Double);
            if double { 1.0 / 6.0 } else { 0.5 }
        })
    }

    /// The area the image is positioned in. It follows `background-origin`
    /// (CSS Backgrounds 3 section 2.8) and the layout box rather than the
    /// snapped painting box, so a centered or sized image lands where the
    /// unsnapped geometry puts it; the tile is snapped to device pixels when
    /// it is drawn. `border-area` and `text` are not valid origins; warn and
    /// fall back to the spec initial `padding-box` instead of silently
    /// mispositioning. A `fixed` layer is positioned in the page area whatever
    /// its origin (CSS Backgrounds 3 section 2.5) and still painted in its
    /// clip box.
    fn positioning(
        &self,
        edges: &Edges,
        url: Option<&url::Url>,
        warnings: &mut Vec<RenderWarning>,
    ) -> Rect {
        let (bl, bt, br, bb) = edges.border;
        let (pl, pt, pr, pb) = edges.padding;
        let (ux0, uy0, ux1, uy1) = (
            self.abs_x as f64,
            self.abs_y as f64,
            (self.abs_x + self.width) as f64,
            (self.abs_y + self.height) as f64,
        );
        let padding_box = (ux0 + bl, uy0 + bt, ux1 - br, uy1 - bb);
        let (px0, py0, px1, py1) = match self.origin {
            VisualBox::BorderBox => (ux0, uy0, ux1, uy1),
            VisualBox::PaddingBox => padding_box,
            VisualBox::ContentBox => (ux0 + bl + pl, uy0 + bt + pt, ux1 - br - pr, uy1 - bb - pb),
            _ => {
                warnings.push(RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Image,
                        url: url.map(redacted_image_url),
                    },
                    node_id: None,
                    details:
                        "background-origin has an unsupported visual box; using padding-box positioning"
                            .into(),
                });
                padding_box
            }
        };
        self.env
            .fixed_viewport
            .unwrap_or_else(|| Rect::new(px0, py0, px1, py1))
    }

    /// Paint a `border-area` background: the area where the borders draw,
    /// with their styles and radii (CSS Backgrounds 4 section 2.6). Draw the
    /// borders opaque as a mask, then composite the background, painted over
    /// the whole border box, into it.
    fn paint_border_ring(
        &self,
        scene: &mut impl PaintScene,
        warnings: &mut Vec<RenderWarning>,
        edges: &Edges,
    ) {
        let (bx0, by0, bx1, by1) = edges.border_box;
        let border_box = Rect::new(bx0, by0, bx1, by1);
        scene.push_layer(Mix::Normal, 1.0, Affine::IDENTITY, &border_box, None, None);
        let mut mask = *self.border;
        for side in [
            &mut mask.top,
            &mut mask.right,
            &mut mask.bottom,
            &mut mask.left,
        ] {
            side.color = BorderColor::Resolved(CssColor::BLACK);
        }
        paint_element_border_rounded(
            scene,
            self.width,
            self.height,
            self.abs_x,
            self.abs_y,
            &mask,
            self.current_color,
            self.border_radius,
        );
        scene.push_layer(
            BlendMode::new(Mix::Normal, Compose::SrcIn),
            1.0,
            Affine::IDENTITY,
            &border_box,
            None,
            None,
        );
        // The mask already carries the corners; clipping the background to
        // them again would square their coverage.
        let square = ComputedBorderRadius::all(ComputedLength(0.0));
        ElementBackgroundLayer {
            clip: VisualBox::BorderBox,
            border_radius: &square,
            // The mask already limits the background to the border ring, so
            // there is no edge to keep clear of the image.
            env: BackgroundEnv {
                border_hides_edge: false,
                ..self.env
            },
            ..*self
        }
        .paint(scene, warnings);
        scene.pop_layer();
        scene.pop_layer();
    }

    /// Paint a gradient layer over its color. A rounded box clips its color
    /// and its gradient to the curve together (CSS Backgrounds 3 section 5.3:
    /// backgrounds clip to the curve). Clipped one by one, the antialiased
    /// edge would cover the color twice, once for the color and once more for
    /// the gradient over it, and show more of it past the curve.
    fn paint_gradient(&self, scene: &mut impl PaintScene, gradient: &Gradient, areas: &Areas) {
        if let Some(curve) = areas.rounded.as_ref() {
            scene.push_clip_layer(Affine::IDENTITY, curve);
        }
        if self.bg.a != 0 {
            let base = Color::from_rgba8(self.bg.r, self.bg.g, self.bg.b, self.bg.a);
            scene.fill(Fill::NonZero, Affine::IDENTITY, base, None, &areas.painting);
        }
        if let Gradient::Conic(conic) = gradient {
            paint_conic_gradient(
                scene,
                conic,
                areas.positioning,
                areas.painting,
                self.current_color,
            );
        } else {
            gradient::paint_gradient_layer(
                scene,
                gradient,
                areas.positioning,
                self.image_slice
                    .map_or(areas.painting, |slice| areas.painting.intersect(slice)),
                self.size,
                self.position,
                self.repeat,
                self.current_color,
            );
        }
        if areas.rounded.is_some() {
            scene.pop_layer();
        }
    }

    /// Paint the color of a layer and its URL image over it. A rounded box
    /// clips its color and its image to the curve together, like a gradient
    /// layer does (see [`Self::paint_gradient`]). Square painting areas and
    /// color-only boxes skip the layer.
    fn paint_color_and_image(
        &self,
        scene: &mut impl PaintScene,
        warnings: &mut Vec<RenderWarning>,
        visible: &Visible,
        areas: &Areas,
    ) {
        let painting = areas.painting;
        let image = visible.url.as_ref().zip(self.pixel_source);
        let grouped = areas.rounded.as_ref().filter(|_| image.is_some());
        if let Some(curve) = grouped {
            scene.push_clip_layer(Affine::IDENTITY, curve);
        }
        if visible.color.a != 0 {
            let color = Color::from_rgba8(
                visible.color.r,
                visible.color.g,
                visible.color.b,
                visible.color.a,
            );
            if grouped.is_some() {
                // The layer clips this fill to the curve.
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &painting);
            } else {
                fill_rounded_background(
                    scene,
                    color,
                    painting.x0,
                    painting.y0,
                    painting.x1,
                    painting.y1,
                    self.border_radius,
                    areas.radius_inset,
                    areas.radius_reference,
                );
            }
        }
        if let Some((url, source)) = image {
            paint_background_from_source(
                self.image_cache,
                scene,
                url,
                areas.positioning,
                self.image_slice
                    .map_or(painting, |slice| painting.intersect(slice)),
                self.size,
                self.position,
                self.repeat,
                self.env.device_grid,
                source,
                warnings,
            );
        }
        if grouped.is_some() {
            scene.pop_layer();
        }
    }
}
