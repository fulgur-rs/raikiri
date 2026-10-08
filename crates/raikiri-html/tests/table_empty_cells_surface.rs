//! Empty and block-content cells keep parent-relative page coordinates.

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
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:10px/10px Ahem}}table{{border-spacing:0}}td{{padding:0;width:10px;height:10px}}td>div{{width:10px;height:10px}}</style>{table}"
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
fn empty_cells_follow_their_rows_without_a_second_row_offset() {
    let document = lay_out(
        "<table><tr><td id='a'></td></tr><tr><td id='b'></td></tr><tr><td id='c'></td></tr></table>",
    );
    for (id, y) in [("a", 0.0), ("b", 10.0), ("c", 20.0)] {
        assert_eq!(rect(&document, id).y, y);
        assert_eq!(rect(&document, id).height, 10.0);
    }
}

#[test]
fn block_content_cells_follow_their_rows_without_a_second_row_offset() {
    let document = lay_out(
        "<table><tr><td id='a'><div></div></td></tr><tr><td id='b'><div></div></td></tr><tr><td id='c'><div></div></td></tr></table>",
    );
    for (id, y) in [("a", 0.0), ("b", 10.0), ("c", 20.0)] {
        assert_eq!(rect(&document, id).y, y);
    }
}

#[test]
fn mixed_empty_and_text_cells_share_the_same_row_coordinates() {
    let document = lay_out(
        "<table><tr><td id='a'></td><td>A</td></tr><tr><td id='b'>B</td><td id='empty'></td></tr><tr><td id='c'></td><td>C</td></tr></table>",
    );
    assert_eq!(rect(&document, "a").y, 0.0);
    assert_eq!(rect(&document, "b").y, 10.0);
    assert_eq!(rect(&document, "empty").y, 10.0);
    assert_eq!(rect(&document, "c").y, 20.0);
}

#[test]
fn empty_cells_keep_row_and_group_offsets_separate() {
    let document = lay_out(
        "<table><tbody><tr><td id='a'></td></tr></tbody><tbody><tr><td id='b'></td></tr><tr><td id='c'></td></tr></tbody></table>",
    );
    assert_eq!(rect(&document, "a").y, 0.0);
    assert_eq!(rect(&document, "b").y, 10.0);
    assert_eq!(rect(&document, "c").y, 20.0);
}
