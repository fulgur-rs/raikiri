use super::*;

#[test]
fn vertical_cell_maximum_height_limits_inline_text_wrapping() {
    use crate::layout::test_support::{page_box_800x600, with_ahem};

    for mode in ["vertical-rl", "vertical-lr"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block"));
        let table = doc.append_element(Some(body), "div", Some("display:table;border-spacing:0"));
        let row = doc.append_element(Some(table), "div", Some("display:table-row"));
        let cell = doc.append_element(
            Some(row),
            "div",
            Some(&format!(
                "display:table-cell;max-height:100px;font:50px/50px Ahem;writing-mode:{mode}"
            )),
        );
        doc.append_text(cell, "xx xx");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        with_ahem(&mut doc);
        crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        assert_eq!(doc.nodes[cell].layout.size.width, 100.0, "{mode}");
        assert_eq!(doc.nodes[cell].layout.size.height, 100.0, "{mode}");
    }
}

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
