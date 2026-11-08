//! A grid container in a vertical writing mode in a horizontal flow fits its
//! lines in the height that flow offers it (CSS Writing Modes 4 §7.3.1), and
//! its width follows from those lines. That holds while its parent measures
//! it, too: a `min-content` or `max-content` width, a float, an absolutely
//! positioned box, an inline-block, a row flex container and a table cell all
//! size to the columns the grid fills in the page area, or in the definite
//! height of the parent, not to a single unbroken column.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin. A box taller than the page
//! is split over pages here; its height is the sum of its fragments.

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

/// A case: its name, its markup with `{W}` for [`words`], and the border boxes
/// by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// A hundred two-letter words: 20 to a 600px column of 10px glyphs.
fn words() -> String {
    vec!["aa"; 100].join(" ")
}

/// The border boxes of the elements of `body` with an `id`: where the first
/// fragment of each lies and how wide it is, and the heights of its fragments
/// on all pages summed.
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
    let mut boxes: Vec<(String, Rect)> = Vec::new();
    for page in result.pages() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            match boxes.iter_mut().find(|(box_id, _)| box_id == id) {
                Some((_, found)) => found.3 += rect.height,
                None => boxes.push((id.to_owned(), (rect.x, rect.y, rect.width, rect.height))),
            }
        }
    }
    boxes
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(name: &str, body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: #{id} is {found:?}, Chromium has {:?}",
            (x, y, width, height)
        );
    }
}

/// The vertical grid container `o` in the horizontal parent `c`.
const CASES: &[Case] = &[
    (
        "min_grid",
        r#"<div id=c style="width:min-content"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "max_grid",
        r#"<div id=c style="width:max-content"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "fl_grid",
        r#"<div id=c style="float:left"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "ig_min",
        r#"<div id=c style="width:min-content"><div id=o style="display:inline-grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "g_lr_min",
        r#"<div id=c style="width:min-content"><div id=o style="display:grid;writing-mode:vertical-lr"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "g_abs",
        r#"<div id=c style="position:absolute"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "g_rowflex",
        r#"<div id=c style="display:flex"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 800.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "g_ib",
        r#"<div id=c style="display:inline-block"><div id=o style="display:grid;writing-mode:vertical-rl;grid-template-columns:auto auto"><div id=i>{W}</div><div id=j>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 600.0)),
            ("o", (0.0, 0.0, 100.0, 600.0)),
            ("i", (0.0, 0.0, 100.0, 300.0)),
            ("j", (0.0, 300.0, 100.0, 300.0)),
        ],
    ),
    (
        "g_maxh",
        r#"<div id=c style="width:min-content;height:200px"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></div>"#,
        &[
            ("c", (0.0, 0.0, 150.0, 200.0)),
            ("o", (0.0, 0.0, 150.0, 200.0)),
            ("i", (0.0, 0.0, 150.0, 200.0)),
        ],
    ),
    (
        "g",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:grid;writing-mode:vertical-rl;"><div id=i>{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "gh300",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="display:grid;writing-mode:vertical-rl;"><div id=i>{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
            ("i", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "gig",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:inline-grid;writing-mode:vertical-rl"><div id=i>{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
            ("i", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "gtwo",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:grid;writing-mode:vertical-rl"><div id=i>{W}</div><div id=j>{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 600.0)),
            ("o", (0.0, 0.0, 100.0, 600.0)),
            ("i", (50.0, 0.0, 50.0, 600.0)),
            ("j", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
];

#[test]
fn a_vertical_grid_container_is_as_wide_as_the_columns_it_fills() {
    let words = words();
    for &(name, body, expected) in CASES {
        check(name, &body.replace("{W}", &words), expected);
    }
}
