//! A table caption at the end of a page: the page breaks before the caption,
//! with the table, when no break between its lines leaves enough of them on
//! either side. Positions come from Chromium 152 (printToPDF, page 200x200,
//! Ahem 8px with a 10px line height).

use super::*;

/// One part of the table, in source order.
enum Part {
    /// A caption above the grid, with its extra style and text.
    Top(&'static str, &'static str),
    /// A caption below the grid.
    Bottom(&'static str, &'static str),
    /// A body row of one 10px cell.
    Row(&'static str),
}

/// Lays `doc` out on pages of 200x200px with no margin, and returns how many
/// pages it takes.
fn paginate(doc: &mut Document) -> usize {
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(doc);
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    layout_pages(with_ahem(doc), &cascade, page).unwrap().len()
}

/// A table of `parts` that starts `before` px down the first page, with a
/// block after it: the flow positions of the table's boxes in source order,
/// the position of the block after it, and the number of pages.
fn laid_out(
    before: u32,
    table_css: &str,
    parts: &[Part],
    holder_css: Option<&str>,
) -> (Vec<f32>, f32, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    if before > 0 {
        let lead = doc.append_element(
            Some(body),
            "div",
            Some(format!("display:block;height:{before}px")),
        );
        doc.append_text(lead, "A");
    }
    let holder = holder_css.map_or(body, |css| {
        doc.append_element(Some(body), "div", Some(format!("display:block;{css}")))
    });
    let table = doc.append_element(
        Some(holder),
        "table",
        Some(format!(
            "display:table;border-spacing:0;border-collapse:collapse;{table_css}"
        )),
    );
    let body_group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
    let mut boxes = Vec::new();
    for part in parts {
        match *part {
            Part::Top(css, text) | Part::Bottom(css, text) => {
                let side = if matches!(part, Part::Top(..)) {
                    "top"
                } else {
                    "bottom"
                };
                let caption = doc.append_element(
                    Some(table),
                    "caption",
                    Some(format!("display:table-caption;caption-side:{side};{css}")),
                );
                doc.append_text(caption, text);
                boxes.push(caption);
            }
            Part::Row(text) => {
                let row = doc.append_element(Some(body_group), "tr", Some("display:table-row"));
                let cell = doc.append_element(
                    Some(row),
                    "td",
                    Some("display:table-cell;padding:0;height:10px"),
                );
                doc.append_text(cell, text);
                boxes.push(row);
            }
        }
    }
    let after = doc.append_element(Some(body), "div", Some("display:block"));
    doc.append_text(after, "Z");
    let pages = paginate(&mut doc);
    let positions = boxes.iter().map(|&id| page_flow_y(&doc, id)).collect();
    (positions, page_flow_y(&doc, after), pages)
}

#[test]
fn a_top_caption_that_does_not_fit_moves_to_the_next_page_with_the_table() {
    // The caption would start 5px above the page end, a line does not fit
    // there, so it takes the first row along to the top of the second page.
    let (positions, after, pages) = laid_out(195, "", &[Part::Top("", "t1"), Part::Row("c")], None);
    assert_eq!(positions, [200.0, 210.0]);
    assert_eq!(after, 220.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_bottom_caption_that_does_not_fit_moves_to_the_next_page_alone() {
    // The row ends exactly 5px above the page end and stays; the caption
    // below it moves to the next page, and the block after it follows.
    let (positions, after, pages) =
        laid_out(185, "", &[Part::Row("c"), Part::Bottom("", "t1")], None);
    assert_eq!(positions, [185.0, 200.0]);
    assert_eq!(after, 210.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_second_top_caption_follows_the_first_to_the_next_page() {
    let (positions, after, pages) = laid_out(
        195,
        "",
        &[Part::Top("", "t1"), Part::Top("", "t2"), Part::Row("c")],
        None,
    );
    assert_eq!(positions, [200.0, 210.0, 220.0]);
    assert_eq!(after, 230.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_top_caption_that_fits_the_page_stays_and_the_row_moves() {
    let (positions, after, pages) = laid_out(190, "", &[Part::Top("", "t1"), Part::Row("c")], None);
    assert_eq!(positions, [190.0, 200.0]);
    assert_eq!(after, 210.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_caption_with_a_height_of_its_own_moves_whole() {
    let (positions, after, pages) = laid_out(
        195,
        "",
        &[Part::Top("height:30px", "t1"), Part::Row("c")],
        None,
    );
    assert_eq!(positions, [200.0, 230.0]);
    assert_eq!(after, 240.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_caption_of_too_few_lines_to_break_between_moves_whole() {
    // Two lines starting 15px above the page end: a break between them
    // would leave one line on either page, short of the two orphans asked
    // for, so the caption moves. So do three lines, which are fewer than the
    // orphans and widows together.
    for (before, text, rows_at, after_at) in [
        (185, "t1 t1", 220.0, 230.0),
        (195, "t1 t1 t1", 230.0, 240.0),
    ] {
        let (positions, after, pages) = laid_out(
            before,
            "",
            &[Part::Top("width:16px", text), Part::Row("c")],
            None,
        );
        assert_eq!(positions, [200.0, rows_at], "{text}");
        assert_eq!(after, after_at, "{text}");
        assert_eq!(pages, 2, "{text}");
    }
}

#[test]
fn a_caption_whose_first_lines_do_not_fit_for_the_orphans_moves_whole() {
    // Four lines starting 15px above the page end: only one fits, and the
    // break after it would leave a single orphan.
    let (positions, after, pages) = laid_out(
        185,
        "",
        &[Part::Top("width:16px", "t1 t1 t1 t1"), Part::Row("c")],
        None,
    );
    assert_eq!(positions, [200.0, 240.0]);
    assert_eq!(after, 250.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_caption_that_splits_between_its_lines_stays_where_it_is() {
    // Four lines starting 25px above the page end: two fit, and a break after
    // them leaves two orphans and two widows, so the caption is not moved.
    let (positions, _, pages) = laid_out(
        175,
        "",
        &[Part::Top("width:16px", "t1 t1 t1 t1"), Part::Row("c")],
        None,
    );
    assert_eq!(positions[0], 175.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_caption_at_the_top_of_a_page_is_not_moved() {
    // Nothing comes before the table, so moving the caption would only leave a
    // blank page. It runs on to the second page, 100px of it.
    let (positions, after, pages) = laid_out(
        0,
        "",
        &[Part::Top("height:300px", "t1"), Part::Row("c")],
        None,
    );
    assert_eq!(positions, [0.0, 300.0]);
    assert_eq!(after, 310.0);
    assert_eq!(pages, 2);
}

#[test]
fn a_moved_caption_takes_the_top_edge_of_the_table_along() {
    // The table box starts under the caption on the second page, where it
    // would have started without the page break.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    let lead = doc.append_element(Some(body), "div", Some("display:block;height:195px"));
    doc.append_text(lead, "A");
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;border-collapse:collapse;border:2px solid"),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some("display:table-caption;caption-side:top"),
    );
    doc.append_text(caption, "t1");
    let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
    let row = doc.append_element(Some(group), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
        Some("display:table-cell;padding:0;height:10px"),
    );
    doc.append_text(cell, "c");
    paginate(&mut doc);
    assert_eq!(page_flow_y(&doc, caption), 200.0);
    let grid = doc.nodes[table].table_grid_box.expect("a grid box");
    let (_, table_y, _, _) = crate::layout::test_support::absolute_rect(&doc, table);
    assert_eq!(table_y + grid.y, 210.0);
    assert_eq!(grid.height, 14.0);
}

#[test]
fn a_caption_of_a_table_in_a_cell_follows_its_row() {
    // The nested table's caption is not a break of its own: the outer row,
    // 60px high from 150px, moves whole with everything in it.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    let lead = doc.append_element(Some(body), "div", Some("display:block;height:195px"));
    doc.append_text(lead, "A");
    let outer = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;border-collapse:collapse"),
    );
    let group = doc.append_element(Some(outer), "tbody", Some("display:table-row-group"));
    let row = doc.append_element(Some(group), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
        Some("display:table-cell;padding:0;vertical-align:top"),
    );
    let inner = doc.append_element(
        Some(cell),
        "table",
        Some("display:table;border-spacing:0;border-collapse:collapse"),
    );
    let caption = doc.append_element(
        Some(inner),
        "caption",
        Some("display:table-caption;caption-side:top"),
    );
    doc.append_text(caption, "t1");
    let inner_group = doc.append_element(Some(inner), "tbody", Some("display:table-row-group"));
    let inner_row = doc.append_element(Some(inner_group), "tr", Some("display:table-row"));
    let inner_cell = doc.append_element(
        Some(inner_row),
        "td",
        Some("display:table-cell;padding:0;height:10px"),
    );
    doc.append_text(inner_cell, "d");
    paginate(&mut doc);
    assert_eq!(page_flow_y(&doc, row), 200.0);
    assert_eq!(page_flow_y(&doc, caption), 200.0);
}

/// The grid box of a bordered table with one row and a bottom caption, which
/// starts `before` px down the first page, and the caption's flow position.
fn grid_box_with_a_bottom_caption(before: u32) -> (raikiri_traits::PaintRect, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    let lead = doc.append_element(
        Some(body),
        "div",
        Some(format!("display:block;height:{before}px")),
    );
    doc.append_text(lead, "A");
    let table = doc.append_element(
        Some(body),
        "table",
        Some("display:table;border-spacing:0;border-collapse:collapse;border:2px solid"),
    );
    let group = doc.append_element(Some(table), "tbody", Some("display:table-row-group"));
    let row = doc.append_element(Some(group), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
        Some("display:table-cell;padding:0;height:10px"),
    );
    doc.append_text(cell, "c");
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some("display:table-caption;caption-side:bottom"),
    );
    doc.append_text(caption, "t1");
    paginate(&mut doc);
    let grid = doc.nodes[table].table_grid_box.expect("a grid box");
    (grid, page_flow_y(&doc, caption))
}

#[test]
fn a_bottom_caption_that_moves_alone_leaves_the_grid_box_as_it_was() {
    // The bordered grid ends just above the page end and stays; the caption
    // under it moves. The grid box does not grow over the caption: it is as
    // high as when the whole table fits.
    let (whole, caption) = grid_box_with_a_bottom_caption(100);
    assert_eq!(caption, 112.0 + 2.0);
    let (grid, caption) = grid_box_with_a_bottom_caption(185);
    assert_eq!(caption, 200.0);
    assert_eq!(grid.height, whole.height);
}
