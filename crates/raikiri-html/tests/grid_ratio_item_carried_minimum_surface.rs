//! A grid item with an aspect ratio, an `auto` size and `normal` inline
//! alignment takes the minimum width its ratio carries over from its
//! `min-height`, even where the grid area is narrower. An item that asks for
//! `stretch` is held to the area. Where each box falls is what Chromium lays out for the
//! same documents.
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
fn a_grid_item_takes_the_width_its_minimum_height_carries() {
    // Items with a ratio of 1 or 2 and a `min-height` in a 50px column,
    // under a `max-width`, with padding in either box, with a `min-width` of
    // 0, with `overflow: hidden`, in a wider grid, and items
    // that ask for `start` or `stretch`, an `auto` ratio that relates the
    // content box, a percentage `max-width`, and percentage padding in a
    // column wide enough. A `max-height` carries a maximum, and
    // an item in a 50px row with a `min-width` keeps its ratio unless it asks
    // for `stretch`.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;max-width:70px'></div></div>",
            &[("c", [0, 300, 70, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:2;min-height:100px'></div></div>",
            &[("c", [0, 300, 200, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;justify-self:stretch'></div></div>",
            &[("c", [0, 300, 50, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;justify-self:start'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;min-width:0'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;max-height:20px'></div></div>",
            &[("c", [0, 300, 20, 20])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;padding:5px'></div></div>",
            &[("c", [0, 300, 110, 110])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;box-sizing:border-box;padding:5px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;overflow:hidden'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:auto'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 300, 300])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:1fr'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 300, 300])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-rows:50px;grid-template-columns:200px'><div id=c style='aspect-ratio:1;min-width:100px'></div></div>",
            &[("c", [0, 300, 200, 200])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-rows:50px;grid-template-columns:200px'><div id=c style='aspect-ratio:1;min-width:100px;align-self:stretch'></div></div>",
            &[("c", [0, 300, 100, 50])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:auto 1;box-sizing:border-box;padding:5px;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;max-width:80%'></div></div>",
            &[("c", [0, 300, 40, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:auto 2;padding:5px;min-height:40px'></div></div>",
            &[("c", [0, 300, 90, 50])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:300px'><div id=c style='aspect-ratio:1;padding:5%;min-height:100px'></div></div>",
            &[("c", [0, 300, 300, 300])],
        ),
    ]);
}

#[test]
fn grid_items_aligned_or_in_other_tracks_take_the_carried_width() {
    // Items under `justify-items: center` or `justify-self: end`, with a
    // margin or taller content, and a flex container as an item. Items whose
    // minimum is zero or a percentage of an indefinite height, flex
    // containers with a percentage size, and a replaced item. Items in
    // `max-content` and `min-content` columns with a definite row, which the
    // row gives their width. An item with content wider than its column, and
    // a table as an item.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px;justify-items:center'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [-25, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:200px;grid-template-columns:200px;justify-items:center'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [50, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:200px;grid-template-columns:200px'><div id=c style='aspect-ratio:1;min-height:100px;justify-self:end'></div></div>",
            &[("c", [100, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px;margin-left:10px'></div></div>",
            &[("c", [10, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:200px;grid-template-columns:200px;grid-template-rows:60px'><div id=c style='aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 200, 200])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='display:flex;aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='display:flex;flex-direction:column;aspect-ratio:1;min-height:100px'><div style='height:150px'></div></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:200px'><div id=c style='aspect-ratio:16/9;min-height:0'></div></div>",
            &[("c", [0, 300, 200, 113])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:200px;justify-items:center'><div id=c style='aspect-ratio:2;min-height:0'><div style='width:40px;height:10px'></div></div></div>",
            &[("c", [80, 300, 40, 20])],
        ),
        (
            "<div style='margin-top:300px;display:inline-grid'><div id=c style='aspect-ratio:2;min-height:0'><div style='width:40px;height:10px'></div></div></div>",
            &[("c", [0, 300, 40, 20])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:200px'><div id=c style='display:flex;aspect-ratio:1;width:50%'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:300px;grid-template-columns:auto'><div id=c style='display:flex;aspect-ratio:2;width:50%'><div style='width:250px;height:10px'></div></div></div>",
            &[("c", [0, 300, 150, 75])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><img id=c style='aspect-ratio:1;min-height:100px' src='data:image/svg+xml,<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"/>'></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:50%'></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:50%'><div style='height:30px'></div></div></div>",
            &[("c", [0, 300, 50, 50])],
        ),
        (
            "<style>body{font:10px/10px Ahem} div{box-sizing:border-box}</style><div id=c style=\"display:grid;grid-template-columns:max-content max-content;grid-template-rows:100px;\"><div id=i style=\"aspect-ratio:2/1;border:25px solid;min-height:60px;\"><div style=\"width:100px;height:10px\"></div></div></div>",
            &[("c", [0, 0, 1280, 100]), ("i", [0, 0, 200, 100])],
        ),
        (
            "<style>body{font:10px/10px Ahem} div{box-sizing:border-box}</style><div id=c style=\"display:grid;grid-template-columns:max-content;grid-template-rows:100px;\"><div id=i style=\"aspect-ratio:2/1;border:25px solid;min-height:60px;\"></div></div>",
            &[("c", [0, 0, 1280, 100]), ("i", [0, 0, 200, 100])],
        ),
        (
            "<style>body{font:10px/10px Ahem} div{box-sizing:border-box}</style><div id=c style=\"display:grid;grid-template-columns:max-content;grid-template-rows:100px;\"><div id=i style=\"aspect-ratio:2/1;min-height:60px;\"><div style=\"width:100px;height:10px\"></div></div></div>",
            &[("c", [0, 0, 1280, 100]), ("i", [0, 0, 200, 100])],
        ),
        (
            "<style>body{font:10px/10px Ahem} div{box-sizing:border-box}</style><div id=c style=\"display:grid;grid-template-columns:min-content;grid-template-rows:100px;\"><div id=i style=\"aspect-ratio:2/1;border:25px solid;min-height:60px;\"><div style=\"width:100px;height:10px\"></div></div></div>",
            &[("c", [0, 0, 1280, 100]), ("i", [0, 0, 200, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='aspect-ratio:1;min-height:100px'><div style='width:150px;height:10px'></div></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
        (
            "<div style='margin-top:300px;display:grid;width:50px;grid-template-columns:50px'><div id=c style='display:table;aspect-ratio:1;min-height:100px'></div></div>",
            &[("c", [0, 300, 100, 100])],
        ),
    ]);
}
