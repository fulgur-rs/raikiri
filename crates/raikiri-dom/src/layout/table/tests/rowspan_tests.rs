use super::*;

#[test]
fn rowspan_clamps_after_parsing_a_wide_integer() {
    for (value, expected) in [
        ("1", 1),
        ("2", 2),
        ("65533", 65533),
        ("65534", 65534),
        ("65535", 65534),
        ("65536", 65534),
        ("70000", 65534),
        ("4294967295", 65534),
        ("4294967296", 65534),
        ("18446744073709551616", 65534),
        ("99999999999999999999999999999999999999999999999999", 65534),
        ("+4294967296", 65534),
        ("00000000000000000000000000000000000000000000070000", 65534),
        ("+2", 2),
        ("0", u16::MAX),
        ("+0", u16::MAX),
        (
            "00000000000000000000000000000000000000000000000000",
            u16::MAX,
        ),
    ] {
        let mut doc = Document::new();
        let cell = doc.append_element(Some(0), "td", Style::default(), None::<&str>);
        doc.set_element_attribute(cell, "rowspan", value).unwrap();
        assert_eq!(super::super::get_rowspan(&doc, cell), expected, "{value}");
    }
}

#[test]
fn missing_or_invalid_rowspan_keeps_the_single_row_default() {
    let mut doc = Document::new();
    let cell = doc.append_element(Some(0), "td", Style::default(), None::<&str>);
    assert_eq!(super::super::get_rowspan(&doc, cell), 1);
    for value in [
        "",
        "-1",
        "invalid",
        "+",
        "++1",
        " 2",
        "2 ",
        "２",
        "4294967296x",
        "99999999999999999999999999999999999999999999999999x",
    ] {
        doc.set_element_attribute(cell, "rowspan", value).unwrap();
        assert_eq!(super::super::get_rowspan(&doc, cell), 1, "{value}");
    }
}

#[test]
fn oversized_rowspan_reserves_the_remaining_source_group() {
    for prepared in [false, true] {
        let mut doc = Document::new();
        let table = append_grid_part(&mut doc, 0, "table", DisplayValue::Table, "display:table");
        let group = append_grid_part(
            &mut doc,
            table,
            "tbody",
            DisplayValue::TableRowGroup,
            "display:table-row-group",
        );
        let row = append_grid_part(
            &mut doc,
            group,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        let span = append_grid_part(
            &mut doc,
            row,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        doc.set_element_attribute(span, "rowspan", "70000").unwrap();
        let row = append_grid_part(
            &mut doc,
            group,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        append_grid_part(
            &mut doc,
            row,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        let next_group = append_grid_part(
            &mut doc,
            table,
            "tbody",
            DisplayValue::TableRowGroup,
            "display:table-row-group",
        );
        let row = append_grid_part(
            &mut doc,
            next_group,
            "tr",
            DisplayValue::TableRow,
            "display:table-row",
        );
        append_grid_part(
            &mut doc,
            row,
            "td",
            DisplayValue::TableCell,
            "display:table-cell",
        );
        doc.mark_in_document_flags();
        if prepared {
            let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
            crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        }
        let grid = super::super::build_table_grid(&doc, table).unwrap();
        assert_eq!(grid.n_cols, 2);
        assert_eq!(
            grid.cells
                .iter()
                .map(|cell| (cell.row, cell.col_start, cell.row_span))
                .collect::<Vec<_>>(),
            [(0, 0, 2), (1, 1, 1), (2, 0, 1)]
        );
    }
}
