//! A grid item with an aspect ratio and no size stated, stretched to the
//! height of its area, takes the width the height of its content gives through the ratio, if that is wider than its
//! content, for the width it contributes to its column and the width it is
//! aligned at: a ratio of 1 around a child 200px high makes a `max-content`
//! column 200px wide. Outside a grid the same box shrinks to the width of
//! its content. Where each box falls is what Chromium lays out for the same
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
fn a_ratio_grid_item_takes_its_width_from_the_height_of_its_content() {
    // Items in `min-content`, `max-content`, `auto`, `fit-content()` and
    // fixed columns, with a ratio of 1 or 2, padding, `box-sizing`, a
    // `max-width`, `overflow: hidden`, start alignment or `min-width: 0`,
    // and in an inline grid.
    assert_rows(&[
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:min-content'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;max-width:50px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:min-content'><div id=c style='aspect-ratio:1;max-width:50px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:100px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:auto'><div id=c style='aspect-ratio:2'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 400, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:fit-content(100px)'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;padding:10px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 220, 220])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;box-sizing:border-box;padding:10px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 220, 220])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;overflow:hidden'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 0])],
        ),
        (
            "<div style='display:grid;width:100px'><div id=c style='aspect-ratio:1;justify-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:100px'><div id=c style='aspect-ratio:1;min-width:0'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:inline-grid'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:flex;width:300px;align-items:flex-start'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:300px'><div id=c style='aspect-ratio:1;max-width:50px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;max-width:50px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;max-width:50px;align-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;max-width:50px;justify-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;max-width:50px;align-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='width:max-content;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='display:flex;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='display:grid;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;contain:size'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 0])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;min-width:10px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:stretch'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;align-items:stretch'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;width:50%'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
    ]);
}

#[test]
fn a_ratio_box_outside_a_grid_keeps_the_width_of_its_content() {
    // Inline-block, floated, absolutely positioned and sized-by-keyword
    // boxes, flex and table items, and an absolutely positioned child of a
    // grid, which take no width from the height of
    // their content.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='position:absolute;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='width:max-content;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='width:min-content;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='width:fit-content;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;flex-direction:column;align-items:flex-start'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:table'><div style='display:table-cell'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1'><div style='height:200px;width:50px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1'><div style='height:50px;width:100px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
    ]);
}
