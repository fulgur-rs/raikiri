//! A box with an aspect ratio and neither size stated, sized around its
//! inline content, takes the width bounds its ratio carries over from its
//! height bounds: `aspect-ratio: 1; max-height: 50px` shrinks to no more than
//! 50px wide however long its lines are, and `min-height: 150px` to no less
//! than 150px. Where each box falls is what Chromium lays out for the same
//! documents.
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
fn a_ratio_box_with_inline_content_takes_the_width_bounds_its_ratio_carries() {
    // Inline-block, floated and in-flow boxes with a ratio of 1 or 2 and a
    // `max-height` or `min-height`, with padding, side borders,
    // `box-sizing`, or a `min-width` or `max-width` of their own.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0;border-left:20px solid'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 70, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0;padding:5px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 60, 60])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0;padding:5px;box-sizing:border-box'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0;box-sizing:border-box;border-left:20px solid'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-height:150px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-height:150px;max-width:120px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 120, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:2;max-height:20px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 40, 20])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50%;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0;min-width:80px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 80, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-height:20px;max-height:50px'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
    ]);
}

#[test]
fn ratio_boxes_sized_on_other_paths_keep_the_bounds_their_ratio_carries() {
    // Flex and grid containers, boxes with a block child, flex and grid
    // items, and vertical boxes, which take the carried bounds elsewhere.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-flex;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-grid;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-height:50px;min-height:0'><div style='width:100px;height:10px'></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;flex-direction:column;align-items:flex-start'><div id=c style='aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-height:150px'>x</div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='width:max-content'><div id=c style='aspect-ratio:1;max-height:50px;min-height:0'><span style='display:inline-block;width:100px;height:10px'></span></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;writing-mode:vertical-rl;aspect-ratio:1;max-width:50px;min-width:0'><span style='display:inline-block;width:10px;height:100px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;writing-mode:vertical-rl;aspect-ratio:1;min-width:150px'><span style='display:inline-block;width:10px;height:100px'></span></div></div>",
            &[("c", [0, 0, 150, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;writing-mode:vertical-rl;aspect-ratio:1;max-width:50px;min-width:0'><span style='display:inline-block;width:10px;height:100px'></span></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
    ]);
}
