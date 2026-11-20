//! A box with no children whose `::before` or `::after` generates content is
//! the root of a paragraph that holds that content (CSS Pseudo-Elements 4
//! §2.1: the pseudo-elements are the box's first and last children). Its
//! shrink-to-fit width and its height come from those lines, so an empty
//! `inline-block`, float, absolutely positioned box, flex or grid item, or
//! table cell with generated text is as wide as the text, not zero.
//!
//! The expected border boxes `[x, y, width, height]` of the elements with an
//! `id` are the ones Chromium computes for the same markup in an 800px by
//! 600px viewport with no body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes `[x, y, width, height]` of the elements with an `id` on
/// the first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
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
    let mut boxes: Vec<(String, [f32; 4])> = Vec::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id")
            && !boxes.iter().any(|(seen, _)| seen == id)
        {
            let rect = fragment.rect();
            boxes.push((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]));
        }
    }
    boxes
}

/// Markup and the border boxes Chromium gives the elements with an `id` in it.
type Row<'a> = (&'a str, &'a [(&'a str, [f32; 4])]);

fn assert_rows(rows: &[Row<'_>]) {
    let failures: Vec<_> = rows
        .iter()
        .filter_map(|(body, want)| {
            let got = boxes(body);
            let matches = want.iter().all(|(id, want)| {
                got.iter().any(|(got_id, got)| {
                    got_id == id
                        && got
                            .iter()
                            .zip(want)
                            .all(|(got, want)| (got - want).abs() <= 0.5)
                })
            });
            (!matches).then(|| format!("{body}\n  expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_empty_inline_block_is_as_wide_as_its_generated_text() {
    assert_rows(&[
        (
            r#"<style>#s::before{content:"xx"}</style><div id=s style="display:inline-block"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::after{content:"xx"}</style><div id=s style="display:inline-block"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx"}</style><div id=s style="display:inline-block;padding:5px"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 30.0, 20.0]),
                ("n", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx yy"}</style><div id=s style="display:inline-block;width:30px"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 30.0, 20.0]),
                ("n", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn a_block_pseudo_element_sizes_an_empty_inline_block() {
    assert_rows(&[
        (
            r#"<style>#s::before{content:"xx";display:block}</style><div id=s style="display:inline-block;vertical-align:top"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx";display:block}</style><div id=s style="display:inline-block"></div>a<div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn other_shrink_to_fit_boxes_size_to_their_generated_text() {
    assert_rows(&[
        (
            r#"<style>#s::before{content:"xx"}</style><div id=s style="float:left"></div><div id=n style="clear:both;height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx"}</style><div id=s style="position:absolute"></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx"}</style><div style="display:flex"><div id=s></div></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx"}</style><div style="display:table"><div id=s style="display:table-cell"></div></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
        (
            r#"<style>#s::before{content:"xx"}</style><div style="display:grid;justify-items:start"><div id=s></div></div><div id=n style="height:10px"></div>"#,
            &[
                ("s", [0.0, 0.0, 20.0, 10.0]),
                ("n", [0.0, 10.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn empty_generated_text_leaves_an_empty_inline_block_empty() {
    assert_rows(&[(
        r#"<style>#s::before{content:""}</style><div id=s style="display:inline-block"></div><div id=n style="height:10px"></div>"#,
        &[("s", [0.0, 8.0, 0.0, 0.0]), ("n", [0.0, 10.0, 800.0, 10.0])],
    )]);
}
