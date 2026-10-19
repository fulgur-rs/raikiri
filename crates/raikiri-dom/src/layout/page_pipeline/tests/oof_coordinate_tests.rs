use super::*;
use crate::layout::test_support::page_box_800x600;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

#[test]
fn percentage_width_under_page_positioned_box_uses_its_owner_page() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(
        Some(html),
        "div",
        Style::default(),
        Some("display:block;height:20px"),
    );
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let positioned = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute;top:90px;left:0;right:0;height:20px"),
    );
    let child = document.append_element(
        Some(positioned),
        "div",
        Style::default(),
        Some("display:block;width:50%;height:10px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let mut page_box = page_box_800x600();
    page_box.height = 100.0;

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box,
        &[],
        &[800.0, 400.0],
        &PageLayoutControl::default(),
    )
    .expect("paged layout");

    assert_eq!(pages.len(), 2);
    assert_eq!(document.layout_origin_of(child).unwrap().y, 90.0);
    assert_eq!(document.nodes[child].unrounded_layout.size.width, 400.0);
    assert_eq!(
        document.layout_offset_of(child),
        Some(document.nodes[child].unrounded_layout.location)
    );
    assert_eq!(current_abs_y(&document, child), 90.0);

    let local_offset = document.nodes[child].unrounded_layout.location;
    let coordinate = document.oof_coordinates.get(&positioned).unwrap();
    assert!(coordinate.initial_owner_origin.is_some());
    assert!(coordinate.initial_static_origin.is_some());
    document.begin_oof_layout();
    let coordinate = document.oof_coordinates.get(&positioned).unwrap();
    assert_eq!(coordinate.initial_owner_origin, None);
    assert_eq!(coordinate.initial_static_origin, None);
    assert_eq!(document.layout_offset_of(child), Some(local_offset));

    document.finalize_oof_layout();
    assert_eq!(document.layout_origin_of(child).unwrap().y, 90.0);
}
