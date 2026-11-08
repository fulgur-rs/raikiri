//! Lines of text that follow a block inside the same parent, and the lines of a
//! paragraph that follows one, around the end of a page. Positions follow
//! Chromium except for the intentional Level 4 `always` compatibility cases.

use raikiri_html::{
    FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, RenderResources, layout,
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
    page_count_and_lines_of(body, wanted).1
}

/// Page count and the selected lines, including pages with no selected text.
fn page_count_and_lines_of(
    body: &str,
    wanted: impl Fn(&str) -> bool,
) -> (u32, Vec<(u32, f32, f32)>) {
    page_count_and_lines_repeated(body, wanted, 1)
}

fn page_count_and_lines_repeated(
    body: &str,
    wanted: impl Fn(&str) -> bool,
    repetitions: usize,
) -> (u32, Vec<(u32, f32, f32)>) {
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
    let mut first = None;
    for _ in 0..repetitions {
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
                if wanted(run.text.trim_end())
                    && !lines.iter().any(|l| l.0 == line.0 && l.2 == line.2)
                {
                    lines.push(line);
                }
            }
        }
        let current = (result.page_count(), lines);
        if let Some(expected) = &first {
            assert_eq!(&current, expected);
        } else {
            first = Some(current);
        }
    }
    first.unwrap()
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

/// The page, top and height of every border box 20px wide in `body`: the
/// paragraphs made by [`paragraph`], on each page they have a part of.
fn paragraph_boxes(body: &str) -> Vec<(u32, f32, f32)> {
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
    let mut boxes: Vec<(u32, f32, f32)> = Vec::new();
    for index in 0..8 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for fragment in page.fragments() {
            let rect = fragment.rect();
            let found = (index, rect.y, rect.height);
            if fragment.kind() == FragmentKind::Box
                && (rect.width - 20.0).abs() < 0.01
                && !boxes.contains(&found)
            {
                boxes.push(found);
            }
        }
    }
    boxes
}

#[test]
fn the_last_line_goes_with_the_padding_and_border_below_it() {
    // Three lines end at 90 on a page of 100px. The 15px of padding below them
    // do not fit, so the last line starts the next page, where the padding
    // follows it: two lines and then one, as in Chromium, with the box ending
    // 45px down the second page.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px", 3),
        [vec![5.0, 35.0], vec![5.0]]
    );
    assert_eq!(
        paragraph_boxes(&paragraph(3, "padding-bottom:15px")),
        [(0, 0.0, 100.0), (1, 0.0, 45.0)]
    );
    // Two lines are split the same way: the break is not one the lines need,
    // so the orphans do not move the paragraph whole.
    assert_eq!(
        paragraph_tops(30, "padding-bottom:15px", 2),
        [vec![35.0], vec![5.0]]
    );
    // A border does the same, and so does padding above the lines.
    assert_eq!(
        paragraph_tops(10, "border-bottom:3px solid red", 3),
        [vec![15.0, 45.0], vec![5.0]]
    );
    assert_eq!(
        paragraph_tops(0, "padding:7px 0;border-top:3px solid red", 3),
        [vec![15.0, 45.0], vec![5.0]]
    );
    // Edges that fit leave the lines where they are.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:10px", 3),
        [vec![5.0, 35.0, 65.0]]
    );
}

#[test]
fn the_edges_below_a_paragraph_follow_the_breaks_its_lines_need() {
    // Three orphans do not move a paragraph whose lines fit.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px;orphans:3", 3),
        [vec![5.0, 35.0], vec![5.0]]
    );
    // Six lines break after three, which leaves the two widows their lines.
    // The padding under the last line does not fit the second page, so that
    // line starts a third one.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px", 6),
        [vec![5.0, 35.0, 65.0], vec![5.0, 35.0], vec![5.0]]
    );
    // Three widows take a line from the first page, and the padding then puts
    // the last line on the page after the second.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px;widows:3", 5),
        [vec![5.0, 35.0], vec![5.0, 35.0], vec![5.0]]
    );
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px;widows:4", 6),
        [vec![5.0, 35.0], vec![5.0, 35.0, 65.0], vec![5.0]]
    );
    // With one orphan the second page gives up lines for the three widows,
    // and the padding then takes the last line of the third.
    assert_eq!(
        paragraph_tops(0, "padding-bottom:15px;orphans:1;widows:3", 7),
        [vec![5.0, 35.0, 65.0], vec![5.0], vec![5.0, 35.0], vec![5.0]]
    );
}

#[test]
fn the_padding_of_a_box_around_a_paragraph_gets_the_same_break() {
    // The padding that closes the box around the paragraph is judged like the
    // paragraph's own: three lines with one orphan and three widows cannot
    // keep both, so the last line goes alone. The same holds with padding on
    // both.
    for (outer, inner) in [
        ("padding-bottom:15px", ""),
        ("padding-bottom:15px", ";padding-bottom:8px"),
        ("padding-bottom:8px", ";padding-bottom:8px"),
    ] {
        assert_eq!(
            tops_per_page(&format!(
                "<div style='{outer}'><div style='width:20px;orphans:1;widows:3{inner}'>z z z</div></div>"
            )),
            [vec![5.0, 35.0], vec![5.0]],
            "{outer} {inner}"
        );
    }
}

#[test]
fn a_paragraph_moved_whole_leaves_the_padding_of_its_box_with_its_last_line() {
    // Three lines do not fit after the block, or too few for the orphans, so
    // they move to the next page whole. The 15px of padding that close the box
    // around them do not fit under them there either, and the last line goes
    // on to a third page with them, as in Chromium.
    for (height, style) in [
        (20, "orphans:3"),
        (30, "orphans:3"),
        (40, "orphans:3"),
        (50, ""),
        (50, "widows:3"),
        (50, "orphans:2;widows:4"),
        (60, ""),
        (60, "orphans:3"),
    ] {
        assert_eq!(
            tops_per_page(&format!(
                "{}<div style='padding-bottom:15px'>{}</div>",
                spacer(height),
                paragraph(3, style)
            )),
            [vec![], vec![5.0, 35.0], vec![5.0]],
            "{height}px block, {style}"
        );
    }
    // A border does the same, and so do the edges of two boxes together.
    for wrapped in [
        "<div style='border-bottom:15px solid red'>PARAGRAPH</div>",
        "<div style='padding-bottom:8px'><div style='padding-bottom:8px'>PARAGRAPH</div></div>",
    ] {
        assert_eq!(
            tops_per_page(&format!(
                "{}{}",
                spacer(50),
                wrapped.replace("PARAGRAPH", &paragraph(3, ""))
            )),
            [vec![], vec![5.0, 35.0], vec![5.0]],
            "{wrapped}"
        );
    }
    // Lines that leave room for the edges stay together.
    for wrapped in [
        "<div style='padding-bottom:8px'>PARAGRAPH</div>",
        "<div style='padding-bottom:4px'><div style='padding-bottom:4px'>PARAGRAPH</div></div>",
    ] {
        assert_eq!(
            tops_per_page(&format!(
                "{}{}",
                spacer(60),
                wrapped.replace("PARAGRAPH", &paragraph(3, ""))
            )),
            [vec![], vec![5.0, 35.0, 65.0]],
            "{wrapped}"
        );
    }
}

#[test]
fn a_block_laid_out_after_the_lines_keeps_the_padding_from_the_last_line() {
    // The padding follows the block after the lines, not the last line, so no
    // line moves for it: the three lines stay on the first page.
    let style = "<style>.a::after{content:'';display:block;height:20px}</style>";
    assert_eq!(
        tops_per_page(&format!(
            "{style}<div class=a style='width:20px;padding-bottom:15px'>z z z</div>"
        )),
        [vec![5.0, 35.0, 65.0]]
    );
    // A box taller than its lines is no different.
    assert_eq!(
        paragraph_tops(0, "min-height:150px;padding-bottom:15px", 3),
        [vec![5.0, 35.0, 65.0]]
    );
}

/// The tops of the lines of a 20px-wide paragraph of `lines` words that
/// follows a block `height` tall and is followed by a one-line marker, on each
/// page from the first to the last one with a line.
fn paragraph_and_marker_tops(height: u32, style: &str, lines: usize) -> Vec<Vec<f32>> {
    let mut tops = tops_per_page(&format!(
        "{}{}<div style='width:20px'>Y</div>",
        spacer(height),
        paragraph(lines, style)
    ));
    for page in &mut tops {
        page.sort_by(f32::total_cmp);
    }
    tops
}

#[test]
fn a_paragraph_with_a_set_height_keeps_the_height_and_breaks_between_its_lines() {
    // The box is 100px tall and starts at 30px, so it crosses the page end,
    // and the lines it holds there break like those of a box that grows.
    for style in ["height:100px", "min-height:100px", "max-height:100px"] {
        assert_eq!(
            paragraph_tops(30, style, 4),
            [vec![35.0, 65.0], vec![5.0, 35.0]],
            "{style}"
        );
    }
    // The lines that overflow a box too short for them go on at the top of
    // the next page.
    assert_eq!(
        paragraph_tops(0, "height:45px;orphans:1;widows:1", 4),
        [vec![5.0, 35.0, 65.0], vec![5.0]]
    );
    // With the orphans and widows of two, the fourth line takes the third
    // with it, and a box that starts on the second page does the same.
    assert_eq!(
        paragraph_tops(30, "height:45px", 4),
        [vec![35.0, 65.0], vec![5.0, 35.0]]
    );
    assert_eq!(
        paragraph_tops(60, "height:45px", 4),
        [vec![], vec![5.0, 35.0], vec![5.0, 35.0]]
    );
    // A box that holds its lines is not touched.
    assert_eq!(
        paragraph_tops(0, "height:100px;orphans:1;widows:1", 6),
        [vec![5.0, 35.0, 65.0], vec![5.0, 35.0, 65.0]]
    );
    assert_eq!(
        paragraph_tops(0, "min-height:60px", 3),
        [vec![5.0, 35.0, 65.0]]
    );
}

#[test]
fn the_padding_below_a_paragraph_with_a_set_height_does_not_pull_its_lines_along() {
    // The padding sits at the end of the 150px box, far below the lines, so
    // it does not make the third line move to the next page with it, as it
    // would for a box that ends with its lines.
    assert_eq!(
        paragraph_tops(0, "height:150px;padding-bottom:15px", 4),
        [vec![5.0, 35.0], vec![5.0, 35.0]]
    );
    assert_eq!(
        paragraph_tops(0, "height:150px;padding-bottom:15px;orphans:1;widows:1", 4),
        [vec![5.0, 35.0, 65.0], vec![5.0]]
    );
    assert_eq!(
        paragraph_tops(
            30,
            "box-sizing:border-box;height:100px;padding-bottom:15px;orphans:1;widows:1",
            4
        ),
        [vec![35.0, 65.0], vec![5.0, 35.0]]
    );
    // A box exactly as tall as its lines ends with them, but its padding does
    // not make the third line follow the fourth: the lines cross the end of
    // the first page and the box keeps its height.
    assert_eq!(
        paragraph_tops(0, "height:120px;padding-bottom:15px;orphans:1;widows:1", 4),
        [vec![5.0, 35.0, 65.0], vec![5.0]]
    );
}

#[test]
fn a_block_after_the_lines_of_a_paragraph_follows_the_lines_that_move() {
    let block = "<div style='height:20px'></div>";
    // The lines break as those of a paragraph that is all text, and the block
    // after them goes along.
    assert_eq!(
        tops_per_page(&format!(
            "{}<div style='width:20px'>z z z{block}</div>",
            spacer(30)
        )),
        [vec![35.0, 65.0], vec![5.0]]
    );
    assert_eq!(
        tops_per_page(&format!(
            "{}<div style='width:20px;orphans:1;widows:1'>z z z{block}</div>",
            spacer(60)
        )),
        [vec![65.0], vec![5.0, 35.0]]
    );
}

#[test]
fn an_out_of_flow_child_at_the_top_of_a_paragraph_does_not_stop_its_lines_moving() {
    // The child sits at a fixed distance from the top of the paragraph, so the
    // lines can move down without it.
    let child = "<span style='position:absolute;left:0;top:0'></span>";
    for orphans_widows in ["", "orphans:1;widows:1"] {
        assert_eq!(
            tops_per_page(&format!(
                "{}<div style='width:20px;position:relative;{orphans_widows}'>z z z z{child}</div>",
                spacer(30)
            )),
            [vec![35.0, 65.0], vec![5.0, 35.0]],
            "{orphans_widows}"
        );
    }
}

#[test]
fn the_box_after_a_paragraph_with_a_set_height_stays_where_the_box_ends() {
    // The lines that overflow the 45px box do not push the marker after it
    // away: it sits at the end of the box, before the overflowing line that
    // starts the next page.
    assert_eq!(
        paragraph_and_marker_tops(0, "height:45px;orphans:1;widows:1", 4),
        [vec![5.0, 35.0, 50.0, 65.0], vec![5.0]]
    );
    // A box that grows with the lines it moves pushes the marker down with
    // it.
    assert_eq!(
        paragraph_and_marker_tops(30, "min-height:150px;orphans:1;widows:1", 4),
        [vec![35.0, 65.0], vec![5.0, 35.0], vec![5.0]]
    );
    assert_eq!(
        paragraph_and_marker_tops(30, "max-height:90px;orphans:1;widows:1", 4),
        [vec![35.0, 65.0], vec![5.0, 25.0, 35.0]]
    );
}

#[test]
fn a_set_height_paragraph_with_a_block_after_its_lines_breaks_between_the_lines() {
    let block = "<div style='height:20px'></div>";
    // The block after the lines is an overflow of a box that keeps its
    // height, or one that has grown to its `min-height` or stopped at its
    // `max-height`, and the lines break as in a paragraph that is all text.
    for style in [
        "height:100px",
        "height:45px",
        "min-height:150px",
        "max-height:90px",
    ] {
        assert_eq!(
            tops_per_page(&format!(
                "{}<div style='width:20px;{style}'>z z z z{block}</div>",
                spacer(30)
            )),
            [vec![35.0, 65.0], vec![5.0, 35.0]],
            "{style}"
        );
    }
    assert_eq!(
        tops_per_page(&format!(
            "<div style='width:20px;height:100px;orphans:1;widows:1'>z z z z{block}</div>"
        )),
        [vec![5.0, 35.0, 65.0], vec![5.0]]
    );
    assert_eq!(
        tops_per_page(&format!(
            "{}<div style='width:20px;min-height:150px;orphans:1;widows:1'>z z z{block}</div>",
            spacer(60)
        )),
        [vec![65.0], vec![5.0, 35.0]]
    );
    // The block after the lines ends the content that the moved lines push
    // past the `min-height`, so the box grows to hold it and the next block
    // starts on the page after.
    assert_eq!(
        tops_per_page(&format!(
            "<div style='width:20px;min-height:150px'>z z z z{block}</div>\
             <div style='width:20px'>Y</div>"
        )),
        [vec![5.0, 35.0], vec![5.0, 35.0], vec![5.0]]
    );
}

#[test]
fn the_box_after_a_set_height_paragraph_with_inline_elements_stays_at_the_box_end() {
    // A later text of the paragraph, in an inline element or after a `<br>`,
    // takes the whole shift of the lines that moved, but the block after the
    // paragraph still follows the end of its box.
    for inner in ["z z z <u>z</u>", "z z z<br>z", "z <u>z z</u> z"] {
        for (style, expected) in [
            (
                "height:45px;orphans:1;widows:1",
                vec![vec![35.0, 65.0], vec![5.0, 35.0]],
            ),
            (
                "max-height:90px;orphans:1;widows:1",
                vec![vec![35.0, 65.0], vec![5.0, 25.0, 35.0]],
            ),
        ] {
            let mut tops = tops_per_page(&format!(
                "{}<div style='width:20px;{style}'>{inner}</div><div style='width:20px'>Y</div>",
                spacer(30)
            ));
            for page in &mut tops {
                page.sort_by(f32::total_cmp);
            }
            assert_eq!(tops, expected, "{inner} {style}");
        }
    }
}

#[test]
fn a_box_taken_out_of_the_flow_after_a_set_height_paragraph_leaves_the_box_end_alone() {
    // An out-of-flow box between the paragraph and the block after it does
    // not stop the flow from going back to the page the box ends on: the
    // block still sits at the end of the box, as it does without the
    // out-of-flow one.
    let positioned = "<div style='position:absolute;left:60px;top:0;width:20px'>F</div>";
    for inner in ["z z z z", "z z z <u>z</u>", "z z z<br>z"] {
        for (style, expected) in [
            (
                "height:45px;orphans:1;widows:1",
                vec![vec![5.0, 35.0, 65.0], vec![5.0, 35.0]],
            ),
            (
                "max-height:90px;orphans:1;widows:1",
                vec![vec![5.0, 35.0, 65.0], vec![5.0, 25.0, 35.0]],
            ),
            (
                "min-height:150px;orphans:1;widows:1",
                vec![vec![5.0, 35.0, 65.0], vec![5.0, 35.0], vec![5.0]],
            ),
        ] {
            let mut tops = tops_per_page(&format!(
                "{}<div style='width:20px;{style}'>{inner}</div>{positioned}\
                 <div style='width:20px'>Y</div>",
                spacer(30)
            ));
            for page in &mut tops {
                page.sort_by(f32::total_cmp);
                page.dedup();
            }
            assert_eq!(tops, expected, "{inner} {style}");
        }
    }
}

#[test]
fn a_set_height_paragraph_that_breaks_more_than_once_keeps_the_block_after_at_its_box_end() {
    // Each page the lines of the paragraph go on to is one more the flow has
    // to come back from to the end of the box.
    let inner = "z z z <u>z z z z z z z z z</u> z";
    for (spacer_height, style, expected) in [
        (
            30,
            "height:45px",
            vec![
                vec![35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0],
            ],
        ),
        (
            0,
            "max-height:90px",
            vec![
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0, 35.0, 65.0],
                vec![5.0],
            ],
        ),
    ] {
        let mut tops = tops_per_page(&format!(
            "{}<div style='width:20px;orphans:1;widows:1;{style}'>{inner}</div>\
             <div style='width:20px'>Y</div>",
            spacer(spacer_height)
        ));
        for page in &mut tops {
            page.sort_by(f32::total_cmp);
            page.dedup();
        }
        assert_eq!(tops, expected, "{style}");
    }
}

/// Where the out-of-flow child of a paragraph stands among its lines.
#[derive(Clone, Copy)]
enum Place {
    Start,
    Middle,
    End,
}

/// The page and top of the `z` lines of a 20px-wide paragraph that follows a
/// block `height` tall, of the `Y` that an out-of-flow child of the paragraph
/// at `place` prints, and of the `Z` of the block after the paragraph.
fn lines_child_and_block_after(
    height: u32,
    style: &str,
    child: &str,
    place: Place,
) -> [Vec<(u32, f32)>; 3] {
    let child = format!("<span style='position:absolute;left:40px;{child}'>Y</span>");
    let inner = match place {
        Place::Start => format!("{child}z z z z"),
        Place::Middle => format!("z z{child} z z"),
        Place::End => format!("z z z z{child}"),
    };
    let body = format!(
        "{}<div style='width:20px;position:relative;{style}'>{inner}</div>\
         <div style='width:20px'>Z</div>",
        spacer(height)
    );
    tops_of_words(&body, ["z", "Y", "Z"])
}

/// The page and top of each line whose first run is one of `words`.
fn tops_of_words<const N: usize>(body: &str, words: [&str; N]) -> [Vec<(u32, f32)>; N] {
    words.map(|word| {
        let mut found: Vec<(u32, f32)> = lines_of(body, |text| text == word)
            .into_iter()
            .map(|(page, _, top)| (page, top))
            .collect();
        found.sort_by(|a, b| a.partial_cmp(b).unwrap());
        found
    })
}

/// The spacer height, the style of the paragraph, where the child stands, and
/// the page and top of the lines, of the child and of the block after.
type ChildCase = (
    u32,
    &'static str,
    Place,
    Vec<(u32, f32)>,
    Vec<(u32, f32)>,
    Vec<(u32, f32)>,
);

fn assert_child_cases(child: &str, cases: Vec<ChildCase>) {
    for (height, style, place, lines, marker, after) in cases {
        assert_eq!(
            lines_child_and_block_after(height, style, child, place),
            [lines, marker, after],
            "{child} after a {height}px block, in {style}"
        );
    }
}

#[test]
fn a_child_at_its_static_position_stays_on_its_line_when_the_lines_move() {
    // Without insets the child sits where its place among the lines puts it,
    // so it goes down with the line it belongs to, onto the next page, and a
    // box that keeps its height or grows does not change that.
    assert_child_cases(
        "",
        vec![
            (
                30,
                "orphans:2;widows:2",
                Place::End,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(1, 35.0)],
                vec![(1, 65.0)],
            ),
            (
                60,
                "orphans:1;widows:1",
                Place::Middle,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 5.0)],
                vec![(2, 5.0)],
            ),
            (
                60,
                "height:45px;orphans:1;widows:1",
                Place::Middle,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 5.0)],
                vec![(1, 10.0)],
            ),
            (
                60,
                "height:45px;orphans:2;widows:2",
                Place::End,
                vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
                vec![(2, 35.0)],
                vec![(1, 50.0)],
            ),
            (
                30,
                "max-height:90px;orphans:1;widows:1",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(0, 35.0)],
                vec![(1, 25.0)],
            ),
            (
                60,
                "min-height:150px;orphans:1;widows:1",
                Place::End,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 65.0)],
                vec![(2, 15.0)],
            ),
        ],
    );
}

#[test]
fn a_child_at_the_bottom_of_a_paragraph_follows_the_end_of_its_last_fragment() {
    // The bottom of the paragraph is where the lines that moved end, so the
    // child, which the `bottom` places against it, moves along with them.
    assert_child_cases(
        "bottom:0",
        vec![
            (
                30,
                "orphans:2;widows:2",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(1, 35.0)],
                vec![(1, 65.0)],
            ),
            (
                60,
                "orphans:1;widows:1",
                Place::Middle,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 65.0)],
                vec![(2, 5.0)],
            ),
            (
                30,
                "height:45px;orphans:1;widows:1",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(0, 50.0)],
                vec![(1, 5.0)],
            ),
            (
                60,
                "max-height:90px;orphans:1;widows:1",
                Place::End,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 25.0)],
                vec![(1, 55.0)],
            ),
            (
                30,
                "min-height:150px;orphans:2;widows:2",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(1, 55.0)],
                vec![(2, 5.0)],
            ),
        ],
    );
}

#[test]
fn a_child_placed_by_a_percentage_of_the_paragraph_follows_the_height_it_grows_to() {
    // A `top` that is a percentage, or a `calc()` with one, resolves against
    // the height of the paragraph with the lines that moved, so it moves with
    // the growth of the box rather than with the lines.
    assert_child_cases(
        "top:50%",
        vec![
            (
                60,
                "orphans:1;widows:1",
                Place::Start,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 30.0)],
                vec![(2, 5.0)],
            ),
            (
                60,
                "max-height:90px;orphans:1;widows:1",
                Place::Middle,
                vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
                vec![(1, 10.0)],
                vec![(1, 55.0)],
            ),
            (
                30,
                "min-height:150px;orphans:1;widows:1",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(1, 10.0)],
                vec![(2, 5.0)],
            ),
        ],
    );
    assert_child_cases(
        "top:calc(10px + 10%)",
        vec![
            (
                30,
                "orphans:1;widows:1",
                Place::Middle,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(0, 58.0)],
                vec![(1, 65.0)],
            ),
            (
                30,
                "max-height:90px;orphans:1;widows:1",
                Place::Start,
                vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
                vec![(0, 54.0)],
                vec![(1, 25.0)],
            ),
        ],
    );
}

/// A 20px-wide paragraph that follows a block `height` tall and holds `inner`,
/// with the `Z` block after it.
fn paragraph_html(height: u32, style: &str, inner: &str) -> String {
    format!(
        "{}<div style='width:20px;position:relative;{style}'>{inner}</div>\
         <div style='width:20px'>Z</div>",
        spacer(height)
    )
}

#[test]
fn a_block_level_child_at_its_static_position_stays_at_the_bottom_of_its_line() {
    // A child that was block-level sits below the line it follows text on,
    // so the lines that move away from that line leave it behind, and the
    // lines that include it take it along.
    let child = "<div style='position:absolute;left:40px;line-height:6px'>Y</div>";
    for (height, style, inner, lines, marker, after) in [
        (
            30,
            "orphans:1;widows:1",
            format!("z z{child} z z"),
            vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
            vec![(0, 83.0)],
            vec![(1, 65.0)],
        ),
        (
            60,
            "orphans:1;widows:1",
            format!("z z{child} z z"),
            vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(1, 23.0)],
            vec![(2, 5.0)],
        ),
        (
            60,
            "orphans:2;widows:2",
            format!("z z z z{child}"),
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
            vec![(2, 53.0)],
            vec![(2, 65.0)],
        ),
        (
            30,
            "orphans:2;widows:2",
            format!("{child}z z z z"),
            vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
            vec![(0, 23.0)],
            vec![(1, 65.0)],
        ),
    ] {
        assert_eq!(
            tops_of_words(&paragraph_html(height, style, &inner), ["z", "Y", "Z"]),
            [lines, marker, after],
            "{inner} after a {height}px block, in {style}"
        );
    }
}

#[test]
fn several_out_of_flow_children_of_a_paragraph_each_follow_their_own_place() {
    let top = "<span style='position:absolute;left:40px;top:5px'>R</span>";
    let line = "<span style='position:absolute;left:40px'>P</span>";
    let bottom = "<span style='position:absolute;left:40px;bottom:0'>Q</span>";
    let inner = format!("{top}z z{line} z z{bottom}");
    // The child with a `top` stays, the one at its static position goes with
    // its line, and the one at the `bottom` with the end of the paragraph.
    assert_eq!(
        tops_of_words(
            &paragraph_html(30, "orphans:1;widows:1", &inner),
            ["z", "P", "Q", "R", "Z"]
        ),
        [
            vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
            vec![(0, 65.0)],
            vec![(1, 35.0)],
            vec![(0, 40.0)],
            vec![(1, 65.0)],
        ]
    );
    assert_eq!(
        tops_of_words(
            &paragraph_html(60, "orphans:2;widows:2", &inner),
            ["z", "P", "Q", "R", "Z"]
        ),
        [
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
            vec![(1, 35.0)],
            vec![(2, 35.0)],
            vec![(1, 10.0)],
            vec![(2, 65.0)],
        ]
    );
}

#[test]
fn out_of_flow_children_follow_a_paragraph_that_breaks_more_than_once() {
    // Each break moves the lines after it again, so the child at the bottom
    // ends up with the last lines, and the one on a line with that line.
    let bottom = "<span style='position:absolute;left:40px;bottom:0'>Q</span>";
    let line = "<span style='position:absolute;left:40px'>P</span>";
    assert_eq!(
        tops_of_words(
            &paragraph_html(
                60,
                "orphans:1;widows:1",
                &format!("{bottom}z z z z z z z z z")
            ),
            ["z", "Q", "Z"]
        ),
        [
            vec![
                (0, 65.0),
                (1, 5.0),
                (1, 35.0),
                (1, 65.0),
                (2, 5.0),
                (2, 35.0),
                (2, 65.0),
                (3, 5.0),
                (3, 35.0)
            ],
            vec![(3, 35.0)],
            vec![(3, 65.0)],
        ]
    );
    assert_eq!(
        tops_of_words(
            &paragraph_html(
                60,
                "orphans:1;widows:1",
                &format!("z z z z{line} z z z z z")
            ),
            ["P", "Z"]
        ),
        [vec![(1, 65.0)], vec![(3, 65.0)]]
    );
    assert_eq!(
        tops_of_words(
            &paragraph_html(
                30,
                "orphans:2;widows:2",
                &format!("z z{line} z z z z z z z{bottom}")
            ),
            ["P", "Q", "Z"]
        ),
        [vec![(0, 65.0)], vec![(3, 35.0)], vec![(3, 65.0)]]
    );
}

#[test]
fn out_of_flow_children_follow_the_lines_that_move_after_a_block_child() {
    // The lines after the block child move down inside the paragraph, and the
    // children placed by a line, the `bottom` or a percentage follow them as
    // in a paragraph that is all text.
    let block = |height: u32| format!("<div style='height:{height}px'></div>");
    let child = |style: &str| format!("<span style='position:absolute;left:40px;{style}'>Y</span>");
    for (height, style, inner, lines, marker, after) in [
        (
            30,
            "orphans:1;widows:1",
            format!("{}z z{} z z", block(30), child("")),
            vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(1, 5.0)],
            vec![(2, 5.0)],
        ),
        (
            30,
            "orphans:1;widows:1",
            format!("{}z z z z{}", block(30), child("bottom:0")),
            vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(1, 65.0)],
            vec![(2, 5.0)],
        ),
        (
            30,
            "orphans:1;widows:1",
            format!("{}{}z z z z", block(30), child("top:50%")),
            vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(1, 15.0)],
            vec![(2, 5.0)],
        ),
        (
            60,
            "orphans:2;widows:2",
            format!("{}z z z z{}", block(50), child("")),
            vec![(1, 15.0), (1, 45.0), (2, 5.0), (2, 35.0)],
            vec![(2, 35.0)],
            vec![(2, 65.0)],
        ),
        (
            60,
            "orphans:2;widows:2",
            format!("{}{}z z z z", block(50), child("bottom:0")),
            vec![(1, 15.0), (1, 45.0), (2, 5.0), (2, 35.0)],
            vec![(2, 35.0)],
            vec![(2, 65.0)],
        ),
        (
            0,
            "orphans:2;widows:2",
            format!("{}z z z z{}", block(50), child("top:50%")),
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
            vec![(1, 35.0)],
            vec![(2, 65.0)],
        ),
        (
            30,
            "orphans:1;widows:1",
            format!("{}{}z z z z", block(20), child("")),
            vec![(0, 55.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(0, 55.0)],
            vec![(2, 5.0)],
        ),
    ] {
        assert_eq!(
            tops_of_words(&paragraph_html(height, style, &inner), ["z", "Y", "Z"]),
            [lines, marker, after],
            "{inner} after a {height}px block, in {style}"
        );
    }
}

#[test]
fn text_of_a_box_far_down_outside_the_flow_does_not_move_the_flow_to_its_page() {
    // The box falls on the third page by its `top`, which says nothing about
    // the page the flow is on: the text of the flow stays on the first page
    // and a forced break goes to the second.
    let far = "<div style='position:absolute;left:40px;top:250px'>Y</div>";
    let near = "<div style='position:absolute;left:40px;top:120px'>Y</div>";
    let flow = "<div style='width:20px'>z</div>";
    let broken = |name: &str| format!("<div style='break-before:page;width:20px'>{name}</div>");
    for (body, lines, marker, after_breaks) in [
        (
            format!("{far}<div style='width:20px'>z z z</div>"),
            vec![(0, 5.0), (0, 35.0), (0, 65.0)],
            vec![(2, 55.0)],
            vec![],
        ),
        (
            format!("{far}{flow}{}", broken("Q")),
            vec![(0, 5.0)],
            vec![(2, 55.0)],
            vec![(1, 5.0)],
        ),
        (
            format!("{flow}{far}{}", broken("Q")),
            vec![(0, 5.0)],
            vec![(2, 55.0)],
            vec![(1, 5.0)],
        ),
        (
            format!("{near}{flow}{}", broken("Q")),
            vec![(0, 5.0)],
            vec![(1, 25.0)],
            vec![(1, 5.0)],
        ),
        (
            format!("{far}{flow}{}{}", broken("Q"), broken("R")),
            vec![(0, 5.0)],
            vec![(2, 55.0)],
            vec![(1, 5.0), (2, 5.0)],
        ),
    ] {
        let [z, y, q, r] = tops_of_words(&body, ["z", "Y", "Q", "R"]);
        let mut breaks = q;
        breaks.extend(r);
        assert_eq!([z, y, breaks], [lines, marker, after_breaks], "{body}");
    }
}

#[test]
fn out_of_flow_children_inside_an_inline_element_follow_the_lines_too() {
    // The child sits in an inline element that is not its containing block,
    // so the lines of the paragraph still decide where it goes.
    let nested = |style: &str| {
        format!("z z<u><span style='position:absolute;left:40px;{style}'>Y</span></u> z z")
    };
    for (height, style, child, lines, marker, after) in [
        (
            30,
            "orphans:1;widows:1",
            "",
            vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
            vec![(0, 65.0)],
            vec![(1, 65.0)],
        ),
        (
            60,
            "orphans:1;widows:1",
            "",
            vec![(0, 65.0), (1, 5.0), (1, 35.0), (1, 65.0)],
            vec![(1, 5.0)],
            vec![(2, 5.0)],
        ),
        (
            60,
            "orphans:2;widows:2",
            "",
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
            vec![(1, 35.0)],
            vec![(2, 65.0)],
        ),
        (
            30,
            "orphans:1;widows:1",
            "bottom:0",
            vec![(0, 35.0), (0, 65.0), (1, 5.0), (1, 35.0)],
            vec![(1, 35.0)],
            vec![(1, 65.0)],
        ),
        (
            60,
            "orphans:2;widows:2",
            "bottom:0",
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
            vec![(2, 35.0)],
            vec![(2, 65.0)],
        ),
    ] {
        assert_eq!(
            tops_of_words(
                &paragraph_html(height, style, &nested(child)),
                ["z", "Y", "Z"]
            ),
            [lines, marker, after],
            "{child} after a {height}px block, in {style}"
        );
    }
}

fn assert_break_placement(
    property: &str,
    value: &str,
    expected_pages: u32,
    expected_lines: &[(u32, f32, f32)],
) {
    let lead_style = if property.ends_with("after") {
        format!("height:30px;{property}:{value}")
    } else {
        "height:30px".to_owned()
    };
    let target_style = if property.ends_with("before") {
        format!("{property}:{value}")
    } else {
        String::new()
    };
    let body = format!(
        "<div id=lead style='{lead_style}'>a</div><div id=target style='{target_style}'>XX XX XX XX</div>"
    );
    let (pages, lines) = page_count_and_lines_of(&body, |text| text != "a");
    assert_eq!(pages, expected_pages, "{property}:{value}: page count");
    assert_eq!(
        lines.as_slice(),
        expected_lines,
        "{property}:{value}: lines"
    );
}

#[test]
fn modern_always_retains_the_level_four_paged_break_contract() {
    // Chromium 152 rejects modern `always`; these are the retained Level 4
    // expectations, distinct from its measured page-0 lines at 35px and 65px.
    for property in ["break-before", "break-after"] {
        assert_break_placement(property, "always", 2, &[(1, 0.0, 5.0), (1, 0.0, 35.0)]);
    }
}

#[test]
fn page_and_legacy_always_keep_their_explicit_page_breaks() {
    for (property, value) in [
        ("break-before", "page"),
        ("break-after", "page"),
        ("page-break-before", "always"),
        ("page-break-after", "always"),
    ] {
        assert_break_placement(property, value, 2, &[(1, 0.0, 5.0), (1, 0.0, 35.0)]);
    }
}

#[test]
fn auto_keeps_text_after_the_leading_block_on_the_same_page() {
    for property in ["break-before", "break-after"] {
        assert_break_placement(property, "auto", 1, &[(0, 0.0, 35.0), (0, 0.0, 65.0)]);
    }
}

#[test]
fn modern_always_targets_the_nearest_column_context() {
    for value in ["always", "column"] {
        let body = format!(
            "<div style='columns:2;column-fill:auto;column-gap:0;width:100px;height:90px'>\
             <div id=lead style='height:30px'>a</div>\
             <div id=target style='break-before:{value}'>XX XX</div></div>"
        );
        let (pages, lines) = page_count_and_lines_of(&body, |text| text != "a");
        assert_eq!(pages, 1, "{value}: the nearest context is a column");
        assert_eq!(lines, [(0, 50.0, 5.0), (0, 50.0, 35.0)], "{value}");
    }
}

struct FloatMarginCase {
    label: &'static str,
    body: &'static str,
    pages: u32,
    lines: &'static [(u32, f32, f32)],
}

#[test]
fn page_start_float_keeps_the_margin_before_rebreaking_lines() {
    let cases = &[
        FloatMarginCase {
            label: "left-h0-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "left-h10-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:10px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "left-h20-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h35-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h40-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:40px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h50-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 40.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h5-m0-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 5.00000),
                (1, 0.00000, 35.00000),
                (1, 0.00000, 65.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h35-m0-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 5.00000),
                (1, 40.00000, 35.00000),
                (1, 0.00000, 65.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h0-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "right-h5-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "right-h10-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:10px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "right-h20-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h35-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h40-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:40px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h50-m10-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h5-m0-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:5px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 5.00000),
                (1, 0.00000, 35.00000),
                (1, 0.00000, 65.00000),
            ],
        },
        FloatMarginCase {
            label: "right-h35-m0-p100-n0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:35px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 0.00000, 5.00000),
                (1, 0.00000, 35.00000),
                (1, 0.00000, 65.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n5",
            body: "<div style='height:5px'></div><section><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div></section>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "left-h5-m10-p200-n0",
            body: "<style>@page{size:100px 200px}</style><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.00000, 15.00000), (1, 0.00000, 45.00000)],
        },
        FloatMarginCase {
            label: "left-h35-m10-p100-n5",
            body: "<div style='height:5px'></div><section><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div></section>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
        FloatMarginCase {
            label: "left-h35-m10-p200-n0",
            body: "<style>@page{size:100px 200px}</style><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[
                (1, 40.00000, 15.00000),
                (1, 0.00000, 45.00000),
                (1, 0.00000, 75.00000),
            ],
        },
    ];
    let mut mismatches = Vec::new();
    for case in cases {
        let (pages, lines) = page_count_and_lines_of(case.body, |text| text != "a");
        if pages != case.pages || lines.as_slice() != case.lines {
            mismatches.push(format!(
                "{}: got {pages} pages {lines:?}, expected {} pages {:?}",
                case.label, case.pages, case.lines
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn page_start_float_margin_reflows_with_offsets_and_repeated_breaks() {
    let cases = [
        FloatMarginCase {
            label: "left-h5-m10-p100-n0-fm3",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;margin-top:3px;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0)],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n0-fm20",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;margin-top:20px;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-h35-m10-p100-n0-fm3",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;margin-top:3px;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-h35-m10-p100-n0-fm20",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;margin-top:20px;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-h5-m10-p100-n0-fm3",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;margin-top:3px;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0)],
        },
        FloatMarginCase {
            label: "right-h5-m10-p100-n0-fm20",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;margin-top:20px;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-h35-m10-p100-n0-fm3",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;margin-top:3px;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-h35-m10-p100-n0-fm20",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;margin-top:20px;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n0-relative3",
            body: "<section style='position:relative;top:3px'><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div></section>",
            pages: 2,
            lines: &[(1, 0.0, 18.0), (1, 0.0, 48.0)],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n0-three-groups",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 4,
            lines: &[
                (1, 0.0, 15.0),
                (1, 0.0, 45.0),
                (2, 0.0, 15.0),
                (2, 0.0, 45.0),
                (3, 0.0, 15.0),
                (3, 0.0, 45.0),
            ],
        },
        FloatMarginCase {
            label: "left-h35-m10-p100-n0-three-groups",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:35px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 6,
            lines: &[
                (1, 40.0, 15.0),
                (1, 0.0, 45.0),
                (1, 0.0, 75.0),
                (3, 40.0, 15.0),
                (3, 0.0, 45.0),
                (3, 0.0, 75.0),
                (5, 40.0, 15.0),
                (5, 0.0, 45.0),
                (5, 0.0, 75.0),
            ],
        },
        FloatMarginCase {
            label: "left-h5-m0-p100-n0-named",
            body: "<style>@page narrow{size:80px 100px}section{page:narrow}</style><section><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div></section>",
            pages: 3,
            lines: &[
                (1, 40.0, 5.0),
                (1, 0.0, 35.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
            ],
        },
        FloatMarginCase {
            label: "left-h5-m10-p100-n0-named",
            body: "<style>@page narrow{size:80px 100px}section{page:narrow}</style><section><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:5px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div></section>",
            pages: 3,
            lines: &[
                (1, 0.0, 15.0),
                (1, 0.0, 45.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
            ],
        },
        FloatMarginCase {
            label: "left-h0-m10-p100-n0-three-groups-control",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 4,
            lines: &[
                (1, 0.0, 15.0),
                (1, 0.0, 45.0),
                (2, 0.0, 15.0),
                (2, 0.0, 45.0),
                (3, 0.0, 15.0),
                (3, 0.0, 45.0),
            ],
        },
        FloatMarginCase {
            label: "left-h10-m10-p100-n0-named80-percent10",
            body: "<style>@page narrow{size:80px 100px}section{page:narrow}</style><section><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:10px'></div><div id=target style='margin-top:10%'>XX XX XX XX</div></section>",
            pages: 3,
            lines: &[
                (1, 40.0, 13.0),
                (1, 0.0, 43.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
            ],
        },
    ];
    let mut mismatches = Vec::new();
    for case in cases {
        let (pages, mut lines) = page_count_and_lines_repeated(case.body, |text| text != "a", 2);
        // PDF extraction may order a float-adjacent line after a full-width line.
        lines.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.2.total_cmp(&b.2))
                .then(a.1.total_cmp(&b.1))
        });
        if pages != case.pages || lines.as_slice() != case.lines {
            mismatches.push(format!(
                "{}: got {pages} pages {lines:?}, expected {} pages {:?}",
                case.label, case.pages, case.lines
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_retained_margin_can_move_a_short_line_into_a_floats_upper_edge() {
    let body = "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;margin-top:35px;width:80px;height:5px'></div><div style='margin-top:10px'>XX XX</div>";
    assert_eq!(
        page_count_and_lines_repeated(body, |text| text != "a", 2),
        (2, vec![(1, 0.0, 45.0)])
    );
}

#[test]
fn page_start_float_keeps_its_own_margins_separate_from_the_paragraph_margin() {
    // The reported single-margin cases finish on the float's initial page.
    // Zero margins are controls; continued paragraphs are tracked separately.
    let cases = [
        FloatMarginCase {
            label: "left-before0-flow0-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "left-before0-flow10-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before0-flow10-top20-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:20px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before0-flow10-top0-bottom15",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:15px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before30-flow0-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "left-before30-flow10-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before30-flow10-top20-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:20px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before30-flow10-top0-bottom15",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px;margin-top:0px;margin-bottom:15px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before0-flow0-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "right-before0-flow10-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before0-flow10-top20-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:20px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before0-flow10-top0-bottom15",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:15px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before30-flow0-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:0px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "right-before30-flow10-top0-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before30-flow10-top20-bottom0",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:20px;margin-bottom:0px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before30-flow10-top0-bottom15",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px;margin-top:0px;margin-bottom:15px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
    ];
    let mut mismatches = Vec::new();
    for case in cases {
        let (pages, mut lines) = page_count_and_lines_repeated(case.body, |text| text != "a", 2);
        lines.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.2.total_cmp(&b.2))
                .then(a.1.total_cmp(&b.1))
        });
        if pages != case.pages || lines.as_slice() != case.lines {
            mismatches.push(format!(
                "{}: got {pages} pages {lines:?}, expected {} pages {:?}",
                case.label, case.pages, case.lines
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_large_margin_after_a_page_start_float_does_not_insert_an_extra_blank_page() {
    let cases = [
        FloatMarginCase {
            label: "left-before0-h20-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before0-h20-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:20px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "left-before0-h50-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before0-h50-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "left-before30-h20-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before30-h20-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:20px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "left-before30-h50-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "left-before30-h50-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "right-before0-h20-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before0-h20-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:20px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "right-before0-h50-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before0-h50-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:0px'>a</div><div id=float style='float:right;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "right-before30-h20-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:20px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before30-h20-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:20px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "right-before30-h50-margin10",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px'></div><div id=target style='margin-top:10px'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)],
        },
        FloatMarginCase {
            label: "right-before30-h50-margin50",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=float style='float:right;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
    ];
    let mut mismatches = Vec::new();
    for case in cases {
        let (pages, mut lines) = page_count_and_lines_repeated(case.body, |text| text != "a", 2);
        lines.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.2.total_cmp(&b.2))
                .then(a.1.total_cmp(&b.1))
        });
        if pages != case.pages || lines.as_slice() != case.lines {
            mismatches.push(format!(
                "{}: got {pages} pages {lines:?}, expected {} pages {:?}",
                case.label, case.pages, case.lines
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn oversized_widows_follow_the_fragment_that_opened_the_paragraph() {
    // Chromium 152 PDF positions with Ahem 20px/30px on 100px pages; the
    // final case makes the following pages 140px tall.
    let cases = [
        (
            "lines4-lead0-widows3",
            "<div style=\"width:20px;widows:3\">z z z z</div>",
            2,
            vec![(0, 0.0, 5.0), (0, 0.0, 35.0), (1, 0.0, 5.0), (1, 0.0, 35.0)],
        ),
        (
            "lines4-lead0-widows4",
            "<div style=\"width:20px;widows:4\">z z z z</div>",
            2,
            vec![(0, 0.0, 5.0), (0, 0.0, 35.0), (1, 0.0, 5.0), (1, 0.0, 35.0)],
        ),
        (
            "lines4-lead45-widows3",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:3\">z z z z</div>",
            3,
            vec![(1, 0.0, 5.0), (1, 0.0, 35.0), (2, 0.0, 5.0), (2, 0.0, 35.0)],
        ),
        (
            "lines4-lead45-widows4",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:4\">z z z z</div>",
            3,
            vec![(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0), (2, 0.0, 5.0)],
        ),
        (
            "lines6-lead0-widows3",
            "<div style=\"width:20px;widows:3\">z z z z z z</div>",
            2,
            vec![
                (0, 0.0, 5.0),
                (0, 0.0, 35.0),
                (0, 0.0, 65.0),
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
            ],
        ),
        (
            "lines6-lead0-widows4",
            "<div style=\"width:20px;widows:4\">z z z z z z</div>",
            3,
            vec![
                (0, 0.0, 5.0),
                (0, 0.0, 35.0),
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
            ],
        ),
        (
            "lines6-lead45-widows3",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:3\">z z z z z z</div>",
            3,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
            ],
        ),
        (
            "lines6-lead45-widows4",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:4\">z z z z z z</div>",
            3,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
            ],
        ),
        (
            "lines7-lead0-widows3",
            "<div style=\"width:20px;widows:3\">z z z z z z z</div>",
            3,
            vec![
                (0, 0.0, 5.0),
                (0, 0.0, 35.0),
                (0, 0.0, 65.0),
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
            ],
        ),
        (
            "lines7-lead0-widows4",
            "<div style=\"width:20px;widows:4\">z z z z z z z</div>",
            3,
            vec![
                (0, 0.0, 5.0),
                (0, 0.0, 35.0),
                (0, 0.0, 65.0),
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
            ],
        ),
        (
            "lines7-lead45-widows3",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:3\">z z z z z z z</div>",
            4,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (3, 0.0, 5.0),
                (3, 0.0, 35.0),
            ],
        ),
        (
            "lines7-lead45-widows4",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:4\">z z z z z z z</div>",
            4,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
                (3, 0.0, 5.0),
            ],
        ),
        (
            "widows5",
            "<div style=\"height:45px\"></div><div style=\"width:20px;widows:5\">z z z z z z</div>",
            3,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
            ],
        ),
        (
            "orphans1",
            "<div style=\"height:45px\"></div><div style=\"width:20px;orphans:1;widows:4\">z z z z</div>",
            2,
            vec![
                (0, 0.0, 50.0),
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (1, 0.0, 65.0),
            ],
        ),
        (
            "block-run",
            "<div style=\"width:20px;widows:4\"><div style=\"height:45px\"></div>z z z z</div>",
            3,
            vec![(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0), (2, 0.0, 5.0)],
        ),
        (
            "taller-pages",
            "<style>@page{size:100px 140px}@page:first{size:100px 100px}</style><div style=\"height:45px\"></div><div style=\"width:20px;widows:4\">z z z z z z</div>",
            3,
            vec![
                (1, 0.0, 5.0),
                (1, 0.0, 35.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
                (2, 0.0, 95.0),
            ],
        ),
    ];
    for (name, body, pages, expected) in cases {
        assert_eq!(
            page_count_and_lines_repeated(body, |_| true, 2),
            (pages, expected),
            "{name}"
        );
    }
}

#[test]
fn a_leading_padded_holder_keeps_a_fitting_line_when_orphans_cannot_be_met() {
    // Chromium 152 keeps the first line despite either line constraint.
    for (text, expected) in [
        ("z z", vec![(0, 0.0, 75.0), (1, 0.0, 5.0)]),
        ("z z z", vec![(0, 0.0, 75.0), (1, 0.0, 5.0), (1, 0.0, 35.0)]),
    ] {
        for rules in ["", "orphans:3", "widows:3", "orphans:3;widows:3"] {
            let body = format!(
                "<div style='padding-top:70px'><div style='width:20px;{rules}'>{text}</div></div>"
            );
            assert_eq!(
                page_count_and_lines_repeated(&body, |_| true, 2),
                (2, expected.clone()),
                "{text}: {rules}"
            );
        }
    }
    // The same leading space can belong to more than one plain holder.
    assert_eq!(
        page_count_and_lines_repeated(
            "<div style='padding-top:20px'><div style='padding-top:50px'><div style='width:20px'>z z z</div></div></div>",
            |_| true,
            2,
        ),
        (2, vec![(0, 0.0, 75.0), (1, 0.0, 5.0), (1, 0.0, 35.0)])
    );
    // A page-start float can leave the same leading space inside a holder.
    assert_eq!(
        page_count_and_lines_repeated(
            "<div style='float:left;width:20px;height:70px'></div><section><div style='width:20px'>z z</div></section>",
            |_| true,
            2,
        ),
        (2, vec![(0, 0.0, 75.0), (1, 0.0, 5.0)])
    );
}

#[test]
fn a_padded_holder_still_breaks_when_no_line_fits_or_content_precedes_it() {
    for (text, expected) in [
        ("z z", vec![(1, 0.0, 5.0), (1, 0.0, 35.0)]),
        ("z z z", vec![(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]),
    ] {
        let body =
            format!("<div style='padding-top:85px'><div style='width:20px'>{text}</div></div>");
        assert_eq!(
            page_count_and_lines_repeated(&body, |_| true, 2),
            (2, expected)
        );
    }
    assert_eq!(
        page_count_and_lines_repeated(
            "<div style='height:50px'></div><div style='padding-top:20px'><div style='width:20px'>z z</div></div>",
            |_| true,
            2,
        ),
        (2, vec![(1, 0.0, 25.0), (1, 0.0, 55.0)])
    );
}

#[test]
fn a_retained_page_high_margin_does_not_open_an_extra_page_after_a_float() {
    // RAIKIRI-1444: Chromium 152 prints both lines on page 2. The margin
    // is retained on page 1 and truncated at its next unforced edge.
    for side in ["left", "right"] {
        for margin in [80, 99, 100, 101, 150, 200] {
            let body = format!(
                "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div>\
                 <div style='float:{side};width:40px;height:50px'></div>\
                 <div style='margin-top:{margin}px'>XX XX XX XX</div>"
            );
            assert_eq!(
                page_count_and_lines_repeated(&body, |text| text != "a", 2),
                (3, vec![(2, 0.0, 5.0), (2, 0.0, 35.0)]),
                "{side} float, margin {margin}px"
            );
        }
    }
}

#[test]
fn retained_margin_paragraphs_follow_their_opening_page_boundary() {
    let cases = [
        FloatMarginCase {
            label: "float-o1-w1",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;orphans:1;widows:1'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "float-o1-w2",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;orphans:1;widows:2'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "float-o2-w1",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;orphans:2;widows:1'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "float-o2-w2",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;orphans:2;widows:2'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "float-o3-w3",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;orphans:3;widows:3'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "no-float-forced-after",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div id=target style='margin-top:50px;'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "no-float-forced-before",
            body: "<div style='height:30px;margin-bottom:30px'>a</div><div id=target style='margin-top:50px;break-before:page'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "real-prefix-no-forced",
            body: "<div style='height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "real-content-after-float",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div style='height:10px'></div><div id=target style='margin-top:50px;'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(2, 0.0, 5.0), (2, 0.0, 35.0)],
        },
        FloatMarginCase {
            label: "named-page-no-float",
            body: "<style>@page b{size:100px 100px}</style><div style='height:30px;margin-bottom:30px'>a</div><div id=target style='margin-top:50px;page:b'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "float-margin80",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:80px;'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(2, 0.0, 5.0), (2, 0.0, 35.0)],
        },
        FloatMarginCase {
            label: "float-avoid",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px;break-inside:avoid'>XX XX XX XX</div>",
            pages: 3,
            lines: &[(1, 0.0, 55.0), (2, 0.0, 5.0)],
        },
        FloatMarginCase {
            label: "own-float-break-before",
            body: "<div style='height:30px;margin-bottom:30px'>a</div><div style='float:left;break-before:page;width:40px;height:50px'></div><div id=target style='margin-top:50px;'>XX XX XX XX</div>",
            pages: 2,
            lines: &[(1, 40.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)],
        },
        FloatMarginCase {
            label: "three-margin50-groups",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div><div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX</div>",
            pages: 7,
            lines: &[
                (1, 0.0, 55.0),
                (2, 0.0, 5.0),
                (3, 0.0, 55.0),
                (4, 0.0, 5.0),
                (5, 0.0, 55.0),
                (6, 0.0, 5.0),
            ],
        },
        FloatMarginCase {
            label: "long-margin50-paragraph",
            body: "<div style='break-after:page;height:30px;margin-bottom:30px'>a</div><div style='float:left;width:40px;height:50px'></div><div id=target style='margin-top:50px'>XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX</div>",
            pages: 5,
            lines: &[
                (1, 0.0, 55.0),
                (2, 0.0, 5.0),
                (2, 0.0, 35.0),
                (2, 0.0, 65.0),
                (3, 0.0, 5.0),
                (3, 0.0, 35.0),
                (4, 0.0, 5.0),
                (4, 0.0, 35.0),
            ],
        },
    ];
    let mut mismatches = Vec::new();
    for case in cases {
        let (pages, mut lines) = page_count_and_lines_repeated(case.body, |text| text != "a", 2);
        lines.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.2.total_cmp(&b.2))
                .then(a.1.total_cmp(&b.1))
        });
        if pages != case.pages || lines.as_slice() != case.lines {
            mismatches.push(format!(
                "{}: got {pages} pages {lines:?}, expected {} pages {:?}",
                case.label, case.pages, case.lines
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_float_stays_on_the_page_above_the_large_retained_margin() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    for (side, x) in [("left", 0.0), ("right", 60.0)] {
        let html = format!(
            "<!doctype html><style>@page{{size:100px 100px;margin:0}}body{{margin:0;font:20px/30px Ahem}}</style><body>\
             <div style='break-after:page;height:30px;margin-bottom:30px'>a</div>\
             <div style='float:{side};width:40px;height:50px;background:red'>F</div>\
             <div style='margin-top:50px'>XX XX XX XX</div></body>"
        );
        let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        for _ in 0..2 {
            let LayoutStatus::Completed(result) = layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new().resources(&resources),
            )
            .unwrap() else {
                panic!("layout must complete")
            };
            let mut floats = Vec::new();
            let mut markers = Vec::new();
            let mut lines = Vec::new();
            for index in 0..result.page_count() {
                let page = result.page(index).unwrap();
                floats.extend(
                    page.fragments()
                        .filter(|fragment| fragment.kind() == FragmentKind::Box)
                        .map(|fragment| fragment.rect())
                        .filter(|rect| rect.width == 40.0 && rect.height == 50.0)
                        .map(|rect| (index, rect.x, rect.y, rect.width, rect.height)),
                );
                for run in page.text_runs() {
                    let point = (index, run.origin.0, run.origin.1 - ASCENT);
                    if run.text.trim() == "F" {
                        markers.push(point);
                    }
                    if run.text.contains('X')
                        && !lines
                            .iter()
                            .any(|line: &(u32, f32, f32)| line.0 == point.0 && line.2 == point.2)
                    {
                        lines.push(point);
                    }
                }
            }
            assert_eq!(result.page_count(), 3);
            assert_eq!(floats, [(1, x, 0.0, 40.0, 50.0)]);
            assert_eq!(markers, [(1, x, 5.0)]);
            assert_eq!(lines, [(1, 0.0, 55.0), (2, 0.0, 5.0)]);
        }
    }
}

/// A 20px float with `style` added.
fn float_box(side: &str, height: u32, style: &str) -> String {
    format!("<div style='float:{side};width:20px;height:{height}px;{style}'></div>")
}

#[test]
fn lines_beside_a_float_after_a_block_start_the_next_page_when_they_cross_the_page_end() {
    // The float and the line beside it begin 90px down a page of 100px: the
    // line would cross the page end, so it starts the next page, where the
    // float's width still indents it, and the line below it is clear of the
    // float.
    let left = float_box("left", 20, "");
    assert_eq!(
        lines(&format!("{}{left}z z z", spacer(90))),
        [(1, 20.0, 5.0), (1, 0.0, 35.0)]
    );
    // The float is as tall as two of the lines, so both stay beside it.
    let tall = float_box("left", 50, "");
    assert_eq!(
        lines(&format!("{}{tall}z z z", spacer(90))),
        [(1, 20.0, 5.0), (1, 20.0, 35.0)]
    );
    // A float on the right leaves the lines at the left edge.
    let right = float_box("right", 20, "");
    assert_eq!(
        lines(&format!("{}{right}z z z", spacer(90))),
        [(1, 0.0, 5.0), (1, 0.0, 35.0)]
    );
    // One on each side leaves room for a single word between them.
    assert_eq!(
        lines(&format!("{}{left}{right}z z", spacer(90))),
        [(1, 20.0, 5.0)]
    );
}

#[test]
fn the_margin_of_a_float_does_not_hold_the_lines_beside_it_on_the_page_before() {
    // The float's border box starts exactly at the page end when its top
    // margin is added to the 90px of the block above, so the float is on the
    // next page. The lines beside it still start on the page before, cross its
    // end, and move to the next page like the lines of a float without margins.
    for (style, expected) in [
        ("margin-top:10px", vec![(1, 20.0, 5.0), (1, 0.0, 35.0)]),
        ("margin:10px", vec![(1, 40.0, 5.0), (1, 40.0, 35.0)]),
        ("margin-left:10px", vec![(1, 30.0, 5.0), (1, 0.0, 35.0)]),
        ("margin-bottom:10px", vec![(1, 20.0, 5.0), (1, 0.0, 35.0)]),
    ] {
        let float = float_box("left", 20, style);
        assert_eq!(
            lines(&format!("{}{float}z z z", spacer(90))),
            expected,
            "{style}"
        );
    }
}

#[test]
fn lines_that_fit_beside_a_float_stay_on_the_page() {
    let left = float_box("left", 20, "");
    // Two words on one line, from 60px to 90px.
    assert_eq!(
        lines(&format!("{}{left}z z", spacer(60))),
        [(0, 20.0, 65.0)]
    );
    // Seven words on four lines: one line would stay on the first page, which
    // is fewer than the two lines orphans asks for, so all of them move.
    assert_eq!(
        lines(&format!("{}{left}z z z z z z z", spacer(60))),
        [(1, 20.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]
    );
}

#[test]
fn a_float_after_the_first_word_of_the_line_moves_the_line_like_one_before_it() {
    let left = float_box("left", 20, "");
    assert_eq!(lines(&format!("{}z{left} z", spacer(90))), [(1, 20.0, 5.0)]);
    // The line after a second block, with the float among its words.
    assert_eq!(
        lines(&format!(
            "{}z<div style='height:10px'></div>{left}z z z",
            spacer(50)
        )),
        [(0, 0.0, 55.0), (1, 20.0, 5.0), (1, 0.0, 35.0)]
    );
}

#[test]
fn the_lines_between_two_blocks_beside_a_float_move_and_the_block_after_them_follows() {
    let tall = float_box("left", 50, "");
    assert_eq!(
        lines(&format!(
            "{}{tall}z<div style='height:10px'></div>z",
            spacer(90)
        )),
        [(1, 20.0, 5.0), (1, 20.0, 45.0)]
    );
    assert_eq!(
        lines(&format!(
            "{}{tall}z z z<div style='height:10px'></div>z",
            spacer(90)
        )),
        [(1, 20.0, 5.0), (1, 20.0, 35.0), (1, 0.0, 75.0)]
    );
}

#[test]
fn lines_beside_a_float_in_a_flow_root_move_with_the_block_after_them() {
    let left = float_box("left", 20, "");
    let mut found = lines(&format!(
        "<div style='display:flow-root'>{}{left}z z z</div>w",
        spacer(90)
    ));
    // The text after the flow root is reported first.
    found.sort_by(|a, b| a.2.total_cmp(&b.2));
    assert_eq!(found, [(1, 20.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]);
}

#[test]
fn lines_with_a_gap_that_a_float_leaves_between_them_stay_where_the_layout_put_them() {
    // The line with the break sits above the float's margin, and the text
    // after the clearance comes 80px below it. The lines are not one block of
    // lines to move whole: the text stays where the margin of the float puts
    // it, as in the reference of the print test of a float with a large
    // bottom margin (a block, a block, then the text).
    let float = float_box("left", 20, "margin-bottom:60px");
    assert_eq!(
        lines(&format!("{}{float}<br style='clear:both'>z", spacer(60))),
        [(1, 0.0, 45.0)]
    );
}
