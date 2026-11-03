//! The lines of a paragraph that run after a block among them, or that stand
//! alone in a box, when the padding and border that close that box (or the
//! body, or a box around it) do not fit under the last of them. Where the
//! page breaks is what Chromium prints for the same documents: the page gives
//! up the `widows` lines when it keeps the `orphans`, and otherwise the lines
//! go to the next page whole when a break before them is there to take, with
//! the box above them in the same container, and only the last one when there
//! is none.
//!
//! Each row is a document and the number of lines of text on each page, as
//! Chromium prints it on pages 100px wide with Ahem lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em and a 20px glyph sits 5px below the top of its 30px
/// line, so the baseline is 21px below the top of the line.
const BASELINE_BELOW_LINE_TOP: f32 = 21.0;

/// The number of lines of text on each page of `body` laid out on pages
/// `page_height` tall, up to the last page that has any.
fn lines_per_page(body: &str, page_height: u32) -> Vec<usize> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px {page_height}px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let mut counts = Vec::new();
    for index in 0..12 {
        let Some(page) = result.page(index) else {
            continue;
        };
        // The words of a line are runs of their own in some paragraphs.
        let mut tops: Vec<f32> = page
            .text_runs()
            .into_iter()
            .map(|run| run.origin.1 - BASELINE_BELOW_LINE_TOP)
            .collect();
        tops.sort_by(f32::total_cmp);
        tops.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        counts.resize(index as usize + 1, 0);
        counts[index as usize] = tops.len();
    }
    // A page that holds only the empty tail of a box has no text to count.
    while counts.last() == Some(&0) {
        counts.pop();
    }
    counts
}

fn check(rows: &[(&str, &[usize])], page_height: u32) {
    for &(body, expected) in rows {
        assert_eq!(
            lines_per_page(body, page_height),
            expected,
            "{body} on pages {page_height}px tall"
        );
    }
}

/// Pages 100px tall hold three lines.
#[test]
fn the_edges_of_a_box_stay_with_the_last_lines_after_a_block_in_it() {
    const ROWS: &[(&str, &[usize])] = &[
        (
            r#"<div style="width:20px;padding-bottom:6px"><div style="height:5px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:10px"><div style="height:5px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:5px"></div>X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:25px"><div style="height:5px"></div>X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:6px"><div style="height:10px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:10px"><div style="height:10px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:10px"></div>X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:25px"><div style="height:10px"></div>X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:6px"><div style="height:20px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:10px"><div style="height:20px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:20px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:25px"><div style="height:20px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:6px"><div style="height:30px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:10px"><div style="height:30px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:30px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:25px"><div style="height:30px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:5px"></div><div style="width:20px;padding-bottom:15px">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:5px"></div><div style="width:20px;padding-bottom:15px">X X X X</div>"#,
            &[2, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:5px"></div>X X X X</div>"#,
            &[2, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:5px"></div>X X</div>"#,
            &[2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:25px"></div>X X</div>"#,
            &[2],
        ),
        (
            r#"<div style="height:25px"></div><div style="width:20px;padding-bottom:15px">X X</div>"#,
            &[2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:5px"></div>X X X X X</div>"#,
            &[3, 2],
        ),
        (
            r#"<div style="height:5px"></div><div style="width:20px;padding-bottom:15px">X X X X X</div>"#,
            &[3, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px"><div style="height:5px"></div>X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px">X<div style="height:5px"></div>X X</div>"#,
            &[1, 2],
        ),
    ];
    check(ROWS, 100);
}

/// Pages 280px tall hold nine lines. A block of `270 - 30 n` px and `n` lines
/// of text in a box that has 15px of padding below, `orphans` and `widows` as
/// named: the edges do not fit under the last line.
#[test]
fn the_page_gives_up_the_widows_or_the_whole_run_by_what_it_can_keep() {
    const AFTER_A_BLOCK_IN_THE_BOX: &[(&str, &[usize])] = &[
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:1"><div style="height:180px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[1, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:4"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:1"><div style="height:180px"></div>X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:3"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:4"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:1"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:3"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:4"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:1"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:3"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:4"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:1"><div style="height:150px"></div>X X X X</div>"#,
            &[3, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:2"><div style="height:150px"></div>X X X X</div>"#,
            &[2, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:4"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:1"><div style="height:150px"></div>X X X X</div>"#,
            &[3, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:150px"></div>X X X X</div>"#,
            &[2, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:4"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:1"><div style="height:150px"></div>X X X X</div>"#,
            &[3, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:2"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:4"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:1"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:2"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:4"><div style="height:150px"></div>X X X X</div>"#,
            &[0, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:1"><div style="height:120px"></div>X X X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:2"><div style="height:120px"></div>X X X X X</div>"#,
            &[3, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:120px"></div>X X X X X</div>"#,
            &[2, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:4"><div style="height:120px"></div>X X X X X</div>"#,
            &[1, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:1"><div style="height:120px"></div>X X X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:120px"></div>X X X X X</div>"#,
            &[3, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:3"><div style="height:120px"></div>X X X X X</div>"#,
            &[2, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:4"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:1"><div style="height:120px"></div>X X X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:2"><div style="height:120px"></div>X X X X X</div>"#,
            &[3, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:3"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:4"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:1"><div style="height:120px"></div>X X X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:2"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:3"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:4"><div style="height:120px"></div>X X X X X</div>"#,
            &[0, 5],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:1"><div style="height:90px"></div>X X X X X X</div>"#,
            &[5, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:2"><div style="height:90px"></div>X X X X X X</div>"#,
            &[4, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:90px"></div>X X X X X X</div>"#,
            &[3, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:4"><div style="height:90px"></div>X X X X X X</div>"#,
            &[2, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:1"><div style="height:90px"></div>X X X X X X</div>"#,
            &[5, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:90px"></div>X X X X X X</div>"#,
            &[4, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:3"><div style="height:90px"></div>X X X X X X</div>"#,
            &[3, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:4"><div style="height:90px"></div>X X X X X X</div>"#,
            &[2, 4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:1"><div style="height:90px"></div>X X X X X X</div>"#,
            &[5, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:2"><div style="height:90px"></div>X X X X X X</div>"#,
            &[4, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:3"><div style="height:90px"></div>X X X X X X</div>"#,
            &[3, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:3;widows:4"><div style="height:90px"></div>X X X X X X</div>"#,
            &[0, 6],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:1"><div style="height:90px"></div>X X X X X X</div>"#,
            &[5, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:2"><div style="height:90px"></div>X X X X X X</div>"#,
            &[4, 2],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:3"><div style="height:90px"></div>X X X X X X</div>"#,
            &[0, 6],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:4;widows:4"><div style="height:90px"></div>X X X X X X</div>"#,
            &[0, 6],
        ),
    ];
    check(AFTER_A_BLOCK_IN_THE_BOX, 280);
}

/// The same lines in a box of their own after the block: no break before them
/// is there to take inside the box that the edges close, so only the last
/// line goes to the next page when the page cannot give up the widows.
#[test]
fn a_paragraph_after_a_block_outside_its_box_gives_up_only_its_last_line() {
    const ROWS: &[(&str, &[usize])] = &[
        (
            r#"<div style="height:180px"></div><div style="width:20px;padding-bottom:15px;orphans:2;widows:2">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:180px"></div><div style="width:20px;padding-bottom:15px;orphans:1;widows:3">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:180px"></div><div style="width:20px;padding-bottom:15px;orphans:3;widows:1">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:180px"></div><div style="width:20px;padding-bottom:15px;orphans:2;widows:4">X X X</div>"#,
            &[2, 1],
        ),
        (
            r#"<div style="height:150px"></div><div style="width:20px;padding-bottom:15px;orphans:2;widows:2">X X X X</div>"#,
            &[2, 2],
        ),
        (
            r#"<div style="height:150px"></div><div style="width:20px;padding-bottom:15px;orphans:1;widows:4">X X X X</div>"#,
            &[3, 1],
        ),
        (
            r#"<div style="height:120px"></div><div style="width:20px;padding-bottom:15px;orphans:3;widows:3">X X X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="height:90px"></div><div style="width:20px;padding-bottom:15px;orphans:2;widows:4">X X X X X X</div>"#,
            &[2, 4],
        ),
    ];
    check(ROWS, 280);
}

/// Edges of both kinds, lines before the block, a block after the lines, a
/// height taller than the lines, a box around the box that has the padding,
/// and a block that avoids a break inside it.
#[test]
fn the_lines_after_a_block_follow_the_edges_only_where_they_follow_them() {
    const ROWS: &[(&str, &[usize])] = &[
        (
            r#"<div style="width:20px;border-bottom:15px solid;orphans:2;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;border-bottom:15px solid;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:7px;border-bottom:8px solid;orphans:2;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:7px;border-bottom:8px solid;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2">X<div style="height:150px"></div>X X X</div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3">X<div style="height:120px"></div>X X X X</div>"#,
            &[2, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:180px"></div>X X X<div style="height:5px"></div></div>"#,
            &[3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:150px"></div>X X X X<div style="height:5px"></div></div>"#,
            &[4],
        ),
        // A block after the lines holds the edges below it even when it has
        // no height.
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:180px"></div>X X X<div style="height:0"></div></div>"#,
            &[3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:150px"></div>X X X X<div style="height:0"></div></div>"#,
            &[4],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:180px"></div>X X X<div style="height:0"></div><div style="height:0"></div></div>"#,
            &[3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:150px"></div>X X<div style="height:0"></div>X X X</div>"#,
            &[4, 1],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:120px"></div>X X<div style="height:5px"></div>X X X</div>"#,
            &[2, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:90px"></div>X X<div style="height:5px"></div>X X X X</div>"#,
            &[3, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;height:290px;orphans:2;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;height:290px;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[4],
        ),
        (
            r#"<div style="padding-bottom:15px"><div style="width:20px;orphans:2;widows:2"><div style="height:180px"></div>X X X</div></div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="padding-bottom:15px"><div style="width:20px;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div></div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding:5px 0 15px;orphans:2;widows:2"><div style="height:175px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding:5px 0 15px;orphans:1;widows:3"><div style="height:145px"></div>X X X X</div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:2;widows:2"><div style="height:180px"></div>X X X</div><div style="height:10px"></div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div><div style="height:10px"></div>"#,
            &[1, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;break-inside:avoid;orphans:2;widows:2"><div style="height:180px"></div>X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<div style="width:20px;padding-bottom:15px;break-inside:avoid;orphans:1;widows:3"><div style="height:150px"></div>X X X X</div>"#,
            &[1, 3],
        ),
    ];
    check(ROWS, 280);
}

/// A line or a paragraph after a block with the padding of the body or of a
/// box around it below.
#[test]
fn the_edges_of_the_body_or_a_wrapper_hold_the_paragraph_above_them_on_the_next_page() {
    const BODY_ROWS: &[(&str, &[usize])] = &[
        (
            r#"<style>body{padding-bottom:6px}</style><div style="height:5px"></div>X"#,
            &[1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:5px"></div><div style="width:20px">X X</div>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:15px}</style><div style="height:5px"></div><div style="width:20px">X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:5px"></div><div style="width:20px">X X X</div>"#,
            &[0, 2, 1],
        ),
        (r#"<style>body{padding-bottom:15px}</style>X X X X X"#, &[2]),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:10px"></div><div style="width:20px">X X</div>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:15px}</style><div style="height:10px"></div><div style="width:20px">X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:10px"></div><div style="width:20px">X X X</div>"#,
            &[0, 2, 1],
        ),
        (r#"<style>body{padding-bottom:40px}</style>X X X X"#, &[2]),
        (
            r#"<style>body{padding-bottom:25px}</style><div style="height:20px"></div><div style="width:20px">X X</div>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:6px}</style><div style="height:20px"></div>X X X X"#,
            &[2],
        ),
        (
            r#"<style>body{padding-bottom:25px}</style><div style="height:20px"></div>X X X X"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:15px}</style><div style="height:30px"></div><div style="width:20px">X X</div>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:30px"></div><div style="width:20px">X X</div>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:15px}</style><div style="height:30px"></div><p style="margin:0;width:20px">X X X</p>"#,
            &[2, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:30px"></div>X X X X"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:40px"></div>X"#,
            &[0, 1],
        ),
        (
            r#"<style>body{padding-bottom:6px}</style><div style="height:40px"></div><p style="margin:0;width:20px">X X</p>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:25px}</style><div style="height:40px"></div><p style="margin:0;width:20px">X X</p>"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:15px}</style><div style="height:40px"></div>X X X"#,
            &[1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:40px"></div>X X X"#,
            &[0, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:40px"></div>X X X X"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:40px"></div>X X X X X"#,
            &[0, 2],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:50px"></div>X"#,
            &[0, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:50px"></div>X X"#,
            &[0, 1],
        ),
        (
            r#"<style>body{padding-bottom:6px}</style><div style="height:50px"></div><div style="width:20px">X X X</div>"#,
            &[0, 3],
        ),
        (
            r#"<style>body{padding-bottom:25px}</style><div style="height:50px"></div><div style="width:20px">X X X</div>"#,
            &[0, 2, 1],
        ),
        (
            r#"<style>body{padding-bottom:40px}</style><div style="height:50px"></div><p style="margin:0;width:20px">X X X</p>"#,
            &[0, 2, 1],
        ),
    ];
    check(BODY_ROWS, 100);
    const WRAPPER_ROWS: &[(&str, &[usize])] = &[
        (
            r#"<div style="padding-bottom:40px"><div style="height:40px"></div>X</div>"#,
            &[0, 1],
        ),
        (
            r#"<div style="padding-bottom:25px"><div style="height:50px"></div>X</div>"#,
            &[0, 1],
        ),
        (
            r#"<div style="padding-bottom:40px"><div style="height:50px"></div>X</div>"#,
            &[0, 1],
        ),
        (
            r#"<div style="padding-bottom:15px"><div style="height:30px"></div>X</div>"#,
            &[1],
        ),
        (
            r#"<div style="padding-bottom:25px"><div style="height:20px"></div>X</div>"#,
            &[1],
        ),
        (
            r#"<div style="padding-bottom:40px"><div style="height:40px"></div><div style="width:20px">X</div></div>"#,
            &[0, 1],
        ),
        (
            r#"<div style="padding-bottom:15px"><div style="height:40px"></div><p style="margin:0;width:20px">X X</p></div>"#,
            &[0, 2],
        ),
    ];
    check(WRAPPER_ROWS, 100);
}
