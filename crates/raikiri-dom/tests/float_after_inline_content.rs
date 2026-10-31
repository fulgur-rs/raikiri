//! A float met after content on a line sits beside that content only when
//! the room the content leaves on the line holds it; otherwise the float goes
//! below the line, and the content stays where it is (CSS 2.1 9.5.1 rules 6
//! and 8).
//!
//! The geometry below is what Chromium lays out for the same documents, with
//! Ahem at 10px (every glyph is a 10px square).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A `(x, y, width, height)` rectangle.
type Rect = (f32, f32, f32, f32);

/// The laid-out page of `body`: the rectangle of each element with an `id`,
/// and the position of the top left of each text run.
struct Laid {
    rects: Vec<(String, Rect)>,
    runs: Vec<(String, f32, f32)>,
}

fn lay_out(body: &str) -> Laid {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{ size: 400px 400px; margin: 0 }}\
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let page = result.page(0).expect("page");
    let dom = page.dom();
    let rects = page
        .fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect();
    let runs = page
        .text_runs()
        .iter()
        .map(|run| {
            (
                run.text.trim().to_owned(),
                run.origin.0,
                run.origin.1 - run.ascent,
            )
        })
        .collect();
    Laid { rects, runs }
}

impl Laid {
    fn rect(&self, id: &str) -> Rect {
        self.rects
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("#{id} has no fragment"))
            .1
    }

    /// The top left of the text run with this text.
    fn run(&self, text: &str) -> (f32, f32) {
        let run = self
            .runs
            .iter()
            .find(|(found, ..)| found == text)
            .unwrap_or_else(|| panic!("no text run {text:?}"));
        (run.1, run.2)
    }
}

/// A 60px wide paragraph with the given content and style.
fn paragraph(style: &str, content: &str) -> String {
    format!("<div id=o style='width:60px;{style}'>{content}</div>")
}

/// A left float of the given width, holding the given text.
fn float(id: &str, side: &str, width: u32, text: &str) -> String {
    format!("<div id={id} style='float:{side};width:{width}px'>{text}</div>")
}

#[test]
fn a_float_that_does_not_fit_beside_the_text_goes_below_the_line() {
    let laid = lay_out(&paragraph(
        "",
        &format!("x{}", float("i", "left", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
    assert_eq!(laid.run("x"), (0.0, 0.0));
}

#[test]
fn a_float_wider_than_the_paragraph_goes_below_the_text_too() {
    let laid = lay_out(&paragraph(
        "",
        &format!("x{}", float("i", "left", 90, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 90.0, 10.0));
}

#[test]
fn a_float_that_fits_beside_the_text_stays_beside_it() {
    let laid = lay_out(&paragraph(
        "",
        &format!("x{}", float("i", "left", 40, "AAAA")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 0.0, 40.0, 10.0));
    assert_eq!(laid.run("x"), (40.0, 0.0));
}

#[test]
fn a_right_float_that_does_not_fit_goes_below_the_line() {
    let laid = lay_out(&paragraph(
        "",
        &format!("x{}", float("i", "right", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (5.0, 10.0, 55.0, 20.0));
    assert_eq!(laid.run("x"), (0.0, 0.0));
}

#[test]
fn the_text_after_the_float_stays_on_the_line_of_the_text_before_it() {
    let laid = lay_out(&paragraph(
        "",
        &format!("x{}y", float("i", "left", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
    assert_eq!(laid.run("x"), (0.0, 0.0));
}

#[test]
fn the_float_goes_below_the_line_it_is_met_on() {
    // "xxxx yyyy" wraps; the float follows the second line.
    let laid = lay_out(&paragraph(
        "",
        &format!("xxxx yyyy{}", float("i", "left", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 20.0));
    assert_eq!(laid.rect("i"), (0.0, 20.0, 55.0, 20.0));
    assert_eq!(laid.run("yyyy"), (0.0, 10.0));
}

#[test]
fn a_float_after_a_word_wider_than_the_line_goes_below_it() {
    let laid = lay_out(&paragraph(
        "",
        &format!("xxxxxxxxxxxx{}", float("i", "left", 25, "AA")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 25.0, 10.0));
}

#[test]
fn the_room_left_by_an_earlier_float_is_what_the_text_and_the_float_share() {
    // The first float leaves 40px; the text takes 10px of it, and the second
    // float is 35px wide.
    let content = format!(
        "{}x{}",
        float("p", "left", 20, "AA"),
        float("i", "left", 35, "BBB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("p"), (0.0, 0.0, 20.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 35.0, 10.0));
    assert_eq!(laid.run("x"), (20.0, 0.0));
}

#[test]
fn a_float_beside_the_text_lets_the_next_float_wait_for_the_next_line() {
    let content = format!(
        "x{}{}",
        float("i", "left", 20, "AA"),
        float("j", "left", 50, "BBBBB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 0.0, 20.0, 10.0));
    assert_eq!(laid.rect("j"), (0.0, 10.0, 50.0, 10.0));
}

#[test]
fn the_text_indent_counts_as_content_before_the_float() {
    // The indent and "x" take 20px of the 60px line.
    let wide = lay_out(&paragraph(
        "text-indent: 10px",
        &format!("x{}", float("i", "left", 45, "AAAA")),
    ));
    assert_eq!(wide.rect("i"), (0.0, 10.0, 45.0, 10.0));
    let narrow = lay_out(&paragraph(
        "text-indent: 10px",
        &format!("x{}", float("i", "left", 40, "AAAA")),
    ));
    assert_eq!(narrow.rect("i"), (0.0, 0.0, 40.0, 10.0));
}

#[test]
fn the_start_edge_of_an_inline_box_counts_as_content_before_the_float() {
    let content = format!(
        "<span style='padding-left:10px'>x{}</span>",
        float("i", "left", 55, "AAAA BBBB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
}

#[test]
fn a_float_after_text_in_a_right_to_left_paragraph_goes_below_the_line() {
    let laid = lay_out(&paragraph(
        "direction: rtl",
        &format!("x{}", float("i", "left", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
}

#[test]
fn a_float_that_starts_its_line_still_goes_to_the_top_of_the_line() {
    // After a line break nothing of the line comes before the float, so the
    // text after it wraps beside it instead.
    let content = format!("x<br>{}y", float("i", "left", 55, "AAAA BBBB"));
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
    assert_eq!(laid.run("x"), (0.0, 0.0));
}

#[test]
fn a_float_that_starts_the_paragraph_pushes_the_text_below_it() {
    let laid = lay_out(&paragraph(
        "",
        &format!("{}x", float("i", "left", 55, "AAAA BBBB")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 30.0));
    assert_eq!(laid.rect("i"), (0.0, 0.0, 55.0, 20.0));
    assert_eq!(laid.run("x"), (0.0, 20.0));
}

#[test]
fn a_float_below_the_line_keeps_the_clearance_of_the_block_after_it() {
    let content = format!(
        "x{}<div id=c style='clear:both'>z</div>",
        float("i", "left", 55, "AAAA BBBB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 40.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
    assert_eq!(laid.rect("c"), (0.0, 30.0, 60.0, 10.0));
}

#[test]
fn a_float_with_clearance_after_text_goes_below_the_line() {
    let laid = lay_out(&paragraph(
        "",
        "x<div id=i style='float:left;clear:left;width:55px'>AAAA BBBB</div>",
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 55.0, 20.0));
}

#[test]
fn two_right_floats_after_text_stack_below_the_line_when_they_do_not_fit() {
    let content = format!(
        "x{}{}",
        float("i", "right", 30, "AA"),
        float("j", "right", 30, "BB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (30.0, 0.0, 30.0, 10.0));
    assert_eq!(laid.rect("j"), (30.0, 10.0, 30.0, 10.0));
}

#[test]
fn a_float_met_after_one_that_waits_stacks_below_it() {
    let content = format!(
        "xx{}{}",
        float("i", "left", 50, "AAAAA"),
        float("j", "left", 30, "BBB")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 50.0, 10.0));
    assert_eq!(laid.rect("j"), (0.0, 20.0, 30.0, 10.0));
}

#[test]
fn floats_that_wait_for_the_next_line_keep_their_source_order() {
    // Neither float fits beside the 20px of text with the other: `j` is taken
    // back first, then `i`, and they go below the line side by side.
    let content = format!(
        "xx{}{}",
        float("i", "left", 45, "AAAA"),
        float("j", "left", 10, "B")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 45.0, 10.0));
    assert_eq!(laid.rect("j"), (45.0, 10.0, 10.0, 10.0));
}

#[test]
fn many_floats_after_a_word_wider_than_the_line_all_go_below_it() {
    let content = format!(
        "xxxxxxxxxxxx{}{}{}{}{}",
        float("a", "left", 10, "A"),
        float("b", "left", 10, "B"),
        float("c", "left", 10, "C"),
        float("d", "left", 10, "D"),
        float("e", "left", 10, "E")
    );
    let laid = lay_out(&paragraph("", &content));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    for (id, x) in [
        ("a", 0.0),
        ("b", 10.0),
        ("c", 20.0),
        ("d", 30.0),
        ("e", 40.0),
    ] {
        assert_eq!(laid.rect(id), (x, 10.0, 10.0, 10.0), "#{id}");
    }
}

#[test]
fn a_float_after_text_that_cannot_wrap_goes_below_the_line() {
    let laid = lay_out(&paragraph(
        "white-space: nowrap",
        &format!("xxxxxxxxxxxx{}", float("i", "left", 25, "AA")),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("i"), (0.0, 10.0, 25.0, 10.0));
}

// Vertical paragraphs and the two-glyph repro. The geometry of these cases is
// what Chromium lays out for the same documents too (Ahem 10px, body margin
// 0). The text sits in a `<span id=t>`, so that where it starts and how far it
// runs can be checked beside the float's box. The across-the-line position of
// an inline box in vertical text is left out: it is 3px off Chromium's, with
// or without a float.

impl Laid {
    /// The start and the length of the box with this id along the vertical
    /// inline axis.
    fn span(&self, id: &str) -> (f32, f32) {
        let (_, y, _, height) = self.rect(id);
        (y, height)
    }
}

/// A 100px wide, 60px high vertical paragraph with the given writing mode,
/// holding the text `aa`, a float and then the given tail.
fn vertical(mode: &str, side: &str, float_height: u32, tail: &str) -> String {
    format!(
        "<div id=r style='writing-mode:{mode};width:100px;height:60px'>\
         <span id=t>aa</span>\
         <div id=f style='float:{side};width:20px;height:{float_height}px'></div>{tail}</div>"
    )
}

#[test]
fn a_float_that_does_not_fit_beside_text_in_a_vertical_rl_line_goes_to_the_next_column() {
    let laid = lay_out(&vertical("vertical-rl", "left", 50, ""));
    assert_eq!(laid.rect("r"), (0.0, 0.0, 100.0, 60.0));
    assert_eq!(laid.span("t"), (0.0, 20.0));
    assert_eq!(laid.rect("f"), (70.0, 0.0, 20.0, 50.0));
}

#[test]
fn a_float_that_does_not_fit_beside_text_in_a_vertical_lr_line_goes_to_the_next_column() {
    let laid = lay_out(&vertical("vertical-lr", "left", 50, ""));
    assert_eq!(laid.rect("r"), (0.0, 0.0, 100.0, 60.0));
    assert_eq!(laid.span("t"), (0.0, 20.0));
    assert_eq!(laid.rect("f"), (10.0, 0.0, 20.0, 50.0));
}

#[test]
fn text_after_a_float_in_a_vertical_line_stays_in_the_column_of_the_text_before_it() {
    let laid = lay_out(&vertical("vertical-rl", "left", 50, "<span id=u>cc</span>"));
    assert_eq!(laid.span("t"), (0.0, 20.0));
    assert_eq!(laid.span("u"), (20.0, 20.0));
    assert_eq!(laid.rect("f"), (70.0, 0.0, 20.0, 50.0));
}

#[test]
fn a_float_that_fits_beside_text_in_a_vertical_line_stays_beside_it() {
    // The float takes the start of the column and the text follows it.
    let laid = lay_out(&vertical("vertical-rl", "left", 40, ""));
    assert_eq!(laid.span("t"), (40.0, 20.0));
    assert_eq!(laid.rect("f"), (80.0, 0.0, 20.0, 40.0));
}

#[test]
fn an_end_float_that_does_not_fit_beside_text_in_a_vertical_line_goes_to_the_next_column() {
    let laid = lay_out(&vertical("vertical-rl", "right", 50, ""));
    assert_eq!(laid.span("t"), (0.0, 20.0));
    assert_eq!(laid.rect("f"), (70.0, 10.0, 20.0, 50.0));
}

#[test]
fn a_float_wider_than_the_room_after_two_glyphs_goes_below_their_line() {
    let laid = lay_out(&paragraph(
        "",
        &format!(
            "<span id=t>aa</span>{}",
            float("f", "left", 50, "AAAA BBBB")
        ),
    ));
    assert_eq!(laid.rect("o"), (0.0, 0.0, 60.0, 10.0));
    assert_eq!(laid.rect("t"), (0.0, 0.0, 20.0, 10.0));
    assert_eq!(laid.rect("f"), (0.0, 10.0, 50.0, 20.0));
}
