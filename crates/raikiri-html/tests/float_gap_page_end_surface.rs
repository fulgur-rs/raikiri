//! A paragraph whose first line the floats before it put below themselves,
//! when the page ends under that line. The page breaks before the line and the
//! floats stay behind, so the line starts the next page. The positions are the
//! ones Chromium prints for the same documents, on 100px pages with Ahem
//! lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus,
    RenderResources, layout, parse_html_with_resources,
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

/// Lays out the standard-mode Chromium oracle document.
fn lay_out(body: &str) -> DocumentLayout {
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
    result
}

/// The page, left edge and top of every line of text in `body`.
fn lines(body: &str) -> Vec<(u32, f32, f32)> {
    let result = lay_out(body);
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

#[test]
fn an_avoiding_box_drops_space_from_floats_left_on_the_previous_page() {
    for (style, top) in [
        ("break-inside:avoid", 0.0),
        ("break-inside:avoid;background:red", 0.0),
        ("break-inside:avoid;padding-top:10px", 10.0),
        ("break-inside:avoid;padding-top:10px;background:red", 10.0),
    ] {
        assert_eq!(
            lines(&after_left_float(60, 20, &narrow(style, "Y"))),
            [(1, 0.0, top)],
            "{style}"
        );
    }
}

#[test]
fn content_after_an_avoiding_box_follows_its_recomputed_height() {
    for (style, top, after) in [
        ("break-inside:avoid", 0.0, 30.0),
        (
            "break-inside:avoid;padding-top:10px;background:red",
            10.0,
            40.0,
        ),
    ] {
        let rest = format!("{}{}", narrow(style, "Y"), narrow("", "Z"));
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            [(1, 0.0, top), (1, 0.0, after)],
            "{style}"
        );
    }
}

#[test]
fn an_avoiding_box_retains_only_the_float_space_still_needed_on_its_page() {
    for (spacer, float, page, top) in [
        (60, 10, 0, 70.0),
        (60, 20, 1, 0.0),
        (70, 40, 1, 10.0),
        (80, 40, 1, 20.0),
    ] {
        assert_eq!(
            lines(&after_left_float(
                spacer,
                float,
                &narrow("break-inside:avoid", "Y")
            )),
            [(page, 0.0, top)],
            "spacer {spacer}, float {float}"
        );
    }
}

#[test]
fn an_avoiding_box_keeps_its_own_float_below_its_authored_edges() {
    for (style, top, after) in [
        ("", 10.0, 40.0),
        ("padding-top:10px", 20.0, 50.0),
        ("height:60px", 10.0, 60.0),
        ("min-height:60px", 10.0, 60.0),
        ("max-height:30px", 10.0, 30.0),
    ] {
        let own = "<span style='float:left;width:10px;height:10px'></span>Y";
        let rest = format!(
            "{}{}",
            narrow(&format!("break-inside:avoid;{style}"), own),
            narrow("", "Z")
        );
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            [(1, 0.0, top), (1, 0.0, after)],
            "{style}"
        );
    }
}

#[test]
fn an_avoiding_box_keeps_authored_height_when_its_float_prefix_disappears() {
    for (style, after) in [
        ("height:60px", 60.0),
        ("min-height:60px", 60.0),
        ("max-height:30px", 30.0),
    ] {
        let rest = format!(
            "{}{}",
            narrow(&format!("break-inside:avoid;{style}"), "Y"),
            narrow("", "Z")
        );
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            [(1, 0.0, 0.0), (1, 0.0, after)],
            "{style}"
        );
    }
}

#[test]
fn nested_avoiding_boxes_move_with_their_wrapper_without_external_float_space() {
    for outer in ["", "height:60px", "min-height:60px", "padding-top:5px"] {
        let rest = format!(
            "<section style='{outer}'>{}</section>{}",
            narrow("break-inside:avoid", "Y"),
            narrow("", "Z")
        );
        let top = if outer == "padding-top:5px" { 5.0 } else { 0.0 };
        let after = if matches!(outer, "height:60px" | "min-height:60px") {
            60.0
        } else {
            top + 30.0
        };
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            [(1, 0.0, top), (1, 0.0, after)],
            "{outer}"
        );
    }
}

#[test]
fn an_avoiding_box_paints_its_recomputed_height_on_the_new_page() {
    for (style, height) in [
        ("", 30.0),
        ("padding-top:10px", 40.0),
        ("min-height:60px", 60.0),
    ] {
        let document = lay_out(&after_left_float(
            60,
            20,
            &narrow(&format!("break-inside:avoid;background:red;{style}"), "Y"),
        ));
        let page = document.page(1).unwrap();
        let boxes: Vec<_> = page
            .fragments()
            .filter(|fragment| fragment.kind() == FragmentKind::Box)
            .map(|fragment| fragment.rect())
            .filter(|rect| rect.width == 20.0)
            .map(|rect| (rect.y, rect.height))
            .collect();
        assert_eq!(boxes, [(0.0, height)], "{style}");
        assert!(document.page(2).is_none());
    }
}

#[test]
fn atomics_move_with_the_avoiding_paragraphs_trimmed_lines() {
    let rest = format!(
        "{}{}",
        narrow(
            "break-inside:avoid;background:red",
            "<span style='display:inline-block;width:20px;height:30px'></span>"
        ),
        narrow("", "Z")
    );
    let body = after_left_float(60, 20, &rest);
    let document = lay_out(&body);
    let page = document.page(1).unwrap();
    let boxes: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.kind() == FragmentKind::Box)
        .map(|fragment| fragment.rect())
        .filter(|rect| rect.width == 20.0)
        .map(|rect| (rect.y, rect.height))
        .collect();
    assert!(boxes.contains(&(0.0, 30.0)), "atomic: {boxes:?}");
    assert!(boxes.contains(&(0.0, 39.0)), "paragraph: {boxes:?}");
    assert_eq!(lines(&body), [(1, 0.0, 39.0)]);
}

#[test]
fn text_in_the_parent_paragraph_follows_the_shortened_avoiding_child() {
    for wrap in [false, true] {
        let child = narrow("break-inside:avoid", "Y");
        let child = if wrap {
            format!("<span style='display:contents'>{child}</span>")
        } else {
            child
        };
        let rest = format!("<section>{child}Z</section>");
        let document = lay_out(&after_left_float(60, 20, &rest));
        let page = document.page(1).unwrap();
        for (text, top) in [("Y", 0.0), ("Z", 30.0)] {
            let run = page
                .text_runs()
                .into_iter()
                .find(|run| run.text.trim() == text)
                .unwrap();
            assert_eq!(
                run.origin.1 - BASELINE_BELOW_LINE_TOP,
                top,
                "{text}, contents {wrap}"
            );
        }
    }
}

#[test]
fn a_later_float_inside_the_moving_wrapper_follows_the_shortened_paragraph() {
    let rest = format!(
        "<section style='break-inside:avoid'>{}<div style='float:left;width:20px;height:10px'></div></section>{}",
        narrow("break-inside:avoid", "Y"),
        narrow("", "Z")
    );
    assert_eq!(
        lines(&after_left_float(60, 20, &rest)),
        [(1, 0.0, 0.0), (1, 0.0, 40.0)]
    );
}

#[test]
fn authored_offsets_and_positioned_children_survive_the_removed_float_gap() {
    for (style, inner, expected) in [
        (
            "position:relative;top:5px",
            "Y",
            vec![(1, 0.0, 5.0), (1, 0.0, 30.0)],
        ),
        (
            "border-top:5px solid cyan",
            "Y",
            vec![(1, 0.0, 5.0), (1, 0.0, 35.0)],
        ),
        (
            "position:relative",
            "Y<div style='position:absolute;top:0;left:50px;width:10px;height:10px'>Q</div>",
            vec![(1, 0.0, 0.0), (1, 0.0, 30.0)],
        ),
        (
            "position:relative",
            "Y<div style='position:absolute;bottom:0;left:50px;width:10px;height:10px'>Q</div>",
            vec![(1, 0.0, 0.0), (1, 50.0, 20.0), (1, 0.0, 30.0)],
        ),
    ] {
        let rest = format!(
            "{}{}",
            narrow(&format!("break-inside:avoid;{style}"), inner),
            narrow("", "Z")
        );
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            expected,
            "{style}, {inner}"
        );
    }
}

#[test]
fn a_float_declaration_on_an_absolute_box_does_not_retain_the_prefix() {
    for float in ["", "float:left", "float:right"] {
        let rest = format!(
            "<section style='break-inside:avoid;position:relative'><div style='position:absolute;top:0;left:50px;width:10px;height:100px;{float}'></div>{}</section>{}",
            narrow("break-inside:avoid", "Y"),
            narrow("", "Z")
        );
        assert_eq!(
            lines(&after_left_float(60, 20, &rest)),
            [(1, 0.0, 0.0), (1, 0.0, 30.0)],
            "{float}"
        );
    }
}

fn assert_word_on_page(document: &DocumentLayout, word: &str, page: u32, x: f32, top: f32) {
    let page = document.page(page).unwrap();
    let run = page
        .text_runs()
        .into_iter()
        .find(|run| run.text.trim() == word)
        .unwrap();
    assert_eq!(
        (run.origin.0, run.origin.1 - BASELINE_BELOW_LINE_TOP),
        (x, top),
        "{word}"
    );
}

fn short_spacer_boxes(document: &DocumentLayout) -> Vec<(u32, f32)> {
    let mut found = Vec::new();
    for index in 0..3 {
        if let Some(page) = document.page(index) {
            found.extend(
                page.fragments()
                    .filter(|fragment| fragment.kind() == FragmentKind::Box)
                    .map(|fragment| fragment.rect())
                    .filter(|rect| rect.width == 10.0 && rect.height == 5.0)
                    .map(|rect| (index, rect.y)),
            );
        }
    }
    found
}

#[test]
fn earlier_content_in_a_moving_holder_does_not_take_a_later_paragraphs_correction() {
    for (fixed, after) in [(false, 35.0), (true, 65.0)] {
        let root = narrow("", "Y");
        let root = if fixed {
            format!("<article style='height:60px'>{root}</article>")
        } else {
            root
        };
        let rest = format!(
            "<section style='break-inside:avoid'><div style='width:10px;height:5px;background:cyan;font:5px/5px Ahem'>P</div>{root}</section>{}",
            narrow("", "Z")
        );
        let document = lay_out(&after_left_float(60, 20, &rest));
        assert_eq!(short_spacer_boxes(&document), [(1, 0.0)], "fixed {fixed}");
        assert_word_on_page(&document, "Y", 1, 0.0, 5.0);
        assert_word_on_page(&document, "Z", 1, 0.0, after);
    }
}

#[test]
fn each_future_gap_owner_keeps_its_correction_until_it_is_visited() {
    let rest = format!(
        "<section style='break-inside:avoid;min-height:60px'><div style='width:10px;height:5px;background:magenta;font:5px/5px Ahem'>P</div>{}{}</section>{}",
        narrow("height:0", "Y"),
        narrow("height:0;text-indent:5px;color:cyan", "W"),
        narrow("", "Z")
    );
    let document = lay_out(&after_left_float(60, 20, &rest));
    assert_eq!(short_spacer_boxes(&document), [(1, 0.0)]);
    assert_word_on_page(&document, "Y", 1, 0.0, 5.0);
    assert_word_on_page(&document, "W", 1, 5.0, 5.0);
    assert_word_on_page(&document, "Z", 1, 0.0, 60.0);
}

#[test]
fn shrinking_a_block_child_keeps_an_ancestors_fixed_top_absolute_box_in_place() {
    let rest = format!(
        "<section style='position:relative;break-inside:avoid'><div style='position:absolute;top:70px;left:50px;width:10px;height:10px'>Q</div><div style='width:10px;height:5px;background:magenta;font:5px/5px Ahem'>P</div>{}<span>Z</span></section>{}",
        narrow("", "Y"),
        narrow("", "W")
    );
    let document = lay_out(&after_left_float(60, 20, &rest));
    assert_word_on_page(&document, "Q", 1, 50.0, 70.0);
    assert_word_on_page(&document, "Y", 1, 0.0, 5.0);
    assert_word_on_page(&document, "Z", 1, 0.0, 35.0);
    assert_word_on_page(&document, "W", 1, 0.0, 65.0);
    assert_eq!(short_spacer_boxes(&document), [(1, 0.0)]);
}

#[test]
fn shrinking_a_block_child_keeps_a_static_absolute_box_with_its_line() {
    let rest = format!(
        "<section style='break-inside:avoid'><div style='width:10px;height:5px;background:magenta;font:5px/5px Ahem'>P</div>{}<span>Z</span><span style='position:absolute;left:50px;width:10px;height:10px'>Q</span></section>{}",
        narrow("", "Y"),
        narrow("", "W")
    );
    let document = lay_out(&after_left_float(60, 20, &rest));
    assert_word_on_page(&document, "Q", 1, 50.0, 35.0);
    assert_word_on_page(&document, "Y", 1, 0.0, 5.0);
    assert_word_on_page(&document, "Z", 1, 0.0, 35.0);
    assert_word_on_page(&document, "W", 1, 0.0, 65.0);
    assert_eq!(short_spacer_boxes(&document), [(1, 0.0)]);
}
