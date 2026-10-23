//! Named grid areas use every edge supplied by the placement shorthand.

use raikiri_html::{
    DocumentLayout, DomView, LayoutConfig, LayoutOptions, LayoutStatus, NodeId, PageDefaults,
    PaintRect, RenderResources, layout, parse_html_with_resources,
};

fn find_id(dom: &DomView<'_>, node: NodeId, id: &str) -> Option<NodeId> {
    if dom.attr(node, "id") == Some(id) {
        return Some(node);
    }
    dom.children(node).find_map(|child| find_id(dom, child, id))
}

fn rect(result: &DocumentLayout, id: &str) -> PaintRect {
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let node = find_id(&dom, dom.root(), id).unwrap();
    page.fragments()
        .find(|fragment| fragment.node() == node)
        .unwrap()
        .rect()
}

#[test]
fn grid_area_shorthand_places_named_items_in_inherited_areas() {
    for values in [
        ["a", "b", "c"],
        ["a / a", "b / b", "c / c"],
        ["a / a / a", "b / b / b", "c / c / c"],
        ["a / a / a / a", "b / b / b / b", "c / c / c / c"],
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0}}.wrapper{{grid-template-areas:'a b' 'a c'}}.grid{{display:grid;grid-template-areas:inherit;grid-template-rows:50px 50px;grid-template-columns:40px 60px;width:100px;height:100px}}</style><div class=wrapper><div class=grid><div id=a style='grid-area:{}'></div><div id=b style='grid-area:{}'></div><div id=c style='grid-area:{}'></div></div></div>",
            values[0], values[1], values[2]
        );
        let resources = RenderResources::new();
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
        assert_eq!(result.pages().len(), 1);
        for (id, expected) in [
            ("a", (0.0, 0.0, 40.0, 100.0)),
            ("b", (40.0, 0.0, 60.0, 50.0)),
            ("c", (40.0, 50.0, 60.0, 50.0)),
        ] {
            let actual = rect(&result, id);
            assert_eq!(
                (actual.x, actual.y, actual.width, actual.height),
                expected,
                "{values:?} {id}"
            );
        }
    }
}
