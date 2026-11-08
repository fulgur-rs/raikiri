//! The max-content width of a box that shrinks to fit counts the floats that a
//! following box of its own formatting context sits beside: a table, a
//! `flow-root`, a scroller, a flex container or a block image after a float
//! adds the width of the float to the width the box asks for, as browsers
//! measure it. Floats count side by side only up to the next in-flow child.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height.

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
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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

#[test]
fn shrink_to_fit_boxes_count_the_floats_a_context_root_sits_beside() {
    check_boxes(&[
        (
            "a table after a float, with a percentage column",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><table id=t><tr><td id=a style=\"width:50%\">A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 330.0, 16.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 70.0, 16.0),
            ],
        ),
        (
            "a table after a float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><table id=t><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 310.0, 16.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a float that shrinks to fit",
            "<div id=w style=\"float:left\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><table id=t><tr><td id=a style=\"width:50%\">A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 330.0, 16.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 70.0, 16.0),
            ],
        ),
        (
            "an absolutely positioned box",
            "<div id=w style=\"position:absolute\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><table id=t><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 310.0, 16.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a `width: fit-content` block",
            "<div id=w style=\"width:fit-content\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><table id=t><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 310.0, 16.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a scroller after a float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><div id=t style=\"overflow:hidden\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 290.0, 10.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 260.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "a `flow-root` after a float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXX</div><div id=t style=\"display:flow-root\">BBB CC</div></div>",
            &[
                ("w", 0.0, 0.0, 110.0, 10.0),
                ("z", 0.0, 0.0, 50.0, 10.0),
                ("t", 50.0, 0.0, 60.0, 10.0),
            ],
        ),
        (
            "a flex container after a float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flex\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 130.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "a block image after a float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><img id=t style=\"display:block;width:30px;height:10px\"></div>",
            &[
                ("w", 0.0, 0.0, 130.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "a table after a right float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:right\">XXXXXXXXXX</div><table id=t><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 150.0, 16.0),
                ("z", 50.0, 0.0, 100.0, 10.0),
                ("t", 0.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a table after two left floats",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=y style=\"float:left\">XX</div><table id=t><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 170.0, 16.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("y", 100.0, 0.0, 20.0, 10.0),
                ("t", 120.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a `flow-root` between a left and a right float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=y style=\"float:right\">XXXXX</div><div id=t style=\"display:flow-root\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 180.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("y", 130.0, 0.0, 50.0, 10.0),
                ("t", 100.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "a left margin smaller than the float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><table id=t style=\"margin-left:5px\"><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 150.0, 16.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 50.0, 16.0),
            ],
        ),
        (
            "a left margin wider than the float, and a right margin",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root;margin-left:150px;margin-right:7px\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 187.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 150.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "a negative left margin pulls the box over the float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root;margin-left:-5px\">BBB</div></div>",
            &[("w", 0.0, 0.0, 125.0, 10.0), ("z", 0.0, 0.0, 100.0, 10.0)],
        ),
        (
            "a right-to-left box: the float and the margin on the right",
            "<div id=w style=\"display:inline-block;direction:rtl\"><div id=z style=\"float:right\">XXXXXXXXXX</div><div id=t style=\"display:flow-root;margin-right:120px\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 150.0, 10.0),
                ("z", 50.0, 0.0, 100.0, 10.0),
                ("t", 0.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "`clear: right` beside a left float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root;clear:right\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 130.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 30.0, 10.0),
            ],
        ),
        (
            "`clear: left` puts the table below the float",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><table id=t style=\"clear:left\"><tr><td id=a>A</td><td id=b>BBB</td></tr></table></div>",
            &[
                ("w", 0.0, 0.0, 100.0, 26.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 0.0, 10.0, 50.0, 16.0),
            ],
        ),
        (
            "the next `flow-root` is below the run of floats",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root\">BBB</div><div id=u style=\"display:flow-root\">CCCCC</div></div>",
            &[
                ("w", 0.0, 0.0, 130.0, 20.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 30.0, 10.0),
                ("u", 0.0, 10.0, 130.0, 10.0),
            ],
        ),
        (
            "a block that shares the float context ends the run",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left;height:50px\">XXXXXXXXXX</div><div id=m>B</div><table id=t><tr><td>A</td><td>BBB</td></tr></table></div>",
            &[("w", 0.0, 0.0, 100.0, 76.0), ("z", 0.0, 0.0, 100.0, 50.0)],
        ),
        (
            "a float after a block that shares the context starts a new run",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left;height:50px\">XXXXXXXXXX</div><div id=m>B</div><div id=y style=\"float:left\">XXXXXXXXXX</div></div>",
            &[("w", 0.0, 0.0, 100.0, 70.0), ("z", 0.0, 0.0, 100.0, 50.0)],
        ),
        (
            "a block that shares the float context",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXXXXXXXXXXXXXXXXXX</div><div id=t>BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 260.0, 20.0),
                ("z", 0.0, 0.0, 260.0, 10.0),
                ("t", 0.0, 0.0, 260.0, 20.0),
            ],
        ),
        (
            "min-content keeps the widest of the float and the box",
            "<div id=w style=\"width:min-content\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 100.0, 10.0),
                ("z", 0.0, 0.0, 100.0, 10.0),
                ("t", 100.0, 0.0, 0.0, 10.0),
            ],
        ),
        (
            "padding and borders of the box and of the container",
            "<div id=w style=\"display:inline-block;padding:0 4px\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=t style=\"display:flow-root;padding:0 3px;border-left:2px solid\">BBB</div></div>",
            &[
                ("w", 0.0, 0.0, 146.0, 10.0),
                ("z", 4.0, 0.0, 100.0, 10.0),
                ("t", 104.0, 0.0, 38.0, 10.0),
            ],
        ),
        (
            "a context root inside a block that shares the float context",
            "<div id=w style=\"display:inline-block\"><div id=z style=\"float:left\">XXXXXXXXXX</div><div id=m><table id=t><tr><td>BBBBB</td></tr></table></div></div>",
            &[("w", 0.0, 0.0, 100.0, 26.0), ("z", 0.0, 0.0, 100.0, 10.0)],
        ),
    ]);
}
