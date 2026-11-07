//! The header of a table whose bottom caption starts the next page. Positions
//! come from Chromium 152 (printToPDF, page 200x200, Ahem 8px with a 10px
//! line height); the second page starts at 200 in flow coordinates.

use super::*;

/// One part of the table, in source order.
enum Part {
    /// A row of the header group.
    Head(&'static str),
    /// A row of the body group.
    Row(&'static str),
    /// A row of the footer group.
    Foot(&'static str),
    /// A caption above the grid, with its extra style.
    Top(&'static str),
    /// A caption below the grid, with its extra style.
    Bottom(&'static str),
}

struct Printed {
    /// The flow position of each part in source order.
    boxes: Vec<f32>,
    /// The block after the holder of the table.
    after: f32,
    pages: usize,
    /// Where the header copy of the second page sits, when there is one.
    copy: Option<(f32, f32)>,
}

/// A table of `parts` that starts `before` px down the first page, inside
/// `depth` plain blocks of a holder with `holder_css`, with a block after it.
fn printed(before: u32, table_css: &str, parts: &[Part], holder_css: &str, depth: u32) -> Printed {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px;font-family:Ahem;font-size:8px;line-height:10px"),
    );
    let mut holder = doc.append_element(
        Some(body),
        "div",
        Some(format!("display:block;{holder_css}")),
    );
    for _ in 0..depth {
        holder = doc.append_element(Some(holder), "div", Some("display:block"));
    }
    let lead = doc.append_element(
        Some(holder),
        "div",
        Some(format!("display:block;height:{before}px")),
    );
    doc.append_text(lead, "A");
    let table = doc.append_element(
        Some(holder),
        "table",
        Some(format!(
            "display:table;border-spacing:0;border-collapse:collapse;{table_css}"
        )),
    );
    let mut groups: [Option<usize>; 3] = [None; 3];
    let mut boxes = Vec::new();
    for part in parts {
        let (group, display, text) = match *part {
            Part::Head(text) => (0, "table-header-group", text),
            Part::Row(text) => (1, "table-row-group", text),
            Part::Foot(text) => (2, "table-footer-group", text),
            Part::Top(css) | Part::Bottom(css) => {
                let side = if matches!(part, Part::Top(_)) {
                    "top"
                } else {
                    "bottom"
                };
                let caption = doc.append_element(
                    Some(table),
                    "caption",
                    Some(format!(
                        "display:table-caption;caption-side:{side};height:10px;{css}"
                    )),
                );
                doc.append_text(caption, "k");
                boxes.push(caption);
                continue;
            }
        };
        let owner = *groups[group].get_or_insert_with(|| {
            let tag = ["thead", "tbody", "tfoot"][group];
            doc.append_element(Some(table), tag, Some(format!("display:{display}")))
        });
        let row = doc.append_element(Some(owner), "tr", Some("display:table-row"));
        let cell = doc.append_element(
            Some(row),
            "td",
            Some("display:table-cell;padding:0;height:10px"),
        );
        doc.append_text(cell, text);
        boxes.push(row);
    }
    let after = doc.append_element(Some(body), "div", Some("display:block"));
    doc.append_text(after, "Z");
    doc.mark_in_document_flags();
    let cascade = crate::layout::test_support::cascaded(&doc);
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    let pages = layout_pages(with_ahem(&mut doc), &cascade, page)
        .unwrap()
        .len();
    let copy = doc
        .table_objects
        .headers
        .headers
        .values()
        .find(|header| !header.footer)
        .and_then(|header| header.placements.get(&1).copied());
    Printed {
        boxes: boxes.iter().map(|&id| page_flow_y(&doc, id)).collect(),
        after: page_flow_y(&doc, after),
        pages,
        copy,
    }
}

const HEAD_ROWS_CAPTION: [Part; 4] = [
    Part::Head("h"),
    Part::Row("c"),
    Part::Row("d"),
    Part::Bottom(""),
];

#[test]
fn a_caption_pushed_alone_by_the_closing_padding_gets_a_header_copy_above_it() {
    // The caption ends exactly at the page end, the 30px padding of the holder
    // does not fit below it, so the caption moves alone to the next page and
    // goes below a copy of the header.
    let out = printed(160, "", &HEAD_ROWS_CAPTION, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [160.0, 170.0, 180.0, 210.0]);
    assert_eq!(out.after, 250.0);
    assert_eq!(out.pages, 2);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn a_caption_that_fits_with_room_to_spare_before_the_padding_gets_the_same_copy() {
    let out = printed(150, "", &HEAD_ROWS_CAPTION, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [150.0, 160.0, 170.0, 210.0]);
    assert_eq!(out.after, 250.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn the_copy_makes_room_for_every_row_of_the_header() {
    let parts = [
        Part::Head("h"),
        Part::Head("i"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Bottom(""),
    ];
    let out = printed(150, "", &parts, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [150.0, 160.0, 170.0, 180.0, 220.0]);
    assert_eq!(out.after, 260.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn the_copy_keeps_the_height_of_the_caption_and_the_padding_that_closes_the_holder() {
    let tall = [
        Part::Head("h"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Bottom("height:20px"),
    ];
    let out = printed(140, "", &tall, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [140.0, 150.0, 160.0, 210.0]);
    assert_eq!(out.after, 260.0);
    for (padding, caption, after) in [(5, 210.0, 225.0), (10, 210.0, 230.0)] {
        let out = printed(
            160,
            "",
            &HEAD_ROWS_CAPTION,
            &format!("padding-bottom:{padding}px"),
            0,
        );
        assert_eq!(out.boxes[3], caption, "padding={padding}");
        assert_eq!(out.after, after, "padding={padding}");
        assert_eq!(out.copy, Some((200.0, 200.0)), "padding={padding}");
    }
}

#[test]
fn a_caption_that_ends_the_page_with_the_holder_stays() {
    let out = printed(160, "", &HEAD_ROWS_CAPTION, "", 0);
    assert_eq!(out.boxes, [160.0, 170.0, 180.0, 190.0]);
    assert_eq!(out.after, 200.0);
    assert_eq!(out.pages, 2);
    assert_eq!(out.copy, None);
}

#[test]
fn a_caption_that_overflows_the_page_by_itself_has_no_header_copy() {
    // The rows end at the page end, so the caption opens the second page
    // whatever follows it.
    let out = printed(170, "", &HEAD_ROWS_CAPTION, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [170.0, 180.0, 190.0, 200.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.copy, None);
}

#[test]
fn a_second_bottom_caption_moving_alone_has_no_header_copy() {
    let parts = [
        Part::Head("h"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Bottom(""),
        Part::Bottom(""),
    ];
    let out = printed(150, "", &parts, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [150.0, 160.0, 170.0, 180.0, 200.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.copy, None);
}

#[test]
fn a_table_with_a_footer_or_without_a_header_gets_no_copy_for_its_caption() {
    let footer = [
        Part::Head("h"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Foot("f"),
        Part::Bottom(""),
    ];
    let out = printed(160, "", &footer, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [160.0, 170.0, 180.0, 190.0, 200.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.copy, None);
    let headless = [Part::Row("c"), Part::Row("d"), Part::Bottom("")];
    let out = printed(160, "", &headless, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [160.0, 170.0, 200.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.copy, None);
}

#[test]
fn a_top_caption_stays_with_the_table_and_the_copy_comes_without_it() {
    let parts = [
        Part::Top(""),
        Part::Head("h"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Bottom(""),
    ];
    let out = printed(150, "", &parts, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [150.0, 160.0, 170.0, 180.0, 210.0]);
    assert_eq!(out.after, 250.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn the_copy_follows_into_blocks_nested_in_the_holder() {
    let out = printed(150, "", &HEAD_ROWS_CAPTION, "padding-bottom:30px", 2);
    assert_eq!(out.boxes, [150.0, 160.0, 170.0, 210.0]);
    assert_eq!(out.after, 250.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn spacing_and_padding_of_the_table_stay_between_the_copy_and_the_caption() {
    // Chromium puts the caption below the copy, the spacing under it, and the
    // padding of the table that closes the grid.
    let spaced = "border-collapse:separate;border-spacing:2px";
    let out = printed(140, spaced, &HEAD_ROWS_CAPTION, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [142.0, 154.0, 166.0, 212.0]);
    assert_eq!(out.after, 252.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
    let padded = "border-collapse:separate;border-spacing:0;padding:3px";
    let out = printed(140, padded, &HEAD_ROWS_CAPTION, "padding-bottom:30px", 0);
    assert_eq!(out.boxes, [143.0, 153.0, 163.0, 213.0]);
    assert_eq!(out.after, 253.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn rows_that_cross_the_page_repeat_the_header_before_a_bottom_caption() {
    let parts = [
        Part::Head("h"),
        Part::Row("c"),
        Part::Row("d"),
        Part::Row("e"),
        Part::Row("f"),
        Part::Bottom(""),
    ];
    let out = printed(170, "", &parts, "", 0);
    assert_eq!(out.boxes, [170.0, 180.0, 190.0, 210.0, 220.0, 230.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.pages, 2);
    assert_eq!(out.copy, Some((200.0, 200.0)));
}

#[test]
fn a_table_with_a_footer_repeats_it_around_the_rows_before_a_bottom_caption() {
    // Chromium closes page one with a copy of the footer below the second row
    // (at 190), and opens page two with a copy of the header when there is one.
    let rows = |head: bool| {
        let mut parts = Vec::new();
        if head {
            parts.push(Part::Head("h"));
        }
        parts.extend([
            Part::Row("c"),
            Part::Row("d"),
            Part::Row("e"),
            Part::Row("f"),
            Part::Foot("f"),
            Part::Bottom(""),
        ]);
        parts
    };
    let out = printed(170, "", &rows(true), "", 0);
    assert_eq!(out.boxes, [170.0, 180.0, 210.0, 220.0, 230.0, 240.0, 250.0]);
    assert_eq!(out.after, 260.0);
    assert_eq!(out.copy, Some((200.0, 200.0)));
    let out = printed(170, "", &rows(false), "", 0);
    assert_eq!(out.boxes, [170.0, 180.0, 200.0, 210.0, 220.0, 230.0]);
    assert_eq!(out.after, 240.0);
    assert_eq!(out.copy, None);
}
