//! Text next to the float that starts a page after a forced break. The
//! positions are the ones Chromium prints for the same documents.

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
/// The same break after a 10px block, for a float that starts the first page.
const SHORT_BREAK: &str = "<div style='break-after:page;height:10px'>a</div>";
/// Two lines of two words, or four lines when something narrows them.
const TEXT: &str = "XX XX XX XX";

fn float(side: &str, width: u32, height: u32) -> String {
    format!("<div style='float:{side};width:{width}px;height:{height}px'></div>")
}

/// The page, left edge and top of every line of text but the lead-in `a`.
fn lines(body: &str) -> Vec<(u32, f32, f32)> {
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
    let mut lines = Vec::new();
    for index in 0..4 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            if run.text.trim_end() != "a" {
                lines.push((index, run.origin.0, run.origin.1 - ASCENT));
            }
        }
    }
    lines
}

#[test]
fn text_wraps_beside_the_float_that_starts_a_page() {
    // The float is 50px tall, so the first two 30px lines are beside it and
    // the third is below it.
    let left = lines(&format!(
        "{BREAK}{}<div>{TEXT}</div>",
        float("left", 40, 50)
    ));
    assert_eq!(left, [(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)]);
    let right = lines(&format!(
        "{BREAK}{}<div>{TEXT}</div>",
        float("right", 40, 50)
    ));
    assert_eq!(right, [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]);
    // Without the float the same text fills the lines.
    let none = lines(&format!("{BREAK}<div>{TEXT}</div>"));
    assert_eq!(none, [(1, 0.0, 5.0), (1, 0.0, 35.0)]);
}

#[test]
fn a_second_float_on_the_new_page_lines_up_beside_the_first() {
    let two = lines(&format!(
        "{BREAK}{}{}<div>{TEXT}</div>",
        float("left", 30, 50),
        float("left", 30, 40)
    ));
    assert_eq!(two, [(1, 60.0, 5.0), (1, 60.0, 35.0), (1, 0.0, 65.0)]);
}

#[test]
fn floats_of_the_earlier_page_do_not_narrow_text_beside_the_new_float() {
    // The 60px float stays on page 0. On page 1 only the 20px float is
    // beside the first line, so the second line is as wide as the page.
    let left_then_left = lines(&format!(
        "{}{SHORT_BREAK}{}<div>{TEXT}</div>",
        float("left", 40, 60),
        float("left", 40, 20)
    ));
    assert_eq!(
        left_then_left,
        [(1, 40.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]
    );
    let right_then_left = lines(&format!(
        "{}{SHORT_BREAK}{}<div>{TEXT}</div>",
        float("right", 40, 60),
        float("left", 40, 50)
    ));
    assert_eq!(
        right_then_left,
        [(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)]
    );
    let left_then_right = lines(&format!(
        "{}{SHORT_BREAK}{}<div>{TEXT}</div>",
        float("left", 40, 60),
        float("right", 40, 50)
    ));
    assert_eq!(
        left_then_right,
        [(1, 0.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]
    );
    // With no float of its own on the new page the text is not narrowed by
    // the earlier one either.
    let earlier_only = lines(&format!(
        "{}{SHORT_BREAK}<div>{TEXT}</div>",
        float("left", 40, 60)
    ));
    assert_eq!(earlier_only, [(1, 0.0, 5.0), (1, 0.0, 35.0)]);
}

#[test]
fn text_in_any_box_after_the_page_start_float_wraps_beside_it() {
    let beside = [(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)];
    let f = float("left", 40, 50);
    let bodies = [
        // Text directly in the body.
        format!("{BREAK}{f}{TEXT}"),
        // A box inside a box.
        format!("{BREAK}{f}<div><div>{TEXT}</div></div>"),
        // The float inside the box that follows the break.
        format!("{BREAK}<div>{f}<p style='margin:0'>{TEXT}</p></div>"),
        // A short box ahead of the text, which reaches beside the float.
        format!("{BREAK}{f}<div>XX</div><div>XX XX XX</div>"),
    ];
    for body in bodies {
        assert_eq!(lines(&body), beside, "{body}");
    }
    // The same with a float on the earlier page too.
    let e = float("left", 40, 60);
    let bodies = [
        format!("{e}{SHORT_BREAK}{f}{TEXT}"),
        format!("{e}{SHORT_BREAK}{f}<div><div>{TEXT}</div></div>"),
        format!("{e}{SHORT_BREAK}<div>{f}<p style='margin:0'>{TEXT}</p></div>"),
    ];
    for body in bodies {
        assert_eq!(lines(&body), beside, "{body}");
    }
}

#[test]
fn padding_around_the_text_beside_the_page_start_float_is_kept() {
    let padded = lines(&format!(
        "{BREAK}{}<div style='padding:0 5px'>{TEXT}</div>",
        float("left", 40, 50)
    ));
    // Inside the 5px padding only one word fits beside the float, and the
    // lines below the float start at the padding edge.
    assert_eq!(
        padded,
        [
            (1, 40.0, 5.0),
            (1, 40.0, 35.0),
            (2, 5.0, 5.0),
            (2, 5.0, 35.0)
        ]
    );
}

#[test]
fn a_box_that_clears_the_page_start_float_goes_below_it() {
    let f = float("left", 40, 50);
    // Below the 50px float only one 30px line fits on the page, which the
    // block with two lines does not allow, so it moves to the next page.
    let below = [(2, 0.0, 5.0), (2, 0.0, 35.0)];
    for clear in ["left", "both"] {
        let body = format!("{BREAK}{f}<div style='clear:{clear}'>{TEXT}</div>");
        assert_eq!(lines(&body), below, "clear: {clear}");
        let body = format!(
            "{}{SHORT_BREAK}{f}<div style='clear:{clear}'>{TEXT}</div>",
            float("left", 40, 60)
        );
        assert_eq!(lines(&body), below, "clear: {clear} after a float");
    }
    // A float on the other side is not cleared.
    let right = lines(&format!("{BREAK}{f}<div style='clear:right'>{TEXT}</div>"));
    assert_eq!(right, [(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)]);
}

#[test]
fn a_page_start_float_that_clears_still_clears_the_earlier_float() {
    let cleared = lines(&format!(
        "{}{SHORT_BREAK}<div style='float:left;width:40px;height:20px;clear:left'></div>\
         <div>{TEXT}</div>",
        float("left", 40, 60)
    ));
    assert_eq!(cleared, [(1, 40.0, 5.0), (1, 0.0, 35.0), (1, 0.0, 65.0)]);
}

#[test]
fn a_formatting_context_box_sits_beside_the_page_start_float() {
    // Its lines are as narrow as the room beside the float. Chromium lays the
    // part of the box on the next page out at full width again.
    let on_page_one = |body: &str| -> Vec<(u32, f32, f32)> {
        lines(body)
            .into_iter()
            .filter(|&(page, _, _)| page <= 1)
            .collect()
    };
    let beside = [(1, 40.0, 5.0), (1, 40.0, 35.0)];
    let boxed = format!("<div style='overflow:hidden'>{TEXT}</div>");
    let body = format!("{BREAK}{}{boxed}", float("left", 40, 50));
    assert_eq!(on_page_one(&body), beside);
    let body = format!(
        "{}{SHORT_BREAK}{}{boxed}",
        float("left", 40, 60),
        float("left", 40, 50)
    );
    assert_eq!(on_page_one(&body), beside);
}

#[test]
fn text_beside_the_page_start_float_in_a_paragraph_with_inline_content() {
    // Text beside the blocks makes the parent a paragraph that places its
    // blocks and floats itself. The `a` is not reported: it stands for the
    // text that is there to make the parent such a paragraph.
    let beside = [(1, 40.0, 5.0), (1, 40.0, 35.0), (1, 0.0, 65.0)];
    let f = float("left", 40, 50);
    let e = float("left", 40, 60);
    let bodies = [
        format!("{BREAK}{f}<div>{TEXT}</div>a"),
        format!("a{BREAK}{f}<div>{TEXT}</div>"),
        // Floats of the earlier page on either side.
        format!("{e}{SHORT_BREAK}{f}<div>{TEXT}</div>a"),
        format!(
            "{}{SHORT_BREAK}{f}<div>{TEXT}</div>a",
            float("right", 40, 60)
        ),
        // The page-start float clears the earlier float, the box after it
        // does not.
        format!("{e}{SHORT_BREAK}{f}<div style='clear:right'>{TEXT}</div>a"),
    ];
    for body in bodies {
        assert_eq!(lines(&body), beside, "{body}");
    }
    // A short box and a line of text ahead of the second box.
    let two = lines(&format!(
        "{e}{SHORT_BREAK}{f}<div>XX</div>a<div>XX XX</div>"
    ));
    assert_eq!(two, [(1, 40.0, 5.0), (1, 0.0, 65.0)]);
    // A box that clears the float goes below it.
    for clear in ["left", "both"] {
        let body = format!("{e}{SHORT_BREAK}{f}<div style='clear:{clear}'>{TEXT}</div>a");
        assert_eq!(
            lines(&body),
            [(2, 0.0, 5.0), (2, 0.0, 35.0)],
            "clear: {clear}"
        );
    }
    // A box that establishes a formatting context sits beside the float.
    let boxed = format!("{e}{SHORT_BREAK}{f}<div style='overflow:hidden'>{TEXT}</div>a");
    let on_page_one: Vec<_> = lines(&boxed)
        .into_iter()
        .filter(|&(page, _, _)| page <= 1)
        .collect();
    assert_eq!(on_page_one, [(1, 40.0, 5.0), (1, 40.0, 35.0)]);
}

#[test]
fn the_margin_above_the_box_after_the_page_start_float_is_kept() {
    // The 10px margin above the text stays at the top of the page, so every
    // line is 10px lower than without it and the third line ends at the bottom
    // edge of the page.
    let left = lines(&format!(
        "{BREAK}{}<div style='margin-top:10px'>{TEXT}</div>",
        float("left", 40, 50)
    ));
    assert_eq!(left, [(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)]);
    let right = lines(&format!(
        "{BREAK}{}<div style='margin-top:10px'>{TEXT}</div>",
        float("right", 40, 50)
    ));
    assert_eq!(right, [(1, 0.0, 15.0), (1, 0.0, 45.0), (1, 0.0, 75.0)]);
    // The margin of the box below a break with no float is kept the same way.
    let none = lines(&format!("{BREAK}<div style='margin-top:10px'>{TEXT}</div>"));
    assert_eq!(none, [(1, 0.0, 15.0), (1, 0.0, 45.0)]);
}

#[test]
fn the_margin_is_kept_whichever_box_carries_it() {
    let float = float("left", 40, 50);
    let expected = [(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)];
    // A margin above and below the box.
    let both = lines(&format!(
        "{BREAK}{float}<div style='margin:10px 0'>{TEXT}</div>"
    ));
    assert_eq!(both, expected);
    // A box whose child holds the text.
    let wrapped = lines(&format!(
        "{BREAK}{float}<div style='margin-top:10px'><div>{TEXT}</div></div>"
    ));
    assert_eq!(wrapped, expected);
    // The block before the break has a smaller margin below it than the box
    // after the float has above it.
    let smaller = lines(&format!(
        "<div style='break-after:page;height:30px;margin-bottom:5px'>a</div>\
         {float}<div style='margin-top:10px'>{TEXT}</div>"
    ));
    assert_eq!(smaller, expected);
    // The break is forced after a box nested in the block before the float,
    // or after the block itself.
    for before in [
        "<div><div style='break-after:page;height:30px'>a</div></div>",
        "<div><div><div style='break-after:page;height:30px'>a</div></div></div>",
        "<div style='break-after:page'><div style='height:30px'>a</div></div>",
    ] {
        let nested = lines(&format!(
            "{before}{float}<div style='margin-top:10px'>{TEXT}</div>"
        ));
        assert_eq!(nested, expected, "{before}");
    }
}
