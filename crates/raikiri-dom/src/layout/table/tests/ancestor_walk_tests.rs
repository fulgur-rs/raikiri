use super::super::{TableGrid, position_source_table_parts};
use super::*;
use raikiri_layout_geometry::BoxLayout;
use raikiri_traits::PaintRect;
use std::collections::HashMap;

#[test]
fn a_rowspan_registers_its_shared_group_once_and_excludes_the_table_boundary() {
    let mut doc = Document::new();
    let mut element = |parent, tag, display| {
        let id = doc.append_element(Some(parent), tag, None::<&str>);
        doc.set_display(id, display);
        id
    };
    let table = element(0, "table", DisplayValue::Table);
    let group = element(table, "tbody", DisplayValue::TableRowGroup);
    let first = element(group, "tr", DisplayValue::TableRow);
    let second = element(group, "tr", DisplayValue::TableRow);
    let cell = element(first, "td", DisplayValue::TableCell);
    let rect = |y, height| {
        let mut layout = BoxLayout::new();
        layout.location.y = y;
        layout.size = Size {
            width: 20.0,
            height,
        };
        layout
    };
    let mut spanning = make_cell(cell, 0, 0, 1, 2, Dimension::auto());
    spanning.resolved = Some(rect(0.0, 40.0));
    let grid = TableGrid {
        n_cols: 1,
        rows: vec![first, second],
        cells: vec![spanning],
        col_widths: vec![],
    };
    let parts = HashMap::from([
        (table, rect(0.0, 40.0)),
        (group, rect(0.0, 40.0)),
        (first, rect(0.0, 20.0)),
        (second, rect(20.0, 20.0)),
    ]);
    with_layout_session(&mut doc, |session| {
        position_source_table_parts(session, table, &grid, Some(parts))
    });
    assert_eq!(doc.table_objects.part_background_cells[&table], []);
    assert_eq!(
        doc.table_objects.part_background_cells[&group],
        [PaintRect::new(0.0, 0.0, 20.0, 40.0)]
    );
    assert_eq!(
        doc.table_objects.part_background_cells[&first],
        [PaintRect::new(0.0, 0.0, 20.0, 20.0)]
    );
    assert_eq!(
        doc.table_objects.part_background_cells[&second],
        [PaintRect::new(0.0, 0.0, 20.0, 20.0)]
    );
}
