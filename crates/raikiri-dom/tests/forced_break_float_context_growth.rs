//! A block that holds its floats (CSS 2.1 §10.6.7) grows around the floats
//! that a forced break inside it moves to the next page, and the flow after
//! the block follows that growth. The pages and positions are the ones
//! Chromium prints for the same markup: 100px pages without margins, Ahem at
//! 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index, top and bottom on that page of each fragment of the
/// element with the id `id`.
fn fragments_of(body: &str, id: &str) -> Vec<(usize, f32, f32)> {
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
    let mut fragments = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            if page.dom().attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                fragments.push((index, rect.y, rect.y + rect.height));
            }
        }
    }
    fragments
}

/// The page index and the top on that page of the first fragment of the
/// element `#z`.
fn place_of_z(body: &str) -> (usize, f32) {
    let &(page, top, _) = fragments_of(body, "z")
        .first()
        .unwrap_or_else(|| panic!("no fragment for #z in {body}"));
    (page, top)
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

/// A flow root of `style` holding a 30px box, a 20px box that forces a break
/// after it with a 5px margin, and `content`, followed by `#z`.
fn case(style: &str, content: &str) -> String {
    format!(
        "<div style='display:flow-root;width:100px;{style}'>\
         <div style='height:30px'></div>\
         <div style='height:20px;margin-bottom:5px;break-after:page'></div>\
         {content}</div><div><span id=z>Z</span></div>"
    )
}

/// A float of `side` with `style`.
fn float(side: &str, style: &str) -> String {
    format!("<div style='float:{side};width:20px;{style}'>F</div>")
}

const BOX: &str = "<div style='height:10px'></div>";
const PADDED: &str = "padding-bottom:15px";

#[test]
fn the_flow_after_a_flow_root_follows_the_floats_a_forced_break_moved() {
    assert_places(&[
        // The float runs on to the third page, and the block's padding after
        // it leaves no room for the line after the block there.
        (
            case(PADDED, &format!("{BOX}{}", float("left", "height:150px"))),
            (3, 5.0),
        ),
        (
            case(
                PADDED,
                &format!(
                    "<div style='height:10px;margin-bottom:30px'></div>{}",
                    float("left", "height:150px")
                ),
            ),
            (3, 20.0),
        ),
        // The float ends on the page the break opened.
        (
            case(PADDED, &format!("{BOX}{}", float("left", "height:50px"))),
            (2, 5.0),
        ),
        (
            case(PADDED, &format!("{BOX}{}", float("left", "height:20px"))),
            (1, 50.0),
        ),
        // Its bottom margin, a float inside a block of the same context and
        // the lowest of two floats count too.
        (
            case(
                PADDED,
                &format!("{BOX}{}", float("left", "height:40px;margin-bottom:20px")),
            ),
            (2, 5.0),
        ),
        (
            case(
                PADDED,
                &format!("{BOX}<div>{}</div>", float("right", "height:60px")),
            ),
            (2, 5.0),
        ),
        (
            case(
                PADDED,
                &format!(
                    "{BOX}{}{}",
                    float("left", "height:40px"),
                    float("left", "height:60px")
                ),
            ),
            (2, 5.0),
        ),
        // In-flow content below the float ends the block lower.
        (
            case(
                PADDED,
                &format!("{BOX}{}{BOX}", float("left", "height:40px")),
            ),
            (1, 70.0),
        ),
        // A min-height below the float leaves the float to end the block.
        (
            case(
                "padding-bottom:15px;min-height:60px",
                &format!("{BOX}{}", float("left", "height:40px")),
            ),
            (1, 70.0),
        ),
    ]);
}

#[test]
fn a_flow_root_inside_a_block_or_holding_the_float_deeper_grows_too() {
    assert_places(&[
        (
            format!(
                "<div>{}</div><div><span id=z>Z</span></div>",
                case(PADDED, &format!("{BOX}{}", float("left", "height:40px")))
                    .replace("<div><span id=z>Z</span></div>", "")
            ),
            (1, 70.0),
        ),
        (
            "<div style='overflow:hidden;width:100px'><div style='height:30px'></div>\
             <div style='height:20px;break-after:page'></div>\
             <div><div style='height:10px'></div>\
             <div style='float:left;width:20px;height:40px'>F</div></div></div>\
             <div><span id=z>Z</span></div>"
                .to_owned(),
            (1, 55.0),
        ),
    ]);
}

#[test]
fn bottom_edges_that_a_forced_break_left_short_of_a_page_end_move_whole() {
    // The content of the block ends 10px above the end of the third page,
    // and its 15px bottom padding goes whole to the fourth, after a float or
    // a box of the flow alike, in a flow root or a plain block.
    let tall = |style: &str| {
        format!(
            "<div style='height:10px;margin-bottom:30px'></div><div style='width:20px;height:150px;{style}'></div>"
        )
    };
    for body in [
        case(PADDED, &tall("float:left")),
        case(PADDED, &tall("")),
        case(PADDED, &tall("")).replace("display:flow-root;", ""),
    ] {
        let w = body.replacen("<div style=", "<div id=w style=", 1);
        let last = fragments_of(&w, "w").last().copied();
        assert_eq!(last, Some((3, 0.0, 15.0)), "{w}");
        assert_eq!(place_of_z(&w), (3, 20.0), "{w}");
    }
}
