//! The block after a box whose content moved down a page, when that box has
//! bottom padding and ends with a margin, a negative one among them, or with
//! room that its `min-height` leaves. The positions are the ones Chromium
//! prints for the same documents on pages 100px square, with Ahem glyphs of
//! 20px on lines of 30px: four lines of one glyph each that the page breaks
//! two and two, then a box `T`, then the block `Z` after the box.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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

/// The page count, then the page and glyph top of `T` and of `Z` in `body`.
fn pages_and_tops(body: &str) -> (u32, Vec<(char, u32, i32)>) {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let glyph = run.text.chars().next().unwrap();
            if "TZ".contains(glyph) {
                tops.push((glyph, index, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    (result.page_count(), tops)
}

/// A box `style` with bottom padding of 15px around the four lines and then
/// `tail`, followed by the block `Z`.
fn moved_box(style: &str, tail: &str) -> String {
    format!(
        "<div style='width:20px;{style}padding-bottom:15px'><div>X X X X</div>{tail}</div>\
         <div>Z</div>"
    )
}

/// Asserts that `T` and `Z` land where Chromium puts them, on three pages.
fn assert_tops(style: &str, tail: &str, t: (u32, i32), z: (u32, i32)) {
    let body = moved_box(style, tail);
    assert_eq!(
        pages_and_tops(&body),
        (3, vec![('T', t.0, t.1), ('Z', z.0, z.1)]),
        "{body}"
    );
}

#[test]
fn a_page_edge_truncates_the_margin_of_an_empty_last_box() {
    // The empty box stays below `T`, and its margin ends at the page edge
    // whatever its size, so the padding alone begins the next page.
    for style in ["", "min-height:170px;", "min-height:190px;"] {
        for margin in [5, 10, 30] {
            let tail = format!(
                "<div style='height:30px'>T</div><div style='margin-bottom:{margin}px'></div>"
            );
            assert_tops(style, &tail, (1, 65), (2, 20));
        }
    }
}

#[test]
fn a_negative_margin_of_the_last_box_pulls_the_padding_up() {
    // An empty box and a box with lines both end the content at their
    // margin, which ends above their border box.
    for style in ["", "min-height:170px;"] {
        let tail = "<div style='height:30px'>T</div><div style='margin-bottom:-10px'></div>";
        assert_tops(style, tail, (1, 65), (2, 5));
    }
    for style in ["min-height:170px;", "min-height:190px;"] {
        let tail = "<div><div style='height:40px;margin-bottom:-10px'>T</div></div>";
        assert_tops(style, tail, (2, 5), (2, 50));
    }
    let tail = "<div><div style='height:40px;margin-bottom:-20px'>T</div></div>";
    assert_tops("min-height:170px;", tail, (1, 65), (2, 5));
}

#[test]
fn padding_below_the_room_of_a_min_height_begins_the_next_page() {
    // The moved content still ends within the room, which ends 10px above
    // the page end: the padding does not fit there and begins the next page.
    let tail = "<div style='height:20px'>T</div>";
    assert_tops("min-height:190px;", tail, (1, 65), (2, 20));
    let body = "<div style='width:20px;min-height:190px;padding-bottom:15px'>\
                <div>X X X X</div></div><div>Z</div>";
    assert_eq!(pages_and_tops(body), (3, vec![('Z', 2, 20)]), "{body}");
}

#[test]
fn an_empty_last_box_of_set_height_zero_moves_on_with_the_padding() {
    // With no margin collapsing into its top, a box that `height: 0` makes
    // empty begins the next page with the padding, and keeps its bottom
    // margin whole above it. A negative margin still pulls the padding up.
    for style in ["", "min-height:170px;"] {
        for (t, margin, z) in [(20, 10, 30), (20, 30, 50), (25, 5, 25), (30, 10, 30)] {
            let tail = format!(
                "<div style='height:{t}px'>T</div><div style='height:0;margin-bottom:{margin}px'></div>"
            );
            assert_tops(style, &tail, (1, 65), (2, z));
        }
        let tail =
            "<div style='height:30px'>T</div><div style='height:0;margin-bottom:-10px'></div>";
        assert_tops(style, tail, (1, 65), (2, 5));
        // A box that holds its content in a context of its own, or whose
        // content overflows it, takes no margins through it either.
        for last in [
            "<div style='display:flow-root;height:0;margin-bottom:30px'></div>",
            "<div style='height:0;margin-bottom:30px'><div style='height:10px'></div></div>",
        ] {
            let tail = format!("<div style='height:20px'>T</div>{last}");
            assert_tops(style, &tail, (1, 65), (2, 50));
        }
    }
}

#[test]
fn a_margin_above_an_empty_box_of_set_height_zero_keeps_it_behind() {
    // A margin that collapses into the top of the empty box, its own or one
    // of the boxes before it, joins the margins that collapse through it: the
    // box stays, and the page edge truncates them.
    for style in ["", "min-height:170px;"] {
        for tail in [
            "<div style='height:20px;margin-bottom:5px'>T</div>\
             <div style='height:0;margin-bottom:30px'></div>",
            "<div style='height:30px;margin-bottom:-5px'>T</div>\
             <div style='height:0;margin-bottom:10px'></div>",
            "<div style='height:20px'>T</div><div style='margin-bottom:5px'></div>\
             <div style='height:0;margin-bottom:30px'></div>",
            "<div style='height:20px'>T</div><div style='height:0;margin-top:10px'></div>",
            // An empty box of auto height after it ends the flow and stays,
            // and the margins of both collapse through to the page edge.
            "<div style='height:20px'>T</div><div style='height:0;margin-bottom:30px'></div>\
             <div style='margin-bottom:5px'></div>",
            // Margins that cancel out still keep the box behind.
            "<div style='height:20px;margin-bottom:10px'>T</div>\
             <div style='height:0;margin-top:-10px;margin-bottom:30px'></div>",
        ] {
            assert_tops(style, tail, (1, 65), (2, 20));
        }
    }
}

#[test]
fn an_empty_last_box_that_begins_past_the_page_end_moves_the_padding_on() {
    // The empty box sits where the margins above it end. Once that is past
    // the page end, the padding begins the next page, even when the negative
    // margin of the empty box pulls the end of the content back up.
    let tail = |t: i32, above: i32, below: i32| {
        format!(
            "<div style='height:{t}px;margin-bottom:{above}px'>T</div>\
             <div style='margin-bottom:{below}px'></div>"
        )
    };
    for (t, above, below) in [
        (30, 20, -30),
        (30, 20, -50),
        (20, 40, -50),
        (35, 10, -30),
        (40, 10, -50),
    ] {
        assert_tops("", &tail(t, above, below), (1, 65), (2, 20));
    }
    // Where the empty box still begins on the page, the padding stays
    // behind, or the next block follows on the same page.
    assert_tops("", &tail(30, 10, -30), (1, 65), (2, 5));
    for (t, above, below, z) in [(20, 5, -30, 75), (20, 20, -50, 70)] {
        let body = moved_box("", &tail(t, above, below));
        assert_eq!(
            pages_and_tops(&body),
            (2, vec![('T', 1, 65), ('Z', 1, z)]),
            "{body}"
        );
    }
}

#[test]
fn a_negative_margin_escapes_a_box_whose_moved_content_outgrew_its_min_height() {
    // Without bottom padding, the last margin collapses through the bottom of
    // the box only while its content fills it. When the moved content grows
    // the box past its `min-height`, a negative margin escapes it and pulls
    // the next block up.
    for min_height in [170, 190] {
        for wrapper in [false, true] {
            for (t, margin, pages, z) in [
                (30, -10, 3, (2, 5)),
                (30, -20, 2, (1, 75)),
                (30, -40, 2, (1, 55)),
                (40, -20, 3, (2, 5)),
                (40, -40, 2, (1, 65)),
            ] {
                let last = format!("<div style='height:{t}px;margin-bottom:{margin}px'>T</div>");
                let last = if wrapper {
                    format!("<div>{last}</div>")
                } else {
                    last
                };
                let body = format!(
                    "<div style='width:20px;min-height:{min_height}px'><div>X X X X</div>{last}</div>\
                     <div>Z</div>"
                );
                assert_eq!(
                    pages_and_tops(&body),
                    (pages, vec![('T', 1, 65), ('Z', z.0, z.1)]),
                    "{body}"
                );
            }
        }
    }
}

#[test]
fn an_escaping_margin_collapses_with_the_margins_after_the_box() {
    // The negative margin that escapes the box collapses with the bottom
    // margin of the box and the top margin of the next block, and moves the
    // flow by no more than that collapsed margin changes.
    for (own, last, z_top, z) in [
        (-10, -40, 0, 55),
        (-10, -20, 0, 75),
        (-60, -40, 0, 35),
        (-60, -20, 0, 35),
        (0, -40, -10, 55),
    ] {
        let body = format!(
            "<div style='width:20px;min-height:170px;margin-bottom:{own}px'><div>X X X X</div>\
             <div style='height:30px;margin-bottom:{last}px'>T</div></div>\
             <div style='margin-top:{z_top}px'>Z</div>"
        );
        assert_eq!(
            pages_and_tops(&body),
            (2, vec![('T', 1, 65), ('Z', 1, z)]),
            "{body}"
        );
    }
}

#[test]
fn the_margins_inside_an_empty_last_box_place_it_below_the_page_end() {
    // A margin of a box inside the empty box collapses into its top, and
    // places it as far down as its own would.
    for (t, inner, below) in [(20, 40, -50), (30, 20, -30), (30, 30, -50)] {
        let tail = format!(
            "<div style='height:{t}px'>T</div>\
             <div style='margin-bottom:{below}px'><div style='margin-top:{inner}px'></div></div>"
        );
        assert_tops("", &tail, (1, 65), (2, 20));
    }
}

#[test]
fn a_negative_margin_stops_escaping_when_truncation_shrinks_the_content() {
    // The content first outgrows the `min-height` and lets the last margin
    // through. The page edge then truncates the top margin of `Y`, the
    // content ends above the `min-height`, and the flow after the box
    // follows the box again.
    for top in [40, 60] {
        let body = format!(
            "<div style='width:20px;min-height:200px'><div>X X X</div>\
             <div style='margin-top:{top}px;height:60px'>Y</div>\
             <div style='height:30px;margin-bottom:-40px'>T</div></div><div>Z</div>"
        );
        assert_eq!(
            pages_and_tops(&body),
            (3, vec![('T', 1, 65), ('Z', 2, 5)]),
            "{body}"
        );
    }
}

#[test]
fn the_margins_after_the_box_reach_through_empty_boxes_and_contents_wrappers() {
    // Empty boxes after the box let their margins through to the escaping
    // one, and so does the top margin of a block inside a contents wrapper.
    let head = "<div style='width:20px;min-height:170px'><div>X X X X</div>\
                <div style='height:30px;margin-bottom:-40px'>T</div></div>";
    for (after, z) in [
        ("<div style='margin-bottom:-60px'></div><div>Z</div>", 35),
        ("<div style='margin-bottom:-20px'></div><div>Z</div>", 55),
        (
            "<div></div><div style='margin-top:-60px'></div><div>Z</div>",
            35,
        ),
        (
            "<div style='display:contents'><div style='margin-top:-60px'>Z</div></div>",
            35,
        ),
        (
            "<div style='display:contents'><div style='margin-top:-10px'>Z</div></div>",
            55,
        ),
    ] {
        let body = format!("{head}{after}");
        assert_eq!(
            pages_and_tops(&body),
            (2, vec![('T', 1, 65), ('Z', 1, z)]),
            "{body}"
        );
    }
}
