use super::*;

#[test]
fn options_default_produces_one_page() {
    let (mut document, cascade) = hello_world_doc();
    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box_800x600(),
        PageLayoutOptions::default(),
    )
    .unwrap();
    assert_eq!(
        pages,
        vec![PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }]
    );
}
