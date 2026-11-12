//! A flex item with an aspect ratio takes the main size the flex algorithm
//! flexes it to and the cross size its line stretches it to, held only to the
//! limits it states. A minimum the ratio carries over from the other axis does
//! not raise them: a `min-height` gives such an item its height, not a width it
//! keeps when its row is narrower. Its automatic minimum still keeps an item
//! that does not scroll from shrinking below its content. A block child takes
//! the width the ratio carries from its `min-height`. Where each box falls is
//! what Chromium lays out for the same documents.
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
fn a_ratio_item_flexes_below_the_width_its_min_height_carries() {
    // Square items with `min-height: 100px` in a row 50px wide: one that
    // clips shrinks to the row, one that does not keeps the automatic minimum
    // its content gives, and a `max-width` or `flex-shrink: 0` sets the width.
    // One that clips and grows from a zero basis takes the row's width. Square
    // items with `min-width: 100px` that clip take the height of a column
    // 50px or 40px tall.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 50, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;max-width:80px'></div></div>",
            &[("c", [0, 300, 80, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;flex-shrink:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:300px;height:50px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-width:100px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px'></div><div id=d style='width:20px;height:5px'></div></div>",
            &[("c", [0, 300, 100, 100]), ("d", [100, 300, 0, 5])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px'><div id=c style='aspect-ratio:1;min-height:100px;flex-grow:1;flex-basis:0;overflow:hidden'></div></div>",
            &[("c", [0, 300, 50, 300])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:50px;height:40px'><div id=c style='aspect-ratio:1;min-width:100px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 100, 40])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:50px;height:300px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;overflow:hidden;flex-wrap:wrap'></div></div>",
            &[("c", [0, 300, 50, 100])],
        ),
    ]);
}

#[test]
fn a_ratio_item_stretches_across_below_the_size_its_minimum_carries() {
    // Square items stretched across a column 50px wide or a row 50px tall,
    // with a `min-height` or a `min-width` of 100px, or a maximum of 20px.
    // An item not stretched, or in a wrapping column, takes the size its
    // minimum carries, as a block child does.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:50px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 50, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:50px;height:300px'><div id=c style='aspect-ratio:1;max-height:20px'></div></div>",
            &[("c", [0, 300, 50, 20])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:300px;height:50px'><div id=c style='aspect-ratio:1;min-width:100px'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:300px;height:50px'><div id=c style='aspect-ratio:1;max-width:20px'></div></div>",
            &[("c", [0, 300, 20, 50])],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:300px;height:50px;align-items:flex-start'><div id=c style='aspect-ratio:1;min-width:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:50px;height:300px'><div id=c style='aspect-ratio:1;min-height:100px;flex-grow:1'></div></div>",
            &[("c", [0, 300, 50, 300])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-wrap:wrap;flex-direction:column;width:50px;height:300px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:50px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;width:50px'><div id=c style='aspect-ratio:1;min-height:100px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
    ]);
}
