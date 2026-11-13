//! A size-contained box with an aspect ratio is sized as if
//! `contain-intrinsic-size` named the size of its content: the ratio gives
//! one axis from the other, the bounds of one axis carry over to the other
//! through it, and an `auto` minimum height is the height of that content,
//! held to the maximum height the box states. Where each box falls is what
//! Chromium lays out for the same documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 1280px wide.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:1280px 1000px;margin:0}} body{{margin:0}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).unwrap();
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
            ))
        })
        .collect()
}

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_contained_inline_block_or_float_keeps_its_ratio() {
    // Inline blocks with a ratio of 1 or 2 under a `max-height` that carries
    // a maximum width, a `max-width` under which the content height still
    // holds, a `min-height`, only a `contain-intrinsic-width`, a stated
    // height, padding, as an inline flex or grid container, and floats.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 300, 200, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 300, 50, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-height:300px'></div></div>",
            &[("c", [0, 300, 300, 300])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:2;max-height:50px'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-width:200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;height:50px'></div></div>",
            &[("c", [0, 300, 200, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;padding:10px;max-height:50px'></div></div>",
            &[("c", [0, 300, 70, 70])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-flex;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-grid;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
    ]);
}

#[test]
fn contained_blocks_and_items_keep_their_ratio() {
    // Floats, blocks, flex items of a row and a column and a grid item with a
    // ratio, under a `max-width` or `max-height`, with content taller than the
    // ratio gives, and inline blocks that scroll, hold their padding in the
    // border box, contain only their inline size, state `auto` intrinsic
    // sizes, take an `auto` ratio or state a `min-height`.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 300, 200, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 300, 50, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 300, 50, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:2'></div></div>",
            &[("c", [0, 300, 500, 250])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:2'></div></div>",
            &[("c", [0, 300, 500, 400])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 300, 200, 200])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex;flex-direction:column;align-items:flex-start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;box-sizing:border-box;padding:10px;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:inline-size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:auto 200px auto 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:auto 1;box-sizing:border-box;padding:10px;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-height:20px;max-height:50px'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
    ]);
}

#[test]
fn contained_ratio_boxes_with_a_stated_width_or_a_wrapper() {
    // A block with a stated width under a `max-height` or with content taller
    // than the ratio gives, a `min-width` that wins over the carried maximum,
    // shrink-to-fit wrappers around a contained box, a vertical inline block
    // whose `max-height` caps its inline size, and a flex item stretched in
    // the cross axis.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='width:300px;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 300, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-width:100px;max-height:50px'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-width:100px;max-height:50px;width:fit-content'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=w style='float:left'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div></div>",
            &[("w", [0, 300, 50, 50]), ("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='writing-mode:vertical-rl;display:inline-block;contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 300, 200, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex'><div id=c style='contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 300, 300, 300])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='width:300px;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1'></div></div>",
            &[("c", [0, 300, 300, 400])],
        ),
    ]);
}

#[test]
fn a_contained_ratio_box_with_padding_relates_the_box_box_sizing_names() {
    // A ratio of 1 / 2 with 10px of padding relates the content box of a
    // `content-box` box and the border box of a `border-box` one, in inline
    // blocks, floats and flex items, under a `max-height` or `max-width`, and
    // with content shorter than the ratio gives.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 420])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 420])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;box-sizing:border-box;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 440])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;max-height:100px;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 70, 120])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;max-width:50px;contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 70, 420])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 420])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 120])],
        ),
        (
            "<div style='margin-top:300px;width:500px;display:flex'><div id=c style='contain:size;contain-intrinsic-size:200px 400px;aspect-ratio:1/2;padding:10px'></div></div>",
            &[("c", [0, 300, 220, 420])],
        ),
    ]);
}
