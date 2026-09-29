use super::*;

#[test]
fn page_geometry_builders_preserve_metadata_and_page_index() {
    assert!(PageFragment::new().is_empty());
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 120.0;
    let margins = PageFragmentInsets::new(10.0, 11.0, 12.0, 13.0);
    let insets = PageFragmentInsets::new(2.0, 3.0, 4.0, 5.0);
    let content_box = PageFragmentRect::new(18.0, 12.0, 74.0, 94.0);
    let geometry = PageFragmentPageGeometry::new(
        0,
        page_box,
        margins,
        insets,
        content_box,
        PageFragmentOrientation::Portrait,
    );
    let shifted = geometry.with_page_index(3);
    assert_eq!(shifted.page_index, 3);
    assert_eq!(shifted.page_box, page_box);
    assert_eq!(shifted.content_box, content_box);

    let page = PageFragment::with_metadata(
        3,
        page_box,
        margins,
        insets,
        content_box,
        240.0,
        Some(String::from("named")),
        PageFragmentOrientation::Portrait,
    );
    assert_eq!(page.page_index, 3);
    assert_eq!(page.page_box, page_box);
    assert_eq!(page.margins, margins);
    assert_eq!(page.content_insets, insets);
    assert_eq!(page.content_box, content_box);
    assert_eq!(page.content_origin_y, 240.0);
    assert_eq!(page.page_name.as_deref(), Some("named"));
}

#[test]
fn fragment_item_split_and_repeat_semantics_are_distinct() {
    let split = PageFragmentItem::new(
        NodeId::new(7),
        PageFragmentRect::new(0.0, 0.0, 10.0, 5.0),
        PageFragmentKind::Box,
        0,
        2,
        false,
    );
    assert!(split.is_split());

    let repeat = PageFragmentItem::new(
        NodeId::new(7),
        PageFragmentRect::new(0.0, 0.0, 10.0, 5.0),
        PageFragmentKind::Box,
        0,
        2,
        true,
    );
    assert!(!repeat.is_split());
    assert!(!PageFragmentLineRange::new(0, 1).is_empty());
    assert!(PageFragmentLineRange::new(1, 1).is_empty());
}
