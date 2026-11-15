//! A replaced element or form control that is a block-level child of a
//! paragraph holding text keeps its border box clear of the floats before
//! it (CSS 2.1 §9.5), as it does in a block without text, and its `auto`
//! inline size is its fit-content size, not the room it is given (CSS 2.1
//! §10.3.4). A button, an image, an input, a select and a textarea sit beside
//! a float on either side in either direction, with auto margins, limits,
//! clearance and margins that collapse with their siblings, and in a vertical
//! paragraph. The `auto` width rows are those of images and buttons: a form
//! control without a natural size gets no fit-content width here or in a
//! block without text. A block that aligns its content roots a formatting
//! context of its own and sits beside the floats as well.
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

/// A block image or form control in a paragraph sits beside a float on
/// either side, in a left-to-right or right-to-left paragraph, with or
/// without text around it.
#[test]
fn a_block_image_or_form_control_sits_beside_a_float() {
    assert_rows(&[
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px'></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 0.0, 40.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 10.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 0.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [45.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [45.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 40.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px'></div>",
            &[("f", [70.0, 0.0, 30.0, 30.0]), ("b", [0.0, 0.0, 40.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[("f", [70.0, 0.0, 30.0, 30.0]), ("b", [0.0, 10.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block'></div>",
            &[("f", [70.0, 0.0, 30.0, 30.0]), ("b", [0.0, 0.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [5.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [5.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [15.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [15.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [60.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [60.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [60.0, 10.0, 40.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px'></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [60.0, 0.0, 40.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [100.0, 10.0, 0.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [100.0, 0.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [53.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [53.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [45.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [45.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [50.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [50.0, 0.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [76.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [76.0, 0.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [50.0, 10.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [50.0, 0.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [70.0, 10.0, 0.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block'></div>",
            &[("f", [70.0, 0.0, 30.0, 30.0]), ("b", [70.0, 0.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:5px;margin-right:7px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [15.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:0 auto'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [15.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [20.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:50%;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [20.0, 0.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [46.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [46.0, 0.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [20.0, 10.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><button id=b style='display:block;width:50px;font:10px/10px Ahem'>XX</button></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [20.0, 0.0, 50.0, 16.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><input id=b style='display:block;width:30px;height:10px;padding:0;border:0'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [40.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'>T</div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><img id=b style='display:block;width:90px;height:10px'></div>",
            &[
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 90.0, 10.0]),
            ],
        ),
    ]);
}

/// The same box with no float, a float after it, a shorter float, margins
/// that collapse with its siblings, limits, clearance and auto margins, in a
/// nested block and in a vertical paragraph.
#[test]
fn a_block_image_or_form_control_beside_its_neighbours() {
    assert_rows(&[
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[("b", [0.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 40.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;width:40px;height:10px'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block'>T</div>",
            &[("b", [0.0, 10.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 10.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[("b", [0.0, 10.0, 0.0, 0.0]), ("f", [0.0, 10.0, 30.0, 30.0])],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 0.0, 0.0]),
                ("q", [0.0, 29.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 0.0, 0.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [10.0, 30.0, 0.0, 0.0])],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 20.0]),
                ("b", [30.0, 10.0, 0.0, 0.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;height:10px'>T</div>",
            &[("b", [0.0, 10.0, 0.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 0.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;height:10px'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 0.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 0.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;height:10px'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 0.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 0.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;height:10px'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 0.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:90px;max-width:50%;height:10px'>T</div>",
            &[("b", [0.0, 10.0, 50.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;max-width:50%;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:90px;max-width:50%;height:10px'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 50.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;max-width:50%;height:10px'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 50.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;width:90px;max-width:50%;height:10px'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 50.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:90px;max-width:50%;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 90.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;width:90px;max-width:50%;height:10px'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 50.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;min-width:20px;height:10px'>T</div>",
            &[("b", [0.0, 10.0, 20.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;min-width:20px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;min-width:20px;height:10px'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 20.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;min-width:20px;height:10px'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 20.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;min-width:20px;height:10px'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 20.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;min-width:20px;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;min-width:20px;height:10px'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'>T</div>",
            &[("b", [0.0, 18.0, 40.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 18.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 18.0, 40.0, 10.0]),
                ("f", [0.0, 34.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 28.0, 40.0, 10.0]),
                ("q", [0.0, 44.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 18.0, 40.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;width:40px;height:10px;margin:8px 0 6px'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 44.0]),
                ("b", [30.0, 18.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;clear:left'>T</div>",
            &[("b", [0.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;clear:left'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [0.0, 30.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;clear:left'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;clear:left'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [0.0, 55.0, 40.0, 10.0]),
                ("q", [0.0, 69.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;width:40px;height:10px;clear:left'>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;clear:left'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;width:40px;height:10px;clear:left'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 50.0]),
                ("b", [0.0, 30.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;margin-left:auto'>T</div>",
            &[("b", [60.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:auto'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [60.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;margin-left:auto'><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:auto'><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [60.0, 25.0, 40.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:auto'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 5.0]),
                ("b", [60.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;margin-left:auto'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<img id=b style='display:block;width:40px;height:10px;margin-left:auto'>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [60.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[("b", [0.0, 10.0, 24.0, 14.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 24.0, 14.0]),
                ("f", [0.0, 24.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 24.0, 14.0]),
                ("q", [0.0, 43.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 24.0, 14.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 14.0, 24.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<button id=b style='display:block;font:10px/10px Ahem;padding:0;border:2px solid'>XX</button>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 34.0]),
                ("b", [30.0, 10.0, 24.0, 14.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[("b", [0.0, 10.0, 30.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 30.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 30.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 30.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<select id=b style='display:block;width:30px;height:10px;padding:0;border:0'></select>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[("b", [0.0, 10.0, 30.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea><div id=f style='float:left;width:30px;height:30px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 30.0, 10.0]),
                ("f", [0.0, 20.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=p style='margin-bottom:5px'>U</div><div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea><div id=q style='margin-top:4px'>V</div>T</div>",
            &[
                ("p", [0.0, 10.0, 100.0, 10.0]),
                ("f", [0.0, 25.0, 30.0, 30.0]),
                ("b", [30.0, 25.0, 30.0, 10.0]),
                ("q", [0.0, 39.0, 100.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:5px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[("f", [0.0, 0.0, 30.0, 5.0]), ("b", [0.0, 10.0, 30.0, 10.0])],
        ),
        (
            "<div style='height:100px;writing-mode:vertical-lr'>T<div id=f style='float:left;width:30px;height:30px'></div><textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [10.0, 30.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=p>T<textarea id=b style='display:block;width:30px;height:10px;padding:0;border:0'></textarea>T</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("p", [0.0, 0.0, 100.0, 30.0]),
                ("b", [30.0, 10.0, 30.0, 10.0]),
            ],
        ),
    ]);
}

/// A block that aligns its content, and a button with no content, beside a
/// float in a paragraph, in a block without text and with no float.
#[test]
fn a_block_that_aligns_its_content_sits_beside_a_float() {
    assert_rows(&[
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem'></button>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 16.0, 6.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><button id=b style='display:block;font:10px/10px Ahem'></button></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 0.0, 16.0, 6.0])],
        ),
        (
            "<div style='width:100px'>T<button id=b style='display:block;font:10px/10px Ahem'></button>T</div>",
            &[("b", [0.0, 10.0, 16.0, 6.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:center;height:20px'>X</div>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 70.0, 20.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:center;height:20px'>X</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 70.0, 20.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=b style='align-content:center;height:20px'>X</div>T</div>",
            &[("b", [0.0, 10.0, 100.0, 20.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:center;margin-top:6px'>X</div>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 16.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:center;margin-top:6px'>X</div></div>",
            &[
                ("f", [0.0, 6.0, 30.0, 30.0]),
                ("b", [30.0, 6.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=b style='align-content:center;margin-top:6px'>X</div>T</div>",
            &[("b", [0.0, 16.0, 100.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:end;height:20px'>X</div>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 70.0, 20.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><div id=b style='align-content:end;height:20px'>X</div></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 70.0, 20.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=b style='align-content:end;height:20px'>X</div>T</div>",
            &[("b", [0.0, 10.0, 100.0, 20.0])],
        ),
    ]);
}

/// An image with an `auto` width that `justify-self: stretch`, or its
/// container's `justify-items: stretch`, asks to stretch fills the room
/// beside a float, as it does in a block without text; `justify-self: start`
/// keeps its natural width. Inside an inline box only its own `justify-self`
/// counts: `justify-items` does not apply there.
#[test]
fn an_image_that_justify_stretches_fills_the_room_beside_a_float() {
    assert_rows(&[
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px;justify-self:stretch'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px;justify-self:stretch'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;height:10px;justify-self:stretch'>T</div>",
            &[("b", [0.0, 10.0, 100.0, 10.0])],
        ),
        (
            "<div style='width:100px;justify-items:stretch'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;justify-items:stretch'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 70.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;justify-items:stretch'>T<img id=b style='display:block;height:10px'>T</div>",
            &[("b", [0.0, 10.0, 100.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;justify-self:stretch'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;width:40px;height:10px;justify-self:stretch'></div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 0.0, 40.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;width:40px;height:10px;justify-self:stretch'>T</div>",
            &[("b", [0.0, 10.0, 40.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px;justify-self:start'>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 0.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'><div id=f style='float:left;width:30px;height:30px'></div><img id=b style='display:block;height:10px;justify-self:start'></div>",
            &[("f", [0.0, 0.0, 30.0, 30.0]), ("b", [30.0, 0.0, 0.0, 10.0])],
        ),
        (
            "<div style='width:100px'>T<img id=b style='display:block;height:10px;justify-self:start'>T</div>",
            &[("b", [0.0, 10.0, 0.0, 10.0])],
        ),
        (
            "<div style='width:100px;justify-items:stretch'>T<div id=f style='float:left;width:30px;height:30px'></div><span><img id=b style='display:block;height:10px'></span>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 0.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px'>T<div id=f style='float:left;width:30px;height:30px'></div><span><img id=b style='display:block;height:10px;justify-self:stretch'></span>T</div>",
            &[
                ("f", [0.0, 0.0, 30.0, 30.0]),
                ("b", [30.0, 10.0, 70.0, 10.0]),
            ],
        ),
    ]);
}
