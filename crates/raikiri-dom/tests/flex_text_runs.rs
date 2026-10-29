//! The text of a flex or grid container is wrapped in anonymous items (CSS
//! Flexbox 1, section 4; CSS Grid 1, section 6): each sequence of text runs
//! that no box separates makes one item. A `display: contents` element
//! generates no box, so its text joins the runs on both sides of it.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and `10px Ahem` text
//! (each character is 10px wide and 10px high): each is the container's left,
//! top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }}</style>{body}"
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// Lays out `<div id=r style=...>content</div>` for each case, and compares
/// the width and height of the container with Chromium's.
fn check(style: &str, cases: &[(&str, &str, f32, f32)]) {
    for &(label, content, width, height) in cases {
        let actual = boxes(&format!("<div id=r style='{style}'>{content}</div>"));
        let (_, rect) = actual
            .iter()
            .find(|(name, _)| name == "r")
            .unwrap_or_else(|| panic!("{label}: no fragment for #r"));
        assert!(
            (rect[2] - width).abs() <= TOLERANCE && (rect[3] - height).abs() <= TOLERANCE,
            "{label}: #r is {}x{}, expected {width}x{height}",
            rect[2],
            rect[3]
        );
    }
}

/// Text on both sides of a `display: contents` element, and its own text,
/// are one anonymous item: the space between the runs stays, so the item is
/// as wide as its text on one line.
#[test]
fn text_around_a_contents_element_is_one_item() {
    check(
        "display:inline-flex",
        &[
            (
                "contents between",
                "a <b style='display:contents'>b</b> c",
                50.0,
                10.0,
            ),
            (
                "nested contents",
                "a <b style='display:contents'>b <i style='display:contents'>i</i> j</b> c",
                90.0,
                10.0,
            ),
            (
                "white space alone in the contents element",
                "a<b style='display:contents'> </b>c",
                30.0,
                10.0,
            ),
            (
                "white space leads the run",
                " <b style='display:contents'> b</b> c",
                30.0,
                10.0,
            ),
        ],
    );
    check(
        "display:inline-grid",
        &[(
            "grid container",
            "a <b style='display:contents'>b</b> c",
            50.0,
            10.0,
        )],
    );
}

/// Preserved white space between the runs is kept, and the lines of the item
/// wrap as one paragraph.
#[test]
fn the_runs_make_one_paragraph() {
    check(
        "display:inline-flex;white-space:pre",
        &[(
            "preserved",
            "a<b style='display:contents'> </b>c",
            30.0,
            10.0,
        )],
    );
    check(
        "display:inline-flex;width:50px",
        &[(
            "wrapped",
            "a <b style='display:contents'>bb</b> cc",
            50.0,
            20.0,
        )],
    );
}

/// An element that generates no box, and a comment, do not separate the text
/// around them; any box does.
#[test]
fn only_a_box_separates_the_runs() {
    check(
        "display:inline-flex",
        &[
            (
                "display none",
                "a <span style='display:none'>x</span> b",
                30.0,
                10.0,
            ),
            ("comment", "a <!-- c --> b", 30.0, 10.0),
            (
                "plain span is an item of its own",
                "a <b>b</b> c",
                30.0,
                10.0,
            ),
            (
                "inline child of a contents element is an item",
                "a <b style='display:contents'><i>x</i></b> c",
                30.0,
                10.0,
            ),
            (
                "block in a contents element splits the text",
                "a <b style='display:contents'>b<div>x</div>c</b> d",
                70.0,
                10.0,
            ),
        ],
    );
}
