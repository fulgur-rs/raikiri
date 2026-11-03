use super::super::{PART_ROW_VISITS, TableGrid, position_source_table_parts};
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

/// The lists the walk over every row of every cell gives, which the lookup by
/// row must reproduce: each part a cell reaches through the rows it spans
/// takes the cell once, in cell order.
fn backgrounds_by_visiting_every_row(
    doc: &mut Document,
    table: usize,
    grid: &TableGrid,
    parts: &HashMap<usize, BoxLayout>,
) -> HashMap<usize, Vec<PaintRect>> {
    let mut lists: HashMap<usize, Vec<PaintRect>> =
        parts.keys().map(|&id| (id, Vec::new())).collect();
    with_layout_session(doc, |session| {
        let boundary = session.table_parts_boundary(table);
        for cell in &grid.cells {
            let layout = cell.resolved.expect("resolved cell");
            let mut listed = std::collections::HashSet::new();
            let end = (usize::from(cell.row) + usize::from(cell.row_span)).min(grid.rows.len());
            for &marker in &grid.rows[usize::from(cell.row)..end] {
                let mut id = Some(marker);
                while let Some(current) = id {
                    if current == boundary {
                        break;
                    }
                    if let Some(part) = parts.get(&current)
                        && listed.insert(current)
                    {
                        let left = part.location.x.max(layout.location.x);
                        let top = part.location.y.max(layout.location.y);
                        let right = (part.location.x + part.size.width)
                            .min(layout.location.x + layout.size.width);
                        let bottom = (part.location.y + part.size.height)
                            .min(layout.location.y + layout.size.height);
                        if right > left && bottom > top {
                            lists.get_mut(&current).unwrap().push(PaintRect::new(
                                left - part.location.x,
                                top - part.location.y,
                                right - left,
                                bottom - top,
                            ));
                        }
                    }
                    id = session.parent_of(current);
                }
            }
        }
    });
    lists
}

#[test]
fn the_lists_of_part_backgrounds_match_a_visit_of_every_row_of_every_cell() {
    let mut state = 0x9e37_79b9_u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let mut compared = 0;
    for case in 0..60 {
        let mut doc = Document::new();
        let element = |doc: &mut Document, parent, tag, display| {
            let id = doc.append_element(Some(parent), tag, None::<&str>);
            doc.set_display(id, display);
            id
        };
        let table = element(&mut doc, 0, "table", DisplayValue::Table);
        let group_count = 1 + next() as usize % 3;
        let mut rows = Vec::new();
        let mut parts = HashMap::new();
        let mut y = 0.0;
        for _ in 0..group_count {
            let group = element(&mut doc, table, "tbody", DisplayValue::TableRowGroup);
            let group_top = y;
            for _ in 0..1 + next() as usize % 4 {
                let row = element(&mut doc, group, "tr", DisplayValue::TableRow);
                // Some rows have no height, as an empty row has none.
                let height = [0.0, 10.0, 20.0][next() as usize % 3];
                let mut layout = BoxLayout::new();
                layout.location.y = y;
                layout.size = Size {
                    width: 60.0,
                    height,
                };
                parts.insert(row, layout);
                rows.push(row);
                y += height;
            }
            let mut layout = BoxLayout::new();
            layout.location.y = group_top;
            layout.size = Size {
                width: 60.0,
                height: y - group_top,
            };
            parts.insert(group, layout);
        }
        let mut layout = BoxLayout::new();
        layout.size = Size {
            width: 60.0,
            height: y,
        };
        parts.insert(table, layout);
        let mut cells = Vec::new();
        for column in 0..1 + next() as usize % 4 {
            let row = next() as usize % rows.len();
            let span = 1 + next() as usize % (rows.len() - row + 1);
            let node = element(&mut doc, rows[row], "td", DisplayValue::TableCell);
            let mut cell = make_cell(
                node,
                row as u16,
                column as u16,
                1,
                span as u16,
                Dimension::auto(),
            );
            let top = [0.0, 5.0, 10.0][next() as usize % 3];
            let mut resolved = BoxLayout::new();
            resolved.location.x = 20.0 * column as f32;
            resolved.location.y = top;
            resolved.size = Size {
                width: 20.0,
                height: [0.0, 15.0, 40.0][next() as usize % 3],
            };
            cell.resolved = Some(resolved);
            cells.push(cell);
        }
        let grid = TableGrid {
            n_cols: 4,
            rows,
            cells,
            col_widths: vec![],
        };
        let expected = backgrounds_by_visiting_every_row(&mut doc, table, &grid, &parts);
        with_layout_session(&mut doc, |session| {
            position_source_table_parts(session, table, &grid, Some(parts.clone()))
        });
        for (&id, rects) in &expected {
            assert_eq!(
                &doc.table_objects.part_background_cells[&id], rects,
                "case {case}, part {id}"
            );
            compared += rects.len();
        }
    }
    assert!(compared > 100, "the cases must hold rectangles to compare");
}

#[test]
fn empty_rows_of_one_group_are_visited_once_however_many_cells_span_them() {
    let rows_count = 40;
    let cells_count = 30;
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "table", None::<&str>);
    doc.set_display(table, DisplayValue::Table);
    let group = doc.append_element(Some(table), "tbody", None::<&str>);
    doc.set_display(group, DisplayValue::TableRowGroup);
    let rows: Vec<usize> = (0..rows_count)
        .map(|_| {
            let row = doc.append_element(Some(group), "tr", None::<&str>);
            doc.set_display(row, DisplayValue::TableRow);
            row
        })
        .collect();
    let mut cells = Vec::new();
    for column in 0..cells_count {
        let node = doc.append_element(Some(rows[0]), "td", None::<&str>);
        doc.set_display(node, DisplayValue::TableCell);
        let mut cell = make_cell(node, 0, column, 1, rows_count as u16, Dimension::auto());
        let mut resolved = BoxLayout::new();
        resolved.location.x = 5.0 * f32::from(column);
        resolved.size = Size {
            width: 5.0,
            height: 10.0,
        };
        cell.resolved = Some(resolved);
        cells.push(cell);
    }
    let mut rect = BoxLayout::new();
    rect.size = Size {
        width: 150.0,
        height: 10.0,
    };
    // The first row is the only one with a height; the rest are empty.
    let mut parts = HashMap::from([(table, rect), (group, rect), (rows[0], rect)]);
    for &row in &rows[1..] {
        let mut empty = rect;
        empty.location.y = 10.0;
        empty.size.height = 0.0;
        parts.insert(row, empty);
    }
    let grid = TableGrid {
        n_cols: cells_count,
        rows,
        cells,
        col_widths: vec![],
    };
    PART_ROW_VISITS.set(0);
    with_layout_session(&mut doc, |session| {
        position_source_table_parts(session, table, &grid, Some(parts))
    });
    // The first row, then one visit for the run of empty rows after it.
    assert_eq!(PART_ROW_VISITS.get(), 2 * usize::from(cells_count));
    // Each cell still reaches the group and the first row.
    assert_eq!(
        doc.table_objects.part_background_cells[&group].len(),
        usize::from(cells_count)
    );
}
