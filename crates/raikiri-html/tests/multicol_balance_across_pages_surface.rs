//! Balanced columns whose content runs past the end of a page. Each page the
//! columns reach fills them to the end of the page, or of the container's
//! content box when it has a definite height, and only the last page balances
//! what is left (CSS Multi-column 1, 8.1). A minimum height still holds the
//! container open below the columns.
//!
//! Each row is a document and where every box with an id, or line of text,
//! lands on each page, as Chromium prints it on pages 180px wide and 160px
//! tall. The containers have two columns 40px wide with a gap of 20px, so the
//! second column starts at x=60.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page of `body`: where the pieces of every box with an id and every
/// line of text start, as `id=x,y` and `text@x,y`, column by column from the
/// top.
fn pages(body: &str) -> Vec<String> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 160px;margin:0}}\
         body{{margin:0;font:20px/20px Ahem}}\
         .mc{{width:100px;column-count:2;column-gap:20px;orphans:1;widows:1}}</style>{body}"
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
    (0..result.page_count())
        .map(|index| {
            let page = result.page(index).unwrap();
            let dom = page.dom();
            let mut placed: Vec<(i32, i32, String)> = page
                .fragments()
                .filter_map(|fragment| {
                    let id = dom.attr(fragment.node(), "id")?;
                    let rect = fragment.rect();
                    let (x, y) = (rect.x.round() as i32, rect.y.round() as i32);
                    Some((x, y, format!("{id}={x},{y}")))
                })
                .collect();
            // Ahem's baseline sits 16px below the top of a 20px line.
            placed.extend(page.text_runs().iter().map(|run| {
                let (x, y) = (
                    run.origin.0.round() as i32,
                    (run.origin.1 - 16.0).round() as i32,
                );
                (x, y, format!("{}@{x},{y}", run.text))
            }));
            placed.sort();
            placed
                .into_iter()
                .map(|(.., text)| text)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// `count` boxes `height` tall with ids from `a`, each with `style` too.
fn boxes(count: u8, height: u32, style: &str) -> String {
    (0..count)
        .map(|index| {
            let id = char::from(b'a' + index);
            format!("<div id={id} style='height:{height}px;{style}'></div>")
        })
        .collect()
}

/// Lines of text `A`, `B`, and so on, one letter each.
fn lines(count: u8) -> String {
    (0..count)
        .map(|index| char::from(b'A' + index).to_string())
        .collect::<Vec<_>>()
        .join("<br>")
}

const AFTER: &str = "<div id=z style='height:10px'></div>";

#[test]
fn auto_height_columns_fill_the_rest_of_the_page_and_balance_on_the_next() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,120 b=0,150 b=60,120 c=60,140",
            "c=0,0 d=0,10 z=0,20 d=60,0",
        ]
    );
}

#[test]
fn a_minimum_height_holds_the_container_open_below_the_columns() {
    let body = format!(
        "<div style='height:120px'></div><div class=mc style='min-height:100px'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,120 b=0,150 b=60,120 c=60,140",
            "c=0,0 d=0,10 z=0,60 d=60,0",
        ]
    );
}

#[test]
fn columns_that_fill_in_turn_fill_the_page_despite_a_minimum_height() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='min-height:100px;column-fill:auto'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        ["a=0,100 b=0,130 c=60,100 d=60,130", "z=0,40"]
    );
}

#[test]
fn only_the_last_page_balances_many_boxes() {
    let body = format!(
        "<div style='height:100px'></div><div class=mc>{}</div>{AFTER}",
        boxes(14, 30, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,100 b=0,130 c=60,100 d=60,130",
            "e=0,0 f=0,30 g=0,60 h=0,90 i=0,120 z=0,150 \
             j=60,0 k=60,30 l=60,60 m=60,90 n=60,120",
        ]
    );
}

#[test]
fn margins_are_truncated_at_the_breaks_of_the_page_filled_columns() {
    let body = format!(
        "<div style='height:100px'></div><div class=mc>{}</div>{AFTER}",
        boxes(5, 30, "margin:10px 0")
    );
    assert_eq!(
        pages(&body),
        ["a=0,110 b=0,150 b=60,100 c=60,130", "d=0,0 z=0,40 e=60,0"]
    );
}

#[test]
fn the_edges_of_the_container_stay_around_the_page_filled_columns() {
    let body = format!(
        "<div style='height:100px'></div>\
         <div class=mc style='padding:10px 0;border:5px solid'>{}</div>{AFTER}",
        boxes(4, 30, "")
    );
    assert_eq!(
        pages(&body),
        ["a=5,115 b=5,145 b=65,115 c=65,130", "z=0,30 d=5,0 d=65,0"]
    );
}

#[test]
fn definite_height_columns_fill_each_page_up_to_the_content_box() {
    let body = format!(
        "<div class=mc style='height:300px'>{}</div>",
        boxes(14, 40, "")
    );
    assert_eq!(
        pages(&body),
        [
            "a=0,0 b=0,40 c=0,80 d=0,120 e=60,0 f=60,40 g=60,80 h=60,120",
            "i=0,0 j=0,40 k=0,80 l=60,0 m=60,40 n=60,80",
        ]
    );
}

#[test]
fn definite_height_columns_of_a_paragraph_fill_each_page() {
    let body = format!(
        "<div class=mc style='height:300px'><p style='margin:0'>{}</p></div>",
        lines(20)
    );
    assert_eq!(
        pages(&body),
        [
            "A@0,0 B@0,20 C@0,40 D@0,60 E@0,80 F@0,100 G@0,120 H@0,140 \
             I@60,0 J@60,20 K@60,40 L@60,60 M@60,80 N@60,100 O@60,120 P@60,140",
            "Q@0,0 R@0,20 S@60,0 T@60,20",
        ]
    );
}
