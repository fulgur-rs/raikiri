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
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:60px"),
        );
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some(&format!(
                "display:table;width:100px;border-spacing:2px;{table_css}"
            )),
        );
        if let Some(caption_css) = caption {
            doc.append_element(
                Some(table),
                "caption",
                Style::default(),
                Some(&format!("display:table-caption;height:20px;{caption_css}")),
            );
        }
        let group = doc.append_element(
            Some(table),
            "tbody",
            Style::default(),
            Some(&format!("display:table-row-group;{group_css}")),
        );
        ids.push(table);
        ids.push(group);
        for &height in rows {
            ids.push(append_row(doc, group, "", height).0);
        }
        ids.push(doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:10px"),
        ));
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
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:60px"),
        );
        let table = doc.append_element(
            Some(body),
            "table",
            Style::default(),
            Some("display:table;width:100px;border-spacing:2px"),
        );
        let group = doc.append_element(
            Some(table),
            "tbody",
            Style::default(),
            Some("display:table-row-group"),
        );
        // The block takes an anonymous row and cell owned by the group.
        doc.append_element(
            Some(group),
            "div",
            Style::default(),
            Some("display:block;height:20px"),
        );
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
