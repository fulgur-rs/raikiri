//! Button defaults through the public HTML layout and computed-style surface.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_style::ComputedLengthPercentageOrCalc as P;
use raikiri_style::property::{BorderStyle, BoxSizing, DisplayValue, FontStyle, TextAlign};
use raikiri_style::resolve::ComputedLineHeight;
use raikiri_traits::{LayoutConfig, PageDefaults};

type Rect = (f32, f32, f32, f32);

fn lay_out(body: &str, extra: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}{extra}</style>{body}"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

fn find_by_id(node: NodeId, dom: &raikiri_html::DomView<'_>, id: &str) -> Option<NodeId> {
    if dom.attr(node, "id") == Some(id) {
        return Some(node);
    }
    dom.children(node)
        .find_map(|child| find_by_id(child, dom, id))
}

fn check(document: &DocumentLayout, expected: &[(&str, Rect)]) {
    let page = document.page(0).unwrap();
    let dom = page.dom();
    for &(id, rect) in expected {
        let node = find_by_id(dom.root(), &dom, id).unwrap();
        let actual = page.fragments().find(|f| f.node() == node).unwrap().rect();
        let actual = (actual.x, actual.y, actual.width, actual.height);
        assert!(
            (actual.0 - rect.0).abs() <= 0.02
                && (actual.1 - rect.1).abs() <= 0.02
                && (actual.2 - rect.2).abs() <= 0.02
                && (actual.3 - rect.3).abs() <= 0.02,
            "#{id}: {actual:?}, expected {rect:?}"
        );
    }
}

#[test]
fn text_and_block_button_contents_include_default_padding_and_borders() {
    for content in ["YYYY", "<div id=k>YYYY</div>"] {
        let document = lay_out(
            &format!(
                "<div id=o style='width:300px'><button id=c style='font:10px/10px Ahem'>{content}</button></div>"
            ),
            "",
        );
        check(
            &document,
            &[
                ("o", (8.0, 8.0, 300.0, 16.0)),
                ("c", (8.0, 8.0, 56.0, 16.0)),
            ],
        );
        if content.starts_with("<div") {
            check(&document, &[("k", (16.0, 11.0, 40.0, 10.0))]);
        }
    }
}

#[test]
fn button_defaults_reset_inherited_font_and_supply_used_box_styles() {
    let document = lay_out(
        "<button id=c><div>YYYY</div></button>",
        "body{font:italic bold 30px/40px Ahem}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, "c").unwrap();
    let cv = page.computed(node).unwrap();
    assert_eq!(cv.display, DisplayValue::InlineBlock);
    assert_eq!(cv.box_sizing, BoxSizing::BorderBox);
    assert_eq!(cv.text_align, TextAlign::Center);
    assert_eq!(cv.font_family[0].to_string(), "Arial");
    assert!((cv.font_size.px() - 13.333333).abs() < 0.0001);
    assert_eq!(cv.font_style, FontStyle::Normal);
    assert_eq!(cv.font_weight, 400.0);
    assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    for (padding, expected) in [
        (cv.padding.top, 1.0),
        (cv.padding.right, 6.0),
        (cv.padding.bottom, 1.0),
        (cv.padding.left, 6.0),
    ] {
        assert_eq!(padding, P::Px(expected));
    }
    for border in [
        cv.border.top,
        cv.border.right,
        cv.border.bottom,
        cv.border.left,
    ] {
        assert_eq!(border.width().px(), 2.0);
        assert_eq!(border.style(), BorderStyle::Outset);
    }
}

#[test]
fn default_font_metrics_reach_text_and_block_contents_with_bundled_fallback() {
    // Only Ahem is registered, so the UA's Arial/sans-serif falls back to it.
    // These are Chromium's boxes for the same 13.3333px Ahem face and normal
    // line height, without depending on a host's installed Arial substitute.
    for content in ["YYYY", "<div id=k>YYYY</div>"] {
        let document = lay_out(&format!("<button id=c>{content}</button>"), "");
        check(&document, &[("c", (8.0, 8.0, 69.3125, 20.0))]);
        if content.starts_with("<div") {
            check(&document, &[("k", (16.0, 11.0, 53.3125, 14.0))]);
        }
    }
}

#[test]
fn author_font_padding_and_border_override_button_defaults() {
    let document = lay_out(
        "<div id=o style='width:300px'><button id=c style='font:10px/10px Ahem;padding:0;border:0'><div id=k>YYYY</div></button></div>",
        "",
    );
    check(
        &document,
        &[("c", (8.0, 8.0, 40.0, 10.0)), ("k", (8.0, 8.0, 40.0, 10.0))],
    );
}

#[test]
fn authored_button_dimensions_use_border_box_and_center_block_contents() {
    let document = lay_out(
        "<div style='width:300px'><button id=c style='font:10px/10px Ahem;width:100px;height:40px'><div id=k>YYYY</div></button></div>",
        "",
    );
    check(
        &document,
        &[
            ("c", (8.0, 8.0, 100.0, 40.0)),
            ("k", (16.0, 23.0, 84.0, 10.0)),
        ],
    );
}
