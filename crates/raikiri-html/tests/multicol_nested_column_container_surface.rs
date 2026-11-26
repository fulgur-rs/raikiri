//! Where a multicol container nested in another one stands. A nested column
//! container that fits in one outer column is laid out whole there, with its
//! own columns balanced inside it, and the boxes after it fill the next outer
//! column (CSS Multi-column Layout 1, 8.1). The positions are the ones
//! Chromium gives the same documents; a box cut across columns is reported
//! by the box around all of its pieces, as `getBoundingClientRect` does.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::BTreeMap;

/// The two-column container of `inner` and a box of `after` pixels: 200px
/// wide, no gaps.
fn nested(inner: &str, heights: (u32, u32), after: u32) -> String {
    format!(
        "<div id=m style='columns:2;column-gap:0;width:200px'>\
         <div id=n style='columns:2;column-gap:0;{inner}'>\
         <div id=a style='height:{}px'></div><div id=b style='height:{}px'></div></div>\
         <div id=c style='height:{after}px'></div></div>",
        heights.0, heights.1
    )
}

/// Each box with an id as `id:x,y,w,h`, in the order the ids first appear on
/// the first page. A box with more than one fragment is the box around them.
fn boxes(body: &str) -> String {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style><body>{body}</body>"
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
    let page = result.page(0).expect("a page");
    let dom = page.dom();
    let mut order = Vec::new();
    let mut united: BTreeMap<String, (f32, f32, f32, f32)> = BTreeMap::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let r = fragment.rect();
        let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
        united
            .entry(id.to_string())
            .and_modify(|b| *b = (b.0.min(x0), b.1.min(y0), b.2.max(x1), b.3.max(y1)))
            .or_insert_with(|| {
                order.push(id.to_string());
                (x0, y0, x1, y1)
            });
    }
    order
        .iter()
        .map(|id| {
            let (x0, y0, x1, y1) = united[id];
            format!(
                "{id}:{},{},{},{}",
                x0.round(),
                y0.round(),
                (x1 - x0).round(),
                (y1 - y0).round()
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_nested_container_stays_in_one_column_and_the_next_box_fills_the_next() {
    assert_eq!(
        boxes(&nested("", (40, 40), 40)),
        "m:0,0,200,40 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40"
    );
}

#[test]
fn a_nested_container_after_a_box_starts_the_next_column() {
    assert_eq!(
        boxes(
            "<div id=m style='columns:2;column-gap:0;width:200px'>\
             <div id=c style='height:40px'></div>\
             <div id=n style='columns:2;column-gap:0'>\
             <div id=a style='height:40px'></div><div id=b style='height:40px'></div></div></div>"
        ),
        "m:0,0,200,40 c:0,0,100,40 n:100,0,100,40 a:100,0,50,40 b:150,0,50,40"
    );
}

#[test]
fn the_outer_columns_balance_around_the_nested_container() {
    // The box after the nested container is taller than it, so the columns
    // grow past the container and the box is cut between them.
    assert_eq!(
        boxes(&nested("", (10, 10), 40)),
        "m:0,0,200,25 n:0,0,100,10 a:0,0,50,10 b:50,0,50,10 c:0,0,200,25"
    );
    // The nested container balances its own columns, cutting its first box.
    assert_eq!(
        boxes(&nested("", (60, 20), 40)),
        "m:0,0,200,40 n:0,0,100,40 a:0,0,100,40 b:50,20,50,20 c:100,0,100,40"
    );
}

#[test]
fn the_edges_of_the_nested_container_go_with_it() {
    assert_eq!(
        boxes(&nested("padding:5px", (40, 40), 40)),
        "m:0,0,200,50 n:0,0,100,50 a:5,5,45,40 b:50,5,45,40 c:100,0,100,40"
    );
    assert_eq!(
        boxes(&nested("margin-bottom:10px", (40, 40), 40)),
        "m:0,0,200,45 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40"
    );
}

#[test]
fn two_nested_containers_share_the_outer_columns() {
    assert_eq!(
        boxes(
            "<div id=m style='columns:2;column-gap:0;width:200px'>\
             <div id=n style='columns:2;column-gap:0'>\
             <div id=a style='height:40px'></div><div id=b style='height:40px'></div></div>\
             <div id=c style='height:40px'></div>\
             <div id=n2 style='columns:2;column-gap:0'>\
             <div id=a2 style='height:40px'></div><div id=b2 style='height:40px'></div></div></div>"
        ),
        "m:0,0,200,60 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:0,0,200,60 \
         n2:100,20,100,40 a2:100,20,50,40 b2:150,20,50,40"
    );
}

#[test]
fn gaps_and_more_outer_columns_place_the_nested_container_the_same_way() {
    assert_eq!(
        boxes(
            "<div id=m style='columns:3;column-gap:0;width:300px'>\
             <div id=n style='columns:2;column-gap:0'>\
             <div id=a style='height:40px'></div><div id=b style='height:40px'></div></div>\
             <div id=c style='height:40px'></div><div id=d style='height:40px'></div></div>"
        ),
        "m:0,0,300,40 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40 d:200,0,100,40"
    );
    assert_eq!(
        boxes(
            "<div id=m style='columns:2;column-gap:20px;width:220px'>\
             <div id=n style='columns:2;column-gap:10px'>\
             <div id=a style='height:40px'></div><div id=b style='height:40px'></div></div>\
             <div id=c style='height:40px'></div></div>"
        ),
        "m:0,0,220,40 n:0,0,100,40 a:0,0,45,40 b:55,0,45,40 c:120,0,100,40"
    );
}

#[test]
fn a_display_contents_box_leaves_the_nested_container_in_the_stream() {
    assert_eq!(
        boxes(
            "<div id=m style='columns:2;column-gap:0;width:200px'><div style='display:contents'>\
             <div id=n style='columns:2;column-gap:0'>\
             <div id=a style='height:40px'></div><div id=b style='height:40px'></div></div>\
             <div id=c style='height:40px'></div></div></div>"
        ),
        "m:0,0,200,40 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40"
    );
}

#[test]
fn columns_a_height_limits_still_cut_the_nested_container() {
    // The outer columns cannot grow past 30px, so the nested container
    // continues into the next outer column.
    assert_eq!(
        boxes(&nested("", (40, 40), 40).replace("width:200px", "width:200px;height:30px")),
        "m:0,0,200,30 n:0,0,200,30 a:0,0,100,30 b:50,0,150,30 c:100,0,200,30"
    );
}

#[test]
fn a_flex_line_that_stretches_the_columns_limits_them_like_a_height() {
    let inside = |flex: &str| {
        boxes(&format!(
            "<div style='display:flex;{flex}'>{}</div>",
            nested("", (40, 40), 40)
        ))
    };
    assert_eq!(
        inside("height:30px"),
        "m:0,0,200,30 n:0,0,200,30 a:0,0,100,30 b:50,0,150,30 c:100,0,200,30"
    );
    assert_eq!(
        inside(""),
        "m:0,0,200,40 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40"
    );
    // A column flex item does not shrink below its content, so its columns
    // still balance around the nested container.
    assert_eq!(
        inside("flex-direction:column;height:30px"),
        "m:0,0,200,40 n:0,0,100,40 a:0,0,50,40 b:50,0,50,40 c:100,0,100,40"
    );
}
