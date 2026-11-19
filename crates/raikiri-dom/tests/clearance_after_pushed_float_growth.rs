//! A box that clears a float which pagination pushed to the next page moves
//! below it, and the block that holds both (CSS 2.1 §10.6.7) grows to hold
//! the moved clearing box, so the flow after the block follows it. The
//! pages and positions are the ones Chromium prints for the same markup:
//! 200px by 100px pages without margins, Ahem at 10px on 20px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index, top and bottom on that page of each fragment of the
/// element with the id `id`.
fn fragments_of(body: &str, id: &str) -> Vec<(usize, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}</style>{body}"
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
    let mut fragments = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            if page.dom().attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                fragments.push((index, rect.y, rect.y + rect.height));
            }
        }
    }
    fragments
}

/// The page index and the top on that page of the first fragment of the
/// element with the id `id`.
fn place_of(body: &str, id: &str) -> (usize, f32) {
    let &(page, top, _) = fragments_of(body, id)
        .first()
        .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"));
    (page, top)
}

fn assert_place(body: &str, id: &str, expected: (usize, f32)) {
    let (page, top) = place_of(body, id);
    let off = (top - expected.1).abs();
    assert!(
        page == expected.0 && !off.is_nan() && off < 0.01,
        "#{id} at {:?}, expected {expected:?} in {body}",
        (page, top)
    );
}

/// A 50px block, then a flow root holding `inner`, then the line `#x`.
fn in_flow_root(root_style: &str, inner: &str) -> String {
    format!(
        "<div style='height:50px'>W</div>\
         <div style='display:flow-root;{root_style}'>{inner}</div>\
         <div><span id=x>X</span></div>"
    )
}

/// A 70px float that may not break, too tall for the 50px left on the
/// first page, so it moves whole to the second.
const FLOAT: &str = "<div style='float:left;height:70px;break-inside:avoid'>Y</div>";

#[test]
fn a_flow_root_grows_to_hold_a_box_that_clears_a_pushed_float() {
    for (body, z, x) in [
        (
            in_flow_root(
                "",
                &format!("{FLOAT}<div style='clear:both'><span id=z>Z</span></div>"),
            ),
            Some((1, 75.0)),
            (2, 5.0),
        ),
        (
            in_flow_root(
                "",
                &format!("{FLOAT}<div>Q</div><div style='clear:both'><span id=z>Z</span></div>"),
            ),
            None,
            (2, 5.0),
        ),
        (
            in_flow_root(
                "padding-bottom:5px",
                &format!("{FLOAT}<div style='clear:both'><span id=z>Z</span></div>"),
            ),
            None,
            (2, 5.0),
        ),
        (
            in_flow_root(
                "",
                &format!(
                    "{FLOAT}<div style='clear:both;margin-top:10px'><span id=z>Z</span></div>"
                ),
            ),
            None,
            (2, 5.0),
        ),
        (
            in_flow_root(
                "",
                &format!("<div>{FLOAT}</div><div style='clear:both'><span id=z>Z</span></div>"),
            ),
            None,
            (2, 5.0),
        ),
    ] {
        if let Some(z) = z {
            assert_place(&body, "z", z);
        }
        assert_place(&body, "x", x);
    }
}

#[test]
fn an_empty_clearing_box_and_the_flow_after_a_one_sided_clear_follow_the_float() {
    let empty = in_flow_root(
        "",
        &format!("{FLOAT}<div style='clear:both;height:10px'></div>"),
    );
    assert_place(&empty, "x", (1, 85.0));
    let left = in_flow_root(
        "",
        &format!("{FLOAT}<div style='clear:left'>Z</div><div><span id=v>V</span></div>"),
    );
    assert_place(&left, "v", (2, 5.0));
    assert_place(&left, "x", (2, 25.0));
}
