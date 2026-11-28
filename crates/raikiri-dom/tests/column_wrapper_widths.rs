//! A block wrapper inside a multicol container breaks between the columns with
//! the blocks it holds whatever width it states: a length, a percentage of the
//! column, `fit-content`, a minimum or a maximum width, or `auto` margins
//! (CSS Multi-column 1, section 8; CSS Fragmentation 3, 4.4). Each piece of it
//! in a column is as wide as the box.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body and 10px
//! Ahem text. `o` is the container; the wrapper `c` is not checked.

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
fn a_wrapper_of_any_width_breaks_between_columns() {
    let mut failures = Vec::new();
    for case in CASES {
        let actual = border_boxes(case.body);
        for &(id, (x, y, width, height)) in case.expected {
            let found = actual.iter().find(|(name, _)| name == id).map(|(_, r)| *r);
            let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
            if !found.is_some_and(|f| {
                close(f.0, x) && close(f.1, y) && close(f.2, width) && close(f.3, height)
            }) {
                failures.push(format!(
                    "{}: #{id} is {found:?}, expected {:?}",
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
        name: "definite_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:80px'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 80.0, 10.0)),
            ("b", (100.0, 0.0, 80.0, 10.0)),
        ],
    },
    Case {
        name: "wider_than_its_column",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:150px'><div id=a>aa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("a", (0.0, 0.0, 150.0, 10.0)),
            ("b", (0.0, 10.0, 150.0, 10.0)),
            ("d", (100.0, 0.0, 150.0, 10.0)),
            ("e", (100.0, 10.0, 150.0, 10.0)),
        ],
    },
    Case {
        name: "percentage_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:50%'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 50.0, 10.0)),
            ("b", (100.0, 0.0, 50.0, 10.0)),
        ],
    },
    Case {
        name: "auto_margins",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:60px;margin:0 auto'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (20.0, 0.0, 60.0, 10.0)),
            ("b", (120.0, 0.0, 60.0, 10.0)),
        ],
    },
    Case {
        name: "maximum_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='max-width:80px'><div id=a>aa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("a", (0.0, 0.0, 80.0, 10.0)),
            ("b", (0.0, 10.0, 80.0, 10.0)),
            ("d", (100.0, 0.0, 80.0, 10.0)),
            ("e", (100.0, 10.0, 80.0, 10.0)),
        ],
    },
    Case {
        name: "minimum_width_wider_than_its_column",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='min-width:150px'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 150.0, 10.0)),
            ("b", (100.0, 0.0, 150.0, 10.0)),
        ],
    },
    Case {
        name: "small_minimum_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='min-width:10px'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "background",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:80px;background:red'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 80.0, 10.0)),
            ("b", (100.0, 0.0, 80.0, 10.0)),
        ],
    },
    Case {
        name: "padding_at_its_sides",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:80px;padding:0 5px;box-sizing:border-box'><div id=a>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (5.0, 0.0, 70.0, 10.0)),
            ("b", (105.0, 0.0, 70.0, 10.0)),
        ],
    },
    Case {
        name: "definite_height_columns",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px;height:20px\"><div id=c style='width:80px'><div id=a>aa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("a", (0.0, 0.0, 80.0, 10.0)),
            ("b", (0.0, 10.0, 80.0, 10.0)),
            ("d", (100.0, 0.0, 80.0, 10.0)),
            ("e", (100.0, 10.0, 80.0, 10.0)),
        ],
    },
    Case {
        name: "nested",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:80px'><div id=c2 style='width:60px'><div id=a>aa</div><div id=b>bb</div></div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 60.0, 10.0)),
            ("b", (100.0, 0.0, 60.0, 10.0)),
        ],
    },
    Case {
        name: "percentage_width_inside",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:80px'><div id=a style='width:50%'>aa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 10.0)),
            ("a", (0.0, 0.0, 40.0, 10.0)),
            ("b", (100.0, 0.0, 80.0, 10.0)),
        ],
    },
    Case {
        name: "fit_content_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:fit-content'><div id=a>aaaaaaaaaaaa aaaa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("a", (0.0, 0.0, 120.0, 20.0)),
            ("b", (0.0, 20.0, 120.0, 10.0)),
            ("d", (100.0, 0.0, 120.0, 10.0)),
            ("e", (100.0, 10.0, 120.0, 10.0)),
        ],
    },
    Case {
        name: "min_content_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:min-content'><div id=a>aaaaaaaaaaaa aaaa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("a", (0.0, 0.0, 120.0, 20.0)),
            ("b", (0.0, 20.0, 120.0, 10.0)),
            ("d", (100.0, 0.0, 120.0, 10.0)),
            ("e", (100.0, 10.0, 120.0, 10.0)),
        ],
    },
    Case {
        name: "percentage_margin",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='margin-left:10%'><div id=a>aaaaaaaaaaaa aaaa</div><div id=b>bb</div><div id=d>cc</div><div id=e>dd</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 30.0)),
            ("a", (10.0, 0.0, 90.0, 20.0)),
            ("b", (10.0, 20.0, 90.0, 10.0)),
            ("d", (110.0, 0.0, 90.0, 10.0)),
            ("e", (110.0, 10.0, 90.0, 10.0)),
        ],
    },
    Case {
        name: "calculated_width",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px\"><div id=c style='width:calc(100% - 20px)'><div id=a>aaaaaaaaaaaa aaaa</div><div id=b>bb</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("a", (0.0, 0.0, 80.0, 20.0)),
            ("b", (100.0, 0.0, 80.0, 10.0)),
        ],
    },
];
