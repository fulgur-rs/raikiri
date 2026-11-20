//! The content of a `<button>` laid out as a block container is centered along
//! the block axis by button layout itself (HTML Standard, "Button layout"), so
//! the centering holds without the UA style sheet and whatever `align-content`
//! the button has. A flex or grid button lays out its items as usual.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with no body margin and `10px Ahem` text: each is
//! the element's left, top, width and height.

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
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// A block-level button without its own padding, border or font.
const BUTTON: &str = "all: unset; display: block;";

/// A button without its default styles still centers its content, and its
/// `align-content` does not move it.
#[test]
fn a_button_centers_its_content_whatever_its_align_content() {
    let cases: Vec<(String, String)> = [
        "", "align-content: start;", "align-content: end;", "align-content: normal;",
        "align-content: baseline;", "align-content: space-between;",
    ]
    .into_iter()
    .map(|alignment| {
        (
            format!("block {alignment}"),
            format!("<button id=b style='{BUTTON} height: 20px; {alignment}'><span id=s>ab</span></button>"),
        )
    })
    .chain([
        (
            "inline-block".to_owned(),
            "<button id=b style='all: unset; display: inline-block; height: 20px'><span id=s>ab</span></button>".to_owned(),
        ),
        (
            "default styles with align-content: start".to_owned(),
            "<button id=b style='display: block; padding: 0; border: 0; font: inherit; height: 20px; align-content: start'><span id=s>ab</span></button>".to_owned(),
        ),
        (
            "default styles with align-content: end".to_owned(),
            "<button id=b style='display: block; padding: 0; border: 0; font: inherit; height: 20px; align-content: end'><span id=s>ab</span></button>".to_owned(),
        ),
    ])
    .collect();
    let cases: Vec<Case<'_>> = cases
        .iter()
        .map(|(label, body)| -> Case<'_> {
            (
                label,
                body,
                &[("b", 0.0, 0.0, 20.0, 20.0), ("s", 0.0, 5.0, 20.0, 10.0)],
            )
        })
        .collect();
    check_boxes(&cases);
}

/// The room left free is measured inside the padding and border, against a
/// minimum or percentage height too, and the margins of block content, which do
/// not collapse with the button's, count as content.
#[test]
fn a_button_centers_its_content_in_its_content_box() {
    check_boxes(&[
        (
            "min-height",
            &format!(
                "<button id=b style='{BUTTON} min-height: 30px'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 10.0, 20.0, 10.0)],
        ),
        (
            "percentage height",
            &format!(
                "<div style='height: 50px'><button id=b style='{BUTTON} height: 100%'><span id=s>ab</span></button></div>"
            ),
            &[("b", 0.0, 0.0, 20.0, 50.0), ("s", 0.0, 20.0, 20.0, 10.0)],
        ),
        (
            "padding and border",
            &format!(
                "<button id=b style='{BUTTON} height: 30px; padding: 5px; border: 2px solid'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 34.0, 44.0), ("s", 7.0, 17.0, 20.0, 10.0)],
        ),
        (
            "block with a margin",
            &format!(
                "<button id=b style='{BUTTON} height: 30px'><div id=s style='margin-top: 4px'>ab</div></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 12.0, 20.0, 10.0)],
        ),
        (
            "empty block with margins",
            &format!(
                "<button id=b style='{BUTTON} height: 30px'><div id=s style='height: 6px; margin: 3px 0 5px'></div></button>"
            ),
            &[("b", 0.0, 0.0, 0.0, 30.0), ("s", 0.0, 11.0, 0.0, 6.0)],
        ),
        (
            "own margin and a child margin",
            &format!(
                "<div style='width: 100px; height: 100px'><button id=b style='{BUTTON} height: 40px; margin-top: 10px'><div id=s style='margin-top: 7px'>ab</div></button></div>"
            ),
            &[("b", 0.0, 10.0, 20.0, 40.0), ("s", 0.0, 28.5, 20.0, 10.0)],
        ),
        (
            "float",
            &format!(
                "<button id=b style='{BUTTON} height: 30px'><div id=s style='float: left'>ab</div></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 10.0, 20.0, 10.0)],
        ),
        (
            "absolutely positioned box at its static position",
            &format!(
                "<button id=b style='{BUTTON} height: 40px'><span id=s>ab</span><div id=a style='position: absolute'>x</div></button>"
            ),
            &[
                ("b", 0.0, 0.0, 20.0, 40.0),
                ("s", 0.0, 15.0, 20.0, 10.0),
                ("a", 0.0, 25.0, 10.0, 10.0),
            ],
        ),
    ]);
}

/// Content taller than the button stays at its top edge, as `safe center`.
#[test]
fn overflowing_button_content_stays_at_the_top() {
    for (label, style) in [("visible", ""), ("overflow: hidden", "overflow: hidden;")] {
        let actual = boxes(&format!(
            "<button id=b style='{BUTTON} height: 20px; width: 20px; {style}'><span id=s>ab cd ef</span></button>"
        ));
        let button = actual
            .iter()
            .find(|(id, _)| id == "b")
            .map(|(_, rect)| *rect);
        assert!(
            button.is_some_and(|rect| rect
                .iter()
                .zip([0.0, 0.0, 20.0, 20.0])
                .all(|(got, want)| (got - want).abs() <= TOLERANCE)),
            "{label}: #b is {button:?}, expected [0, 0, 20, 20]"
        );
        let first_line = actual
            .iter()
            .find(|(id, _)| id == "s")
            .map(|(_, rect)| rect[1]);
        assert!(
            first_line.is_some_and(|top| top.abs() <= TOLERANCE),
            "{label}: the first line of #s is at {first_line:?}, expected 0"
        );
    }
}

/// A flex or grid button places its items as such a container does: without
/// the UA style sheet's help or button layout's centering, its items stretch
/// or start at the top.
#[test]
fn flex_and_grid_buttons_do_not_center_their_items() {
    let defaults = "padding: 0; border: 0; font: inherit; height: 30px;";
    check_boxes(&[
        (
            "grid",
            &format!(
                "<button id=b style='display: grid; {defaults}'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 0.0, 20.0, 30.0)],
        ),
        (
            "grid with a sized row",
            &format!(
                "<button id=b style='display: grid; grid-template-rows: 10px; {defaults}'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 0.0, 20.0, 10.0)],
        ),
        (
            "wrapping flex",
            &format!(
                "<button id=b style='display: flex; flex-wrap: wrap; {defaults}'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 0.0, 20.0, 30.0)],
        ),
        (
            "wrapping flex with align-items: start",
            &format!(
                "<button id=b style='display: flex; flex-wrap: wrap; align-items: start; {defaults}'><span id=s>ab</span></button>"
            ),
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 0.0, 20.0, 10.0)],
        ),
        (
            "unset flex",
            "<button id=b style='all: unset; display: flex; height: 30px'><span id=s>ab</span></button>",
            &[("b", 0.0, 0.0, 20.0, 30.0), ("s", 0.0, 0.0, 20.0, 30.0)],
        ),
    ]);
}
