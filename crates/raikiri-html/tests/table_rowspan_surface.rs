//! Row-spanning cells keep distinct columns through the public page geometry.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, PaintRect,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(table: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:10px/10px Ahem}}table{{border-spacing:0}}td{{padding:0}}td>div{{width:10px;height:10px}}</style>{table}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
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

fn rect(document: &DocumentLayout, id: &str) -> PaintRect {
    let page = document.page(0).unwrap();
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, id).unwrap();
    page.fragments()
        .find(|fragment| fragment.node() == node)
        .unwrap()
        .rect()
}

#[test]
fn a_rowspan_reserves_its_column_and_releases_it_after_the_last_row() {
    let document = lay_out(
        "<table><tr><td id='span' rowspan='2'><div style='height:20px'></div></td><td id='b'><div></div></td></tr><tr><td id='c'><div></div></td></tr><tr><td id='d'><div></div></td><td id='e'><div></div></td></tr></table>",
    );
    assert_eq!(rect(&document, "span").x, 0.0);
    assert_eq!(rect(&document, "b").x, 10.0);
    assert_eq!(rect(&document, "c").x, 10.0);
    assert_eq!(rect(&document, "d").x, 0.0);
    assert_eq!(rect(&document, "e").x, 10.0);
}

#[test]
fn a_rowspan_with_colspan_reserves_every_covered_column() {
    let document = lay_out(
        "<table><tr><td id='span' rowspan='2' colspan='2'><div style='width:20px;height:20px'></div></td><td id='b'><div></div></td></tr><tr><td id='c'><div></div></td></tr></table>",
    );
    assert_eq!(rect(&document, "span").width, 20.0);
    assert_eq!(rect(&document, "b").x, 20.0);
    assert_eq!(rect(&document, "c").x, 20.0);
}

#[test]
fn a_reserved_middle_column_does_not_displace_the_free_column_before_it() {
    let document = lay_out(
        "<table><tr><td id='a'><div></div></td><td id='span' rowspan='2'><div style='height:20px'></div></td><td id='b'><div></div></td></tr><tr><td id='c'><div></div></td><td id='d'><div></div></td></tr></table>",
    );
    assert_eq!(rect(&document, "span").x, 10.0);
    assert_eq!(rect(&document, "c").x, 0.0);
    assert_eq!(rect(&document, "d").x, 20.0);
}

#[test]
fn a_rowspan_does_not_reserve_columns_in_the_next_row_group() {
    for span in ["0", "2", "20"] {
        let document = lay_out(&format!(
            "<table><tbody><tr><td id='span' rowspan='{span}'><div></div></td><td id='b'><div></div></td></tr></tbody><tbody><tr><td id='c'><div></div></td><td id='d'><div></div></td></tr></tbody></table>"
        ));
        assert_eq!(rect(&document, "c").x, 0.0);
        assert_eq!(rect(&document, "d").x, 10.0);
        assert_eq!(rect(&document, "span").height, 10.0);
    }
}
