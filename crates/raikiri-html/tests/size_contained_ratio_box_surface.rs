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

#[test]
fn a_contained_flex_item_keeps_the_height_its_contained_size_names() {
    // A row flex item with a ratio of 1 and a `max-width` stays as high as
    // `contain-intrinsic-size` says, whatever the alignment, in a container
    // that wraps or shrinks to fit, with padding in either box; a `max-height`,
    // a `min-height` of zero or a scroll container lets the ratio give the
    // height, and a `max-height` carries a maximum width.
    assert_rows(&[
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:flex;width:500px;align-items:start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-width:0;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div><div id=d style='width:10px;height:10px'></div></div>",
            &[("c", [0, 0, 50, 200]), ("d", [50, 0, 10, 10])],
        ),
        (
            "<div style='display:inline-flex'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;padding:5px;box-sizing:border-box'></div></div>",
            &[("c", [0, 0, 50, 210])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;padding:5px'></div></div>",
            &[("c", [0, 0, 60, 210])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;max-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;min-height:0;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;overflow:hidden'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1'></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
    ]);
}

#[test]
fn a_contained_grid_item_keeps_the_height_its_contained_size_names() {
    // A grid item with a ratio of 1 and a `max-width`, stretched or aligned to
    // the start, stays as high as `contain-intrinsic-size` says, within a
    // `max-height` it states.
    assert_rows(&[
        (
            "<div style='display:grid;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:500px;justify-items:start;align-items:start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:500px;align-items:start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;max-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
    ]);
}

#[test]
fn a_contained_vertical_box_takes_its_width_from_its_ratio() {
    // A box in a vertical writing mode that shrinks to fit, an inline block,
    // a float, an absolutely positioned box or an inline flex container, takes
    // its width from its height through the ratio, and a `max-width` carries a
    // maximum height. The width `contain-intrinsic-size` names is its floor,
    // past a stated or carried height, unless a `max-width` caps it.
    assert_rows(&[
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;display:inline-block;writing-mode:vertical-lr'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;float:left;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;position:relative;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;position:absolute'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;min-width:0;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;display:inline-block;writing-mode:vertical-rl;padding:5px;box-sizing:border-box'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;display:inline-block;writing-mode:vertical-rl;padding:5px'></div></div>",
            &[("c", [0, 0, 60, 60])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 200, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;height:100px;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 200, 100])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;height:100px;max-width:50px;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;display:inline-block;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-height:50px;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 200, 50])],
        ),
        (
            "<div style='width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:inline-flex'></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
    ]);
}

#[test]
fn a_contained_flex_item_stretched_to_a_definite_height_takes_it() {
    // A row container with a definite height, or a `min-height`, stretches
    // its contained item to it, and the ratio gives the width from that
    // height within a `max-width`; one that wraps its items, or aligns the
    // item to the center, leaves it the height `contain-intrinsic-size`
    // names, as does a line that a taller sibling makes.
    assert_rows(&[
        (
            "<div style='display:flex;width:500px;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:400px'></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='display:flex;width:500px;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 300])],
        ),
        (
            "<div style='display:flex;width:500px;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='display:flex;width:500px;height:100px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:400px'></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:flex;width:500px;height:100px;align-items:start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:100px'></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:500px;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 300])],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:500px;height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 100px;aspect-ratio:1'></div></div>",
            &[("c", [0, 0, 200, 300])],
        ),
        (
            "<div style='display:flex;width:500px;height:300px;align-items:center'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 50, 50, 200])],
        ),
        (
            "<div style='display:flex;width:500px;height:300px;align-items:start'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;align-self:stretch'></div></div>",
            &[("c", [0, 0, 50, 300])],
        ),
        (
            "<div style='display:flex;width:500px;min-height:300px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px'></div></div>",
            &[("c", [0, 0, 50, 300])],
        ),
        (
            "<div style='display:flex;width:500px'><div id=c style='contain:size;contain-intrinsic-size:200px 200px;aspect-ratio:1;max-width:50px;margin:10px'></div><div id=d style='height:300px;width:10px'></div></div>",
            &[("c", [10, 10, 50, 280]), ("d", [70, 0, 10, 300])],
        ),
    ]);
}
