//! A forced column break (`break-before: column` or `break-after: column`)
//! between the blocks of a multicol container, inside a wrapper or not,
//! starts a column. Balanced columns are as tall as the least height that
//! holds the content with those breaks, and columns of a definite height
//! start a column at each break too (CSS Multi-column 1, sections 7.1 and
//! 8; CSS Fragmentation 3, 3.1).
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body and 10px
//! Ahem text. `o` is the container; the wrapper `c` is not checked. A box that
//! breaks lists one border box per column it is in, in column order.

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

/// One markup and the boxes Chromium gives it.
struct Case {
    name: &'static str,
    body: &'static str,
    expected: &'static [(&'static str, Rect)],
}

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

#[test]
fn a_forced_break_starts_a_balanced_column() {
    let mut failures = Vec::new();
    for case in CASES {
        let actual = border_boxes(case.body);
        // Each box has as many pieces as Chromium gives it, no more.
        for &(id, _) in case.expected {
            let expected = case.expected.iter().filter(|(name, _)| *name == id).count();
            let found = actual.iter().filter(|(name, _)| name == id).count();
            if found != expected {
                failures.push(format!(
                    "{}: #{id} has {found} pieces, expected {expected}",
                    case.name
                ));
            }
        }
        for (index, &(id, (x, y, width, height))) in case.expected.iter().enumerate() {
            // The nth box of `id` in the expectations is its nth piece.
            let nth = case.expected[..index]
                .iter()
                .filter(|(name, _)| *name == id)
                .count();
            let found = actual
                .iter()
                .filter(|(name, _)| name == id)
                .nth(nth)
                .map(|(_, r)| *r);
            let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
            if !found.is_some_and(|f| {
                close(f.0, x) && close(f.1, y) && close(f.2, width) && close(f.3, height)
            }) {
                failures.push(format!(
                    "{}: #{id} piece {nth} is {found:?}, expected {:?}",
                    case.name,
                    (x, y, width, height)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const CASES: &[Case] = &[
    Case {
        name: "break_before_in_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa</div><div id=b style='break-before:column'>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "break_after_in_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa</div><div id=b style='break-after:column'>bb</div><div id=d>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_padding",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c style='padding:5px 0'><div id=a>aa</div><div id=b style='break-before:column'>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 15.0)),
            ("a", (0.0, 5.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_background",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c style='background:red'><div id=a>aa</div><div id=b style='break-before:column'>bb</div><div id=d>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_between_blocks",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=p>pp</div><div id=c><div id=a>aa</div><div id=b style='break-before:column'>bb</div></div><div id=q>qq</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("p", (0.0, 0.0, 100.0, 10.0)),
            ("a", (0.0, 10.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("q", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "nested_wrappers",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=c2><div id=a>aa</div><div id=b style='break-before:column'>bb</div></div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "margins_at_forced_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a style='margin-bottom:10px'>aa</div><div id=b style='break-before:column;margin-top:10px'>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "uneven_blocks_around_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa<br>aa<br>aa</div><div id=b style='break-before:column'>bb</div><div id=d>dd<br>dd<br>dd<br>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 20.0)),
            ("d", (200.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "flow_root_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c style='display:flow-root'><div id=a>aa</div><div id=b style='break-before:column'>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "break_after_first_block",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a style='break-after:column'>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "break_with_no_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa<br>aa<br>aa</div><div id=b style='break-before:column'>bb</div><div id=d>dd<br>dd<br>dd<br>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 20.0)),
            ("d", (200.0, 0.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "definite_height",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:95px\"><div id=a>x<br>x<br>x</div><div id=b style='break-before:column'>x<br>x<br>x</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 95.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "definite_height_fill_auto",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:95px;column-fill:auto\"><div id=a>x<br>x<br>x</div><div id=b style='break-before:column'>x<br>x<br>x</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 95.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "definite_height_wrapper_with_height",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:95px\"><div id=c style='height:150px'><div id=a>x<br>x<br>x</div><div id=b style='break-before:column'>x<br>x<br>x</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 95.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "definite_height_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:95px\"><div id=c><div id=a>x<br>x<br>x</div><div id=b style='break-before:column'>x<br>x<br>x</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 95.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("b", (100.0, 0.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "block_breaks_before_forced_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa<br>aa<br>aa<br>aa</div><div id=b style='break-before:column'>bb</div><div id=d>dd<br>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 30.0)),
            ("a", (100.0, 0.0, 100.0, 20.0)),
            ("b", (200.0, 0.0, 100.0, 10.0)),
            ("d", (200.0, 10.0, 100.0, 20.0)),
        ],
    },
    Case {
        name: "avoiding_block_after_forced_break_keeps_margin",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-before:column;break-inside:avoid;margin-top:20px'>b<br>b<br>b</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 50.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 20.0, 100.0, 30.0)),
        ],
    },
    Case {
        name: "max_height_overflow_column",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;max-height:20px\"><div id=a>aa</div><div id=b style='break-before:column'>bb<br>bb<br>bb<br>bb<br>bb</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 20.0)),
            ("b", (200.0, 0.0, 100.0, 20.0)),
            ("b", (300.0, 0.0, 100.0, 10.0)),
        ],
    },
];
