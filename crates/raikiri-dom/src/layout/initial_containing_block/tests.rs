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

#[test]
fn pagination_truncates_root_and_descendant_margins_from_used_boxes() {
    use crate::layout::test_support::absolute_rect;
    for (root_margin, body_margin, child_margin, expected_pages, expected_y) in [
        (10, 20, 40, 1, 50.0),
        (250, 8, 0, 3, 208.0),
        (250, 150, 0, 4, 300.0),
        (0, 250, 0, 3, 200.0),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some(format!("display:block;margin-top:{root_margin}px").as_str()),
        );
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some(format!("display:block;margin:{body_margin}px 0 0").as_str()),
        );
        let child = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("display:block;height:10px;margin-top:{child_margin}px").as_str()),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        let mut page = page_box_800x600();
        page.width = 100.0;
        page.height = 100.0;
        let pages = crate::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert_eq!(
            pages.len(),
            expected_pages,
            "root {root_margin}, body {body_margin}, child {child_margin}"
        );
        assert_eq!(
            absolute_rect(&doc, child).1,
            expected_y,
            "root {root_margin}, body {body_margin}, child {child_margin}"
        );
    }
}

#[test]
fn pagination_keeps_block_alignment_space_instead_of_truncating_it_as_a_margin() {
    use crate::layout::test_support::absolute_rect;
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;height:500px;align-content:center"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut page = page_box_800x600();
    page.height = 100.0;
    crate::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert_eq!(absolute_rect(&doc, body).1, 240.0);
}

#[test]
fn truncating_a_sibling_margin_shrinks_auto_height_body_and_root_boxes() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:20px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:10px;margin-top:1000px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut page = page_box_800x600();
    page.height = 100.0;
    let pages = crate::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert_eq!(doc.nodes[body].unrounded_layout.size.height, 110.0);
    assert_eq!(doc.nodes[html].unrounded_layout.size.height, 110.0);
    assert_eq!(pages.len(), 2);
}

#[test]
fn a_contents_root_keeps_its_blockified_box_properties_and_percentage_basis() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:contents;margin:10px;padding:5px;border:2px solid;width:240px;height:180px"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:20px;width:50%;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let root = &doc.nodes[html].unrounded_layout;
    assert_eq!(
        (
            root.location.x,
            root.location.y,
            root.size.width,
            root.size.height
        ),
        (10.0, 10.0, 254.0, 194.0)
    );
    assert_eq!(doc.nodes[body].unrounded_layout.size.width, 120.0);
    assert_eq!(
        crate::used_relative_offset(&doc, &cascade, &cascade.computed[body], body),
        (0.0, 0.0)
    );
}

#[test]
fn vertical_root_percentage_padding_uses_the_icb_inline_size() {
    for display in ["block", "inline"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(),
            Some("display:block;writing-mode:vertical-rl;width:80px;height:60px;padding:10%;font-family:Ahem;font-size:10px;line-height:10px"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some(format!("display:{display};width:30px;height:20px").as_str()),
        );
        doc.append_text(body, "A");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        let mut page = page_box_800x600();
        page.width = 200.0;
        page.height = 100.0;
        layout_single_page(with_ahem(&mut doc), &cascade, page).expect("layout");
        let padding = doc.nodes[html].unrounded_layout.padding;
        assert_eq!(
            (padding.top, padding.right, padding.bottom, padding.left),
            (10.0, 10.0, 10.0, 10.0),
            "body {display}"
        );
        assert_eq!(
            doc.nodes[html].unrounded_layout.size,
            taffy::Size {
                width: 100.0,
                height: 80.0
            },
            "body {display}"
        );
    }
}

#[test]
fn root_box_queries_blockify_contents_before_testing_fixed_containment() {
    use crate::layout::test_support::absolute_rect;
    for effect in ["transform:translateX(3px)", "contain:paint"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(format!(
            "display:contents;margin:10px;padding:5px;border:2px solid;width:240px;height:180px;{effect}"
        ).as_str()));
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
        let fixed = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("position:fixed;left:50%;top:50%;width:20px;height:10px"),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        assert!(
            crate::establishes_fixed_containing_block(&doc, html, &cascade.computed[html]),
            "{effect}"
        );
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        // The 250x190 root padding box starts at (12,12). Transforms paint
        // later, so its fixed descendant's layout center stays at (137,107).
        assert_eq!(
            absolute_rect(&doc, fixed),
            (137.0, 107.0, 20.0, 10.0),
            "{effect}"
        );
    }
}

#[test]
fn relative_root_offsets_do_not_change_normal_flow_page_breaks() {
    use crate::layout::test_support::absolute_rect;
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;position:relative;top:20px"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let first = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:70px;break-inside:avoid"),
    );
    let second = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:70px;break-inside:avoid"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut page = page_box_800x600();
    page.height = 100.0;
    let pages = crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert_eq!(pages.len(), 2);
    assert_eq!(absolute_rect(&doc, first).1, 20.0);
    assert_eq!(absolute_rect(&doc, second).1, 120.0);
}

#[test]
fn moving_a_direct_body_child_grows_the_actual_auto_height_boxes() {
    for boundary in ["break-inside:avoid", "break-before:page"] {
        for (body_height, expected_height) in [
            ("", 170.0),
            ("height:100px", 100.0),
            ("max-height:120px", 120.0),
        ] {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
            let body = doc.append_element(
                Some(html),
                "body",
                Style::default(),
                Some(format!("display:block;margin:0;{body_height}").as_str()),
            );
            doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:block;height:70px;break-inside:avoid"),
            );
            let second = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some(format!("display:block;height:70px;{boundary}").as_str()),
            );
            let rules = build_rule_tree(&doc);
            let cascade = cascade(&doc, &rules).expect("cascade");
            let mut page = page_box_800x600();
            page.height = 100.0;
            let pages =
                crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
            assert_eq!(pages.len(), 2, "{body_height}");
            assert_eq!(
                crate::layout::test_support::absolute_rect(&doc, second).1,
                100.0,
                "{body_height}"
            );
            assert_eq!(
                doc.nodes[body].unrounded_layout.size.height, expected_height,
                "body {body_height}"
            );
            assert_eq!(
                doc.nodes[html].unrounded_layout.size.height, expected_height,
                "html {body_height}"
            );
        }
    }
}

#[test]
fn blockified_internal_root_displays_keep_vertical_box_geometry() {
    for display in [
        "block",
        "inline",
        "contents",
        "table-cell",
        "table-row",
        "table-caption",
        "table-column",
        "ruby-base",
        "ruby-text",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(format!(
            "display:{display};writing-mode:vertical-rl;width:80px;height:60px;padding:10%;margin:0"
        ).as_str()));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0;width:30px;height:20px"),
        );
        let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
        let mut page = page_box_800x600();
        page.width = 200.0;
        page.height = 100.0;
        layout_single_page(with_ahem(&mut doc), &cascade, page).expect("layout");
        assert_eq!(
            crate::layout::test_support::absolute_rect(&doc, html),
            (100.0, 0.0, 100.0, 80.0),
            "root {display}"
        );
        assert_eq!(
            crate::layout::test_support::absolute_rect(&doc, body),
            (160.0, 10.0, 30.0, 20.0),
            "body under {display}"
        );
    }
}

#[test]
fn root_blockification_preserves_flex_and_grid_inner_layout() {
    for (display, second_origin) in [
        ("block", (17.0, 37.0)),
        ("inline-block", (17.0, 37.0)),
        ("flex", (47.0, 17.0)),
        ("inline-flex", (47.0, 17.0)),
        ("grid", (47.0, 17.0)),
        ("inline-grid", (47.0, 17.0)),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(format!(
            "display:{display};margin:10px;padding:5px;border:2px solid;width:120px;height:80px;grid-template-columns:30px 20px"
        ).as_str()));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0;width:30px;height:20px;flex-shrink:0"),
        );
        let second = doc.append_element(
            Some(html),
            "section",
            Style::default(),
            Some("display:block;width:20px;height:15px;flex-shrink:0"),
        );
        let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            crate::layout::test_support::absolute_rect(&doc, html),
            (10.0, 10.0, 134.0, 94.0),
            "{display}"
        );
        assert_eq!(
            crate::layout::test_support::absolute_rect(&doc, body),
            (17.0, 17.0, 30.0, 20.0),
            "{display}"
        );
        let rect = crate::layout::test_support::absolute_rect(&doc, second);
        assert_eq!((rect.0, rect.1), second_origin, "{display}");
    }
}

#[test]
fn percentage_heights_follow_the_authored_definite_root_and_body_chain() {
    for (root_height, html_height, body_height, child_height) in
        [("100%", 600.0, 300.0, 150.0), ("50%", 300.0, 150.0, 75.0)]
    {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some(format!("display:block;height:{root_height}").as_str()),
        );
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0;height:50%"),
        );
        let child = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:50%"),
        );
        let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(doc.nodes[html].unrounded_layout.size.height, html_height);
        assert_eq!(doc.nodes[body].unrounded_layout.size.height, body_height);
        assert_eq!(doc.nodes[child].unrounded_layout.size.height, child_height);
    }
}

#[test]
fn absolute_overflow_page_count_includes_the_used_root_offset() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin-top:20px;height:0"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;height:0"),
    );
    let absolute = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;position:absolute;top:199px;left:0;width:10px;height:2px"),
    );
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    let mut page = page_box_800x600();
    page.height = 100.0;
    let pages = crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert_eq!(
        crate::layout::test_support::absolute_rect(&doc, absolute),
        (0.0, 199.0, 10.0, 2.0)
    );
    assert_eq!(pages.len(), 3);
}

#[test]
fn viewport_overflow_propagation_caps_absolute_overflow_pages() {
    for (root_overflow, body_overflow, expected_pages) in [
        ("visible", "visible", 3),
        ("hidden", "visible", 1),
        ("visible", "hidden", 1),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(
            Some(0),
            "html",
            Style::default(),
            Some(format!("display:block;height:0;overflow:{root_overflow}").as_str()),
        );
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some(format!("display:block;height:0;margin:0;overflow:{body_overflow}").as_str()),
        );
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("position:absolute;top:199px;width:10px;height:2px"),
        );
        let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
        let mut page = page_box_800x600();
        page.height = 100.0;
        let pages =
            crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert_eq!(
            pages.len(),
            expected_pages,
            "html {root_overflow}, body {body_overflow}"
        );
    }
}
