//! A box that pagination pushes down inside a fixed-height box clipping its
//! overflow is cut by that clip where the clipping box ends, not where the
//! pushed box would end: the hidden part reaches no page. The page counts and
//! positions are the ones Chromium prints for the same documents (`page.pdf`
//! with the page size taken from `@page`).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The number of pages, and the top of the first fragment of the element with
/// the id `x` on each page (`None` for a page without any), on pages 100px
/// tall.
fn pages(body: &str) -> (usize, Vec<Option<i32>>) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}</style>{body}"
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
    let tops: Vec<Option<i32>> = result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some("x"))
                .map(|fragment| fragment.rect().y.round() as i32)
        })
        .collect();
    (tops.len(), tops)
}

#[test]
fn a_box_pushed_past_the_clip_adds_no_page() {
    for clip in [
        "height:150px;overflow:hidden",
        "height:150px;overflow:clip",
        "height:150px;overflow:auto",
        "height:150px;overflow-y:hidden",
        "max-height:150px;overflow:hidden",
    ] {
        // The tall box does not fit below the first block and moves to the
        // next page, where the clip ends 50px down.
        let body = format!(
            "<div style='{clip}'><div style='height:20px'>A</div>\
             <div style='height:300px;contain:size'>B</div></div><div id=x>X</div>"
        );
        assert_eq!(pages(&body), (2, vec![None, Some(50)]), "{clip}");
    }
}

#[test]
fn a_box_a_forced_break_pushes_past_the_clip_adds_no_page() {
    let body = "<div style='height:150px;overflow:hidden'><div style='height:20px'>A</div>\
         <div id=x style='height:170px;break-before:page'>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
    // The clip of a box nested deeper still cuts it.
    let body = "<div style='height:150px;overflow:hidden'><div>\
         <div style='height:20px'>A</div>\
         <div id=x style='height:300px;contain:size'>B</div></div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
}

#[test]
fn without_a_clip_the_pushed_box_still_reaches_its_pages() {
    let body = "<div style='height:150px'><div style='height:20px'>A</div>\
         <div id=x style='height:300px;contain:size'>B</div></div>";
    assert_eq!(pages(body), (4, vec![None, Some(0), Some(0), Some(0)]));
}
