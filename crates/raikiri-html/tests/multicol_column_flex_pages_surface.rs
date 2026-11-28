//! A column flex container in a multi-column container whose items, stacked
//! in the container as if in one column, reach past the page end: the columns
//! place the items, so the page does not break between them, and nothing
//! moves to a page of its own. The values are the ones Chromium gives for the
//! same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page count of `body`, laid out on 800x600 pages with no margins, and
/// the page and the `(x, y)` of every fragment of the element `id`.
fn pages_and_places(body: &str, id: &str) -> (u32, Vec<(u32, i32, i32)>) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}} body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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
    let mut places = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                places.push((index, rect.x.round() as i32, rect.y.round() as i32));
            }
        }
    }
    (result.page_count(), places)
}

#[test]
fn items_past_the_page_end_stay_in_their_columns() {
    let items: String = (0..11)
        .map(|i| format!("<div id=m{i} style='height:60px'></div>"))
        .collect();
    // Columns of 120px hold two items each; the last one starts the sixth
    // column, at the top of the page, although it is 600px down the
    // container.
    let filled = format!(
        "<div style='columns:10;column-gap:0;width:1000px;height:120px;column-fill:auto'>\
         <div style='display:flex;flex-direction:column'>{items}</div></div>"
    );
    let (pages, places) = pages_and_places(&filled, "m10");
    assert_eq!(pages, 1);
    assert_eq!(places.last(), Some(&(0, 500, 0)));
    // Balanced columns of 66px hold one each, and the last one ends the
    // tenth column.
    let balanced = format!(
        "<div style='columns:10;column-gap:0;width:1000px'>\
         <div style='display:flex;flex-direction:column'>{items}</div></div>"
    );
    let (pages, places) = pages_and_places(&balanced, "m10");
    assert_eq!(pages, 1);
    assert_eq!(places.last(), Some(&(0, 900, 6)));
}

#[test]
fn columns_past_the_page_end_show_each_item_once() {
    // Forty items in two balanced columns fill two pages, and no item is
    // printed again on a page of its own.
    let items: String = (0..40)
        .map(|i| format!("<div id=m{i} style='height:60px'></div>"))
        .collect();
    let body = format!(
        "<div style='columns:2;column-gap:0;width:200px'>\
         <div style='display:flex;flex-direction:column'>{items}</div></div>"
    );
    for i in 0..40 {
        let (pages, places) = pages_and_places(&body, &format!("m{i}"));
        assert_eq!(pages, 2);
        assert_eq!(places.len(), 1, "m{i}: {places:?}");
    }
}

#[test]
fn a_spanner_breaks_between_its_items() {
    // A spanner lies in the flow of the pages, so the page breaks between
    // its items as in any column flex container.
    let items: String = (0..20)
        .map(|i| format!("<div id=m{i} style='height:60px'></div>"))
        .collect();
    let body = format!(
        "<div style='columns:2'>\
         <div style='column-span:all;display:flex;flex-direction:column'>{items}</div></div>"
    );
    assert_eq!(pages_and_places(&body, "m9"), (2, vec![(0, 0, 540)]));
    assert_eq!(pages_and_places(&body, "m10"), (2, vec![(1, 0, 0)]));
}
