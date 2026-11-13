//! A grid item with an aspect ratio and an `auto` height that is not
//! stretched to the height of its area takes the height of its content where
//! that is taller than the height its ratio gives: `normal` alignment
//! behaves as `start` for such a box. A row with a definite height gives the
//! width the ratio carries over, and the content still raises the height.
//! Where each box falls is what Chromium lays out for the same documents.
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
fn an_unstretched_ratio_grid_item_grows_to_the_height_of_its_content() {
    // `start`, `center`, `end` and baseline alignment, `auto` margins,
    // and `normal` alignment in definite rows.
    assert_rows(&[
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:stretch'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;align-items:start'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:baseline'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;margin-top:auto'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:center'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;align-items:stretch'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:end'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:100px'><div id=c style='aspect-ratio:1;align-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;margin-bottom:auto'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:50px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:stretch'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:stretch'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:start'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:start'><div style='height:20px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1'><div style='height:20px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;justify-self:start'><div style='height:20px'></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:center'><div style='height:20px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;max-height:60px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 60, 60])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;overflow:hidden'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;min-height:0'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 100])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;padding:10px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 220])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;display:flex'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:100px'><div id=c style='aspect-ratio:1;align-self:start;width:50px'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:end'><div style='height:20px'></div></div></div>",
            &[("c", [0, 250, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:center'><div style='height:20px'></div></div></div>",
            &[("c", [0, 275, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:100px;grid-template-rows:50px'><div id=c style='aspect-ratio:1;align-self:end'><div style='height:200px'></div></div></div>",
            &[("c", [0, 150, 100, 200])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:100px;grid-template-rows:150px'><div id=c style='aspect-ratio:1;align-self:end'><div style='height:20px'></div></div></div>",
            &[("c", [0, 350, 100, 100])],
        ),
    ]);
}

#[test]
fn a_ratio_grid_item_counts_percentage_padding_inside_it() {
    // A child's `padding-top: 50%` resolves against the width of the item,
    // and the content it makes taller raises the height of the item.
    assert_rows(&[
        (
            "<div style='display:grid;width:300px;grid-template-columns:200px'><div id=c style='aspect-ratio:1'><div style='padding-top:50%'></div><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 300])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;justify-self:start'><div style='padding-top:50%'></div><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 300])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='padding-top:50%'></div><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 300])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;align-self:start'><div style='padding-top:50%'></div><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 0, 200])],
        ),
    ]);
}

#[test]
fn a_definite_row_gives_the_width_of_a_ratio_grid_item() {
    assert_rows(&[
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:50px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 50, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1;width:50%'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 100, 200])],
        ),
        (
            "<div style='display:grid;width:300px;height:50px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:250px'><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 0, 250, 250])],
        ),
    ]);
}

#[test]
fn the_rows_a_grid_stretches_give_the_width_of_a_ratio_grid_item() {
    // A fixed row before an implicit `auto` one, `auto` rows a grid of a
    // stated height or minimum height stretches, rows `align-content` packs,
    // and margins inside a fixed row.
    assert_rows(&[
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:50px'><div style='height:10px'></div><div id=c style='aspect-ratio:1'><div style='height:200px'></div></div></div>",
            &[("c", [0, 50, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;min-height:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:50px'></div></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='display:grid;width:300px;height:300px;align-content:start;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:50px'></div></div></div>",
            &[("c", [0, 0, 50, 50])],
        ),
        (
            "<div style='height:400px'><div style='display:grid;width:300px;height:50%;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:50px'></div></div></div></div>",
            &[("c", [0, 0, 200, 200])],
        ),
        (
            "<div style='display:grid;width:300px;height:300px;grid-template-columns:max-content'><div id=c style='aspect-ratio:1'><div style='height:50px'></div></div></div>",
            &[("c", [0, 0, 300, 300])],
        ),
        (
            "<div style='display:grid;width:300px;grid-template-columns:max-content;grid-template-rows:100px'><div id=c style='aspect-ratio:1;margin-top:10px;margin-bottom:10px'><div style='height:20px'></div></div></div>",
            &[("c", [0, 10, 80, 80])],
        ),
    ]);
}
