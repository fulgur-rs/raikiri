use super::*;

#[test]
fn cell_content_maximum_scope_restores_limits_and_separates_cached_sizes() {
    let mut doc = Document::new();
    let cell = doc.append_element(
        Some(0),
        "div",
        Some("display:table-cell;max-height:50px;max-width:37px"),
    );
    let child = doc.append_element(
        Some(cell),
        "div",
        Some("display:block;width:20px;height:100px;max-height:80px"),
    );
    doc.mark_in_document_flags();
    with_layout_session(&mut doc, |session| {
        let mut inputs = column_probe_input();
        inputs.known_dimensions.width = Some(30.0);
        inputs.known_dimensions_are_definite.width = true;
        for _ in 0..3 {
            let ordinary = session.compute_child_layout(cell.into(), inputs);
            assert_eq!(ordinary.size.height, 50.0);
            let contents = super::super::layout_cell_contents(session, cell, inputs);
            assert_eq!(contents.size.height, 80.0);
            assert_eq!(session.nodes[child].layout.size.height, 80.0);
            let restored = session.compute_child_layout(cell.into(), inputs);
            assert_eq!(restored.size.height, 50.0);
            assert_eq!(session.override_checkpoint(), 0);
            assert_eq!(
                session.physical_style(cell).max_size(),
                Size {
                    width: LengthPercentageAuto::length(37.0),
                    height: LengthPercentageAuto::length(50.0),
                }
            );
            assert_eq!(
                session.physical_style(child).max_size().height,
                LengthPercentageAuto::length(80.0)
            );
        }
    });
}
