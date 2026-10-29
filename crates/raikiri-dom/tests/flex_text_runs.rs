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

/// The text under a `display: contents` element has the values that element
/// gives it, which shape it in the one paragraph of the item: a larger font
/// makes its characters wider and its line taller.
#[test]
fn the_text_under_a_contents_element_keeps_its_own_style() {
    check(
        "display:inline-flex",
        &[
            (
                "larger font",
                "a<span style='display:contents;font-size:20px'>b</span>c",
                40.0,
                13.0,
            ),
            (
                "larger font between spaces",
                "a <span style='display:contents;font-size:20px'>b</span> c",
                60.0,
                13.0,
            ),
            (
                "nested larger fonts",
                "a<span style='display:contents;font-size:20px'>b\
                 <i style='display:contents;font-size:30px'>c</i></span>d",
                70.0,
                16.0,
            ),
            (
                "the first text has the larger font",
                "<span style='display:contents;font-size:20px'>b</span>c",
                30.0,
                13.0,
            ),
            (
                "white space before the root has a larger font",
                "<span style='display:contents;font-size:20px'> </span>c\
                 <span style='display:contents;font-size:20px'>d</span>",
                30.0,
                13.0,
            ),
        ],
    );
}

/// The strut of the item's lines comes from the container: a text node under a
/// `display: contents` element with another `line-height` makes only its own
/// line taller, whichever text of the run it is.
#[test]
fn the_strut_of_the_item_comes_from_the_container() {
    check(
        "display:inline-flex;width:30px",
        &[
            (
                "taller first text, only the first line grows",
                "<span style='display:contents;line-height:100px'>a </span>b c d",
                30.0,
                110.0,
            ),
            (
                "taller text in the middle",
                "a <span style='display:contents;line-height:100px'>b </span>c d",
                30.0,
                110.0,
            ),
            (
                "white space that the line start removes does not make a line taller",
                "<span style='display:contents;line-height:100px'> </span>a b c d",
                30.0,
                20.0,
            ),
        ],
    );
    check(
        "display:inline-flex",
        &[
            (
                "taller first text in a single line",
                "<span style='display:contents;line-height:100px'>a</span>b",
                20.0,
                100.0,
            ),
            (
                "kept white space makes the line taller",
                "a<span style='display:contents;line-height:100px'> </span>c",
                30.0,
                100.0,
            ),
        ],
    );
}
