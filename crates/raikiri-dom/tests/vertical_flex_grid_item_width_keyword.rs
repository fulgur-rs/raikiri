//! A flex or grid item in a vertical writing mode whose `width` is
//! `max-content` or `fit-content` takes the width its content gives it, as
//! one with a `min-content` width already did: its physical width is its
//! block size, which the keyword sizes to the content instead of leaving the
//! item to stretch to its grid area or to the cross size of a column flex
//! line (CSS Sizing 3 §3.1.1, CSS Flexbox 1 §9.4: only an `auto` cross size
//! stretches). A wrapping vertical column flex container with such a width
//! keeps its items on the line the keyword gives it. An item with an
//! `aspect-ratio` takes the content size the keyword names from its ratio, in
//! a column flex container as much as along a row (CSS Sizing 4 §5.1).
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin and border-box divs. The
//! guard cases matched before and pin what already worked: row flex parents,
//! `min-content` widths and `auto` widths that stretch.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// A case: its name, its markup, and the border boxes by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check_case(&(name, body, expected): &Case) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: {body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn a_vertical_wrapping_column_flex_item_keeps_its_keyword_width() {
    ISSUE.iter().for_each(check_case);
}

#[test]
fn vertical_flex_and_grid_items_take_their_block_size_from_the_keyword() {
    BLOCK.iter().for_each(check_case);
}

#[test]
fn vertical_paragraph_items_take_their_block_size_from_the_keyword() {
    PARA.iter().for_each(check_case);
}

#[test]
fn vertical_ratio_items_with_keyword_widths_follow_their_ratio() {
    RATIO.iter().for_each(check_case);
}

#[test]
fn row_flex_min_content_and_stretching_items_keep_their_sizes() {
    GUARD.iter().for_each(check_case);
}

const ISSUE: &[Case] = &[
    (
        "k01",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k02",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k09",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k10",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k33",
        r#"<div id=p style="display:grid;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k34",
        r#"<div id=p style="display:grid;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k49",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k50",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
];

const BLOCK: &[Case] = &[
    (
        "g01",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g02",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g04",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g05",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g10",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g11",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g13",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g14",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g19",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g20",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g22",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g23",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g28",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g29",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g31",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g32",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g37",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g38",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g40",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g41",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g46",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g47",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g49",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g50",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h005",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 300.0)),
            ("a", (40.0, 0.0, 10.0, 300.0)),
            ("b", (30.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h006",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 300.0)),
            ("a", (18.0, 0.0, 10.0, 300.0)),
            ("b", (8.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h007",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 0.0, 20.0, 30.0)),
        ],
    ),
    (
        "h008",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 300.0)),
            ("a", (10.0, 0.0, 10.0, 300.0)),
            ("b", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h014",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 60.0)),
            ("a", (40.0, 0.0, 10.0, 60.0)),
            ("b", (30.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h015",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 60.0)),
            ("a", (18.0, 0.0, 10.0, 60.0)),
            ("b", (8.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h016",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 0.0, 20.0, 30.0)),
        ],
    ),
    (
        "h017",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h032",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (30.0, 0.0, 20.0, 40.0)),
            ("b", (10.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h033",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h034",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 0.0, 20.0, 30.0)),
        ],
    ),
    (
        "h035",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h050",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (30.0, 0.0, 20.0, 40.0)),
            ("b", (10.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h051",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h052",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 0.0, 20.0, 30.0)),
        ],
    ),
    (
        "h053",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h058",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;max-width:15px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
            ("a", (0.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h059",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 300.0)),
            ("a", (0.0, 0.0, 50.0, 60.0)),
            ("b", (0.0, 60.0, 50.0, 60.0)),
        ],
    ),
    (
        "h060",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 23.0, 300.0)),
            ("a", (8.0, 0.0, 10.0, 60.0)),
            ("b", (8.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h061",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 30.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h062",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
            ("a", (0.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h067",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;max-width:15px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 120.0)),
            ("a", (0.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h068",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 120.0)),
            ("a", (0.0, 0.0, 50.0, 60.0)),
            ("b", (0.0, 60.0, 50.0, 60.0)),
        ],
    ),
    (
        "h069",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 23.0, 120.0)),
            ("a", (8.0, 0.0, 10.0, 60.0)),
            ("b", (8.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h070",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 30.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h071",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 120.0)),
            ("a", (0.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h086",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 70.0)),
            ("a", (0.0, 0.0, 50.0, 30.0)),
            ("b", (0.0, 30.0, 50.0, 40.0)),
        ],
    ),
    (
        "h087",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 33.0, 70.0)),
            ("a", (8.0, 0.0, 20.0, 30.0)),
            ("b", (8.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h088",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 20.0, 30.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h089",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 20.0, 70.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h104",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 70.0)),
            ("a", (0.0, 0.0, 50.0, 30.0)),
            ("b", (0.0, 30.0, 50.0, 40.0)),
        ],
    ),
    (
        "h105",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 33.0, 70.0)),
            ("a", (8.0, 0.0, 20.0, 30.0)),
            ("b", (8.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h106",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 20.0, 30.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h107",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 20.0, 70.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "h113",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 300.0)),
            ("a", (25.0, 0.0, 25.0, 300.0)),
            ("b", (0.0, 0.0, 25.0, 300.0)),
        ],
    ),
    (
        "h114",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 300.0)),
            ("a", (18.0, 0.0, 10.0, 300.0)),
            ("b", (8.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h115",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h116",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 300.0)),
            ("a", (10.0, 0.0, 10.0, 300.0)),
            ("b", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h122",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 60.0)),
            ("a", (25.0, 0.0, 25.0, 60.0)),
            ("b", (0.0, 0.0, 25.0, 60.0)),
        ],
    ),
    (
        "h123",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 60.0)),
            ("a", (18.0, 0.0, 10.0, 60.0)),
            ("b", (8.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h124",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h125",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h140",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (25.0, 0.0, 25.0, 40.0)),
            ("b", (0.0, 0.0, 25.0, 40.0)),
        ],
    ),
    (
        "h141",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h142",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h143",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h158",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (25.0, 0.0, 25.0, 40.0)),
            ("b", (0.0, 0.0, 25.0, 40.0)),
        ],
    ),
    (
        "h159",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h160",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h161",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:grid;writing-mode:vertical-rl;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h166",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;max-width:15px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
            ("a", (0.0, 0.0, 10.0, 150.0)),
            ("b", (0.0, 150.0, 10.0, 150.0)),
        ],
    ),
    (
        "h167",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 300.0)),
            ("a", (40.0, 0.0, 10.0, 300.0)),
            ("b", (30.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h168",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 300.0)),
            ("a", (18.0, 0.0, 10.0, 300.0)),
            ("b", (8.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h169",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h170",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 300.0)),
            ("a", (10.0, 0.0, 10.0, 300.0)),
            ("b", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "h175",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;max-width:15px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 120.0)),
            ("a", (0.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 60.0, 10.0, 60.0)),
        ],
    ),
    (
        "h176",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 60.0)),
            ("a", (40.0, 0.0, 10.0, 60.0)),
            ("b", (30.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h177",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 33.0, 60.0)),
            ("a", (18.0, 0.0, 10.0, 60.0)),
            ("b", (8.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h178",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h179",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "h194",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (30.0, 0.0, 20.0, 40.0)),
            ("b", (10.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h195",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h196",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h197",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h212",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;min-width:50px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 50.0, 40.0)),
            ("a", (30.0, 0.0, 20.0, 40.0)),
            ("b", (10.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h213",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;padding:0 5px;border-left:3px solid"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 53.0, 40.0)),
            ("a", (28.0, 0.0, 20.0, 40.0)),
            ("b", (8.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h214",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content;height:30px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 30.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
    (
        "h215",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content;align-self:stretch;justify-self:stretch"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
            ("a", (20.0, 0.0, 20.0, 40.0)),
            ("b", (0.0, 0.0, 20.0, 40.0)),
        ],
    ),
];

const PARA: &[Case] = &[
    (
        "i004",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i005",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i007",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i008",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i012",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i013",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i015",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i016",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i028",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i029",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i031",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i032",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i044",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i045",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i047",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i048",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i060",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i061",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i063",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i064",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i068",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i069",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i071",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i084",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i085",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i087",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i088",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i100",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i101",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i103",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i104",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:flex;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i116",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i117",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 300.0)),
        ],
    ),
    (
        "i119",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i120",
        r#"<div id=p style="display:grid;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i124",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i125",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i127",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i128",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px;height:300px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i140",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i141",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i143",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 10.0, 130.0)),
        ],
    ),
    (
        "i144",
        r#"<div id=p style="display:grid;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i156",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i157",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i159",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:max-content;height:max-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
    (
        "i160",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px;height:40px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:fit-content;height:min-content">xx xxx xxxx x</div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 40.0, 40.0)),
        ],
    ),
];

const GUARD: &[Case] = &[
    (
        "k03",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k04",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 10.0)),
            ("c", (0.0, 0.0, 400.0, 10.0)),
            ("a", (320.0, 0.0, 80.0, 30.0)),
            ("b", (270.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k17",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k18",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k19",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k20",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k21",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;flex:0 0 auto;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k22",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;flex:0 1 auto"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k23",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;justify-self:start"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k24",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;align-self:start"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k35",
        r#"<div id=p style="display:grid;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k36",
        r#"<div id=p style="display:grid;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k41",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:max-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k42",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:fit-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k43",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k44",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k45",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;flex:0 0 auto;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k46",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;flex:0 1 auto"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k47",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;justify-self:start"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k48",
        r#"<div id=p style="display:flex;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;align-self:start"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 20.0)),
            ("c", (0.0, 0.0, 130.0, 20.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k51",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("c", (0.0, 0.0, 130.0, 10.0)),
            ("a", (50.0, 0.0, 80.0, 30.0)),
            ("b", (0.0, 0.0, 50.0, 30.0)),
        ],
    ),
    (
        "k52",
        r#"<div id=p style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;"><div id=a style="writing-mode:horizontal-tb;flex:0 0 80px">xxxx xxxx xxxx</div><div id=b style="writing-mode:horizontal-tb;flex:0 0 50px">xx xx xx xx</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 100.0, 40.0)),
            ("c", (0.0, 0.0, 100.0, 40.0)),
            ("a", (20.0, 0.0, 80.0, 30.0)),
            ("b", (50.0, 30.0, 50.0, 20.0)),
        ],
    ),
    (
        "g03",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g06",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g07",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="writing-mode:vertical-rl;width:max-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g08",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="writing-mode:vertical-rl;width:fit-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g09",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g12",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g15",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g18",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (0.0, 0.0, 20.0, 30.0)),
            ("b", (0.0, 30.0, 20.0, 40.0)),
        ],
    ),
    (
        "g21",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g24",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g30",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g33",
        r#"<div id=p style="display:flex;flex-direction:column;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g36",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:grid;writing-mode:vertical-rl;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g45",
        r#"<div id=p style="display:flex;width:400px"><div id=c style="display:flex;flex-direction:column;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
    (
        "g48",
        r#"<div id=p style="display:grid;width:400px"><div id=c style="display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:min-content;height:60px"><div id=a>xx xxx</div><div id=b>xxxx x</div></div></div>"#,
        &[
            ("p", (0.0, 0.0, 400.0, 60.0)),
            ("c", (0.0, 0.0, 20.0, 60.0)),
            ("a", (10.0, 0.0, 10.0, 60.0)),
            ("b", (0.0, 0.0, 10.0, 60.0)),
        ],
    ),
];

const RATIO: &[Case] = &[
    (
        "r009",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r010",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r011",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r012",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r029",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r030",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r031",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r032",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r041",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r042",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r043",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r044",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r045",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r046",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r047",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r048",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r049",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r050",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r053",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r055",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r069",
        r#"<div id=p style="display:grid;width:500px;justify-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r070",
        r#"<div id=p style="display:grid;width:500px;justify-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r094",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 30.0)),
            ("c", (0.0, 0.0, 30.0, 30.0)),
        ],
    ),
    (
        "r101",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r102",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r103",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r104",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r105",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r106",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r107",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r108",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r109",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r110",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r111",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r112",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r113",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r114",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 30.0)),
            ("c", (0.0, 0.0, 30.0, 30.0)),
        ],
    ),
    (
        "r115",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r116",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 30.0)),
            ("c", (0.0, 0.0, 30.0, 30.0)),
        ],
    ),
    (
        "r117",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r118",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r119",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r120",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r001",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r002",
        r#"<div id=p style="display:flex;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r021",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r022",
        r#"<div id=p style="display:flex;width:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r057",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r059",
        r#"<div id=p style="display:grid;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r061",
        r#"<div id=p style="display:grid;width:500px;justify-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r096",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 30.0)),
            ("c", (0.0, 0.0, 30.0, 30.0)),
        ],
    ),
    (
        "r097",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r099",
        r#"<div id=p style="display:flex;flex-direction:column;width:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "r121",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r122",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 200.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "r125",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r129",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r133",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content;max-height:30px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 20.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "r137",
        r#"<div id=p style="display:block;width:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 15.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v041",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v042",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v043",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v044",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v045",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v046",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v047",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v048",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v049",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v050",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v051",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v052",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v053",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v054",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v055",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v056",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v061",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v062",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v063",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v064",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v065",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v066",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v067",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v068",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v069",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v070",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v071",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v072",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v073",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v074",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v075",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v076",
        r#"<div id=p style="display:flex;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:max-content;max-width:15px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 15.0, 500.0)),
            ("c", (0.0, 0.0, 15.0, 15.0)),
        ],
    ),
    (
        "v109",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v110",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v111",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 20.0, 500.0)),
            ("c", (0.0, 0.0, 20.0, 20.0)),
        ],
    ),
    (
        "v112",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 200.0, 500.0)),
            ("c", (0.0, 0.0, 200.0, 200.0)),
        ],
    ),
    (
        "v001",
        r#"<div id=p style="display:grid;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v009",
        r#"<div id=p style="display:grid;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v017",
        r#"<div id=p style="display:grid;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:auto">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v019",
        r#"<div id=p style="display:grid;writing-mode:vertical-rl;height:500px"><div id=c style="display:flex;aspect-ratio:1;writing-mode:vertical-rl;width:auto">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v029",
        r#"<div id=p style="display:grid;writing-mode:vertical-rl;height:500px;align-items:start"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v081",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v089",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v097",
        r#"<div id=p style="display:flex;flex-direction:column;writing-mode:vertical-rl;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:auto">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v121",
        r#"<div id=p style="display:flex;flex-direction:column-reverse;writing-mode:vertical-lr;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:fit-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
    (
        "v129",
        r#"<div id=p style="display:flex;flex-direction:column-reverse;writing-mode:vertical-lr;height:500px"><div id=c style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content">XX</div></div>"#,
        &[
            ("p", (0.0, 0.0, 500.0, 500.0)),
            ("c", (0.0, 0.0, 500.0, 500.0)),
        ],
    ),
];
