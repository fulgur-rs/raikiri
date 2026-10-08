//! Default HTML table styling through the independent painter surface.

use raikiri_html::computed::ComputedLengthPercentage;
use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_style::ComputedTextIndent;
use raikiri_style::property::{BoxSizing, TextAlign};
use raikiri_traits::{LayoutConfig, PageDefaults};

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
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}{extra}</style><body>{body}</body>"
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

#[test]
fn default_cells_have_one_pixel_padding_and_headers_are_bold() {
    let document = lay_out(
        "<table><tr><th id='header'>H</th><td id='cell'>A</td></tr></table>",
        "",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    for id in ["header", "cell"] {
        let node = find_by_id(dom.root(), &dom, id).unwrap();
        let cv = page.computed(node).unwrap();
        for side in [
            cv.padding.top,
            cv.padding.right,
            cv.padding.bottom,
            cv.padding.left,
        ] {
            assert_eq!(side, ComputedLengthPercentage::Px(1.0));
        }
        assert_eq!(cv.font_weight, if id == "header" { 700.0 } else { 400.0 });
        let rect = page
            .fragments()
            .find(|fragment| fragment.node() == node)
            .unwrap()
            .rect();
        assert_eq!(rect.height, 22.0);
    }
    // The first cell starts at the table's 2px spacing and adds 1px padding.
    let runs = page.text_runs();
    let header = runs.iter().find(|run| run.text == "H").unwrap();
    assert_eq!(header.origin.0, 3.0);
}

#[test]
fn caption_is_centered_in_the_table_width() {
    let document = lay_out(
        "<table style='width:80px'><caption id='caption'>C</caption><tr><td>A</td></tr></table>",
        "",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, "caption").unwrap();
    assert_eq!(page.computed(node).unwrap().text_align, TextAlign::Center);
    let runs = page.text_runs();
    let caption = runs.iter().find(|run| run.text == "C").unwrap();
    // A 10px Ahem glyph is centered in an 80px caption.
    assert_eq!(caption.origin.0, 35.0);
}

#[test]
fn tables_reset_inherited_indent_and_use_border_box_sizing() {
    let document = lay_out(
        "<table id='outer' style='width:80px;border:5px solid'>\
         <tr><td><table id='inner'><tr><td>A</td></tr></table></td></tr></table>",
        "body{ text-indent:30px }td{ text-indent:20px }",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    for id in ["outer", "inner"] {
        let node = find_by_id(dom.root(), &dom, id).unwrap();
        let cv = page.computed(node).unwrap();
        assert_eq!(cv.box_sizing, BoxSizing::BorderBox);
        assert_eq!(cv.text_indent, ComputedTextIndent::Px(0.0));
    }
    let outer = find_by_id(dom.root(), &dom, "outer").unwrap();
    let rect = page
        .fragments()
        .find(|fragment| fragment.node() == outer)
        .unwrap()
        .rect();
    assert_eq!(rect.width, 80.0);
}

#[test]
fn author_styles_override_each_table_default() {
    let document = lay_out(
        "<table id='table'><caption id='caption'>C</caption><tr><th id='header'>H</th></tr></table>",
        "table{box-sizing:content-box;text-indent:9px}th{padding:4px;font-weight:400}\
         caption{text-align:left}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let table = page
        .computed(find_by_id(dom.root(), &dom, "table").unwrap())
        .unwrap();
    assert_eq!(table.box_sizing, BoxSizing::ContentBox);
    assert_eq!(table.text_indent, ComputedTextIndent::Px(9.0));
    let header = page
        .computed(find_by_id(dom.root(), &dom, "header").unwrap())
        .unwrap();
    assert_eq!(header.padding.left, ComputedLengthPercentage::Px(4.0));
    assert_eq!(header.font_weight, 400.0);
    let caption = page
        .computed(find_by_id(dom.root(), &dom, "caption").unwrap())
        .unwrap();
    assert_eq!(caption.text_align, TextAlign::Left);
}

#[test]
fn cell_padding_hint_beats_ua_padding_but_author_css_still_wins() {
    let document = lay_out(
        "<table cellpadding='0'><tr><td id='zero'>A</td><th id='authored'>H</th></tr></table>",
        "#authored{padding:4px}",
    );
    let page = document.page(0).unwrap();
    let dom = page.dom();
    for (id, padding) in [("zero", 0.0), ("authored", 4.0)] {
        let node = find_by_id(dom.root(), &dom, id).unwrap();
        let cv = page.computed(node).unwrap();
        for side in [
            cv.padding.top,
            cv.padding.right,
            cv.padding.bottom,
            cv.padding.left,
        ] {
            assert_eq!(side, ComputedLengthPercentage::Px(padding));
        }
    }
}
