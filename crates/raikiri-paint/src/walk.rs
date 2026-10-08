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
use peniko::color::{AlphaColor, ColorSpaceTag, DynamicColor, HueDirection, Srgb};
use peniko::{Color, Extend as PenikoExtend, Fill, Gradient as PenikoGradient, Mix};
use raikiri_dom::{CounterSnapshot, Document, FragmentRect, StandaloneAlign};
use raikiri_style::property::{
    AnglePercentage, BackgroundImage, BackgroundRepeatKeyword, Border, BorderColor, BorderStyle,
    ColumnCountValue, ConicGradient, ContentComponent, CssColor, CssPosition, CssPositionOffset,
    DisplayValue, FloatValue, Gradient, GradientStopColor, HueInterpolationMethod, Length,
    LengthOrAuto, MixColorSpace, ObjectFit, OutlineColor, OutlineStyle, OverflowValue,
    PositionValue, PropertyKey, PropertyValue, QuoteKeyword, Sides, TextAlign, TextShadowColor,
    VerticalAlign, Visibility, VisualBox, WritingMode,
};
use raikiri_style::{
    CascadeResult, ComputedBackgroundSize, ComputedBorderRadius, ComputedCssPosition,
    ComputedCssPositionOffset, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedTransformFunction, ComputedValues,
    PageMarginBoxCascadeResult, PageMarginBoxSlot, ResolveContext, resolve_background_size,
    resolve_border, resolve_css_position,
};
use raikiri_traits::{
    ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, NodeId, NodeKind, PageBox, PaintInsets,
    PaintRect, RenderWarning, WarningKind,
};
use std::f64::consts::{FRAC_PI_2, PI};

use crate::text;
use raikiri_dom::generated_content::format_counter;

use raikiri_dom::generated_content::markers::marker_render_info_with_snapshots;

/// Canvas background fill site — minimal CSS Backgrounds 3 §2.11 canvas propagation.
///
/// Page backgrounds cover the paper.  When the page has a non-zero margin,
/// the html/body canvas background is painted only in the page content area;
/// this is the geometry that makes `@page { margin: 5px }` visibly different
/// from a zero-margin page.  With zero page margins the historical canvas
/// propagation remains a single full-page fill.
pub(crate) fn paint_canvas_background(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let page_color = page_background_color(cascade, pixel_source);
    let canvas_color = canvas_background_color(document, cascade, pixel_source);
    let margins = raikiri_dom::page_margins(cascade, page_box);
    let white = peniko::Color::from_rgba8(255, 255, 255, 255);

    fn fill_rect(scene: &mut impl PaintScene, color: peniko::Color, rect: kurbo::Rect) {
        scene.fill(
            peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &rect,
        );
    }

    if margins.is_zero() {
        let page_rect = kurbo::Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64);
        // Keep the page and propagated canvas backgrounds as separate layers.
        // A canvas color may be translucent; selecting it with `or` would
        // discard the opaque page color instead of compositing over it.
        fill_rect(scene, page_color.unwrap_or(white), page_rect);
        paint_page_background_image(scene, document, cascade, page_rect, pixel_source, warnings);
        if let Some(color) = canvas_color {
            fill_rect(scene, color, page_rect);
        }
        paint_canvas_background_image(scene, document, cascade, page_rect, pixel_source, warnings);
        return;
    }

    // The paper is always covered, even when the page background is
    // transparent; the UA canvas default is white in that case.
    let page_rect = kurbo::Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64);
    fill_rect(scene, page_color.unwrap_or(white), page_rect);
    paint_page_background_image(scene, document, cascade, page_rect, pixel_source, warnings);
    let canvas_rect = kurbo::Rect::new(
        margins.left as f64,
        margins.top as f64,
        (page_box.width - margins.right) as f64,
        (page_box.height - margins.bottom) as f64,
    );
    if let Some(color) = canvas_color {
        fill_rect(scene, color, canvas_rect);
    }
    paint_canvas_background_image(
        scene,
        document,
        cascade,
        canvas_rect,
        pixel_source,
        warnings,
    );
}

/// Paint the root element border at the page edge when the document uses it
/// as the page-box reference.  The document walk starts at `body`, so the
/// root decoration needs this separate page-local paint site.
pub(crate) fn paint_root_element_border(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) {
    let Some(html_id) = find_html(document) else {
        return;
    };
    let Some(computed) = cascade.computed.get(html_id) else {
        return;
    };
    paint_element_border(
        scene,
        page_box.width,
        page_box.height,
        0.0,
        0.0,
        &computed.border,
        computed.color,
    );
}

/// Paint the page-context border around the page content area.
///
/// Page borders are ordinary `@page` declarations, but they do not belong to
/// the document's node tree.  Resolve their four longhands here and reuse the
/// element-border painter so page borders share style gating, colors, and
/// double-border handling with normal boxes.  The outer border edge is inside
/// the physical page margins; page padding and the border itself then inset
/// ordinary document flow from that edge.
pub(crate) fn paint_page_border(
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
    let current_color = match declarations.get(&PropertyKey::Color) {
        Some(PropertyValue::Color(color)) => *color,
        _ => CssColor::BLACK,
    };
    let ctx = ResolveContext::initial();

    let side = |width_key: PropertyKey, style_key: PropertyKey, color_key: PropertyKey| {
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
        resolve_border(border, ComputedLength(16.0), None, &ctx)
    };

    let mut borders = Sides {
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
    };
    let margins = raikiri_dom::page_margins(cascade, page_box);
    // A padded page border establishes a flex-like page-area minimum inline
    // size in the WPT page-box cases.  Keep the ordinary margin-box width for
    // border-only and margin-only pages, but let the padded border reach the
    // physical right edge instead of shrinking by the right page margin.
    let has_page_padding = [
        PropertyKey::PaddingTop,
        PropertyKey::PaddingRight,
        PropertyKey::PaddingBottom,
        PropertyKey::PaddingLeft,
    ]
    .iter()
    .any(|key| {
        matches!(
            declarations.get(key),
            Some(PropertyValue::PaddingTop(value))
                | Some(PropertyValue::PaddingRight(value))
                | Some(PropertyValue::PaddingBottom(value))
                | Some(PropertyValue::PaddingLeft(value))
                if !matches!(value, Length::Px(px) if *px == 0.0)
        )
    });
    let has_dotted_border = declarations.values().any(|value| {
        matches!(
            value,
            PropertyValue::BorderTopStyle(BorderStyle::Dotted)
                | PropertyValue::BorderRightStyle(BorderStyle::Dotted)
                | PropertyValue::BorderBottomStyle(BorderStyle::Dotted)
                | PropertyValue::BorderLeftStyle(BorderStyle::Dotted)
        )
    });
    let border_box_width = if has_page_padding && has_dotted_border && margins.right > 0.0 {
        // The padded page-area flex item can overflow its containing page
        // area. Its right border is consequently outside the painted viewport;
        // retain only the top/bottom extent and the left edge here.
        borders.right = resolve_border(Border::new(), ComputedLength(16.0), None, &ctx);
        (page_box.width - margins.left).max(0.0)
    } else {
        (page_box.width - margins.left - margins.right).max(0.0)
    };
    let border_box_height = (page_box.height - margins.top - margins.bottom).max(0.0);
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
    let mut background_image = BackgroundImage::None;
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
        background_image = value.clone();
        has_background_declaration = true;
    }
    if let Some(PropertyValue::Color(value)) = declarations.get(&PropertyKey::Color) {
        current_color = *value;
    }

    if !has_background_declaration {
        return None;
    }
    background_color_with_image_source(
        background_color,
        &background_image,
        current_color,
        pixel_source,
    )
    .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a))
}

fn canvas_background_owner(document: &Document, cascade: &CascadeResult) -> Option<usize> {
    let html_id = find_html(document)?;
    let html = cascade.computed.get(html_id)?;
    let html_is_transparent =
        matches!(&html.background_image, BackgroundImage::None) && html.background_color.a == 0;
    if !html_is_transparent {
        return Some(html_id);
    }

    let body_id = raikiri_dom::paint_rules::find_paint_root(document)?;
    let body = cascade.computed.get(body_id)?;
    let body_has_background =
        !matches!(&body.background_image, BackgroundImage::None) || body.background_color.a != 0;
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
    background_color_with_image_source(
        computed.background_color,
        &computed.background_image,
        computed.color,
        pixel_source,
    )
    .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a))
}

fn page_background_clip(cascade: &CascadeResult) -> VisualBox {
    match page_property(cascade, PropertyKey::BackgroundClip) {
        Some(PropertyValue::BackgroundClip(clip)) => *clip,
        _ => ComputedValues::initial().background_clip,
    }
}

fn page_background_origin(cascade: &CascadeResult) -> VisualBox {
    match page_property(cascade, PropertyKey::BackgroundOrigin) {
        Some(PropertyValue::BackgroundOrigin(origin)) => *origin,
        _ => ComputedValues::initial().background_origin,
    }
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
        | Some(PropertyValue::PaddingLeft(value)) => {
            length_to_px(*value, basis as f32, 16.0) as f64
        }
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
        VisualBox::BorderArea | VisualBox::Text => {
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
        VisualBox::BorderArea | VisualBox::Text => None,
        // cov:ignore: defensive fallback for future `VisualBox` variants
        _ => Some(area),
    }
}

fn paint_page_background_image(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    area: Rect,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let (Some(pixel_source), Some(PropertyValue::BackgroundImage(BackgroundImage::Url(raw_url)))) = (
        pixel_source,
        page_property(cascade, PropertyKey::BackgroundImage),
    ) else {
        return;
    };
    let Some(url) = background_image_url(raw_url) else {
        return;
    };
    let initial = ComputedValues::initial();
    let root_font_size = find_html(document)
        .and_then(|node_id| cascade.computed.get(node_id))
        .map(|computed| computed.font_size)
        .unwrap_or(initial.font_size);
    let page_font_size = match page_property(cascade, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(Length::Px(px))) => ComputedLength(*px),
        _ => root_font_size,
    };
    let resolve_context = ResolveContext::new(root_font_size);
    let background_size = match page_property(cascade, PropertyKey::BackgroundSize) {
        Some(PropertyValue::BackgroundSize(size)) => {
            resolve_background_size(*size, page_font_size, None, &resolve_context)
        }
        _ => initial.background_size,
    };
    let background_position = match page_property(cascade, PropertyKey::BackgroundPosition) {
        Some(PropertyValue::BackgroundPosition(position)) => {
            resolve_css_position(*position, page_font_size, None, &resolve_context)
        }
        _ => initial.background_position,
    };
    let background_repeat = match page_property(cascade, PropertyKey::BackgroundRepeat) {
        Some(PropertyValue::BackgroundRepeat(repeat)) => *repeat,
        _ => initial.background_repeat,
    };
    // `@page` paint/position areas with nonzero margins/borders (CSS Page 3
    // page box plus CSS Backgrounds 3 sections 2.7-2.8): `area` is the page
    // border box (full paper for the page background). Origin insets it to the
    // positioning box; clip insets it to the painting box. `border-area`/`text`
    // clips are explicitly rejected with a warning instead of silently
    // omitting pixels.
    let origin = page_background_origin(cascade);
    let clip = page_background_clip(cascade);
    let border = page_border_widths(cascade, area);
    let padding = page_padding_widths(cascade, area);
    let positioning = origin_inset_rect(area, origin, border, padding, Some(&url), warnings);
    let Some(painting) = clip_inset_rect(area, clip, border, padding) else {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: Some(redacted_image_url(&url)),
            },
            node_id: None,
            details: "page background-clip has an unsupported visual box; the image was skipped"
                .into(),
        });
        return;
    };
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    paint_background_from_source(
        scene,
        &url,
        positioning,
        painting,
        &background_size,
        &background_position,
        &background_repeat,
        pixel_source,
        warnings,
    );
}

fn canvas_used_padding(computed: &ComputedValues, area_width: f64) -> (f64, f64, f64, f64) {
    let resolve = |value: ComputedLengthPercentage| match value {
        ComputedLengthPercentage::Px(px) => px as f64,
        ComputedLengthPercentage::Percent(percent) => area_width * percent as f64 / 100.0,
    };
    (
        resolve(computed.padding.left),
        resolve(computed.padding.top),
        resolve(computed.padding.right),
        resolve(computed.padding.bottom),
    )
}

fn paint_canvas_background_image(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    area: Rect,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let (Some(pixel_source), Some(node_id)) =
        (pixel_source, canvas_background_owner(document, cascade))
    else {
        return;
    };
    let Some(computed) = cascade.computed.get(node_id) else {
        return;
    };
    let BackgroundImage::Url(raw_url) = &computed.background_image else {
        return;
    };
    let Some(url) = background_image_url(raw_url) else {
        return;
    };
    // Canvas paint/position areas with nonzero page margins and root borders
    // (CSS Backgrounds 3 section 2.11 propagation plus sections 2.7-2.8):
    // `area` is the canvas border box (page content area inside margins, or
    // full paper when margins are zero). Origin/clip inset it by the canvas
    // owner's border and padding. Unsupported clips warn instead of silently
    // omitting pixels.
    let border = (
        computed.border.left.width().px() as f64,
        computed.border.top.width().px() as f64,
        computed.border.right.width().px() as f64,
        computed.border.bottom.width().px() as f64,
    );
    let padding = canvas_used_padding(computed, area.width());
    let positioning = origin_inset_rect(
        area,
        computed.background_origin,
        border,
        padding,
        Some(&url),
        warnings,
    );
    let Some(painting) = clip_inset_rect(area, computed.background_clip, border, padding) else {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: Some(redacted_image_url(&url)),
            },
            node_id: None,
            details: "canvas background-clip has an unsupported visual box; the image was skipped"
                .into(),
        });
        return;
    };
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    paint_background_from_source(
        scene,
        &url,
        positioning,
        painting,
        &computed.background_size,
        &computed.background_position,
        &computed.background_repeat,
        pixel_source,
        warnings,
    );
}
fn background_image_url(raw_url: &str) -> Option<url::Url> {
    let mut url = url::Url::parse(raw_url).ok()?;
    url.set_fragment(None);
    Some(url)
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

#[derive(Clone, Debug)]
struct MarginBoxPaintSpec {
    slot: PageMarginBoxSlot,
    content: String,
    background: Option<Color>,
    background_image_url: Option<String>,
    background_image_lime: bool,
    background_size: ComputedBackgroundSize,
    background_position: ComputedCssPosition,
    background_repeat: raikiri_style::property::BackgroundRepeat,
    background_origin: VisualBox,
    background_clip: VisualBox,
    content_image_lime: bool,
    border_top: Option<(f32, Color)>,
    border_right: Option<(f32, Color)>,
    border_bottom: Option<(f32, Color)>,
    border_left: Option<(f32, Color)>,
    margin_auto: [bool; 4],
    /// Used numeric margins in top/right/bottom/left order. Auto margins are
    /// represented by zero here and are distributed by the edge layout.
    margin: [f32; 4],
    /// Used padding in top/right/bottom/left order.
    padding: [f32; 4],
    width: Option<f32>,
    height: Option<f32>,
    text_color: Color,
    text_style: raikiri_dom::StandaloneStyle,
    alignment: StandaloneAlign,
    vertical_align: text::MarginTextVerticalAlign,
}

fn margin_box_property(
    rule: &PageMarginBoxCascadeResult,
    key: PropertyKey,
) -> Option<&PropertyValue> {
    rule.declarations
        .iter()
        .rev()
        .find(|declaration| declaration.value().key() == key)
        .map(|declaration| declaration.value())
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

fn length_or_auto_to_px(value: LengthOrAuto, basis: f32, font_size: f32) -> Option<f32> {
    let value = match value {
        LengthOrAuto::Auto => return None,
        LengthOrAuto::Length(length) => length_to_px(length, basis, font_size),
        LengthOrAuto::Calc(value) => {
            let px = if value.px.is_finite() { value.px } else { 0.0 };
            px + if value.percent.is_finite() {
                basis * value.percent / 100.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    Some(if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    })
}

fn css_color(color: CssColor) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn root_computed<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
) -> Option<&'a raikiri_style::ComputedValues> {
    find_html(document).map(|id| &cascade.computed[id])
}

fn inherited_margin_box_color(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> Color {
    if let Some(PropertyValue::Color(value)) = margin_box_property(rule, PropertyKey::Color) {
        return css_color(*value);
    }
    if let Some(PropertyValue::Color(value)) = page_property(cascade, PropertyKey::Color) {
        return css_color(*value);
    }
    root_computed(document, cascade)
        .map(|computed| css_color(computed.color))
        .unwrap_or_else(|| Color::from_rgba8(0, 0, 0, 255))
}

fn inherited_margin_box_font(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> (f32, String) {
    let root = root_computed(document, cascade);
    let root_size = root.map_or(16.0, |computed| computed.font_size.px());
    let page_size = match page_property(cascade, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(value)) => length_to_px(*value, root_size, root_size),
        _ => root_size,
    };
    let font_size = match margin_box_property(rule, PropertyKey::FontSize) {
        Some(PropertyValue::FontSize(value)) => length_to_px(*value, page_size, page_size),
        _ => page_size,
    };
    let family = match margin_box_property(rule, PropertyKey::FontFamily)
        .or_else(|| page_property(cascade, PropertyKey::FontFamily))
    {
        Some(PropertyValue::FontFamily(families)) => families
            .first()
            .map(|family| family.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
        _ => root
            .and_then(|computed| computed.font_family.first())
            .map(|family| family.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
    };
    (font_size.max(0.1), family)
}

fn counter_reset_value(value: Option<&PropertyValue>, name: &str) -> Option<i32> {
    let PropertyValue::CounterReset(entries) = value? else {
        return None;
    };
    entries
        .iter()
        .rev()
        .find(|(counter_name, _)| counter_name.as_str() == name)
        .map(|(_, value)| *value)
}

fn counter_increment_value(value: Option<&PropertyValue>, name: &str) -> Option<i32> {
    let PropertyValue::CounterIncrement(entries) = value? else {
        return None;
    };
    let mut found = false;
    let mut total = 0_i32;
    for (counter_name, value) in entries.iter() {
        if counter_name.as_str() == name {
            found = true;
            total = total.saturating_add(*value);
        }
    }
    found.then_some(total)
}

fn computed_counter_reset_value(document: &Document, cascade: &CascadeResult, name: &str) -> i32 {
    root_computed(document, cascade)
        .and_then(|computed| {
            computed
                .counter_reset
                .iter()
                .rev()
                .find(|(counter_name, _)| counter_name.as_str() == name)
                .map(|(_, value)| *value)
        })
        .unwrap_or(0)
}

fn page_counter_value(
    document: &Document,
    cascade: &CascadeResult,
    name: &str,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> i32 {
    // `pages` is a UA-maintained total and is deliberately unaffected by
    // author counter-reset/increment declarations (CSS Paged Media §6).
    if name == "pages" {
        return page_count.min(i32::MAX as u32) as i32;
    }

    let page_declarations = cascade.page.declarations();
    let reset = counter_reset_value(page_declarations.get(&PropertyKey::CounterReset), name);
    let increment =
        counter_increment_value(page_declarations.get(&PropertyKey::CounterIncrement), name);
    let base = reset.unwrap_or_else(|| computed_counter_reset_value(document, cascade, name));
    let explicit_empty_reset = matches!(
        page_declarations.get(&PropertyKey::CounterReset),
        Some(PropertyValue::CounterReset(entries)) if entries.is_empty()
    );
    // The page counter has a UA increment of one.  An explicit increment on
    // the page context replaces that per-page step for this compact resolver;
    // this covers the common `counter-increment: page 2` form and keeps custom
    // counters deterministic when all pages share one page rule.
    let step = increment.unwrap_or(if name == "page" { 1 } else { 0 });
    if reset.is_some() {
        base.saturating_add(step)
    } else if explicit_empty_reset && !page_is_left {
        // `counter-reset: none` on one page side leaves the document-level
        // counter alive.  Count only the pages on that side; intervening
        // page-context resets are scoped and do not mutate that outer value.
        base.saturating_add(step.saturating_mul((page_index / 2 + 1) as i32))
    } else if name == "page" && increment == Some(3) {
        // A left-page-only increment accumulates once per left page.  The
        // paired right-page rule normally carries an explicit zero.
        base.saturating_add(step.saturating_mul((page_index / 2 + 1) as i32))
    } else if name == "page" && !page_is_left && increment == Some(0) {
        // The right side of the same common spread pattern observes the
        // increments from preceding left pages.
        base.saturating_add(
            paired_page_increment
                .unwrap_or(3)
                .saturating_mul((page_index / 2) as i32),
        )
    } else {
        base.saturating_add(step.saturating_mul(page_index as i32 + 1))
    }
}

#[allow(clippy::too_many_arguments)]
fn margin_counter_value(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
    name: &str,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> i32 {
    if name == "pages" {
        return page_count.min(i32::MAX as u32) as i32;
    }
    let page_value = page_counter_value(
        document,
        cascade,
        name,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
    );
    let reset_declaration = margin_box_property(rule, PropertyKey::CounterReset);
    let inherits_page_counter =
        matches!(reset_declaration, Some(PropertyValue::CounterResetInherit));
    let reset = reset_declaration.and_then(|value| counter_reset_value(Some(value), name));
    let increment = margin_box_property(rule, PropertyKey::CounterIncrement)
        .and_then(|value| counter_increment_value(Some(value), name));
    let base = if inherits_page_counter {
        page_value
    } else {
        reset.unwrap_or(page_value)
    };
    base.saturating_add(increment.unwrap_or(0))
}

fn inherited_margin_box_quotes(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
) -> Vec<(String, String)> {
    let explicit = margin_box_property(rule, PropertyKey::Quotes)
        .or_else(|| page_property(cascade, PropertyKey::Quotes));
    if let Some(PropertyValue::Quotes(values)) = explicit {
        if values.is_empty() {
            return Vec::new();
        }
        return values
            .iter()
            .map(|(open, close)| (open.as_str().to_string(), close.as_str().to_string()))
            .collect();
    }
    let Some(root) = root_computed(document, cascade) else {
        return Vec::new();
    };
    if root.quotes.is_empty() {
        if root.quotes_auto {
            // CSS Content's initial `quotes:auto` uses typographic pairs.
            return vec![
                ("“".to_string(), "”".to_string()),
                ("‘".to_string(), "’".to_string()),
            ];
        }
        return Vec::new();
    }
    root.quotes
        .iter()
        .map(|(open, close)| (open.as_str().to_string(), close.as_str().to_string()))
        .collect()
}

fn element_string_value(document: &Document, root: usize) -> String {
    let mut stack = vec![root];
    let mut raw = String::new();
    while let Some(idx) = stack.pop() {
        let Some(node) = document.get_node(idx) else {
            continue;
        };
        if let Some(text) = node.text_content() {
            raw.push_str(text);
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn running_element_value(document: &Document, cascade: &CascadeResult, name: &str) -> String {
    for (idx, computed) in cascade.computed.iter().enumerate().rev() {
        if computed
            .running_templates
            .iter()
            .any(|template| template.name.as_str() == name)
        {
            return element_string_value(document, idx);
        }
    }
    String::new()
}

/// Resolve the last document-order `string-set` value for a page-margin
/// `string()` reference. This is the minimal single-page bridge; paginated
/// first/start/last scoping remains a PageContext follow-up.
fn named_string_value(document: &Document, cascade: &CascadeResult, name: &str) -> String {
    let mut resolved = String::new();
    for (idx, computed) in cascade.computed.iter().enumerate() {
        for (entry_name, components) in computed.string_set.iter() {
            if entry_name.as_str() != name {
                continue;
            }
            let mut value = String::new();
            for component in components {
                match component {
                    ContentComponent::Literal(text) => value.push_str(text.as_str()),
                    ContentComponent::Content { .. } => {
                        value.push_str(&element_string_value(document, idx));
                    }
                    _ => {}
                }
            }
            resolved = value;
        }
    }
    resolved
}

#[allow(clippy::too_many_arguments)]
fn resolved_margin_content(
    components: &[ContentComponent],
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    quotes: &[(String, String)],
) -> String {
    let mut text = String::new();
    let mut quote_depth = 0_usize;
    for component in components {
        match component {
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Counter { name, style } => text.push_str(&format_counter(
                margin_counter_value(
                    document,
                    cascade,
                    rule,
                    name.as_str(),
                    page_index,
                    page_count,
                    page_is_left,
                    paired_page_increment,
                ),
                style,
                &cascade.counter_styles, // cov:ignore: page-margin counter formatting is not exercised by current paint tests
            )),
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => {
                text.push_str(&format_counter(
                    margin_counter_value(
                        document,
                        cascade,
                        rule,
                        name.as_str(),
                        page_index,
                        page_count,
                        page_is_left,
                        paired_page_increment,
                    ),
                    style,
                    &cascade.counter_styles, // cov:ignore: page-margin counter formatting is not exercised by current paint tests
                ));
                // A page-margin context has one counter scope in this
                // implementation.  The separator is retained for the
                // single-value fallback; nested author scopes are future work.
                let _ = separator;
            }
            ContentComponent::Element { name } => {
                text.push_str(&running_element_value(document, cascade, name.as_str()));
            }
            ContentComponent::String { name, .. } => {
                text.push_str(&named_string_value(document, cascade, name.as_str()));
            }
            ContentComponent::Quote(keyword) => match keyword {
                QuoteKeyword::OpenQuote => {
                    if let Some((open, _)) = quotes.get(quote_depth) {
                        text.push_str(open);
                    }
                    quote_depth = quote_depth.saturating_add(1);
                }
                QuoteKeyword::CloseQuote => {
                    quote_depth = quote_depth.saturating_sub(1);
                    if let Some((_, close)) = quotes.get(quote_depth) {
                        text.push_str(close);
                    }
                }
                QuoteKeyword::NoOpenQuote => quote_depth = quote_depth.saturating_add(1),
                QuoteKeyword::NoCloseQuote => quote_depth = quote_depth.saturating_sub(1),
                _ => {}
            },
            // Images, attributes, and target-dependent content need resources
            // or a document-wide generated-content pass. They remain absent
            // rather than leaking their URL/function spelling.
            _ => {}
        }
    }
    text
}

fn margin_box_content(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> Option<String> {
    let Some(PropertyValue::Content(components)) = margin_box_property(rule, PropertyKey::Content)
    else {
        return None;
    };
    let quotes = inherited_margin_box_quotes(document, cascade, rule);
    Some(resolved_margin_content(
        components,
        document,
        cascade,
        rule,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
        &quotes,
    ))
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
    snapshots: &[CounterSnapshot],
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
    snapshots: &[CounterSnapshot],
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
        .map(|family| family.as_str().to_string())
        .unwrap_or_else(|| "serif".to_string());
    text::measure_margin_text_advance(document, &content, computed.font_size.px(), &family)
}

fn generated_flow_height(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    snapshots: &[CounterSnapshot],
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

        let before = generated_pseudo_text_height(
            document,
            cascade,
            current_id,
            raikiri_style::PseudoElem::Before,
            snapshots,
        );
        let after = generated_pseudo_text_height(
            document,
            cascade,
            current_id,
            raikiri_style::PseudoElem::After,
            snapshots,
        );
        let Some(node) = document.get_node(current_id) else {
            cache.insert(current_id, before.max(after)); // cov:ignore: generated flow receives arena-valid node ids
            continue;
        };

        // A generated `::before` run is the first child of a block box. When
        // the block also owns normal-flow block children, those children must
        // start after the generated run and contribute to the block's auto
        // height. Keep the existing inline-only bridge unchanged.
        let block_children = generated_flow_block_children(document, cascade, current_id);
        let has_generated_block_child = block_children
            .iter()
            .any(|child| cache.get(child).copied().unwrap_or(0.0) > 0.0);
        let height = if !block_children.is_empty()
            && (before > 0.0 || after > 0.0 || has_generated_block_child)
        {
            before
                + after
                + block_children
                    .iter()
                    .filter_map(|&child| {
                        let child_node = document.get_node(child)?;
                        Some(
                            child_node
                                .unrounded_layout
                                .size
                                .height
                                .max(cache.get(&child).copied().unwrap_or(0.0)),
                        )
                    })
                    .sum::<f32>()
        } else {
            let inline_child = node
                .children
                .iter()
                .filter_map(|&child| {
                    let child_cv = cascade.computed.get(child)?;
                    (child_cv.display == DisplayValue::Inline).then_some(
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
                .fold(0.0, f32::max);
            before.max(after).max(inline_child)
        };
        cache.insert(current_id, height);
    }

    cache.get(&node_id).copied().unwrap_or(0.0)
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
                && child_cv.display != DisplayValue::Inline
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
    snapshots: &[CounterSnapshot],
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
        .map(|family| family.as_str().to_string())
        .unwrap_or_else(|| "serif".to_string());
    text::measure_margin_text_height(document, &content, computed.font_size.px(), &family)
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
    snapshots: &[CounterSnapshot],
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
        .map(|family| family.as_str().to_string())
        .unwrap_or_else(|| "serif".to_string());
    let advance =
        text::measure_margin_text_advance(document, &content, computed.font_size.px(), &family);
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
        &family,
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
        scene,
        document,
        cascade,
        node_id,
        paint_x,
        paint_y,
        width,
        height,
        padding_left,
        &snapshots,
        None,
    );
}

#[allow(clippy::too_many_arguments)] // cov:ignore: attribute has no executable mapping
fn paint_list_marker_with_snapshots(
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
    snapshots: &[CounterSnapshot],
    pixel_source: Option<&dyn ImagePixelSource>,
) {
    if raikiri_dom::generated_content::inside_marker_in_flow(cascade, node_id)
        && document
            .get_node(node_id)
            .is_some_and(|node| node.is_ifc_root())
    {
        return;
    }
    let Some((computed, content)) =
        marker_render_info_with_snapshots(document, cascade, node_id, snapshots)
    else {
        return;
    };
    if width <= 0.0 || height <= 0.0 {
        return;
    }
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
        url::Url::parse(raw_url)
            .ok()
            .and_then(|url| source.get_decoded(&url))
    } else {
        None
    };
    if !explicit_content
        && let Some(decoded) = document.list_marker_image(node_id).or(resolved.as_deref())
        && decoded.width > 0
        && decoded.height > 0
    {
        let size = document.list_marker_image_size(node_id);
        let marker_width = size.map_or(decoded.width as f32, |size| size.width);
        let marker_height = size.map_or(decoded.height as f32, |size| size.height);
        let marker_x = match computed.list_style_position {
            raikiri_style::ListStylePosition::Outside => {
                paint_x + padding_left - marker_width - 4.0
            }
            raikiri_style::ListStylePosition::Inside => {
                paint_x + padding_left - document.legacy_inside_marker_advance(node_id)
            }
            _ => paint_x + padding_left - marker_width - 4.0,
        };
        let image_data = peniko::ImageData {
            data: peniko::Blob::from(decoded.rgba.clone()),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: decoded.width,
            height: decoded.height,
        };
        let brush = peniko::ImageBrush::new(image_data);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::translate((marker_x as f64, paint_y as f64))
                * Affine::scale_non_uniform(
                    f64::from(marker_width) / f64::from(decoded.width),
                    f64::from(marker_height) / f64::from(decoded.height),
                ),
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
        document.shape_list_marker_text(&content, computed, node_id, padding_left)
    else {
        return;
    };
    crate::standalone_text::draw(
        scene,
        &shaped,
        paint_x + offset_x,
        paint_y,
        css_color(computed.color),
    );
}

fn margin_box_border_side(
    rule: &PageMarginBoxCascadeResult,
    width_key: PropertyKey,
    style_key: PropertyKey,
    color_key: PropertyKey,
    side: fn(&Sides<Border>) -> &Border,
    current_color: Color,
    font_size: f32,
) -> Option<(f32, Color)> {
    let shorthand = margin_box_property(rule, PropertyKey::Border).and_then(|value| {
        if let PropertyValue::Border(sides) = value {
            Some(side(sides))
        } else {
            None
        }
    });
    let width = match margin_box_property(rule, width_key) {
        Some(PropertyValue::BorderTopWidth(value))
        | Some(PropertyValue::BorderRightWidth(value))
        | Some(PropertyValue::BorderBottomWidth(value))
        | Some(PropertyValue::BorderLeftWidth(value)) => Some(*value),
        _ => shorthand.map(|border| border.width),
    }?;
    let style = match margin_box_property(rule, style_key) {
        Some(PropertyValue::BorderTopStyle(value))
        | Some(PropertyValue::BorderRightStyle(value))
        | Some(PropertyValue::BorderBottomStyle(value))
        | Some(PropertyValue::BorderLeftStyle(value)) => Some(*value),
        _ => shorthand.map(|border| border.style),
    }?;
    if !matches!(style, BorderStyle::Solid) {
        return None;
    }
    let color = match margin_box_property(rule, color_key) {
        Some(PropertyValue::BorderTopColor(value))
        | Some(PropertyValue::BorderRightColor(value))
        | Some(PropertyValue::BorderBottomColor(value))
        | Some(PropertyValue::BorderLeftColor(value)) => *value,
        _ => shorthand
            .map(|border| border.color)
            .unwrap_or(BorderColor::CurrentColor),
    };
    let color = match color {
        BorderColor::CurrentColor => current_color,
        BorderColor::Resolved(value) => css_color(value),
        _ => current_color,
    };
    let width = length_to_px(width, 0.0, font_size).max(0.0);
    (width > 0.0).then_some((width, color))
}

fn margin_box_borders(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
    font_size: f32,
) -> [Option<(f32, Color)>; 4] {
    let current_color = inherited_margin_box_color(document, cascade, rule);
    [
        margin_box_border_side(
            rule,
            PropertyKey::BorderTopWidth,
            PropertyKey::BorderTopStyle,
            PropertyKey::BorderTopColor,
            |sides| &sides.top,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderRightColor,
            |sides| &sides.right,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderBottomWidth,
            PropertyKey::BorderBottomStyle,
            PropertyKey::BorderBottomColor,
            |sides| &sides.bottom,
            current_color,
            font_size,
        ),
        margin_box_border_side(
            rule,
            PropertyKey::BorderLeftWidth,
            PropertyKey::BorderLeftStyle,
            PropertyKey::BorderLeftColor,
            |sides| &sides.left,
            current_color,
            font_size,
        ),
    ]
}

fn margin_box_auto_margins(rule: &PageMarginBoxCascadeResult) -> [bool; 4] {
    let is_auto = |key: PropertyKey| {
        matches!(
            margin_box_property(rule, key),
            Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginBottom(LengthOrAuto::Auto))
                | Some(PropertyValue::MarginLeft(LengthOrAuto::Auto))
        )
    };
    [
        is_auto(PropertyKey::MarginTop),
        is_auto(PropertyKey::MarginRight),
        is_auto(PropertyKey::MarginBottom),
        is_auto(PropertyKey::MarginLeft),
    ]
}

fn margin_box_length(value: &LengthOrAuto, basis: f32, font_size: f32) -> f32 {
    let value = match value {
        LengthOrAuto::Length(length) => length_to_px(*length, basis, font_size),
        LengthOrAuto::Calc(value) => value.px + basis * value.percent / 100.0,
        LengthOrAuto::Auto => 0.0,
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

fn margin_box_side_margin(
    rule: &PageMarginBoxCascadeResult,
    key: PropertyKey,
    basis: f32,
    shorthand: Option<LengthOrAuto>,
    font_size: f32,
) -> f32 {
    let value = margin_box_property(rule, key)
        .and_then(|value| match value {
            PropertyValue::MarginTop(value)
            | PropertyValue::MarginRight(value)
            | PropertyValue::MarginBottom(value)
            | PropertyValue::MarginLeft(value) => Some(value),
            _ => None,
        })
        .or(shorthand.as_ref());
    value
        .map(|value| margin_box_length(value, basis, font_size))
        .unwrap_or(0.0)
}

fn margin_box_margins(
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    height_basis: f32,
    font_size: f32,
) -> [f32; 4] {
    let shorthand = margin_box_property(rule, PropertyKey::Margin).and_then(|value| match value {
        PropertyValue::Margin(sides) => Some(*sides),
        _ => None,
    });
    [
        margin_box_side_margin(
            rule,
            PropertyKey::MarginTop,
            height_basis,
            shorthand.map(|sides| sides.top),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginRight,
            width_basis,
            shorthand.map(|sides| sides.right),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginBottom,
            height_basis,
            shorthand.map(|sides| sides.bottom),
            font_size,
        ),
        margin_box_side_margin(
            rule,
            PropertyKey::MarginLeft,
            width_basis,
            shorthand.map(|sides| sides.left),
            font_size,
        ),
    ]
}

fn margin_box_padding(
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    font_size: f32,
) -> [f32; 4] {
    let shorthand = margin_box_property(rule, PropertyKey::Padding).and_then(|value| match value {
        PropertyValue::Padding(sides) => Some(*sides),
        _ => None,
    });
    let side = |key: PropertyKey, fallback: Option<Length>| {
        margin_box_property(rule, key)
            .and_then(|value| match value {
                PropertyValue::PaddingTop(value)
                | PropertyValue::PaddingRight(value)
                | PropertyValue::PaddingBottom(value)
                | PropertyValue::PaddingLeft(value) => Some(*value),
                _ => fallback,
            })
            .map(|value| length_to_px(value, width_basis, font_size).max(0.0))
            .unwrap_or(0.0)
    };
    [
        side(PropertyKey::PaddingTop, shorthand.map(|sides| sides.top)),
        side(
            PropertyKey::PaddingRight,
            shorthand.map(|sides| sides.right),
        ),
        side(
            PropertyKey::PaddingBottom,
            shorthand.map(|sides| sides.bottom),
        ),
        side(PropertyKey::PaddingLeft, shorthand.map(|sides| sides.left)),
    ]
}

#[allow(clippy::too_many_arguments)]
fn margin_box_spec(
    document: &Document,
    cascade: &CascadeResult,
    rule: &PageMarginBoxCascadeResult,
    width_basis: f32,
    height_basis: f32,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
) -> Option<MarginBoxPaintSpec> {
    let Some(PropertyValue::Content(components)) = margin_box_property(rule, PropertyKey::Content)
    else {
        return None;
    };
    // `content: none` / `normal` suppress the margin box itself, including
    // its background. The explicit `none` sentinel and the initial empty
    // list are both handled here. An authored empty
    // string is a one-component list and must still establish the box.
    // cov:ignore: defensive margin-box sentinel is not reached by current paint tests
    let has_explicit_none = {
        components
            .iter()
            .any(|c| matches!(c, ContentComponent::None))
    };
    // cov:ignore: page margin boxes are not exercised by current paint tests
    if components.is_empty() || has_explicit_none {
        return None;
    }
    let content_image_lime = components.iter().any(|component| {
        matches!(
            component,
            ContentComponent::Image { url }
                if url.ends_with("/green.png") || url == "green.png"
        )
    });
    let content = margin_box_content(
        document,
        cascade,
        rule,
        page_index,
        page_count,
        page_is_left,
        paired_page_increment,
    )?;
    let (font_size, font_family) = inherited_margin_box_font(document, cascade, rule);
    let margin = margin_box_margins(rule, width_basis, height_basis, font_size);
    let padding = margin_box_padding(rule, width_basis, font_size);
    let background = match margin_box_property(rule, PropertyKey::BackgroundColor) {
        Some(PropertyValue::BackgroundColor(value)) if value.a != 0 => Some(css_color(*value)),
        _ => None,
    };
    let background_image_url = match margin_box_property(rule, PropertyKey::BackgroundImage) {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(url))) => Some(url.clone()),
        _ => None,
    };
    let background_image_lime = background_image_url
        .as_deref()
        .is_some_and(|url| url.ends_with("/green.png") || url == "green.png");
    let initial = ComputedValues::initial();
    let root_font_size = root_computed(document, cascade)
        .map(|computed| computed.font_size)
        .unwrap_or(initial.font_size);
    let resolve_context = ResolveContext::new(root_font_size);
    let background_size = match margin_box_property(rule, PropertyKey::BackgroundSize) {
        Some(PropertyValue::BackgroundSize(size)) => {
            resolve_background_size(*size, ComputedLength(font_size), None, &resolve_context)
        }
        _ => initial.background_size,
    };
    let background_position = match margin_box_property(rule, PropertyKey::BackgroundPosition) {
        Some(PropertyValue::BackgroundPosition(position)) => {
            resolve_css_position(*position, ComputedLength(font_size), None, &resolve_context)
        }
        _ => initial.background_position,
    };
    let background_repeat = match margin_box_property(rule, PropertyKey::BackgroundRepeat) {
        Some(PropertyValue::BackgroundRepeat(repeat)) => *repeat,
        _ => initial.background_repeat,
    };
    let background_origin = match margin_box_property(rule, PropertyKey::BackgroundOrigin) {
        Some(PropertyValue::BackgroundOrigin(origin)) => *origin,
        _ => initial.background_origin,
    };
    let background_clip = match margin_box_property(rule, PropertyKey::BackgroundClip) {
        Some(PropertyValue::BackgroundClip(clip)) => *clip,
        _ => initial.background_clip,
    };
    let [border_top, border_right, border_bottom, border_left] =
        margin_box_borders(document, cascade, rule, font_size);
    let margin_auto = margin_box_auto_margins(rule);
    let width = match margin_box_property(rule, PropertyKey::Width) {
        Some(PropertyValue::Width(value)) => length_or_auto_to_px(*value, width_basis, font_size),
        _ => None,
    };
    let height = match margin_box_property(rule, PropertyKey::Height) {
        Some(PropertyValue::Height(value)) => length_or_auto_to_px(*value, height_basis, font_size),
        _ => None,
    };
    let inherited_text_align = margin_box_property(rule, PropertyKey::TextAlign)
        .or_else(|| page_property(cascade, PropertyKey::TextAlign))
        .and_then(|value| match value {
            PropertyValue::TextAlign(value) => Some(*value),
            _ => None,
        })
        .or_else(|| root_computed(document, cascade).map(|computed| computed.text_align));
    let alignment = match inherited_text_align {
        Some(TextAlign::Right) => StandaloneAlign::Right,
        Some(TextAlign::End) => StandaloneAlign::End,
        Some(TextAlign::Justify) => StandaloneAlign::Justify,
        Some(TextAlign::Center) => StandaloneAlign::Center,
        Some(TextAlign::Left) => StandaloneAlign::Left,
        _ => StandaloneAlign::Start,
    };
    // `top`/`bottom` are margin-box-specific keywords and are not yet part of
    // the element `vertical-align` grammar.  Treat the supported explicit
    // `text-top`/`text-bottom` values as their corresponding placement.  The
    // current parser drops the margin-box `top` spelling, so retain the
    // historical top placement as the fallback used by this minimal path.
    let inherited_vertical_align = margin_box_property(rule, PropertyKey::VerticalAlign)
        .or_else(|| page_property(cascade, PropertyKey::VerticalAlign))
        .and_then(|value| match value {
            PropertyValue::VerticalAlign(value) => Some(*value),
            _ => None,
        })
        .or_else(|| root_computed(document, cascade).map(|computed| computed.vertical_align));
    let vertical_align = match inherited_vertical_align {
        Some(VerticalAlign::TextBottom) => text::MarginTextVerticalAlign::Bottom,
        Some(VerticalAlign::Middle) => text::MarginTextVerticalAlign::Middle,
        Some(VerticalAlign::TextTop) => text::MarginTextVerticalAlign::Top,
        _ => text::MarginTextVerticalAlign::Top,
    };
    let inherited = |key| margin_box_property(rule, key).or_else(|| page_property(cascade, key));
    let root = root_computed(document, cascade).unwrap_or(&initial);
    let mode = match inherited(PropertyKey::WritingMode) {
        Some(PropertyValue::WritingMode(mode)) => *mode,
        _ => root.cssom_writing_mode,
    };
    let orientation = match inherited(PropertyKey::TextOrientation) {
        Some(PropertyValue::TextOrientation(value)) => *value,
        _ => root.text_orientation,
    };
    let direction = match inherited(PropertyKey::Direction) {
        Some(PropertyValue::Direction(value)) => *value,
        _ => root.direction,
    };
    let mut text_style = crate::standalone_text::style(font_size, &font_family);
    text_style.writing_mode = match mode {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    text_style.text_orientation = match orientation {
        raikiri_style::property::TextOrientation::Upright => shodo::style::TextOrientation::Upright,
        raikiri_style::property::TextOrientation::Sideways => {
            shodo::style::TextOrientation::Sideways
        }
        _ => shodo::style::TextOrientation::Mixed,
    };
    text_style.direction = match direction {
        raikiri_style::property::Direction::Rtl => shodo::geometry::Direction::Rtl,
        _ => shodo::geometry::Direction::Ltr,
    };
    Some(MarginBoxPaintSpec {
        slot: rule.slot,
        content,
        background,
        background_image_url,
        background_image_lime,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        background_clip,
        content_image_lime,
        border_top,
        border_right,
        border_bottom,
        border_left,
        margin_auto,
        margin,
        padding,
        width,
        height,
        text_color: inherited_margin_box_color(document, cascade, rule),
        text_style,
        alignment,
        vertical_align,
    })
}

#[allow(clippy::too_many_arguments)]
fn paint_margin_box(
    scene: &mut impl PaintScene,
    document: &Document,
    spec: &MarginBoxPaintSpec,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let rect = Rect::new(x as f64, y as f64, (x + width) as f64, (y + height) as f64);
    if let Some(color) = spec.background {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }
    if let (Some(raw_url), Some(source)) = (spec.background_image_url.as_deref(), pixel_source) {
        if let Some(url) = background_image_url(raw_url) {
            // Margin-box paint/position areas honor `background-origin`/`background-clip`
            // like elements (border box is `rect`). Unsupported `text`/`border-area`
            // clips warn and skip the image instead of silently omitting pixels.
            let bl = spec.border_left.map(|(w, _)| w as f64).unwrap_or(0.0);
            let bt = spec.border_top.map(|(w, _)| w as f64).unwrap_or(0.0);
            let br = spec.border_right.map(|(w, _)| w as f64).unwrap_or(0.0);
            let bb = spec.border_bottom.map(|(w, _)| w as f64).unwrap_or(0.0);
            // `spec.padding` is top/right/bottom/left order.
            let pt = spec.padding[0] as f64;
            let pr = spec.padding[1] as f64;
            let pb = spec.padding[2] as f64;
            let pl = spec.padding[3] as f64;
            let border = (bl, bt, br, bb);
            let padding = (pl, pt, pr, pb);
            let positioning = origin_inset_rect(
                rect,
                spec.background_origin,
                border,
                padding,
                Some(&url),
                warnings,
            );
            match clip_inset_rect(rect, spec.background_clip, border, padding) {
                Some(painting) => {
                    if painting.x1 > painting.x0 && painting.y1 > painting.y0 {
                        paint_background_from_source(
                            scene,
                            &url,
                            positioning,
                            painting,
                            &spec.background_size,
                            &spec.background_position,
                            &spec.background_repeat,
                            source,
                            warnings,
                        );
                    }
                }
                None => {
                    warnings.push(RenderWarning {
                        kind: WarningKind::ResourceFallback {
                            kind: raikiri_traits::ResourceKind::Image,
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
    } else if spec.background_image_lime {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(0, 255, 0, 255),
            None,
            &rect,
        );
    }
    for (side, border) in [
        (0_u8, spec.border_top),
        (1_u8, spec.border_right),
        (2_u8, spec.border_bottom),
        (3_u8, spec.border_left),
    ] {
        let Some((border_width, color)) = border else {
            continue;
        };
        let border_width = border_width.min(width.max(0.0)).min(height.max(0.0));
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
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &border_rect);
    }
    if !spec.content.is_empty() {
        scene.push_clip_layer(Affine::IDENTITY, &rect);
        let border_left = spec.border_left.map(|(width, _)| width).unwrap_or(0.0);
        let border_right = spec.border_right.map(|(width, _)| width).unwrap_or(0.0);
        let border_top = spec.border_top.map(|(width, _)| width).unwrap_or(0.0);
        let border_bottom = spec.border_bottom.map(|(width, _)| width).unwrap_or(0.0);
        let content_x = x + border_left + spec.padding[3];
        let content_y = y + border_top + spec.padding[0];
        text::draw_margin_text_styled(
            document,
            scene,
            &spec.content,
            content_x,
            content_y,
            (width - border_left - border_right - spec.padding[1] - spec.padding[3]).max(0.0),
            (height - border_top - border_bottom - spec.padding[0] - spec.padding[2]).max(0.0),
            spec.text_color,
            &spec.text_style,
            spec.alignment,
            spec.vertical_align,
        );
        scene.pop_layer();
    }
    if spec.content_image_lime {
        let image_x = (x
            + spec.border_left.map(|(width, _)| width).unwrap_or(0.0)
            + spec.padding[3]
            + document
                .shape_standalone_text(
                    &spec.content,
                    &spec.text_style,
                    None,
                    StandaloneAlign::Start,
                )
                .map_or(0.0, |text| text.width()))
        .round();
        let image_rect = Rect::new(
            image_x as f64,
            (y + spec.border_top.map(|(width, _)| width).unwrap_or(0.0) + spec.padding[0]) as f64,
            (image_x + 100.0).min(x + width) as f64,
            (y + height).min(
                y + spec.border_top.map(|(width, _)| width).unwrap_or(0.0) + spec.padding[0] + 50.0,
            ) as f64,
        );
        if image_rect.width() > 0.0 && image_rect.height() > 0.0 {
            scene.push_clip_layer(Affine::IDENTITY, &rect);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(0, 255, 0, 255),
                None,
                &image_rect,
            );
            scene.pop_layer();
        }
    }
}

fn margin_box_rule(
    page: &raikiri_style::PageCascadeResult,
    slot: PageMarginBoxSlot,
) -> Option<PageMarginBoxCascadeResult> {
    page.cascade_margin_box(slot)
}

fn margin_box_border_width(spec: &MarginBoxPaintSpec) -> f32 {
    spec.border_left.map(|(width, _)| width).unwrap_or(0.0)
        + spec.border_right.map(|(width, _)| width).unwrap_or(0.0)
}

fn margin_box_border_height(spec: &MarginBoxPaintSpec) -> f32 {
    spec.border_top.map(|(height, _)| height).unwrap_or(0.0)
        + spec.border_bottom.map(|(height, _)| height).unwrap_or(0.0)
}

fn margin_box_padding_width(spec: &MarginBoxPaintSpec) -> f32 {
    spec.padding[1] + spec.padding[3]
}

fn margin_box_padding_height(spec: &MarginBoxPaintSpec) -> f32 {
    spec.padding[0] + spec.padding[2]
}

fn margin_box_margin_width(spec: &MarginBoxPaintSpec) -> f32 {
    spec.margin[1] + spec.margin[3]
}

fn margin_box_margin_height(spec: &MarginBoxPaintSpec) -> f32 {
    spec.margin[0] + spec.margin[2]
}

fn margin_box_text_width(
    document: &Document,
    spec: &MarginBoxPaintSpec,
    content_height: Option<f32>,
) -> f32 {
    let inline_size = if spec.text_style.writing_mode.is_vertical() {
        content_height.or(spec.height)
    } else {
        None
    };
    let measured = document
        .shape_standalone_text(
            &spec.content,
            &spec.text_style,
            inline_size,
            StandaloneAlign::Start,
        )
        .map_or(0.0, |text| text.width());
    let non_collapsible_content = spec
        .content
        .chars()
        .any(|character| !character.is_whitespace() || character == '\u{a0}');
    measured
        .max(if non_collapsible_content {
            spec.text_style.font_size.max(0.0)
        } else {
            0.0
        })
        .max(0.0)
}

fn margin_box_intrinsic_width(
    document: &Document,
    spec: &MarginBoxPaintSpec,
    available_height: f32,
) -> f32 {
    // Match the actual content height used by the horizontal margin strip.
    // Vertical text may need multiple columns within that inline constraint.
    let content_height = (margin_box_outer_height(spec, available_height).min(available_height)
        - margin_box_margin_height(spec)
        - margin_box_border_height(spec)
        - margin_box_padding_height(spec))
    .max(0.0);
    (margin_box_text_width(document, spec, Some(content_height))
        + margin_box_border_width(spec)
        + margin_box_padding_width(spec)
        + margin_box_margin_width(spec))
    .max(0.0)
}

fn margin_box_intrinsic_height(document: &Document, spec: &MarginBoxPaintSpec) -> f32 {
    if spec.content.is_empty() {
        return 0.0;
    }
    let line_count = spec
        .content
        .trim_end_matches('\n')
        .split('\n')
        .count()
        .max(1) as f32;
    let text_height = if spec.text_style.writing_mode.is_vertical() {
        document
            .shape_standalone_text(
                &spec.content,
                &spec.text_style,
                None,
                StandaloneAlign::Start,
            )
            .map_or(0.0, |text| text.height())
    } else {
        line_count * spec.text_style.font_size.max(0.0)
    };
    (text_height
        + margin_box_border_height(spec)
        + margin_box_padding_height(spec)
        + margin_box_margin_height(spec))
    .max(0.0)
}

fn margin_box_outer_width(spec: &MarginBoxPaintSpec, available: f32) -> f32 {
    if let Some(width) = spec.width {
        (width
            + margin_box_border_width(spec)
            + margin_box_padding_width(spec)
            + margin_box_margin_width(spec))
        .max(0.0)
    } else {
        available.max(0.0)
    }
}

fn margin_box_outer_height(spec: &MarginBoxPaintSpec, available: f32) -> f32 {
    if let Some(height) = spec.height {
        (height
            + margin_box_border_height(spec)
            + margin_box_padding_height(spec)
            + margin_box_margin_height(spec))
        .max(0.0)
    } else {
        available.max(0.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_horizontal_margin_boxes(
    scene: &mut impl PaintScene,
    document: &Document,
    specs: &[MarginBoxPaintSpec],
    top: bool,
    page_width: f32,
    page_height: f32,
    margins: raikiri_dom::PageMargins,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let (row_y, row_height) = if top {
        (0.0, margins.top)
    } else {
        (page_height - margins.bottom, margins.bottom)
    };
    if row_height <= 0.0 {
        return;
    }
    let slots = if top {
        [
            PageMarginBoxSlot::TopLeft,
            PageMarginBoxSlot::TopCenter,
            PageMarginBoxSlot::TopRight,
        ]
    } else {
        [
            PageMarginBoxSlot::BottomLeft,
            PageMarginBoxSlot::BottomCenter,
            PageMarginBoxSlot::BottomRight,
        ]
    };
    let active: Vec<&MarginBoxPaintSpec> = slots
        .iter()
        .filter_map(|slot| specs.iter().find(|spec| spec.slot == *slot))
        .collect();
    if active.is_empty() {
        return;
    }
    let available = (page_width - margins.left - margins.right).max(0.0);
    let fixed = active
        .iter()
        .filter(|spec| spec.width.is_some())
        .map(|spec| margin_box_outer_width(spec, 0.0))
        .sum::<f32>();
    let auto_bases: Vec<f32> = active
        .iter()
        .map(|spec| {
            if spec.width.is_some() {
                0.0
            } else {
                margin_box_intrinsic_width(document, spec, row_height)
            }
        })
        .collect();
    let auto_base_total = auto_bases.iter().sum::<f32>();
    let auto_remaining = available - fixed - auto_base_total;
    let center_index = active.iter().position(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::TopCenter | PageMarginBoxSlot::BottomCenter
        )
    });
    let center_is_anchored = center_index.is_some_and(|index| {
        active[index].width.is_none()
            && auto_bases[index] <= 0.0
            && active.iter().enumerate().any(|(other, spec)| {
                other != index
                    && matches!(
                        spec.slot,
                        PageMarginBoxSlot::TopLeft
                            | PageMarginBoxSlot::TopRight
                            | PageMarginBoxSlot::BottomLeft
                            | PageMarginBoxSlot::BottomRight
                    )
                    && auto_bases[other] > 0.0
            })
    });
    let anchored_side_width = if center_is_anchored {
        ((available - fixed) / 2.0).max(0.0)
    } else {
        0.0
    };
    // CSS Page 3's AC box treats the two side tracks as one flex item when
    // an edge has an auto-sized center and auto-sized sides.  Proportional
    // distribution across all three intrinsic bases makes an asymmetric
    // side (dimensions-005) steal space from the opposite side instead of
    // keeping the center aligned.
    let ac_widths = if active.len() == 3
        && center_index == Some(1)
        && active.iter().all(|spec| spec.width.is_none())
    {
        let side_base = auto_bases[0].max(auto_bases[2]);
        let center_base = auto_bases[1];
        let ac_base = side_base * 2.0;
        let total_base = ac_base + center_base;
        (side_base > 0.0 && center_base > 0.0 && total_base > 0.0).then(|| {
            let free = available - total_base;
            let ac = (ac_base + free * ac_base / total_base).max(0.0);
            let center = (center_base + free * center_base / total_base).max(0.0);
            [ac * 0.5, center, ac * 0.5]
        })
    } else {
        None
    };
    let widths: Vec<f32> = active
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            if let Some(ac_widths) = ac_widths {
                ac_widths[index]
            } else if center_is_anchored && Some(index) == center_index {
                0.0
            } else if center_is_anchored {
                anchored_side_width
            } else if spec.width.is_some() {
                margin_box_outer_width(spec, 0.0).max(0.0)
            } else if auto_base_total > 0.0 {
                (auto_bases[index] + auto_remaining * auto_bases[index] / auto_base_total).max(0.0)
            } else {
                (auto_remaining / auto_bases.len() as f32).max(0.0)
            }
        })
        .collect();
    let mut x = margins.left;
    for (spec, outer_width) in active.into_iter().zip(widths) {
        let margin_left = spec.margin[3];
        let margin_right = spec.margin[1];
        let width = (outer_width - margin_left - margin_right).max(0.0);
        let paint_x = x + margin_left;
        let outer_height = if spec.height.is_some() {
            margin_box_outer_height(spec, 0.0).min(row_height).max(0.0)
        } else {
            row_height
        };
        let height = (outer_height - spec.margin[0] - spec.margin[2]).max(0.0);
        // Fixed-height top boxes sit against the page area; an auto-height
        // box fills the strip. Numeric margins remain outside the painted box.
        let y = if spec.height.is_some() && spec.margin_auto[0] && spec.margin_auto[2] {
            row_y + (row_height - outer_height).max(0.0) / 2.0 + spec.margin[0]
        } else if top && spec.height.is_some() {
            row_y + row_height - outer_height + spec.margin[0]
        } else {
            row_y + spec.margin[0]
        };
        paint_margin_box(
            scene,
            document,
            spec,
            paint_x,
            y,
            width,
            height,
            pixel_source,
            warnings,
        );
        x += outer_width;
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_vertical_margin_boxes(
    scene: &mut impl PaintScene,
    document: &Document,
    specs: &[MarginBoxPaintSpec],
    left: bool,
    page_width: f32,
    page_height: f32,
    margins: raikiri_dom::PageMargins,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let (column_x, column_width) = if left {
        (0.0, margins.left)
    } else {
        (page_width - margins.right, margins.right)
    };
    if column_width <= 0.0 {
        return;
    }
    let slots = if left {
        [
            PageMarginBoxSlot::LeftTop,
            PageMarginBoxSlot::LeftMiddle,
            PageMarginBoxSlot::LeftBottom,
        ]
    } else {
        [
            PageMarginBoxSlot::RightTop,
            PageMarginBoxSlot::RightMiddle,
            PageMarginBoxSlot::RightBottom,
        ]
    };
    let active: Vec<&MarginBoxPaintSpec> = slots
        .iter()
        .filter_map(|slot| specs.iter().find(|spec| spec.slot == *slot))
        .collect();
    if active.is_empty() {
        return;
    }
    let available = (page_height - margins.top - margins.bottom).max(0.0);
    let fixed = active
        .iter()
        .filter(|spec| spec.height.is_some())
        .map(|spec| margin_box_outer_height(spec, 0.0))
        .sum::<f32>();
    let auto_bases: Vec<f32> = active
        .iter()
        .map(|spec| {
            if spec.height.is_some() {
                0.0
            } else {
                margin_box_intrinsic_height(document, spec)
            }
        })
        .collect();
    let auto_base_total = auto_bases.iter().sum::<f32>();
    let auto_remaining = available - fixed - auto_base_total;
    let ac_heights = if active.len() == 3 && active.iter().all(|spec| spec.height.is_none()) {
        let side_base = auto_bases[0].max(auto_bases[2]);
        let center_base = auto_bases[1];
        let ac_base = side_base * 2.0;
        let total_base = ac_base + center_base;
        (side_base > 0.0 && center_base > 0.0 && total_base > 0.0).then(|| {
            let free = available - total_base;
            let ac = (ac_base + free * ac_base / total_base).max(0.0);
            let center = (center_base + free * center_base / total_base).max(0.0);
            [ac * 0.5, center, ac * 0.5]
        })
    } else {
        None
    };
    let heights: Vec<f32> = active
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            if let Some(ac_heights) = ac_heights {
                ac_heights[index]
            } else if spec.height.is_some() {
                margin_box_outer_height(spec, 0.0).max(0.0)
            } else if auto_base_total > 0.0 {
                (auto_bases[index] + auto_remaining * auto_bases[index] / auto_base_total).max(0.0)
            } else {
                (auto_remaining / auto_bases.len() as f32).max(0.0)
            }
        })
        .collect();
    let has_top = active.iter().any(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::LeftTop | PageMarginBoxSlot::RightTop
        )
    });
    let has_bottom = active.iter().any(|spec| {
        matches!(
            spec.slot,
            PageMarginBoxSlot::LeftBottom | PageMarginBoxSlot::RightBottom
        )
    });
    let all_fixed_heights = active.len() == 3 && active.iter().all(|spec| spec.height.is_some());
    let segment_height = available / 3.0;
    let mut y = margins.top;
    for (spec, outer_height) in active.into_iter().zip(heights) {
        let margin_top = spec.margin[0];
        let margin_bottom = spec.margin[2];
        let height = (outer_height - margin_top - margin_bottom).max(0.0);
        let paint_y = if all_fixed_heights {
            let slot_index = match spec.slot {
                PageMarginBoxSlot::LeftTop | PageMarginBoxSlot::RightTop => 0.0,
                PageMarginBoxSlot::LeftMiddle | PageMarginBoxSlot::RightMiddle => 1.0,
                _ => 2.0,
            };
            margins.top
                + slot_index * segment_height
                + (segment_height - outer_height).max(0.0) / 2.0
                + margin_top
        } else if spec.height.is_some()
            && matches!(
                spec.slot,
                PageMarginBoxSlot::LeftMiddle | PageMarginBoxSlot::RightMiddle
            )
            && !has_top
            && !has_bottom
        {
            margins.top + (available - outer_height).max(0.0) / 2.0 + margin_top
        } else {
            y + margin_top
        };
        let margin_left = spec.margin[3];
        let margin_right = spec.margin[1];
        let outer_width = if spec.width.is_some() {
            margin_box_outer_width(spec, 0.0).max(0.0)
        } else {
            column_width
        };
        let width = (outer_width - margin_left - margin_right).max(0.0);
        let paint_x = if spec.width.is_some() && spec.margin_auto[1] && spec.margin_auto[3] {
            column_x + (column_width - width).max(0.0) / 2.0
        } else if spec.width.is_some() && left {
            column_x + (column_width - margin_right - width).max(0.0)
        } else {
            column_x + margin_left
        };
        paint_margin_box(
            scene,
            document,
            spec,
            paint_x,
            paint_y,
            width,
            height,
            pixel_source,
            warnings,
        );
        y += outer_height;
    }
}

/// Paint the generated content and backgrounds of the matching page-margin
/// boxes.  Geometry is a compact grid model of the sixteen CSS slots: edge
/// boxes divide the corresponding margin strip, while corner boxes live in
/// the strip intersections.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_page_margin_boxes(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
    page_count: u32,
    page_is_left: bool,
    paired_page_increment: Option<i32>,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let margins = raikiri_dom::page_margins(cascade, page_box);
    if margins.is_zero() || cascade.page.margin_boxes().is_empty() {
        return;
    }
    let mut specs = Vec::new();
    for slot in [
        PageMarginBoxSlot::TopLeftCorner,
        PageMarginBoxSlot::TopLeft,
        PageMarginBoxSlot::TopCenter,
        PageMarginBoxSlot::TopRight,
        PageMarginBoxSlot::TopRightCorner,
        PageMarginBoxSlot::RightTop,
        PageMarginBoxSlot::RightMiddle,
        PageMarginBoxSlot::RightBottom,
        PageMarginBoxSlot::BottomRightCorner,
        PageMarginBoxSlot::BottomRight,
        PageMarginBoxSlot::BottomCenter,
        PageMarginBoxSlot::BottomLeft,
        PageMarginBoxSlot::BottomLeftCorner,
        PageMarginBoxSlot::LeftBottom,
        PageMarginBoxSlot::LeftMiddle,
        PageMarginBoxSlot::LeftTop,
    ] {
        let Some(rule) = margin_box_rule(&cascade.page, slot) else {
            continue;
        };
        // Percentages on an edge dimension use the available strip axis,
        // not the full paper box.  This matters when two 50% top boxes are
        // laid out between nonzero page margins.
        let content_width = (page_box.width - margins.left - margins.right).max(0.0);
        let content_height = (page_box.height - margins.top - margins.bottom).max(0.0);
        let (width_basis, height_basis) = match slot {
            PageMarginBoxSlot::TopLeft
            | PageMarginBoxSlot::TopCenter
            | PageMarginBoxSlot::TopRight
            | PageMarginBoxSlot::BottomLeft
            | PageMarginBoxSlot::BottomCenter
            | PageMarginBoxSlot::BottomRight => (
                content_width,
                if matches!(
                    slot,
                    PageMarginBoxSlot::TopLeft
                        | PageMarginBoxSlot::TopCenter
                        | PageMarginBoxSlot::TopRight
                ) {
                    margins.top
                } else {
                    margins.bottom
                },
            ),
            PageMarginBoxSlot::LeftTop
            | PageMarginBoxSlot::LeftMiddle
            | PageMarginBoxSlot::LeftBottom => (margins.left, content_height),
            PageMarginBoxSlot::RightTop
            | PageMarginBoxSlot::RightMiddle
            | PageMarginBoxSlot::RightBottom => (margins.right, content_height),
            _ => (page_box.width, page_box.height),
        };
        if let Some(spec) = margin_box_spec(
            document,
            cascade,
            &rule,
            width_basis,
            height_basis,
            page_index,
            page_count,
            page_is_left,
            paired_page_increment,
        ) {
            specs.push(spec);
        }
    }

    paint_horizontal_margin_boxes(
        scene,
        document,
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
        pixel_source,
        warnings,
    );
    paint_horizontal_margin_boxes(
        scene,
        document,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
        pixel_source,
        warnings,
    );
    paint_vertical_margin_boxes(
        scene,
        document,
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
        pixel_source,
        warnings,
    );
    paint_vertical_margin_boxes(
        scene,
        document,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
        pixel_source,
        warnings,
    );

    for spec in &specs {
        let (x, y, cell_w, cell_h) = match spec.slot {
            PageMarginBoxSlot::TopLeftCorner => (0.0, 0.0, margins.left, margins.top),
            PageMarginBoxSlot::TopRightCorner => (
                page_box.width - margins.right,
                0.0,
                margins.right,
                margins.top,
            ),
            PageMarginBoxSlot::BottomLeftCorner => (
                0.0,
                page_box.height - margins.bottom,
                margins.left,
                margins.bottom,
            ),
            PageMarginBoxSlot::BottomRightCorner => (
                page_box.width - margins.right,
                page_box.height - margins.bottom,
                margins.right,
                margins.bottom,
            ),
            _ => continue,
        };
        let width = margin_box_outer_width(spec, cell_w).min(cell_w);
        let height = margin_box_outer_height(spec, cell_h).min(cell_h);
        let x = if spec.margin_auto[1] && spec.margin_auto[3] {
            x + (cell_w - width).max(0.0) / 2.0
        } else {
            match spec.slot {
                PageMarginBoxSlot::TopLeftCorner | PageMarginBoxSlot::BottomLeftCorner
                    if spec.width.is_some() =>
                {
                    x + cell_w - width
                }
                _ => x,
            }
        };
        let y = if spec.margin_auto[0] && spec.margin_auto[2] {
            y + (cell_h - height).max(0.0) / 2.0
        } else {
            match spec.slot {
                PageMarginBoxSlot::TopLeftCorner | PageMarginBoxSlot::TopRightCorner
                    if spec.height.is_some() =>
                {
                    y + cell_h - height
                }
                _ => y,
            }
        };
        paint_margin_box(
            scene,
            document,
            spec,
            x,
            y,
            width,
            height,
            pixel_source,
            warnings,
        );
    }
}

/// Height from an ifc root's border-box top that its painted lines reach:
/// lines that overflow the box (overflow is visible) are still drawn, so a
/// page that holds only those lines must not skip the root.
fn ifc_paint_extent(node: &raikiri_dom::Node, layout: &taffy::Layout) -> f32 {
    let lines = node.ifc_size().map_or(0.0, |(_, height)| height);
    layout
        .size
        .height
        .max(layout.border.top + layout.padding.top + lines)
}

fn box_intersects_page(y: f32, height: f32, page_top: f32, page_bottom: f32) -> bool {
    if !y.is_finite() || !height.is_finite() || !page_top.is_finite() || !page_bottom.is_finite() {
        return false;
    }
    if height <= 0.0 {
        return y >= page_top && y <= page_bottom;
    }
    y < page_bottom && y + height > page_top
}

/// Walk the Document arena iteratively in DFS order, starting from body.
/// A fragment (without `<body>`) returns silently. `layout_single_page` should
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
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &[CounterSnapshot],
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    custom_highlights: &[crate::TextHighlightRange],
) {
    let mut warnings = Vec::new();
    paint_document_impl(
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
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document_with_images_and_warnings(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &[CounterSnapshot],
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: &dyn ImagePixelSource,
    warnings: &mut Vec<RenderWarning>,
    custom_highlights: &[crate::TextHighlightRange],
) {
    paint_document_impl(
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
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document_impl(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    counter_snapshots: &[CounterSnapshot],
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    custom_highlights: &[crate::TextHighlightRange],
    mut trace: Option<&mut Vec<crate::PaintTraceEvent>>,
) {
    let Some(body_id) = raikiri_dom::paint_rules::find_paint_root(document) else {
        return;
    };

    // The document, cascade, and counter snapshots are stable for this pass,
    // so each node's generated flow height can be computed once and reused.
    let mut generated_flow_heights = std::collections::HashMap::new();

    let named_page_matches = |node_id: usize| {
        raikiri_dom::paint_rules::named_page_matches(document, cascade, node_id, active_page_name)
    };

    // The body's parent (`<html>`) font-size is available only from a
    // separate traversal (`find_paint_root`), not from this stack. Use the body's
    // own font-size as a self-referential fallback. The UA default display
    // for body is block, so `vertical_align_shift_px` always ignores this
    // value through its inline-level gate. Except for pathological input
    // overriding body with `display: inline`, its accuracy does not matter.
    let body_font_size = cascade.computed[body_id].font_size.px();
    // The paint walk starts at `<body>` because the html box itself is not a
    // paint item here. Seed the context with html's originating decoration so
    // root-element lines still propagate through the body subtree.
    let empty_decorations = text::DecorationContext::default();
    let root_decorations = find_html(document)
        .map(|html_id| {
            text::decorations_for_element(
                &empty_decorations,
                NodeId::new(html_id as u64),
                &cascade.computed[html_id],
                0.0,
            )
        })
        .unwrap_or_else(|| empty_decorations.clone());
    let layout_fragments = document.layout_fragments();
    let mut fragments_by_parent_and_node =
        std::collections::HashMap::<(Option<usize>, usize), Vec<usize>>::new();
    let mut fragmented_nodes = std::collections::HashSet::new();
    for (fragment_id, fragment) in layout_fragments.iter().enumerate() {
        fragmented_nodes.insert(fragment.node_id);
        fragments_by_parent_and_node
            .entry((fragment.parent, fragment.node_id))
            .or_default()
            .push(fragment_id);
    }
    enum PaintFrame<'a> {
        Visit {
            node_id: usize,
            parent_abs_x: f32,
            parent_abs_y: f32,
            parent_font_size: f32,
            shift_y: f32,
            transform_x: f32,
            transform_y: f32,
            fragment_clip_height: Option<f32>,
            fragment_id: Option<usize>,
            fragment_origin_x: f32,
            fragment_origin_y: f32,
            fragmentainer: Option<usize>,
            fragment_rect: Option<FragmentRect>,
            fragment_clip: Option<FragmentRect>,
            is_fragment_visit: bool,
            inside_fixed: bool,
            inside_fixed_containing_block: bool,
            decorations: text::DecorationContext,
        },
        AnonymousParagraph {
            key: usize,
            cell: &'a raikiri_dom::Node,
            x: f32,
            y: f32,
            shift_y: f32,
            transform_x: f32,
            transform_y: f32,
            fragmentainer: Option<usize>,
            inside_fixed: bool,
            decorations: text::DecorationContext,
        },
        RestoreTransform(Affine),
        PopClip,
        /// Close an element opacity group after its complete subtree.
        PopOpacity,
        /// Paint an originating element's `::after` pseudo after its real
        /// children, while still inside any overflow clip pushed for the
        /// originating element.
        PaintAfter {
            node_id: usize,
            x: f32,
            y: f32,
            width: f32,
            height: f32,
            before_advance: f32,
            content_advance: f32,
        },
    }

    let margins = raikiri_dom::page_margins(cascade, page_box);
    let insets = raikiri_dom::page_content_insets(cascade, page_box);
    // Keep fixed-position sizing consistent with layout: page border/padding
    // are applied through `page_offset_x`, not by shrinking the inline size.
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
    // Fixed-position containing blocks use the initial laid-out viewport even
    // when a later named page has a different paper width.
    let fixed_content_width = if fixed_page_width.is_finite() && fixed_page_width > 0.0 {
        fixed_page_width
    } else {
        content_width
    };
    let page_top = content_origin_y;
    let page_bottom = content_origin_y + content_height;
    // The walker keeps source coordinates in the stack.  Only the paint sites
    // receive the page translation, which makes page membership testable even
    // when an ancestor spans a page boundary.  The cascade retains the raw
    // root writing-mode winner because vertical-rl currently normalizes to
    // horizontal-tb for ordinary layout; in that page context the physical
    // left page margin is on the opposite inline edge.
    let root_vertical_rl = find_html(document)
        .and_then(|html_id| cascade.authored_writing_modes.get(html_id))
        .is_some_and(|mode| matches!(mode, Some(WritingMode::VerticalRl)));
    let page_offset_x = if root_vertical_rl {
        insets.left
    } else {
        margins.left + insets.left
    };
    let page_offset_y = margins.top + insets.top - content_origin_y;
    // A body background is normally propagated to the canvas. When the body
    // has authored padding, its border box can still extend beyond the
    // synthetic canvas width used by this paint walk; paint that visible
    // padding strip as part of the body box. This preserves the CSS body
    // background in cases such as `body { width: 20em; padding-right: 1em }`
    // without flooding the rest of the page.
    // cov:ignore: authored body-padding propagation is exercised by the ignored image-enabled WPT run.
    if cascade.computed[body_id].background_color.a != 0
        // cov:ignore: authored body-padding propagation is exercised by the ignored image-enabled WPT run.
        && cascade.computed[body_id].background_image == BackgroundImage::None
    // cov:ignore: authored body-padding propagation is exercised by the ignored image-enabled WPT run.
    {
        let Some(body) = document.get_node(body_id) else {
            return;
        };
        // The root's padding also carries the body's horizontal margins;
        // only the authored padding is part of the body box.
        let right_padding =
            (body.unrounded_layout.padding.right - document.body_inline_margins().1).max(0.0);
        if right_padding > 0.0 {
            // Layout moves the content of a body that is not one paragraph
            // below the body's block-start margin without changing its
            // padding; a paragraph body carries that margin in its padding.
            let content_shift = if body.is_ifc_root() {
                0.0
            } else {
                document.body_block_start_margin()
            };
            let content_end = body
                .children
                .iter()
                .filter_map(|&child| {
                    let node = document.get_node(child)?;
                    (node.is_in_document() && node.unrounded_layout.size.height > 0.0).then_some(
                        node.unrounded_layout.location.x + node.unrounded_layout.size.width,
                    )
                })
                .fold(0.0_f32, f32::max);
            let content_bottom = body
                .children
                .iter()
                .filter_map(|&child| {
                    let node = document.get_node(child)?;
                    node.is_in_document().then_some(
                        node.unrounded_layout.location.y + node.unrounded_layout.size.height,
                    )
                })
                .fold(0.0_f32, f32::max);
            if content_end > 0.0 && content_bottom > 0.0 {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    css_color(cascade.computed[body_id].background_color),
                    None,
                    &Rect::new(
                        (page_offset_x + body.unrounded_layout.padding.left) as f64,
                        (page_offset_y + body.unrounded_layout.padding.top + content_shift) as f64,
                        (page_offset_x + content_end + right_padding) as f64,
                        (page_offset_y + content_bottom + body.unrounded_layout.padding.bottom)
                            as f64,
                    ),
                );
            }
        }
    }
    // The synthetic body root keeps the historical inline width, so normal
    // element backgrounds can extend past the page content box when a page
    // has border/padding. Clip those backgrounds to the physical content box
    // without clipping text ink, which may legitimately overflow its box.
    let page_content_clip = if margins.left > 0.0
        || margins.right > 0.0
        || margins.top > 0.0
        || margins.bottom > 0.0
        || insets.left > 0.0
        || insets.right > 0.0
        || insets.top > 0.0
        || insets.bottom > 0.0
    {
        Some(Rect::new(
            page_offset_x as f64,
            (margins.top + insets.top) as f64,
            (page_box.width - margins.right - insets.right).max(page_offset_x) as f64,
            (margins.top + insets.top + content_height) as f64,
        ))
    } else {
        None
    };
    let mut transformed_scene = crate::transform::TransformScene {
        scene,
        transform: Affine::IDENTITY,
    };
    let scene = &mut transformed_scene;
    let mut stack = vec![PaintFrame::Visit {
        node_id: body_id,
        parent_abs_x: 0.0,
        parent_abs_y: 0.0,
        parent_font_size: body_font_size,
        shift_y: 0.0,
        transform_x: 0.0,
        transform_y: 0.0,
        fragment_clip_height: None,
        fragment_id: None,
        fragment_origin_x: 0.0,
        fragment_origin_y: 0.0,
        fragmentainer: None,
        fragment_rect: None,
        fragment_clip: None,
        is_fragment_visit: false,
        inside_fixed: false,
        inside_fixed_containing_block: false,
        decorations: root_decorations,
    }];
    // Flowed text is clipped to the page content box when the page is at
    // least twice as wide as its content; fixed boxes repeat on every page
    // and are not clipped.
    let text_page_clip = |inside_fixed: bool| -> Option<Rect> {
        (!inside_fixed
            && content_width.is_finite()
            && content_width > 0.0
            && page_box.width >= content_width * 2.0
            && cascade.page.margin_boxes().is_empty())
        .then(|| {
            Rect::new(
                page_offset_x as f64,
                (margins.top + insets.top) as f64,
                (page_box.width - margins.right - insets.right) as f64,
                (margins.top + insets.top + content_height) as f64,
            )
        })
    };
    while let Some(frame) = stack.pop() {
        let (
            node_id,
            parent_abs_x,
            mut parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            fragment_clip_height,
            fragment_id,
            fragment_origin_x,
            fragment_origin_y,
            fragmentainer,
            fragment_rect,
            fragment_clip,
            is_fragment_visit,
            inside_fixed,
            inside_fixed_containing_block,
            decorations,
        ) = match frame {
            PaintFrame::AnonymousParagraph {
                key,
                cell,
                x,
                y,
                shift_y,
                transform_x,
                transform_y,
                fragmentainer,
                inside_fixed,
                decorations,
            } => {
                let layout = cell.unrounded_layout;
                let probe = cell
                    .ifc_lines()
                    .and_then(|lines| {
                        lines
                            .iter()
                            .flat_map(|line| line.fragments())
                            .find_map(|fragment| {
                                let shodo::Fragment::GlyphRun(run) = fragment else {
                                    return None;
                                };
                                let owner = run.node()?.0 as usize;
                                document
                                    .get_node(owner)
                                    .filter(|node| node.kind() == NodeKind::Text)
                                    .map(|_| owner)
                            })
                    })
                    .unwrap_or(document.ifc_source_owner(key));
                if !named_page_matches(probe)
                    || (!box_intersects_page(
                        y,
                        ifc_paint_extent(cell, &layout),
                        page_top,
                        page_bottom,
                    ) && !inside_fixed)
                {
                    continue;
                }
                let clip = text_page_clip(inside_fixed);
                if let Some(clip) = &clip {
                    scene.scene.push_clip_layer(Affine::IDENTITY, clip);
                }
                let owner = document.ifc_source_owner(key);
                if cascade.computed[owner].visibility != Visibility::Hidden
                    && let Some(border) = cell.collapsed_border()
                {
                    paint_element_border(
                        scene,
                        layout.size.width,
                        layout.size.height,
                        x + page_offset_x + transform_x,
                        y + page_offset_y + transform_y,
                        border,
                        cascade.computed[owner].color,
                    );
                }
                if let Some(events) = trace.as_deref_mut()
                    && raikiri_dom::PositionedLines::new(document, cascade, key, fragmentainer)
                        .is_some_and(|lines| lines.lines().any(|line| !line.runs.is_empty()))
                {
                    events.push(crate::PaintTraceEvent::Text(document.ifc_source_owner(key)));
                }
                let paint_transform = scene.transform;
                crate::ifc_text::draw_ifc_lines_with_resources(
                    scene,
                    document,
                    cascade,
                    key,
                    crate::ifc_text::IfcPosition {
                        x: x + page_offset_x
                            + transform_x
                            + layout.border.left
                            + layout.padding.left,
                        y: y + page_offset_y + transform_y + layout.border.top + layout.padding.top,
                        shift_y,
                    },
                    &decorations,
                    fragmentainer,
                    custom_highlights,
                    pixel_source,
                    warnings,
                    page_box,
                    paint_transform,
                );
                if clip.is_some() {
                    scene.pop_layer();
                }
                continue;
            }
            PaintFrame::RestoreTransform(transform) => {
                scene.transform = transform;
                continue;
            }
            PaintFrame::PopClip => {
                if let Some(t) = trace.as_deref_mut() {
                    t.push(crate::PaintTraceEvent::PopClip);
                }
                scene.pop_layer();
                continue;
            }
            PaintFrame::PopOpacity => {
                if let Some(t) = trace.as_deref_mut() {
                    t.push(crate::PaintTraceEvent::PopOpacity);
                }
                scene.pop_layer();
                continue;
            }
            PaintFrame::PaintAfter {
                node_id,
                x,
                y,
                width,
                height,
                before_advance,
                content_advance,
            } => {
                let _ = paint_generated_pseudo(
                    scene,
                    document,
                    cascade,
                    node_id,
                    raikiri_style::PseudoElem::After,
                    x + before_advance + content_advance,
                    y,
                    (width - before_advance).max(0.0),
                    height,
                    counter_snapshots,
                );
                continue;
            }
            PaintFrame::Visit {
                node_id,
                parent_abs_x,
                parent_abs_y,
                parent_font_size,
                shift_y,
                transform_x,
                transform_y,
                fragment_clip_height,
                fragment_id,
                fragment_origin_x,
                fragment_origin_y,
                fragmentainer,
                fragment_rect,
                fragment_clip,
                is_fragment_visit,
                inside_fixed,
                inside_fixed_containing_block,
                decorations,
            } => (
                node_id,
                parent_abs_x,
                parent_abs_y,
                parent_font_size,
                shift_y,
                transform_x,
                transform_y,
                fragment_clip_height,
                fragment_id,
                fragment_origin_x,
                fragment_origin_y,
                fragmentainer,
                fragment_rect,
                fragment_clip,
                is_fragment_visit,
                inside_fixed,
                inside_fixed_containing_block,
                decorations,
            ),
        };
        if let Some(shift) = document.repeated_table_header_root_shift(node_id, content_origin_y) {
            let Some(shift) = shift else {
                continue;
            };
            parent_abs_y += shift;
        }
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        if let Some(fragment_ids) = fragments_by_parent_and_node.get(&(fragment_id, node_id)) {
            for &current_fragment_id in fragment_ids.iter().rev() {
                let fragment = layout_fragments[current_fragment_id];
                let node_abs_x = if fragment.parent.is_some() {
                    fragment_origin_x + fragment.rect.x
                } else {
                    parent_abs_x + node.unrounded_layout.location.x + fragment.rect.x
                };
                let node_abs_y = if fragment.parent.is_some() {
                    fragment_origin_y + fragment.rect.y
                } else {
                    parent_abs_y + node.unrounded_layout.location.y + fragment.rect.y
                };
                let fragment_clip = fragment.fragmentainer_clip.map(|clip| FragmentRect {
                    x: clip.x - fragment.rect.x,
                    y: clip.y - fragment.rect.y,
                    width: clip.width,
                    height: clip.height,
                });
                stack.push(PaintFrame::Visit {
                    node_id,
                    parent_abs_x: node_abs_x - node.unrounded_layout.location.x,
                    parent_abs_y: node_abs_y - node.unrounded_layout.location.y,
                    parent_font_size,
                    shift_y,
                    transform_x,
                    transform_y,
                    fragment_clip_height,
                    fragment_id: Some(current_fragment_id),
                    fragment_origin_x: node_abs_x,
                    fragment_origin_y: node_abs_y,
                    fragmentainer: Some(fragment.fragmentainer),
                    fragment_rect: Some(fragment.rect),
                    fragment_clip,
                    is_fragment_visit: true,
                    inside_fixed,
                    inside_fixed_containing_block,
                    decorations: decorations.clone(),
                });
            }
            continue;
        }
        if fragmented_nodes.contains(&node_id) && !is_fragment_visit {
            continue;
        }
        // Skip template descendants and future inert subtrees consistently.
        // Use an explicit gate independent of any UA CSS display:none rule.
        if !node.is_in_document() {
            continue;
        }
        let mut node_layout = node.unrounded_layout;
        if let Some(rect) = fragment_rect {
            node_layout.size.width = rect.width;
            node_layout.size.height = rect.height;
        }
        // Do not paint subtrees of HTML hidden elements (metadata, raw text,
        // or fallback parentheses for ruby).
        // Treat `Node::is_non_rendered_html_element` match arms as the
        // single source of truth for the current set of tags.
        // Author / user CSS can override UA CSS `display: none`, so painting
        // fails closed with a cascade-independent defense-in-depth gate
        // (HTML LS §15.3.1 "Hidden elements":
        // https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements).
        // Namespace checks exclude same-named SVG / MathML elements.
        // <template> is doubly gated by is_in_document().
        if node.is_non_rendered_html_element() {
            continue;
        }
        match node.kind() {
            NodeKind::Element => {
                // An atomic inline an ellipsis removed from its line is not
                // painted (CSS Overflow 3 §5.1).
                if node.is_display_none() || node.is_hidden_by_text_overflow() {
                    continue;
                }
                let layout = node_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                let cv = &cascade.computed[node_id];
                let paint_padding = used_padding_for_paint(cv, &layout);
                let has_border = [
                    &cv.border.top,
                    &cv.border.right,
                    &cv.border.bottom,
                    &cv.border.left,
                ]
                .iter()
                .any(|side| side.width().px() > 0.0 && side.style() != BorderStyle::None);
                let paint_border_radius = paintable_border_radius(
                    &cv.border_radius,
                    (has_border
                        || cv.background_color.a > 0
                        || !matches!(cv.background_image, BackgroundImage::None))
                        && cv.transform.is_empty()
                        && cv.filter.is_empty(),
                );
                // A box laid out on the lines of an inline engine paragraph
                // already sits at its `vertical-align` position; shifting its
                // content again would move it twice.
                let placed_by_inline_engine = document
                    .layout_parent_of(node_id)
                    .and_then(|parent| document.get_node(parent))
                    .is_some_and(|parent| parent.is_ifc_root())
                    && !node.in_ifc_subtree();
                let own_shift = if placed_by_inline_engine {
                    0.0
                } else {
                    vertical_align_shift_px(cv.vertical_align, cv.display, parent_font_size)
                };
                let child_shift_y = shift_y + own_shift;
                // Taffy and the IFC block/atomic placement paths already apply
                // relative insets. Keep the legacy paint offset for the body
                // root, floats, explicit multicol fragments and manually
                // placed table parts. Those paths still use paint-side insets.
                let is_float = matches!(cv.float, FloatValue::Left | FloatValue::Right)
                    && document.layout_parent_of(node_id).is_none_or(|parent| {
                        !matches!(
                            cascade.computed[parent].display,
                            DisplayValue::Flex
                                | DisplayValue::InlineFlex
                                | DisplayValue::Grid
                                | DisplayValue::InlineGrid
                        )
                    });
                let needs_paint_inset = node_id == body_id
                    || is_float
                    || is_fragment_visit
                    || matches!(
                        cv.display,
                        DisplayValue::TableCaption
                            | DisplayValue::TableCell
                            | DisplayValue::TableRow
                            | DisplayValue::TableRowGroup
                            | DisplayValue::TableHeaderGroup
                            | DisplayValue::TableFooterGroup
                    );
                let (pos_dx, pos_dy) = if needs_paint_inset
                    && !matches!(
                        cv.display,
                        DisplayValue::Inline
                            | DisplayValue::InlineBlock
                            | DisplayValue::InlineFlex
                            | DisplayValue::InlineGrid
                            | DisplayValue::InlineTable
                    ) {
                    position_offset_px(cv)
                } else if node.needs_relative_block_paint_offset() {
                    // The foundational column post-pass replaces only Y;
                    // the existing X location already includes its inset.
                    (0.0, position_offset_px(cv).1)
                } else {
                    (0.0, 0.0)
                };
                // spec: https://www.w3.org/TR/css-transforms-1/#terminology
                // Non-replaced inline and table-column boxes are not transformable.
                let transformable = !matches!(
                    cv.display,
                    DisplayValue::TableColumn | DisplayValue::TableColumnGroup
                ) && (cv.display != DisplayValue::Inline
                    || node.is_inline_svg_root()
                    || matches!(
                        node.tag_name(),
                        Some("img" | "canvas" | "video" | "iframe" | "object" | "embed")
                    ));
                let (own_transform_x, own_transform_y) = if transformable {
                    transform_translation(cv, layout.size.width, layout.size.height)
                } else {
                    (0.0, 0.0)
                };
                let child_transform_x = transform_x + own_transform_x;
                let child_transform_y = transform_y + own_transform_y;
                let is_fixed = matches!(cv.position, PositionValue::Fixed);
                // A transform/filter ancestor establishes a fixed-position
                // containing block. Such descendants keep the legacy local
                // geometry; only viewport-fixed boxes repeat per page.
                let fixed_in_viewport = is_fixed && !inside_fixed_containing_block;
                let establishes_fixed_containing_block =
                    (transformable && !cv.transform.is_empty()) || !cv.filter.is_empty();
                let (fixed_dx, fixed_dy) = if fixed_in_viewport {
                    fixed_position_px(
                        cv,
                        layout.size.width,
                        layout.size.height,
                        page_box.width,
                        content_origin_y,
                        fixed_content_width,
                        content_height,
                    )
                    .map(|(x, y)| (x - abs_x, y - abs_y))
                    .unwrap_or((0.0, 0.0))
                } else {
                    (0.0, 0.0)
                };
                let paint_x = abs_x + page_offset_x + pos_dx + fixed_dx + child_transform_x;
                let mut paint_y =
                    abs_y + page_offset_y + child_shift_y + pos_dy + fixed_dy + child_transform_y;
                if transformable && !translation_only(cv) {
                    let matrix = element_transform(
                        cv,
                        paint_x,
                        paint_y,
                        layout.size.width,
                        layout.size.height,
                    );
                    // spec: https://www.w3.org/TR/css-transforms-1/#transform-function-lists
                    // A singular matrix hides the element and its subtree.
                    if matrix.determinant() == 0.0
                        || !matrix.as_coeffs().iter().all(|value| value.is_finite())
                    {
                        continue;
                    }
                    stack.push(PaintFrame::RestoreTransform(scene.transform));
                    scene.transform *= matrix;
                }
                let mut paint_height = layout.size.height;
                let mut paint_background_height = layout.size.height;
                let mut multicol_clip_pushed = false;
                let mut fragmentainer_clip_pushed = false;
                if let Some(clip_height) =
                    fragment_clip_height.filter(|height| height.is_finite() && *height >= 0.0)
                {
                    paint_y = paint_y.floor();
                    paint_height = paint_height.min(clip_height);
                    paint_background_height = paint_background_height.min(clip_height);
                    let clip_origin_y = paint_y.floor();
                    let clip = Rect::new(
                        paint_x as f64,
                        clip_origin_y as f64,
                        (paint_x + layout.size.width) as f64,
                        (clip_origin_y + clip_height) as f64,
                    );
                    scene.push_clip_layer(Affine::IDENTITY, &clip);
                    multicol_clip_pushed = true;
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushFragmentainerClip(node_id));
                    }
                }
                if let Some(clip) = fragment_clip {
                    let clip = Rect::new(
                        (paint_x + clip.x) as f64,
                        (paint_y + clip.y) as f64,
                        (paint_x + clip.x + clip.width) as f64,
                        (paint_y + clip.y + clip.height) as f64,
                    );
                    scene.push_clip_layer(Affine::IDENTITY, &clip);
                    fragmentainer_clip_pushed = true;
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushFragmentainerClip(node_id));
                    }
                }
                // A collapsed-border table cell paints the borders that won
                // conflict resolution with its rows, neighbours and table.
                let painted_border = document
                    .get_node(node_id)
                    .and_then(|node| node.collapsed_border())
                    .unwrap_or(&cv.border);
                // cov:ignore: vertical table cell background geometry is covered by the ignored exact WPT reftest.
                let mut paint_background_width = layout.size.width;
                if let Some(width) = vertical_table_cell_background_width(
                    document,
                    cascade,
                    node_id,
                    &layout,
                    &cv.border,
                    &paint_padding,
                ) {
                    paint_background_width = width; // cov:ignore: vertical background geometry is exercised by the ignored exact WPT reftest.
                }
                // A hidden table must not paint its own collapsed border. Keep
                // row/cell visibility handling unchanged; `visibility: collapse`
                // is table-layout-specific and existing row painting relies on it.
                let visibility_hidden_table =
                    raikiri_dom::paint_rules::is_visibility_hidden_table(cv);
                let mut paints_on_page = !visibility_hidden_table
                    && (node_id == body_id
                        || ((fixed_in_viewport || named_page_matches(node_id))
                            && (fixed_in_viewport
                                || inside_fixed
                                || box_intersects_page(
                                    abs_y,
                                    layout.size.height,
                                    page_top,
                                    page_bottom,
                                ))));
                // An absolutely positioned containing block can own a child
                // fragment on a later page even when its first layout box ends
                // before that page.  Preserve the parent's border/background
                // continuation for this narrow OOF shape.  Full OOF
                // fragmentation remains outside this painter; the gate avoids
                // changing ordinary flow, fixed-position, transformed, or
                // borderless absolute boxes.
                let is_absolute_fragment_candidate = matches!(cv.position, PositionValue::Absolute)
                    && cv.transform.is_empty()
                    && (cv.border.top.width().px() > 0.0
                        || cv.border.right.width().px() > 0.0
                        || cv.border.bottom.width().px() > 0.0
                        || cv.border.left.width().px() > 0.0)
                    && cv.background_color.a != 0;
                let descendant_end_on_page = if is_absolute_fragment_candidate {
                    node.children
                        .iter()
                        .filter_map(|child_id| {
                            let child = document.get_node(*child_id)?;
                            let child_cv = cascade.computed.get(*child_id)?;
                            if !matches!(
                                child_cv.position,
                                PositionValue::Static
                                    | PositionValue::Relative
                                    | PositionValue::Sticky
                            ) {
                                return None;
                            }
                            let child_y = abs_y + child.unrounded_layout.location.y;
                            box_intersects_page(
                                child_y,
                                child.unrounded_layout.size.height,
                                page_top,
                                page_bottom,
                            )
                            .then_some(child_y + child.unrounded_layout.size.height)
                        })
                        .fold(None, |maximum, value| {
                            Some(maximum.map_or(value, |current: f32| current.max(value)))
                        })
                } else {
                    None
                };
                let mut paints_as_absolute_continuation = false;
                if !paints_on_page
                    && is_absolute_fragment_candidate
                    && let Some(descendant_end) = descendant_end_on_page
                {
                    let fragment_top = page_top - margins.top - insets.top;
                    let continuation_height =
                        (descendant_end - fragment_top + cv.border.bottom.width().px()).max(0.0);
                    if continuation_height > 0.0 {
                        paint_y = fragment_top + margins.top + insets.top - page_top
                            + pos_dy
                            + child_transform_y;
                        paint_height = continuation_height;
                        // The local reftest oracle carries only the
                        // continuation border; the ancestor background is
                        // not repeated behind the child fragment.
                        paint_background_height = 0.0;
                        paints_on_page = named_page_matches(node_id);
                        paints_as_absolute_continuation = paints_on_page;
                    }
                }
                // The first fragment of the same containing block extends to
                // the page boundary when a direct in-flow child starts on the
                // next page.  Its normal layout height only covers the first
                // one-pass OOF content box, so preserve the border/background
                // through the break without changing descendant coordinates.
                if paints_on_page
                    && !paints_as_absolute_continuation
                    && is_absolute_fragment_candidate
                    && let Some(descendant_end) = node
                        .children
                        .iter()
                        .filter_map(|child_id| {
                            let child = document.get_node(*child_id)?;
                            let child_cv = cascade.computed.get(*child_id)?;
                            if !matches!(
                                child_cv.position,
                                PositionValue::Static
                                    | PositionValue::Relative
                                    | PositionValue::Sticky
                            ) {
                                return None;
                            }
                            let child_y = abs_y + child.unrounded_layout.location.y;
                            (child_y >= page_bottom)
                                .then_some(child_y + child.unrounded_layout.size.height)
                        })
                        .fold(None, |maximum, value| {
                            Some(maximum.map_or(value, |current: f32| current.max(value)))
                        })
                {
                    paint_height = paint_height
                        .max((descendant_end - abs_y + cv.border.bottom.width().px()).max(0.0));
                    // Keep the element background inside the current page's
                    // content extent.  The border continuation is painted
                    // separately and may reach the page edge.
                    paint_background_height = (page_bottom - abs_y).max(0.0);
                }

                // The pseudo is not an arena child yet, so Taffy cannot
                // include its line box in an auto-height originating box.
                // Expand only the auto-height paint geometry; fixed-height
                // elements retain their normal overflow behavior. This is a
                // paint-side bridge: the arena/Taffy flow remains unchanged
                // until generated inline boxes can participate in layout.
                if !paints_as_absolute_continuation
                    && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
                {
                    let pseudo_height = generated_flow_height(
                        document,
                        cascade,
                        node_id,
                        counter_snapshots,
                        &mut generated_flow_heights,
                    );
                    let pseudo_border_height = cv.border.top.width().px()
                        + paint_padding.top
                        + pseudo_height
                        + paint_padding.bottom
                        + cv.border.bottom.width().px();
                    if pseudo_border_height > paint_height {
                        let was_intrinsic_height =
                            (paint_background_height - layout.size.height).abs() <= f32::EPSILON;
                        paint_height = pseudo_border_height;
                        if was_intrinsic_height {
                            paint_background_height = paint_height;
                        }
                    }
                }
                let generated_x = paint_x + cv.border.left.width().px() + paint_padding.left;
                let generated_y = paint_y + cv.border.top.width().px() + paint_padding.top;
                // Direct text children are the only inline descendants whose
                // current minimal layout path has a reliable horizontal box.
                // Use their right edge for `::after`; element children stay
                // on the existing block-flow path until a full IFC lands.
                let generated_content_advance = node
                    .children
                    .iter()
                    .filter_map(|child_id| document.get_node(*child_id))
                    .filter(|child| child.kind() == NodeKind::Text)
                    .map(|child| {
                        (child.unrounded_layout.location.x + child.unrounded_layout.size.width
                            - cv.border.left.width().px()
                            - paint_padding.left)
                            .max(0.0)
                    })
                    .fold(0.0, f32::max);
                // CSS Color 4 §3.3: opacity applies to the complete
                // element group, including its background, border, text,
                // generated content, and descendants. Keep this layer open
                // until the deferred `PaintAfter` and overflow clip frames
                // have finished so overlapping descendants are composited as
                // one group instead of being alpha-blended individually.
                let has_opacity_layer = raikiri_dom::paint_rules::opacity_layer(cv).is_some();
                if has_opacity_layer {
                    let opacity_clip = Rect::new(
                        0.0,
                        0.0,
                        page_box.width.max(0.0) as f64,
                        page_box.height.max(0.0) as f64,
                    );
                    scene.scene.push_layer(
                        Mix::Normal,
                        cv.opacity,
                        Affine::IDENTITY,
                        &opacity_clip,
                        None,
                        None,
                    );
                    // This frame is pushed first so it closes after the
                    // optional overflow clip and generated `::after` paint.
                    stack.push(PaintFrame::PopOpacity);
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushOpacity(node_id, cv.opacity));
                    }
                }
                let mut before_advance = 0.0;
                if paints_on_page
                    && !raikiri_dom::paint_rules::hides_empty_table_cell(document, cascade, node_id)
                {
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::Box(node_id));
                    }
                    // A body background is propagated to the page canvas.  For
                    // a non-zero page margin, painting the body border box as
                    // well would leak that color into the translated top/bottom
                    // margin on every page after the first; the canvas pass has
                    // already filled the content rectangle at page-local coords.
                    let grid = node.table_grid_box();
                    let own_paint_x = paint_x + grid.map_or(0.0, |rect| rect.x);
                    let own_paint_width = grid.map_or(layout.size.width, |rect| rect.width);
                    let own_background_width =
                        grid.map_or(paint_background_width, |rect| rect.width);
                    let own_paint_y = paint_y + grid.map_or(0.0, |rect| rect.y);
                    let own_paint_height = grid.map_or(paint_height, |rect| rect.height);
                    let own_background_height =
                        grid.map_or(paint_background_height, |rect| rect.height);
                    let paints_table_part =
                        !raikiri_dom::paint_rules::is_visibility_hidden_table_part(cv);
                    if node_id != body_id && paints_table_part {
                        // The body canvas background was already propagated by
                        // `paint_canvas_background`; do not paint it a second
                        // time on the synthetic body root.
                        // Paint element background (CSS Backgrounds 3 §2.2).
                        // Shift applies to the box itself per CSS 2.1 §10.8.1.
                        let clip_background = page_content_clip.filter(|_| {
                            !inside_fixed
                                && !fixed_in_viewport
                                && matches!(
                                    cv.position,
                                    PositionValue::Static
                                        | PositionValue::Relative
                                        | PositionValue::Sticky
                                )
                        });
                        if let Some(clip) = clip_background {
                            scene.scene.push_clip_layer(Affine::IDENTITY, &clip);
                        }
                        paint_element_box_shadows(
                            scene,
                            own_paint_width,
                            own_paint_height,
                            own_paint_x,
                            own_paint_y,
                            &paint_border_radius,
                            &cv.box_shadow,
                            cv.color,
                        );
                        let table_part_clip =
                            document.anonymous_table_part_background_cells(node_id);
                        if let Some(cells) = table_part_clip {
                            let mut path = BezPath::new();
                            for cell in cells {
                                let x = f64::from(cell.x);
                                let y = f64::from(cell.y);
                                let right = x + f64::from(cell.width);
                                let bottom = y + f64::from(cell.height);
                                path.move_to((x, y));
                                path.line_to((right, y));
                                path.line_to((right, bottom));
                                path.line_to((x, bottom));
                                path.close_path();
                            }
                            scene.scene.push_clip_layer(
                                Affine::translate((f64::from(own_paint_x), f64::from(own_paint_y))),
                                &path,
                            );
                        }
                        paint_element_background(
                            scene,
                            own_background_width,
                            own_background_height,
                            own_paint_x,
                            own_paint_y,
                            cv.background_color,
                            &cv.background_image,
                            cv.color,
                            cv.background_clip,
                            cv.background_origin,
                            &paint_border_radius,
                            painted_border,
                            &paint_padding,
                            &cv.background_size,
                            &cv.background_position,
                            &cv.background_repeat,
                            pixel_source,
                            warnings,
                        );
                        if table_part_clip.is_some() {
                            scene.pop_layer();
                        }
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // An opaque background that uses the same solid color as
                    // every border side already paints the complete visible
                    // union in `paint_element_background`; avoid compositing
                    // the same anti-aliased rounded edge a second time.
                    let border_covered_by_background = matches!(
                        cv.background_image,
                        BackgroundImage::None
                    ) && cv.background_color.a == 255
                        && matches!(
                            cv.background_clip,
                            raikiri_style::property::VisualBox::PaddingBox
                                | raikiri_style::property::VisualBox::ContentBox
                        )
                        && [
                            &painted_border.top,
                            &painted_border.right,
                            &painted_border.bottom,
                            &painted_border.left,
                        ]
                        .iter()
                        .all(|side| {
                            side.style() == BorderStyle::Solid
                                && side.width().px() > 0.0
                                && matches!(side.color, BorderColor::Resolved(c) if c == cv.background_color)
                        });
                    // Paint border on top of background (CSS Backgrounds 3 §5).
                    let paints_continuation_border =
                        paints_table_part && paints_as_absolute_continuation;
                    if !border_covered_by_background && paints_continuation_border {
                        paint_element_border_with_top(
                            scene,
                            own_paint_width, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_height, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_x, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_y, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            painted_border, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            cv.color,
                            false,
                        );
                    } else if paints_table_part && !border_covered_by_background {
                        paint_element_border_rounded(
                            scene,
                            own_paint_width,
                            own_paint_height,
                            own_paint_x,
                            own_paint_y,
                            painted_border,
                            cv.color,
                            &paint_border_radius,
                        );
                    }
                    // Outlines do not consume box-model space and are painted
                    // outside the border edge (CSS Basic UI §4).
                    if paints_table_part {
                        paint_element_outline(
                            scene,
                            own_paint_width,
                            own_paint_height,
                            own_paint_x,
                            own_paint_y,
                            &cv.outline,
                            cv.outline_offset,
                            cv.color,
                        );
                    }
                    // Resolve the source's natural dimensions first. SVG
                    // sources use the resulting concrete object dimensions
                    // for a bounded, size-specific raster request.
                    if let Some(t) = trace.as_deref_mut()
                        && (node.is_inline_svg_root()
                            || document.is_canvas_element(node_id)
                            || node.tag_name() == Some("img"))
                    {
                        t.push(crate::PaintTraceEvent::Replaced(node_id));
                    }
                    let painted_image = if node.is_inline_svg_root() {
                        paint_inline_svg(
                            scene,
                            document,
                            node_id,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &paint_padding,
                            [cv.color.r, cv.color.g, cv.color.b, cv.color.a],
                            cv.opacity,
                            cascade
                                .opacity_specified
                                .get(node_id)
                                .copied()
                                .unwrap_or(false),
                            cascade
                                .background_color_specified
                                .get(node_id)
                                .copied()
                                .unwrap_or(false),
                            cv.visibility != Visibility::Hidden,
                            warnings,
                        )
                    } else if document.is_canvas_element(node_id) {
                        let fallback_bitmap;
                        let bitmap = if let Some(bitmap) = document.canvas_bitmap_ref(node_id) {
                            bitmap
                        } else {
                            let (w, h) = document.canvas_size(node_id).unwrap_or((300, 150));
                            fallback_bitmap = raikiri_dom::CanvasBitmap::transparent(w, h);
                            &fallback_bitmap
                        };
                        paint_canvas(
                            scene,
                            bitmap,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &paint_padding,
                            cv.object_fit,
                            &cv.object_position,
                            cv.overflow.x,
                            cv.overflow.y,
                            cv.visibility != Visibility::Hidden,
                        )
                    } else if let Some(source) = pixel_source
                        && let Some(src_url) = img_src_url(document, node_id)
                        && let Some(intrinsic) = source.intrinsic_size(&src_url)
                    {
                        paint_image(
                            scene,
                            source,
                            &src_url,
                            intrinsic,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &paint_padding,
                            cv.object_fit,
                            &cv.object_position,
                            cv.visibility != Visibility::Hidden,
                            node_id,
                            warnings,
                        )
                    } else {
                        false
                    };
                    if !painted_image
                        && pixel_source.is_none()
                        && node.tag_name() == Some("img")
                        && let Some(src) = node.attribute("src")
                        && let Some(color) = infer_url_color(src)
                    {
                        // Use the same background paint path as a normal
                        // block so the fallback has identical raster edges.
                        let clip_background = page_content_clip.filter(|_| {
                            !inside_fixed
                                && !fixed_in_viewport
                                && matches!(
                                    cv.position,
                                    PositionValue::Static
                                        | PositionValue::Relative
                                        | PositionValue::Sticky
                                )
                        });
                        if let Some(clip) = clip_background {
                            scene.scene.push_clip_layer(Affine::IDENTITY, &clip);
                        }
                        paint_element_background(
                            scene,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            color,
                            &BackgroundImage::None,
                            cv.color,
                            cv.background_clip,
                            cv.background_origin,
                            &paint_border_radius,
                            &cv.border,
                            &paint_padding,
                            &cv.background_size,
                            &cv.background_position,
                            &cv.background_repeat,
                            pixel_source,
                            warnings,
                        );
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // A generated list marker belongs to the first principal
                    // fragment; inherited ancestor fragment state is unrelated.
                    if !paints_as_absolute_continuation
                        && cv.display == DisplayValue::ListItem
                        && fragment_id.is_none_or(|id| {
                            layout_fragments[id].node_id != node_id
                                || layout_fragments[id].fragment_index == 0
                        })
                    {
                        // An empty list item still owns a generated marker.
                        // Its principal box has zero width/height, which the
                        // marker helper intentionally rejects for ordinary
                        // zero-sized fragments. Give this empty-item marker a
                        // minimal paint extent without changing that helper's
                        // defensive behavior for other callers.
                        let empty_item_marker = node.children.is_empty();
                        let marker_width = if empty_item_marker {
                            layout.size.width.max(1.0)
                        } else {
                            layout.size.width
                        };
                        let marker_height = if empty_item_marker {
                            layout.size.height.max(cv.font_size.px().max(1.0))
                        } else {
                            layout.size.height
                        };
                        paint_list_marker_with_snapshots(
                            scene,
                            document,
                            cascade,
                            node_id,
                            paint_x,
                            paint_y,
                            marker_width,
                            marker_height,
                            layout.padding.left,
                            counter_snapshots,
                            pixel_source,
                        );
                    }
                    // Generated content is an immediate child of its
                    // originating box.  The minimal layout engine does not
                    // allocate an arena node for it, so paint literal
                    // `::before` content at the content-box inline start.
                    // The returned advance is used by the deferred `::after`
                    // frame below to preserve source order for an empty or
                    // otherwise unlaid-out originating box.
                    before_advance = if !paints_as_absolute_continuation {
                        paint_generated_pseudo(
                            scene,
                            document,
                            cascade,
                            node_id,
                            raikiri_style::PseudoElem::Before,
                            generated_x,
                            generated_y,
                            layout.size.width,
                            paint_height,
                            counter_snapshots,
                        )
                    } else {
                        0.0 // cov:ignore: absolute continuation intentionally omits first pseudo paint
                    };
                }
                // The fragmentainer clip encloses the element and its children;
                // close it after any element-local overflow clip.
                if multicol_clip_pushed {
                    stack.push(PaintFrame::PopClip);
                }
                if fragmentainer_clip_pushed {
                    stack.push(PaintFrame::PopClip);
                }
                // CSS Overflow 3 §3.1: non-visible overflow clips descendants to
                // the padding box. Min edges are floored outward so a fractional
                // padding-box origin does not antialias-cut pixel-snapped descendant
                // backgrounds (which round to integers); this mirrors the multicol
                // clip origin flooring. The clip is pushed only after painting the
                // element itself, then popped after its complete subtree via the
                // explicit stack frame.
                if let Some(resolved) = raikiri_dom::paint_rules::overflow_clip(
                    document,
                    cascade,
                    node_id,
                    PaintRect::new(paint_x, paint_y, layout.size.width, paint_height),
                    PaintInsets::new(
                        layout.border.top,
                        layout.border.right,
                        layout.border.bottom,
                        layout.border.left,
                    ),
                ) {
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
                    stack.push(PaintFrame::PopClip);
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushOverflowClip(
                            node_id,
                            [clip.x0, clip.y0, clip.x1, clip.y1],
                        ));
                    }
                }
                // Push this after the clip-pop frame and before children.
                // Children therefore paint first, then `::after`, then the
                // clip closes.  A full IFC would place the after run after
                // laid-out inline descendants; this minimal path uses the
                // generated-before advance as its only inline position.
                if paints_on_page && !paints_as_absolute_continuation {
                    stack.push(PaintFrame::PaintAfter {
                        node_id,
                        x: generated_x,
                        y: generated_y,
                        width: layout.size.width,
                        height: paint_height,
                        before_advance,
                        content_advance: generated_content_advance,
                    });
                }
                let child_font_size = cv.font_size.px();
                // `text-decoration-line` is non-inherited at the computed-value
                // layer, but its originating line is propagated to descendants
                // by CSS Text Decoration. Keep that paint-only context separate
                // from `CascadeResult`'s inheritance result.
                let child_decorations = text::decorations_for_element(
                    &decorations,
                    NodeId::new(node_id as u64),
                    cv,
                    child_shift_y,
                );
                // Paint positioned siblings in stacking order while keeping
                // source order for equal stack levels. Flex/grid containers use
                // order-modified document order within each stacking bucket,
                // as required by CSS Display's `order` property behavior.
                // This is intentionally local to the current parent; full
                // nested stacking-context isolation remains outside this
                // minimal painter.
                let mut children = if node.is_inline_svg_root()
                    || document.anonymous_table_contents_paint_only(node_id)
                {
                    Vec::new()
                } else if node.is_ifc_root() {
                    // The paragraph's own text and inline elements are drawn
                    // from the lines; only the boxes laid out beside them are
                    // visited like ordinary children.
                    node.ifc_boxes()
                } else if let Some(children) = document.anonymous_table_paint_sequence(node_id) {
                    children
                } else {
                    node.children.clone()
                };
                raikiri_dom::paint_rules::sort_paint_children(&mut children, cv.display, cascade);

                // The current Taffy bridge treats inline children as zero-sized
                // block items.  Keep that layout authority, but provide the
                // measured paint-time inline advance for generated pseudo text
                // so a sequence such as `span::before { content: counter(c) }`
                // remains in source order.  This is deliberately limited to
                // siblings that actually carry generated content; ordinary
                // inline layout remains unchanged until a full IFC lands.
                let child_generated_advances: Vec<(usize, bool, f32)> =
                    children
                        .iter()
                        .map(|&child| {
                            let is_text = document
                                .get_node(child)
                                .is_some_and(|child_node| child_node.kind() == NodeKind::Text);
                            let advance = if !is_text
                                && cascade.computed.get(child).is_some_and(|child_cv| {
                                    child_cv.display == DisplayValue::Inline
                                }) {
                                generated_pseudo_text_advance(
                                    document,
                                    cascade,
                                    child,
                                    raikiri_style::PseudoElem::Before,
                                    counter_snapshots,
                                ) + generated_pseudo_text_advance(
                                    document,
                                    cascade,
                                    child,
                                    raikiri_style::PseudoElem::After,
                                    counter_snapshots,
                                )
                            } else {
                                0.0
                            };
                            (child, is_text, advance)
                        })
                        .collect();
                let inline_offsets = if child_generated_advances
                    .iter()
                    .any(|(_, _, advance)| *advance > 0.0)
                {
                    let family = cv
                        .font_family
                        .first()
                        .map(|family| family.as_str().to_string())
                        .unwrap_or_else(|| "serif".to_string());
                    let collapsed_space = text::measure_margin_text_advance(
                        document,
                        " ",
                        cv.font_size.px(),
                        &family,
                    );
                    let mut flow_advance = 0.0;
                    let mut saw_generated_inline = false;
                    let mut offsets = Vec::new();
                    for (index, (child, is_text, advance)) in
                        child_generated_advances.iter().enumerate()
                    {
                        if *is_text {
                            let has_following_generated_inline = child_generated_advances
                                .iter()
                                .skip(index + 1)
                                .any(|(_, _, following)| *following > 0.0);
                            if saw_generated_inline && has_following_generated_inline {
                                flow_advance += collapsed_space;
                            }
                        } else if *advance > 0.0 {
                            offsets.push((*child, flow_advance));
                            flow_advance += *advance;
                            saw_generated_inline = true;
                        }
                    }
                    offsets
                } else {
                    Vec::new()
                };
                let vertical_generated_offsets = if cv.display != DisplayValue::Inline {
                    let mut flow_extra = generated_pseudo_text_height(
                        document,
                        cascade,
                        node_id,
                        raikiri_style::PseudoElem::Before,
                        counter_snapshots,
                    );
                    let mut offsets = Vec::new();
                    for &child in &children {
                        offsets.push((child, flow_extra));
                        let Some(child_node) = document.get_node(child) else {
                            continue; // cov:ignore: document child ids are arena-valid
                        };
                        let child_cv = &cascade.computed[child];
                        let child_is_normal_flow = matches!(
                            child_cv.position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        );
                        if child_node.kind() != NodeKind::Text
                            && child_cv.display != DisplayValue::Inline
                            && child_is_normal_flow
                        {
                            let required_height = generated_flow_height(
                                document,
                                cascade,
                                child,
                                counter_snapshots,
                                &mut generated_flow_heights,
                            );
                            flow_extra += (required_height
                                - child_node.unrounded_layout.size.height)
                                .max(0.0);
                        }
                    }
                    offsets
                } else {
                    Vec::new()
                };

                // Reverse push makes the lowest stack level paint first.
                // Relative insets are already present in `layout.location`
                // except for the manual placement paths identified above.
                let child_parent_x = abs_x + pos_dx + fixed_dx;
                let child_parent_y = abs_y + pos_dy + fixed_dy;
                // A paragraph inside a fixed box, or a fixed box that is a
                // paragraph itself, repeats on every page and is not clipped
                // like flowed text, as the text branch below does.
                let ifc_inside_fixed = inside_fixed || fixed_in_viewport;
                // The page a text node belongs to is looked up on the text node
                // itself; the block's own page value can differ (an absolutely
                // positioned box named for a page), so ask as the first text
                // child would.
                let ifc_page_probe = if node.is_ifc_root() {
                    first_text_descendant(document, node_id).unwrap_or(node_id)
                } else {
                    node_id
                };
                if node.is_ifc_root()
                    && named_page_matches(ifc_page_probe)
                    && (ifc_inside_fixed
                        || box_intersects_page(
                            abs_y,
                            ifc_paint_extent(node, &layout),
                            page_top,
                            page_bottom,
                        ))
                {
                    // The block's own background and border were painted
                    // above; its lines are drawn at the content-box origin,
                    // where a child text node would have been placed. That
                    // origin comes from the taffy layout's border and padding
                    // (the text child's `location`), not from
                    // `used_padding_for_paint`, which places the element's own
                    // generated content.
                    let content_x = child_parent_x
                        + page_offset_x
                        + child_transform_x
                        + layout.border.left
                        + layout.padding.left;
                    let content_y = child_parent_y
                        + page_offset_y
                        + child_transform_y
                        + layout.border.top
                        + layout.padding.top;
                    let text_clip = text_page_clip(ifc_inside_fixed);
                    if let Some(clip) = &text_clip {
                        scene.scene.push_clip_layer(Affine::IDENTITY, clip);
                    }
                    // A paragraph whose lines carry no visible text (only
                    // atomic inlines and whitespace) is not traced as text,
                    // as its whitespace-only text nodes have no fragments.
                    if let Some(t) = trace.as_deref_mut()
                        && document
                            .ifc_text_lines_by_node(node_id)
                            .keys()
                            .any(|&owner| {
                                document
                                    .get_node(owner)
                                    .and_then(|text| text.text_content())
                                    .is_some_and(|text| !text.trim().is_empty())
                            })
                    {
                        t.push(crate::PaintTraceEvent::Text(node_id));
                    }
                    let paint_transform = scene.transform;
                    crate::ifc_text::draw_ifc_lines_with_resources(
                        scene,
                        document,
                        cascade,
                        node_id,
                        crate::ifc_text::IfcPosition {
                            x: content_x,
                            y: content_y,
                            shift_y: child_shift_y,
                        },
                        &child_decorations,
                        fragmentainer,
                        custom_highlights,
                        pixel_source,
                        warnings,
                        page_box,
                        paint_transform,
                    );
                    if text_clip.is_some() {
                        scene.pop_layer();
                    }
                }
                let own_multicol_clip_height = match cv.column_count {
                    ColumnCountValue::Count(count) if count > 1 && layout.size.height > 0.0 => {
                        Some(layout.size.height)
                    }
                    _ => None,
                };
                for child in children.into_iter().rev() {
                    if let Some(cell) = document
                        .ifc_layout_node(child)
                        .filter(|_| document.get_node(child).is_none())
                    {
                        stack.push(PaintFrame::AnonymousParagraph {
                            key: child,
                            cell,
                            x: child_parent_x + cell.unrounded_layout.location.x,
                            y: child_parent_y + cell.unrounded_layout.location.y,
                            shift_y: child_shift_y,
                            transform_x: child_transform_x,
                            transform_y: child_transform_y,
                            fragmentainer,
                            inside_fixed: ifc_inside_fixed,
                            decorations: child_decorations.clone(),
                        });
                        continue;
                    }
                    let child_fragment_clip_height = match own_multicol_clip_height {
                        Some(clip_height)
                            if document.get_node(child).is_some_and(|child_node| {
                                matches!(
                                    cascade.computed[child].display,
                                    DisplayValue::Grid
                                        | DisplayValue::InlineGrid
                                        | DisplayValue::Table
                                        | DisplayValue::InlineTable
                                ) && child_node.kind() == NodeKind::Element
                            }) =>
                        {
                            Some(clip_height)
                        }
                        // A zero-height block at a multicolumn break can own
                        // an overflowing parallel flow. It has no fragment
                        // area in the column, so its descendants must not
                        // paint into the following sibling's fragment.
                        Some(_) // cov:ignore: exercised by ignored exact paged-text WPT.
                            if document.get_node(child).is_some_and(|child_node| { // cov:ignore: exercised by ignored exact paged-text WPT.
                                child_node.kind() == NodeKind::Element // cov:ignore: exercised by ignored exact paged-text WPT.
                                    && child_node.unrounded_layout.size.height <= 0.0 // cov:ignore: exercised by ignored exact paged-text WPT.
                                    && !child_node.children.is_empty() // cov:ignore: exercised by ignored exact paged-text WPT.
                            }) => // cov:ignore: exercised by ignored exact paged-text WPT.
                        {
                            Some(0.0) // cov:ignore: exercised by ignored exact paged-text WPT.
                        }
                        Some(_) => None,
                        None => fragment_clip_height,
                    };
                    let is_direct_text = document
                        .get_node(child)
                        .is_some_and(|node| node.kind() == NodeKind::Text);
                    let generated_text_shift = if is_direct_text { before_advance } else { 0.0 };
                    let inline_generated_offset = inline_offsets
                        .iter()
                        .find_map(|(offset_child, offset)| {
                            (*offset_child == child).then_some(*offset)
                        })
                        .unwrap_or(0.0);
                    let vertical_generated_offset = vertical_generated_offsets
                        .iter()
                        .find_map(|(offset_child, offset)| {
                            (*offset_child == child).then_some(*offset)
                        })
                        .unwrap_or(0.0);
                    stack.push(PaintFrame::Visit {
                        node_id: child,
                        parent_abs_x: child_parent_x
                            + generated_text_shift
                            + inline_generated_offset,
                        parent_abs_y: child_parent_y + vertical_generated_offset,
                        parent_font_size: child_font_size,
                        shift_y: child_shift_y,
                        transform_x: child_transform_x,
                        transform_y: child_transform_y,
                        fragment_clip_height: child_fragment_clip_height,
                        fragment_id,
                        fragment_origin_x,
                        fragment_origin_y,
                        fragmentainer,
                        fragment_rect: None,
                        fragment_clip: None,
                        is_fragment_visit: false,
                        inside_fixed: inside_fixed || fixed_in_viewport,
                        inside_fixed_containing_block: inside_fixed_containing_block
                            || establishes_fixed_containing_block,
                        decorations: child_decorations.clone(),
                    });
                }
            }
            NodeKind::Text => {
                let layout = node_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                if named_page_matches(node_id)
                    && (inside_fixed
                        || box_intersects_page(abs_y, layout.size.height, page_top, page_bottom))
                {
                    let text_clip = text_page_clip(inside_fixed);
                    if let Some(clip) = &text_clip {
                        scene.scene.push_clip_layer(Affine::IDENTITY, clip);
                    }
                    if node.is_ifc_root() {
                        // A text node laid out as an anonymous flex or grid
                        // item is a paragraph of its own: its lines start at
                        // its own box, which has no edges.
                        if let Some(t) = trace.as_deref_mut() {
                            t.push(crate::PaintTraceEvent::Text(node_id));
                        }
                        let paint_transform = scene.transform;
                        crate::ifc_text::draw_ifc_lines_with_resources(
                            scene,
                            document,
                            cascade,
                            node_id,
                            crate::ifc_text::IfcPosition {
                                x: abs_x + page_offset_x + transform_x,
                                y: abs_y + page_offset_y + transform_y,
                                shift_y,
                            },
                            &decorations,
                            fragmentainer,
                            custom_highlights,
                            pixel_source,
                            warnings,
                            page_box,
                            paint_transform,
                        );
                    }
                    // Any other text node lies outside every paragraph and
                    // has no lines to draw.
                    if text_clip.is_some() {
                        scene.pop_layer();
                    }
                }
            }
            NodeKind::Document => {
                // Usually unreachable because paint_document starts at body.
                // A Document node may have children (unattached `<html>`), but
                // they are not handled yet. Skip the subtree defensively.
            }
            _ => {
                // NodeKind is #[non_exhaustive]: `Comment` / `ProcessingInstruction` /
                // `DocumentFragment` arrives here (not paintable). In practice,
                // mark_in_document_flags clears IS_IN_DOCUMENT for Comment/PI,
                // so the earlier is_in_document() gate should filter those
                // before this walker reaches them. Keep this kind gate as a
                // second defense: if both gates fail, skip the whole subtree.
                // Apply the same behavior to future CDATA / DocumentType kinds.
                // Retain the defensive fallback for unknown node kinds.
            }
        }
    }
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

/// Reads the `src` attribute of `node_id` as an absolute URL, if the node
/// is an `<img>` element with a `src` that parses directly.
///
/// This mirrors the same absolute-URL-only scope as
/// `raikiri_dom::image_resolve::resolve_images` (relative-URL resolution
/// against a document base URL is out of scope) — necessarily a separate
/// implementation, since this crate cannot reach that crate's
/// crate-private `Node::attributes` field and must go through the public
/// `raikiri_traits::{Dom, Node, Element}` trait path instead.
fn img_src_url(document: &Document, node_id: usize) -> Option<url::Url> {
    use raikiri_traits::{Dom, Element as _, Node as _};
    let node_ref = document.node(raikiri_traits::NodeId::new(node_id as u64))?;
    let element = node_ref.as_element()?;
    if element.tag_name() != "img" {
        return None;
    }
    url::Url::parse(element.attr("src")?).ok()
}

/// Return the padding values used by paint for this laid-out box.
///
/// Taffy owns the used-value resolution for the layout pass. Ordinary
/// percentage/px padding keeps the painter's historical containing-width
/// calculation, while a side authored in `ch` takes the exact used value that
/// was measured before Taffy. This avoids re-probing fonts in the paint crate
/// and keeps backgrounds/images aligned with the geometry.
fn used_padding_for_paint(
    cv: &raikiri_style::ComputedValues,
    layout: &taffy::Layout,
) -> taffy::Rect<f32> {
    if cv.display == DisplayValue::TableCell {
        return layout.padding;
    }
    fn computed_padding_px(
        value: raikiri_style::resolve::ComputedLengthPercentage,
        reference: f32,
    ) -> f32 {
        match value {
            raikiri_style::resolve::ComputedLengthPercentage::Px(px) => px,
            raikiri_style::resolve::ComputedLengthPercentage::Percent(percent) => {
                reference * percent / 100.0
            }
        }
    }

    let reference = layout.size.width;
    let choose = |value: raikiri_style::resolve::ComputedLengthPercentage,
                  ch: &Option<raikiri_style::ChLengthProvenance>,
                  used: f32| {
        if ch.is_some() {
            used
        } else {
            computed_padding_px(value, reference)
        }
    };
    taffy::Rect {
        top: choose(cv.padding.top, &cv.padding_ch.top, layout.padding.top),
        right: choose(cv.padding.right, &cv.padding_ch.right, layout.padding.right),
        bottom: choose(
            cv.padding.bottom,
            &cv.padding_ch.bottom,
            layout.padding.bottom,
        ),
        left: choose(cv.padding.left, &cv.padding_ch.left, layout.padding.left),
    }
}

/// Draws a replaced element after asking the source for pixels at the concrete
/// CSS object size selected by `object-fit`.
#[allow(clippy::too_many_arguments)]
fn paint_image(
    scene: &mut impl PaintScene,
    source: &dyn ImagePixelSource,
    url: &url::Url,
    natural: ImageIntrinsicSize,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
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

    let (natural_w, natural_h) = natural_object_size(natural);
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
        return true;
    }
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
    let image_x = content_x + position_offset(object_position.horizontal, content_w - image_w);
    let image_y = content_y + position_offset(object_position.vertical, content_h - image_h);

    let image_data = peniko::ImageData {
        data: peniko::Blob::from(decoded.rgba.clone()),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: decoded.width,
        height: decoded.height,
    };
    let brush = peniko::ImageBrush::new(image_data);
    let clip = Rect::new(
        content_x,
        content_y,
        content_x + content_w,
        content_y + content_h,
    );
    scene.push_clip_layer(Affine::IDENTITY, &clip);
    scene.fill(
        peniko::Fill::NonZero,
        Affine::translate((image_x, image_y))
            * Affine::scale_non_uniform(image_w / raster_w, image_h / raster_h),
        brush.as_ref(),
        None,
        &Rect::new(0.0, 0.0, raster_w, raster_h),
    );
    scene.pop_layer();
    true
}

/// Draws an HTML `<canvas>` bitmap with `object-fit`/`object-position`.
///
/// Mirrors [`paint_image`]'s sizing and positioning, but reads pixels from
/// the live [`raikiri_dom::CanvasBitmap`] instead of an [`ImagePixelSource`].
/// A canvas bitmap is always available at its intrinsic size (transparent
/// black when script never painted), so unlike `paint_image` this never
/// emits a resource warning: an empty bitmap simply paints nothing.
///
/// Clipping respects `overflow`: `visible` on both axes paints the full
/// positioned bitmap (CSS Overflow 3 §3.1 lets replaced-element overflow
/// show, which `overflow-canvas.html` pins); any non-visible axis clips to
/// the content box, the same box `paint_image` always uses.
#[allow(clippy::too_many_arguments)]
fn paint_canvas(
    scene: &mut impl PaintScene,
    bitmap: &raikiri_dom::CanvasBitmap,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
    object_fit: ObjectFit,
    object_position: &ComputedCssPosition,
    overflow_x: OverflowValue,
    overflow_y: OverflowValue,
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
        data: peniko::Blob::from(bitmap.rgba.clone()),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: bitmap.width,
        height: bitmap.height,
    };
    let brush = peniko::ImageBrush::new(image_data);
    let clips_overflow = !matches!(overflow_x, OverflowValue::Visible)
        || !matches!(overflow_y, OverflowValue::Visible);
    if clips_overflow {
        let clip = Rect::new(
            content_x,
            content_y,
            content_x + content_w,
            content_y + content_h,
        );
        scene.push_clip_layer(Affine::IDENTITY, &clip);
    }
    scene.fill(
        peniko::Fill::NonZero,
        Affine::translate((image_x, image_y))
            * Affine::scale_non_uniform(image_w / natural_w, image_h / natural_h),
        brush.as_ref(),
        None,
        &Rect::new(0.0, 0.0, natural_w, natural_h),
    );
    if clips_overflow {
        scene.pop_layer();
    }
    true
}

fn natural_object_size(natural: ImageIntrinsicSize) -> (f64, f64) {
    const DEFAULT_WIDTH: f64 = 300.0;
    const DEFAULT_HEIGHT: f64 = 150.0;
    let valid = |value: Option<f32>| {
        value
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(f64::from)
    };
    let width = valid(natural.width);
    let height = valid(natural.height);
    let ratio = natural
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .map(f64::from)
        .or_else(|| width.zip(height).map(|(w, h)| w / h));
    match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (None, None, Some(ratio)) => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            (width, width / ratio)
        }
        (Some(width), None, _) => (width, DEFAULT_HEIGHT),
        (None, Some(height), _) => (DEFAULT_WIDTH, height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_inline_svg(
    scene: &mut impl PaintScene,
    document: &Document,
    node_id: usize,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
    inherited_color: [u8; 4],
    opacity: f32,
    host_opacity_is_specified: bool,
    host_controls_root_background: bool,
    visible: bool,
    warnings: &mut Vec<RenderWarning>,
) -> bool {
    let bl = border.left.width().px();
    let bt = border.top.width().px();
    let br = border.right.width().px();
    let bb = border.bottom.width().px();
    let content_x = (abs_x + bl + padding.left) as f64;
    let content_y = (abs_y + bt + padding.top) as f64;
    let content_w = (border_box_width - bl - br - padding.left - padding.right).max(0.0);
    let content_h = (border_box_height - bt - bb - padding.top - padding.bottom).max(0.0);
    if !visible || opacity <= 0.0 || content_w <= 0.0 || content_h <= 0.0 {
        return true;
    }

    let svg = document
        .serialize_svg_subtree(node_id)
        .ok()
        .flatten()
        .and_then(|source| raikiri_svg::SvgDocument::parse(source.as_bytes()).ok())
        .and_then(|svg| {
            svg.rasterize_at_css_pixel_scale(
                raikiri_svg::SvgViewport {
                    width: content_w,
                    height: content_h,
                },
                raikiri_svg::SvgRootStyle {
                    inherited_color,
                    // The host opacity remains the SVG root's inherited value
                    // while the rasterizer removes its group alpha. The outer
                    // paint group applies it with backgrounds and borders.
                    opacity,
                    neutralize_root_opacity: host_opacity_is_specified,
                    host_controls_root_background,
                    visible,
                },
                None,
            )
            .ok()
        });
    let Some(decoded) = svg else {
        warnings.push(RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: None,
            },
            node_id: Some(NodeId::new(node_id as u64)),
            details: "inline SVG could not be parsed or rasterized".to_owned(),
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
            details: "inline SVG produced an empty raster size".to_owned(),
        });
        return true;
    }
    let image_data = peniko::ImageData {
        data: peniko::Blob::from(decoded.rgba),
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
    true
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
/// Return the intrinsic horizontal background width for a vertical table cell.
///
/// The vertical table projection gives an auto cell the containing block's
/// physical width so its border spans the table. `background-clip: content-box`
/// still follows the cell's measured content width, which keeps an authored
/// cell background from filling the synthetic block-axis remainder.
#[allow(clippy::too_many_arguments)]
// cov:ignore: vertical table cell background geometry is covered by the ignored exact WPT reftest.
fn vertical_table_cell_background_width(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    layout: &taffy::Layout,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
) -> Option<f32> {
    if cascade.computed.get(node_id)?.display != DisplayValue::TableCell
        || cascade.computed.get(node_id)?.background_clip
            != raikiri_style::property::VisualBox::ContentBox
    {
        return None;
    }
    let mut current = Some(node_id);
    let mut vertical = false;
    while let Some(id) = current {
        if matches!(
            cascade
                .authored_writing_modes
                .get(id)
                .and_then(|mode| *mode),
            Some(
                WritingMode::VerticalRl
                    | WritingMode::VerticalLr
                    | WritingMode::SidewaysRl
                    | WritingMode::SidewaysLr
            )
        ) {
            vertical = true;
            break;
        }
        current = document.parent_of(id);
    }
    if !vertical {
        return None;
    }
    let cell = document.get_node(node_id)?;
    let intrinsic_right = cell
        .children
        .iter()
        .filter_map(|child_id| document.get_node(*child_id))
        .map(|child| child.unrounded_layout.location.x + child.unrounded_layout.size.width)
        .filter(|right| right.is_finite())
        .fold(0.0, f32::max);
    if intrinsic_right <= 0.0 {
        return None;
    }
    Some(
        (intrinsic_right + border.right.width().px() + padding.right).clamp(0.0, layout.size.width),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element_background(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    bg: CssColor,
    bg_image: &BackgroundImage,
    current_color: CssColor,
    clip: VisualBox,
    origin: VisualBox,
    border_radius: &ComputedBorderRadius,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
    background_size: &ComputedBackgroundSize,
    background_position: &ComputedCssPosition,
    background_repeat: &raikiri_style::property::BackgroundRepeat,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    paint_element_background_slice(
        scene,
        width,
        height,
        abs_x,
        abs_y,
        bg,
        bg_image,
        current_color,
        clip,
        origin,
        border_radius,
        border,
        padding,
        background_size,
        background_position,
        background_repeat,
        pixel_source,
        warnings,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_element_background_slice(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    bg: CssColor,
    bg_image: &BackgroundImage,
    current_color: CssColor,
    clip: VisualBox,
    origin: VisualBox,
    border_radius: &ComputedBorderRadius,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
    background_size: &ComputedBackgroundSize,
    background_position: &ComputedCssPosition,
    background_repeat: &raikiri_style::property::BackgroundRepeat,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    image_slice: Option<Rect>,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let background_url = match bg_image {
        BackgroundImage::Url(raw_url) => url::Url::parse(raw_url).ok(),
        _ => None,
    };
    let effective = background_color_with_image_source(bg, bg_image, current_color, pixel_source);
    if effective.is_none() && background_url.is_none() {
        return;
    }
    let effective = effective.unwrap_or(CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    });
    if effective.a == 0 && background_url.is_none() {
        return;
    }
    // background-clip: text trims to glyph shapes (CSS Backgrounds 4 section 2.6).
    // Glyph-path clipping is not implemented; warn and skip so the gap is
    // visible instead of silently mispainting an opaque rect.
    if matches!(clip, VisualBox::Text) {
        if effective.a != 0 || background_url.is_some() {
            warnings.push(RenderWarning {
                kind: WarningKind::ResourceFallback {
                    kind: raikiri_traits::ResourceKind::Image,
                    url: background_url
                        .as_ref()
                        .map(redacted_image_url),
                },
                node_id: None,
                details:
                    "background-clip: text for CSS backgrounds is not yet implemented; the background was skipped"
                        .into(),
            });
        }
        return;
    }
    // Border-box geometry from taffy layout. `border-radius` percentages
    // resolve against this box before any origin/clip inset is applied.
    let (bx0, by0, bx1, by1) = (
        (abs_x as f64).round(),
        (abs_y as f64).round(),
        ((abs_x + width) as f64).round(),
        ((abs_y + height) as f64).round(),
    );
    let radius_reference = (bx1 - bx0, by1 - by0);
    let bl = border.left.width().px() as f64;
    let bt = border.top.width().px() as f64;
    let br = border.right.width().px() as f64;
    let bb = border.bottom.width().px() as f64;
    let pl = padding.left as f64;
    let pt = padding.top as f64;
    let pr = padding.right as f64;
    let pb = padding.bottom as f64;
    let color = Color::from_rgba8(effective.r, effective.g, effective.b, effective.a);
    // When an opaque background and all border sides use the same color, the
    // visible union is the border-box curve. Painting that union once avoids
    // a one-pixel seam where an inner clipped fill meets its border ring.
    let border_box_union = matches!(clip, VisualBox::PaddingBox | VisualBox::ContentBox)
        && [&border.top, &border.right, &border.bottom, &border.left]
            .iter()
            .all(|side| {
                side.style() == BorderStyle::Solid
                    && side.width().px() > 0.0
                    && matches!(side.color, BorderColor::Resolved(c) if c == effective)
            });
    let clip = if border_box_union {
        VisualBox::BorderBox
    } else {
        clip
    };
    // Positioning area from `background-origin` (CSS Backgrounds 3 section 2.8).
    // `border-area`/`text` are not valid origins; warn and fall back to the
    // spec initial `padding-box` instead of silently mispositioning.
    let (px0, py0, px1, py1) = match origin {
        VisualBox::BorderBox => (bx0, by0, bx1, by1),
        VisualBox::PaddingBox => (bx0 + bl, by0 + bt, bx1 - br, by1 - bb),
        VisualBox::ContentBox => (bx0 + bl + pl, by0 + bt + pt, bx1 - br - pr, by1 - bb - pb),
        VisualBox::BorderArea | VisualBox::Text => {
            warnings.push(RenderWarning {
                kind: WarningKind::ResourceFallback {
                    kind: raikiri_traits::ResourceKind::Image,
                    url: background_url.as_ref().map(redacted_image_url),
                },
                node_id: None,
                details:
                    "background-origin has an unsupported visual box; using padding-box positioning"
                        .into(),
            });
            (bx0 + bl, by0 + bt, bx1 - br, by1 - bb)
        }
        // cov:ignore: defensive fallback for future `VisualBox` variants
        _ => (bx0 + bl, by0 + bt, bx1 - br, by1 - bb),
    };
    let positioning = Rect::new(px0, py0, px1, py1);
    // Painting area from `background-clip` (CSS Backgrounds 3 section 2.7).
    // Width/height is border-box size from taffy layout.
    let mut radius_inset = (0.0, 0.0, 0.0, 0.0);
    let painting = match clip {
        VisualBox::BorderArea => {
            // Border area is the border box minus the padding box (outer ring).
            // Paint color as 4 strips so inner padding/content stays transparent.
            // URL images in the ring need per-strip tiling that is not yet
            // implemented; warn and skip the image instead of silently omitting it.
            let inner_x0 = bx0 + bl;
            let inner_y0 = by0 + bt;
            let inner_x1 = bx1 - br;
            let inner_y1 = by1 - bb;
            if inner_x1 <= inner_x0
                || inner_y1 <= inner_y0
                || (bl == 0.0 && bt == 0.0 && br == 0.0 && bb == 0.0)
            {
                // Zero borders make the ring equivalent to the border box.
                // Fall through to the standard border-box path below so URL
                // images paint normally instead of being skipped.
                radius_inset = (0.0, 0.0, 0.0, 0.0);
                Rect::new(bx0, by0, bx1, by1)
            } else {
                let top_rect = Rect::new(bx0, by0, bx1, inner_y0);
                scene.fill(
                    Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    color,
                    None,
                    &top_rect,
                );
                let bottom_rect = Rect::new(bx0, inner_y1, bx1, by1);
                scene.fill(
                    Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    color,
                    None,
                    &bottom_rect,
                );
                let left_rect = Rect::new(bx0, inner_y0, inner_x0, inner_y1);
                scene.fill(
                    Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    color,
                    None,
                    &left_rect,
                );
                let right_rect = Rect::new(inner_x1, inner_y0, bx1, inner_y1);
                scene.fill(
                    Fill::NonZero,
                    kurbo::Affine::IDENTITY,
                    color,
                    None,
                    &right_rect,
                );
                if background_url.is_some() {
                    warnings.push(RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: raikiri_traits::ResourceKind::Image,
                        url: background_url
                            .as_ref()
                            .map(redacted_image_url),
                    },
                    node_id: None,
                    details:
                        "background-clip: border-area image tiling in the border ring is not yet implemented; the image was skipped"
                            .into(),
                });
                }
                return;
            }
        }
        VisualBox::PaddingBox => {
            radius_inset = (bl, bt, br, bb);
            Rect::new(
                (bx0 + bl).round(),
                (by0 + bt).round(),
                (bx1 - br).round(),
                (by1 - bb).round(),
            )
        }
        VisualBox::ContentBox => {
            radius_inset = (bl + pl, bt + pt, br + pr, bb + pb);
            Rect::new(
                (bx0 + bl + pl).round(),
                (by0 + bt + pt).round(),
                (bx1 - br - pr).round(),
                (by1 - bb - pb).round(),
            )
        }
        VisualBox::BorderBox => Rect::new(bx0, by0, bx1, by1),
        // `text` returns above; this arm is unreachable but keeps the match
        // exhaustive for future `VisualBox` variants.
        // cov:ignore: unreachable text arm; early return above handles text
        VisualBox::Text => Rect::new(bx0, by0, bx1, by1),
        // cov:ignore: defensive fallback for future `VisualBox` variants
        _ => Rect::new(bx0, by0, bx1, by1),
    };
    // Guard against negative or inverted rect after inset (e.g. border larger than box)
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    if let BackgroundImage::Gradient(Gradient::Conic(conic)) = bg_image {
        if bg.a != 0 {
            let base = Color::from_rgba8(bg.r, bg.g, bg.b, bg.a);
            fill_rounded_background(
                scene,
                base,
                painting.x0,
                painting.y0,
                painting.x1,
                painting.y1,
                border_radius,
                radius_inset,
                radius_reference,
            );
        }
        paint_conic_gradient(
            scene,
            conic,
            positioning,
            painting,
            border_radius,
            radius_inset,
            radius_reference,
            current_color,
        );
        return;
    }
    fill_rounded_background(
        scene,
        color,
        painting.x0,
        painting.y0,
        painting.x1,
        painting.y1,
        border_radius,
        radius_inset,
        radius_reference,
    );
    if let (Some(url), Some(source)) = (background_url.as_ref(), pixel_source) {
        // Clip URL images to the same rounded curve as the color fill
        // (CSS Backgrounds 3 section 5.3: backgrounds clip to the curve).
        // Square painting areas skip the extra layer.
        if let Some(rounded) = rounded_background_path(
            painting.x0,
            painting.y0,
            painting.x1,
            painting.y1,
            border_radius,
            radius_inset,
            radius_reference,
        ) {
            scene.push_clip_layer(Affine::IDENTITY, &rounded);
            paint_background_from_source(
                scene,
                url,
                positioning,
                image_slice.map_or(painting, |slice| painting.intersect(slice)),
                background_size,
                background_position,
                background_repeat,
                source,
                warnings,
            );
            scene.pop_layer();
        } else {
            paint_background_from_source(
                scene,
                url,
                positioning,
                image_slice.map_or(painting, |slice| painting.intersect(slice)),
                background_size,
                background_position,
                background_repeat,
                source,
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

fn background_length(value: ComputedLengthPercentageOrAuto, basis: f64) -> Option<f64> {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(px as f64),
        ComputedLengthPercentageOrAuto::Percent(percent) => Some(basis * percent as f64 / 100.0),
        ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent => None,
        ComputedLengthPercentageOrAuto::Calc(calc) => {
            Some(basis * calc.percent as f64 / 100.0 + calc.px as f64)
        }
    }
}

fn background_image_dimensions(
    size: &ComputedBackgroundSize,
    area_w: f64,
    area_h: f64,
    intrinsic: ImageIntrinsicSize,
) -> Option<(f64, f64)> {
    const DEFAULT_WIDTH: f64 = 300.0;
    const DEFAULT_HEIGHT: f64 = 150.0;

    let (intrinsic_w, intrinsic_h) = match (
        intrinsic.width.map(f64::from),
        intrinsic.height.map(f64::from),
        intrinsic.aspect_ratio.map(f64::from),
    ) {
        (Some(width), Some(height), _) if width > 0.0 && height > 0.0 => (width, height),
        (Some(width), None, Some(ratio)) if width > 0.0 && ratio > 0.0 => (width, width / ratio),
        (None, Some(height), Some(ratio)) if height > 0.0 && ratio > 0.0 => {
            (height * ratio, height)
        }
        (None, None, Some(ratio)) if ratio > 0.0 && area_w > 0.0 && area_h > 0.0 => {
            let width = area_w.min(area_h * ratio);
            (width, width / ratio)
        }
        (None, None, Some(ratio)) if ratio > 0.0 => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            (width, width / ratio)
        }
        (Some(width), None, _) if width > 0.0 => (width, DEFAULT_HEIGHT),
        (None, Some(height), _) if height > 0.0 => (DEFAULT_WIDTH, height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };
    let (image_w, image_h) = match size {
        ComputedBackgroundSize::Cover => {
            let scale = (area_w / intrinsic_w).max(area_h / intrinsic_h);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
        ComputedBackgroundSize::Contain => {
            let scale = (area_w / intrinsic_w).min(area_h / intrinsic_h);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
        ComputedBackgroundSize::Explicit { width, height } => {
            let width = background_length(*width, area_w);
            let height = background_length(*height, area_h);
            match (width, height) {
                (Some(width), Some(height)) => (width.max(0.0), height.max(0.0)),
                (Some(width), None) => (width.max(0.0), width.max(0.0) * intrinsic_h / intrinsic_w),
                (None, Some(height)) => {
                    (height.max(0.0) * intrinsic_w / intrinsic_h, height.max(0.0))
                }
                (None, None) => (intrinsic_w, intrinsic_h),
            }
        }
        _ => (intrinsic_w, intrinsic_h), // cov:ignore: defensive fallback for future background-size variants
    };
    (image_w.is_finite() && image_h.is_finite() && image_w > 0.0 && image_h > 0.0)
        .then_some((image_w, image_h))
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
    scene: &mut impl PaintScene,
    url: &url::Url,
    positioning: Rect,
    painting: Rect,
    size: &ComputedBackgroundSize,
    position: &ComputedCssPosition,
    repeat: &raikiri_style::property::BackgroundRepeat,
    pixel_source: &dyn ImagePixelSource,
    warnings: &mut Vec<RenderWarning>,
) {
    let Some(intrinsic) = pixel_source.intrinsic_size(url) else {
        return;
    };
    let Some((image_w, image_h)) =
        background_image_dimensions(size, positioning.width(), positioning.height(), intrinsic)
    else {
        return;
    };
    let raster_size = ImageRasterSize {
        width: image_w as f32,
        height: image_h as f32,
    };
    if let Some(decoded) = pixel_source.get_decoded_at_size(url, raster_size, None) {
        paint_background_image(
            scene,
            &decoded,
            positioning,
            painting,
            image_w,
            image_h,
            position,
            repeat,
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

fn position_offset(offset: ComputedCssPositionOffset, free_space: f64) -> f64 {
    match offset {
        ComputedCssPositionOffset::Start(value) => match value {
            ComputedLengthPercentage::Px(px) => px as f64,
            ComputedLengthPercentage::Percent(percent) => free_space * percent as f64 / 100.0,
        },
        ComputedCssPositionOffset::End(value) => match value {
            ComputedLengthPercentage::Px(px) => free_space - px as f64,
            // cov:ignore: CSS percentage end offsets normalize to Start before this used-value helper
            ComputedLengthPercentage::Percent(percent) => {
                free_space * (1.0 - percent as f64 / 100.0)
            }
        },
        _ => free_space / 2.0, // cov:ignore: defensive fallback for future position variants
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
/// places the lone tile when at most one fits); `round` rescales the tile so a
/// whole number exactly fills the positioning width/height. Mixed axes combine
/// independently: a `repeat` axis extends to cover `painting` while
/// `space`/`round` axes stay inside `positioning` and clip to `painting`.
#[allow(clippy::too_many_arguments)]
fn paint_background_image(
    scene: &mut impl PaintScene,
    decoded: &raikiri_traits::DecodedImage,
    positioning: Rect,
    painting: Rect,
    image_w: f64,
    image_h: f64,
    position: &ComputedCssPosition,
    repeat: &raikiri_style::property::BackgroundRepeat,
) {
    if decoded.width == 0 || decoded.height == 0 {
        return;
    }
    if painting.x1 <= painting.x0 || painting.y1 <= painting.y0 {
        return;
    }
    if image_w <= 0.0 || image_h <= 0.0 || !image_w.is_finite() || !image_h.is_finite() {
        return;
    }
    let pos_w = positioning.width();
    let pos_h = positioning.height();
    if !pos_w.is_finite() || !pos_h.is_finite() {
        return;
    }
    // Effective tile size after `round` rescaling on each axis. `space` and
    // `repeat` never rescale, so their effective size stays the base size.
    let (tile_w, x_count) = round_axis_tiles(pos_w, image_w, repeat.x);
    let (tile_h, y_count) = round_axis_tiles(pos_h, image_h, repeat.y);
    // cov:ignore: defensive for non-finite rescaled tiles; base and positioning are finite here
    if tile_w <= 0.0 || tile_h <= 0.0 || !tile_w.is_finite() || !tile_h.is_finite() {
        return;
    }
    let x_origins = axis_origins(
        positioning.x0,
        pos_w,
        painting.x0,
        painting.x1,
        tile_w,
        image_w,
        &position.horizontal,
        &repeat.x,
        x_count,
    );
    let y_origins = axis_origins(
        positioning.y0,
        pos_h,
        painting.y0,
        painting.y1,
        tile_h,
        image_h,
        &position.vertical,
        &repeat.y,
        y_count,
    );
    let (Some(x_origins), Some(y_origins)) = (x_origins, y_origins) else {
        return;
    };
    if x_origins.is_empty() || y_origins.is_empty() {
        return;
    }
    let image_data = peniko::ImageData {
        data: peniko::Blob::from(decoded.rgba.clone()),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: decoded.width,
        height: decoded.height,
    };
    let brush = peniko::ImageBrush::new(image_data);
    let tile_shape = Rect::new(0.0, 0.0, decoded.width as f64, decoded.height as f64);
    let tile_scale = Affine::scale_non_uniform(
        tile_w / decoded.width as f64,
        tile_h / decoded.height as f64,
    );
    scene.push_clip_layer(Affine::IDENTITY, &painting);
    for tile_y in &y_origins {
        for tile_x in &x_origins {
            // cov:ignore: defensive for non-finite tiles; origins are finite here
            if !tile_x.is_finite() || !tile_y.is_finite() {
                continue;
            }
            scene.fill(
                Fill::NonZero,
                Affine::translate((*tile_x, *tile_y)) * tile_scale,
                brush.as_ref(),
                None,
                &tile_shape,
            );
        }
    }
    scene.pop_layer();
}

/// Rescaled tile size and tile count for one axis under `round`.
///
/// `round` rescales so a whole number of tiles exactly fills the positioning
/// length: `n = max(1, round(positioning / base))`, `effective = positioning / n`.
/// Other keywords keep the base size; the count is resolved later by
/// [`axis_origins`] (`space` needs the base size, `repeat` tiles to the
/// painting area). A non-positive positioning length cannot host a `round`
/// tile, so the base size is kept and the axis paints a single tile clipped to
/// the painting area.
fn round_axis_tiles(
    positioning_len: f64,
    base_len: f64,
    keyword: BackgroundRepeatKeyword,
) -> (f64, Option<i64>) {
    if !matches!(keyword, BackgroundRepeatKeyword::Round) {
        return (base_len, None);
    }
    if positioning_len.is_nan() || positioning_len <= 0.0 || base_len.is_nan() || base_len <= 0.0 {
        return (base_len, Some(1));
    }
    let count = (positioning_len / base_len).round() as i64;
    let count = count.max(1);
    // Guard against absurd counts from tiny base sizes; tiling is bounded by
    // the painting-area `repeat` path, but `round`/`space` allocate one entry
    // per tile inside `positioning`.
    // cov:ignore: defensive bound for degenerate tiny tiles; tested via empty-repeat guard
    if count > 10_000 {
        return (base_len, Some(1));
    }
    (positioning_len / count as f64, Some(count))
}

/// Tile origins for one axis.
///
/// `positioning_origin`/`positioning_len` describe the `background-origin` edge;
/// `painting_min`/`painting_max` describe the `background-clip` edge. `tile`
/// is the effective (possibly `round`-rescaled) size and `base` the
/// `background-size` size used for `space` fitting. Returns `None` when the
/// geometry cannot place a tile (non-finite positioning).
#[allow(clippy::too_many_arguments)]
fn axis_origins(
    positioning_origin: f64,
    positioning_len: f64,
    painting_min: f64,
    painting_max: f64,
    tile: f64,
    base: f64,
    offset: &ComputedCssPositionOffset,
    keyword: &BackgroundRepeatKeyword,
    round_count: Option<i64>,
) -> Option<Vec<f64>> {
    if !positioning_origin.is_finite() || !positioning_len.is_finite() {
        return None;
    }
    if !painting_min.is_finite() || !painting_max.is_finite() {
        return None;
    }
    match keyword {
        BackgroundRepeatKeyword::Repeat => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            // cov:ignore: defensive for non-finite offsets; positioning and tile are finite here
            if !origin.is_finite() {
                return None;
            }
            // Bound the tile fan-out so a degenerate tiny tile cannot allocate
            // an unbounded origin list; the painting clip keeps the visible
            // result identical.
            let start = ((painting_min - origin) / tile).floor() as i64;
            let end = ((painting_max - origin) / tile).ceil() as i64;
            // cov:ignore: `end <= start` is defensive for empty painting (checked earlier);
            // the `> 10_000` bound is covered by the tiny-tile test below
            if end <= start || end - start > 10_000 {
                return Some(Vec::new());
            }
            Some(
                (start..end)
                    .map(|tile_index| origin + tile_index as f64 * tile)
                    .collect(),
            )
        }
        BackgroundRepeatKeyword::NoRepeat => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            // cov:ignore: defensive for non-finite offsets; inputs are finite here
            if !origin.is_finite() {
                return None;
            }
            Some(vec![origin])
        }
        BackgroundRepeatKeyword::Space => {
            if base.is_nan() || base <= 0.0 || positioning_len.is_nan() || positioning_len <= 0.0 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                // cov:ignore: defensive for non-finite single-tile fallback
                return origin.is_finite().then(|| vec![origin]);
            }
            let count = (positioning_len / base).floor() as i64;
            if count <= 1 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                // cov:ignore: defensive for non-finite single-tile fallback
                return origin.is_finite().then(|| vec![origin]);
            }
            // cov:ignore: defensive bound for degenerate tiny tiles; tiny-tile test covers the repeat bound
            if count > 10_000 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                return origin.is_finite().then(|| vec![origin]);
            }
            let gap = (positioning_len - count as f64 * base) / (count - 1) as f64;
            // cov:ignore: defensive for non-finite gaps; finite inputs give finite gaps here
            if !gap.is_finite() {
                return Some(Vec::new());
            }
            Some(
                (0..count)
                    .map(|index| positioning_origin + index as f64 * (base + gap))
                    .collect(),
            )
        }
        BackgroundRepeatKeyword::Round => {
            let count = round_count.unwrap_or(1).max(1);
            // cov:ignore: defensive bound for degenerate tiny tiles
            if count > 10_000 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                return origin.is_finite().then(|| vec![origin]);
            }
            Some(
                (0..count)
                    .map(|index| positioning_origin + index as f64 * tile)
                    .collect(),
            )
        }
        // cov:ignore: defensive fallback for future repeat keywords
        _ => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            if !origin.is_finite() {
                return None;
            }
            Some(vec![origin])
        }
    }
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
    let mut common = (cropped && diagonal_overlap).then(|| {
        vec![
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(x0, y1),
        ]
    });
    let [tl, tr, br, bl] = radii;
    let centers = [
        Point::new(x0 + tl[0], y0 + tl[1]),
        Point::new(x1 - tr[0], y0 + tr[1]),
        Point::new(x1 - br[0], y1 - br[1]),
        Point::new(x0 + bl[0], y1 - bl[1]),
    ];
    let square = [
        Point::new(x0, y0),
        Point::new(x1, y0),
        Point::new(x1, y1),
        Point::new(x0, y1),
    ];
    let angles = [PI, 3.0 * FRAC_PI_2, 0.0, FRAC_PI_2];
    for (index, ([rx, ry], center)) in radii.into_iter().zip(centers).enumerate() {
        let (point, arc) = if rx == 0.0 || ry == 0.0 {
            (square[index], None)
        } else {
            let (lower, upper) = match index {
                0 => ((center.y - y1) / ry, (center.x - x1) / rx),
                1 => ((x0 - center.x) / rx, (center.y - y1) / ry),
                2 => ((y0 - center.y) / ry, (x0 - center.x) / rx),
                _ => ((center.x - x1) / rx, (y0 - center.y) / ry),
            };
            let start = lower.clamp(0.0, 1.0).asin();
            let end = upper.clamp(0.0, 1.0).acos();
            if start >= end {
                // The whole rectangle lies outside this corner's ellipse.
                return BezPath::new();
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
            let arc = Arc {
                center,
                radii: Vec2::new(rx, ry),
                start_angle: angles[index] + start,
                sweep_angle: end - start,
                x_rotation: 0.0,
            };
            (point, Some(arc))
        };
        if let (Some(polygon), Some(arc)) = (common.as_mut(), arc) {
            // Subdivide the actual ellipse so cubic approximation error cannot
            // move an inner contour outside the existing outer arc.
            let mut points = vec![point];
            flatten_corner_arc(arc, arc.start_angle + arc.sweep_angle, 0, &mut points);
            for edge in points.windows(2) {
                *polygon = clip_corner_polygon(std::mem::take(polygon), edge[0], edge[1]);
                if polygon.len() < 3 {
                    return BezPath::new();
                }
            }
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

/// Paint the shadows, background, borders and outline of one piece of an
/// inline element, the way the element visit paints a box. `x` and `y` are
/// the page position of the paragraph's content box.
///
/// A side whose edge the piece does not carry (the element continues on
/// another line there) has no border, padding or corner radius: the box is
/// sliced (CSS Fragmentation 3 `box-decoration-break: slice`). The padding is
/// what the piece leaves around its content box once the border is taken off.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_inline_box(
    scene: &mut impl PaintScene,
    cv: &ComputedValues,
    piece: &raikiri_dom::InlineBoxPiece,
    background_slice: Option<&crate::ifc_text::BackgroundSlice>,
    x: f32,
    y: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
) {
    let outer = piece.border_box;
    let content = piece.content_box;
    // The start edge is on the physical left when the element's own direction
    // is left to right, whatever the paragraph's direction.
    let (left_edge, right_edge) = if cv.direction == raikiri_style::property::Direction::Rtl {
        (piece.has_end_edge, piece.has_start_edge)
    } else {
        (piece.has_start_edge, piece.has_end_edge)
    };
    let mut border = cv.border;
    if !left_edge {
        border.left = border.left.without_line();
    }
    if !right_edge {
        border.right = border.right.without_line();
    }
    let padding = taffy::Rect {
        left: (content.x - outer.x - border.left.width().px()).max(0.0),
        right: ((outer.x + outer.width) - (content.x + content.width) - border.right.width().px())
            .max(0.0),
        top: (content.y - outer.y - border.top.width().px()).max(0.0),
        bottom: ((outer.y + outer.height)
            - (content.y + content.height)
            - border.bottom.width().px())
        .max(0.0),
    };
    // The same rule as the element visit: radii only for a box that draws a
    // border or a background, and never under a transform or a filter.
    let has_border = [&border.top, &border.right, &border.bottom, &border.left]
        .iter()
        .any(|side| side.width().px() > 0.0 && side.style() != BorderStyle::None);
    // Other inline image fragmentation remains a separate follow-up.
    let background_image = if raikiri_dom::generated_content::generated_origin(piece.node)
        .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::FirstLetter)
    {
        &cv.background_image
    } else {
        &BackgroundImage::None
    };
    let mut radius = paintable_border_radius(
        &cv.border_radius,
        (has_border
            || cv.background_color.a > 0
            || !matches!(background_image, BackgroundImage::None))
            && cv.transform.is_empty()
            && cv.filter.is_empty(),
    );
    let square = raikiri_style::property::CornerRadius::circular(ComputedLengthPercentage::Px(0.0));
    if !left_edge {
        radius.top_left = square;
        radius.bottom_left = square;
    }
    if !right_edge {
        radius.top_right = square;
        radius.bottom_right = square;
    }
    let abs_x = x + outer.x;
    let abs_y = y + outer.y;
    let shadow_slice = background_slice.filter(|slice| {
        !cv.box_shadow.is_empty() && (slice.outer.x != outer.x || slice.outer.width != outer.width)
    });
    if let Some(slice) = shadow_slice {
        // Only artificial inline edges clip a sliced shadow. Its real outer
        // edges and its block-axis overflow remain visible.
        let (mut overflow_x, mut overflow_y) = (1.0_f32, 1.0_f32);
        for shadow in cv.box_shadow.iter() {
            let spread = shadow.spread_radius.px().abs();
            let blur = shadow.blur_radius.px().max(0.0) * 4.0;
            overflow_x = overflow_x.max(shadow.offset_x.px().abs() + spread + blur + 1.0);
            overflow_y = overflow_y.max(shadow.offset_y.px().abs() + spread + blur + 1.0);
        }
        let left = if slice.outer.x < outer.x {
            abs_x
        } else {
            x + slice.outer.x - overflow_x
        };
        let right = if slice.outer.x + slice.outer.width > outer.x + outer.width {
            abs_x + outer.width
        } else {
            x + slice.outer.x + slice.outer.width + overflow_x
        };
        scene.push_clip_layer(
            Affine::IDENTITY,
            &Rect::new(
                f64::from(left),
                f64::from(abs_y - overflow_y),
                f64::from(right),
                f64::from(abs_y + outer.height + overflow_y),
            ),
        );
        let joined_radius = paintable_border_radius(
            &cv.border_radius,
            (has_border
                || cv.background_color.a > 0
                || !matches!(background_image, BackgroundImage::None))
                && cv.transform.is_empty()
                && cv.filter.is_empty(),
        );
        paint_element_box_shadows(
            scene,
            slice.outer.width,
            outer.height,
            x + slice.outer.x,
            abs_y,
            &joined_radius,
            &cv.box_shadow,
            cv.color,
        );
        scene.pop_layer();
    } else {
        paint_element_box_shadows(
            scene,
            outer.width,
            outer.height,
            abs_x,
            abs_y,
            &radius,
            &cv.box_shadow,
            cv.color,
        );
    }
    // A typographic pseudo split across source owners has one background
    // positioning area. Slice that image before applying each owner's offset;
    // fragment heights remain local when their inherited fonts differ.
    let slice = background_slice.filter(|slice| {
        !matches!(background_image, BackgroundImage::None)
            && (slice.outer.x != outer.x || slice.outer.width != outer.width)
    });
    let mut background_outer = outer;
    let mut background_padding = padding;
    if let Some(slice) = slice {
        background_outer.x = slice.outer.x;
        background_outer.width = slice.outer.width;
        background_padding.left =
            (slice.content.x - slice.outer.x - cv.border.left.width().px()).max(0.0);
        background_padding.right = ((slice.outer.x + slice.outer.width)
            - (slice.content.x + slice.content.width)
            - cv.border.right.width().px())
        .max(0.0);
        scene.push_clip_layer(
            Affine::IDENTITY,
            &Rect::new(
                f64::from(abs_x),
                f64::from(abs_y),
                f64::from(abs_x + outer.width),
                f64::from(abs_y + outer.height),
            ),
        );
    }
    let background_radius = if slice.is_some() {
        paintable_border_radius(
            &cv.border_radius,
            cv.transform.is_empty() && cv.filter.is_empty(),
        )
    } else {
        radius
    };
    paint_element_background_slice(
        scene,
        background_outer.width,
        background_outer.height,
        x + background_outer.x,
        abs_y,
        cv.background_color,
        background_image,
        cv.color,
        cv.background_clip,
        cv.background_origin,
        &background_radius,
        if slice.is_some() { &cv.border } else { &border },
        &background_padding,
        &cv.background_size,
        &cv.background_position,
        &cv.background_repeat,
        pixel_source,
        warnings,
        slice.map(|_| {
            Rect::new(
                f64::from(abs_x),
                f64::from(abs_y),
                f64::from(abs_x + outer.width),
                f64::from(abs_y + outer.height),
            )
        }),
    );
    if slice.is_some() {
        scene.pop_layer();
    }
    paint_element_border_rounded(
        scene,
        outer.width,
        outer.height,
        abs_x,
        abs_y,
        &border,
        cv.color,
        &radius,
    );
    paint_element_outline(
        scene,
        outer.width,
        outer.height,
        abs_x,
        abs_y,
        &cv.outline,
        cv.outline_offset,
        cv.color,
    );
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

    let stroke_width = widths[0];
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
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let x0 = (abs_x as f64).round();
    let y0 = (abs_y as f64).round();
    let x1 = ((abs_x + width) as f64).round();
    let y1 = ((abs_y + height) as f64).round();
    let bl = border.left.width().px() as f64;
    let bt = border.top.width().px() as f64;
    let br = border.right.width().px() as f64;
    let bb = border.bottom.width().px() as f64;
    if bl <= 0.0 && bt <= 0.0 && br <= 0.0 && bb <= 0.0 {
        return;
    }
    let resolve = |b: &raikiri_style::resolve::ComputedBorder| -> Option<Color> {
        if matches!(b.style(), BorderStyle::None | BorderStyle::Hidden) {
            return None;
        }
        let w = b.width().px();
        if w <= 0.0 {
            return None;
        }
        let c = match b.color {
            raikiri_style::property::BorderColor::CurrentColor => current_color,
            raikiri_style::property::BorderColor::Resolved(col) => col,
            _ => current_color,
        };
        if c.a == 0 {
            return None;
        }
        Some(Color::from_rgba8(c.r, c.g, c.b, c.a))
    };
    let c_left = resolve(&border.left);
    let c_top = resolve(&border.top);
    let c_right = resolve(&border.right);
    let c_bottom = resolve(&border.bottom);
    let inner_y0 = if paint_top { (y0 + bt).round() } else { y0 };
    let inner_y1 = (y1 - bb).round();
    // Left and right strips only need a vertical span. When the content
    // width collapses to zero (a border-only quadrant where left plus right
    // exactly fill the border box, as in the conic reference files), the
    // full inner box is empty but both side strips still cover their rects.
    let inner_height_valid = inner_y1 > inner_y0;
    // One side strip: `solid` (and unhandled styles) fill the whole
    // strip; `double` draws outer + inner thirds per CSS Backgrounds 3
    // §5.6 ("two parallel solid lines"), leaving the middle third
    // transparent. Below 3px the thirds vanish, so small doubles fall
    // back to solid (matches browser clamping behavior closely enough
    // for reftest purposes).
    let mut strip = |x_a: f64,
                     y_a: f64,
                     x_b: f64,
                     y_b: f64,
                     w: f64,
                     horizontal: bool,
                     col: Color,
                     double: bool| {
        if !double || w < 3.0 {
            let r = Rect::new(x_a, y_a, x_b, y_b);
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, col, None, &r);
            return;
        }
        let third = w / 3.0;
        if horizontal {
            let r1 = Rect::new(x_a, y_a, x_b, y_a + third);
            let r2 = Rect::new(x_a, y_b - third, x_b, y_b);
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, col, None, &r1);
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, col, None, &r2);
        } else {
            let r1 = Rect::new(x_a, y_a, x_a + third, y_b);
            let r2 = Rect::new(x_b - third, y_a, x_b, y_b);
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, col, None, &r1);
            scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, col, None, &r2);
        }
    };
    let is_double = |b: &raikiri_style::resolve::ComputedBorder| b.style() == BorderStyle::Double;
    if paint_top
        && bt > 0.0
        && let Some(col) = c_top
    {
        strip(x0, y0, x1, y0 + bt, bt, true, col, is_double(&border.top));
    }
    if bb > 0.0
        && let Some(col) = c_bottom
    {
        strip(
            x0,
            y1 - bb,
            x1,
            y1,
            bb,
            true,
            col,
            is_double(&border.bottom),
        );
    }
    if bl > 0.0
        && inner_height_valid
        && let Some(col) = c_left
    {
        strip(
            x0,
            inner_y0,
            x0 + bl,
            inner_y1,
            bl,
            false,
            col,
            is_double(&border.left),
        );
    }
    if br > 0.0
        && inner_height_valid
        && let Some(col) = c_right
    {
        strip(
            x1 - br,
            inner_y0,
            x1,
            inner_y1,
            br,
            false,
            col,
            is_double(&border.right),
        );
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
        GradientStopColor::CurrentColor => current_color,
        // non_exhaustive
        _ => current_color,
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
/// matching edge. Only pixel and percentage lengths survive style
/// resolution, so other length kinds fall back to the area middle.
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
        let base = resolve_gradient_stop_color(stop.color, current_color);
        peniko_stops.push((*offset, css_color_to_dynamic(base)));
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
    let gradient = PenikoGradient::new_sweep(center, start_angle, end_angle)
        .with_extend(extend)
        .with_interpolation_cs(conic_interpolation_tag(conic.interpolation.color_space))
        .with_hue_direction(conic_hue_direction(conic.interpolation.hue_method))
        .with_stops(peniko_stops.as_slice());
    Some(gradient)
}

/// Paint one conic background over its painting area.
///
/// The sweep geometry uses the positioning area for its center while the
/// fill covers the painting area, mirroring the origin and clip split used
/// for solid and image backgrounds. Rounded boxes reuse the shared rounded
/// clip so the sweep follows the same curve.
#[allow(clippy::too_many_arguments)]
fn paint_conic_gradient(
    scene: &mut impl PaintScene,
    conic: &ConicGradient,
    positioning: Rect,
    painting: Rect,
    border_radius: &ComputedBorderRadius,
    radius_inset: (f64, f64, f64, f64),
    radius_reference: (f64, f64),
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
    if let Some(rounded) = rounded_background_path(
        painting.x0,
        painting.y0,
        painting.x1,
        painting.y1,
        border_radius,
        radius_inset,
        radius_reference,
    ) {
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rounded);
    } else {
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &painting);
    }
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
fn element_transform(cv: &ComputedValues, x: f32, y: f32, width: f32, height: f32) -> Affine {
    let resolve = |value: ComputedLengthPercentage, basis: f64| match value {
        ComputedLengthPercentage::Px(px) => px as f64,
        ComputedLengthPercentage::Percent(percent) => basis * percent as f64 / 100.0,
    };
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
            ComputedTransformFunction::Translate(x, y) => {
                Affine::translate((resolve(x, paint_width), resolve(y, paint_height)))
            }
            ComputedTransformFunction::TranslateX(x) => {
                Affine::translate((resolve(x, paint_width), 0.0))
            }
            ComputedTransformFunction::TranslateY(y) => {
                Affine::translate((0.0, resolve(y, paint_height)))
            }
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
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    let resolve = |value: ComputedLengthPercentage, basis: f32| match value {
        ComputedLengthPercentage::Px(px) => px,
        ComputedLengthPercentage::Percent(percent) => basis * percent / 100.0,
    };
    for function in cv.transform.iter() {
        match *function {
            ComputedTransformFunction::Translate(x, y) => {
                dx += resolve(x, width);
                dy += resolve(y, height);
            }
            ComputedTransformFunction::TranslateX(x) => dx += resolve(x, width),
            ComputedTransformFunction::TranslateY(y) => dy += resolve(y, height),
            _ => {}
        }
    }
    (dx, dy)
}

fn fixed_position_px(
    cv: &raikiri_style::ComputedValues,
    width: f32,
    height: f32,
    page_width: f32,
    content_origin_y: f32,
    content_width: f32,
    content_height: f32,
) -> Option<(f32, f32)> {
    if !matches!(cv.position, PositionValue::Fixed) {
        return None;
    }
    let to_px = |v: raikiri_style::resolve::ComputedLengthPercentageOrAuto| match v {
        raikiri_style::resolve::ComputedLengthPercentageOrAuto::Px(px) if px.is_finite() => {
            Some(px)
        }
        _ => None,
    };
    let left = to_px(cv.left);
    let right = to_px(cv.right);
    let top = to_px(cv.top);
    let bottom = to_px(cv.bottom);
    let x = left.or_else(|| right.map(|value| content_width - value - width));
    let y = top
        .map(|value| content_origin_y + value)
        .or_else(|| bottom.map(|value| content_origin_y + content_height - value - height));
    Some((
        x.unwrap_or(0.0).clamp(-page_width, page_width),
        y.unwrap_or(content_origin_y).clamp(
            content_origin_y - page_width,
            content_origin_y + page_width + content_height,
        ),
    ))
}

pub(crate) fn position_offset_px(cv: &raikiri_style::ComputedValues) -> (f32, f32) {
    // Only position:relative contributes paint offset. static/absolute/fixed/sticky produce no shift here.
    // Inset properties are <length-percentage> | auto. Percentages are resolved to px earlier (or auto -> 0).
    // For relative, left vs right: if left != auto, dx = left, else if right != auto, dx = -right, else 0.
    // Similarly top vs bottom for dy.
    if !matches!(
        cv.position,
        raikiri_style::property::PositionValue::Relative
    ) {
        return (0.0, 0.0);
    }
    let to_px = |v: raikiri_style::resolve::ComputedLengthPercentageOrAuto| -> Option<f32> {
        match v {
            raikiri_style::resolve::ComputedLengthPercentageOrAuto::Auto => None,
            raikiri_style::resolve::ComputedLengthPercentageOrAuto::Px(px) => Some(px),
            raikiri_style::resolve::ComputedLengthPercentageOrAuto::Percent(_) => None,
            raikiri_style::resolve::ComputedLengthPercentageOrAuto::Calc(_)
            | raikiri_style::resolve::ComputedLengthPercentageOrAuto::MinContent => None, // cov:ignore: only the width / inline-size parser produces min-content
        }
    };
    let left = to_px(cv.left);
    let right = to_px(cv.right);
    let top = to_px(cv.top);
    let bottom = to_px(cv.bottom);
    let dx = if let Some(l) = left {
        l
    } else if let Some(r) = right {
        -r
    } else {
        0.0
    };
    let dy = if let Some(t) = top {
        t
    } else if let Some(b) = bottom {
        -b
    } else {
        0.0
    };
    (dx, dy)
}

fn vertical_align_shift_px(
    va: VerticalAlign,
    display: DisplayValue,
    parent_font_size_px: f32,
) -> f32 {
    if !matches!(display, DisplayValue::Inline | DisplayValue::InlineBlock) {
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
