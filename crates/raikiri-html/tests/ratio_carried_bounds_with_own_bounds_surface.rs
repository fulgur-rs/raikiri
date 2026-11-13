//! A box with an aspect ratio and an `auto` size takes the bounds the ratio
//! carries over from the other axis also when it states bounds of its own on
//! that axis: the tighter of the two holds. A `min-height` gives the box the
//! width the ratio relates to it, whatever `min-width` it states below that,
//! and a `max-height` caps its width below a larger `max-width`. Where each
//! box falls is what Chromium lays out for the same documents.
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
fn a_float_takes_the_bounds_its_ratio_carries_over_its_own() {
    // Square floats with `min-height: 100px` and a `min-width` of 0, 30px or
    // 150px, or a `max-width` of 60px; with a `max-height: 50px` and a wider
    // child under a `max-width` of 300px; with a `min-width` and a smaller
    // `min-height`; with padding in either box; clipping; and absolutely
    // positioned.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:30px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:150px'></div></div>",
            &[("c", [0, 300, 150, 150])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;max-width:60px'></div></div>",
            &[("c", [0, 300, 60, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:0;max-width:60px'></div></div>",
            &[("c", [0, 300, 60, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;max-height:50px;max-width:300px'><p id=a style='margin:0;width:200px;height:10px'></p></div></div>",
            &[("c", [0, 300, 50, 50]), ("a", [0, 300, 200, 10])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-width:100px;min-height:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-width:100px;min-height:30px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:0;box-sizing:border-box;padding:10px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:0;padding:10px'></div></div>",
            &[("c", [0, 300, 120, 120])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='float:left;aspect-ratio:1;min-height:100px;min-width:0;height:auto;width:auto;overflow:hidden'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='aspect-ratio:1;min-height:100px;min-width:0;position:absolute'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
    ]);
}

#[test]
fn an_inline_block_and_a_flex_item_take_the_bounds_their_ratio_carries() {
    // Inline blocks with a ratio of 2 or 1 and a `min-height` over a
    // `min-width` of 0 or 30px, with padding in either box, under a
    // `max-width`, with a `min-width` over a `min-height` of 0, or a `width`
    // they state, or an `auto` ratio that relates the content box. Flex items
    // of a row and a column that do not stretch.
    assert_rows(&[
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:2;min-height:50px;min-width:0'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;width:500px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;display:flex;flex-direction:column;align-items:flex-start'><div id=c style='aspect-ratio:1;min-width:100px;min-height:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;min-width:30px;box-sizing:border-box;padding:10px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;min-width:30px;padding:10px'></div></div>",
            &[("c", [0, 300, 120, 120])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;min-width:0;max-width:60px'></div></div>",
            &[("c", [0, 300, 60, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-width:100px;min-height:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;min-width:0;width:40px'></div></div>",
            &[("c", [0, 300, 40, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:1;min-height:100px;max-width:60px'></div></div>",
            &[("c", [0, 300, 60, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;'><div id=c style='display:inline-block;aspect-ratio:auto 2;box-sizing:border-box;padding:10px;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 180, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;min-width:0;max-width:60px'></div></div>",
            &[("c", [0, 300, 60, 100])],
        ),
        (
            "<div style='margin-top:300px;width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:auto 2;box-sizing:border-box;padding:10px;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 180, 100])],
        ),
    ]);
}

#[test]
fn an_out_of_flow_box_caps_the_minimum_its_ratio_carries_at_its_own_maximum() {
    // An absolutely positioned box with a ratio takes the minimum its ratio
    // carries over from the other axis only up to the maximum it states on
    // that axis: `min-height: 100px` gives a box with `max-width: 50px` a
    // width of 50px, not 100px, and `min-width` with `max-height` does the
    // same the other way round, with any insets, `box-sizing`, or ratio, in a
    // block or a grid containing block, and in a vertical writing mode. A
    // carried minimum below the maximum still holds.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;min-width:100px;max-height:50px'></div></div>",
            &[("c", [0, 0, 100, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px;padding:5px'></div></div>",
            &[("c", [0, 0, 60, 110])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px;padding:5px;box-sizing:border-box'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:150px;min-height:100px'></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;left:0;right:0;max-width:50px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;top:0;bottom:0;max-width:50px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='display:grid;width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;left:0;right:0;min-width:100px;max-height:50px'></div></div>",
            &[("c", [0, 0, 100, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1/2;position:absolute;max-width:80px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:2;position:absolute;max-width:150px;min-height:100px'></div></div>",
            &[("c", [0, 0, 150, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px;display:flex'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;width:30px;min-height:100px'></div></div>",
            &[("c", [0, 0, 30, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-height:50px;min-width:100px;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 100, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;width:80px;max-width:50px;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:100px;max-height:80px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
    ]);
}

#[test]
fn an_out_of_flow_box_caps_a_percentage_minimum_its_ratio_carries() {
    // The same cap where the carried minimum or the maximum is a percentage
    // or a `calc()` of the containing block: `min-height: 50%` of a 300px
    // tall containing block gives a box with `max-width: 50px` a width of
    // 50px, not 150px, and `max-width: 20%` caps a `min-height: 150px` at
    // 100px. Either way round, with padding, `box-sizing`, insets, another
    // ratio, a grid area, a vertical writing mode in the box or its
    // containing block, a box that is a grid container, and an image. A
    // carried minimum below the maximum still holds, a border-box maximum
    // below the border still gives way to it, and a stated width keeps the
    // box out of the cap.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:20%;min-height:150px'></div></div>",
            &[("c", [0, 0, 100, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:20%;min-height:50%'></div></div>",
            &[("c", [0, 0, 100, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;min-width:40%;max-height:50px'></div></div>",
            &[("c", [0, 0, 200, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;min-width:100px;max-height:10%'></div></div>",
            &[("c", [0, 0, 100, 30])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:10%'></div></div>",
            &[("c", [0, 0, 30, 30])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:calc(50% + 10px)'></div></div>",
            &[("c", [0, 0, 50, 160])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;padding:5px'></div></div>",
            &[("c", [0, 0, 60, 160])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;padding:5px;box-sizing:border-box'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;writing-mode:vertical-rl'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;aspect-ratio:2;max-width:150px;min-height:50%'></div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;left:0;right:0'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;top:0;bottom:0'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:10%;min-height:100px'></div></div>",
            &[("c", [0, 0, 50, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;min-width:100px;max-height:10%;top:0;bottom:0'></div></div>",
            &[("c", [0, 0, 100, 30])],
        ),
        (
            "<div style='display:grid;width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='display:grid;grid-template-columns:100px 400px;grid-template-rows:100px 200px;width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;grid-row:2;grid-column:2;max-width:50px;min-height:50%'></div></div>",
            &[("c", [100, 100, 50, 100])],
        ),
        (
            "<div style='display:grid;grid-template-columns:100px 400px;grid-template-rows:100px 200px;width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;grid-row:2;grid-column:2;max-width:10%;min-height:100px'></div></div>",
            &[("c", [100, 100, 40, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative;writing-mode:vertical-rl'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%'></div></div>",
            &[("c", [450, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative;writing-mode:vertical-rl'><div id=c style='aspect-ratio:1;position:absolute;max-height:50px;min-width:40%'></div></div>",
            &[("c", [300, 0, 200, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;padding:10%'></div></div>",
            &[("c", [0, 0, 150, 250])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative;display:flex'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%'></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50px;min-height:50%;display:grid'><span>x</span></div></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;max-width:50%;min-height:100%'></div></div>",
            &[("c", [0, 0, 250, 300])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='aspect-ratio:1;position:absolute;min-width:50%;max-height:calc(10% + 5px)'></div></div>",
            &[("c", [0, 0, 250, 35])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;left:0;right:0;aspect-ratio:2/1;box-sizing:border-box;border:25px solid;max-height:10%;min-width:10px'></div></div>",
            &[("c", [0, 0, 100, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><img id=c style='position:absolute;aspect-ratio:1;max-width:50px;min-height:50%' src='data:image/svg+xml,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"/>'></div>",
            &[("c", [0, 0, 50, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;max-width:50px;min-height:50%;width:30px'></div></div>",
            &[("c", [0, 0, 30, 150])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;box-sizing:border-box;border:40px solid;max-width:10%;min-height:150px'></div></div>",
            &[("c", [0, 0, 80, 150])],
        ),
    ]);
}
