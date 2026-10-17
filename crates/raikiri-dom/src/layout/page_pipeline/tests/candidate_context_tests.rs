use super::super::candidates::BoxContext;

#[test]
fn box_context_preserves_overlapping_candidate_attributes() {
    let context = BoxContext {
        is_direct_body_element: true,
        is_flex_item: true,
        is_named: true,
        page_name_in_flow: true,
        ..BoxContext::default()
    };

    assert!(context.is_direct_body_element);
    assert!(context.is_flex_item);
    assert!(context.is_named);
    assert!(context.page_name_in_flow);
}
