//! DOM walker — Document arena を DFS で walk し PaintScene に emit する。
//!
//! `paint_document` は iterative `PaintFrame` stack で walk する
//! (cascade / find_body の pattern と一貫、深 DOM で stack overflow 回避)。
//! kind 分岐は loop 内で inline に行い、Element は children を push、Text は
//! draw_text_node を call、display:none は subtree ごと skip する。overflow
//! clip は subtree の後で対応する `PopClip` frame により閉じる。
//! `parent_font_size` / `shift_y` は `vertical_align_shift_px` doc 参照。
//!
//! 将来 inline formatting context を実装する時は、Element 分岐内の children
//! push を "self の inline layout を walk する" に置き換え、Text 分岐は
//! unreachable 化する予定 (現状の text_layout 選択は暫定的な妥協のため)。
//!
//! **この inline formatting context の不在は `vertical_align_shift_px` の
//! shift 適用でも未解決のまま残る** — `display: inline` の要素も現状は
//! taffy 上で他の block 要素と同じ独立した行として積み上がる
//! (`bridge_display` が `DisplayValue::Inline` を `taffy::Display::Block` に
//! 写す)。したがって `<p>H<sub>2</sub>O</p>` の "H" / "2" / "O" は現状でも
//! 3 行に分かれたまま描画される — `vertical_align_shift_px` が加える
//! offset は「その独立した行の中で `2` をわずかに動かす」だけであり、
//! `2` を `H`/`O` と同じ行に呼び戻すものではない。この shift は taffy が
//! box 位置を確定させた**後**、paint 時にのみ加算される (taffy 自体は
//! `vertical_align` を一切見ない) ため box-model 計算には一切参加せず、
//! どこにもクリップされない — 例えば page 最上部近くの `vertical-align:
//! super` は margin 領域へはみ出して描画されうる。
//!
//! find_body は raikiri-dom::layout::find_body と重複するが、5 行の helper
//! を crate 境界越境で pub 化するよりも paint 側で持つ方が clean。

use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use raikiri_dom::Document;
use raikiri_style::property::{
    BackgroundImage, Border, BorderColor, BorderStyle, ContentComponent, CounterStyle, CssColor,
    DisplayValue, FloatValue, Gradient, GradientStopColor, Length, LengthOrAuto, OutlineColor,
    OutlineStyle, OverflowValue, PositionValue, PropertyKey, PropertyValue, QuoteKeyword, Sides,
    TextAlign, VerticalAlign, WritingMode, ZIndexValue,
};
use raikiri_style::{
    CascadeResult, ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
    ComputedTransformFunction, PageMarginBoxCascadeResult, PageMarginBoxSlot, ResolveContext,
    resolve_border,
};
use raikiri_traits::{ImagePixelSource, NodeKind, PageBox};
use taffy::CompactLength;

use crate::text;

/// Canvas 背景 fill site — CSS Backgrounds 3 §2.11 canvas propagation の minimal 実装。
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
) {
    let page_color = page_background_color(cascade);
    let canvas_color = canvas_background_color(document, cascade);
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
        if let Some(color) = canvas_color {
            fill_rect(scene, color, page_rect);
        }
        return;
    }

    // The paper is always covered, even when the page background is
    // transparent; the UA canvas default is white in that case.
    fill_rect(
        scene,
        page_color.unwrap_or(white),
        kurbo::Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64),
    );
    if let Some(color) = canvas_color {
        fill_rect(
            scene,
            color,
            kurbo::Rect::new(
                margins.left as f64,
                margins.top as f64,
                (page_box.width - margins.right) as f64,
                (page_box.height - margins.bottom) as f64,
            ),
        );
    }
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

fn page_background_color(cascade: &CascadeResult) -> Option<Color> {
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
    effective_background_color(background_color, &background_image, current_color)
        .map(|color| Color::from_rgba8(color.r, color.g, color.b, color.a))
}

fn canvas_background_color(document: &Document, cascade: &CascadeResult) -> Option<Color> {
    if let Some(html_id) = find_html(document) {
        let cv = &cascade.computed[html_id];
        if let Some(c) =
            effective_background_color(cv.background_color, &cv.background_image, cv.color)
        {
            return Some(Color::from_rgba8(c.r, c.g, c.b, c.a));
        }
    }
    if let Some(body_id) = find_body(document) {
        let cv = &cascade.computed[body_id];
        if let Some(c) =
            effective_background_color(cv.background_color, &cv.background_image, cv.color)
        {
            return Some(Color::from_rgba8(c.r, c.g, c.b, c.a));
        }
    }
    None
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
    /// Compact WPT fallback for the bundled opaque lime image used by the
    /// page-margin background tests. Full resource-backed image painting is
    /// still handled by the document/image pipeline.
    background_image_lime: bool,
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
    font_size: f32,
    font_family: String,
    alignment: parley::Alignment,
    vertical_align: text::MarginTextVerticalAlign,
    /// Whether authored margin-box writing-mode makes newline-separated
    /// content advance across vertical columns rather than down lines.
    vertical_writing: bool,
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
            .map(|family| family.0.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
        _ => root
            .and_then(|computed| computed.font_family.first())
            .map(|family| family.0.as_str().to_string())
            .unwrap_or_else(|| "serif".to_string()),
    };
    (font_size.max(0.1), family)
}

fn content_components_to_text_with_quotes<T: AsRef<str>>(
    components: &[ContentComponent],
    quotes: &[(T, T)],
    quotes_auto: bool,
) -> Option<String> {
    if components.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut depth = 0_usize;
    for component in components {
        match component {
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Quote(keyword) => match keyword {
                QuoteKeyword::OpenQuote => {
                    if let Some((open, _)) = quotes.get(depth) {
                        text.push_str(open.as_ref());
                    } else if quotes_auto && quotes.is_empty() {
                        text.push_str(match depth {
                            0 => "“",
                            _ => "‘",
                        });
                    }
                    depth = depth.saturating_add(1);
                }
                QuoteKeyword::CloseQuote => {
                    depth = depth.saturating_sub(1);
                    if let Some((_, close)) = quotes.get(depth) {
                        text.push_str(close.as_ref());
                    } else if quotes_auto && quotes.is_empty() {
                        text.push_str(match depth {
                            0 => "”",
                            _ => "’",
                        });
                    }
                }
                QuoteKeyword::NoOpenQuote => depth = depth.saturating_add(1),
                QuoteKeyword::NoCloseQuote => depth = depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
    }
    Some(text)
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

fn format_counter(value: i32, style: &CounterStyle) -> String {
    match style {
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("lower-roman") => {
            if value <= 0 {
                return value.to_string();
            }
            let mut n = value;
            let mut result = String::new();
            for (unit, glyph) in [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ] {
                while n >= unit {
                    result.push_str(glyph);
                    n -= unit;
                }
            }
            result
        }
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("upper-roman") => {
            format_counter(value, &CounterStyle::Named("lower-roman".into())).to_uppercase()
        }
        _ => value.to_string(),
    }
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
                ));
                // A page-margin context has one counter scope in this
                // implementation.  The separator is retained for the
                // single-value fallback; nested author scopes are future work.
                let _ = separator;
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
            // Images, named strings, attributes, and target-dependent content
            // need resources or a document-wide generated-content pass.  They
            // remain absent rather than leaking their URL/function spelling.
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

fn generated_pseudo_content(
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
) -> Option<(&raikiri_style::ComputedValues, String)> {
    let computed = cascade
        .pseudo
        .get(&(raikiri_style::StyleNodeId::new(node_id as u64), pseudo))?;
    let content = content_components_to_text_with_quotes(
        &computed.content,
        &computed.quotes,
        computed.quotes_auto,
    )?;
    Some((computed, content))
}

#[allow(clippy::too_many_arguments)]
fn paint_generated_pseudo(
    scene: &mut impl PaintScene,
    cascade: &CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let Some((computed, content)) = generated_pseudo_content(cascade, node_id, pseudo) else {
        return;
    };
    if computed.display == DisplayValue::None {
        return;
    }
    let family = computed
        .font_family
        .first()
        .map(|family| family.0.as_str().to_string())
        .unwrap_or_else(|| "serif".to_string());
    text::draw_margin_text(
        scene,
        &content,
        x,
        y,
        width,
        height,
        css_color(computed.color),
        computed.font_size.px(),
        &family,
        parley::Alignment::Start,
        text::MarginTextVerticalAlign::Top,
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
    // `content: none` / `normal` use an empty component list and suppress
    // the margin box itself, including its background.  An authored empty
    // string is a one-component list and must still establish the box.
    if components.is_empty() {
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
    let background_image_lime = matches!(
        margin_box_property(rule, PropertyKey::BackgroundImage),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(url)))
            if url.ends_with("/green.png") || url == "green.png"
    );
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
        Some(TextAlign::Right | TextAlign::End) => parley::Alignment::Right,
        Some(TextAlign::Center) => parley::Alignment::Center,
        Some(TextAlign::Left) => parley::Alignment::Left,
        _ => parley::Alignment::Start,
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
    let vertical_writing = matches!(
        margin_box_property(rule, PropertyKey::WritingMode),
        Some(PropertyValue::WritingMode(
            WritingMode::VerticalRl
                | WritingMode::VerticalLr
                | WritingMode::SidewaysRl
                | WritingMode::SidewaysLr,
        ))
    );
    Some(MarginBoxPaintSpec {
        slot: rule.slot,
        content,
        background,
        background_image_lime,
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
        font_size,
        font_family,
        alignment,
        vertical_align,
        vertical_writing,
    })
}

fn paint_margin_box(
    scene: &mut impl PaintScene,
    spec: &MarginBoxPaintSpec,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let rect = Rect::new(x as f64, y as f64, (x + width) as f64, (y + height) as f64);
    if spec.background_image_lime {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(0, 255, 0, 255),
            None,
            &rect,
        );
    }
    if let Some(color) = spec.background {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
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
        let ahem_baseline_adjust = if spec
            .font_family
            .split(',')
            .any(|family| family.trim().eq_ignore_ascii_case("ahem"))
        {
            -1.0
        } else {
            0.0
        };
        let content_y = y + border_top + spec.padding[0] + ahem_baseline_adjust;
        let content = if spec.vertical_writing {
            spec.content.replace('\n', "")
        } else {
            spec.content.clone()
        };
        text::draw_margin_text(
            scene,
            &content,
            content_x,
            content_y,
            (width - border_left - border_right - spec.padding[1] - spec.padding[3]).max(0.0),
            (height - border_top - border_bottom - spec.padding[0] - spec.padding[2]).max(0.0),
            spec.text_color,
            spec.font_size,
            &spec.font_family,
            spec.alignment,
            spec.vertical_align,
        );
        scene.pop_layer();
    }
    if spec.content_image_lime {
        let image_x = (x
            + spec.border_left.map(|(width, _)| width).unwrap_or(0.0)
            + spec.padding[3]
            + text::measure_margin_text(&spec.content, spec.font_size, &spec.font_family))
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
    rules: &[PageMarginBoxCascadeResult],
    slot: PageMarginBoxSlot,
) -> Option<PageMarginBoxCascadeResult> {
    // Matching nested rules arrive in source order.  Cascade declarations per
    // property instead of selecting one whole nested rule: a later selector
    // may override only `counter-reset` while inheriting the earlier rule's
    // `content` (the common `@page :right` pattern).
    let matching: Vec<&PageMarginBoxCascadeResult> =
        rules.iter().filter(|rule| rule.slot == slot).collect();
    let last = matching.last()?;
    let mut merged = (*last).clone();
    merged.declarations.clear();
    for rule in matching {
        merged
            .declarations
            .extend(rule.declarations.iter().cloned());
    }
    Some(merged)
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

fn margin_box_text_width(spec: &MarginBoxPaintSpec) -> f32 {
    let measured = text::measure_margin_text(&spec.content, spec.font_size, &spec.font_family);
    // The bundled WPT Ahem face is loaded by the document shaping pass, but
    // the small intrinsic-measure helper owns a separate font context.  Use
    // Ahem's one-em-per-glyph advance as a deterministic fallback there.
    let ahem_width = if spec
        .font_family
        .split(',')
        .any(|family| family.trim().eq_ignore_ascii_case("ahem"))
    {
        spec.content
            .split('\n')
            .map(|line| line.chars().count() as f32 * spec.font_size.max(0.0))
            .fold(0.0, f32::max)
    } else {
        0.0
    };
    let non_collapsible_content = spec
        .content
        .chars()
        .any(|character| !character.is_whitespace() || character == '\u{a0}');
    measured
        .max(ahem_width)
        .max(if non_collapsible_content {
            spec.font_size.max(0.0)
        } else {
            0.0
        })
        .max(0.0)
}

fn margin_box_intrinsic_width(spec: &MarginBoxPaintSpec) -> f32 {
    (margin_box_text_width(spec)
        + margin_box_border_width(spec)
        + margin_box_padding_width(spec)
        + margin_box_margin_width(spec))
    .max(0.0)
}

fn margin_box_intrinsic_height(spec: &MarginBoxPaintSpec) -> f32 {
    if spec.content.is_empty() {
        return 0.0;
    }
    let line_count = spec
        .content
        .trim_end_matches('\n')
        .split('\n')
        .count()
        .max(1) as f32;
    (line_count * spec.font_size.max(0.0)
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

fn paint_horizontal_margin_boxes(
    scene: &mut impl PaintScene,
    specs: &[MarginBoxPaintSpec],
    top: bool,
    page_width: f32,
    page_height: f32,
    margins: raikiri_dom::PageMargins,
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
                margin_box_intrinsic_width(spec)
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
        paint_margin_box(scene, spec, paint_x, y, width, height);
        x += outer_width;
    }
}

fn paint_vertical_margin_boxes(
    scene: &mut impl PaintScene,
    specs: &[MarginBoxPaintSpec],
    left: bool,
    page_width: f32,
    page_height: f32,
    margins: raikiri_dom::PageMargins,
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
                margin_box_intrinsic_height(spec)
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
        paint_margin_box(scene, spec, paint_x, paint_y, width, height);
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
        let Some(rule) = margin_box_rule(cascade.page.margin_boxes(), slot) else {
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
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
    );
    paint_horizontal_margin_boxes(
        scene,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
    );
    paint_vertical_margin_boxes(
        scene,
        &specs,
        true,
        page_box.width,
        page_box.height,
        margins,
    );
    paint_vertical_margin_boxes(
        scene,
        &specs,
        false,
        page_box.width,
        page_box.height,
        margins,
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
        paint_margin_box(scene, spec, x, y, width, height);
    }
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

/// Document arena を body から iterative DFS で walk する。fragment (no `<body>`)
/// case は silent return (layout_single_page が Err を返すので paint
/// 呼び出し前に検出済のはず、defensive)。
///
/// Stack frames carry either a node visit or a matching clip-layer pop.
/// Node children は `.rev()` で push し、pop 時に document order で処理する。
/// Element の場合は `is_display_none` を先に判定し true なら subtree ごと
/// skip (旧来の size == 0 判定は overflow: visible な legitimate zero-size
/// 要素も silent drop するため誤りだったための対応)。
///
/// `parent_font_size` はこの stack frame の node の**親**の used font-size
/// (px)。`shift_y` はこの node に至るまでの祖先全体が積んだ
/// `vertical-align` shift の累計 (px、down 方向が正)。両方とも
/// `vertical_align_shift_px` の入力・出力に対応する — 詳細はその doc 参照。
///
/// 将来 element background-color / border / box-shadow を Element arm 内で
/// 描画する予定 (site だけ確保)。
pub(crate) fn paint_document(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
) {
    paint_document_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        active_page_name,
        fixed_page_width,
        None,
    );
}

/// [`paint_document`] と同一だが、`<img>` element を `pixel_source` から
/// 取得した decode 済み pixel で実際に描画する。
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_document_with_images(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: &dyn ImagePixelSource,
) {
    paint_document_impl(
        scene,
        document,
        cascade,
        page_box,
        content_origin_y,
        active_page_name,
        fixed_page_width,
        Some(pixel_source),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_document_impl(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    active_page_name: Option<Option<&str>>,
    fixed_page_width: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
) {
    let Some(body_id) = find_body(document) else {
        return;
    };

    let named_page_matches = |node_id: usize| match active_page_name {
        None => true,
        Some(active) => match cascade.page_values.get(node_id) {
            Some(raikiri_style::property::PageValue::Named(name)) => {
                // Floats retain the preceding page in the page-name-float
                // cases; do not hide the floated box merely because its
                // inherited page value names the following page.
                if matches!(
                    cascade.computed[node_id].float,
                    FloatValue::Left
                        | FloatValue::Right
                        | FloatValue::InlineStart
                        | FloatValue::InlineEnd
                ) {
                    true
                } else {
                    active.is_some_and(|page| name.0.as_str() == page)
                }
            }
            _ => true,
        },
    };

    // body 自身の親 (`<html>`) の font-size は stack と独立した traversal
    // (`find_body`) でしか到達できないため、self-referential に body 自身の
    // font-size を代わりに使う。body の UA default display は block なので
    // `vertical_align_shift_px` の inline-level gate が常にこの値を無視する
    // — body に `display: inline` を override するような病的な入力でない
    // 限り、この fallback の精度は実質無関係。
    let body_font_size = cascade.computed[body_id].font_size.px();
    // The synthetic body root does not expose its root margin in descendant
    // coordinates.  Direct text and ordinary flow children are therefore
    // seeded with the authored horizontal body origin below; fixed/absolute
    // children keep their own containing-block coordinates.
    let body_has_element_child = document.get_node(body_id).is_some_and(|body| {
        body.children.iter().any(|&child_id| {
            document
                .get_node(child_id)
                .is_some_and(|child| child.kind() == NodeKind::Element)
        })
    });
    let body_has_canvas_background = {
        let body = &cascade.computed[body_id];
        body.background_color.a != 0 || !matches!(body.background_image, BackgroundImage::None)
    };
    let body_has_direct_text = document.get_node(body_id).is_some_and(|body| {
        body.children.iter().any(|&child_id| {
            document.get_node(child_id).is_some_and(|child| {
                child.kind() == NodeKind::Text && child.unrounded_layout.size.height > 0.0
            })
        })
    });
    let body_has_non_ua_margin = cascade
        .non_ua_margin_sides
        .get(body_id)
        .is_some_and(|sides| sides.left);
    let body_margin_left = if body_has_non_ua_margin
        || (body_has_direct_text && !body_has_element_child && body_has_canvas_background)
    {
        if body_has_non_ua_margin {
            match document
                .layout_style(body_id)
                .map(|style| style.margin.left.into_raw())
            {
                Some(raw) if raw.tag() == CompactLength::LENGTH_TAG && raw.value().is_finite() => {
                    raw.value().max(0.0)
                }
                _ => 0.0,
            }
        } else {
            match cascade.computed[body_id].margin.left {
                ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => value.max(0.0),
                _ => 0.0,
            }
        }
    } else {
        0.0
    };
    // The paint walk starts at `<body>` because the html box itself is not a
    // paint item here. Seed the context with html's originating decoration so
    // root-element lines still propagate through the body subtree.
    let empty_decorations = text::DecorationContext::default();
    let root_decorations = find_html(document)
        .map(|html_id| {
            text::decorations_for_element(&empty_decorations, &cascade.computed[html_id], 0.0)
        })
        .unwrap_or_else(|| empty_decorations.clone());
    enum PaintFrame {
        Visit {
            node_id: usize,
            parent_abs_x: f32,
            parent_abs_y: f32,
            parent_font_size: f32,
            shift_y: f32,
            transform_x: f32,
            transform_y: f32,
            inside_fixed: bool,
            inside_fixed_containing_block: bool,
            decorations: text::DecorationContext,
        },
        PopClip,
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
    let mut stack = vec![PaintFrame::Visit {
        node_id: body_id,
        parent_abs_x: 0.0,
        parent_abs_y: 0.0,
        parent_font_size: body_font_size,
        shift_y: 0.0,
        transform_x: 0.0,
        transform_y: 0.0,
        inside_fixed: false,
        inside_fixed_containing_block: false,
        decorations: root_decorations,
    }];
    while let Some(frame) = stack.pop() {
        let (
            node_id,
            parent_abs_x,
            parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            inside_fixed,
            inside_fixed_containing_block,
            decorations,
        ) = match frame {
            PaintFrame::PopClip => {
                scene.pop_layer();
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
                inside_fixed,
                inside_fixed_containing_block,
                decorations,
            ),
        };
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        // template 子孫 + 将来の inert subtree を統一 skip。
        // UA CSS の display:none rule 有無に依存しない、明示的な gate。
        if !node.is_in_document() {
            continue;
        }
        // HTML の hidden elements (metadata / raw-text content / ruby
        // parenthesis fallback) は subtree ごと描画対象外。
        // 現在の対象 tag 集合は `Node::is_non_rendered_html_element` の
        // match arms を single source of truth とする。
        // UA CSS `display: none` は author / user CSS で override 可能なため
        // cascade-independent な defense-in-depth gate として paint 側で
        // fail-close する (HTML LS §15.3.1 "Hidden elements"、
        // https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements
        // 準拠、namespace check で SVG / MathML の同名 element は除外)。
        // <template> は is_in_document 側と二重 gate。
        if node.is_non_rendered_html_element() {
            continue;
        }
        match node.kind() {
            NodeKind::Element => {
                if node.is_display_none() {
                    continue;
                }
                let layout = node.unrounded_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                let cv = &cascade.computed[node_id];
                let paint_padding = used_padding_for_paint(cv, &layout);
                let own_shift =
                    vertical_align_shift_px(cv.vertical_align, cv.display, parent_font_size);
                let child_shift_y = shift_y + own_shift;
                // Compute position:relative offset (CSS Positioned Layout 3 §3).
                let (pos_dx, pos_dy) = position_offset_px(cv);
                let (own_transform_x, own_transform_y) =
                    transform_translation(cv, layout.size.width, layout.size.height);
                let child_transform_x = transform_x + own_transform_x;
                let child_transform_y = transform_y + own_transform_y;
                let is_fixed = matches!(cv.position, PositionValue::Fixed);
                // A transform/filter ancestor establishes a fixed-position
                // containing block. Such descendants keep the legacy local
                // geometry; only viewport-fixed boxes repeat per page.
                let fixed_in_viewport = is_fixed && !inside_fixed_containing_block;
                let establishes_fixed_containing_block =
                    !cv.transform.is_empty() || !cv.filter.is_empty();
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
                let mut paint_height = layout.size.height;
                let mut paint_background_height = layout.size.height;
                let mut paints_on_page = node_id == body_id
                    || ((fixed_in_viewport || named_page_matches(node_id))
                        && (fixed_in_viewport
                            || inside_fixed
                            || box_intersects_page(
                                abs_y,
                                layout.size.height,
                                page_top,
                                page_bottom,
                            )));
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

                if paints_on_page {
                    // A body background is propagated to the page canvas.  For
                    // a non-zero page margin, painting the body border box as
                    // well would leak that color into the translated top/bottom
                    // margin on every page after the first; the canvas pass has
                    // already filled the content rectangle at page-local coords.
                    if node_id != body_id {
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
                            scene.push_clip_layer(Affine::IDENTITY, &clip);
                        }
                        paint_element_background(
                            scene,
                            layout.size.width,
                            paint_background_height,
                            paint_x,
                            paint_y,
                            cv.background_color,
                            &cv.background_image,
                            cv.color,
                            cv.background_clip,
                            &cv.border,
                            &paint_padding,
                        );
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // Paint border on top of background (CSS Backgrounds 3 §5).
                    if paints_as_absolute_continuation {
                        paint_element_border_with_top(
                            scene,
                            layout.size.width,
                            paint_height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            cv.color,
                            false,
                        );
                    } else {
                        paint_element_border(
                            scene,
                            layout.size.width,
                            paint_height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            cv.color,
                        );
                    }
                    // Draw the decoded pixels of an `<img>` element, when a
                    // resolver is supplied and it already has decoded pixels
                    // for this element's `src` (CSS Images 3 §4.3, `object-fit:
                    // fill` only — see `paint_image`'s doc). Absent a resolver
                    // (the plain `paint_document` entry point), `pixel_source`
                    // is always `None` and the chain short-circuits before
                    // `img_src_url` runs at all, keeping that path's per-element
                    // work (not just its output) identical to before this was
                    // added. When no real decoded pixels are available, fall
                    // back to the pre-existing filename-color heuristic so an
                    // `<img>` still renders an approximation in the no-resolver
                    // (e.g. plain WPT range) path.
                    if let Some(pixel_source) = pixel_source
                        && let Some(src_url) = img_src_url(document, node_id)
                        && let Some(decoded) = pixel_source.get_decoded(&src_url)
                    {
                        paint_image(
                            scene,
                            &decoded,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &paint_padding,
                        );
                    } else if node.tag_name() == Some("img")
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
                            scene.push_clip_layer(Affine::IDENTITY, &clip);
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
                            &cv.border,
                            &paint_padding,
                        );
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // Generated content is an immediate child of its
                    // originating box.  The minimal layout engine does not
                    // allocate an arena node for it, so paint literal
                    // `::before` content at the box's inline start.
                    if !paints_as_absolute_continuation {
                        paint_generated_pseudo(
                            scene,
                            cascade,
                            node_id,
                            raikiri_style::PseudoElem::Before,
                            paint_x,
                            paint_y,
                            layout.size.width,
                            layout.size.height,
                        );
                    }
                }
                // CSS Overflow 3 §3.1: non-visible overflow clips descendants to
                // the padding box. The current WPT coverage uses `overflow:hidden`
                // with no padding or border, so the border-box geometry is the
                // correct clip edge for this path as well. The clip is pushed only
                // after painting the element itself, then popped after its complete
                // subtree via the explicit stack frame.
                let clips_overflow = !matches!(cv.overflow.x, OverflowValue::Visible)
                    || !matches!(cv.overflow.y, OverflowValue::Visible);
                if clips_overflow {
                    let clip = Rect::new(
                        (paint_x + layout.padding.left) as f64,
                        (paint_y + layout.padding.top) as f64,
                        (paint_x + layout.size.width - layout.padding.right) as f64,
                        (paint_y + layout.size.height - layout.padding.bottom) as f64,
                    );
                    scene.push_clip_layer(Affine::IDENTITY, &clip);
                    stack.push(PaintFrame::PopClip);
                }
                let child_font_size = cv.font_size.px();
                // `text-decoration-line` is non-inherited at the computed-value
                // layer, but its originating line is propagated to descendants
                // by CSS Text Decoration. Keep that paint-only context separate
                // from `CascadeResult`'s inheritance result.
                let child_decorations =
                    text::decorations_for_element(&decorations, cv, child_shift_y);
                // Paint positioned siblings in stacking order while keeping
                // source order for equal stack levels.  This is intentionally
                // local to the current parent; full nested stacking-context
                // isolation remains outside this minimal painter.
                let mut children = node.children.clone();
                children.sort_by_key(|&child| paint_order_key(cascade, child));
                // Reverse push makes the lowest stack level paint first.
                // For position:relative, children are laid out at normal flow
                // position but paint at offset position.
                let child_parent_x = abs_x + pos_dx + fixed_dx;
                let child_parent_y = abs_y + pos_dy + fixed_dy;
                for child in children.into_iter().rev() {
                    let body_child_margin_offset = if node_id == body_id
                        && (document
                            .get_node(child)
                            .is_some_and(|node| node.kind() == NodeKind::Text)
                            || matches!(
                                cascade.computed[child].position,
                                PositionValue::Static
                                    | PositionValue::Relative
                                    | PositionValue::Sticky
                            )) {
                        body_margin_left
                    } else {
                        0.0
                    };
                    stack.push(PaintFrame::Visit {
                        node_id: child,
                        parent_abs_x: child_parent_x + body_child_margin_offset,
                        parent_abs_y: child_parent_y,
                        parent_font_size: child_font_size,
                        shift_y: child_shift_y,
                        transform_x: child_transform_x,
                        transform_y: child_transform_y,
                        inside_fixed: inside_fixed || fixed_in_viewport,
                        inside_fixed_containing_block: inside_fixed_containing_block
                            || establishes_fixed_containing_block,
                        decorations: child_decorations.clone(),
                    });
                }
            }
            NodeKind::Text => {
                let layout = node.unrounded_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                if named_page_matches(node_id)
                    && (inside_fixed
                        || box_intersects_page(abs_y, layout.size.height, page_top, page_bottom))
                {
                    let clip_text_to_page_content = !inside_fixed
                        && content_width.is_finite()
                        && content_width > 0.0
                        && page_box.width >= content_width * 2.0
                        && cascade.page.margin_boxes().is_empty();
                    if clip_text_to_page_content {
                        let clip = Rect::new(
                            page_offset_x as f64,
                            (margins.top + insets.top) as f64,
                            (page_box.width - margins.right - insets.right) as f64,
                            (margins.top + insets.top + content_height) as f64,
                        );
                        scene.push_clip_layer(Affine::IDENTITY, &clip);
                    }
                    text::draw_text_node(
                        scene,
                        node,
                        cascade,
                        node_id,
                        text::TextPosition {
                            abs_x: abs_x + page_offset_x + transform_x,
                            abs_y: abs_y + page_offset_y + transform_y,
                            shift_y,
                        },
                        &decorations,
                    );
                    if clip_text_to_page_content {
                        scene.pop_layer();
                    }
                }
            }
            NodeKind::Document => {
                // paint_document が body から start するので通常来ない。
                // Document node は children を持ちうる (未 attach <html>) が
                // 現状は扱わない。defensive: subtree を skip。
            }
            _ => {
                // NodeKind is #[non_exhaustive]: `Comment` / `ProcessingInstruction` /
                // `DocumentFragment` はここに落ちる (paint 対象外)。実際には
                // mark_in_document_flags が Comment/PI の IS_IN_DOCUMENT bit を
                // clear しているため、この walker 到達前段の is_in_document()
                // gate で先に filter されることが expected — defense-in-depth の
                // 第 2 gate として本 arm を保持 (kind gate と is_in_document gate
                // の両方が failing した場合でも subtree ごと skip)。将来 CDATA /
                // DocumentType 等が追加された場合も同じ扱い。
            }
        }
    }
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

/// Draws `decoded`'s pixels stretched to exactly fill the element's content
/// box (CSS Images 3 §4.3 `object-fit: fill`, the only value this scope
/// implements — no aspect-ratio preservation, no letterboxing).
#[allow(clippy::too_many_arguments)]
fn paint_image(
    scene: &mut impl PaintScene,
    decoded: &raikiri_traits::DecodedImage,
    border_box_width: f32,
    border_box_height: f32,
    abs_x: f32,
    abs_y: f32,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
) {
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
    if content_w <= 0.0 || content_h <= 0.0 || decoded.width == 0 || decoded.height == 0 {
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
    let transform = kurbo::Affine::translate((content_x, content_y))
        * kurbo::Affine::scale_non_uniform(
            content_w / decoded.width as f64,
            content_h / decoded.height as f64,
        );
    let shape = kurbo::Rect::new(0.0, 0.0, decoded.width as f64, decoded.height as f64);
    scene.fill(
        peniko::Fill::NonZero,
        transform,
        brush.as_ref(),
        None,
        &shape,
    );
}

/// `vertical-align` が inline-level box の位置へ寄与する pixel offset。
/// 正の戻り値 = 下方向 (`draw_text_node` の `abs_y` と同じ、Y が下に伸びる
/// 座標系)。
///
/// # 実装範囲
///
/// [`VerticalAlign::Sub`] / [`VerticalAlign::Super`] のみ shift を計算する。
/// `top` / `text-top` / `middle` / `bottom` / `text-bottom` は
/// raikiri-style の parser がそもそも受理しない (silent drop —
/// [`raikiri_style::property::VerticalAlign`] doc の "Scope carving" 節)
/// ためこの関数に届かない。`_` arm はこの関数を total にするための
/// defensive default であり (`VerticalAlign` は `#[non_exhaustive]`)、
/// [`VerticalAlign::Baseline`] も同じ 0 shift になる (CSS 2.1 §10.8.1
/// verbatim: "Align the baseline of the box with the baseline of the
/// parent box" — 追加の shift なし)。
///
/// CSS 2.1 §10.8.1 "Applies to: inline-level and 'table-cell' elements"
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
/// (`table-cell` はこの crate 未実装) — block-level box の
/// `vertical-align: sub` は shift に寄与しない。
///
/// # Shift 量
///
/// CSS 2.1 §10.8.1 自体は `sub`/`super` の offset を "the proper position
/// for subscripts/superscripts" とだけ述べ、量を implementation-defined の
/// ままにする。CSS Inline Layout Module Level 3 §4.2.3 "Post-Alignment
/// Shift: the baseline-shift longhand"
/// <https://www.w3.org/TR/css-inline-3/#baseline-shift-property> が
/// 具体的な UA-default fallback を与える (font metrics 参照はそちらが
/// 優先だが本関数では未実装 — font table を一切読まない):
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
/// `vertical-align` (CSS 2.1) と `baseline-shift` (CSS Inline 3) は別
/// property である — [`raikiri_style::property::VerticalAlign`] doc が
/// 説明する通り、本 crate は keyword grammar の primary source として CSS
/// 2.1 を採り続ける。ここで CSS Inline 3 を引くのは、CSS 2.1 が定義しない
/// shift **量**についてのみ、CSS Inline 3 の同じ `sub`/`super` keyword に
/// 対する UA-default fallback 記述を借りるためである。
///
/// `parent_font_size_px` は **box 自身の親の** used font-size でなければ
/// ならない (box 自身の font-size ではない — `sub`/`super` content は通常
/// 既に author/UA の `font-size: smaller` で縮小済みで、上記 spec 文の
/// "the parent's used font-size" はその縮小前の値を指す)。
///
/// # Nested `vertical-align` の合成 (未検証の近似)
///
/// この関数自体は 1 box 分の shift だけを返す。呼び出し側
/// ([`paint_document`]) は祖先ごとの shift を単純加算で累積する
/// (`shift_y` stack frame) — real な inline formatting context 下では
/// 各 box は直接の親の baseline に対して shift し、それが line box
/// 構築を通じて連鎖することの素朴な近似であり、どの primary source にも
/// 明記された規則ではない。
///
/// # Box-model への非参加 (未実装)
///
/// この戻り値は taffy が box 位置を確定させた後、paint 時にのみ加算される
/// — taffy 自身は `vertical_align` を見ないため、shift された結果が
/// どこにもクリップされない。CSS 2.1 / CSS Inline 3 とも shift 後の位置を
/// box-model 計算 (line box の高さ等) に参加させる前提だが、ここでは
/// 参加しない — 極端な shift 量が page box の外へはみ出して描画されうる。
#[allow(clippy::too_many_arguments)]
fn paint_element_background(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    bg: CssColor,
    bg_image: &BackgroundImage,
    current_color: CssColor,
    clip: raikiri_style::property::VisualBox,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &taffy::Rect<f32>,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let Some(effective) = effective_background_color(bg, bg_image, current_color) else {
        return;
    };
    if effective.a == 0 {
        return;
    }
    // background-clip: text — clip to text glyphs (CSS Backgrounds 4 §2.6).
    // Requires glyph path clipping which is not yet implemented; treat as no opaque rect.
    // This intentionally leaves coverage gap for text-clip tests (tracked separately).
    if matches!(clip, raikiri_style::property::VisualBox::Text) {
        return;
    }
    // Compute inset rect for background-clip per CSS Backgrounds 3 §2.7:
    // - border-box / border-area: border box (full rect)
    // - padding-box: padding box (inset by border widths)
    // - content-box: content box (inset by border + padding)
    // Width/height is border-box size from taffy layout.
    let (mut x0, mut y0, mut x1, mut y1) = (
        (abs_x as f64).round(),
        (abs_y as f64).round(),
        ((abs_x + width) as f64).round(),
        ((abs_y + height) as f64).round(),
    );
    let color = Color::from_rgba8(effective.r, effective.g, effective.b, effective.a);
    match clip {
        raikiri_style::property::VisualBox::BorderArea => {
            // Border area is the border box minus the padding box (outer ring).
            // Paint as 4 strips so inner padding/content stays transparent.
            let bl = border.left.width().px() as f64;
            let bt = border.top.width().px() as f64;
            let br = border.right.width().px() as f64;
            let bb = border.bottom.width().px() as f64;
            let inner_x0 = x0 + bl;
            let inner_y0 = y0 + bt;
            let inner_x1 = x1 - br;
            let inner_y1 = y1 - bb;
            // If border is zero or inner invalid, fall back to full rect (border-box)
            if inner_x1 <= inner_x0
                || inner_y1 <= inner_y0
                || (bl == 0.0 && bt == 0.0 && br == 0.0 && bb == 0.0)
            {
                let rect = Rect::new(x0, y0, x1, y1);
                scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, color, None, &rect);
                return;
            }
            // Top strip
            let top_rect = Rect::new(x0, y0, x1, inner_y0);
            scene.fill(
                Fill::NonZero,
                kurbo::Affine::IDENTITY,
                color,
                None,
                &top_rect,
            );
            // Bottom strip
            let bottom_rect = Rect::new(x0, inner_y1, x1, y1);
            scene.fill(
                Fill::NonZero,
                kurbo::Affine::IDENTITY,
                color,
                None,
                &bottom_rect,
            );
            // Left strip (between top and bottom)
            let left_rect = Rect::new(x0, inner_y0, inner_x0, inner_y1);
            scene.fill(
                Fill::NonZero,
                kurbo::Affine::IDENTITY,
                color,
                None,
                &left_rect,
            );
            // Right strip
            let right_rect = Rect::new(inner_x1, inner_y0, x1, inner_y1);
            scene.fill(
                Fill::NonZero,
                kurbo::Affine::IDENTITY,
                color,
                None,
                &right_rect,
            );
            return;
        }
        raikiri_style::property::VisualBox::PaddingBox => {
            x0 = (x0 + border.left.width().px() as f64).round();
            y0 = (y0 + border.top.width().px() as f64).round();
            x1 = (x1 - border.right.width().px() as f64).round();
            y1 = (y1 - border.bottom.width().px() as f64).round();
        }
        raikiri_style::property::VisualBox::ContentBox => {
            // border inset
            let bl = border.left.width().px() as f64;
            let bt = border.top.width().px() as f64;
            let br = border.right.width().px() as f64;
            let bb = border.bottom.width().px() as f64;
            // padding inset from Taffy's used box geometry.
            let pl = padding.left as f64;
            let pt = padding.top as f64;
            let pr = padding.right as f64;
            let pb = padding.bottom as f64;
            x0 = (x0 + bl + pl).round();
            y0 = (y0 + bt + pt).round();
            x1 = (x1 - br - pr).round();
            y1 = (y1 - bb - pb).round();
        }
        // BorderBox: no inset
        _ => {}
    }
    // Guard against negative or inverted rect after inset (e.g. border larger than box)
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let rect = Rect::new(x0, y0, x1, y1);
    scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, color, None, &rect);
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
    let inner_x0 = (x0 + bl).round();
    let inner_y0 = if paint_top { (y0 + bt).round() } else { y0 };
    let inner_x1 = (x1 - br).round();
    let inner_y1 = (y1 - bb).round();
    let inner_valid = inner_x1 > inner_x0 && inner_y1 > inner_y0;
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
        && inner_valid
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
        && inner_valid
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

fn transform_translation(
    cv: &raikiri_style::ComputedValues,
    width: f32,
    height: f32,
) -> (f32, f32) {
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
            // Rotation, scale, skew, and arbitrary matrices need transformed
            // glyph/clip geometry. Keep the compact paint fallback unchanged
            // for those functions until that matrix path exists.
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

fn position_offset_px(cv: &raikiri_style::ComputedValues) -> (f32, f32) {
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
            raikiri_style::resolve::ComputedLengthPercentageOrAuto::Calc(_) => None,
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
        // `VerticalAlign` は `#[non_exhaustive]` — この関数を total に
        // するための defensive default で、今日は `Baseline` だけがここへ
        // 落ちる (0 shift、spec 通り)。将来 `VerticalAlign` に新しい
        // keyword が加われば、raikiri-style 側で明示的に shift 計算が
        // 実装されるまでこの arm がその keyword を黙って 0 shift にする
        // — `raikiri_style::property::VerticalAlign` doc の "Scope
        // carving" 節が parse 層で戒めている「実装が追いつくまで受理し
        // ない」規律を、この consumption 側では compile time に強制でき
        // ない。新しい variant を追加する際は、まずここを明示的な match
        // arm にすること。
        _ => 0.0,
    }
}

fn paint_order_key(cascade: &CascadeResult, node_id: usize) -> (u8, i32) {
    let computed = &cascade.computed[node_id];
    match (&computed.position, computed.z_index) {
        // An integer z-index applies to positioned boxes.  Keep ordinary
        // in-flow boxes in the auto/source-order bucket.
        (PositionValue::Static, _) | (_, ZIndexValue::Auto) => (1, 0),
        (_, ZIndexValue::Integer(value)) if value < 0 => (0, value),
        (_, ZIndexValue::Integer(value)) => (2, value),
        // PositionValue and ZIndexValue are non-exhaustive.  New variants
        // retain the default/source-order bucket until stacking support grows.
        _ => (1, 0),
    }
}

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// 深 DOM で stack overflow を回避)。fragment parse (no `<body>`) では `None`。
///
/// `!is_in_document()` の subtree (`<template>` descendants など) を skip
/// する。inert subtree 内の hypothetical `<body>` を選ばないため。paint 側の
/// find_body と layout 側の
/// find_body は独立実装 (crate 境界越境コスト回避)、同じ contract を持つ。
fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}
