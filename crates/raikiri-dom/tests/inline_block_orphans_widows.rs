//! A block inside an inline box of a paragraph lies on a line of its own in
//! that paragraph, and Chromium counts it as one line for `orphans` and
//! `widows`. A page break before such a block that leaves fewer than
//! `orphans` lines of the paragraph before it on the page moves those lines
//! to the next page with the block; otherwise the lines that `widows` asks
//! for beyond the block go along, as far as `orphans` lets them. A break
//! among the lines after the block counts the block, and the lines before
//! it on the page, among its orphans, and the widows it asks for take the
//! block along too. A block straight in the paragraph is not a line, and
//! the lines before it stay. The widows pull lines even when the next page
//! cannot hold all of them, except where the paragraph starts its page right
//! after a break, below no box of the flow on that page: there the lines go
//! only when they meet the widows.
//! The pages and positions are the ones Chromium prints for the same markup:
//! 100px pages without margins, Ahem at 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index and the top on that page of the first fragment of each of
/// the elements `#t` and `#z`.
fn places(body: &str) -> [(usize, f32); 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut found = [None, None];
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            let slot = match page.dom().attr(fragment.node(), "id") {
                Some("t") => 0,
                Some("z") => 1,
                _ => continue,
            };
            found[slot].get_or_insert((index, fragment.rect().y));
        }
    }
    found.map(|place| place.unwrap_or_else(|| panic!("no fragment for #t or #z in {body}")))
}

/// Lays out every `(body, [#t, #z])` case and reports all that differ.
fn assert_places(cases: &[(String, [(usize, f32); 2])]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(body, expected)| {
            let got = places(body);
            got.iter()
                .zip(expected)
                .any(|(got, expected)| got.0 != expected.0 || (got.1 - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A block `height` px tall that holds `#t`.
fn block(height: u32) -> String {
    format!("<div style=\"height:{height}px\"><span id=t>T</span></div>")
}

/// A block `above` px tall, then a paragraph with the style `style` that
/// holds `content`, then `#z`.
fn paragraph(above: u32, style: &str, content: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
         <div style=\"{style}\">{content}</div><div><span id=z>Z</span></div></div>"
    )
}

/// `lines` lines of `A`, then a block `height` px tall inside the same span.
fn in_span(lines: usize, height: u32) -> String {
    format!(
        "<span>{}{}</span>",
        vec!["A"; lines].join("<br>"),
        block(height)
    )
}

#[test]
fn too_few_orphans_before_the_block_move_with_it() {
    assert_places(&[
        (paragraph(50, "", &in_span(1, 40)), [(1, 35.0), (1, 75.0)]),
        (paragraph(50, "", &in_span(1, 120)), [(1, 35.0), (2, 55.0)]),
        // In a span of their own, the lines still share the paragraph.
        (
            paragraph(50, "", &format!("<span>A</span><span>{}</span>", block(40))),
            [(1, 35.0), (1, 75.0)],
        ),
        // The paragraph goes whole, with its padding and border above the
        // line.
        (
            paragraph(50, "border-top:5px solid;padding-top:5px", &in_span(1, 40)),
            [(1, 45.0), (2, 5.0)],
        ),
        // `orphans: 1` leaves the line on its page.
        (
            paragraph(50, "orphans:1", &in_span(1, 40)),
            [(1, 5.0), (1, 45.0)],
        ),
        // A block straight in the paragraph is no line.
        (
            paragraph(50, "", &format!("A{}", block(40))),
            [(1, 5.0), (1, 45.0)],
        ),
    ]);
}

#[test]
fn a_monolithic_block_takes_the_lines_too() {
    let img = "<span>A<div><img id=t style=\"display:block;width:10px;height:40px\"></div></span>";
    assert_places(&[(paragraph(50, "", img), [(1, 30.0), (1, 75.0)])]);
}

#[test]
fn widows_take_a_line_before_the_block() {
    assert_places(&[
        (paragraph(10, "", &in_span(3, 20)), [(1, 35.0), (1, 55.0)]),
        (paragraph(10, "", &in_span(3, 80)), [(1, 35.0), (2, 15.0)]),
        (
            paragraph(10, "widows:3", &in_span(3, 20)),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(10, "widows:1", &in_span(3, 20)),
            [(1, 5.0), (1, 25.0)],
        ),
        // Orphans come first.
        (
            paragraph(10, "orphans:3", &in_span(3, 20)),
            [(1, 5.0), (1, 25.0)],
        ),
        // Lines after the block do not count.
        (
            paragraph(10, "", &format!("<span>A<br>A<br>A{}C</span>", block(20))),
            [(1, 35.0), (2, 5.0)],
        ),
        // The paragraph moved whole for its orphans still keeps its widows.
        (paragraph(50, "", &in_span(3, 20)), [(2, 35.0), (2, 55.0)]),
    ]);
}

#[test]
fn the_block_is_one_of_the_widows_of_a_break_between_the_lines() {
    assert_places(&[
        (
            paragraph(30, "orphans:1", &in_span(3, 20)),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(30, "orphans:1", &in_span(3, 80)),
            [(1, 35.0), (2, 15.0)],
        ),
    ]);
}

#[test]
fn a_block_whose_first_piece_fits_breaks_inside() {
    assert_places(&[
        (paragraph(10, "", &in_span(1, 80)), [(0, 45.0), (1, 25.0)]),
        (paragraph(40, "", &in_span(1, 40)), [(0, 75.0), (1, 15.0)]),
        // A relative offset only paints the block lower: the first piece
        // fits where the flow puts it.
        (
            paragraph(
                40,
                "",
                "<span>A<div style=\"height:40px;position:relative;top:5px\">\
                 <span id=t>T</span></div></span>",
            ),
            [(0, 80.0), (1, 15.0)],
        ),
    ]);
}

/// A block `height` px tall that holds `#t` and avoids a break inside it.
fn whole_block(height: u32) -> String {
    format!("<div style=\"height:{height}px;break-inside:avoid\"><span id=t>T</span></div>")
}

#[test]
fn lines_after_a_block_child_break_from_it_for_their_orphans() {
    assert_places(&[
        // The line after the block child goes with the inline block.
        (
            paragraph(
                0,
                "",
                &format!(
                    "<div style=\"height:50px\"></div><span>A{}</span>",
                    whole_block(40)
                ),
            ),
            [(1, 35.0), (1, 75.0)],
        ),
        // A line that starts its page stays.
        (
            paragraph(
                0,
                "",
                &format!(
                    "<div style=\"height:100px\"></div><span>A{}</span>",
                    whole_block(80)
                ),
            ),
            [(2, 5.0), (3, 5.0)],
        ),
        // Enough orphans stay, and no line is left to give to the widows.
        (
            paragraph(
                0,
                "",
                &format!(
                    "<div style=\"height:20px\"></div><span>A<br>A{}</span>",
                    whole_block(80)
                ),
            ),
            [(1, 5.0), (2, 5.0)],
        ),
    ]);
}

#[test]
fn lines_that_start_their_page_stay() {
    assert_places(&[
        // The paragraph starts the page.
        (
            paragraph(0, "", &format!("<span>A{}</span>", whole_block(80))),
            [(1, 5.0), (2, 5.0)],
        ),
        // The line went on from the page before.
        (
            paragraph(
                0,
                "",
                &format!("<span>A<br>A<br>A<br>A{}</span>", whole_block(80)),
            ),
            [(2, 5.0), (3, 5.0)],
        ),
    ]);
}

/// `before` lines of `A`, a block `height` px tall, then `after` lines of
/// `C`, all inside one span.
fn around(before: usize, height: u32, after: usize) -> String {
    format!(
        "<span>{}{}{}</span>",
        vec!["A"; before].join("<br>"),
        block(height),
        vec!["C"; after].join("<br>")
    )
}

#[test]
fn widows_after_the_block_take_the_block() {
    assert_places(&[
        (paragraph(10, "", &around(2, 20, 1)), [(1, 5.0), (1, 55.0)]),
        // And a line after it, as far as the orphans let them.
        (
            paragraph(10, "widows:3", &around(2, 20, 2)),
            [(1, 5.0), (2, 5.0)],
        ),
        // Lines that went on from the page before count from the top of
        // their page.
        (
            paragraph(50, "orphans:1", &around(2, 50, 1)),
            [(2, 5.0), (3, 5.0)],
        ),
    ]);
}

#[test]
fn the_block_and_the_lines_before_it_are_orphans_of_the_lines_after_it() {
    assert_places(&[
        // Three lines before the break: the last line breaks alone.
        (
            paragraph(10, "widows:1", &around(1, 20, 2)),
            [(0, 45.0), (1, 35.0)],
        ),
        // The block goes on from the page before: it is one line there.
        (
            paragraph(10, "orphans:3", &around(2, 80, 2)),
            [(0, 75.0), (2, 35.0)],
        ),
        // Too few orphans take the paragraph whole.
        (
            paragraph(30, "orphans:3", &around(1, 20, 1)),
            [(1, 35.0), (2, 5.0)],
        ),
        // Also when the block ends the page.
        (
            paragraph(50, "orphans:3", &around(1, 20, 1)),
            [(1, 35.0), (2, 5.0)],
        ),
    ]);
}

#[test]
fn widows_take_the_block_only_when_they_fit_a_page() {
    assert_places(&[
        // The block and the line before it go with the line after it.
        (
            paragraph(0, "orphans:1;widows:3", &around(2, 20, 1)),
            [(1, 35.0), (2, 5.0)],
        ),
        // Four widows do not fit the page: none move.
        (
            paragraph(100, "orphans:1;widows:4", &around(1, 20, 2)),
            [(1, 35.0), (2, 35.0)],
        ),
        // On the page the paragraph opens on, the lines after the block
        // still give what widows they can.
        (
            paragraph(0, "orphans:2;widows:4", &around(1, 20, 2)),
            [(0, 35.0), (1, 65.0)],
        ),
        // Not on a later page.
        (
            paragraph(30, "orphans:2;widows:4", &around(2, 20, 3)),
            [(1, 5.0), (2, 35.0)],
        ),
    ]);
}

/// `first` lines of `A`, a block `above` px tall, `middle` lines of `B`, a
/// block `below` px tall that holds `#t`, then `last` lines of `C`, all
/// inside one span.
fn two_blocks(first: usize, above: u32, middle: usize, below: u32, last: usize) -> String {
    format!(
        "<span>{}<div style=\"height:{above}px\"><span>Q</span></div>{}{}{}</span>",
        vec!["A"; first].join("<br>"),
        vec!["B"; middle].join("<br>"),
        block(below),
        vec!["C"; last].join("<br>")
    )
}

#[test]
fn every_block_inside_an_inline_box_is_a_line_of_the_paragraph() {
    assert_places(&[
        // The first block is an orphan of the break among the lines before
        // the second.
        (
            paragraph(0, "orphans:1;widows:2", &two_blocks(1, 10, 2, 10, 1)),
            [(1, 35.0), (1, 75.0)],
        ),
        // The second block and the line after it are widows of a break
        // after the first block.
        (
            paragraph(0, "orphans:2;widows:2", &two_blocks(2, 20, 1, 10, 1)),
            [(1, 35.0), (1, 75.0)],
        ),
        // Too few orphans after the first block leave the break where it
        // falls.
        (
            paragraph(0, "orphans:4;widows:1", &two_blocks(1, 20, 2, 20, 2)),
            [(1, 35.0), (2, 35.0)],
        ),
        (
            paragraph(0, "orphans:3;widows:1", &two_blocks(1, 10, 1, 10, 1)),
            [(0, 75.0), (1, 35.0)],
        ),
        // A paragraph that the first block opens goes whole for them.
        (
            paragraph(40, "orphans:3;widows:1", &two_blocks(0, 20, 1, 20, 1)),
            [(1, 55.0), (2, 5.0)],
        ),
        (
            paragraph(40, "orphans:3;widows:1", &two_blocks(0, 10, 2, 10, 1)),
            [(1, 75.0), (2, 35.0)],
        ),
    ]);
}

#[test]
fn every_block_and_line_of_the_paragraph_on_the_page_is_an_orphan() {
    assert_places(&[
        // The line and the block before the run count with it.
        (
            paragraph(0, "orphans:1;widows:2", &two_blocks(1, 10, 1, 10, 1)),
            [(1, 5.0), (1, 45.0)],
        ),
        (
            paragraph(0, "orphans:1;widows:3", &two_blocks(1, 10, 1, 10, 1)),
            [(1, 35.0), (1, 75.0)],
        ),
        // Blocks next to each other are one line together.
        (
            paragraph(
                0,
                "orphans:2;widows:3",
                &format!(
                    "<span>A<div style=\"height:10px\"><span>Q</span></div>{}C<br>C</span>",
                    block(10)
                ),
            ),
            [(0, 45.0), (1, 65.0)],
        ),
        // The page still breaks between them.
        (
            paragraph(
                20,
                "orphans:1;widows:2",
                &format!(
                    "<span>A<br>A<div style=\"height:10px\"><span>Q</span></div>{}C</span>",
                    block(20)
                ),
            ),
            [(1, 5.0), (1, 55.0)],
        ),
        (
            paragraph(
                50,
                "orphans:1;widows:2",
                &format!(
                    "<span>A<div style=\"height:40px\"><span>Q</span></div>{}C<br>C</span>",
                    block(10)
                ),
            ),
            [(1, 45.0), (2, 65.0)],
        ),
    ]);
}

#[test]
fn a_block_straight_in_the_paragraph_ends_the_lines_around_it() {
    assert_places(&[
        (
            paragraph(
                0,
                "orphans:1;widows:2",
                &format!("X<br>X<div>P</div>A<span>{}C<br>C</span>", block(20)),
            ),
            [(1, 35.0), (2, 65.0)],
        ),
        (
            paragraph(
                30,
                "orphans:1;widows:2",
                &format!("<span>A{}C</span><div>P</div>X", block(20)),
            ),
            [(1, 5.0), (2, 35.0)],
        ),
    ]);
}

#[test]
fn a_block_that_opens_the_paragraph_opens_it_on_its_page() {
    let content = |height| format!("<span>{}C<br>C<br>C</span>", block(height));
    assert_places(&[
        (
            paragraph(0, "orphans:1;widows:3", &content(40)),
            [(0, 5.0), (2, 5.0)],
        ),
        // A block that goes on from the page before keeps the line after
        // it on its page.
        (
            paragraph(70, "orphans:1;widows:3", &content(80)),
            [(0, 75.0), (2, 65.0)],
        ),
    ]);
}

#[test]
fn the_padding_below_breaks_among_the_lines_after_the_block() {
    assert_places(&[(
        paragraph(
            0,
            "orphans:1;widows:2;padding-bottom:30px",
            &around(1, 20, 2),
        ),
        [(0, 35.0), (2, 5.0)],
    )]);
}

/// In a 20px wide block: `before`, a block `above` px tall, then a paragraph
/// with `orphans` and `widows` whose span holds `lines` lines of `A`, a block
/// `height` px tall that holds `#t`, and `after` lines of `C`, then `#z`.
fn spanned(
    before: &str,
    above: u32,
    [lines, height, after]: [u32; 3],
    [orphans, widows]: [u32; 2],
) -> String {
    let join = |letter: &str, count: u32| vec![letter; count as usize].join("<br>");
    format!(
        "<div style=\"width:20px\">{before}<div style=\"height:{above}px\"></div>\
         <div style=\"orphans:{orphans};widows:{widows}\"><span>{}{}{}</span></div>\
         <div><span id=z>Z</span></div></div>",
        join("A", lines),
        block(height),
        join("C", after),
    )
}

#[test]
fn widows_the_next_page_cannot_hold_still_pull_on_the_page_the_paragraph_opens() {
    assert_places(&[
        (spanned("", 0, [1, 20, 2], [1, 4]), [(1, 5.0), (2, 5.0)]),
        (spanned("", 10, [2, 20, 1], [2, 4]), [(1, 5.0), (1, 55.0)]),
        (spanned("", 10, [1, 40, 3], [1, 4]), [(1, 5.0), (2, 35.0)]),
        // A box of no height before the paragraph at the top of the page.
        (
            spanned(
                "<div style=\"break-after:page\">X</div>",
                0,
                [1, 20, 2],
                [1, 4],
            ),
            [(2, 5.0), (3, 5.0)],
        ),
        // A box that runs on from the page before.
        (spanned("", 110, [1, 20, 2], [1, 4]), [(2, 5.0), (3, 5.0)]),
    ]);
}

#[test]
fn a_paragraph_that_starts_its_page_after_a_break_pulls_only_widows_it_meets() {
    let forced = "<div style=\"break-after:page\">X</div>";
    assert_places(&[
        // Lines through the block, then among the lines after it.
        (spanned("", 100, [1, 20, 2], [1, 4]), [(1, 35.0), (2, 35.0)]),
        (
            format!(
                "<div style=\"width:20px\">{forced}<div style=\"orphans:1;widows:4\"><span>A{}C<br>C</span></div>\
                 <div><span id=z>Z</span></div></div>",
                block(20)
            ),
            [(1, 35.0), (2, 35.0)],
        ),
        (spanned("", 80, [1, 20, 2], [1, 4]), [(1, 35.0), (2, 35.0)]),
        (spanned("", 30, [2, 20, 2], [1, 4]), [(1, 35.0), (2, 35.0)]),
        (spanned("", 60, [1, 20, 2], [2, 4]), [(1, 35.0), (2, 35.0)]),
        (spanned("", 60, [1, 20, 2], [2, 3]), [(1, 35.0), (2, 65.0)]),
        (spanned("", 60, [1, 20, 2], [2, 2]), [(1, 35.0), (2, 65.0)]),
        // The block crosses the page end.
        (spanned("", 80, [3, 20, 1], [2, 4]), [(2, 5.0), (2, 55.0)]),
        (spanned("", 80, [3, 20, 1], [2, 3]), [(2, 35.0), (3, 5.0)]),
        (spanned("", 50, [3, 40, 2], [2, 4]), [(2, 5.0), (3, 5.0)]),
    ]);
}

#[test]
fn edges_and_blocks_above_the_first_line_decide_whether_the_paragraph_starts_its_page() {
    let forced = "<div style=\"break-after:page\">X</div>";
    let z = "<div><span id=z>Z</span></div>";
    let lines = format!("A{}C<br>C", block(20));
    assert_places(&[
        // The paragraph's own top padding comes after the break.
        (
            format!(
                "<div style=\"width:20px\">{forced}<div style=\"padding-top:10px;orphans:1;\
                 widows:4\"><span>{lines}</span></div>{z}</div>"
            ),
            [(1, 45.0), (2, 35.0)],
        ),
        // The top padding of a box around it comes before the paragraph.
        (
            format!(
                "<div style=\"width:20px\">{forced}<div style=\"padding-top:10px\">\
                 <div style=\"orphans:1;widows:4\"><span>{lines}</span></div></div>{z}</div>"
            ),
            [(2, 5.0), (3, 5.0)],
        ),
        // So does a block of the paragraph before its first line.
        (
            format!(
                "<div style=\"width:20px\">{forced}<div style=\"orphans:1;widows:4\">\
                 <div style=\"height:10px\"></div><span>{lines}</span></div>{z}</div>"
            ),
            [(2, 5.0), (3, 5.0)],
        ),
        (
            format!(
                "<div style=\"width:20px\">{forced}<div style=\"orphans:1;widows:4\"><span>\
                 <div style=\"height:10px\"></div>{lines}</span></div>{z}</div>"
            ),
            [(2, 35.0), (3, 35.0)],
        ),
    ]);
}

#[test]
fn a_paragraph_moved_whole_for_its_orphans_still_breaks_between_its_lines() {
    assert_places(&[
        (spanned("", 10, [0, 20, 3], [4, 1]), [(1, 5.0), (2, 35.0)]),
        (spanned("", 40, [0, 20, 3], [3, 2]), [(1, 5.0), (2, 35.0)]),
    ]);
}

#[test]
fn a_block_that_opens_the_paragraph_is_one_line_for_the_orphans_of_a_break_after_it() {
    assert_places(&[
        // The block ends the page and the lines after it start the next.
        (spanned("", 80, [0, 20, 1], [2, 1]), [(1, 5.0), (1, 55.0)]),
        (spanned("", 60, [0, 40, 2], [3, 1]), [(1, 5.0), (2, 5.0)]),
    ]);
}

#[test]
fn an_inline_box_that_holds_only_blocks_stacks_them_in_block_flow() {
    let z = "<div><span id=z>Z</span></div>";
    assert_places(&[
        // The line that starts the block crosses the page end: the block
        // moves whole, as one outside the inline box does.
        (
            paragraph(80, "", &format!("<span>{}</span>", block(40))),
            [(1, 5.0), (1, 45.0)],
        ),
        // The blocks after it in the inline box follow it.
        (
            format!(
                "<div style=\"width:20px\"><div style=\"height:80px\"></div><div><span>{}{z}</span></div></div>",
                block(40)
            ),
            [(1, 5.0), (1, 45.0)],
        ),
        (
            format!(
                "<div style=\"width:20px\"><div style=\"height:50px\"></div><div><span>\
                 <div style=\"height:10px\"></div>{}{z}</span></div></div>",
                block(40)
            ),
            [(0, 65.0), (1, 5.0)],
        ),
    ]);
}
