//! A vertical item of a horizontal row flex container or grid container with
//! an `aspect-ratio` and an `auto` width contributes the width its ratio gives
//! for the height its content settles, held to the bounds its width carries
//! over through the ratio (CSS Sizing 4 §5.1), as a vertical inline-block
//! does: `aspect-ratio: 1; max-width: 50px` over a 200px column of text makes
//! a 50px square item, not one as wide as its column. The flex and grid
//! algorithms take that width as the item's content size. A vertical flex or
//! grid container item measured for its content leaves its stated sizes out,
//! so the ratio gives its width there as well.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin. Some cases matched before
//! and guard the parts that already worked.

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
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
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
fn a_vertical_ratio_item_contributes_the_width_its_ratio_gives() {
    CASES.iter().for_each(check_case);
}

const CASES: &[Case] = &[
    (
        "f01",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f02",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "f03",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XX</div></div>"#,
        &[("b", (0.0, 0.0, 20.0, 20.0))],
    ),
    (
        "f04",
        r#"<div style="display:flex;width:500px;align-items:start"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f05",
        r#"<div style="display:flex;width:500px;align-items:start"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "f06",
        r#"<div style="display:flex;width:500px;height:100px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 100.0, 100.0))],
    ),
    (
        "f07",
        r#"<div style="display:flex;width:500px;height:100px;align-items:start"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "f08",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:2;writing-mode:vertical-rl;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 30.0))],
    ),
    (
        "f09",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;height:40px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 40.0, 40.0))],
    ),
    (
        "f10",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;padding:5px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 60.0))],
    ),
    (
        "f12",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;display:flex">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f15",
        r#"<div style="display:grid;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f16",
        r#"<div style="display:grid;width:500px;justify-items:start"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f17",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">AAAA BBBB CCCC DDDD EEEE FFFF</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "f18",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;min-width:80px">XX</div></div>"#,
        &[("b", (0.0, 0.0, 80.0, 80.0))],
    ),
    (
        "f19",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;flex-basis:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "f20",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;overflow:hidden">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g02",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;flex-grow:1">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g03",
        r#"<div style="display:flex;width:100px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div><div id=c style="width:80px;height:10px"></div></div>"#,
        &[
            ("b", (0.0, 0.0, 200.0, 200.0)),
            ("c", (200.0, 0.0, 0.0, 10.0)),
        ],
    ),
    (
        "g04",
        r#"<div style="display:flex;width:500px;flex-wrap:wrap"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div><div id=c style="width:10px;height:10px"></div></div>"#,
        &[
            ("b", (0.0, 0.0, 50.0, 50.0)),
            ("c", (50.0, 0.0, 10.0, 10.0)),
        ],
    ),
    (
        "g05",
        r#"<div style="display:flex;width:500px;flex-direction:row-reverse"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (450.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g06",
        r#"<div style="display:inline-flex"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g08",
        r#"<div style="display:grid;width:500px;grid-template-columns:min-content 1fr"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XX</div><div id=c>A</div></div>"#,
        &[
            ("b", (0.0, 0.0, 20.0, 20.0)),
            ("c", (20.0, 0.0, 480.0, 20.0)),
        ],
    ),
    (
        "g09",
        r#"<div style="display:grid;width:500px;grid-template-columns:100px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XX</div></div>"#,
        &[("b", (0.0, 0.0, 100.0, 100.0))],
    ),
    (
        "g10",
        r#"<div style="display:grid;width:500px;grid-template-columns:100px;justify-items:start"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XX</div></div>"#,
        &[("b", (0.0, 0.0, 20.0, 20.0))],
    ),
    (
        "g11",
        r#"<div style="display:grid;width:500px;justify-items:center"><div id=b style="aspect-ratio:2;writing-mode:vertical-rl;max-height:30px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (220.0, 0.0, 60.0, 30.0))],
    ),
    (
        "g12",
        r#"<div style="display:flex;width:500px;align-items:center;height:300px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 125.0, 50.0, 50.0))],
    ),
    (
        "g13",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;margin:5px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (5.0, 5.0, 50.0, 50.0))],
    ),
    (
        "g14",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;display:grid">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g16",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;box-sizing:border-box;padding:5px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g17",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;contain:size;contain-intrinsic-size:30px 40px"></div></div>"#,
        &[("b", (0.0, 0.0, 40.0, 40.0))],
    ),
    (
        "g18",
        r#"<div style="display:flex;width:500px"><div id=b style="aspect-ratio:1;writing-mode:horizontal-tb;max-height:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "g20",
        r#"<div style="display:grid;width:500px;writing-mode:vertical-rl;height:300px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:horizontal-tb">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (450.0, 0.0, 50.0, 50.0))],
    ),
];
