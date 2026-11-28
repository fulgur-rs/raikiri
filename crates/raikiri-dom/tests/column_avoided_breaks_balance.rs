//! A break between columns that `break-before` or `break-after` avoids
//! (`avoid` or `avoid-column`) is not taken: the column ends at the last
//! break before it that nothing avoids, and the box after that break starts
//! the next column. Balanced columns grow when no such break is left in a
//! column (CSS Fragmentation 3, 4.4; CSS Multi-column 1, section 7.1).
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body and 10px
//! Ahem text. `o` is the container; the wrappers `c`, `v`, `w`, `w2` and
//! `x` are not checked. A box that breaks lists one border box per column it is in, in
//! column order.

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
fn an_avoided_break_moves_up_to_an_earlier_one() {
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
        name: "avoid_before_moves_the_block_before",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_after_moves_the_block",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-after:avoid'>bb</div><div id=k>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_in_fill_auto_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px;column-fill:auto\"><div id=a>aa</div><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_in_definite_height_balance",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px\"><div id=a>aa</div><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "chain_of_avoided_breaks",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-after:avoid'>bb</div><div id=k style='break-after:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 20.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "break_between_lines_is_left",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa<br>aa<br>aa</div><div id=b style='break-before:avoid'>bb</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 20.0)),
            ("a", (100.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "two_avoided_breaks_in_a_row",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d style='break-before:avoid'>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 20.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_page_takes_no_part",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=k>kk</div><div id=d style='break-before:avoid-page'>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_inside_a_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa</div><div id=b>bb</div><div id=d style='break-before:avoid'>dd</div><div id=e>ee</div><div id=f>ff</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_after_before_a_forced_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-after:avoid'>bb</div><div id=d style='break-before:column'>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "moved_block_loses_its_margin",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='margin-top:5px'>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "moved_wrapper_block",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=c><div id=b>bb</div></div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "moved_block_of_two_lines",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb<br>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 20.0)),
            ("k", (100.0, 20.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "no_break_left_grows_the_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-before:avoid'>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (0.0, 20.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "definite_height_overflow_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px\"><div id=a>aa</div><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div><div id=g>gg</div><div id=h>hh</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
            ("f", (300.0, 0.0, 100.0, 10.0)),
            ("g", (300.0, 10.0, 100.0, 10.0)),
            ("h", (400.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_in_a_wrapper_of_four",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa</div><div id=b style='break-before:avoid'>bb</div><div id=d>dd</div><div id=e>ee</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_with_no_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-before:avoid'>bb</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_after_grows_the_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-after:avoid'>bb</div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div><div id=g>gg</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (0.0, 20.0, 100.0, 10.0)),
            ("e", (100.0, 0.0, 100.0, 10.0)),
            ("f", (100.0, 10.0, 100.0, 10.0)),
            ("g", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_before_grows_the_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=d style='break-before:avoid'>dd</div><div id=e>ee</div><div id=f>ff</div><div id=g>gg</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (0.0, 20.0, 100.0, 10.0)),
            ("e", (100.0, 0.0, 100.0, 10.0)),
            ("f", (100.0, 10.0, 100.0, 10.0)),
            ("g", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_before_in_a_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=c><div id=a>aa</div><div id=b>bb</div><div id=d style='break-before:avoid'>dd</div><div id=e>ee</div><div id=f>ff</div><div id=g>gg</div></div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (0.0, 20.0, 100.0, 10.0)),
            ("e", (100.0, 0.0, 100.0, 10.0)),
            ("f", (100.0, 10.0, 100.0, 10.0)),
            ("g", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_column_before",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=d style='break-before:avoid-column'>dd</div><div id=e>ee</div><div id=f>ff</div><div id=g>gg</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (0.0, 20.0, 100.0, 10.0)),
            ("e", (100.0, 0.0, 100.0, 10.0)),
            ("f", (100.0, 10.0, 100.0, 10.0)),
            ("g", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_after_before_a_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a style='break-after:avoid'>aa</div><div id=c><div id=b>bb</div></div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_padding_moves_with_its_box",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w style='padding-top:5px'><div id=x><div id=b>bb</div><div id=k style='break-before:avoid'>kk</div></div></div><div id=d>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 25.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 5.0, 100.0, 10.0)),
            ("k", (100.0, 15.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "later_column_end_moves_after_one_that_cannot",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px;column-fill:auto\"><div id=a>aa</div><div id=b style='break-before:avoid'>bb</div><div id=d>dd</div><div id=e>ee</div><div id=f style='break-before:avoid'>ff</div><div id=g>gg</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
            ("g", (300.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "avoid_before_after_a_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=c><div id=b>bb</div><div id=k>kk</div></div><div id=d style='break-before:avoid'>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "margins_at_a_moved_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='margin-bottom:10px'>bb</div><div id=k style='break-before:avoid;margin-top:10px'>kk</div><div id=d>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 20.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "fill_auto_column_end_with_no_free_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px;column-fill:auto\"><div id=a>aa<br>aa<br>aa</div><div id=b style='break-before:avoid'>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 20.0)),
            ("a", (100.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 10.0, 100.0, 10.0)),
            ("k", (200.0, 0.0, 100.0, 10.0)),
            ("d", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "fill_auto_chain_of_avoided_breaks",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px;column-fill:auto\"><div id=a>aa</div><div id=b style='break-before:avoid'>bb</div><div id=k style='break-before:avoid'>kk</div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "white_space_between_wrapper_padding_and_box",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div>\n<div id=w style='padding-top:5px'>\n<div id=b>bb</div>\n<div id=k style='break-before:avoid'>kk</div>\n</div>\n<div id=d>dd</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 25.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 5.0, 100.0, 10.0)),
            ("k", (100.0, 15.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "white_space_between_blocks",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div>\n<div id=b>bb</div>\n<div id=k style='break-before:avoid'>kk</div>\n<div id=d>dd</div>\n<div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "many_avoided_column_ends_in_overflow_columns",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px;height:20px;column-fill:auto\"><div id=b0>xx</div><div id=b1>xx</div><div id=b2 style='break-before:avoid'>xx</div><div id=b3>xx</div><div id=b4>xx</div><div id=b5 style='break-before:avoid'>xx</div><div id=b6>xx</div><div id=b7>xx</div><div id=b8 style='break-before:avoid'>xx</div><div id=b9>xx</div><div id=b10>xx</div><div id=b11 style='break-before:avoid'>xx</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("b0", (0.0, 0.0, 100.0, 10.0)),
            ("b1", (100.0, 0.0, 100.0, 10.0)),
            ("b2", (100.0, 10.0, 100.0, 10.0)),
            ("b3", (200.0, 0.0, 100.0, 10.0)),
            ("b4", (300.0, 0.0, 100.0, 10.0)),
            ("b5", (300.0, 10.0, 100.0, 10.0)),
            ("b6", (400.0, 0.0, 100.0, 10.0)),
            ("b7", (500.0, 0.0, 100.0, 10.0)),
            ("b8", (500.0, 10.0, 100.0, 10.0)),
            ("b9", (600.0, 0.0, 100.0, 10.0)),
            ("b10", (700.0, 0.0, 100.0, 10.0)),
            ("b11", (700.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "many_avoided_column_ends_in_balanced_overflow_columns",
        body: "<div id=o style=\"columns:2;column-gap:0;width:200px;height:20px\"><div id=b0>xx</div><div id=b1>xx</div><div id=b2 style='break-before:avoid'>xx</div><div id=b3>xx</div><div id=b4>xx</div><div id=b5 style='break-before:avoid'>xx</div><div id=b6>xx</div><div id=b7>xx</div><div id=b8 style='break-before:avoid'>xx</div><div id=b9>xx</div><div id=b10>xx</div><div id=b11 style='break-before:avoid'>xx</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 200.0, 20.0)),
            ("b0", (0.0, 0.0, 100.0, 10.0)),
            ("b1", (100.0, 0.0, 100.0, 10.0)),
            ("b2", (100.0, 10.0, 100.0, 10.0)),
            ("b3", (200.0, 0.0, 100.0, 10.0)),
            ("b4", (300.0, 0.0, 100.0, 10.0)),
            ("b5", (300.0, 10.0, 100.0, 10.0)),
            ("b6", (400.0, 0.0, 100.0, 10.0)),
            ("b7", (500.0, 0.0, 100.0, 10.0)),
            ("b8", (500.0, 10.0, 100.0, 10.0)),
            ("b9", (600.0, 0.0, 100.0, 10.0)),
            ("b10", (700.0, 0.0, 100.0, 10.0)),
            ("b11", (700.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_avoids_a_break_before_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=w style='break-before:avoid'><div id=k>kk</div><div id=d>dd</div></div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_avoids_a_break_after_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w style='break-after:avoid'><div id=b>bb</div><div id=k>kk</div></div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_avoids_a_column_break_before_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=w style='break-before:avoid-column'><div id=k>kk</div><div id=d>dd</div></div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_avoid_in_fill_auto_columns",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px;height:20px;column-fill:auto\"><div id=a>aa</div><div id=b>bb</div><div id=w style='break-before:avoid'><div id=k>kk</div><div id=d>dd</div></div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "nested_wrapper_avoids_a_break_before_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=w><div id=v style='break-before:avoid'><div id=k>kk</div><div id=d>dd</div></div></div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "padded_wrapper_avoids_a_break_before_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w style='break-before:avoid;padding-top:5px'><div id=b>bb</div><div id=k>kk</div></div><div id=d>dd</div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 25.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 15.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrappers_avoid_the_break_between_them",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w style='break-after:avoid'><div id=b>bb</div></div><div id=w2 style='break-before:avoid'><div id=k>kk</div></div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (0.0, 20.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
            ("f", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_of_two_blocks_avoids_a_break_before_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b>bb</div><div id=c style='break-before:avoid'><div id=k>kk</div><div id=d>dd</div></div><div id=e>ee</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (100.0, 0.0, 100.0, 10.0)),
            ("k", (100.0, 10.0, 100.0, 10.0)),
            ("d", (200.0, 0.0, 100.0, 10.0)),
            ("e", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_avoids_a_break_after_itself_before_another_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w style='break-after:avoid'><div id=b>bb</div></div><div id=w2><div id=k>kk</div></div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (0.0, 20.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
            ("f", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "box_avoids_a_break_after_itself_before_a_wrapper",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=b style='break-after:avoid'>bb</div><div id=w><div id=k>kk</div><div id=d>dd</div></div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 30.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (0.0, 20.0, 100.0, 10.0)),
            ("d", (100.0, 0.0, 100.0, 10.0)),
            ("e", (100.0, 10.0, 100.0, 10.0)),
            ("f", (100.0, 20.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "nested_wrapper_avoids_a_break_after_itself",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=a>aa</div><div id=w><div id=b>bb</div><div id=v style='break-after:avoid'><div id=k>kk</div></div></div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
    Case {
        name: "wrapper_at_the_start_avoids_no_break",
        body: "<div id=o style=\"columns:3;column-gap:0;width:300px\"><div id=w style='break-before:avoid'><div id=a>aa</div><div id=b>bb</div></div><div id=k>kk</div><div id=d>dd</div><div id=e>ee</div><div id=f>ff</div></div>",
        expected: &[
            ("o", (0.0, 0.0, 300.0, 20.0)),
            ("a", (0.0, 0.0, 100.0, 10.0)),
            ("b", (0.0, 10.0, 100.0, 10.0)),
            ("k", (100.0, 0.0, 100.0, 10.0)),
            ("d", (100.0, 10.0, 100.0, 10.0)),
            ("e", (200.0, 0.0, 100.0, 10.0)),
            ("f", (200.0, 10.0, 100.0, 10.0)),
        ],
    },
];
