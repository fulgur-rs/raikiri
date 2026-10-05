//! External-consumer coverage for the computed-value surface a painter reads.
//!
//! Every type below is named through `raikiri_html` (and `raikiri_traits`)
//! only, the way a downstream painter that does not depend on the style crate
//! would write it.

use raikiri_html::computed::{
    BorderColor, BorderStyle, ComputedBackgroundImage, ComputedBorder, ComputedBorderRadius,
    ComputedBoxShadowItem, ComputedDisplay, ComputedGradient, ComputedLengthPercentage,
    ComputedOverflow, ComputedTextDecorationColor, ComputedTextDecorationLine,
    ComputedTextDecorationStyle, ComputedValues, ComputedVisibility, CssColor, GradientColorStop,
    Length, OverflowValue, Sides, TextShadowColor,
};
use raikiri_html::{
    LayoutOptions, LayoutStatus, NodeId, PageSize, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeKind, PageDefaults};

const GREEN: CssColor = CssColor {
    r: 0,
    g: 128,
    b: 0,
    a: 255,
};

fn resolve_border_color(color: BorderColor, current: CssColor) -> CssColor {
    match color {
        BorderColor::CurrentColor => current,
        BorderColor::Resolved(color) => color,
        _ => panic!("unexpected border color {color:?}"),
    }
}

fn border_summary(
    border: &Sides<ComputedBorder>,
    current: CssColor,
) -> (f32, BorderStyle, CssColor) {
    let top: &ComputedBorder = &border.top;
    (
        top.width().px(),
        top.style(),
        resolve_border_color(top.color, current),
    )
}

fn radius_px(radius: &ComputedBorderRadius) -> Option<f32> {
    match radius.top_left {
        ComputedLengthPercentage::Px(px) => Some(px),
        _ => None,
    }
}

fn shadow_summary(shadow: &ComputedBoxShadowItem, current: CssColor) -> (f32, f32, CssColor) {
    let color = match shadow.color {
        TextShadowColor::CurrentColor => current,
        TextShadowColor::Resolved(color) => color,
        _ => panic!("unexpected shadow color"),
    };
    (shadow.offset_x.px(), shadow.blur_radius.px(), color)
}

fn first_stop_px(image: &ComputedBackgroundImage) -> Option<f32> {
    let ComputedBackgroundImage::Gradient(gradient) = image else {
        return None;
    };
    let gradient: &ComputedGradient = gradient;
    let ComputedGradient::Linear(linear) = gradient else {
        return None;
    };
    let stop: &GradientColorStop = linear.stops.first()?;
    match stop.position {
        // Font-relative stop positions are absolutized by the cascade.
        Some(Length::Px(px)) => Some(px),
        _ => None,
    }
}

fn decoration_summary(cv: &ComputedValues) -> (bool, bool, CssColor) {
    let line: ComputedTextDecorationLine = cv.text_decoration_line;
    let wavy = matches!(cv.text_decoration_style, ComputedTextDecorationStyle::Wavy);
    let color = match cv.text_decoration_color {
        ComputedTextDecorationColor::CurrentColor => cv.color,
        ComputedTextDecorationColor::Resolved(color) => color,
        _ => panic!("unexpected decoration color"),
    };
    (line.underline, wavy, color)
}

fn find_by_id(node: NodeId, dom: &raikiri_html::DomView<'_>, id: &str) -> Option<NodeId> {
    if dom.kind(node) == Some(NodeKind::Element) && dom.attr(node, "id") == Some(id) {
        return Some(node);
    }
    dom.children(node)
        .find_map(|child| find_by_id(child, dom, id))
}

#[test]
fn painter_reads_typed_computed_values_through_raikiri_html_only() {
    let html = br#"<!doctype html>
        <style>
          @page { size: 300px 400px; margin: 10px }
          html { font-size: 10px }
          #box {
            color: rgb(0, 128, 0);
            background-image: linear-gradient(red 2em, blue);
            border: 3px dashed currentColor;
            border-radius: 4px;
            box-shadow: 5px 6px 7px currentColor;
            text-decoration: underline wavy rgb(0, 0, 255);
            overflow: hidden;
            visibility: hidden;
          }
        </style>
        <div id="box">text</div>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(summary) = status else {
        panic!("expected a complete layout");
    };
    let page = summary.pages().next().expect("one page");
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, "box").expect("#box");
    let cv = page.computed(node).expect("computed values for #box");

    assert_eq!(cv.color, GREEN);
    assert_eq!(first_stop_px(&cv.background_image), Some(20.0));
    assert_eq!(
        border_summary(&cv.border, cv.color),
        (3.0, BorderStyle::Dashed, GREEN)
    );
    assert_eq!(radius_px(&cv.border_radius), Some(4.0));
    let shadows: &[ComputedBoxShadowItem] = &cv.box_shadow;
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadow_summary(&shadows[0], cv.color), (5.0, 7.0, GREEN));
    assert_eq!(
        decoration_summary(cv),
        (
            true,
            true,
            CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255
            }
        )
    );
    let overflow: ComputedOverflow = cv.overflow;
    assert_eq!(overflow.x, OverflowValue::Hidden);
    assert!(matches!(cv.visibility, ComputedVisibility::Hidden));
    assert!(matches!(cv.display, ComputedDisplay::Block));

    // The page context is readable with the same crate.
    assert!(matches!(
        page.page_style().size(),
        Some(PageSize::Lengths { .. })
    ));
}
