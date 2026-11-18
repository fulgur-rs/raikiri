//! An empty box of set height zero whose children have margins lets them
//! collapse through its top, so the box sits below them. A page edge in those
//! margins truncates them, but the box then starts the next page and keeps
//! its own bottom margin whole below it, as Chromium does. The pages and
//! positions are the ones Chromium prints for the same markup: 100px pages
//! without margins, Ahem at 20px on 30px lines.

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

/// Lays out every `(wrapper style, empty boxes, z style, (page, top))` case
/// after two pages of lines and a 20px box, and reports all that differ.
fn assert_places(lead: &str, cases: &[(&str, &str, &str, (usize, f32))]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|&(wrapper, boxes, z, expected)| {
            let body = format!(
                "<div style='width:20px;{wrapper}'>{lead}\
                 <div style='height:20px'>T</div>{boxes}</div>\
                 <div style='{z}'><span id=z>Z</span></div>"
            );
            let (page, y) = place_of_z(&body);
            (page != expected.0 || (y - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: #z expected {expected:?}, got ({page}, {y})"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

const LINES: &str = "<div>X X X X</div>";
const PADDED: &str = "padding-bottom:15px";

#[test]
fn the_margin_below_the_box_is_what_its_own_adds_to_those_inside_it() {
    // Unpaginated, the box sits 40px below the 20px box and its own 30px
    // margin adds nothing below it: the 15px padding then ends at 75 and the
    // line after it moves to the next page whole.
    assert_places(
        "",
        &[
            (
                PADDED,
                "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:40px'></div></div>",
                "",
                (1, 5.0),
            ),
            (
                PADDED,
                "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>",
                "",
                (0, 70.0),
            ),
        ],
    );
}

#[test]
fn a_box_past_the_page_edge_keeps_its_own_bottom_margin() {
    let split =
        "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:40px'></div></div>";
    assert_places(
        LINES,
        &[
            ("", split, "", (2, 35.0)),
            ("", split, "margin-top:20px", (2, 35.0)),
            ("", split, "margin-top:50px", (2, 55.0)),
            ("", split, "margin-top:-10px", (2, 25.0)),
            (
                "",
                "<div style='height:0;margin-bottom:30px'><div style='margin-top:40px'></div></div>",
                "",
                (2, 35.0),
            ),
            (
                "",
                "<div style='height:0;margin-bottom:30px'><div><div style='margin-bottom:40px'></div></div></div>",
                "",
                (2, 35.0),
            ),
            (
                "",
                "<div style='height:0;margin:30px 0'><div style='margin-bottom:40px'></div></div>",
                "",
                (2, 35.0),
            ),
            (
                "",
                "<div style='height:0;margin:40px 0 30px'><div style='margin-bottom:10px'></div></div>",
                "",
                (2, 35.0),
            ),
            (
                "",
                &format!("{split}<div style='height:0;margin-bottom:20px'></div>"),
                "",
                (2, 35.0),
            ),
            (
                "",
                &format!("{split}<div style='height:10px'></div>"),
                "",
                (2, 45.0),
            ),
            // A relative offset moves the box but not where the flow puts it.
            (
                "",
                "<div style='height:0;margin-bottom:30px;position:relative;top:-60px'><div style='margin-bottom:40px'></div></div>",
                "",
                (2, 35.0),
            ),
            (PADDED, split, "", (2, 50.0)),
            (
                PADDED,
                "<div style='height:0;margin:30px 0'><div style='margin-bottom:40px'></div></div>",
                "",
                (2, 50.0),
            ),
        ],
    );
}

#[test]
fn a_box_that_moves_with_the_edges_after_it_keeps_its_own_bottom_margin() {
    // The box ends on the page, but the margin and padding after it do not:
    // it moves on with them and keeps its whole margin.
    assert_places(
        LINES,
        &[(
            PADDED,
            "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>",
            "",
            (2, 50.0),
        )],
    );
}

#[test]
fn margins_around_a_box_on_the_page_or_without_inner_margins_are_truncated() {
    assert_places(
        LINES,
        &[
            // The box ends above the page edge, which truncates the rest.
            (
                "",
                "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:15px'></div></div>",
                "",
                (2, 5.0),
            ),
            // Relative offsets on the box or around it do not change that.
            (
                "",
                "<div style='height:0;margin-bottom:30px;position:relative;top:100px'><div style='margin-bottom:15px'></div></div>",
                "",
                (2, 5.0),
            ),
            (
                "",
                "<div style='position:relative;top:100px'><div style='height:0;margin-bottom:30px'><div style='margin-bottom:15px'></div></div></div>",
                "",
                (2, 5.0),
            ),
            // No margin inside the box: its own margins are truncated too.
            (
                "",
                "<div style='height:0;margin:30px 0'><div></div></div>",
                "",
                (2, 5.0),
            ),
            (
                "",
                "<div style='height:0;margin:30px 0'></div>",
                "",
                (2, 5.0),
            ),
            // A box of auto height lets the margins through as one.
            (
                "",
                "<div style='margin-bottom:30px'><div style='margin-bottom:40px'></div></div>",
                "",
                (2, 5.0),
            ),
        ],
    );
}

#[test]
fn display_contents_around_the_box_lends_it_the_margins_before_it() {
    // The 40px margin below the box before the split box collapses with the
    // margins inside it, whether or not `display: contents` elements hold it.
    let split =
        "<div style='height:0;margin-bottom:30px'><div style='margin-bottom:10px'></div></div>";
    let place = |boxes: &str, wrapper: &str, z: &str| {
        place_of_z(&format!(
            "<div style='width:20px;{wrapper}'>{LINES}\
             <div style='height:20px;margin-bottom:40px'>T</div>{boxes}</div>\
             <div style='{z}'><span id=z>Z</span></div>"
        ))
    };
    let contents = format!("<div style='display:contents'>{split}</div>");
    let nested = format!("<div style='display:contents'>{contents}</div>");
    assert_eq!(place(split, PADDED, "margin-top:-10px"), (2, 40.0));
    assert_eq!(place(&contents, PADDED, "margin-top:-10px"), (2, 40.0));
    assert_eq!(place(&nested, "", ""), (2, 35.0));
    assert_eq!(place(split, "", ""), (2, 35.0));
}
