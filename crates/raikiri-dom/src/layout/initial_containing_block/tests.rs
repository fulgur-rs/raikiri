use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

#[test]
fn authored_html_and_body_sizes_are_laid_out_inside_the_page_area() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin:10px;padding:5px;border:2px solid;width:240px;height:180px"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:20px;padding:3px;border:1px solid;width:100px;height:50px"),
    );
    let child = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:50%;height:50%"),
    );
    let count = doc.node_count();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let root = &doc.nodes[html].unrounded_layout;
    assert_eq!((root.location.x, root.location.y), (10.0, 10.0));
    assert_eq!((root.size.width, root.size.height), (254.0, 194.0));
    let body_box = &doc.nodes[body].unrounded_layout;
    assert_eq!((body_box.location.x, body_box.location.y), (27.0, 27.0));
    assert_eq!((body_box.size.width, body_box.size.height), (108.0, 58.0));
    let child_box = &doc.nodes[child].unrounded_layout;
    assert_eq!((child_box.location.x, child_box.location.y), (4.0, 4.0));
    assert_eq!((child_box.size.width, child_box.size.height), (50.0, 25.0));
    assert_eq!(doc.node_count(), count);
    assert_eq!(doc.parent_of(html), Some(0));
    assert_eq!(doc.parent_of(body), Some(html));
}

#[test]
fn root_margins_add_to_the_margins_collapsing_through_body() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin:10px"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:20px"),
    );
    let child = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:30px;margin-top:40px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(doc.nodes[html].unrounded_layout.location.y, 10.0);
    assert_eq!(doc.nodes[body].unrounded_layout.location.y, 40.0);
    assert_eq!(doc.nodes[child].unrounded_layout.location.y, 0.0);
    assert_eq!(doc.nodes[html].unrounded_layout.size.height, 90.0);
    assert_eq!(doc.nodes[body].unrounded_layout.size.height, 30.0);
}

#[test]
fn positioned_descendants_use_the_root_padding_box_or_the_page_area() {
    use crate::layout::test_support::absolute_rect;
    for (root_position, position, expected) in [
        ("static", "absolute", (400.0, 300.0, 20.0, 10.0)),
        ("relative", "absolute", (122.0, 72.0, 20.0, 10.0)),
        ("relative", "fixed", (400.0, 300.0, 20.0, 10.0)),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(format!(
            "display:block;position:{root_position};margin:10px;padding:10px;border:2px solid;width:200px;height:100px"
        ).as_str()));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:7px;width:80px;height:40px"),
        );
        let child = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(
                format!(
                    "display:block;position:{position};left:50%;top:50%;width:20px;height:10px"
                )
                .as_str(),
            ),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            absolute_rect(&doc, child),
            expected,
            "{root_position}, {position}"
        );
    }
}

#[test]
fn the_icb_uses_the_root_direction_for_a_definite_root_width() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;direction:rtl;width:300px"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;direction:ltr;width:200px;height:30px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(doc.nodes[html].unrounded_layout.location.x, 500.0);
    assert_eq!(doc.nodes[body].unrounded_layout.location.x, 100.0);
}

#[test]
fn the_canvas_root_box_uses_the_laid_out_content_height() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin:10px;padding:5px"),
    );
    doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:20px;padding:10px;height:50px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let page = page_box_800x600();
    layout_single_page(with_ahem(&mut doc), &cascade, page).expect("layout");
    let root = crate::root_box(&doc, &cascade, page, 0.0, 1).expect("canvas root");
    assert_eq!(
        root.border_box,
        raikiri_traits::PaintRect::new(10.0, 10.0, 780.0, 120.0)
    );
    assert_eq!(
        root.padding,
        raikiri_traits::PaintInsets::new(5.0, 5.0, 5.0, 5.0)
    );
}

#[test]
fn source_growth_and_anonymous_table_rebuild_keep_the_icb_outside_the_dom() {
    use crate::layout::test_support::absolute_rect;
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin:10px"),
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let table = doc.append_element(Some(body), "table", Style::default(), Some("display:table"));
    let row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let first = cascade(&doc, &rules).expect("first cascade");
    layout_single_page(with_ahem(&mut doc), &first, page_box_800x600()).expect("first layout");
    assert_eq!(doc.node_count(), 6);
    assert_eq!(absolute_rect(&doc, table), (10.0, 10.0, 20.0, 10.0));
    assert_eq!(doc.anonymous_table_cells(row).count(), 1);
    let icb = doc.nodes.initial_containing_block_key().expect("ICB");
    assert!(doc.get_node(icb).is_none());
    let child = doc.append_element(
        Some(row),
        "div",
        Style::default(),
        Some("display:block;width:20px;height:20px"),
    );
    assert_eq!(child, 6);
    let rules = build_rule_tree(&doc);
    let second = cascade(&doc, &rules).expect("second cascade");
    layout_single_page(&mut doc, &second, page_box_800x600()).expect("second layout");
    assert_eq!(doc.node_count(), 7);
    assert_eq!(doc.nodes.initial_containing_block_key(), Some(icb));
    assert!(doc.get_node(icb).is_none());
    assert_eq!(doc.parent_of(html), Some(0));
    assert_eq!(absolute_rect(&doc, table), (10.0, 10.0, 20.0, 30.0));
    assert_eq!(absolute_rect(&doc, child), (10.0, 20.0, 20.0, 20.0));
}
