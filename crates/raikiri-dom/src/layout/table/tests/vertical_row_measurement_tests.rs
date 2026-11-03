use super::*;

fn fixture() -> (Document, raikiri_style::CascadeResult, usize) {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Some("display:grid;width:100px"));
    let table = append_grid_part(
        &mut doc,
        parent,
        "table",
        DisplayValue::Table,
        "display:table;writing-mode:vertical-rl;width:stretch;border-spacing:0",
    );
    for (hidden, width) in [(true, 20), (false, 30)] {
        let row = append_grid_part(
            &mut doc,
            table,
            "tr",
            DisplayValue::TableRow,
            if hidden {
                "display:table-row;visibility:collapse"
            } else {
                "display:table-row"
            },
        );
        append_grid_part(
            &mut doc,
            row,
            "td",
            DisplayValue::TableCell,
            &format!("display:table-cell;width:{width}px;height:10px;padding:0"),
        );
    }
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    (doc, computed, table)
}

#[test]
fn vertical_row_measurement_intrinsic_stretch_does_not_invent_parent_room() {
    for (available, expected) in [
        (AvailableSpace::MinContent, 30.0),
        (AvailableSpace::MaxContent, 30.0),
        (AvailableSpace::Definite(100.0), 60.0),
    ] {
        let (mut doc, computed, table) = fixture();
        let mut tree = LayoutSession::bridged(&mut doc, &computed);
        assert!(super::super::item_owns_width(&tree, table));
        let mut inputs = column_probe_input();
        inputs.available_space.width = available;
        let output =
            super::super::compute_table_layout_checked(&mut tree, table.into(), inputs).unwrap();
        assert_eq!(output.size.width, expected);
    }
}

#[test]
fn vertical_row_measurement_a_detached_table_has_no_parent_width_owner() {
    let (mut doc, computed, table) = fixture();
    assert!(doc.detach_from_parent(table).is_some());
    let tree = LayoutSession::bridged(&mut doc, &computed);
    assert_eq!(tree.taffy_parent_of(table), None);
    assert!(super::super::table_writing_mode(&tree, table).is_vertical());
    assert!(!super::super::item_owns_width(&tree, table));
}
