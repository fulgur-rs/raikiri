use super::*;

#[test]
fn initial_page_context_errors_map_to_render_errors() {
    let layout_error = map_initial_page_context_error(InitialPageContextError::Layout(
        raikiri_traits::LayoutError::Internal {
            message: "test".to_owned(),
        },
    ));
    assert!(matches!(layout_error, RenderError::Layout(_)));
    assert!(matches!(
        map_initial_page_context_error(InitialPageContextError::PageGeometryDidNotConverge {
            iterations: 3
        }),
        RenderError::PageGeometryDidNotConverge { iterations: 3 }
    ));
}

mod pipeline_tests;
