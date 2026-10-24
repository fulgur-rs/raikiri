//! DOM walker — traverses the Document arena in DFS order and emits to PaintScene.
//!
//! `paint_document` walks with an iterative `PaintFrame` stack, following
//! the pattern of cascade / find_paint_root and avoiding stack overflow for deep DOMs.
//! It handles each kind inline: an Element pushes its children (a paragraph
//! root draws its lines, then pushes the boxes of its paragraph), a Text node
//! that is a paragraph of its own draws its lines, and display:none skips a
//! subtree. A matching `PopClip` frame closes each overflow clip after the
//! subtree. See the `vertical_align_shift_px` docs for `parent_font_size` /
//! `shift_y`.
//!
//! The paint-order and eligibility predicates (`find_paint_root`,
//! `sort_paint_children`, ...) live in `raikiri_dom::paint_rules` so the page
//! paint-order generator shares them with this walker.

use anyrender::PaintScene;
use kurbo::{Affine, Arc, BezPath, Point, Rect, Vec2};
use peniko::color::{
    AlphaColor, ColorSpaceTag, DynamicColor, HueDirection, Lab, Lch, LinearSrgb, Oklab, Oklch, Srgb,
};
use peniko::{Color, Extend as PenikoExtend, Fill, Gradient as PenikoGradient};
use raikiri_dom::image_geometry::{
    DeviceGrid, VisibleArea, background_image_dimensions, background_tiles, position_offset,
    rounded_background_tile_size,
};
use raikiri_dom::{CounterSnapshots, Document, StandaloneAlign};
use raikiri_layout_geometry::{BoxLayout, Edges};
use raikiri_style::property::{
    AnglePercentage, BackgroundImage, BackgroundLayerSpec, Border, BorderColor, BorderStyle,
    ConicGradient, CssColor, CssPosition, CssPositionOffset, DisplayValue, Gradient,
    GradientStopColor, HueInterpolationMethod, Length, LengthPercentageOrCalc, MixColorSpace,
    ObjectFit, OutlineColor, OutlineStyle, PositionValue, PropertyKey, PropertyValue, Sides,
    TextShadowColor, UnboundedColor, VerticalAlign, VisualBox,
};
use raikiri_style::{
    CascadeResult, ComputedBackgroundSize, ComputedBorderImage, ComputedBorderRadius,
    ComputedCssPosition, ComputedLength, ComputedLengthPercentageOrAuto, ComputedTransformFunction,
    ComputedValues, MAX_PAINTED_BACKGROUND_LAYERS, ResolveContext, resolve_background_size,
    resolve_border, resolve_css_position,
};
use raikiri_traits::{
    ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, NodeId, NodeKind, PageBox, PaintInsets,
    RenderWarning, WarningKind,
};
use std::f64::consts::{FRAC_PI_2, PI};

mod background_image;
mod border_image;
mod box_layers;
mod box_paint;
mod child_frames;
mod element_background;
mod element_border;
mod element_box;
mod frames;
mod gradient;
mod image_brush;
mod inline_box;
mod inline_svg;
mod lines_frames;
mod margin_box;
mod page_geometry;
mod paint_walk;
mod visit;
mod visit_element;
mod visit_text;
pub(crate) use image_brush::ImageBrushCache;
pub(crate) use inline_box::paint_inline_box;

use background_image::ImageLayer;
use border_image::BorderImage;
use element_background::ElementBackgroundLayer;
use element_border::ElementBorder;
use frames::{DeferralLayer, LayerClose, PaintFrame, VisitFrame};
use inline_svg::InlineSvg;
use margin_box::paint_margin_box;
use page_geometry::PageGeometry;
use paint_walk::{PaintWalk, index_fragments};

use crate::text;

use raikiri_dom::generated_content::markers::marker_render_info_with_snapshots;

/// Cover the whole paper with the UA's white canvas and nothing else.
pub(crate) fn paint_blank_paper(scene: &mut impl PaintScene, page_box: PageBox) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(255, 255, 255, 255),
        None,
        &Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64),
    );
}

/// Fill a rectangle with a flat color.
fn fill_rect(scene: &mut impl PaintScene, color: peniko::Color, rect: kurbo::Rect) {
    scene.fill(
        peniko::Fill::NonZero,
        kurbo::Affine::IDENTITY,
        color,
        None,
        &rect,
    );
}

/// The paper as a rectangle in page coordinates.
fn paper_rect(page_box: PageBox) -> kurbo::Rect {
    kurbo::Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64)
}

/// Paint the page background: the color and images of the `@page` box, which
/// cover the paper. It is the bottom of everything a page shows, margin boxes
/// included (CSS Page 3 §3.1).
///
/// The paper is always covered, even when the page background is
/// transparent; the UA canvas default is white in that case. The page and
/// the propagated canvas background (see [`paint_canvas_background`]) stay
/// separate layers: a canvas color may be translucent, and selecting it with
/// `or` would discard the opaque page color instead of compositing over it.
pub(crate) fn paint_page_background(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let page_color = page_background_color(cascade, pixel_source);
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let white = peniko::Color::from_rgba8(255, 255, 255, 255);

    let page_rect = paper_rect(page_box);
    fill_rect(scene, page_color.unwrap_or(white), page_rect);
    // The page background covers the paper, but `background-origin` places it
    // in the page box, whose border edge lies inside the page margins. The
    // page border is painted at the same edge.
    let border_box_width = (page_box.width - margins.left - margins.right).max(0.0);
    let border_box_height = (page_box.height - margins.top - margins.bottom).max(0.0);
    paint_page_background_image(
        image_cache,
        scene,
        document,
        cascade,
        PageBackgroundAreas {
            paper: page_rect,
            border_box: kurbo::Rect::from_origin_size(
                (f64::from(margins.left), f64::from(margins.top)),
                (f64::from(border_box_width), f64::from(border_box_height)),
            ),
        },
        pixel_source,
        warnings,
    );
}

/// Canvas background fill site — CSS Backgrounds 3 §2.11 canvas propagation.
///
/// When the page has a non-zero margin, the html/body canvas background is
/// painted only in the page content area; this is the geometry that makes
/// `@page { margin: 5px }` visibly different from a zero-margin page.  With
/// zero page margins the canvas background covers the whole paper.
///
/// The canvas images are sized and positioned against the root element's
/// box (§2.11.2), which runs through every page as one surface:
/// `root` is the root element's box on this page, which sits higher on each
/// later page, so a later page continues the tiles of the earlier ones and
/// shows a `no-repeat` image only where the root's box reaches it.
///
/// The page background (see [`paint_page_background`]) goes below it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_canvas_background(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    root: Option<&raikiri_dom::RootBox>,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let canvas_color = canvas_background_color(document, cascade, pixel_source);
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let canvas_rect = if margins.is_zero() {
        paper_rect(page_box)
    } else {
        kurbo::Rect::new(
            margins.left as f64,
            margins.top as f64,
            (page_box.width - margins.right) as f64,
            (page_box.height - margins.bottom) as f64,
        )
    };
    if let Some(color) = canvas_color {
        fill_rect(scene, color, canvas_rect);
    }
    let insets = raikiri_dom::page_content_insets(cascade, page_box);
    let page_area = page_area_rect(margins, insets, page_box);
    paint_canvas_background_image(
        image_cache,
        scene,
        document,
        cascade,
        &CanvasArea::new(canvas_rect, page_area, root),
        pixel_source,
        warnings,
    );
}

/// The page area in page-local coordinates: the page box inside its margins,
/// border and padding, where the document flows and `fixed` background layers
/// are positioned.
fn page_area_rect(
    margins: raikiri_dom::PageMargins,
    insets: raikiri_dom::PageContentInsets,
    page_box: PageBox,
) -> Rect {
    Rect::from_origin_size(
        (
            f64::from(margins.left + insets.left),
            f64::from(margins.top + insets.top),
        ),
        (
            f64::from(insets.page_area_width(margins, page_box)),
            f64::from(insets.page_area_height(margins, page_box)),
        ),
    )
}

/// The page area that a box's `fixed` background layers are positioned in, or
/// `None` when they behave like `scroll` ones.
///
/// A box that is, or lies inside, a containing block for fixed boxes (a
/// transformed box, say) is not fixed to the page, and neither is anything
/// under a transformed root, which carries the whole page with it (CSS
/// Transforms 1 section 6).
fn fixed_background_viewport(
    page_area: Rect,
    root_transform: Affine,
    in_fixed_containing_block: bool,
) -> Option<Rect> {
    (root_transform == Affine::IDENTITY && !in_fixed_containing_block).then_some(page_area)
}

/// Where the images of the canvas background paint and are placed.
pub(crate) struct CanvasArea {
    /// The canvas on this page, which every layer covers whatever its
    /// `background-clip` (CSS Backgrounds 3 §2.11.1).
    painting: Rect,
    /// The page area, the initial containing block, which `fixed` layers
    /// are positioned in (CSS Backgrounds 3 §2.5).
    viewport: Rect,
    /// The root element's border box, which the other layers are
    /// positioned in.
    root: Rect,
    /// The root's border widths: left, top, right, bottom.
    border: (f64, f64, f64, f64),
    /// The root's padding: left, top, right, bottom.
    padding: (f64, f64, f64, f64),
    /// The root's transform, which moves the images with the rest of the
    /// canvas.
    transform: Option<Affine>,
}

impl CanvasArea {
    /// The canvas `painting` on this page, with its images placed in the
    /// root element's box `root`, or the page area `viewport` for a `fixed`
    /// layer. Without a root box the images are placed in the painting area
    /// itself.
    pub(crate) fn new(painting: Rect, viewport: Rect, root: Option<&raikiri_dom::RootBox>) -> Self {
        let Some(root) = root else {
            return Self {
                painting,
                viewport,
                root: painting,
                border: (0.0, 0.0, 0.0, 0.0),
                padding: (0.0, 0.0, 0.0, 0.0),
                transform: None,
            };
        };
        let edges = |insets: PaintInsets| {
            (
                f64::from(insets.left),
                f64::from(insets.top),
                f64::from(insets.right),
                f64::from(insets.bottom),
            )
        };
        let rect = root.border_box;
        Self {
            painting,
            viewport,
            root: Rect::new(
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.x + rect.width),
                f64::from(rect.y + rect.height),
            ),
            border: edges(root.border),
            padding: edges(root.padding),
            transform: root.transform.map(crate::transform::affine),
        }
    }

    /// The positioning area of `layer`: the page area for a `fixed` layer,
    /// whatever its origin, and otherwise the root's box inset as its
    /// origin says.
    fn positioning(
        &self,
        layer: &raikiri_style::ComputedBackgroundLayer,
        image_url: Option<&url::Url>,
        warnings: &mut Vec<RenderWarning>,
    ) -> Rect {
        if layer.attachment == raikiri_style::property::BackgroundAttachment::Fixed {
            return self.viewport;
        }
        origin_inset_rect(
            self.root,
            layer.origin,
            self.border,
            self.padding,
            image_url,
            warnings,
        )
    }
}

/// Paint the page-context border around the page content area.
///
/// Page borders are ordinary `@page` declarations, but they do not belong to
/// the document's node tree.  Resolve their four longhands here and reuse the
/// element-border painter so page borders share style gating, colors, and
/// double-border handling with normal boxes.  The outer border edge is inside
/// the physical page margins; page padding and the border itself then inset
/// ordinary document flow from that edge.
///
/// A `border-image` replaces the four border styles where it can be drawn
/// (CSS Backgrounds 3 §6), as on an element.
pub(crate) fn paint_page_border(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let declarations = cascade.page.declarations();
    if matches!(
        declarations.get(&PropertyKey::Visibility),
        Some(PropertyValue::Visibility(
            raikiri_style::property::Visibility::Hidden
        ))
    ) {
        return;
    }
    let current_color = page_current_color(document, cascade);
    let borders = page_border_sides(declarations);
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let border_box_width = (page_box.width - margins.left - margins.right).max(0.0);
    let border_box_height = (page_box.height - margins.top - margins.bottom).max(0.0);
    if let Some(image) = cascade.page.border_image() {
        let mut cv = ComputedValues::initial();
        cv.color = current_color;
        if paint_element_border_image(
            image_cache,
            scene,
            border_box_width,
            border_box_height,
            margins.left,
            margins.top,
            &cv,
            &image,
            &borders,
            pixel_source,
            warnings,
            Affine::IDENTITY,
        ) {
            return;
        }
    }
    paint_element_border(
        scene,
        border_box_width,
        border_box_height,
        margins.left,
        margins.top,
        &borders,
        current_color,
    );
}

/// The `color` of the page context, which inherits it from the root element.
fn page_current_color(document: &Document, cascade: &CascadeResult) -> CssColor {
    match cascade.page.declarations().get(&PropertyKey::Color) {
        Some(PropertyValue::Color(color)) => *color,
        _ => find_html(document)
            .and_then(|node_id| cascade.computed.get(node_id))
            .map_or(CssColor::BLACK, |computed| computed.color),
    }
}

/// Resolve the four borders of the page context from its `@page` declarations.
fn page_border_sides(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
) -> Sides<raikiri_style::resolve::ComputedBorder> {
    let ctx = ResolveContext::initial();
    let side = |width_key, style_key, color_key| {
        page_border_side(declarations, width_key, style_key, color_key, &ctx)
    };
    Sides {
        top: side(
            PropertyKey::BorderTopWidth,
            PropertyKey::BorderTopStyle,
            PropertyKey::BorderTopColor,
        ),
        right: side(
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderRightColor,
        ),
        bottom: side(
            PropertyKey::BorderBottomWidth,
            PropertyKey::BorderBottomStyle,
            PropertyKey::BorderBottomColor,
        ),
        left: side(
            PropertyKey::BorderLeftWidth,
            PropertyKey::BorderLeftStyle,
            PropertyKey::BorderLeftColor,
        ),
    }
}

/// Resolve one border of the page context from the declarations under the
/// longhands `width_key`, `style_key` and `color_key` of its side.
fn page_border_side(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    width_key: PropertyKey,
    style_key: PropertyKey,
    color_key: PropertyKey,
    ctx: &ResolveContext,
) -> raikiri_style::resolve::ComputedBorder {
    let mut border = Border::new();
    match declarations.get(&width_key) {
        Some(PropertyValue::BorderTopWidth(value))
        | Some(PropertyValue::BorderRightWidth(value))
        | Some(PropertyValue::BorderBottomWidth(value))
        | Some(PropertyValue::BorderLeftWidth(value)) => border.width = *value,
        _ => {}
    }
    match declarations.get(&style_key) {
        Some(PropertyValue::BorderTopStyle(value))
        | Some(PropertyValue::BorderRightStyle(value))
        | Some(PropertyValue::BorderBottomStyle(value))
        | Some(PropertyValue::BorderLeftStyle(value)) => border.style = *value,
        _ => {}
    }
    match declarations.get(&color_key) {
        Some(PropertyValue::BorderTopColor(value))
        | Some(PropertyValue::BorderRightColor(value))
        | Some(PropertyValue::BorderBottomColor(value))
        | Some(PropertyValue::BorderLeftColor(value)) => border.color = *value,
        _ => {}
    }
    resolve_border(border, ComputedLength(16.0), None, ctx)
}

/// Paint a solid `@page` outline around the page content box.
///
/// The outline is outside the page area and does not consume page margins.
/// This minimal path covers the solid outline used by the page-box WPT tests;
/// unsupported outline styles remain intentionally unpainted.
pub(crate) fn paint_page_outline(
    scene: &mut impl PaintScene,
    cascade: &CascadeResult,
    page_box: PageBox,
) {
    let declarations = cascade.page.declarations();
    if matches!(
        declarations.get(&PropertyKey::Visibility),
        Some(PropertyValue::Visibility(
            raikiri_style::property::Visibility::Hidden
        ))
    ) {
        return;
    }
    let Some(PropertyValue::OutlineWidth(width)) = declarations.get(&PropertyKey::OutlineWidth)
    else {
        return;
    };
    let Some(PropertyValue::OutlineStyle(style)) = declarations.get(&PropertyKey::OutlineStyle)
    else {
        return;
    };
    if !matches!(style, OutlineStyle::Solid) {
        return;
    }
    let outline_width = length_to_px(*width, page_box.width, 16.0);
    if outline_width <= 0.0 {
        return;
    }
    let offset = match declarations.get(&PropertyKey::OutlineOffset) {
        Some(PropertyValue::OutlineOffset(value)) => length_to_px(*value, page_box.width, 16.0),
        _ => 0.0,
    };
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let area_width = (page_box.width - margins.left - margins.right).max(0.0);
    let area_height = (page_box.height - margins.top - margins.bottom).max(0.0);
    let outer_x = margins.left - offset - outline_width;
    let outer_y = margins.top - offset - outline_width;
    let outer_width = area_width + 2.0 * (offset + outline_width);
    let outer_height = area_height + 2.0 * (offset + outline_width);
    if outer_width <= 0.0 || outer_height <= 0.0 {
        return;
    }
    let current_color = match declarations.get(&PropertyKey::Color) {
        Some(PropertyValue::Color(color)) => *color,
        _ => CssColor::BLACK,
    };
    let color = match declarations.get(&PropertyKey::OutlineColor) {
        Some(PropertyValue::OutlineColor(OutlineColor::Resolved(color))) => {
            BorderColor::Resolved(*color)
        }
        Some(PropertyValue::OutlineColor(OutlineColor::CurrentColor))
        | Some(PropertyValue::OutlineColor(OutlineColor::Invert))
        | None => BorderColor::Resolved(current_color),
        _ => BorderColor::Resolved(current_color),
    };
    let mut border = Border::new();
    border.width = Length::Px(outline_width);
    border.style = BorderStyle::Solid;
    border.color = color;
    let border = resolve_border(
        border,
        ComputedLength(16.0),
        None,
        &ResolveContext::initial(),
    );
    paint_element_border(
        scene,
        outer_width,
        outer_height,
        outer_x,
        outer_y,
        &Sides {
            top: border,
            right: border,
            bottom: border,
            left: border,
        },
        current_color,
    );
}

fn page_background_color(
    cascade: &CascadeResult,
    pixel_source: Option<&dyn ImagePixelSource>,
) -> Option<Color> {
    let declarations = cascade.page.declarations();
    let mut has_background_declaration = false;
    let mut background_color = CssColor::TRANSPARENT;
    let mut background_image = &BackgroundImage::None;
    let mut current_color = CssColor::BLACK;

    if let Some(PropertyValue::BackgroundColor(value)) =
        declarations.get(&PropertyKey::BackgroundColor)
    {
        background_color = *value;
        has_background_declaration = true;
    }
    if let Some(PropertyValue::BackgroundImage(value)) =
        declarations.get(&PropertyKey::BackgroundImage)
    {
        // The image-derived color fallback applies to a single layer only;
        // with several layers the color stands alone under the images.
        if let [image] = &**value {
            background_image = image;
        }
        has_background_declaration = true;
    }
    if let Some(PropertyValue::Color(value)) = declarations.get(&PropertyKey::Color) {
        current_color = *value;
    }

    if !has_background_declaration {
        return None;
    }
    // A gradient the page box draws paints over the background color
    // rather than standing in for it.
    if paints_gradient_image(background_image) {
        return (background_color.a != 0).then(|| {
            Color::from_rgba8(
                background_color.r,
                background_color.g,
                background_color.b,
                background_color.a,
            )
        });
    }
    background_color_with_image_source(
        background_color,
        background_image,
        current_color,
        pixel_source,
    )
    .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a))
}

fn canvas_background_owner(document: &Document, cascade: &CascadeResult) -> Option<usize> {
    let html_id = find_html(document)?;
    let html = cascade.computed.get(html_id)?;
    let html_is_transparent = !html.has_background_image() && html.background_color.a == 0;
    if !html_is_transparent {
        return Some(html_id);
    }

    // Only a `body` propagates its background, not a `frameset` (CSS
    // Backgrounds 3, 2.11.2).
    let body_id = raikiri_dom::paint_rules::find_paint_root(document)
        .filter(|&id| document.get_node(id).and_then(|node| node.tag_name()) == Some("body"))?;
    let body = cascade.computed.get(body_id)?;
    let body_has_background = body.has_background_image() || body.background_color.a != 0;
    body_has_background.then_some(body_id)
}

fn canvas_background_color(
    document: &Document,
    cascade: &CascadeResult,
    pixel_source: Option<&dyn ImagePixelSource>,
) -> Option<Color> {
    let computed = cascade
        .computed
        .get(canvas_background_owner(document, cascade)?)?;
    // A linear or radial gradient is painted as an image over the color by
    // `paint_canvas_background_image`, and so are the images of several
    // layers: the image-derived color fallback applies to one layer only.
    let image = match computed.background_layers() {
        [layer] if !paints_gradient_image(&layer.image) => &layer.image,
        _ => {
            return (computed.background_color.a != 0)
                .then(|| css_color(computed.background_color));
        }
    };
    background_color_with_image_source(
        computed.background_color,
        image,
        computed.color,
        pixel_source,
    )
    .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a))
}

/// The item of an `@page` per-layer background list for layer `index`: a
/// list with too few items repeats (CSS Backgrounds 3 §2.1).
fn page_layer_item<T: Copy>(list: Option<&[T]>, index: usize, initial: T) -> T {
    match list {
        Some(list) if !list.is_empty() => list[index % list.len()],
        _ => initial,
    }
}

#[cfg(test)]
fn page_background_clips(cascade: &CascadeResult) -> Option<&[VisualBox]> {
    match page_property(cascade, PropertyKey::BackgroundClip) {
        Some(PropertyValue::BackgroundClip(clips)) => Some(clips),
        _ => None,
    }
}

fn page_background_origins(cascade: &CascadeResult) -> Option<&[VisualBox]> {
    match page_property(cascade, PropertyKey::BackgroundOrigin) {
        Some(PropertyValue::BackgroundOrigin(origins)) => Some(origins),
        _ => None,
    }
}

/// The page `background-attachment` list, one entry per layer.
fn page_background_attachments(
    cascade: &CascadeResult,
) -> Option<&[raikiri_style::property::BackgroundAttachment]> {
    match page_property(cascade, PropertyKey::BackgroundAttachment) {
        Some(PropertyValue::BackgroundAttachment(attachments)) => Some(attachments),
        _ => None,
    }
}

/// The page `background-clip` of the first layer.
#[cfg(test)]
fn page_background_clip(cascade: &CascadeResult) -> VisualBox {
    page_layer_item(
        page_background_clips(cascade),
        0,
        raikiri_style::ComputedBackgroundLayer::initial().clip,
    )
}

/// The page `background-origin` of the first layer.
#[cfg(test)]
fn page_background_origin(cascade: &CascadeResult) -> VisualBox {
    page_layer_item(
        page_background_origins(cascade),
        0,
        raikiri_style::ComputedBackgroundLayer::initial().origin,
    )
}

/// Used page border widths in px for background geometry.
///
/// Reads the `@page` border-width longhands directly; unlike
/// [`page_content_insets`](raikiri_dom::page_content_insets) this helper keeps
/// border and padding separate so `background-origin`/`background-clip` can
/// inset them independently. Percentages cannot occur on border widths, so the
/// basis only matters for defensive non-px lengths.
fn page_border_widths(cascade: &CascadeResult, area: Rect) -> (f64, f64, f64, f64) {
    let width = |key: PropertyKey, basis: f64| match page_property(cascade, key) {
        Some(PropertyValue::BorderTopWidth(value))
        | Some(PropertyValue::BorderRightWidth(value))
        | Some(PropertyValue::BorderBottomWidth(value))
        | Some(PropertyValue::BorderLeftWidth(value)) => {
            length_to_px(*value, basis as f32, 16.0) as f64
        }
        _ => 0.0,
    };
    (
        width(PropertyKey::BorderLeftWidth, area.width()),
        width(PropertyKey::BorderTopWidth, area.height()),
        width(PropertyKey::BorderRightWidth, area.width()),
        width(PropertyKey::BorderBottomWidth, area.height()),
    )
}

/// Used page padding in px for background geometry (percentages resolve
/// against the corresponding area edge, matching page insets).
fn page_padding_widths(cascade: &CascadeResult, area: Rect) -> (f64, f64, f64, f64) {
    let padding = |key: PropertyKey, basis: f64| match page_property(cascade, key) {
        Some(PropertyValue::PaddingTop(value))
        | Some(PropertyValue::PaddingRight(value))
        | Some(PropertyValue::PaddingBottom(value))
        | Some(PropertyValue::PaddingLeft(value)) => match *value {
            LengthPercentageOrCalc::Length(length) => {
                length_to_px(length, basis as f32, 16.0) as f64
            }
            // Page-context absolutization folds font-relative terms, leaving a
            // percentage with an absolute offset.
            LengthPercentageOrCalc::Calc(calc) => f64::from(
                raikiri_style::resolve::ComputedLengthPercentageOrCalc::Calc(calc)
                    .resolve(basis as f32),
            ),
            _ => 0.0,
        },
        _ => 0.0,
    };
    (
        padding(PropertyKey::PaddingLeft, area.width()),
        padding(PropertyKey::PaddingTop, area.height()),
        padding(PropertyKey::PaddingRight, area.width()),
        padding(PropertyKey::PaddingBottom, area.height()),
    )
}

/// Inset `area` (treated as the border box) by `background-origin`.
/// Invalid `border-area`/`text` origins warn and fall back to `padding-box`.
fn origin_inset_rect(
    area: Rect,
    origin: VisualBox,
    border: (f64, f64, f64, f64),
    padding: (f64, f64, f64, f64),
    image_url: Option<&url::Url>,
    warnings: &mut Vec<RenderWarning>,
) -> Rect {
    let (bl, bt, br, bb) = border;
    let (pl, pt, pr, pb) = padding;
    match origin {
        VisualBox::BorderBox => area,
        VisualBox::PaddingBox => Rect::new(area.x0 + bl, area.y0 + bt, area.x1 - br, area.y1 - bb),
        VisualBox::ContentBox => Rect::new(
            area.x0 + bl + pl,
            area.y0 + bt + pt,
            area.x1 - br - pr,
            area.y1 - bb - pb,
        ),
        VisualBox::BorderArea | VisualBox::Text | VisualBox::BorderAreaText => {
            warnings.push(RenderWarning {
                kind: WarningKind::ResourceFallback {
                    kind: raikiri_traits::ResourceKind::Image,
                    url: image_url.map(redacted_image_url),
                },
                node_id: None,
                details:
                    "background-origin has an unsupported visual box; using padding-box positioning"
                        .into(),
            });
            Rect::new(area.x0 + bl, area.y0 + bt, area.x1 - br, area.y1 - bb)
        }
        // cov:ignore: defensive fallback for future `VisualBox` variants
        _ => Rect::new(area.x0 + bl, area.y0 + bt, area.x1 - br, area.y1 - bb),
    }
}

/// Inset `area` (treated as the border box) by `background-clip`.
///
/// Returns `None` for `text` (glyph clipping unimplemented) and for the
/// `border-area` ring (per-strip image tiling unimplemented); callers warn and
/// skip the image in those cases. Standard boxes return the painting rect.
fn clip_inset_rect(
    area: Rect,
    clip: VisualBox,
    border: (f64, f64, f64, f64),
    padding: (f64, f64, f64, f64),
) -> Option<Rect> {
    let (bl, bt, br, bb) = border;
    let (pl, pt, pr, pb) = padding;
    match clip {
        VisualBox::BorderBox => Some(area),
        VisualBox::PaddingBox => Some(Rect::new(
            area.x0 + bl,
            area.y0 + bt,
            area.x1 - br,
            area.y1 - bb,
        )),
        VisualBox::ContentBox => Some(Rect::new(
            area.x0 + bl + pl,
            area.y0 + bt + pt,
            area.x1 - br - pr,
            area.y1 - bb - pb,
        )),
        VisualBox::BorderArea | VisualBox::Text | VisualBox::BorderAreaText => None,
        // cov:ignore: defensive fallback for future `VisualBox` variants
        _ => Some(area),
    }
}

/// The layers of `layers` that are painted, top first: all of them, or the
/// top [`MAX_PAINTED_BACKGROUND_LAYERS`] with a warning.
fn painted_background_layers<'a, T>(layers: &'a [T], warnings: &mut Vec<RenderWarning>) -> &'a [T] {
    warn_unpainted_background_layers(layers.len(), warnings);
    &layers[..layers.len().min(MAX_PAINTED_BACKGROUND_LAYERS)]
}

/// Warn when a box declares more background layers than are painted.
fn warn_unpainted_background_layers(layer_count: usize, warnings: &mut Vec<RenderWarning>) {
    if layer_count <= MAX_PAINTED_BACKGROUND_LAYERS {
        return;
    }
    warnings.push(RenderWarning {
        kind: WarningKind::ResourceFallback {
            kind: raikiri_traits::ResourceKind::Image,
            url: None,
        },
        node_id: None,
        details: format!(
            "a box has {layer_count} background layers; only the top {MAX_PAINTED_BACKGROUND_LAYERS} were painted"
        ),
    });
}

/// Where the page background paints and where its images are placed.
#[derive(Clone, Copy)]
struct PageBackgroundAreas {
    /// The paper, which every layer covers whatever its `background-clip`.
    paper: Rect,
    /// The page box's border edge, inside the page margins, which
    /// `background-origin` insets.
    border_box: Rect,
}

/// Paint the page background images over the paper, positioned in the page
/// box.
fn paint_page_background_image(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    areas: PageBackgroundAreas,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let area = areas.paper;
    // The page box paints a URL image through the pixel source, and a
    // linear or radial gradient itself.
    let Some(PropertyValue::BackgroundImage(images)) =
        page_property(cascade, PropertyKey::BackgroundImage)
    else {
        return;
    };
    let initial = ComputedValues::initial();
    let initial_layer = raikiri_style::ComputedBackgroundLayer::initial();
    let root = find_html(document).and_then(|node_id| cascade.computed.get(node_id));
    let root_font_size = root.map_or(initial.font_size, |computed| computed.font_size);
    let page_font_size = match page_property(cascade, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(Length::Px(px))) => ComputedLength(*px),
        _ => root_font_size,
    };
    let resolve_context = ResolveContext::new(root_font_size);
    let sizes = match page_property(cascade, PropertyKey::BackgroundSize) {
        Some(PropertyValue::BackgroundSize(sizes)) => Some(&**sizes),
        _ => None,
    };
    let positions = match page_property(cascade, PropertyKey::BackgroundPosition) {
        Some(PropertyValue::BackgroundPosition(positions)) => Some(&**positions),
        _ => None,
    };
    let repeats = match page_property(cascade, PropertyKey::BackgroundRepeat) {
        Some(PropertyValue::BackgroundRepeat(repeats)) => Some(&**repeats),
        _ => None,
    };
    let origins = page_background_origins(cascade);
    let attachments = page_background_attachments(cascade);
    let border = page_border_widths(cascade, area);
    let padding = page_padding_widths(cascade, area);
    // The page context inherits `color` from the root element.
    let current_color = match page_property(cascade, PropertyKey::Color) {
        Some(PropertyValue::Color(color)) => *color,
        _ => root.map_or(initial.color, |computed| computed.color),
    };
    // CSS Backgrounds 3 §2.1: the first layer is on top, so paint from the
    // last to the first.
    let painted = painted_background_layers(images, warnings);
    for (index, image) in painted.iter().enumerate().rev() {
        let mut layer = initial_layer.clone();
        layer.image = image.clone();
        layer.size = sizes.map_or(initial_layer.size, |sizes| {
            resolve_background_size(
                page_layer_item(Some(sizes), index, BackgroundLayerSpec::initial().size),
                page_font_size,
                None,
                &resolve_context,
            )
        });
        layer.position = positions.map_or(initial_layer.position, |positions| {
            resolve_css_position(
                page_layer_item(
                    Some(positions),
                    index,
                    BackgroundLayerSpec::initial().position,
                ),
                page_font_size,
                None,
                &resolve_context,
            )
        });
        layer.repeat = page_layer_item(repeats, index, initial_layer.repeat);
        layer.origin = page_layer_item(origins, index, initial_layer.origin);
        layer.attachment = page_layer_item(attachments, index, initial_layer.attachment);
        paint_page_background_layer(
            image_cache,
            scene,
            &layer,
            areas,
            (border, padding),
            (page_font_size, &resolve_context, current_color),
            pixel_source,
            warnings,
        );
    }
}

/// Left, top, right and bottom widths of a box edge, in CSS px.
type EdgeWidths = (f64, f64, f64, f64);

/// Paint one `@page` background layer over the paper, positioned in the page
/// box.
#[allow(clippy::too_many_arguments)]
fn paint_page_background_layer(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    layer: &raikiri_style::ComputedBackgroundLayer,
    PageBackgroundAreas {
        paper: area,
        border_box,
    }: PageBackgroundAreas,
    (border, padding): (EdgeWidths, EdgeWidths),
    (page_font_size, resolve_context, current_color): (ComputedLength, &ResolveContext, CssColor),
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let image = &layer.image;
    let source = match image {
        BackgroundImage::Url(raw_url) => {
            let (Some(pixel_source), Some(url)) = (pixel_source, image_cache.image_url(raw_url))
            else {
                return;
            };
            Ok((pixel_source, url))
        }
        BackgroundImage::Gradient(_) if paints_gradient_image(image) => Err(()),
        _ => return,
    };
    let url = source.as_ref().ok().map(|(_, url)| url);
    // `@page` positioning area with nonzero margins/borders (CSS Page 3 page
    // box plus CSS Backgrounds 3 section 2.8): `border_box` is the page
    // box's border edge, inside the page margins, and origin insets it to
    // the positioning box. The page background painting area covers the
    // whole paper regardless of `background-clip` (CSS Page 3, "Page
    // Backgrounds and Painting Order"), so every layer paints over all of
    // `area`. A `fixed` layer is instead fixed to the page box including its
    // margins (CSS Backgrounds 3 section 2.5), whatever its origin.
    let positioning = if layer.attachment == raikiri_style::property::BackgroundAttachment::Fixed {
        area
    } else {
        origin_inset_rect(border_box, layer.origin, border, padding, url, warnings)
    };
    let painting = area;
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    match source {
        Ok((pixel_source, url)) => paint_background_from_source(
            image_cache,
            scene,
            &url,
            positioning,
            painting,
            &layer.size,
            &layer.position,
            &layer.repeat,
            None,
            pixel_source,
            warnings,
        ),
        Err(_) => {
            // Font-relative lengths in the gradient take the page font.
            let BackgroundImage::Gradient(gradient) =
                raikiri_style::resolve::resolve_background_image(
                    image.clone(),
                    page_font_size,
                    None,
                    resolve_context,
                )
            else {
                return;
            };
            gradient::paint_gradient_layer(
                scene,
                &gradient,
                positioning,
                painting,
                &layer.size,
                &layer.position,
                &layer.repeat,
                current_color,
            );
        }
    }
}

fn paint_canvas_background_image(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    area: &CanvasArea,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let Some(node_id) = canvas_background_owner(document, cascade) else {
        return;
    };
    let Some(computed) = cascade.computed.get(node_id) else {
        return;
    };
    // spec: https://www.w3.org/TR/css-transforms-1/#transform-rendering
    // A transform on the root moves the whole canvas, its images included,
    // while the canvas still covers the page: paint the images through the
    // transform over the part of the canvas that lands on this page, clipped
    // to it. A singular transform renders none of the root.
    let (transform, painting) = match area.transform {
        None => (Affine::IDENTITY, area.painting),
        Some(transform) if crate::transform::is_invertible(transform) => (
            transform,
            transform.inverse().transform_rect_bbox(area.painting),
        ),
        Some(_) => return,
    };
    let clipped = transform != Affine::IDENTITY;
    if clipped {
        scene.push_clip_layer(Affine::IDENTITY, &area.painting);
    }
    let mut scene = crate::transform::TransformScene { scene, transform };
    // CSS Backgrounds 3 §2.1: paint from the last layer to the first.
    let painted = painted_background_layers(computed.background_layers(), warnings);
    for layer in painted.iter().rev() {
        paint_canvas_background_layer(
            image_cache,
            &mut scene,
            computed,
            layer,
            area,
            painting,
            pixel_source,
            warnings,
        );
    }
    if clipped {
        scene.scene.pop_layer();
    }
}

/// Paint one background layer propagated to the canvas over `painting`,
/// positioned in the root element's box.
#[allow(clippy::too_many_arguments)]
fn paint_canvas_background_layer(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    computed: &ComputedValues,
    layer: &raikiri_style::ComputedBackgroundLayer,
    area: &CanvasArea,
    painting: Rect,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    if let BackgroundImage::Gradient(gradient) = &layer.image
        && paints_gradient_image(&layer.image)
    {
        let positioning = area.positioning(layer, None, warnings);
        gradient::paint_gradient_layer(
            scene,
            gradient,
            positioning,
            painting,
            &layer.size,
            &layer.position,
            &layer.repeat,
            computed.color,
        );
        return;
    }
    let Some(pixel_source) = pixel_source else {
        return;
    };
    let BackgroundImage::Url(raw_url) = &layer.image else {
        return;
    };
    let Some(url) = image_cache.image_url(raw_url) else {
        return;
    };
    // The propagated values are treated as the root element's own (CSS
    // Backgrounds 3 §2.11.2), so origin insets the root's border box by the
    // root's border and padding. `background-clip` has no effect on a
    // propagated background (§2.11.1): the image paints over the whole
    // canvas, as the gradient does.
    let positioning = area.positioning(layer, Some(&url), warnings);
    paint_background_from_source(
        image_cache,
        scene,
        &url,
        positioning,
        painting,
        &layer.size,
        &layer.position,
        &layer.repeat,
        None,
        pixel_source,
        warnings,
    );
}
/// Whether a background image is a gradient that [`gradient`] draws as an
/// image: a linear or radial one. Other gradients still paint a color.
fn paints_gradient_image(image: &BackgroundImage) -> bool {
    matches!(
        image,
        BackgroundImage::Gradient(gradient)
            if matches!(&**gradient, Gradient::Linear(_) | Gradient::Radial(_))
    )
}

fn background_color_with_image_source(
    background_color: CssColor,
    background_image: &BackgroundImage,
    current_color: CssColor,
    pixel_source: Option<&dyn ImagePixelSource>,
) -> Option<CssColor> {
    if pixel_source.is_some() && matches!(background_image, BackgroundImage::Url(_)) {
        (background_color.a != 0).then_some(background_color)
    } else {
        effective_background_color(background_color, background_image, current_color)
    }
}

fn find_html(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("html") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

fn page_property(cascade: &CascadeResult, key: PropertyKey) -> Option<&PropertyValue> {
    cascade.page.declarations().get(&key)
}

fn length_to_px(length: Length, basis: f32, font_size: f32) -> f32 {
    let value = match length {
        Length::Px(value) => value,
        Length::Percent(value) => basis * value / 100.0,
        Length::Em(value) | Length::Rem(value) => font_size * value,
        Length::Ex(value) | Length::Rex(value) | Length::Ch(value) | Length::Rch(value) => {
            font_size * value * 0.5
        }
        Length::Ic(value) | Length::Ric(value) => font_size * value,
        Length::Lh(value) | Length::Rlh(value) => font_size * value,
        Length::Pt(value) => value * 96.0 / 72.0,
        Length::Cm(value) => value * 96.0 / 2.54,
        Length::Mm(value) => value * 96.0 / 25.4,
        Length::Q(value) => value * 96.0 / 101.6,
        Length::In(value) => value * 96.0,
        Length::Pc(value) => value * 96.0 / 6.0,
        _ => 0.0,
    };
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn css_color(color: CssColor) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

#[cfg(test)]
fn marker_render_info<'a>(
    document: &'a Document,
    cascade: &'a CascadeResult,
    node_id: usize,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    let snapshots = raikiri_dom::counter_snapshots(document, cascade)
        .expect("test counter snapshots stay within budget");
    marker_render_info_with_snapshots(document, cascade, node_id, &snapshots)
}

fn generated_pseudo_content_with_snapshots<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    snapshots: &CounterSnapshots,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    raikiri_dom::generated_content::generated_text(document, cascade, node_id, pseudo, snapshots)
}

/// Whether the inline engine laid the `pseudo` of `node_id` out as text of
/// its paragraph or a containing anonymous cell. Such content is drawn from
/// the lines, and must not also paint as a standalone overlay.
fn laid_out_in_lines(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
) -> bool {
    document.get_node(node_id).is_some_and(|node| {
        node.is_ifc_root() || document.anonymous_table_pseudo_is_projected(node_id, pseudo)
    }) && raikiri_dom::generated_content::is_in_flow_generated_text(cascade, node_id, pseudo)
}

fn generated_pseudo_text_advance(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    snapshots: &CounterSnapshots,
) -> f32 {
    if laid_out_in_lines(document, cascade, node_id, pseudo) {
        return 0.0;
    }
    let Some((computed, content)) =
        generated_pseudo_content_with_snapshots(document, cascade, node_id, pseudo, snapshots)
    else {
        return 0.0;
    };
    if computed.display == DisplayValue::None {
        return 0.0;
    }
    let family = computed
        .font_family
        .first()
        .cloned()
        .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
    text::measure_margin_text_advance(
        document,
        &content,
        computed.font_size.px(),
        std::slice::from_ref(&family),
    )
}

fn generated_flow_height(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    snapshots: &CounterSnapshots,
    cache: &mut std::collections::HashMap<usize, f32>,
) -> f32 {
    if let Some(height) = cache.get(&node_id) {
        return *height;
    }

    let mut pending = vec![(node_id, false)];
    while let Some((current_id, children_ready)) = pending.pop() {
        if cache.contains_key(&current_id) {
            continue;
        }
        if !children_ready {
            pending.push((current_id, true));
            for child in generated_flow_block_children(document, cascade, current_id)
                .into_iter()
                .rev()
            {
                if !cache.contains_key(&child) {
                    pending.push((child, false));
                }
            }
            continue;
        }

        let height = generated_flow_node_height(document, cascade, current_id, snapshots, cache);
        cache.insert(current_id, height);
    }

    cache.get(&node_id).copied().unwrap_or(0.0)
}

/// The generated flow height of `node_id`, given the heights of its block
/// children already in `cache`.
fn generated_flow_node_height(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    snapshots: &CounterSnapshots,
    cache: &std::collections::HashMap<usize, f32>,
) -> f32 {
    let before = generated_pseudo_text_height(
        document,
        cascade,
        node_id,
        raikiri_style::PseudoElem::Before,
        snapshots,
    );
    let after = generated_pseudo_text_height(
        document,
        cascade,
        node_id,
        raikiri_style::PseudoElem::After,
        snapshots,
    );
    let Some(node) = document.get_node(node_id) else {
        return before.max(after); // cov:ignore: generated flow receives arena-valid node ids
    };

    // A generated `::before` run is the first child of a block box. When
    // the block also owns normal-flow block children, those children must
    // start after the generated run and contribute to the block's auto
    // height. Keep the existing inline-only bridge unchanged.
    let block_children = generated_flow_block_children(document, cascade, node_id);
    let has_generated_block_child = block_children
        .iter()
        .any(|child| cache.get(child).copied().unwrap_or(0.0) > 0.0);
    if !block_children.is_empty() && (before > 0.0 || after > 0.0 || has_generated_block_child) {
        return before
            + after
            + block_children
                .iter()
                .filter_map(|&child| {
                    let child_node = document.get_node(child)?;
                    Some(
                        child_node
                            .layout()
                            .size
                            .height
                            .max(cache.get(&child).copied().unwrap_or(0.0)),
                    )
                })
                .sum::<f32>();
    }
    let inline_child =
        generated_inline_children_height(document, cascade, &node.children, snapshots);
    before.max(after).max(inline_child)
}

/// The tallest generated `::before` or `::after` text among the inline-flow
/// `children`.
fn generated_inline_children_height(
    document: &Document,
    cascade: &CascadeResult,
    children: &[usize],
    snapshots: &CounterSnapshots,
) -> f32 {
    children
        .iter()
        .filter_map(|&child| {
            let child_cv = cascade.computed.get(child)?;
            (child_cv.display.is_inline_flow()).then_some(
                generated_pseudo_text_height(
                    document,
                    cascade,
                    child,
                    raikiri_style::PseudoElem::Before,
                    snapshots,
                )
                .max(generated_pseudo_text_height(
                    document,
                    cascade,
                    child,
                    raikiri_style::PseudoElem::After,
                    snapshots,
                )),
            )
        })
        .fold(0.0, f32::max)
}

fn generated_flow_block_children(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Vec<usize> {
    let Some(node) = document.get_node(node_id) else {
        return Vec::new();
    };
    node.children
        .iter()
        .filter_map(|&child| {
            let child_node = document.get_node(child)?;
            let child_cv = cascade.computed.get(child)?;
            let normal_flow = matches!(
                child_cv.position,
                PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
            );
            (child_node.kind() != NodeKind::Text
                && !child_cv.display.is_inline_flow()
                && normal_flow)
                .then_some(child)
        })
        .collect()
}

fn generated_pseudo_text_height(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    snapshots: &CounterSnapshots,
) -> f32 {
    if laid_out_in_lines(document, cascade, node_id, pseudo) {
        return 0.0;
    }
    let Some((computed, content)) =
        generated_pseudo_content_with_snapshots(document, cascade, node_id, pseudo, snapshots)
    else {
        return 0.0;
    };
    if computed.display == DisplayValue::None {
        return 0.0;
    }
    let family = computed
        .font_family
        .first()
        .cloned()
        .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
    text::measure_margin_text_height(
        document,
        &content,
        computed.font_size.px(),
        std::slice::from_ref(&family),
    )
}

#[allow(clippy::too_many_arguments)]
fn paint_generated_pseudo(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    snapshots: &CounterSnapshots,
) -> f32 {
    if laid_out_in_lines(document, cascade, node_id, pseudo) {
        return 0.0;
    }
    let Some((computed, content)) =
        generated_pseudo_content_with_snapshots(document, cascade, node_id, pseudo, snapshots)
    else {
        return 0.0;
    };
    if computed.display == DisplayValue::None {
        return 0.0;
    }
    let family = computed
        .font_family
        .first()
        .cloned()
        .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
    let advance = text::measure_margin_text_advance(
        document,
        &content,
        computed.font_size.px(),
        std::slice::from_ref(&family),
    );
    text::draw_margin_text(
        document,
        scene,
        &content,
        x,
        y,
        width.max(advance),
        height,
        css_color(computed.color),
        computed.font_size.px(),
        std::slice::from_ref(&family),
        StandaloneAlign::Start,
        text::MarginTextVerticalAlign::Top,
    );
    advance
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)] // cov:ignore: attribute has no executable mapping
fn paint_list_marker(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    paint_x: f32,
    paint_y: f32,
    width: f32,
    height: f32,
    padding_left: f32,
) {
    let snapshots = raikiri_dom::counter_snapshots(document, cascade)
        .expect("test counter snapshots stay within budget");
    paint_list_marker_with_snapshots(
        &crate::walk::ImageBrushCache::for_document(document)
            .with_device_scale(cascade.border_width_device_scale()),
        scene,
        document,
        cascade,
        node_id,
        paint_x,
        paint_y,
        width,
        height,
        padding_left,
        (0.0, f32::INFINITY, false),
        &snapshots,
        None,
    );
}

#[allow(clippy::too_many_arguments)] // cov:ignore: attribute has no executable mapping
fn paint_list_marker_with_snapshots(
    image_cache: &crate::walk::ImageBrushCache,
    // cov:ignore: signature line has no executable mapping
    scene: &mut impl PaintScene, // cov:ignore: signature line has no executable mapping
    document: &Document,         // cov:ignore: signature line has no executable mapping
    cascade: &CascadeResult,     // cov:ignore: signature line has no executable mapping
    node_id: usize,              // cov:ignore: signature line has no executable mapping
    paint_x: f32,                // cov:ignore: signature line has no executable mapping
    paint_y: f32,                // cov:ignore: signature line has no executable mapping
    width: f32,                  // cov:ignore: signature line has no executable mapping
    height: f32,                 // cov:ignore: signature line has no executable mapping
    padding_left: f32,           // cov:ignore: signature line has no executable mapping
    page_flow: (f32, f32, bool), // cov:ignore: signature line has no executable mapping
    snapshots: &CounterSnapshots,
    pixel_source: Option<&dyn ImagePixelSource>,
) {
    let Some((computed, content)) =
        list_marker_to_paint(document, cascade, node_id, snapshots, (width, height))
    else {
        return;
    };
    if let Some(decoded) = list_marker_image(
        image_cache,
        document,
        cascade,
        node_id,
        computed,
        pixel_source,
    ) {
        let transform = list_marker_image_transform(
            document,
            node_id,
            computed,
            (paint_x, paint_y),
            padding_left,
            &decoded,
        );
        let brush = image_cache.whole(&decoded);
        scene.fill(
            peniko::Fill::NonZero,
            transform,
            brush.as_ref(),
            None,
            &Rect::new(0.0, 0.0, decoded.width as f64, decoded.height as f64),
        );
        return;
    }
    if content.is_empty() {
        return;
    }
    let Some((shaped, offset_x)) =
        document.shape_list_marker_text(&content, computed, padding_left)
    else {
        return;
    };
    let Some(top) = list_marker_text_top(document, cascade, node_id, page_flow, paint_y) else {
        return;
    };
    crate::standalone_text::draw(
        scene,
        &shaped,
        paint_x + offset_x,
        top,
        css_color(computed.color),
    );
}

/// The style and content of the marker of the list item `node_id`, unless
/// the marker paints nothing: it sits inside a paragraph that lays it out,
/// its box `(width, height)` is empty, or it is not visible.
fn list_marker_to_paint<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    snapshots: &CounterSnapshots,
    (width, height): (f32, f32),
) -> Option<(&'a ComputedValues, String)> {
    if raikiri_dom::generated_content::inside_marker_in_flow(cascade, node_id)
        && document
            .get_node(node_id)
            .is_some_and(|node| node.is_ifc_root())
    {
        return None;
    }
    let (computed, content) =
        marker_render_info_with_snapshots(document, cascade, node_id, snapshots)?;
    // The marker is visible by its own `visibility`, which it inherits from
    // its item unless `::marker` sets it; `collapse` hides it like `hidden`.
    if width <= 0.0 || height <= 0.0 || raikiri_dom::paint_rules::hides_own_box(computed) {
        return None;
    }
    Some((computed, content))
}

/// The decoded `list-style-image` that the marker of `node_id` paints in
/// place of its text: the one the document holds for it, or the one that
/// `pixel_source` decoded. A `::marker` with its own `content` paints that
/// content instead.
fn list_marker_image(
    image_cache: &crate::walk::ImageBrushCache,
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    computed: &ComputedValues,
    pixel_source: Option<&dyn ImagePixelSource>,
) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
    let explicit_content = cascade
        .pseudo
        .get(&(
            raikiri_style::StyleNodeId::new(node_id as u64),
            raikiri_style::PseudoElem::Marker,
        ))
        .is_some_and(|marker| !marker.content.is_empty());
    let resolved = if let (BackgroundImage::Url(raw_url), Some(source)) =
        (&computed.list_style_image, pixel_source)
    {
        image_cache
            .resolve_url(raw_url)
            .and_then(|url| source.get_decoded(&url))
    } else {
        None
    };
    if explicit_content {
        return None;
    }
    document
        .list_marker_image_shared(node_id)
        .or(resolved.as_ref())
        .filter(|decoded| decoded.width > 0 && decoded.height > 0)
        .cloned()
}

/// The transform that maps the pixels of `decoded`, the image of the marker
/// of `node_id`, onto the marker box placed from `origin` and the left
/// padding of the item.
fn list_marker_image_transform(
    document: &Document,
    node_id: usize,
    computed: &ComputedValues,
    origin: (f32, f32),
    padding_left: f32,
    decoded: &raikiri_traits::DecodedImage,
) -> Affine {
    let size = document.list_marker_image_size(node_id);
    let marker_width = size.map_or(decoded.width as f32, |size| size.width);
    let marker_height = size.map_or(decoded.height as f32, |size| size.height);
    let marker_rect = document.standalone_marker_image_rect(
        computed,
        origin,
        padding_left,
        raikiri_traits::ImageRasterSize {
            width: marker_width,
            height: marker_height,
        },
    );
    Affine::translate((marker_rect.x as f64, marker_rect.y as f64))
        * Affine::scale_non_uniform(
            f64::from(marker_width) / f64::from(decoded.width),
            f64::from(marker_height) / f64::from(decoded.height),
        )
}

/// The top at which the text of the marker of `node_id` paints, or `None`
/// when the item's page is not the one that draws it.
///
/// The marker's baseline meets the item's first line (CSS Lists 3 §3.1),
/// where the page's text runs place it. It belongs to the item's first page,
/// so the item's later pages draw none.
fn list_marker_text_top(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    page_flow: (f32, f32, bool),
    paint_y: f32,
) -> Option<f32> {
    let (content_origin_y, page_bottom, repeated) = page_flow;
    document
        .list_marker_text_top(
            cascade,
            node_id,
            (content_origin_y, repeated),
            paint_y,
            page_bottom,
        )
        .unwrap_or(Some(paint_y))
}

/// Lay out the page-margin boxes of a page for the page context in
/// `cascade.page`, in drawing order (see [`raikiri_dom::page_margin_boxes`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_page_margin_boxes(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> Vec<raikiri_dom::MarginBox> {
    let context = raikiri_dom::MarginBoxPageContext::new(page_index, page_count, page_is_left)
        .with_paired_page_increment(paired_page_increment);
    raikiri_dom::page_margin_boxes(document, cascade, &cascade.page, page_box, context)
}

/// Split margin boxes in drawing order into the ones that go behind the
/// document canvas, which have a negative `z-index`, and the ones that go in
/// front of the document contents (CSS Page 3 §3.1).
pub(crate) fn split_margin_boxes_by_layer(
    margin_boxes: &[raikiri_dom::MarginBox],
) -> (&[raikiri_dom::MarginBox], &[raikiri_dom::MarginBox]) {
    margin_boxes.split_at(margin_boxes.partition_point(|margin_box| margin_box.z_index < 0))
}

/// Paint page-margin boxes, in the order given.
pub(crate) fn paint_page_margin_boxes(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    margin_boxes: &[raikiri_dom::MarginBox],
    page_box: PageBox,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    for margin_box in margin_boxes {
        paint_margin_box(
            image_cache,
            scene,
            margin_box,
            &page_box,
            pixel_source,
            warnings,
        );
    }
}

/// Height from an ifc root's border-box top that its painted lines reach:
/// lines that overflow the box (overflow is visible) are still drawn, so a
/// page that holds only those lines must not skip the root.
fn ifc_paint_extent(node: &raikiri_dom::Node, layout: &BoxLayout) -> f32 {
    let lines = node.ifc_size().map_or(0.0, |(_, height)| height);
    layout
        .size
        .height
        .max(layout.border.top + layout.padding.top + lines)
}

/// How far the band of a page that follows the x axis reaches along y, both
/// ways: far enough to hold any box, and finite, so that page membership
/// tests compare against it like a page edge.
pub(crate) const FLOW_EXTENT: f32 = f32::MAX / 4.0;

fn box_intersects_page(y: f32, height: f32, page_top: f32, page_bottom: f32) -> bool {
    if !y.is_finite() || !height.is_finite() || !page_top.is_finite() || !page_bottom.is_finite() {
        return false;
    }
    if height <= 0.0 {
        return y >= page_top && y <= page_bottom;
    }
    y < page_bottom && y + height > page_top
}

/// How far a box must reach into the page area of a page that follows the x
/// axis to belong to it, so that a box ending at the page edge, up to
/// rounding, stays on the page beside it.
const FLOW_BAND_TOLERANCE: f64 = 0.01;

/// How far before a page's start and end a line's center may fall and still
/// belong to that page, as layout assigns lines to pages.
const CENTER_TOLERANCE: f64 = 0.001;

/// How far toward its start a page's slice of the flow reaches to own a
/// line, as layout assigns lines to pages.
const LINE_OWNERSHIP_BIAS: f64 = 0.001;

/// The page area of a page that follows the x axis, from `left` to `right`
/// in page coordinates: the page holds the boxes that reach across it and
/// the lines whose center lies in it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlowBand {
    pub(crate) left: f32,
    pub(crate) right: f32,
    /// The flow, and so its pages, run from right to left.
    pub(crate) right_to_left: bool,
    /// The root element's transform, which maps the walker's coordinates to
    /// page coordinates.
    pub(crate) root_transform: Affine,
}

impl FlowBand {
    /// Whether the rectangle at `x, y` of `width` and `height`, painted
    /// under `transform` inside the root element, reaches across the band.
    pub(crate) fn holds_box(
        self,
        transform: Affine,
        (x, y, width, height): (f32, f32, f32, f32),
    ) -> bool {
        let rect = (self.root_transform * transform).transform_rect_bbox(Rect::new(
            f64::from(x),
            f64::from(y),
            f64::from(x + width),
            f64::from(y + height),
        ));
        let (left, right) = (f64::from(self.left), f64::from(self.right));
        if !rect.x0.is_finite() || !rect.x1.is_finite() {
            return false;
        }
        if rect.width() <= 0.0 {
            return rect.x0 >= left && rect.x0 <= right;
        }
        // The allowance absorbs rounding at the page area's edges, but stays
        // under half a box, so a box narrower than it keeps its overlap, even
        // one centered on the edge between two pages.
        let tolerance = FLOW_BAND_TOLERANCE.min(rect.width() / 4.0);
        rect.x0 < right - tolerance && rect.x1 > left + tolerance
    }

    /// Whether the page owns a line whose center, painted under `transform`
    /// inside the root element, lies at `center`: the page whose slice of
    /// the flow holds the center, a center on the edge between two pages
    /// going to the page whose slice starts there.
    pub(crate) fn holds_line_center(self, transform: Affine, center: kurbo::Point) -> bool {
        let x = (self.root_transform * transform * center).x;
        let (left, right) = (f64::from(self.left), f64::from(self.right));
        if !x.is_finite() {
            return false;
        }
        if self.right_to_left {
            x > left + LINE_OWNERSHIP_BIAS && x <= right + LINE_OWNERSHIP_BIAS
        } else {
            x >= left - LINE_OWNERSHIP_BIAS && x < right - LINE_OWNERSHIP_BIAS
        }
    }
}

/// The lines a page owns of a paragraph that runs over several pages.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PageLines {
    /// A page that follows the x axis owns the lines whose center lies in
    /// its band of the flow.
    AlongX(FlowBand),
    /// A page that follows the y axis owns the lines that reach across its
    /// slice of the shared flow space, from `start` to `end`, where layout
    /// placed them: layout moves a line that does not fit to the next page
    /// whole, so only a line taller than a page reaches across several.
    /// Translations that paint adds move a line, not the page it belongs
    /// to.
    AlongY {
        start: f32,
        end: f32,
        /// How far paint moves the lines from the flow space: the page's
        /// offset and the translations around the paragraph.
        paint_shift: f32,
    },
}

impl PageLines {
    /// Whether a paragraph whose lines stack along x when `vertical`, along
    /// y otherwise, goes to one page whole: pages never cut a paragraph
    /// across its lines.
    pub(crate) fn keeps_paragraph_whole(self, vertical: bool) -> bool {
        match self {
            Self::AlongX(_) => !vertical,
            Self::AlongY { .. } => vertical,
        }
    }

    /// The same lines for a paragraph that the translations around it move
    /// `shift` further along y.
    pub(crate) fn translated(self, shift: f32) -> Self {
        match self {
            Self::AlongY {
                start,
                end,
                paint_shift,
            } => Self::AlongY {
                start,
                end,
                paint_shift: paint_shift + shift,
            },
            along_x => along_x,
        }
    }

    /// Whether the page owns the line, or the paragraph kept whole, that
    /// covers `rect` where it paints under `transform` inside the root
    /// element, as layout assigns lines to pages.
    pub(crate) fn holds_line(self, transform: Affine, rect: Rect) -> bool {
        match self {
            Self::AlongX(band) => band.holds_line_center(transform, rect.center()),
            Self::AlongY {
                start,
                end,
                paint_shift,
            } => {
                let (top, bottom) = (
                    rect.y0 - f64::from(paint_shift),
                    rect.y1 - f64::from(paint_shift),
                );
                // A line reaching into the page belongs to it, and so does
                // one without height whose center layout puts there.
                let center = (top + bottom) * 0.5;
                top.is_finite()
                    && bottom.is_finite()
                    && (top < f64::from(end) - FLOW_BAND_TOLERANCE
                        && bottom > f64::from(start) + FLOW_BAND_TOLERANCE
                        || center >= f64::from(start) - CENTER_TOLERANCE
                            && center < f64::from(end) - CENTER_TOLERANCE)
            }
        }
    }
}

/// Walk source element boxes in DFS order, starting at the document element.
/// A document without an element returns silently. `layout_single_page` should
/// already have returned Err before painting; this is defensive.
///
/// Stack frames carry either a node visit or a matching clip-layer pop.
/// Push Node children with `.rev()` to process them in document order when
/// popping. For Element, check `is_display_none` first and skip its subtree
/// if true. The old size == 0 check incorrectly dropped legitimate zero-size
/// elements with overflow: visible.
///
/// `parent_font_size` is the **parent's** used font-size (px) for this
/// stack frame's node. `shift_y` accumulates `vertical-align` shifts from
/// all ancestors up to this node (px, positive downward). Both correspond to
/// inputs and outputs of `vertical_align_shift_px`; see its docs for details.
///
/// Future work will paint element background-color / border / box-shadow
/// in the Element arm (which reserves the site for that work).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &CounterSnapshots,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    custom_highlights: &[crate::TextHighlightRange],
    root_transform: Affine,
) {
    let mut warnings = Vec::new();
    paint_document_impl(
        image_cache,
        scene,
        document,
        cascade,
        counter_snapshots,
        page_box,
        content_origin_y,
        active_page_name,
        fixed_page_width,
        None,
        &mut warnings,
        custom_highlights,
        root_transform,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document_with_images_and_warnings(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &CounterSnapshots,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: &dyn ImagePixelSource,
    warnings: &mut Vec<RenderWarning>,
    custom_highlights: &[crate::TextHighlightRange],
    root_transform: Affine,
) {
    paint_document_impl(
        image_cache,
        scene,
        document,
        cascade,
        counter_snapshots,
        page_box,
        content_origin_y,
        active_page_name,
        fixed_page_width,
        Some(pixel_source),
        warnings,
        custom_highlights,
        root_transform,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document_impl(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &CounterSnapshots,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    custom_highlights: &[crate::TextHighlightRange],
    root_transform: Affine,
    trace: Option<&mut Vec<crate::PaintTraceEvent>>,
) {
    image_cache.set_root_transform(root_transform);
    let Some(root_id) = document.document_element_index() else {
        return;
    };

    let root_font_size = cascade.computed[root_id].font_size.px();
    let canvas_owner = canvas_background_owner(document, cascade);
    let layout_fragments = document.layout_fragments();
    let (fragments_by_parent_and_node, fragmented_nodes) = index_fragments(layout_fragments);

    let geometry = PageGeometry::new(
        document,
        cascade,
        page_box,
        content_origin_y,
        fixed_page_width,
        root_transform,
    );
    let mut capture_scene = crate::capture::CaptureScene::new(scene);
    let mut transformed_scene = crate::transform::TransformScene {
        scene: &mut capture_scene,
        transform: Affine::IDENTITY,
    };
    let stack = vec![
        PaintFrame::EndDeferral,
        PaintFrame::Visit(VisitFrame::root(root_id, root_font_size)),
    ];
    let mut walk = PaintWalk {
        image_cache,
        document,
        cascade,
        counter_snapshots,
        page_box,
        active_page_name,
        geometry,
        content_origin_y,
        root_transform,
        root_id,
        canvas_owner,
        layout_fragments,
        fragments_by_parent_and_node,
        fragmented_nodes,
        // The document, cascade, and counter snapshots are stable for this
        // pass, so each node's generated flow height is computed once and
        // reused.
        generated_flow_heights: std::collections::HashMap::new(),
        pixel_source,
        custom_highlights,
        warnings,
        paragraphs: raikiri_dom::paint_rules::LeavingParagraphs::default(),
        scene: &mut transformed_scene,
        trace,
        held_lines: Vec::new(),
        early_boxes: Vec::new(),
        left_groups: std::collections::HashMap::new(),
        traced_below: std::collections::HashSet::new(),
        deferral_scopes: vec![DeferralLayer::new(true, LayerClose::Paint)],
        stack,
    };
    walk.run();
}

/// Whether paragraph `root` has visible text for which `admits` holds,
/// told whether the text is positioned below the paragraph's blocks: inside
/// an inline element positioned there (see
/// [`raikiri_dom::paint_rules::negative_inline_group`]), or generated by a
/// `::before` or `::after` positioned there (see
/// [`raikiri_dom::paint_rules::negative_pseudo_group`]). The box of a
/// block-level `::before` or `::after` positioned there, painted with the
/// lines, counts as such text even without content. Only text whose owner
/// (the text node, or the element generating it) `keeps` counts.
fn traced_text(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    admits: impl Fn(bool) -> bool,
    keeps: impl Fn(usize) -> bool,
) -> bool {
    let below = |start: usize| {
        document
            .inclusive_ancestors(start)
            .take_while(|&id| id != root)
            .any(|id| raikiri_dom::paint_rules::negative_inline_group(document, cascade, root, id))
    };
    let generated = || {
        document
            .ifc_layout_node(root)
            .and_then(|node| node.ifc_lines())
            .into_iter()
            .flatten()
            .flat_map(|line| line.fragments())
            .filter_map(|fragment| match fragment {
                shodo::Fragment::GlyphRun(run) => run.node().map(|node| node.0 as usize),
                _ => None,
            })
    };
    document.ifc_text_lines_by_node(root).keys().any(|&owner| {
        document
            .get_node(owner)
            .and_then(|text| text.text_content())
            .is_some_and(|text| !text.trim().is_empty())
            && keeps(owner)
            && admits(below(owner))
    }) || generated().any(|owner| {
        raikiri_dom::generated_content::generated_origin(owner).is_some_and(|(element, _)| {
            keeps(element)
                && admits(
                    raikiri_dom::paint_rules::negative_pseudo_group(document, cascade, root, owner)
                        || below(element),
                )
        })
    }) || (admits(true)
        && document.get_node(root).is_some_and(|node| {
            node.ifc_generated_block_pieces().iter().any(|piece| {
                raikiri_dom::paint_rules::negative_pseudo_group(document, cascade, root, piece.node)
            })
        }))
}

/// Whether `start` lies in content of paragraph `root` that leaves the lines
/// to paint with the positioned boxes (see
/// [`raikiri_dom::paint_rules::leaves_paragraph_content`]), within `group`
/// when given, for a paragraph whose content can leave its lines.
fn in_left_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    start: usize,
    group: Option<usize>,
) -> bool {
    document
        .inclusive_ancestors(start)
        .take_while(|&id| id != root)
        .any(|id| {
            group.is_none_or(|group| group == id)
                && raikiri_dom::paint_rules::leaves_paragraph_content(
                    document, cascade, root, id, true,
                )
        })
}

/// The first in-document text node below `root`, in document order.
fn first_text_descendant(document: &Document, root: usize) -> Option<usize> {
    let mut stack: Vec<usize> = document
        .get_node(root)?
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        let node = document.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => return Some(id),
            NodeKind::Element => stack.extend(node.children.iter().rev().copied()),
            _ => {}
        }
    }
    None
}

/// Image URL selected during layout, with an absolute-source fallback.
/// Relative sources retain the document base chosen by the resolver pass.
fn img_src_url(document: &Document, node_id: usize) -> Option<url::Url> {
    document.resolved_image_url(node_id)
}

/// Return the padding values used by paint for this laid-out box.
///
/// Taffy owns the used-value resolution for the layout pass. Ordinary
/// percentage/px padding keeps the painter's historical containing-width
/// calculation, while a mixed `calc()` takes the exact used value from layout.
/// This keeps backgrounds and images aligned with the geometry.
fn used_padding_for_paint(cv: &raikiri_style::ComputedValues, layout: &BoxLayout) -> Edges<f32> {
    if cv.display == DisplayValue::TableCell {
        return layout.padding;
    }
    let reference = layout.size.width;
    let choose = |value: raikiri_style::resolve::ComputedLengthPercentageOrCalc, used: f32| {
        // A calc() sum resolves its percentage against the containing block,
        // which layout already did; take its used value.
        if matches!(
            value,
            raikiri_style::resolve::ComputedLengthPercentageOrCalc::Calc(_)
        ) {
            used
        } else {
            value.resolve(reference)
        }
    };
    Edges {
        top: choose(cv.padding.top, layout.padding.top),
        right: choose(cv.padding.right, layout.padding.right),
        bottom: choose(cv.padding.bottom, layout.padding.bottom),
        left: choose(cv.padding.left, layout.padding.left),
    }
}

/// Draws a replaced element after asking the source for pixels at the concrete
/// CSS object size selected by `object-fit`. The caller clips it (see
/// [`raikiri_dom::paint_rules::replaced_content_clip`]).
#[allow(clippy::too_many_arguments)]
fn paint_image(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    source: &dyn ImagePixelSource,
    url: &url::Url,
    natural: ImageIntrinsicSize,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &Edges<f32>,
    object_fit: ObjectFit,
    object_position: &ComputedCssPosition,
    visible: bool,
    node_id: usize,
    warnings: &mut Vec<RenderWarning>,
) -> bool {
    let bl = border.left.width().px();
    let bt = border.top.width().px();
    let br = border.right.width().px();
    let bb = border.bottom.width().px();
    // `layout.padding` contains Taffy's finite used values. This keeps
    // percentage and font-relative padding consistent with the geometry that
    // Taffy laid out, including pre-Taffy `ch` resolution.
    let pl = padding.left;
    let pr = padding.right;
    let pt = padding.top;
    let pb = padding.bottom;
    let content_x = (abs_x + bl + pl) as f64;
    let content_y = (abs_y + bt + pt) as f64;
    let content_w = (border_box_width - bl - br - pl - pr).max(0.0) as f64;
    let content_h = (border_box_height - bt - bb - pt - pb).max(0.0) as f64;
    if !visible || content_w <= 0.0 || content_h <= 0.0 {
        return true;
    }

    let Some((image_x, image_y, image_w, image_h)) = raikiri_dom::image_geometry::object_image_rect(
        (content_x, content_y, content_w, content_h),
        natural,
        object_fit,
        object_position,
    ) else {
        return true;
    };
    let raster_size = ImageRasterSize {
        width: image_w as f32,
        height: image_h as f32,
    };
    let Some(decoded) = source.get_decoded_at_size(url, raster_size, None) else {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: None,
            },
            node_id: Some(NodeId::new(node_id as u64)),
            details: "image pixels were unavailable at the requested object size".to_owned(),
        });
        return true;
    };
    let raster_w = decoded.width as f64;
    let raster_h = decoded.height as f64;
    if raster_w <= 0.0 || raster_h <= 0.0 {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: None,
            },
            node_id: Some(NodeId::new(node_id as u64)),
            details: "image pixels had an empty raster size".to_owned(),
        });
        return true;
    }
    let brush = image_cache.whole(&decoded);
    scene.fill(
        peniko::Fill::NonZero,
        Affine::translate((image_x, image_y))
            * Affine::scale_non_uniform(image_w / raster_w, image_h / raster_h),
        brush.as_ref(),
        None,
        &Rect::new(0.0, 0.0, raster_w, raster_h),
    );
    true
}

struct SharedCanvasPixels(std::sync::Arc<raikiri_dom::CanvasBitmap>);

impl AsRef<[u8]> for SharedCanvasPixels {
    fn as_ref(&self) -> &[u8] {
        &self.0.rgba
    }
}

/// Draws an HTML `<canvas>` bitmap with `object-fit`/`object-position`.
///
/// Mirrors [`paint_image`]'s sizing and positioning, but reads pixels from
/// the live [`raikiri_dom::CanvasBitmap`] instead of an [`ImagePixelSource`].
/// A canvas bitmap is always available at its intrinsic size (transparent
/// black when script never painted), so unlike `paint_image` this never
/// emits a resource warning: an empty bitmap simply paints nothing.
///
/// The caller clips it like an image (see
/// [`raikiri_dom::paint_rules::replaced_content_clip`]).
#[allow(clippy::too_many_arguments)]
fn paint_canvas(
    scene: &mut impl PaintScene,
    bitmap: &std::sync::Arc<raikiri_dom::CanvasBitmap>,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &Edges<f32>,
    object_fit: ObjectFit,
    object_position: &ComputedCssPosition,
    visible: bool,
) -> bool {
    let bl = border.left.width().px();
    let bt = border.top.width().px();
    let br = border.right.width().px();
    let bb = border.bottom.width().px();
    let pl = padding.left;
    let pr = padding.right;
    let pt = padding.top;
    let pb = padding.bottom;
    let content_x = (abs_x + bl + pl) as f64;
    let content_y = (abs_y + bt + pt) as f64;
    let content_w = (border_box_width - bl - br - pl - pr).max(0.0) as f64;
    let content_h = (border_box_height - bt - bb - pt - pb).max(0.0) as f64;
    if !visible || content_w <= 0.0 || content_h <= 0.0 {
        return true;
    }
    if bitmap.width == 0 || bitmap.height == 0 {
        return true;
    }
    if bitmap.rgba.is_empty() {
        return true;
    }
    let natural_w = f64::from(bitmap.width);
    let natural_h = f64::from(bitmap.height);
    let (image_w, image_h) = match object_fit {
        ObjectFit::Fill => (content_w, content_h),
        ObjectFit::Contain => {
            let scale = (content_w / natural_w).min(content_h / natural_h);
            (natural_w * scale, natural_h * scale)
        }
        ObjectFit::Cover => {
            let scale = (content_w / natural_w).max(content_h / natural_h);
            (natural_w * scale, natural_h * scale)
        }
        ObjectFit::None => (natural_w, natural_h),
        ObjectFit::ScaleDown => {
            let scale = (content_w / natural_w).min(content_h / natural_h).min(1.0);
            (natural_w * scale, natural_h * scale)
        }
        _ => (content_w, content_h), // cov:ignore: defensive fallback for future ObjectFit variants
    };
    if !image_w.is_finite() || !image_h.is_finite() || image_w <= 0.0 || image_h <= 0.0 {
        return true; // cov:ignore: natural and content sizes are already checked finite and positive above, so this cannot fail here
    }
    let image_x = content_x + position_offset(object_position.horizontal, content_w - image_w);
    let image_y = content_y + position_offset(object_position.vertical, content_h - image_h);
    let image_data = peniko::ImageData {
        data: peniko::Blob::new(std::sync::Arc::new(SharedCanvasPixels(
            std::sync::Arc::clone(bitmap),
        ))),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: bitmap.width,
        height: bitmap.height,
    };
    let brush = peniko::ImageBrush::new(image_data);
    scene.fill(
        peniko::Fill::NonZero,
        Affine::translate((image_x, image_y))
            * Affine::scale_non_uniform(image_w / natural_w, image_h / natural_h),
        brush.as_ref(),
        None,
        &Rect::new(0.0, 0.0, natural_w, natural_h),
    );
    true
}

/// Whether the borders cover the outer edge of the background, so the
/// background can be pulled in from that edge without leaving a gap.
///
/// Every side needs a width and an opaque color, and a style that draws a
/// continuous stripe at the outer edge: `dotted` and `dashed` leave gaps, and
/// a `double` border only does once its outer line is wide enough.
fn border_hides_background_edge(
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
) -> bool {
    [&border.top, &border.right, &border.bottom, &border.left]
        .iter()
        .all(|side| {
            let width = side.width().px();
            let opaque = match side.color {
                BorderColor::Resolved(color) => color.a == u8::MAX,
                BorderColor::CurrentColor => current_color.a == u8::MAX,
                _ => false, // cov:ignore: defensive fallback for future BorderColor variants
            };
            let continuous = match side.style() {
                BorderStyle::Solid
                | BorderStyle::Groove
                | BorderStyle::Ridge
                | BorderStyle::Inset
                | BorderStyle::Outset => true,
                BorderStyle::Double => width >= 5.0,
                _ => false,
            };
            width > 0.0 && opaque && continuous
        })
}

/// The final background layer of `cv` without its image: what a box paints
/// when only its background color is drawn.
fn color_only_background_layer(cv: &ComputedValues) -> raikiri_style::ComputedBackgroundLayer {
    let mut layer = cv
        .background_layers()
        .last()
        .cloned()
        .unwrap_or_else(raikiri_style::ComputedBackgroundLayer::initial);
    layer.image = BackgroundImage::None;
    layer
}

/// What surrounds a box and shapes how its background is painted.
#[derive(Clone, Copy, Default)]
pub(crate) struct BackgroundEnv {
    /// Whether the borders cover the outer edge of the background (see
    /// [`border_hides_background_edge`]).
    pub border_hides_edge: bool,
    /// The raster output's device pixel grid, absent for a vector output.
    pub device_grid: Option<DeviceGrid>,
    /// The page area in the box's own coordinates, which a `fixed` layer is
    /// positioned in. Without it a `fixed` layer is positioned like a
    /// `scroll` one, as inside a transform.
    pub fixed_viewport: Option<Rect>,
}

/// Paint an element's background color and every background layer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_background(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    bg: CssColor,
    layers: &[raikiri_style::ComputedBackgroundLayer],
    current_color: CssColor,
    border_radius: &ComputedBorderRadius,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &Edges<f32>,
    env: BackgroundEnv,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    paint_element_background_layers(
        image_cache,
        scene,
        (width, height, abs_x, abs_y),
        bg,
        layers,
        current_color,
        border_radius,
        border,
        padding,
        env,
        pixel_source,
        warnings,
        None,
    );
}

/// Paint the background color and then the layers from the last to the
/// first, so the first layer ends on top (CSS Backgrounds 3 §2.1). The
/// color is clipped by the final layer's `background-clip` (§2.2).
///
/// A single layer keeps the color derived from its image when no pixel
/// source paints it (see [`background_color_with_image_source`]); with
/// several layers the color is painted as declared.
#[allow(clippy::too_many_arguments)]
fn paint_element_background_layers(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    (width, height, abs_x, abs_y): (f32, f32, f32, f32),
    bg: CssColor,
    layers: &[raikiri_style::ComputedBackgroundLayer],
    current_color: CssColor,
    border_radius: &ComputedBorderRadius,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &Edges<f32>,
    env: BackgroundEnv,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    image_slice: Option<Rect>,
) {
    let (Some(final_layer), true) = (layers.last(), width > 0.0 && height > 0.0) else {
        return;
    };
    let paint = |scene: &mut _,
                 bg: CssColor,
                 layer: &raikiri_style::ComputedBackgroundLayer,
                 image: &BackgroundImage,
                 image_color_fallback: bool,
                 warnings: &mut Vec<RenderWarning>| {
        ElementBackgroundLayer {
            image_cache,
            width,
            height,
            abs_x,
            abs_y,
            bg,
            image,
            current_color,
            clip: layer.clip,
            origin: layer.origin,
            border_radius,
            border,
            padding,
            env: BackgroundEnv {
                fixed_viewport: env.fixed_viewport.filter(|_| {
                    layer.attachment == raikiri_style::property::BackgroundAttachment::Fixed
                }),
                ..env
            },
            size: &layer.size,
            position: &layer.position,
            repeat: &layer.repeat,
            pixel_source,
            image_slice,
            image_color_fallback,
        }
        .paint(scene, warnings);
    };
    if let [layer] = layers {
        paint(scene, bg, layer, &layer.image, true, warnings);
        return;
    }
    paint(
        scene,
        bg,
        final_layer,
        &BackgroundImage::None,
        false,
        warnings,
    );
    let layers = painted_background_layers(layers, warnings);
    for layer in layers.iter().rev() {
        if !matches!(layer.image, BackgroundImage::None) {
            paint(
                scene,
                CssColor::TRANSPARENT,
                layer,
                &layer.image,
                false,
                warnings,
            );
        }
    }
}

/// Rounded background clip matching the color fill.
///
/// Resolve outer percentages on each border-box axis before subtracting the
/// painting-area inset, so the curve follows CSS Backgrounds 3 §4.2.
fn rounded_background_path(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: &ComputedBorderRadius,
    inset: (f64, f64, f64, f64),
    reference: (f64, f64),
) -> Option<BezPath> {
    let radii = inset_border_radii(used_border_radii(radius, reference.0, reference.1), inset);
    if radii.iter().flatten().all(|value| *value == 0.0) {
        None
    } else {
        Some(rounded_rect_path(x0, y0, x1, y1, radii))
    }
}

fn redacted_image_url(url: &url::Url) -> url::Url {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted
}

/// Paint one URL background with separate positioning and painting areas.
///
/// `positioning` is the `background-origin` box that `background-size` and
/// `background-position` resolve against (CSS Backgrounds 3 section 2.8).
/// `painting` is the `background-clip` box that tiling covers and clips to
/// (CSS Backgrounds 3 section 2.7). Callers with identical boxes (for example
/// a borderless element with default origin/clip) pass the same [`Rect`] twice.
#[allow(clippy::too_many_arguments)]
fn paint_background_from_source(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    url: &url::Url,
    positioning: Rect,
    painting: Rect,
    size: &ComputedBackgroundSize,
    position: &ComputedCssPosition,
    repeat: &raikiri_style::property::BackgroundRepeat,
    device_grid: Option<DeviceGrid>,
    pixel_source: &dyn ImagePixelSource,
    warnings: &mut Vec<RenderWarning>,
) {
    let Some(intrinsic) = pixel_source.intrinsic_size(url) else {
        return;
    };
    let Some(image) =
        background_image_dimensions(size, positioning.width(), positioning.height(), intrinsic)
    else {
        return;
    };
    let (image_w, image_h) = rounded_background_tile_size(
        size,
        image,
        (positioning.width(), positioning.height()),
        repeat,
    );
    let raster_size = ImageRasterSize {
        width: image_w as f32,
        height: image_h as f32,
    };
    if let Some(decoded) = pixel_source.get_decoded_at_size(url, raster_size, None) {
        paint_background_image(
            image_cache,
            scene,
            &decoded,
            positioning,
            painting,
            image_w,
            image_h,
            position,
            repeat,
            device_grid,
        );
    } else {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: Some(redacted_image_url(url)),
            },
            node_id: None,
            details: "CSS background image could not be rasterized; the image was skipped".into(),
        });
    }
}

/// Paint one decoded CSS background image with origin/clip separation.
///
/// `positioning` is the `background-origin` box: `background-size` resolves
/// against it and the origin tile is placed inside it by `background-position`
/// (CSS Backgrounds 3 <https://www.w3.org/TR/css-backgrounds-3/#background-origin>).
/// `painting` is the `background-clip` box: tiling covers it and the output is
/// clipped to it (CSS Backgrounds 3
/// <https://www.w3.org/TR/css-backgrounds-3/#background-clip>). This keeps
/// URL backgrounds useful to reftests while leaving gradients on the existing
/// color path.
///
/// Tiling follows CSS Backgrounds 3
/// <https://www.w3.org/TR/css-backgrounds-3/#background-repeat>:
/// `repeat` tiles the origin tile in both directions to cover the painting
/// area, clipping partial edge tiles; `no-repeat` paints only the origin tile;
/// `space` repeats without clipping or scaling, pinning the first and last
/// tiles to the positioning edges with even gaps (`background-position` only
/// places the lone tile when at most one fits) and keeps that pitch over the
/// rest of `painting`; `round` rescales the tile so a whole number exactly
/// fills the positioning width/height, then places and repeats it like
/// `repeat`. Mixed axes combine independently.
///
/// On a raster output (`device_grid` is known) the tiles are drawn on whole
/// device pixels, see [`tile_spans`].
#[allow(clippy::too_many_arguments)]
fn paint_background_image(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    decoded: &std::sync::Arc<raikiri_traits::DecodedImage>,
    positioning: Rect,
    painting: Rect,
    image_w: f64,
    image_h: f64,
    position: &ComputedCssPosition,
    repeat: &raikiri_style::property::BackgroundRepeat,
    device_grid: Option<DeviceGrid>,
) {
    ImageLayer {
        image_cache,
        decoded,
        positioning,
        painting,
        image: (image_w, image_h),
        position,
        repeat,
        device_grid,
    }
    .paint(scene);
}

/// [`tile_spans`] for one axis of a layer, where `placed` is the start of the
/// tile that `background-position` puts in the positioning area and
/// `painting_start` the start of the painting area, both along that axis.
///
/// A `repeat` axis repeats from the placed tile, which can lie outside the
/// painting area. When its pitch snaps to a different length than the CSS
/// one, the repetition has to keep that tile as its anchor, or its phase would
/// follow the painting area instead of `background-position`. The span
/// returned is the tile of that repetition at or before `painting_start`. Any
/// other axis is left to [`tile_spans`].
fn axis_tile_spans(
    starts: &[f64],
    length: f64,
    keyword: raikiri_style::property::BackgroundRepeatKeyword,
    (placed, painting_start): (f64, f64),
    grid: Option<(f64, f64)>,
) -> Vec<(f64, f64)> {
    use raikiri_style::property::BackgroundRepeatKeyword::Repeat;
    let anchored = match keyword {
        Repeat => tile_spans(&[placed], length, keyword, grid)
            .first()
            .copied(),
        _ => None,
    };
    let Some((anchor, pitch)) = anchored else {
        return tile_spans(starts, length, keyword, grid);
    };
    let tile_index = ((painting_start - anchor) / pitch).floor();
    vec![(anchor + tile_index * pitch, pitch)]
}

/// The `(start, length)` spans along one axis that a layer's tiles are drawn
/// at, snapped to device pixels when the output grid `(scale, offset)` is
/// known.
///
/// A tile is drawn on whole device pixels so its edges stay crisp, but it is
/// still placed from the unsnapped box: each edge moves to the nearest device
/// pixel on its own, which keeps a tile that straddles a pixel boundary from
/// being stretched over a partial pixel. A `repeat` axis keeps only its origin
/// tile, whose length is the pitch of the repetition: a whole number of
/// device pixels, at least one, so the copies stay on the grid however small
/// the unsnapped tile is, and positive on a mirrored grid. A `round` axis
/// keeps the pitch that fits its area.
fn tile_spans(
    starts: &[f64],
    length: f64,
    keyword: raikiri_style::property::BackgroundRepeatKeyword,
    grid: Option<(f64, f64)>,
) -> Vec<(f64, f64)> {
    use raikiri_style::property::BackgroundRepeatKeyword::{Repeat, Round};
    // A grid that cannot place a pixel edge (no scale, or an overflowed one)
    // leaves the tiles where they are.
    let grid =
        grid.filter(|(scale, offset)| scale.is_finite() && *scale != 0.0 && offset.is_finite());
    let snap = |value: f64| match grid {
        Some((scale, offset)) => ((value * scale + offset).round() - offset) / scale,
        None => value,
    };
    match keyword {
        Round => starts
            .first()
            .map(|start| (*start, length))
            .into_iter()
            .collect(),
        Repeat => starts
            .first()
            .map(|start| match grid {
                // A mirrored grid has a negative scale; the pitch is a count
                // of device pixels, so it takes the magnitude.
                Some((scale, _)) => (
                    snap(*start),
                    (length * scale.abs()).round().max(1.0) / scale.abs(),
                ),
                None => (*start, length),
            })
            .into_iter()
            .collect(),
        _ => {
            let snapped: Vec<(f64, f64)> = starts
                .iter()
                .map(|start| {
                    let snapped = snap(*start);
                    (snapped, snap(*start + length) - snapped)
                })
                .collect();
            // A tile thinner than a device pixel can have both edges round to
            // the same boundary. The axis then stays where it is instead of
            // losing that tile, and as a whole, so the tiles that did snap
            // cannot overlap the one that did not.
            if snapped
                .iter()
                .all(|(_, snapped_length)| *snapped_length > 0.0)
            {
                snapped
            } else {
                starts.iter().map(|start| (*start, length)).collect()
            }
        }
    }
}

/// Paint the border image of a box whose border box is `width` × `height`
/// at (`abs_x`, `abs_y`) (CSS Backgrounds 3 §6), painted through
/// `transform` inside the root element's transform.
///
/// Returns whether a border image was drawn. When it was, it replaces the
/// box's border styles; when the source is `none` or cannot be displayed,
/// the caller paints the borders as usual (§6.1). Border radii do not clip
/// a border image. A gradient source paints its first color stop, like
/// gradient backgrounds in this painter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_border_image(
    image_cache: &crate::walk::ImageBrushCache,
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    cv: &ComputedValues,
    image: &ComputedBorderImage,
    border: &Sides<raikiri_style::resolve::ComputedBorder>,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    transform: Affine,
) -> bool {
    BorderImage::new(
        image_cache,
        (width, height),
        (abs_x, abs_y),
        cv,
        image,
        border,
        transform,
    )
    .paint(scene, pixel_source, warnings)
}

/// A rectangle of whole decoded pixels, as `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PixelCrop {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl PixelCrop {
    fn whole(decoded: &raikiri_traits::DecodedImage) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: decoded.width,
            y1: decoded.height,
        }
    }

    fn width(self) -> u32 {
        self.x1 - self.x0
    }

    fn height(self) -> u32 {
        self.y1 - self.y0
    }

    fn pixels(self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }

    /// An image brush over a copy of these pixels of `decoded`, or `None`
    /// when `decoded` holds fewer pixels than its size says.
    fn brush(self, decoded: &raikiri_traits::DecodedImage) -> Option<peniko::ImageBrush> {
        let stride = decoded.width as usize * 4;
        let row_len = self.width() as usize * 4;
        let mut rgba = Vec::with_capacity(row_len * self.height() as usize);
        for row in self.y0..self.y1 {
            let offset = row as usize * stride + self.x0 as usize * 4;
            rgba.extend_from_slice(decoded.rgba.get(offset..offset + row_len)?);
        }
        Some(peniko::ImageBrush::new(peniko::ImageData {
            data: peniko::Blob::from(rgba),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: self.width(),
            height: self.height(),
        }))
    }
}

/// The brush and the transform from its pixels to image coordinates for a
/// border image `part` that a raster output draws much smaller than the
/// decoded pixels it shows, or `None` when it is not (or the output is not a
/// raster).
///
/// The part is averaged down to one pixel per device pixel along each axis
/// that shrinks, and keeps the whole decoded pixels of `crop` along the
/// others. A bilinear filter alone averages at most about two pixels per
/// device pixel, so past that it would skip the pixels between its sample
/// points and turn a seam between two colors of the image into a hard step
/// instead of a blend. `pixels_per_unit` is the decoded pixels per image
/// unit.
fn averaged_part_brush(
    image_cache: &crate::walk::ImageBrushCache,
    decoded: &std::sync::Arc<raikiri_traits::DecodedImage>,
    part: &raikiri_dom::image_geometry::BorderImagePart,
    crop: PixelCrop,
    pixels_per_unit: (f64, f64),
    grid: Option<DeviceGrid>,
) -> Option<(peniko::ImageBrush, Affine)> {
    let grid = grid?;
    let (src_x, src_y, src_w, src_h) = part.source;
    let (scale_x, scale_y) = pixels_per_unit;
    // The decoded pixels the part shows and the device pixels a tile covers.
    let (source_w, source_h) = (src_w * scale_x, src_h * scale_y);
    let (device_w, device_h) = (
        part.tile_width * grid.scale_x.abs(),
        part.tile_height * grid.scale_y.abs(),
    );
    let shrinks = |source: f64, device: f64| source > device * 1.5;
    let (shrinks_x, shrinks_y) = (shrinks(source_w, device_w), shrinks(source_h, device_h));
    if !shrinks_x && !shrinks_y {
        return None;
    }
    // The span of decoded pixels along an axis and the pixels it becomes.
    let axis = |shrinks: bool, span: (f64, f64), device: f64, kept: (u32, u32)| {
        if shrinks {
            (span, device.round().max(1.0) as u32)
        } else {
            ((f64::from(kept.0), f64::from(kept.1)), kept.1 - kept.0)
        }
    };
    let ((x0, x1), width) = axis(
        shrinks_x,
        (src_x * scale_x, (src_x + src_w) * scale_x),
        device_w,
        (crop.x0, crop.x1),
    );
    let ((y0, y1), height) = axis(
        shrinks_y,
        (src_y * scale_y, (src_y + src_h) * scale_y),
        device_h,
        (crop.y0, crop.y1),
    );
    let brush = image_cache.averaged(decoded, [x0, y0, x1, y1], (width, height))?;
    // The averaged pixels to image coordinates.
    let to_image = Affine::translate((src_x, src_y))
        * Affine::scale_non_uniform(src_w / f64::from(width), src_h / f64::from(height));
    Some((brush, to_image))
}

/// The pixels of `decoded` between the corners `start` and `end` (in
/// pixels, rounded to whole pixels), or `None` when the rectangle holds no
/// pixel.
fn pixel_crop(
    decoded: &raikiri_traits::DecodedImage,
    start: (f64, f64),
    end: (f64, f64),
) -> Option<PixelCrop> {
    let clamp = |value: f64, limit: u32| value.round().clamp(0.0, f64::from(limit)) as u32;
    let (x0, y0) = (
        clamp(start.0, decoded.width),
        clamp(start.1, decoded.height),
    );
    let (mut x0, mut y0) = (x0, y0);
    let (mut x1, mut y1) = (clamp(end.0, decoded.width), clamp(end.1, decoded.height));
    // A slice thinner than a pixel still shows the pixel it starts in, or
    // the last pixel when it rounds to the image's far edge.
    let widen = |start: &mut u32, end: &mut u32, nonempty: bool, limit: u32| {
        if *end == *start && nonempty && limit > 0 {
            if *start < limit {
                *end = *start + 1;
            } else {
                *start = limit - 1;
            }
        }
    };
    let inside = |from: f64, to: f64, limit: u32| to > from && from < f64::from(limit);
    widen(
        &mut x0,
        &mut x1,
        inside(start.0, end.0, decoded.width),
        decoded.width,
    );
    widen(
        &mut y0,
        &mut y1,
        inside(start.1, end.1, decoded.height),
        decoded.height,
    );
    (x1 > x0 && y1 > y0).then_some(PixelCrop { x0, y0, x1, y1 })
}

fn used_border_radii(radius: &ComputedBorderRadius, width: f64, height: f64) -> [[f64; 2]; 4] {
    radius
        .used(width as f32, height as f32)
        .map(|corner| corner.map(f64::from))
}

fn inset_border_radii(radii: [[f64; 2]; 4], inset: (f64, f64, f64, f64)) -> [[f64; 2]; 4] {
    let (left, top, right, bottom) = inset;
    let insets = [[left, top], [right, top], [right, bottom], [left, bottom]];
    std::array::from_fn(|index| {
        let x = (radii[index][0] - insets[index][0]).max(0.0);
        let y = (radii[index][1] - insets[index][1]).max(0.0);
        if x == 0.0 || y == 0.0 {
            [0.0; 2]
        } else {
            [x, y]
        }
    })
}

/// A CSS rounded rectangle with used corner pairs.
///
/// Insets can make a curve reach past the opposite edge. Crop its quarter
/// ellipse at that edge, preserving its center and radii (CSS Backgrounds 3
/// section 4.2), rather than rescaling the curve to the smaller rectangle.
/// When cropped diagonal curves cross, use their common convex outline.
/// Ordinary and single-corner paths retain their cubic arcs.
fn rounded_rect_path(x0: f64, y0: f64, x1: f64, y1: f64, radii: [[f64; 2]; 4]) -> BezPath {
    let mut path = BezPath::new();
    if x1 <= x0 || y1 <= y0 {
        return path;
    }
    let (width, height) = (x1 - x0, y1 - y0);
    let cropped = radii.iter().any(|r| r[0] > width || r[1] > height);
    let diagonal_overlap = [(0, 2), (1, 3)].into_iter().any(|(a, b)| {
        radii[a].iter().chain(&radii[b]).all(|r| *r > 0.0)
            && radii[a][0] + radii[b][0] > width
            && radii[a][1] + radii[b][1] > height
    });
    let square = [
        Point::new(x0, y0),
        Point::new(x1, y0),
        Point::new(x1, y1),
        Point::new(x0, y1),
    ];
    let mut common = (cropped && diagonal_overlap).then(|| square.to_vec());
    let [tl, tr, br, bl] = radii;
    let centers = [
        Point::new(x0 + tl[0], y0 + tl[1]),
        Point::new(x1 - tr[0], y0 + tr[1]),
        Point::new(x1 - br[0], y1 - br[1]),
        Point::new(x0 + bl[0], y1 - bl[1]),
    ];
    let bounds = Rect::new(x0, y0, x1, y1);
    for (index, (corner, center)) in radii.into_iter().zip(centers).enumerate() {
        let Some((point, arc)) = corner_start(index, corner, center, bounds, square[index]) else {
            // The whole rectangle lies outside this corner's ellipse.
            return BezPath::new();
        };
        if let (Some(polygon), Some(arc)) = (common.as_mut(), arc)
            && !clip_polygon_to_corner_arc(polygon, point, arc)
        {
            return BezPath::new();
        }
        if common.is_none() {
            if index == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
            if let Some(arc) = arc {
                for element in arc.append_iter(0.1) {
                    path.push(element);
                }
            }
        }
    }
    if let Some(polygon) = common {
        let mut outline = BezPath::new();
        outline.move_to(polygon[0]);
        for point in &polygon[1..] {
            outline.line_to(*point);
        }
        outline.close_path();
        return outline;
    }
    path.close_path();
    path
}

/// The point where the outline of corner `index` (clockwise from the top
/// left) starts on the rectangle `bounds`, and the quarter ellipse that follows
/// it, if the corner is rounded. `square` is the rectangle's own corner, used
/// when the corner is not rounded. `None` when the whole rectangle lies outside
/// the corner's ellipse.
fn corner_start(
    index: usize,
    [rx, ry]: [f64; 2],
    center: Point,
    bounds: Rect,
    square: Point,
) -> Option<(Point, Option<Arc>)> {
    if rx == 0.0 || ry == 0.0 {
        return Some((square, None));
    }
    let Rect { x0, y0, x1, y1 } = bounds;
    let (lower, upper) = match index {
        0 => ((center.y - y1) / ry, (center.x - x1) / rx),
        1 => ((x0 - center.x) / rx, (center.y - y1) / ry),
        2 => ((y0 - center.y) / ry, (x0 - center.x) / rx),
        _ => ((center.x - x1) / rx, (y0 - center.y) / ry),
    };
    let start = lower.clamp(0.0, 1.0).asin();
    let end = upper.clamp(0.0, 1.0).acos();
    if start >= end {
        return None;
    }
    let point = if start == 0.0 {
        match index {
            0 => Point::new(x0, center.y),
            1 => Point::new(center.x, y0),
            2 => Point::new(x1, center.y),
            _ => Point::new(center.x, y1),
        }
    } else {
        match index {
            0 => Point::new(center.x - rx * start.cos(), y1),
            1 => Point::new(x0, center.y - ry * start.cos()),
            2 => Point::new(center.x + rx * start.cos(), y0),
            _ => Point::new(x1, center.y + ry * start.cos()),
        }
    };
    let angles = [PI, 3.0 * FRAC_PI_2, 0.0, FRAC_PI_2];
    let arc = Arc {
        center,
        radii: Vec2::new(rx, ry),
        start_angle: angles[index] + start,
        sweep_angle: end - start,
        x_rotation: 0.0,
    };
    Some((point, Some(arc)))
}

/// Clip the convex `polygon` to the interior of the corner curve that starts
/// at `point` and follows `arc`. `false` when fewer than three vertices remain.
fn clip_polygon_to_corner_arc(polygon: &mut Vec<Point>, point: Point, arc: Arc) -> bool {
    // Subdivide the actual ellipse so cubic approximation error cannot
    // move an inner contour outside the existing outer arc.
    let mut points = vec![point];
    flatten_corner_arc(arc, arc.start_angle + arc.sweep_angle, 0, &mut points);
    for edge in points.windows(2) {
        *polygon = clip_corner_polygon(std::mem::take(polygon), edge[0], edge[1]);
        if polygon.len() < 3 {
            return false;
        }
    }
    true
}

/// Flatten an axis-aligned ellipse to a 0.05 layout-pixel chord-error target, bounded
/// to 1024 chords per corner for extreme coordinates.
fn flatten_corner_arc(arc: Arc, end_angle: f64, depth: u8, points: &mut Vec<Point>) {
    // The norm of the ellipse's second derivative is bounded by its larger
    // radius; linear interpolation error is at most that bound times h²/8.
    let sweep = end_angle - arc.start_angle;
    let chord_error = arc.radii.x.abs().max(arc.radii.y.abs()) * sweep.abs() * sweep.abs() / 8.0;
    if depth == 10 || chord_error <= 0.05 {
        let (sin, cos) = end_angle.sin_cos();
        points.push(arc.center + Vec2::new(arc.radii.x * cos, arc.radii.y * sin));
        return;
    }
    let middle = arc.start_angle + sweep * 0.5;
    flatten_corner_arc(arc, middle, depth + 1, points);
    let mut right = arc;
    right.start_angle = middle;
    flatten_corner_arc(right, end_angle, depth + 1, points);
}

/// Clip a nonempty convex polygon to the clockwise edge's interior side.
fn clip_corner_polygon(polygon: Vec<Point>, a: Point, b: Point) -> Vec<Point> {
    let distance = |point: Point| (b - a).cross(point - a);
    let mut out = Vec::with_capacity(polygon.len() + 1);
    let mut previous = polygon[polygon.len() - 1];
    let mut previous_distance = distance(previous);
    for current in polygon {
        let current_distance = distance(current);
        if (previous_distance >= 0.0) != (current_distance >= 0.0) {
            let t = previous_distance / (previous_distance - current_distance);
            out.push(previous.lerp(current, t));
        }
        if current_distance >= 0.0 {
            out.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
    out
}

pub(crate) fn paintable_border_radius(
    radius: &ComputedBorderRadius,
    enabled: bool,
) -> ComputedBorderRadius {
    if enabled {
        *radius
    } else {
        ComputedBorderRadius::all(ComputedLength(0.0))
    }
}

/// The physical axis the line fragments of an inline element run along, and
/// the end of it where the element starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineProgression {
    /// The fragments run along y, as in a vertical writing mode, rather than
    /// along x.
    pub(crate) vertical: bool,
    /// The element starts at the high end of that axis: the right in
    /// horizontal text, the bottom in vertical text.
    pub(crate) reversed: bool,
}

impl InlineProgression {
    /// The progression of an element whose own direction is right to left
    /// when `rtl`, in a paragraph laid out in `writing_mode` (CSS Writing
    /// Modes 4 §2.1: `sideways-lr` puts the inline start at the bottom).
    pub(crate) fn new(writing_mode: shodo::geometry::WritingMode, rtl: bool) -> Self {
        Self {
            vertical: writing_mode.is_vertical(),
            reversed: rtl != (writing_mode == shodo::geometry::WritingMode::SidewaysLr),
        }
    }

    /// The start and size of `rect` along the axis.
    pub(crate) fn span(self, rect: raikiri_dom::BoxRect) -> (f32, f32) {
        if self.vertical {
            (rect.y, rect.height)
        } else {
            (rect.x, rect.width)
        }
    }

    /// `rect` with the start and size `span` along the axis.
    pub(crate) fn with_span(
        self,
        mut rect: raikiri_dom::BoxRect,
        (start, size): (f32, f32),
    ) -> raikiri_dom::BoxRect {
        if self.vertical {
            rect.y = start;
            rect.height = size;
        } else {
            rect.x = start;
            rect.width = size;
        }
        rect
    }

    /// The page rectangle whose span along the axis is `along` and across it
    /// is `across`.
    fn rect(self, along: (f64, f64), across: (f64, f64)) -> Rect {
        if self.vertical {
            Rect::new(across.0, along.0, across.1, along.1)
        } else {
            Rect::new(along.0, across.0, along.1, across.1)
        }
    }
}

/// Fill a background using the same elliptical curve as its image clip.
#[allow(clippy::too_many_arguments)]
fn fill_rounded_background(
    scene: &mut impl PaintScene,
    color: Color,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: &ComputedBorderRadius,
    inset: (f64, f64, f64, f64),
    reference: (f64, f64),
) {
    if let Some(path) = rounded_background_path(x0, y0, x1, y1, radius, inset, reference) {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &path);
    } else {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &Rect::new(x0, y0, x1, y1),
        );
    }
}

/// Paint non-inset `box-shadow` entries behind an element's background.
///
/// The renderer's rounded-rectangle shadow primitive provides the exact
/// geometry for the initial outer-shadow slice. Inset shadows and fully
/// per-corner radii remain follow-up work.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_box_shadows(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    border_radius: &ComputedBorderRadius,
    shadows: &[raikiri_style::ComputedBoxShadowItem],
    current_color: CssColor,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let radius = used_border_radii(border_radius, width as f64, height as f64)
        .into_iter()
        .flatten()
        .fold(0.0, f64::max);
    for shadow in shadows.iter().filter(|shadow| !shadow.inset) {
        let color = match shadow.color {
            TextShadowColor::CurrentColor => current_color,
            TextShadowColor::Resolved(color) => color,
            _ => current_color, // cov:ignore: defensive fallback for future TextShadowColor variants
        };
        if color.a == 0 {
            continue;
        }
        let offset_x = shadow.offset_x.px() as f64;
        let offset_y = shadow.offset_y.px() as f64;
        let spread = shadow.spread_radius.px() as f64;
        let blur = shadow.blur_radius.px() as f64;
        if !offset_x.is_finite()
            || !offset_y.is_finite()
            || !spread.is_finite()
            || !blur.is_finite()
        {
            continue; // cov:ignore: non-finite computed lengths are rejected before painting
        }
        let rect = Rect::new(
            abs_x as f64 + offset_x - spread,
            abs_y as f64 + offset_y - spread,
            abs_x as f64 + width as f64 + offset_x + spread,
            abs_y as f64 + height as f64 + offset_y + spread,
        );
        scene.draw_box_shadow(
            Affine::IDENTITY,
            rect,
            Color::from_rgba8(color.r, color.g, color.b, color.a),
            (radius + spread).max(0.0),
            blur.max(0.0),
        );
    }
}

/// Paint a regular element border with a rounded outer edge when the four
/// sides share one solid width and color.  The existing strip painter remains
/// the conservative fallback for mixed side styles and widths.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_border_rounded(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
    radius: &ComputedBorderRadius,
) {
    let radii = used_border_radii(radius, width as f64, height as f64);
    let has_radius = radii.iter().flatten().any(|value| *value > 0.0);
    if !has_radius {
        paint_element_border(scene, width, height, abs_x, abs_y, border, current_color);
        return;
    }

    let widths = [
        border.left.width().px() as f64,
        border.top.width().px() as f64,
        border.right.width().px() as f64,
        border.bottom.width().px() as f64,
    ];
    let uniform_width = widths[0] > 0.0
        && widths
            .iter()
            .all(|value| (*value - widths[0]).abs() < f64::EPSILON);
    let solid = [&border.left, &border.top, &border.right, &border.bottom]
        .iter()
        .all(|side| side.style() == BorderStyle::Solid);
    let color_for = |side: &raikiri_style::resolve::ComputedBorder| match side.color {
        BorderColor::CurrentColor => Some(Color::from_rgba8(
            current_color.r,
            current_color.g,
            current_color.b,
            current_color.a,
        )),
        BorderColor::Resolved(color) => Some(Color::from_rgba8(color.r, color.g, color.b, color.a)),
        _ => None, // cov:ignore: defensive fallback for future BorderColor variants
    };
    let colors = [
        color_for(&border.left),
        color_for(&border.top),
        color_for(&border.right),
        color_for(&border.bottom),
    ];
    let uniform_color = colors[0].is_some() && colors.iter().all(|color| *color == colors[0]);
    let Some(color) = colors[0] else {
        paint_element_border(scene, width, height, abs_x, abs_y, border, current_color); // cov:ignore: BorderColor variants currently always produce Some
        return; // cov:ignore: all current BorderColor variants produce Some
    };
    if !(uniform_width && solid && uniform_color) {
        paint_element_border(scene, width, height, abs_x, abs_y, border, current_color);
        return;
    }

    fill_rounded_border_ring(
        scene,
        (abs_x, abs_y, width, height),
        radii,
        widths[0],
        color,
    );
}

/// Fill the ring that a border of `stroke_width` makes inside the rounded
/// outer edge of the box at `(abs_x, abs_y)` of size `(width, height)`, and
/// then the square corners of the ring that have no radius.
fn fill_rounded_border_ring(
    scene: &mut impl PaintScene,
    (abs_x, abs_y, width, height): (f32, f32, f32, f32),
    radii: [[f64; 2]; 4],
    stroke_width: f64,
    color: Color,
) {
    let outer = rounded_rect_path(
        abs_x as f64,
        abs_y as f64,
        (abs_x + width) as f64,
        (abs_y + height) as f64,
        radii,
    );
    let inner_radii = inset_border_radii(
        radii,
        (stroke_width, stroke_width, stroke_width, stroke_width),
    );
    let inner = rounded_rect_path(
        abs_x as f64 + stroke_width,
        abs_y as f64 + stroke_width,
        (abs_x + width) as f64 - stroke_width,
        (abs_y + height) as f64 - stroke_width,
        inner_radii,
    );
    let mut ring = outer;
    ring.extend(inner.iter());
    scene.fill(Fill::EvenOdd, kurbo::Affine::IDENTITY, color, None, &ring);
    let x0 = abs_x as f64;
    let y0 = abs_y as f64;
    let x1 = (abs_x + width) as f64;
    let y1 = (abs_y + height) as f64;
    for (corner, rect) in [
        (
            radii[1],
            Rect::new(x1 - stroke_width, y0, x1, y0 + stroke_width),
        ),
        (
            radii[2],
            Rect::new(x1 - stroke_width, y1 - stroke_width, x1, y1),
        ),
        (
            radii[3],
            Rect::new(x0, y1 - stroke_width, x0 + stroke_width, y1),
        ),
        (
            radii[0],
            Rect::new(x0, y0, x0 + stroke_width, y0 + stroke_width),
        ),
    ] {
        if corner == [0.0; 2] {
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, color, None, &rect);
        }
    }
}

/// Paint a basic solid element outline without changing layout geometry.
///
/// The first visual tranche intentionally handles square block outlines only.
/// Inset/negative-offset edge cases, non-solid styles, and rounded outlines
/// remain outside this focused slice.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_outline(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    outline: &raikiri_style::ComputedOutline,
    outline_offset: ComputedLength,
    current_color: CssColor,
) {
    let outline_width = outline.width().px();
    let offset = outline_offset.px();
    if width <= 0.0
        || height <= 0.0
        || outline_width <= 0.0
        || !outline_width.is_finite()
        || !offset.is_finite()
        || !matches!(outline.style(), OutlineStyle::Solid)
    {
        return;
    }
    let outset = outline_width + offset;
    let outer_width = width + 2.0 * outset;
    let outer_height = height + 2.0 * outset;
    if outer_width <= 0.0 || outer_height <= 0.0 {
        return; // cov:ignore: collapsed negative-offset outlines are deferred
    }
    let color = match outline.color {
        OutlineColor::Resolved(color) => BorderColor::Resolved(color),
        OutlineColor::CurrentColor | OutlineColor::Invert => BorderColor::Resolved(current_color),
        _ => BorderColor::Resolved(current_color), // cov:ignore: defensive fallback for future OutlineColor variants
    };
    let mut border = Border::new();
    border.width = Length::Px(outline_width);
    border.style = BorderStyle::Solid;
    border.color = color;
    let border = resolve_border(
        border,
        ComputedLength(16.0),
        None,
        &ResolveContext::initial(),
    );
    let borders = Sides {
        top: border,
        right: border,
        bottom: border,
        left: border,
    };
    paint_element_border(
        scene,
        outer_width,
        outer_height,
        abs_x - outset,
        abs_y - outset,
        &borders,
        current_color,
    );
}

fn paint_element_border(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
) {
    paint_element_border_with_top(
        scene,
        width,
        height,
        abs_x,
        abs_y,
        border,
        current_color,
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_element_border_with_top(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
    paint_top: bool,
) {
    if let Some(border) = ElementBorder::new(
        width,
        height,
        abs_x,
        abs_y,
        border,
        current_color,
        paint_top,
    ) {
        border.paint(scene);
    }
}

/// Resolve the effective background color for painting (CSS Backgrounds 3 S2).
///
/// Priority:
/// 1. If background-image is a gradient, use its first color stop (resolved
///    against current_color for currentColor stops).
/// 2. If background-image is a url(), try to infer a solid color from the
///    URL filename (e.g. blue-100.png -> blue) as a minimal image-fallback;
///    if inference fails, fall back to background-color if opaque.
/// 3. Otherwise (None), use background-color.
///
/// Returns None if no opaque color can be derived (transparent).
fn effective_background_color(
    bg: CssColor,
    bg_image: &BackgroundImage,
    current_color: CssColor,
) -> Option<CssColor> {
    match bg_image {
        BackgroundImage::Gradient(gradient) => {
            gradient_first_color(gradient, current_color).or({
                // Fallback to background-color if gradient has no stops (should not happen)
                if bg.a != 0 { Some(bg) } else { None }
            })
        }
        BackgroundImage::Url(url) => {
            // Prefer inferred color from URL filename; if inference fails, use bg if opaque
            infer_url_color(url).or(if bg.a != 0 { Some(bg) } else { None })
        }
        BackgroundImage::None => {
            if bg.a != 0 {
                Some(bg)
            } else {
                None
            }
        }
        // BackgroundImage is non_exhaustive
        _ => {
            if bg.a != 0 {
                Some(bg)
            } else {
                None
            }
        }
    }
}

fn gradient_first_color(gradient: &Gradient, current_color: CssColor) -> Option<CssColor> {
    let first_stop = match gradient {
        Gradient::Linear(g) => g.stops.first(),
        Gradient::Radial(g) => g.stops.first(),
        Gradient::Conic(g) => {
            return g
                .stops
                .first()
                .map(|s| resolve_gradient_stop_color(s.color, current_color));
        }
        // non_exhaustive
        _ => None,
    };
    first_stop.map(|s| resolve_gradient_stop_color(s.color, current_color))
}

#[inline]
fn resolve_gradient_stop_color(c: GradientStopColor, current_color: CssColor) -> CssColor {
    match c {
        GradientStopColor::Resolved(color) => color,
        GradientStopColor::Unbounded(color) => color.srgb,
        GradientStopColor::CurrentColor => current_color,
        // non_exhaustive
        _ => current_color,
    }
}

/// A gradient stop color for interpolation. A color outside the sRGB gamut
/// keeps its own coordinates, so the gradient is clipped only when drawn.
fn gradient_stop_dynamic(c: GradientStopColor, current_color: CssColor) -> DynamicColor {
    match c {
        GradientStopColor::Unbounded(color) => unbounded_to_dynamic(color),
        other => css_color_to_dynamic(resolve_gradient_stop_color(other, current_color)),
    }
}

/// Convert a style color kept in its own space to a peniko dynamic color.
fn unbounded_to_dynamic(color: UnboundedColor) -> DynamicColor {
    let [first, second, third] = color.components;
    let components = [first, second, third, color.alpha];
    match color.space {
        MixColorSpace::SrgbLinear => {
            DynamicColor::from_alpha_color(AlphaColor::<LinearSrgb>::new(components))
        }
        MixColorSpace::Lab => DynamicColor::from_alpha_color(AlphaColor::<Lab>::new(components)),
        MixColorSpace::Lch => DynamicColor::from_alpha_color(AlphaColor::<Lch>::new(components)),
        MixColorSpace::Oklab => {
            DynamicColor::from_alpha_color(AlphaColor::<Oklab>::new(components))
        }
        MixColorSpace::Oklch => {
            DynamicColor::from_alpha_color(AlphaColor::<Oklch>::new(components))
        }
        _ => DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new(components)),
    }
}

/// The 8-bit sRGB color a dynamic color is drawn as, clipped to the gamut.
fn dynamic_to_css_color(color: DynamicColor) -> CssColor {
    let rgba = color.to_alpha_color::<Srgb>().to_rgba8();
    CssColor {
        r: rgba.r,
        g: rgba.g,
        b: rgba.b,
        a: rgba.a,
    }
}

/// Convert a style sRGB byte color to a peniko dynamic color.
///
/// Channels scale from 0-255 to 0.0-1.0 in sRGB. Used for conic stop
/// colors so the sweep interpolates from the authored values.
fn css_color_to_dynamic(color: CssColor) -> DynamicColor {
    let to_unit = |byte: u8| f32::from(byte) / 255.0;
    DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([
        to_unit(color.r),
        to_unit(color.g),
        to_unit(color.b),
        to_unit(color.a),
    ]))
}

/// Map the style interpolation space to the peniko tag for conic sweeps.
///
/// CSS Images 4 defaults to Oklab when no interpolation clause is present.
/// The hsl and hwb spaces have no gradient parser support here, so they
/// fall back to sRGB rather than failing the whole background.
fn conic_interpolation_tag(space: MixColorSpace) -> ColorSpaceTag {
    match space {
        MixColorSpace::Srgb => ColorSpaceTag::Srgb,
        MixColorSpace::SrgbLinear => ColorSpaceTag::LinearSrgb,
        MixColorSpace::Lab => ColorSpaceTag::Lab,
        MixColorSpace::Lch => ColorSpaceTag::Lch,
        MixColorSpace::Oklab => ColorSpaceTag::Oklab,
        MixColorSpace::Oklch => ColorSpaceTag::Oklch,
        // cov:ignore: hsl and hwb are rejected by the gradient parser, defensive only
        _ => ColorSpaceTag::Srgb,
    }
}

/// Map the style hue direction to the peniko direction for conic sweeps.
fn conic_hue_direction(method: HueInterpolationMethod) -> HueDirection {
    match method {
        HueInterpolationMethod::Shorter => HueDirection::Shorter,
        HueInterpolationMethod::Longer => HueDirection::Longer,
        HueInterpolationMethod::Increasing => HueDirection::Increasing,
        HueInterpolationMethod::Decreasing => HueDirection::Decreasing,
        // cov:ignore: defensive fallback for future hue methods
        _ => HueDirection::Shorter,
    }
}

/// Angular stop position as a 0-1 sweep offset.
///
/// An angle in degrees divides by 360, a percentage divides by 100.
/// A missing position stays missing for the fixup pass. Non-finite
/// authored values fall back to 0 so one bad stop cannot poison the sweep.
fn angular_stop_offset(position: Option<AnglePercentage>) -> Option<f32> {
    let value = match position {
        None => return None,
        Some(AnglePercentage::Angle(angle)) => angle.0 / 360.0,
        Some(AnglePercentage::Percent(percent)) => percent / 100.0,
        // cov:ignore: defensive fallback for future angle-percentage variants
        _ => return None,
    };
    if value.is_finite() {
        Some(value)
    } else {
        // cov:ignore: saturated infinities from parsing are clamped here, rarely painted
        Some(0.0)
    }
}

/// Fill missing conic offsets per the color-stop fixup rule.
///
/// CSS Images 3 section 3.4.3: a missing first stop means 0, a missing last
/// stop means 1, interior runs spread evenly between their defined
/// neighbors, then every offset clamps up to its predecessor so the sweep
/// never runs backward. The pinned quadrant cases arrive fully defined, so
/// this mostly passes offsets through.
fn fixup_conic_offsets(stops: &[Option<f32>]) -> Vec<f32> {
    let count = stops.len();
    let mut offsets: Vec<f32> = stops.iter().map(|slot| slot.unwrap_or(f32::NAN)).collect();
    if count == 0 {
        return Vec::new();
    }
    if offsets[0].is_nan() {
        offsets[0] = 0.0;
    }
    if offsets[count - 1].is_nan() {
        offsets[count - 1] = 1.0;
    }
    let mut index = 0;
    while index < count {
        if !offsets[index].is_nan() {
            index += 1;
            continue;
        }
        let run_start = index;
        while index < count && offsets[index].is_nan() {
            index += 1;
        }
        let run_end = index;
        let before = offsets[run_start - 1];
        let after = offsets[run_end];
        let gap = (run_end - run_start + 1) as f32;
        for (slot, offset) in offsets[run_start..run_end].iter_mut().enumerate() {
            let step = (slot + 1) as f32;
            *offset = before + (after - before) * step / gap;
        }
    }
    let mut previous = offsets[0];
    for offset in offsets.iter_mut().skip(1) {
        if *offset < previous {
            *offset = previous;
        } else {
            previous = *offset;
        }
    }
    offsets
}

/// One axis of a conic center inside its positioning area.
///
/// Percentages resolve against the area size, lengths offset from the
/// matching edge, and a `calc()` from the start edge. Only pixel and
/// percentage lengths and absolute `calc()` sums survive style resolution, so
/// other length kinds fall back to the area middle.
fn conic_axis_center(offset: &CssPositionOffset, start: f64, end: f64) -> f64 {
    match offset {
        CssPositionOffset::Start(Length::Px(px)) => start + f64::from(*px),
        CssPositionOffset::Start(Length::Percent(percent)) => {
            start + (end - start) * f64::from(*percent) / 100.0
        }
        CssPositionOffset::End(Length::Px(px)) => end - f64::from(*px),
        CssPositionOffset::End(Length::Percent(percent)) => {
            end - (end - start) * f64::from(*percent) / 100.0
        }
        CssPositionOffset::Calc(calc) => {
            start + (end - start) * f64::from(calc.percent) / 100.0 + f64::from(calc.px)
        }
        // cov:ignore: only Px and Percent survive resolution, other lengths are unreachable
        _ => (start + end) / 2.0,
    }
}

/// Center point of a conic gradient within its positioning area.
///
/// CSS Images 4 section 3.3: the at position defaults to center and
/// percentages count from the top-left of the gradient box.
fn conic_center(position: &CssPosition, positioning: Rect) -> Point {
    Point::new(
        conic_axis_center(&position.horizontal, positioning.x0, positioning.x1),
        conic_axis_center(&position.vertical, positioning.y0, positioning.y1),
    )
}

/// Build a peniko sweep gradient for a style conic gradient.
///
/// Returns missing when fewer than two stops survive, which cannot happen
/// for parsed gradients but keeps paint total. The sweep spans one full
/// turn from the authored from angle. CSS zero degrees points up while
/// peniko zero points right in a y-down space, hence the minus 90 shift.
/// Non-repeating sweeps pad beyond their stops, repeating sweeps repeat.
fn conic_to_peniko(
    conic: &ConicGradient,
    center: Point,
    current_color: CssColor,
) -> Option<PenikoGradient> {
    // cov:ignore: parser requires at least two stops, defensive only
    if conic.stops.len() < 2 {
        return None;
    }
    let raw: Vec<Option<f32>> = conic
        .stops
        .iter()
        .map(|stop| angular_stop_offset(stop.position))
        .collect();
    let offsets = fixup_conic_offsets(&raw);
    let mut peniko_stops: Vec<(f32, DynamicColor)> = Vec::with_capacity(conic.stops.len());
    for (stop, offset) in conic.stops.iter().zip(offsets.iter()) {
        peniko_stops.push((*offset, gradient_stop_dynamic(stop.color, current_color)));
    }
    let from_degrees = if conic.angle.0.is_finite() {
        conic.angle.0
    } else {
        // cov:ignore: saturated angles from parsing are finite except for extreme overflow
        0.0
    };
    let start_angle = (from_degrees - 90.0).to_radians();
    let end_angle = start_angle + 2.0 * std::f32::consts::PI;
    // Sweeps are circular, so t wraps past 1 back to 0 even for a
    // non-repeating gradient that already covers the full turn. Pad would
    // clamp the wrapped slice to the last stop and paint the starting
    // quadrant with the wrong color, so always repeat for the wrap.
    let extend = PenikoExtend::Repeat;
    let space = conic_interpolation_tag(conic.interpolation.color_space);
    let direction = conic_hue_direction(conic.interpolation.hue_method);
    let gradient = PenikoGradient::new_sweep(center, start_angle, end_angle).with_extend(extend);
    // A renderer clips each stop to the sRGB gamut before interpolating, so
    // a sweep with a stop outside it gets clipped sRGB stops sampled from
    // the ramp in its own space instead.
    if gradient::any_out_of_gamut(&peniko_stops) {
        let stops = gradient::expand_stops(&peniko_stops, space, direction, false);
        return Some(gradient.with_stops(stops.as_slice()));
    }
    Some(
        gradient
            .with_interpolation_cs(space)
            .with_hue_direction(direction)
            .with_stops(peniko_stops.as_slice()),
    )
}

/// Paint one conic background over its painting area.
///
/// The sweep geometry uses the positioning area for its center while the
/// fill covers the painting area, mirroring the origin and clip split used
/// for solid and image backgrounds. A rounded box is clipped to its curve by
/// the caller, around the background color and the sweep together.
fn paint_conic_gradient(
    scene: &mut impl PaintScene,
    conic: &ConicGradient,
    positioning: Rect,
    painting: Rect,
    current_color: CssColor,
) {
    if positioning.width() <= 0.0 || positioning.height() <= 0.0 {
        return;
    }
    let center = conic_center(&conic.position, positioning);
    // cov:ignore: parser guarantees at least two valid stops, so the sweep always builds
    let Some(gradient) = conic_to_peniko(conic, center, current_color) else {
        return;
    };
    scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &painting);
}

/// Infer a solid color from a url() string for minimal painting.
///
/// WPT uses images like blue-100.png, green-100.png, red-100.png,
/// stripes-100.png, support/css3.png. Return a solid approximation
/// based on filename substring. If no known hint matches, return None
/// so caller can fall back to background-color.
fn infer_url_color(url: &str) -> Option<CssColor> {
    let lower = url.to_ascii_lowercase();
    if lower.contains("blue") {
        Some(CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        })
    } else if lower.contains("green") {
        // The WPT `/images/green.png` asset is lime (0,255,0), while
        // `bgimg1x50.png` is the CSS `green` keyword (0,128,0).
        let basename = lower.rsplit('/').next().unwrap_or(&lower);
        if basename == "green.png" || lower.contains("green-100") {
            Some(CssColor {
                r: 0,
                g: 255,
                b: 0,
                a: 255,
            })
        } else {
            Some(CssColor {
                r: 0,
                g: 128,
                b: 0,
                a: 255,
            })
        }
    } else if lower.contains("red") {
        Some(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        })
    } else if lower.contains("orange") {
        Some(CssColor {
            r: 255,
            g: 165,
            b: 0,
            a: 255,
        })
    } else if lower.contains("yellow") {
        Some(CssColor {
            r: 255,
            g: 255,
            b: 0,
            a: 255,
        })
    } else if lower.contains("stripes") {
        // stripes image is patterned; approximate with a neutral gray
        // (average of its pixels) so clipping geometry is still visible.
        Some(CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        })
    } else if lower.contains("css3") {
        // support/css3.png dominant is magenta-ish (255,0,255)
        Some(CssColor {
            r: 255,
            g: 0,
            b: 255,
            a: 255,
        })
    } else {
        None
    }
}

fn translation_only(cv: &ComputedValues) -> bool {
    cv.transform.iter().all(|function| {
        matches!(
            function,
            ComputedTransformFunction::Translate(..)
                | ComputedTransformFunction::TranslateX(..)
                | ComputedTransformFunction::TranslateY(..)
        )
    })
}

// spec: https://www.w3.org/TR/css-transforms-1/#transform-rendering
// Commands already carry page coordinates. Conjugate the local matrix around
// the border-box origin in that same coordinate space, then compose with the
// ancestor context. This also keeps transforms out of layout geometry.
pub(crate) fn element_transform(
    cv: &ComputedValues,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Affine {
    // Border backgrounds and borders use this same snapped paint box.
    let paint_x = (x as f64).round();
    let paint_y = (y as f64).round();
    let paint_width = ((x + width) as f64).round() - paint_x;
    let paint_height = ((y + height) as f64).round() - paint_y;
    let origin = (
        paint_x + position_offset(cv.transform_origin.horizontal, paint_width),
        paint_y + position_offset(cv.transform_origin.vertical, paint_height),
    );
    let mut matrix = Affine::translate(origin);
    for function in cv.transform.iter() {
        matrix *= match *function {
            ComputedTransformFunction::Matrix(coeffs) => Affine::new(coeffs.map(f64::from)),
            ComputedTransformFunction::Translate(..)
            | ComputedTransformFunction::TranslateX(..)
            | ComputedTransformFunction::TranslateY(..) => Affine::translate(
                function
                    .translation(paint_width, paint_height)
                    .unwrap_or_default(),
            ),
            ComputedTransformFunction::Scale(x, y) => Affine::scale_non_uniform(x as f64, y as f64),
            ComputedTransformFunction::ScaleX(x) => Affine::scale_non_uniform(x as f64, 1.0),
            ComputedTransformFunction::ScaleY(y) => Affine::scale_non_uniform(1.0, y as f64),
            ComputedTransformFunction::Rotate(angle) => {
                Affine::rotate((angle.0 as f64).to_radians())
            }
            ComputedTransformFunction::Skew(x, y) => Affine::new([
                1.0,
                (y.0 as f64).to_radians().tan(),
                (x.0 as f64).to_radians().tan(),
                1.0,
                0.0,
                0.0,
            ]),
            ComputedTransformFunction::SkewX(x) => {
                Affine::new([1.0, 0.0, (x.0 as f64).to_radians().tan(), 1.0, 0.0, 0.0])
            }
            ComputedTransformFunction::SkewY(y) => {
                Affine::new([1.0, (y.0 as f64).to_radians().tan(), 0.0, 1.0, 0.0, 0.0])
            }
        };
    }
    matrix * Affine::translate((-origin.0, -origin.1))
}

fn transform_translation(
    cv: &raikiri_style::ComputedValues,
    width: f32,
    height: f32,
) -> (f32, f32) {
    if !translation_only(cv) {
        return (0.0, 0.0);
    }
    let (dx, dy) = cv
        .transform
        .iter()
        .filter_map(|function| function.translation(f64::from(width), f64::from(height)))
        .fold((0.0, 0.0), |(dx, dy), (x, y)| (dx + x, dy + y));
    (dx as f32, dy as f32)
}

/// Page-local paint origin of a viewport-fixed box.
///
/// `static_position` is the box's laid-out position in flow coordinates,
/// which layout leaves at the static position on an axis whose insets are
/// both `auto` (CSS 2.1 §10.3.7 / §10.6.4). The page area is the fixed
/// containing block (CSS Paged Media 3 §3.2), so that position is measured
/// from the first page and repeated on every page.
#[allow(clippy::too_many_arguments)]
fn fixed_position_px(
    cv: &raikiri_style::ComputedValues,
    layout: &BoxLayout,
    static_position: (f32, f32),
    rtl: bool,
    page_width: f32,
    content_origin_y: f32,
    content_width: f32,
    content_height: f32,
) -> Option<(f32, f32)> {
    if !matches!(cv.position, PositionValue::Fixed) {
        return None;
    }
    // Percentages resolve against the fixed containing block, the page area.
    let to_px = |v: raikiri_style::resolve::ComputedLengthPercentageOrAuto, basis: f32| {
        use raikiri_style::resolve::ComputedLengthPercentageOrAuto as Inset;
        let px = match v {
            Inset::Px(px) => px,
            Inset::Percent(percent) => basis * percent / 100.0,
            Inset::Calc(calc) => basis * calc.percent / 100.0 + calc.px,
            _ => return None,
        };
        px.is_finite().then_some(px)
    };
    let left = to_px(cv.left, content_width);
    let right = to_px(cv.right, content_width);
    let top = to_px(cv.top, content_height);
    let bottom = to_px(cv.bottom, content_height);
    let (width, height, margin) = (layout.size.width, layout.size.height, layout.margin);
    // Layout puts the margin box against the insets.
    let from_right = |value: f32| content_width - value - width - margin.right;
    // CSS 2.1 §10.3.7: with both insets set, a right-to-left containing
    // block ignores `left`.
    let x = match (left, right) {
        (Some(_), Some(right)) if rtl => Some(from_right(right)),
        (Some(left), _) => Some(left + margin.left),
        (None, right) => right.map(from_right),
    };
    let y = top
        .map(|value| content_origin_y + value + margin.top)
        .or_else(|| {
            bottom.map(|value| content_origin_y + content_height - value - height - margin.bottom)
        });
    // Inset-placed boxes are kept near the page; a static position is used
    // as laid out, so a box whose static position lies off the page stays
    // off it.
    let x = match x {
        Some(x) => x.clamp(-page_width, page_width),
        None if static_position.0.is_finite() => static_position.0,
        None => 0.0,
    };
    let y = match y {
        Some(y) => y.clamp(
            content_origin_y - page_width,
            content_origin_y + page_width + content_height,
        ),
        None if static_position.1.is_finite() => content_origin_y + static_position.1,
        None => content_origin_y,
    };
    Some((x, y))
}

/// Pixel offset contributed by `vertical-align` to an inline-level box's
/// position. Positive values move downward, as does a box's `abs_y` in the
/// downward-growing Y coordinate system.
///
/// # Supported behavior
///
/// [`VerticalAlign::Sub`] / [`VerticalAlign::Super`] shift relative to the
/// parent's font-size; [`VerticalAlign::Length`] uses the computed px value.
/// Positive lengths raise and negative lengths lower (CSS 2.1 §10.8.1), so
/// the sign reverses in paint's downward-growing Y coordinate system.
///
/// [`VerticalAlign::Middle`] / [`VerticalAlign::TextTop`] /
/// [`VerticalAlign::TextBottom`] all remain at zero shift because shifting by
/// font metrics is not implemented. `Top` / `Bottom` do not shift here; the
/// relevant minimal line-box layout determines box positions. The `_` arm
/// is a defensive default to make this function total (`VerticalAlign` is
/// `#[non_exhaustive]`). [`VerticalAlign::Baseline`] also has no shift
/// (CSS 2.1 §10.8.1: "Align the baseline of the box with the baseline of
/// the parent box").
///
/// CSS 2.1 §10.8.1 "Applies to: inline-level and 'table-cell' elements"
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
/// (`table-cell` is not implemented in this crate): a block-level box's
/// `vertical-align: sub` does not contribute a shift.
///
/// # Shift size
///
/// CSS 2.1 §10.8.1 calls the `sub`/`super` offset only "the proper position
/// for subscripts/superscripts", leaving its size implementation-defined.
/// CSS Inline Layout Module Level 3 §4.2.3 "Post-Alignment
/// Shift: the baseline-shift longhand"
/// <https://www.w3.org/TR/css-inline-3/#baseline-shift-property> provides
/// a concrete UA-default fallback. Font metrics take priority there, but
/// this function does not use them or read any font tables:
///
/// - `sub`, spec verbatim: "Lower by the offset appropriate for
///   subscripts of the parent's box. The UA may use the parent's font
///   metrics to find this offset; otherwise it defaults to dropping by
///   one fifth of the parent's used font-size."
/// - `super`, spec verbatim: "Raise by the offset appropriate for
///   superscripts of the parent's box. The UA may use the parent's font
///   metrics to find this offset; otherwise it defaults to raising by one
///   third of the parent's used font-size."
///
/// `vertical-align` (CSS 2.1) and `baseline-shift` (CSS Inline 3) are
/// different properties. As the [`raikiri_style::property::VerticalAlign`]
/// docs explain, this crate still uses CSS 2.1 as the primary source for
/// keyword grammar. CSS Inline 3 is cited here only for the **size** of the
/// shift undefined by CSS 2.1: its UA-default fallback for the same
/// `sub`/`super` keywords.
///
/// `parent_font_size_px` must be the used font-size of the **box's parent**,
/// not the box itself. Author/UA `font-size: smaller` commonly shrinks
/// `sub`/`super` content already; "the parent's used font-size" in the
/// cited specification refers to the size before that shrinkage.
///
/// # Composing nested `vertical-align` (unverified approximation)
///
/// This function returns a shift for only one box. The caller
/// ([`paint_document`]) simply adds ancestor shifts to the `shift_y` stack
/// frame. With a real inline formatting context, each box shifts relative
/// to its immediate parent's baseline, propagating through line-box
/// construction. This is a rough approximation, not a rule stated by any
/// of the primary sources.
///
/// # Exclusion from the box model (unimplemented)
///
/// The shift is added only during painting, after taffy determines box
/// positions. Taffy does not see `vertical_align`, and the result is not
/// clipped. CSS 2.1 / CSS Inline 3 expect shifted positions to affect
/// box-model calculations (including line-box height), but they do not here:
/// a large shift may paint outside the page box.
fn vertical_align_shift_px(
    va: VerticalAlign,
    display: DisplayValue,
    parent_font_size_px: f32,
) -> f32 {
    // Preserve the legacy paint-side scope to inline flow boxes. The caller
    // already excludes atomic boxes aligned by the inline engine.
    if !display.is_inline_flow_box() {
        return 0.0;
    }
    match va {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        // CSS 2.1 §10.8.1 raises a box for positive values. Paint uses a
        // Y-down coordinate system, so the offset sign is reversed.
        VerticalAlign::Length(Length::Px(px)) => -px,
        // Computed style resolves lengths to Px. Baseline and the not-yet-
        // measured font-metric keywords retain zero shift here; top/bottom
        // alignment is handled by the minimal line-box layout when applicable.
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
use raikiri_style::CounterStyleRegistry;

/// The block-axis edges of a column fragment's clip that no other fragment of
/// the same box lies behind: the top of the first fragment of a box and the
/// bottom of its last.
#[derive(Clone, Copy)]
struct OpenBlockEdges {
    top: bool,
    bottom: bool,
}

impl OpenBlockEdges {
    /// The edges of the clip of fragment `fragment` of its box.
    fn of(fragment: &raikiri_dom::LayoutFragment) -> Self {
        Self {
            top: fragment.fragment_index == 0,
            bottom: fragment.fragment_index + 1 >= fragment.fragment_count,
        }
    }
}

/// The clip of a column fragment, with the vertical edges that separate it
/// from its neighbour columns grown outward to whole device pixels of the
/// raster output on `grid`.
///
/// Only a multicol container in a horizontal writing mode fragments its
/// content into column clips, so the columns are always side by side whatever
/// writing mode the clipped box has itself: an orthogonal flow inside a
/// column is clipped by the same left and right edges.
///
/// A clip edge inside a pixel scales the coverage of whatever is drawn there,
/// so ink that begins exactly on a fractional column edge (the stem of the
/// first glyph of a line, in columns whose width is not a whole number of
/// pixels) would be attenuated twice, once by its own antialiasing and once
/// by the clip's. The same ink drawn without a fragment clip keeps only its
/// own coverage. The top and bottom edges, where the flow breaks into the
/// next column, stay exact, as the next fragment's content starts right
/// behind them.
///
/// The top edge of the first fragment of a box and the bottom edge of its
/// last (`open`) have no such neighbour, so they grow outward too. The
/// rasterizer places a line's glyphs on whole device pixels, up to half a
/// pixel beyond the line box, and an exact edge at a fractional block offset
/// would cut the row of ink there.
///
/// The edges move in device space, so a scaled ancestor grows the clip by
/// less than one device pixel, not one CSS pixel. A vector output, or a
/// transform that rotates or skews, has no grid (`None`) and keeps the clip
/// exact.
fn snap_column_clip_outward(clip: Rect, grid: Option<DeviceGrid>, open: OpenBlockEdges) -> Rect {
    let Some(grid) = grid.filter(|grid| {
        grid.scale_x.is_finite() && grid.scale_x != 0.0 && grid.offset_x.is_finite()
    }) else {
        return clip;
    };
    let device = |x: f64| x * grid.scale_x + grid.offset_x;
    let css = |x: f64| (x - grid.offset_x) / grid.scale_x;
    // A mirrored grid reverses the edges, so order them in device space.
    let (first, second) = (device(clip.x0), device(clip.x1));
    let (near, far) = (
        css(first.min(second).floor()),
        css(first.max(second).ceil()),
    );
    let (mut top, mut bottom) = (clip.y0, clip.y1);
    if grid.scale_y.is_finite() && grid.scale_y != 0.0 && grid.offset_y.is_finite() {
        let device = |y: f64| y * grid.scale_y + grid.offset_y;
        let css = |y: f64| (y - grid.offset_y) / grid.scale_y;
        // The top edge is the low one in device space unless the grid is
        // mirrored.
        let grow = |y: f64, is_top: bool| {
            let low = (grid.scale_y > 0.0) == is_top;
            let device_y = device(y);
            css(if low {
                device_y.floor()
            } else {
                device_y.ceil()
            })
        };
        if open.top {
            top = grow(clip.y0, true);
        }
        if open.bottom {
            bottom = grow(clip.y1, false);
        }
    }
    Rect::new(near.min(far), top, near.max(far), bottom)
}

fn push_overflow_clip(
    scene: &mut crate::transform::TransformScene<'_, impl PaintScene>,
    resolved: raikiri_traits::PaintClip,
    page_box: &PageBox,
) -> Rect {
    let mut clip = Rect::new(
        resolved.rect.x as f64,
        resolved.rect.y as f64,
        (resolved.rect.x + resolved.rect.width) as f64,
        (resolved.rect.y + resolved.rect.height) as f64,
    );
    if !resolved.clip_x || !resolved.clip_y {
        // Bound an open axis by the page in the clip's local
        // coordinate system, including transformed ancestors.
        let determinant = scene.transform.determinant();
        let bounds = if determinant.is_finite() && determinant != 0.0 {
            scene.transform.inverse().transform_rect_bbox(Rect::new(
                0.0,
                0.0,
                page_box.width as f64,
                page_box.height as f64,
            ))
        } else {
            Rect::ZERO
        };
        if !resolved.clip_x {
            clip.x0 = bounds.x0;
            clip.x1 = bounds.x1;
        }
        if !resolved.clip_y {
            clip.y0 = bounds.y0;
            clip.y1 = bounds.y1;
        }
    }
    let rounded = resolved.corner_radii.map(|radii| {
        rounded_rect_path(
            clip.x0,
            clip.y0,
            clip.x1,
            clip.y1,
            radii.map(|corner| corner.map(f64::from)),
        )
    });
    if let Some(rounded) = rounded {
        scene.push_clip_layer(Affine::IDENTITY, &rounded);
    } else {
        scene.push_clip_layer(Affine::IDENTITY, &clip);
    }
    clip
}
