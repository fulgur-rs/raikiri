//! A paragraph whose first line the floats before it put below themselves,
//! when the page ends under that line. The page breaks before the line and the
//! floats stay behind, so the line starts the next page. The positions are the
//! ones Chromium prints for the same documents, on 100px pages with Ahem
//! lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em and a 20px glyph sits 5px below the top of its 30px
/// line, so the baseline is 21px below the top of the line.
const BASELINE_BELOW_LINE_TOP: f32 = 21.0;

/// The floats and spacers pairs whose paragraph line crosses the page end:
/// the line starts at the bottom of the float, `spacer + float`, which is
/// 70 to 100px down a 100px page.
const CROSSING: [(u32, u32); 4] = [(60, 20), (70, 10), (50, 30), (80, 10)];

/// Resources with the Ahem font.
fn ahem() -> RenderResources<'static> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    RenderResources::new().fonts(fonts)
}

/// The page, left edge and top of every line of text in `body`.
fn lines(body: &str) -> Vec<(u32, f32, f32)> {
    let resources = ahem();
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
    for index in 0..6 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            let top = run.origin.1 - BASELINE_BELOW_LINE_TOP;
            // The words of a line are runs of their own in some paragraphs.
            match lines
                .iter_mut()
                .find(|line| line.0 == index && line.2 == top)
            {
                Some(line) => line.1 = line.1.min(run.origin.0),
                None => lines.push((index, run.origin.0, top)),
            }
        }
    }
    lines
}

/// A block `spacer`px tall, a left float `float`px tall and 20px wide, and
/// `rest`.
fn after_left_float(spacer: u32, float: u32, rest: &str) -> String {
    format!(
        "<div style='height:{spacer}px'></div>\
         <div style='float:left;width:20px;height:{float}px'></div>{rest}"
    )
}

/// A paragraph as wide as the float, so that no line fits beside it.
fn narrow(style: &str, text: &str) -> String {
    format!("<div style='width:20px;{style}'>{text}</div>")
}

/// Checks `expected` for the document `rest` follows the float in, at each of
/// the spacer and float heights whose first line crosses the page end.
fn assert_after_every_crossing_float(rest: &str, expected: &[(u32, f32, f32)]) {
    for (spacer, float) in CROSSING {
        assert_eq!(
            lines(&after_left_float(spacer, float, rest)),
            expected,
            "{rest} after a {float}px float {spacer}px down"
        );
    }
}

#[test]
fn the_line_below_a_float_at_the_page_end_starts_the_next_page_or_follows_the_float() {
    // (spacer, float, page, top): the line goes below the float at
    // `spacer + float`. It stays when it ends on the page, it starts the next
    // one when it crosses the page end, and it starts below what remains of
    // the float when the float itself crosses the page end.
    let expected = [
        (40, 10, 0, 50.0),
        (40, 20, 0, 60.0),
        (40, 30, 0, 70.0),
        (40, 40, 1, 0.0),
        (50, 10, 0, 60.0),
        (50, 20, 0, 70.0),
        (50, 30, 1, 0.0),
        (50, 40, 1, 0.0),
        (60, 10, 0, 70.0),
        (60, 20, 1, 0.0),
        (60, 30, 1, 0.0),
        (60, 40, 1, 0.0),
        (70, 10, 1, 0.0),
        (70, 20, 1, 0.0),
        (70, 30, 1, 0.0),
        (70, 40, 1, 10.0),
        (80, 10, 1, 0.0),
        (80, 20, 1, 0.0),
        (80, 30, 1, 10.0),
        (80, 40, 1, 20.0),
        (90, 10, 1, 0.0),
        (90, 20, 1, 10.0),
        (90, 30, 1, 20.0),
        (90, 40, 1, 30.0),
    ];
    for (spacer, float, page, top) in expected {
        assert_eq!(
            lines(&after_left_float(spacer, float, &narrow("", "Y"))),
            [(page, 0.0, top)],
            "a line after a {float}px float {spacer}px down"
        );
    }
}

#[test]
fn a_line_below_a_right_float_starts_the_next_page_too() {
    let body = format!(
        "<div style='height:60px'></div>\
         <div style='float:right;width:20px;height:20px'></div>{}",
        narrow("margin-left:80px", "Y")
    );
    assert_eq!(lines(&body), [(1, 80.0, 0.0)]);
}

#[test]
fn a_line_below_two_floats_starts_the_next_page() {
    let body = format!(
        "<div style='height:60px'></div>\
         <div style='float:left;width:20px;height:20px'></div>\
         <div style='float:left;width:20px;height:20px'></div>{}",
        narrow("", "Y")
    );
    assert_eq!(lines(&body), [(1, 0.0, 0.0)]);
}

#[test]
fn every_line_of_the_paragraph_moves_with_the_first_one() {
    let word = |count: usize| vec!["Y"; count].join(" ");
    // Two and three lines: the page ends under the first one, and the orphans
    // and widows leave all of them together.
    assert_after_every_crossing_float(&narrow("", &word(2)), &[(1, 0.0, 0.0), (1, 0.0, 30.0)]);
    assert_after_every_crossing_float(
        &narrow("", &word(3)),
        &[(1, 0.0, 0.0), (1, 0.0, 30.0), (1, 0.0, 60.0)],
    );
    // A forced break between the lines.
    assert_after_every_crossing_float(&narrow("", "Y<br>Y"), &[(1, 0.0, 0.0), (1, 0.0, 30.0)]);
    // A paragraph that is longer than a page goes on to the next one.
    assert_after_every_crossing_float(
        &narrow("", &word(6)),
        &[
            (1, 0.0, 0.0),
            (1, 0.0, 30.0),
            (1, 0.0, 60.0),
            (2, 0.0, 0.0),
            (2, 0.0, 30.0),
            (2, 0.0, 60.0),
        ],
    );
}

#[test]
fn the_orphans_and_widows_of_the_moved_paragraph_still_decide_where_it_breaks_next() {
    let four = "Y Y Y Y";
    assert_after_every_crossing_float(
        &narrow("orphans:3;widows:1", four),
        &[(1, 0.0, 0.0), (1, 0.0, 30.0), (1, 0.0, 60.0), (2, 0.0, 0.0)],
    );
    assert_after_every_crossing_float(
        &narrow("orphans:1;widows:3", four),
        &[(1, 0.0, 0.0), (2, 0.0, 0.0), (2, 0.0, 30.0), (2, 0.0, 60.0)],
    );
    assert_after_every_crossing_float(&narrow("orphans:1;widows:1", "Y"), &[(1, 0.0, 0.0)]);
}

#[test]
fn a_paragraph_of_several_texts_moves_as_one() {
    // The lines of an inline element and of text after it follow the first.
    assert_after_every_crossing_float(&narrow("", "<span>Y</span>"), &[(1, 0.0, 0.0)]);
    assert_after_every_crossing_float(&narrow("", "Y<u>Y</u>Y"), &[(1, 0.0, 0.0)]);
    assert_after_every_crossing_float(&narrow("text-indent:10px", "Y"), &[(1, 10.0, 0.0)]);
}

#[test]
fn the_box_around_the_line_keeps_what_it_paints_where_the_page_breaks() {
    for style in [
        // A margin above the box is dropped at the page break.
        "margin-top:10px",
        // The box is cut between its top and its line: nothing is lost.
        "background:#ccc",
        "border-top:1px solid #000",
        "padding-bottom:10px",
        // A set height does not hold the line back.
        "height:60px",
    ] {
        assert_after_every_crossing_float(&narrow(style, "Y"), &[(1, 0.0, 0.0)]);
    }
}

#[test]
fn padding_above_the_line_stays_on_the_page_the_float_leaves_when_the_float_left_room_above_it() {
    // The float is taller than the padding, so there is room between the
    // padding and the line: the page breaks there, and the padding stays.
    for (spacer, float) in [(60, 20), (50, 30)] {
        let body = after_left_float(spacer, float, &narrow("padding-top:10px", "Y"));
        assert_eq!(lines(&body), [(1, 0.0, 0.0)], "{float}px float at {spacer}");
    }
    // The padding is as tall as the float: the line starts right after the
    // padding, with nothing to break at, and the box moves whole.
    for (spacer, float) in [(70, 10), (80, 10)] {
        let body = after_left_float(spacer, float, &narrow("padding-top:10px", "Y"));
        assert_eq!(
            lines(&body),
            [(1, 0.0, 10.0)],
            "{float}px float at {spacer}"
        );
    }
}

#[test]
fn a_line_in_a_wrapper_moves_without_the_float_height_too() {
    for wrapper in [
        "",
        "padding-top:10px",
        "border-top:1px solid #000",
        "background:#ccc",
    ] {
        let rest = format!("<div style='{wrapper}'>{}</div>", narrow("", "Y"));
        for (spacer, float) in [(60, 20), (50, 30)] {
            assert_eq!(
                lines(&after_left_float(spacer, float, &rest)),
                [(1, 0.0, 0.0)],
                "{wrapper:?} around a line after a {float}px float {spacer}px down"
            );
        }
    }
    // The float sits inside the wrapper.
    let inside = format!("<div>{}</div>", after_left_float(60, 20, &narrow("", "Y")));
    assert_eq!(lines(&inside), [(1, 0.0, 0.0)]);
}

#[test]
fn a_relative_offset_keeps_moving_the_line_from_where_the_flow_puts_it() {
    assert_after_every_crossing_float(&narrow("position:relative;top:5px", "Y"), &[(1, 0.0, 5.0)]);
    let wrapped = format!(
        "<div style='position:relative;top:5px'>{}</div>",
        after_left_float(60, 20, &narrow("", "Y"))
    );
    assert_eq!(lines(&wrapped), [(1, 0.0, 5.0)]);
}

#[test]
fn what_follows_the_paragraph_follows_the_line_that_moved() {
    let next = format!("{}{}", narrow("", "Y"), narrow("", "Z"));
    assert_after_every_crossing_float(&next, &[(1, 0.0, 0.0), (1, 0.0, 30.0)]);
    // The margin below the paragraph keeps its size.
    let spaced = format!("{}{}", narrow("margin-bottom:20px", "Y"), narrow("", "Z"));
    assert_after_every_crossing_float(&spaced, &[(1, 0.0, 0.0), (1, 0.0, 50.0)]);
    // A box that clears the floats, and a table.
    let cleared = format!("{}{}", narrow("", "Y"), narrow("clear:both", "Z"));
    assert_after_every_crossing_float(&cleared, &[(1, 0.0, 0.0), (1, 0.0, 30.0)]);
    let table = format!(
        "{}<table style='border-collapse:collapse'><tr><td style='padding:0'>Z</td></tr></table>",
        narrow("", "Y")
    );
    assert_after_every_crossing_float(&table, &[(1, 0.0, 0.0), (1, 0.0, 30.0)]);
    // A float after the paragraph lands below the line that moved.
    let later_float = format!(
        "{}<div style='float:left;width:20px;height:10px'></div>{}",
        narrow("", "Y"),
        narrow("", "Z")
    );
    assert_after_every_crossing_float(&later_float, &[(1, 0.0, 0.0), (1, 0.0, 40.0)]);
}

#[test]
fn a_box_positioned_in_the_paragraph_does_not_hold_the_line_back() {
    let rest = narrow(
        "position:relative",
        "Y<div style='position:absolute;top:0;left:50px;width:10px;height:10px'></div>",
    );
    assert_after_every_crossing_float(&rest, &[(1, 0.0, 0.0)]);
}
