//! Lines of text that follow a block inside the same parent, and the lines of a
//! paragraph that follows one, around the end of a page. The positions are the
//! ones Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A forced break after a 30px block.
const BREAK: &str = "<div style='break-after:page;height:30px'>a</div>";

fn spacer(height: u32) -> String {
    format!("<div style='height:{height}px'></div>")
}

/// The page, left edge and top of every line of text but the lead-in `a`,
/// read from the first run on the line.
fn lines(body: &str) -> Vec<(u32, f32, f32)> {
    lines_of(body, |text| text != "a")
}

/// The same for the lines whose first run `wanted` accepts.
fn lines_of(body: &str, wanted: impl Fn(&str) -> bool) -> Vec<(u32, f32, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
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
    let mut lines: Vec<(u32, f32, f32)> = Vec::new();
    for index in 0..8 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            let line = (index, run.origin.0, run.origin.1 - ASCENT);
            if wanted(run.text.trim_end()) && !lines.iter().any(|l| l.0 == line.0 && l.2 == line.2)
            {
                lines.push(line);
            }
        }
    }
    lines
}

#[test]
fn a_line_that_crosses_the_page_end_after_a_block_starts_the_next_page() {
    // The line would run from 65px to 95px, within the page.
    assert_eq!(lines(&format!("{}z", spacer(65))), [(0, 0.0, 70.0)]);
    // From 80px or 95px it would cross the end of the page, and from 100px
    // it starts the next one anyway.
    for height in [80, 95, 100] {
        assert_eq!(
            lines(&format!("{}z", spacer(height))),
            [(1, 0.0, 5.0)],
            "{height}px block"
        );
    }
}

#[test]
fn the_line_after_a_block_on_a_later_page_starts_the_page_after_it() {
    assert_eq!(lines(&format!("{BREAK}{}z", spacer(90))), [(2, 0.0, 5.0)]);
}

#[test]
fn the_line_after_a_block_that_runs_onto_the_next_page_follows_it() {
    // The 80px block runs 10px onto the second page, where the line goes on
    // below it.
    assert_eq!(lines(&format!("a{}z", spacer(80))), [(1, 0.0, 15.0)]);
}

#[test]
fn lines_after_a_block_that_keep_their_orphans_move_together() {
    // Three lines of three words: one would stay on the first page, which is
    // fewer than the two lines orphans asks for, so all three move.
    assert_eq!(
        lines(&format!("{}z z z z z z z", spacer(60))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]
    );
    // Two lines with orphans:3 cannot keep three on the first page either.
    assert_eq!(
        lines(&format!(
            "<div style='orphans:3'>{}z z z z z z</div>",
            spacer(55)
        )),
        [(1, 0.0, 5.0), (1, 0.0, 35.0)]
    );
}

#[test]
fn a_break_that_honors_the_orphans_and_widows_still_does_not_split_a_line() {
    // With one orphan and one widow allowed, the first of two lines stays on
    // the page and the second, which would cross its end, starts the next.
    assert_eq!(
        lines(&format!(
            "<div style='orphans:1;widows:1'>{}z z z z z</div>",
            spacer(65)
        )),
        [(0, 0.0, 70.0), (1, 0.0, 5.0)]
    );
}

#[test]
fn text_inside_a_wrapper_follows_the_same_rule() {
    assert_eq!(
        lines(&format!("<div>{}z</div>", spacer(95))),
        [(1, 0.0, 5.0)]
    );
    // Padding moves the line right but not down from the page top.
    assert_eq!(
        lines(&format!("<div style='padding:5px'>{}z</div>", spacer(90))),
        [(1, 5.0, 5.0)]
    );
}

#[test]
fn each_run_of_lines_between_blocks_is_judged_for_itself() {
    // "z" fits the first page, and "y" comes after the 30px block that runs
    // onto the second page.
    assert_eq!(
        lines(&format!("{}z{}y", spacer(60), spacer(30))),
        [(0, 0.0, 65.0), (1, 0.0, 25.0)]
    );
}

#[test]
fn a_list_item_that_starts_with_a_block_moves_its_text_with_it() {
    assert_eq!(
        lines(&format!(
            "<ul style='margin:0;padding:0;list-style:none'><li>{}z z z z</li></ul>",
            spacer(80)
        )),
        [(1, 0.0, 5.0), (1, 0.0, 35.0)]
    );
}

#[test]
fn the_blocks_after_moved_lines_follow_them() {
    assert_eq!(
        lines(&format!("{}z<div>y</div>", spacer(95))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0)]
    );
    // The blocks nest, and a line follows them.
    assert_eq!(
        lines(&format!(
            "{}z<div><div>y</div><div>w</div></div>v",
            spacer(95)
        )),
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0), (2, 0.0, 5.0)]
    );
    // Three moved lines fill the page, so the block after them starts the
    // next one.
    assert_eq!(
        lines(&format!("{}z z z z z z z<div>y</div>", spacer(60))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0), (2, 0.0, 5.0)]
    );
}

#[test]
fn trailing_lines_keep_their_widows_on_the_next_page() {
    // Four lines would leave three on the first page and one on the next, so
    // the last two go to the next page together.
    assert_eq!(
        lines(&format!("{}z z z z z z z z z z z z", spacer(10))),
        [
            (0, 0.0, 15.0),
            (0, 0.0, 45.0),
            (1, 0.0, 5.0),
            (1, 0.0, 35.0)
        ]
    );
}

#[test]
fn a_run_taller_than_a_page_moves_to_keep_its_orphans() {
    // Four lines are taller than a page. One would stay on the first page,
    // so they start the next one and break two and two.
    assert_eq!(
        lines(&format!("{}z z z z z z z z z z", spacer(70))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (2, 0.0, 5.0), (2, 0.0, 35.0)]
    );
}

#[test]
fn an_inline_element_among_the_moved_lines_does_not_move_them_twice() {
    assert_eq!(
        lines(&format!("{}z z <i>z</i> z z z z", spacer(60))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]
    );
}

#[test]
fn text_in_a_positioned_box_leaves_the_flow_where_it_is() {
    // The box takes no room in the flow, so the line after it stays on the
    // first page.
    for position in [
        "position:absolute;top:0;left:0",
        "position:fixed;top:0;left:0",
    ] {
        assert_eq!(
            lines_of(
                &format!("<div style='{position};width:100px'>{}x</div>z", spacer(80)),
                |text| text == "z"
            ),
            [(0, 0.0, 5.0)],
            "{position}"
        );
    }
}

#[test]
fn a_box_placed_by_its_insets_does_not_keep_the_lines_from_moving() {
    // An absolutely positioned box with a block-axis inset sits where its
    // insets put it, not with the lines, so the line after the block still
    // starts the next page.
    for inset in ["top:150px", "bottom:-150px"] {
        let positioned = format!("<div style='position:absolute;{inset}'>a</div>");
        assert_eq!(
            lines(&format!("{positioned}{}z", spacer(80))),
            [(1, 0.0, 5.0)],
            "{inset}"
        );
    }
}

/// The tops of the lines of the text in `body` on each page from the first to
/// the last with a line.
fn tops_per_page(body: &str) -> Vec<Vec<f32>> {
    let found = lines(body);
    let pages = found.last().map_or(0, |line| line.0 as usize + 1);
    let mut tops = vec![Vec::new(); pages];
    for (page, _, top) in found {
        tops[page as usize].push(top);
    }
    tops
}

/// A 20px-wide paragraph of `lines` words, one to a line.
fn paragraph(lines: usize, style: &str) -> String {
    format!(
        "<div style='width:20px;{style}'>{}</div>",
        vec!["z"; lines].join(" ")
    )
}

/// The tops of the lines of a 20px-wide paragraph of `lines` words that
/// follows a block `height` tall, on each page from the first to the last one
/// with a line.
fn paragraph_tops(height: u32, style: &str, lines: usize) -> Vec<Vec<f32>> {
    tops_per_page(&format!("{}{}", spacer(height), paragraph(lines, style)))
}

#[test]
fn a_paragraph_that_follows_a_block_breaks_between_its_lines() {
    // The block ends 5px into the 30px grid of the lines, so the line that
    // crosses each page end starts the next page and the lines after it follow.
    assert_eq!(
        paragraph_tops(5, "", 5),
        [vec![10.0, 40.0, 70.0], vec![5.0, 35.0]]
    );
    assert_eq!(paragraph_tops(20, "", 3), [vec![25.0, 55.0], vec![5.0]]);
    // The last page takes the line that the widows ask for, and the page
    // before keeps its orphans.
    assert_eq!(
        paragraph_tops(0, "", 7),
        [vec![5.0, 35.0, 65.0], vec![5.0, 35.0], vec![5.0, 35.0]]
    );
    assert_eq!(
        paragraph_tops(0, "widows:3", 7),
        [vec![5.0, 35.0, 65.0], vec![5.0, 35.0], vec![5.0, 35.0]]
    );
    // Orphans that no page holds leave the lines that fit together.
    assert_eq!(
        paragraph_tops(0, "orphans:4", 4),
        [vec![5.0, 35.0, 65.0], vec![5.0]]
    );
}

#[test]
fn a_paragraph_that_follows_a_block_moves_whole_when_its_orphans_do_not_fit() {
    // No line fits after the block, or too few for the orphans: the paragraph
    // starts the next page.
    assert_eq!(
        paragraph_tops(45, "", 5),
        [vec![], vec![5.0, 35.0, 65.0], vec![5.0, 35.0]]
    );
    assert_eq!(
        paragraph_tops(15, "orphans:3;widows:3", 5),
        [vec![], vec![5.0, 35.0, 65.0], vec![5.0, 35.0]]
    );
    // With one orphan and one widow the line that fits stays.
    assert_eq!(
        paragraph_tops(45, "orphans:1;widows:1", 5),
        [vec![50.0], vec![5.0, 35.0, 65.0], vec![5.0]]
    );
}

#[test]
fn paragraphs_that_follow_each_other_break_each_for_itself() {
    // The second paragraph starts where the first ends, on the page its lines
    // were pushed to, and the lines of each cross the page ends of that grid.
    let two = |first: &str, second: &str| format!("{}{first}{second}", spacer(5));
    assert_eq!(
        tops_per_page(&two(&paragraph(5, ""), &paragraph(5, ""))),
        [
            vec![10.0, 40.0, 70.0],
            vec![5.0, 35.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0]
        ]
    );
    assert_eq!(
        tops_per_page(&format!(
            "{}{}{}{}",
            spacer(20),
            paragraph(2, ""),
            paragraph(5, ""),
            paragraph(3, "")
        )),
        [
            vec![25.0, 55.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0],
            vec![5.0, 35.0, 65.0]
        ]
    );
    // Each takes its own orphans and widows.
    assert_eq!(
        tops_per_page(&format!(
            "{}{}{}{}",
            spacer(5),
            paragraph(5, "widows:3"),
            paragraph(5, "orphans:1;widows:1"),
            paragraph(3, "")
        )),
        [
            vec![10.0, 40.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0],
            vec![5.0, 35.0, 65.0]
        ]
    );
}

#[test]
fn paragraphs_in_the_same_box_break_each_for_itself() {
    // The box begins on the first page, and its second paragraph starts on the
    // page its first paragraph's lines were pushed to.
    let expected = [
        vec![10.0, 40.0, 70.0],
        vec![5.0, 35.0],
        vec![5.0, 35.0, 65.0],
        vec![5.0, 35.0],
    ];
    assert_eq!(
        tops_per_page(&format!(
            "{}<div>{}{}</div>",
            spacer(5),
            paragraph(5, ""),
            paragraph(5, "")
        )),
        expected
    );
    assert_eq!(
        tops_per_page(&format!(
            "<div>{}{}{}</div>",
            spacer(5),
            paragraph(5, ""),
            paragraph(5, "")
        )),
        expected
    );
    // The paragraphs of two boxes, and three paragraphs in one.
    assert_eq!(
        tops_per_page(&format!(
            "{}<div>{}</div><div>{}</div>",
            spacer(5),
            paragraph(5, ""),
            paragraph(5, "")
        )),
        expected
    );
    assert_eq!(
        tops_per_page(&format!(
            "{}<div>{}{}{}</div>",
            spacer(5),
            paragraph(5, ""),
            paragraph(5, ""),
            paragraph(5, "")
        )),
        [
            vec![10.0, 40.0, 70.0],
            vec![5.0, 35.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0],
            vec![5.0, 35.0, 65.0],
            vec![5.0, 35.0]
        ]
    );
}

/// A paragraph of `normal` words on lines of 30px and then `tall` words on
/// lines of 40px, in a 20px-wide box.
fn mixed_paragraph(normal: usize, tall: usize, style: &str) -> String {
    format!(
        "<div style='width:20px;{style}'>{} <span style='line-height:40px'>{}</span></div>",
        vec!["z"; normal].join(" "),
        vec!["z"; tall].join(" ")
    )
}

#[test]
fn widows_are_taken_from_lines_of_different_heights() {
    // The lines the first page gives up do not fit the next page with the ones
    // already there, and the last of them starts the page after it.
    assert_eq!(
        tops_per_page(&mixed_paragraph(3, 2, "widows:3")),
        [vec![5.0, 35.0], vec![5.0, 40.0], vec![10.0]]
    );
    assert_eq!(
        tops_per_page(&mixed_paragraph(4, 2, "widows:4")),
        [vec![5.0, 35.0], vec![5.0, 35.0, 70.0], vec![10.0]]
    );
    // Two widows are all the lines the second page holds.
    assert_eq!(
        tops_per_page(&mixed_paragraph(3, 2, "")),
        [vec![5.0, 35.0, 65.0], vec![10.0, 50.0]]
    );
}
