use super::super::candidates::{BoxContext, CandidateFlowContext, ElementContext};

#[test]
fn box_context_preserves_overlapping_candidate_attributes() {
    let context = BoxContext::element(
        CandidateFlowContext {
            page_name_in_flow: true,
            ..Default::default()
        },
        ElementContext {
            is_direct_body_element: true,
            is_flex_item: true,
            is_named: true,
            ..Default::default()
        },
    );

    assert!(context.is_direct_body_element());
    assert!(context.is_flex_item());
    assert!(context.is_named());
    assert!(context.page_name_in_flow());
    assert!(!context.is_text());
}

#[test]
fn text_context_keeps_flow_attributes_without_element_attributes() {
    let context = BoxContext::text(CandidateFlowContext {
        is_float_descendant: true,
        is_out_of_flow_descendant: true,
        page_name_in_flow: true,
        lines_closing_edges: 7.0,
    });

    assert!(context.is_text());
    assert_eq!(context.lines_closing_edges(), 7.0);
    assert!(context.is_float_descendant());
    assert!(context.is_out_of_flow_descendant());
    assert!(context.page_name_in_flow());
    assert!(!context.is_named());
    assert!(!context.is_flex_item());
}
