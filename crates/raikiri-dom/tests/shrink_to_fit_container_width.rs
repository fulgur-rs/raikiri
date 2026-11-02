//! The width of a flex or grid container that shrinks to fit.
//!
//! The width of a box that is not stretched is its `fit-content` size (CSS
//! Sizing 3 section 3.1.1): the room it is offered, but at least its min-content
//! width and at most its max-content width. That holds for the keyword
//! `width: fit-content`, for a box that a grid or a column flex container does
//! not stretch (an alignment other than `stretch`, or an `auto` margin), and for
//! an absolutely positioned box. The tracks of a grid container are sized
//! against that width, which is a definite length to share among the flexible
//! ones, not the largest max-content size of their items. The text of the items
//! is thirty words of ten characters, 3300px on one line and 100px at its
//! least in Ahem at 10px. The expected border boxes are the ones Chromium lays
//! out for the same markup, in an 800px by 600px viewport without a body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
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

/// Thirty words of ten characters.
fn words() -> String {
    "iiiiiiiiii ".repeat(30)
}

/// The expected border box `(x, y, width, height)` of each element, by `id`.
type Expected<'a> = &'a [(&'a str, [f32; 4])];

/// Check that every `(id, expected box)` of every case is laid out as given.
fn check(cases: Vec<(&str, String, Expected<'_>)>) {
    for (what, body, expected) in cases {
        let actual = boxes(&body);
        for &(id, want) in expected {
            let got = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{what}: no fragment for #{id} in {body}"))
                .1;
            assert!(
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() < 0.01),
                "{what}: #{id} is {got:?}, expected {want:?} for {body}"
            );
        }
    }
}

#[test]
fn a_grid_with_a_fit_content_width_stops_at_the_room_its_parent_offers() {
    // A grid whose `width` is `fit-content`, which its flexible columns are sized against.
    let t = words();
    check(vec![
        (
            "1fr columns with width: fit-content stop at the room",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "minmax(0, 1fr) columns",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr)"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a single 1fr column",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:1fr"><div id=a>{t}</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 100.0]),
                ("a", [0.0, 0.0, 400.0, 100.0]),
            ],
        ),
        (
            "1fr and 2fr columns",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:1fr 2fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 133.328_12, 300.0]),
                ("b", [133.328_12, 0.0, 266.671_88, 300.0]),
            ],
        ),
        (
            "a length and a 1fr column",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:50px 1fr"><div id=a>b</div><div id=b>{t}</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 100.0]),
                ("a", [0.0, 0.0, 50.0, 100.0]),
                ("b", [50.0, 0.0, 350.0, 100.0]),
            ],
        ),
        (
            "a grid that fits the room is as wide as its content",
            r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:1fr 1fr"><div id=a>aaa</div><div id=b>b</div></div></div>"#.to_string(),
            &[
                ("e1", [0.0, 0.0, 60.0, 10.0]),
                ("a", [0.0, 0.0, 30.0, 10.0]),
                ("b", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "words wider than the room make the min-content width",
            r#"<div style="width:400px"><div id=e style="width:fit-content;display:grid;grid-template-columns:1fr 1fr"><div id=a>xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div><div id=b>yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy</div></div></div>"#.to_string(),
            &[
                ("e", [0.0, 0.0, 950.0, 10.0]),
                ("a", [0.0, 0.0, 500.0, 10.0]),
                ("b", [500.0, 0.0, 450.0, 10.0]),
            ],
        ),
        (
            "short words in two columns",
            r#"<div style="width:400px"><div id=e style="width:fit-content;display:grid;grid-template-columns:1fr 1fr"><div id=a>abc abc abc abc abc abc abc abc abc abc </div><div id=b>de de de de de de de de de de </div></div></div>"#.to_string(),
            &[
                ("e", [0.0, 0.0, 400.0, 20.0]),
                ("a", [0.0, 0.0, 200.0, 20.0]),
                ("b", [200.0, 0.0, 200.0, 20.0]),
            ],
        ),
        (
            "a column gap is part of the width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:fit-content;column-gap:30px"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 185.0, 300.0]),
                ("b", [215.0, 0.0, 185.0, 300.0]),
            ],
        ),
        (
            "padding and border are part of the width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:fit-content;padding:0 20px;border:5px solid"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 310.0]),
                ("a", [25.0, 5.0, 175.0, 300.0]),
                ("b", [200.0, 5.0, 175.0, 300.0]),
            ],
        ),
        (
            "a min-width raises the width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:fit-content;min-width:500px"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 500.0, 150.0]),
                ("a", [0.0, 0.0, 250.0, 150.0]),
                ("b", [250.0, 0.0, 250.0, 150.0]),
            ],
        ),
        (
            "a max-width bounds the width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:fit-content;max-width:300px"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 300.0, 300.0]),
                ("a", [0.0, 0.0, 150.0, 300.0]),
                ("b", [150.0, 0.0, 150.0, 300.0]),
            ],
        ),
        (
            "a nested grid in a fit-content grid",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:fit-content"><div id=a style="display:grid;grid-template-columns:1fr 1fr"><div>{t}</div><div>b</div></div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "the box after a fit-content grid",
            format!(
                r#"<div style="width:400px"><div id=e style="width:fit-content;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div><div id=z>after</div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
                ("z", [0.0, 300.0, 400.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn a_grid_that_its_parent_does_not_stretch_fits_the_room_it_is_offered() {
    // A grid that a grid or a column flex container aligns, and an absolutely positioned one.
    let t = words();
    check(vec![
        (
            "a grid item with justify-self: start",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e3 style="justify-self:start;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e3", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with justify-self: center",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;justify-self:center"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with justify-self: end",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;justify-self:end"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with margin: auto",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e2 style="margin:auto;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e2", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with margin-left: auto",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;margin-left:auto"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "padding and border",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;justify-self:start;padding:0 20px;border:5px solid"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 310.0]),
                ("a", [25.0, 5.0, 175.0, 300.0]),
                ("b", [200.0, 5.0, 175.0, 300.0]),
            ],
        ),
        (
            "a column gap",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;justify-self:start;column-gap:30px"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 185.0, 300.0]),
                ("b", [215.0, 0.0, 185.0, 300.0]),
            ],
        ),
        (
            "an item of a column flex container aligned to the start",
            format!(
                r#"<div style="display:flex;flex-direction:column;align-items:flex-start;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an item of a column flex container aligned to the center",
            format!(
                r#"<div style="display:flex;flex-direction:column;align-items:center;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an item with align-self: flex-end",
            format!(
                r#"<div style="display:flex;flex-direction:column;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;align-self:flex-end"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an absolutely positioned grid",
            format!(
                r#"<div style="position:relative;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;position:absolute;left:0"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an absolutely positioned grid with width: fit-content",
            format!(
                r#"<div style="position:relative;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;position:absolute;left:0;width:fit-content"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
    ]);
}

#[test]
fn a_flex_container_that_shrinks_to_fit_stops_at_the_room_its_parent_offers() {
    // A flex container in the same places, whose items shrink with it.
    let t = words();
    check(vec![
        (
            "width: fit-content",
            format!(
                r#"<div style="width:400px"><div id=e style="display:flex;width:fit-content"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with justify-self: start",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:flex;justify-self:start"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with justify-self: center",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:flex;justify-self:center"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid item with margin: auto",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:flex;margin:auto"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an item of a column flex container aligned to the start",
            format!(
                r#"<div style="display:flex;flex-direction:column;align-items:flex-start;width:400px"><div id=e style="display:flex;"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an item of a column flex container aligned to the center",
            format!(
                r#"<div style="display:flex;flex-direction:column;align-items:center;width:400px"><div id=e style="display:flex;"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an item of a column flex container with margin: 0 auto",
            format!(
                r#"<div style="display:flex;flex-direction:column;width:400px"><div id=e style="display:flex;margin:0 auto"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "items that keep their flex basis",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:flex;justify-self:start"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 100.0]),
                ("a", [0.0, 0.0, 390.0, 100.0]),
                ("b", [390.0, 0.0, 10.0, 100.0]),
            ],
        ),
        (
            "the same in a column flex container",
            format!(
                r#"<div style="display:flex;flex-direction:column;align-items:flex-start;width:400px"><div id=e style="display:flex;"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 100.0]),
                ("a", [0.0, 0.0, 390.0, 100.0]),
                ("b", [390.0, 0.0, 10.0, 100.0]),
            ],
        ),
        (
            "a wrapping flex container wraps at the room",
            format!(
                r#"<div style="display:grid;width:400px"><div id=e style="display:flex;justify-self:start;flex-wrap:wrap"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 110.0]),
                ("a", [0.0, 0.0, 400.0, 100.0]),
                ("b", [0.0, 100.0, 10.0, 10.0]),
            ],
        ),
        (
            "a short flex container keeps its content width",
            r#"<div style="display:grid;width:400px"><div id=e style="display:flex;justify-self:start"><div id=a>aaa</div><div id=b>bb</div></div></div>"#.to_string(),
            &[
                ("e", [0.0, 0.0, 50.0, 10.0]),
                ("a", [0.0, 0.0, 30.0, 10.0]),
                ("b", [30.0, 0.0, 20.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn boxes_that_are_stretched_or_sized_by_their_content_are_left_alone() {
    // What a parent stretches, a float, an inline grid and the keywords `min-content` and `max-content`.
    let t = words();
    check(vec![
        (
            "a stretched flex item of a column container",
            format!(
                r#"<div style="display:flex;flex-direction:column;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a block-level grid takes the width of its container",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid at its min-content width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:min-content"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 110.0, 300.0]),
                ("a", [0.0, 0.0, 100.0, 300.0]),
                ("b", [100.0, 0.0, 10.0, 300.0]),
            ],
        ),
        (
            "a grid at its max-content width",
            format!(
                r#"<div style="width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;width:max-content"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 6580.0, 10.0]),
                ("a", [0.0, 0.0, 3290.0, 10.0]),
                ("b", [3290.0, 0.0, 3290.0, 10.0]),
            ],
        ),
        (
            "a floated grid",
            format!(
                r#"<div style="width:400px"><div id=c1 style="float:left;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("c1", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "an inline grid",
            format!(
                r#"<div style="width:400px"><div id=c2 style="display:inline-grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("c2", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a flex container that is a flex item with margin: auto",
            format!(
                r#"<div style="display:flex;width:400px"><div id=c3 style="margin:auto;display:grid;grid-template-columns:1fr 1fr"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("c3", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a grid with width: fit-content and auto columns",
            format!(
                r#"<div style="width:400px"><div id=e1 style="width:fit-content;display:grid;grid-template-columns:auto auto"><div id=a>{t}</div><div id=b>b</div></div></div>"#
            ),
            &[
                ("e1", [0.0, 0.0, 400.0, 100.0]),
                ("a", [0.0, 0.0, 390.0, 100.0]),
                ("b", [390.0, 0.0, 10.0, 100.0]),
            ],
        ),
        (
            "a flex container that is a block",
            format!(
                r#"<div style="width:400px"><div id=e style="display:flex;"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
        (
            "a floated flex container",
            format!(
                r#"<div style="width:400px"><div id=e style="display:flex;float:left"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div></div>"#
            ),
            &[
                ("e", [0.0, 0.0, 400.0, 300.0]),
                ("a", [0.0, 0.0, 200.0, 300.0]),
                ("b", [200.0, 0.0, 200.0, 300.0]),
            ],
        ),
    ]);
}

#[test]
fn a_container_that_is_a_flex_item_of_a_row_container_shrinks_with_the_line() {
    // A flex item takes the size the line gives it, whatever its own `fit-content`
    // width is: its base size is its max-content width, `flex-basis: content`
    // measures it the same way, and the flex factors then resize it.
    let t = words();
    let grid = |style: &str| {
        format!(
            r#"<div id=e style="display:grid;grid-template-columns:1fr 1fr{style}"><div id=a>{t}</div><div id=b>b</div></div>"#
        )
    };
    let flex = |style: &str| {
        format!(
            r#"<div id=e style="display:flex{style}"><div id=a style="flex:1">{t}</div><div id=b style="flex:1">b</div></div>"#
        )
    };
    let shrunk: &[(&str, [f32; 4])] = &[
        ("e", [0.0, 0.0, 400.0, 300.0]),
        ("a", [0.0, 0.0, 200.0, 300.0]),
        ("b", [200.0, 0.0, 200.0, 300.0]),
    ];
    check(vec![
        (
            "a grid that shrinks to the line",
            format!(r#"<div style="display:flex;width:400px">{}</div>"#, grid("")),
            shrunk,
        ),
        (
            "a flex container that shrinks to the line",
            format!(r#"<div style="display:flex;width:400px">{}</div>"#, flex("")),
            shrunk,
        ),
        (
            "a grid with flex-basis: content",
            format!(
                r#"<div style="display:flex;width:400px">{}</div>"#,
                grid(";flex-basis:content")
            ),
            shrunk,
        ),
        (
            "a flex container with flex-basis: content",
            format!(
                r#"<div style="display:flex;width:400px">{}</div>"#,
                flex(";flex-basis:content")
            ),
            shrunk,
        ),
        (
            "a grid in a wrapping line",
            format!(
                r#"<div style="display:flex;flex-wrap:wrap;width:400px">{}</div>"#,
                grid("")
            ),
            shrunk,
        ),
        (
            "a grid that cannot shrink keeps its max-content width",
            format!(
                r#"<div style="display:flex;width:400px">{}</div>"#,
                grid(";flex:0 0 auto")
            ),
            &[
                ("e", [0.0, 0.0, 6580.0, 10.0]),
                ("a", [0.0, 0.0, 3290.0, 10.0]),
                ("b", [3290.0, 0.0, 3290.0, 10.0]),
            ],
        ),
        (
            "a grid that shares the line with a box",
            format!(
                r#"<div style="display:flex;width:400px">{}<div id=z>zz</div></div>"#,
                grid("")
            ),
            &[
                ("e", [0.0, 0.0, 380.0, 300.0]),
                ("a", [0.0, 0.0, 190.0, 300.0]),
                ("b", [190.0, 0.0, 190.0, 300.0]),
                ("z", [380.0, 0.0, 20.0, 300.0]),
            ],
        ),
        (
            "a short grid keeps its content width",
            r#"<div style="display:flex;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr"><div id=a>aaa</div><div id=b>b</div></div></div>"#
                .to_string(),
            &[
                ("e", [0.0, 0.0, 60.0, 10.0]),
                ("a", [0.0, 0.0, 30.0, 10.0]),
                ("b", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "a short grid that grows fills the line",
            r#"<div style="display:flex;width:400px"><div id=e style="display:grid;grid-template-columns:1fr 1fr;flex-grow:1"><div id=a>aaa</div><div id=b>b</div></div></div>"#
                .to_string(),
            &[
                ("e", [0.0, 0.0, 400.0, 10.0]),
                ("a", [0.0, 0.0, 200.0, 10.0]),
                ("b", [200.0, 0.0, 200.0, 10.0]),
            ],
        ),
    ]);
}
