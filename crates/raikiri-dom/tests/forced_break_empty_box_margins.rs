//! An empty box right after a forced break starts the new page. Chromium
//! truncates the block-end margin of the box before the break and keeps the
//! margins of the empty boxes after it whole below the page top, collapsed
//! with the margin of the box that follows them or ending the block that
//! holds them. The pages and positions are the ones Chromium prints for the
//! same markup: 100px pages without margins, Ahem at 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index and the top on that page of the element `#z`.
fn place_of_z(body: &str) -> (usize, f32) {
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
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            if page.dom().attr(fragment.node(), "id") == Some("z") {
                return (index, fragment.rect().y);
            }
        }
    }
    panic!("no fragment for #z in {body}");
}

/// Lays out every `(body, (page, top))` case and reports all that differ.
fn assert_places(cases: &[(String, (usize, f32))]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(body, expected)| {
            let (page, y) = place_of_z(body);
            (page != expected.0 || (y - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: #z expected {expected:?}, got ({page}, {y})"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A 20px box that forces a break after it, with a block-end margin.
fn breaking(margin: u32) -> String {
    format!("<div style='height:20px;margin-bottom:{margin}px;break-after:page'>T</div>")
}

/// An empty box of the given style.
fn empty(style: &str) -> String {
    format!("<div style='{style}'></div>")
}

/// The box `#z`, of the given style.
fn z(style: &str) -> String {
    format!("<div style='{style}'><span id=z>Z</span></div>")
}

/// A 30px line, the forced-break box with a 5px margin, `boxes` and `#z`.
fn sibling_case(boxes: &str, z_style: &str) -> String {
    format!(
        "<div style='height:30px'>A</div>{}{boxes}{}",
        breaking(5),
        z(z_style)
    )
}

/// A 20px-wide block of `wrapper` style holding four lines, the
/// forced-break box and `boxes`, followed by `after` and `#z`.
fn closing_case(wrapper: &str, breaking_box: &str, boxes: &str, after: &str) -> String {
    format!(
        "<div style='width:20px;{wrapper}'><div>X X X X</div>{breaking_box}{boxes}</div>{after}{}",
        z("")
    )
}

#[test]
fn the_box_after_empty_boxes_keeps_their_margins_below_the_page_top() {
    let mb30 = empty("height:0;margin-bottom:30px");
    assert_places(&[
        (sibling_case(&mb30, ""), (1, 35.0)),
        // An empty box of auto height collapses its top margin in.
        (
            sibling_case(&empty("margin-top:10px;margin-bottom:30px"), ""),
            (1, 35.0),
        ),
        // The top margin of the box after them joins the collapsed margin.
        (sibling_case(&mb30, "margin-top:20px"), (1, 35.0)),
        (sibling_case(&mb30, "margin-top:50px"), (1, 55.0)),
        (sibling_case(&mb30, "margin-top:-10px"), (1, 25.0)),
        // Two empty boxes collapse into one margin.
        (sibling_case(&format!("{mb30}{mb30}"), ""), (1, 35.0)),
        // A box of set height zero whose child has a margin sits below that
        // margin and keeps its own whole below it.
        (
            sibling_case(
                "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>",
                "",
            ),
            (1, 45.0),
        ),
        // Such a box right after the break splits the run: the margins after
        // it collapse with its own and add to those inside it.
        (
            sibling_case(
                &format!(
                    "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>{}",
                    empty("height:0;margin-bottom:50px")
                ),
                "",
            ),
            (1, 65.0),
        ),
        // After another empty box, all of them collapse into one.
        (
            sibling_case(
                &format!(
                    "{}<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>{}",
                    empty("height:0;margin-bottom:20px"),
                    empty("height:0;margin-bottom:50px")
                ),
                "",
            ),
            (1, 55.0),
        ),
        // A negative top margin collapses in too.
        (
            sibling_case(&empty("margin-top:-20px;margin-bottom:30px"), ""),
            (1, 15.0),
        ),
        (
            sibling_case(&empty("height:0;margin-top:-20px;margin-bottom:30px"), ""),
            (1, 15.0),
        ),
    ]);
}

#[test]
fn the_next_page_edge_truncates_the_kept_margin() {
    assert_places(&[
        (
            sibling_case(&empty("height:0;margin-bottom:150px"), ""),
            (2, 5.0),
        ),
        (
            closing_case(
                "padding-bottom:15px",
                &breaking(5),
                &empty("height:0;margin-bottom:150px"),
                "",
            ),
            (3, 20.0),
        ),
    ]);
}

#[test]
fn the_margin_before_the_break_does_not_add_to_the_kept_one() {
    let mb30 = empty("height:0;margin-bottom:30px");
    let lead = "<div style='height:30px'>A</div>";
    assert_places(&[
        (format!("{lead}{}{mb30}{}", breaking(40), z("")), (1, 35.0)),
        (
            format!(
                "{lead}<div style='height:20px;break-after:page'>T</div>{mb30}{}",
                z("")
            ),
            (1, 35.0),
        ),
    ]);
}

#[test]
fn empty_boxes_that_end_a_block_keep_their_margins_inside_it() {
    let mb30 = empty("height:0;margin-bottom:30px");
    let padded = "padding-bottom:15px";
    let t5 = breaking(5);
    assert_places(&[
        // The padding of the block ends below the kept margin.
        (closing_case(padded, &t5, &mb30, ""), (2, 50.0)),
        (
            closing_case(
                padded,
                &t5,
                &empty("margin-top:10px;margin-bottom:30px"),
                "",
            ),
            (2, 50.0),
        ),
        (
            closing_case(
                padded,
                &t5,
                &format!("{mb30}{}", empty("height:0;margin-bottom:50px")),
                "",
            ),
            (2, 70.0),
        ),
        (
            closing_case(
                padded,
                &t5,
                &empty("margin-top:30px;margin-bottom:-10px"),
                "",
            ),
            (2, 40.0),
        ),
        // A larger margin before the break is truncated all the same.
        (closing_case(padded, &breaking(40), &mb30, ""), (2, 50.0)),
        (
            closing_case(
                padded,
                &breaking(60),
                &format!("{}{mb30}", empty("height:0;margin-bottom:10px")),
                "",
            ),
            (2, 50.0),
        ),
        // A box of set height zero sits below the margin inside it and keeps
        // its own whole below it, which takes the line after it on.
        (
            closing_case(
                padded,
                &t5,
                "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:45px'></div></div>",
                "",
            ),
            (3, 5.0),
        ),
    ]);
}

#[test]
fn empty_boxes_that_end_a_block_pass_their_margins_on_to_the_box_after_it() {
    let mb30 = empty("height:0;margin-bottom:30px");
    let t5 = breaking(5);
    assert_places(&[
        (closing_case("", &t5, &mb30, ""), (2, 35.0)),
        (
            closing_case("margin-bottom:40px", &t5, &mb30, ""),
            (2, 45.0),
        ),
        (
            format!(
                "<div>{}</div>{}",
                closing_case("", &t5, &mb30, "").replace(&z(""), ""),
                z("")
            ),
            (2, 35.0),
        ),
        (
            format!(
                "<div style='width:20px'><div>X X X X</div>{t5}{mb30}</div>{}",
                z("margin-top:50px")
            ),
            (2, 55.0),
        ),
        // A block of set height holds the kept margin, and so does one
        // whose min-height ends it below the empty box.
        (closing_case("height:200px", &t5, &mb30, ""), (2, 5.0)),
        (closing_case("min-height:200px", &t5, &mb30, ""), (2, 35.0)),
        // A line after the empty box keeps the margin above it.
        (
            closing_case("", &t5, &format!("{mb30}<div>W</div>"), ""),
            (2, 65.0),
        ),
    ]);
}
