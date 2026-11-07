use super::*;
use crate::layout::test_support::absolute_rect;

/// Lays out a 60px block, then a table whose row group holds rows of the
/// given heights, then a 10px block, and returns the ids of
/// `[table, group, rows..., after]`. A `caption` style adds a 20px caption.
fn table_after_spacer(
    table_css: &str,
    caption: Option<&str>,
    group_css: &str,
    rows: &[f32],
) -> (Document, Vec<usize>) {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some(&format!(
                "display:table;width:100px;border-spacing:2px;{table_css}"
            )),
        );
        if let Some(caption_css) = caption {
            doc.append_element(
                Some(table),
                "caption",
                Some(&format!("display:table-caption;height:20px;{caption_css}")),
            );
        }
        let group = doc.append_element(
            Some(table),
            "tbody",
            Some(&format!("display:table-row-group;{group_css}")),
        );
        ids.push(table);
        ids.push(group);
        for &height in rows {
            ids.push(append_row(doc, group, "", height).0);
        }
        ids.push(doc.append_element(Some(body), "div", Some("display:block;height:10px")));
    });
    (doc, ids)
}

/// The block-start and block-end page coordinates of `node`'s box.
fn span(doc: &Document, node: usize) -> (f32, f32) {
    let (_, y, _, height) = absolute_rect(doc, node);
    (y, y + height)
}

#[test]
fn a_table_grows_over_its_last_row_moved_to_the_next_page() {
    let (doc, ids) = table_after_spacer("", None, "", &[20.0, 30.0]);
    let [table, group, first, last, after] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, first), (62.0, 82.0));
    assert_eq!(span(&doc, last), (100.0, 130.0));
    // The row group ends with the moved row, and the table keeps its spacing
    // below it.
    assert_eq!(span(&doc, group), (62.0, 130.0));
    assert_eq!(span(&doc, table), (60.0, 132.0));
    assert_eq!(span(&doc, after), (132.0, 142.0));
    // The group's background is clipped to its cells where they are now.
    let clips = &doc.table_objects.part_background_cells[&group];
    assert!(
        clips
            .iter()
            .any(|cell| (cell.y - 38.0).abs() < 0.001 && (cell.height - 30.0).abs() < 0.001),
        "{clips:?}"
    );
}

#[test]
fn a_table_moves_with_its_first_row_and_the_border_above_it() {
    let (doc, ids) = table_after_spacer("border:3px solid", None, "", &[50.0, 10.0]);
    let [table, group, first, last, after] = ids[..] else {
        unreachable!()
    };
    // The border and spacing above the first row start the next page with
    // it, leaving no empty top edge of the table behind.
    assert_eq!(span(&doc, table), (100.0, 172.0));
    assert_eq!(span(&doc, group), (105.0, 167.0));
    assert_eq!(span(&doc, first), (105.0, 155.0));
    assert_eq!(span(&doc, last), (157.0, 167.0));
    assert_eq!(span(&doc, after), (172.0, 182.0));
}

#[test]
fn a_table_moves_with_its_avoiding_first_row_group() {
    let (doc, ids) = table_after_spacer("", None, "break-inside:avoid", &[20.0, 30.0]);
    let [table, group, first, last, after] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, table), (100.0, 156.0));
    assert_eq!(span(&doc, group), (102.0, 154.0));
    assert_eq!(span(&doc, first), (102.0, 122.0));
    assert_eq!(span(&doc, last), (124.0, 154.0));
    assert_eq!(span(&doc, after), (156.0, 166.0));
}

#[test]
fn a_caption_above_the_grid_keeps_the_table_top_on_its_page() {
    let (doc, ids) = table_after_spacer("", Some(""), "", &[30.0]);
    let [table, group, row, _] = ids[..] else {
        unreachable!()
    };
    // The row starts the next page without the table's spacing, and the
    // table keeps its caption on the first page.
    assert_eq!(span(&doc, row), (100.0, 130.0));
    assert_eq!(span(&doc, group), (100.0, 130.0));
    assert_eq!(span(&doc, table), (60.0, 132.0));
}

#[test]
fn a_row_group_with_content_outside_its_rows_keeps_its_top() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        // The block takes an anonymous row and cell owned by the group.
        doc.append_element(Some(group), "div", Some("display:block;height:20px"));
        let row = append_row(doc, group, "", 30.0).0;
        ids.extend([table, group, row]);
    });
    let [table, group, row] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, row), (100.0, 130.0));
    assert_eq!(span(&doc, group), (62.0, 130.0));
    assert_eq!(span(&doc, table), (60.0, 132.0));
}

#[test]
fn a_collapsed_table_grows_over_its_own_moved_row() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-collapse:collapse"),
        );
        // Rows directly in the table, without a row group.
        ids.push(table);
        ids.push(append_row(doc, table, "", 20.0).0);
        ids.push(append_row(doc, table, "", 30.0).0);
    });
    let [table, first, last] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, first), (60.0, 80.0));
    assert_eq!(span(&doc, last), (100.0, 130.0));
    // Collapsed borders keep no spacing below the last row.
    assert_eq!(span(&doc, table), (60.0, 130.0));
}

#[test]
fn a_short_table_moves_with_its_first_row_after_a_forced_break() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:10px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px;border:3px solid"),
        );
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        ids.extend([table, group]);
        ids.push(append_row(doc, group, "break-before:page", 20.0).0);
        ids.push(append_row(doc, group, "", 10.0).0);
    });
    let [table, group, first, last] = ids[..] else {
        unreachable!()
    };
    // The row moves farther than the table is tall; the table's border and
    // spacing still start the new page above it.
    assert_eq!(span(&doc, first), (105.0, 125.0));
    assert_eq!(span(&doc, last), (127.0, 137.0));
    assert_eq!(span(&doc, group), (105.0, 137.0));
    assert_eq!(span(&doc, table), (100.0, 142.0));
}

#[test]
fn an_absolutely_positioned_caption_does_not_hold_the_table_top() {
    let (doc, ids) = table_after_spacer(
        "border:3px solid",
        Some("position:absolute"),
        "",
        &[50.0, 10.0],
    );
    let [table, _, first, ..] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, first), (105.0, 155.0));
    assert_eq!(span(&doc, table), (100.0, 172.0));
}

#[test]
fn a_later_row_group_starts_with_its_row_moved_by_an_earlier_one() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        ids.push(table);
        for rows in [&[20.0, 30.0][..], &[10.0]] {
            let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
            ids.push(group);
            for &height in rows {
                ids.push(append_row(doc, group, "", height).0);
            }
        }
    });
    let [table, first_group, _, moved, second_group, follower] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, moved), (100.0, 130.0));
    // The second group's row follows the moved row, and so does its box.
    assert_eq!(span(&doc, follower), (132.0, 142.0));
    assert_eq!(span(&doc, second_group), (132.0, 142.0));
    assert_eq!(span(&doc, first_group), (62.0, 130.0));
    assert_eq!(span(&doc, table), (60.0, 144.0));
}

#[test]
fn a_table_encloses_a_moved_row_without_cells() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        append_row(doc, group, "", 20.0);
        let empty = doc.append_element(
            Some(group),
            "tr",
            Some("display:table-row;height:20px;break-before:page"),
        );
        ids.extend([table, group, empty]);
    });
    let [table, group, empty] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, empty), (100.0, 120.0));
    assert_eq!(span(&doc, group), (62.0, 120.0));
    assert_eq!(span(&doc, table), (60.0, 122.0));
}

#[test]
fn a_table_with_a_spanning_cell_keeps_its_row_tracks() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        let first = append_row(doc, group, "", 20.0).0;
        let spanning = doc.append_element(
            Some(first),
            "td",
            Some("display:table-cell;padding:0;height:10px"),
        );
        doc.set_element_attribute(spanning, "rowspan", "2").unwrap();
        append_row(doc, group, "", 10.0);
        ids.push(first);
        ids.push(append_row(doc, group, "", 30.0).0);
    });
    let [first, moved] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, moved), (100.0, 130.0));
    // The spanning cell reaches into the second row's track, which does not
    // make the first row any taller.
    assert_eq!(span(&doc, first), (62.0, 82.0));
}

#[test]
fn a_direct_row_with_content_outside_cells_takes_the_table_top_along() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px;border:3px solid"),
        );
        // The block takes an anonymous cell owned by the row.
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        doc.append_element(Some(row), "div", Some("display:block;height:50px"));
        ids.extend([table, row]);
    });
    let [table, row] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, row), (105.0, 155.0));
    assert_eq!(span(&doc, table), (100.0, 160.0));
}

#[test]
fn a_later_row_group_of_a_table_with_a_repeated_header_follows_its_row() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:30px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let head = doc.append_element(Some(table), "thead", Some("display:table-header-group"));
        append_row(doc, head, "", 10.0);
        for rows in [&[20.0, 40.0][..], &[10.0]] {
            let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
            ids.push(group);
            for &height in rows {
                ids.push(append_row(doc, group, "", height).0);
            }
        }
    });
    let [_, _, moved, second_group, follower] = ids[..] else {
        unreachable!()
    };
    // The moved row starts the next page below the header's copy.
    assert_eq!(span(&doc, moved), (112.0, 152.0));
    assert_eq!(span(&doc, follower), (154.0, 164.0));
    assert_eq!(span(&doc, second_group), (154.0, 164.0));
}

#[test]
fn a_footer_written_before_the_body_moves_without_the_body_rows() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let footer = doc.append_element(Some(table), "tfoot", Some("display:table-footer-group"));
        let footer_row = append_row(doc, footer, "", 20.0).0;
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        let body_row = append_row(doc, group, "", 20.0).0;
        let after = doc.append_element(Some(body), "div", Some("display:block;height:10px"));
        ids.extend([table, footer_row, body_row, after]);
    });
    let [table, footer_row, body_row, after] = ids[..] else {
        unreachable!()
    };
    // The table lays the footer out below the body, so only the footer
    // crosses the page edge and moves.
    assert_eq!(span(&doc, body_row), (62.0, 82.0));
    assert_eq!(span(&doc, footer_row), (100.0, 120.0));
    assert_eq!(span(&doc, table), (60.0, 122.0));
    assert_eq!(span(&doc, after), (122.0, 132.0));
}

#[test]
fn a_footer_inside_a_contents_wrapper_moves_without_the_body_rows() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let table = doc.append_element(
            Some(body),
            "table",
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let wrapper = doc.append_element(Some(table), "div", Some("display:contents"));
        let footer = doc.append_element(Some(wrapper), "tfoot", Some("display:table-footer-group"));
        let footer_row = append_row(doc, footer, "", 20.0).0;
        let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
        let body_row = append_row(doc, group, "", 20.0).0;
        ids.extend([footer_row, body_row]);
    });
    let [footer_row, body_row] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, body_row), (62.0, 82.0));
    assert_eq!(span(&doc, footer_row), (100.0, 120.0));
}

/// A 10px row with one cell, outside any table, in `parent`.
fn loose_row(doc: &mut Document, parent: usize) -> usize {
    append_row(doc, parent, "", 10.0).0
}

#[test]
fn rows_outside_a_table_move_whole_to_the_next_page() {
    // Table fixup wraps the rows in an anonymous table, whose rows move to
    // the next page whole, as the rows of a table do: the sixth ends at 95
    // and the seventh would cross the page edge.
    let mut rows = Vec::new();
    let (doc, pages) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:35px"));
        for _ in 0..8 {
            rows.push(loose_row(doc, body));
        }
    });
    assert_eq!(pages, 2);
    let spans: Vec<_> = rows.iter().map(|&row| span(&doc, row)).collect();
    assert_eq!(
        spans,
        [35.0, 45.0, 55.0, 65.0, 75.0, 85.0, 100.0, 110.0].map(|top| (top, top + 10.0))
    );
}

#[test]
fn a_row_group_outside_a_table_grows_over_its_row_moved_to_the_next_page() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let group = doc.append_element(Some(body), "div", Some("display:table-row-group"));
        ids.push(group);
        for height in [20.0, 30.0] {
            ids.push(append_row(doc, group, "", height).0);
        }
        ids.push(doc.append_element(Some(body), "div", Some("display:block;height:10px")));
    });
    let [group, first, last, after] = ids[..] else {
        unreachable!()
    };
    assert_eq!(span(&doc, first), (60.0, 80.0));
    assert_eq!(span(&doc, last), (100.0, 130.0));
    // The group ends with the moved row, and its background is clipped to
    // the cells where they are now.
    assert_eq!(span(&doc, group), (60.0, 130.0));
    assert_eq!(span(&doc, after), (130.0, 140.0));
    let clips = &doc.table_objects.part_background_cells[&group];
    assert!(
        clips
            .iter()
            .any(|cell| (cell.y - 40.0).abs() < 0.001 && (cell.height - 30.0).abs() < 0.001),
        "{clips:?}"
    );
}

#[test]
fn a_row_group_outside_a_table_starts_with_its_first_row_on_the_next_page() {
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:95px"));
        let group = doc.append_element(Some(body), "div", Some("display:table-row-group"));
        ids.push(group);
        for _ in 0..3 {
            ids.push(loose_row(doc, group));
        }
    });
    let [group, first, _, last] = ids[..] else {
        unreachable!()
    };
    // No empty fragment of the group stays behind on the first page.
    assert_eq!(span(&doc, group), (100.0, 130.0));
    assert_eq!(span(&doc, first), (100.0, 110.0));
    assert_eq!(span(&doc, last), (120.0, 130.0));
}

#[test]
fn a_later_row_group_outside_a_table_follows_the_row_moved_by_an_earlier_one() {
    let mut groups = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:75px"));
        for rows in [2, 3] {
            let group = doc.append_element(Some(body), "div", Some("display:table-row-group"));
            for _ in 0..rows {
                loose_row(doc, group);
            }
            groups.push(group);
        }
    });
    // The first group fits on the first page. The first row of the second
    // would cross its edge, so the group starts the next page.
    assert_eq!(span(&doc, groups[0]), (75.0, 95.0));
    assert_eq!(span(&doc, groups[1]), (100.0, 130.0));
}

#[test]
fn the_rows_of_a_table_in_a_cell_do_not_move_the_boxes_of_the_cell() {
    // The rows of the anonymous table in the cell belong to it, not to the
    // table around the cell: the outer table keeps its own row and cell.
    let mut ids = Vec::new();
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:60px"));
        let outer = doc.append_element(Some(body), "div", Some("display:table;border-spacing:0"));
        let outer_row = doc.append_element(Some(outer), "div", Some("display:table-row"));
        let cell = doc.append_element(Some(outer_row), "div", Some("display:table-cell;padding:0"));
        let group = doc.append_element(Some(cell), "div", Some("display:table-row-group"));
        ids.extend([outer, outer_row, cell, group]);
        for _ in 0..3 {
            ids.push(append_row(doc, group, "", 20.0).0);
        }
    });
    let [outer, outer_row, cell, group, first, second, third] = ids[..] else {
        unreachable!()
    };
    // The whole cell moves to the next page, the group and rows within it.
    assert_eq!(span(&doc, first), (100.0, 120.0));
    assert_eq!(span(&doc, second), (120.0, 140.0));
    assert_eq!(span(&doc, third), (140.0, 160.0));
    for part in [outer, outer_row, cell, group] {
        assert_eq!(span(&doc, part), (100.0, 160.0), "{part}");
    }
}

#[test]
fn a_block_encloses_the_collapsed_border_that_hangs_below_the_rows_in_it() {
    // The rows are in a block, or in a table that is written out. Either
    // way the cells' 3px bottom border lies half below the last row, in the
    // box of the table, which the block encloses.
    let spans = |explicit: bool| {
        let mut ids = Vec::new();
        let (doc, _) = paged_body(|doc, body| {
            doc.append_element(Some(body), "div", Some("display:block;height:70px"));
            let display = if explicit { "table" } else { "block" };
            let wrapper = doc.append_element(
                Some(body),
                "div",
                Some(&format!("display:{display};border-collapse:collapse")),
            );
            for _ in 0..6 {
                let row = doc.append_element(Some(wrapper), "div", Some("display:table-row"));
                doc.append_element(
                    Some(row),
                    "div",
                    Some("display:table-cell;padding:0;height:10px;border-bottom:3px solid"),
                );
            }
            ids.push(wrapper);
            ids.push(doc.append_element(Some(body), "div", Some("display:block;height:5px")));
        });
        ids.iter()
            .map(|&id| {
                let (_, y, _, height) = absolute_rect(&doc, id);
                (y, y + height)
            })
            .collect::<Vec<_>>()
    };
    let (loose, explicit) = (spans(false), spans(true));
    assert_eq!(loose, explicit);
    // The six rows end at 70 + 6 * 13 = 148 less what the page edge cut, and
    // the block follows right below the table's box.
    assert_eq!(loose[0].1, loose[1].0);
}

#[test]
fn a_footer_group_belongs_to_the_rows_of_its_own_table() {
    let (mut footer, mut in_footer, mut in_cell, mut loose_footer, mut loose) = (0, 0, 0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let table = doc.append_element(Some(body), "div", Some("display:table"));
        footer = doc.append_element(Some(table), "div", Some("display:table-footer-group"));
        in_footer = loose_row(doc, footer);
        let row = doc.append_element(Some(footer), "div", Some("display:table-row"));
        let cell = doc.append_element(Some(row), "div", Some("display:table-cell"));
        // A row in the footer's cell, outside any table of its own, belongs
        // to the anonymous table around it, which has no footer group.
        in_cell = loose_row(doc, cell);
        loose_footer = doc.append_element(Some(body), "div", Some("display:table-footer-group"));
        loose = loose_row(doc, loose_footer);
    });
    assert_eq!(footer_group_of_row(&doc, in_footer), Some(footer));
    assert_eq!(footer_group_of_row(&doc, in_cell), None);
    assert_eq!(footer_group_of_row(&doc, loose), Some(loose_footer));
}
