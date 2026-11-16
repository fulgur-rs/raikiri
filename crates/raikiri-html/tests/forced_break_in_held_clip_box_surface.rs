//! A forced break inside a box that clips its overflow, keeps its height and
//! ends on the page the break falls on breaks nothing: Chromium prints the
//! rest of such a box in one piece. A box that runs past the end of that page
//! still breaks, and a break at its first or last box falls outside it. The
//! page counts and positions are the ones Chromium prints for the same
//! documents (`page.pdf` with the page size taken from `@page`).

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
fn a_forced_break_inside_a_clip_box_that_fits_the_page_breaks_nothing() {
    for clip in [
        "height:60px;overflow:hidden",
        "height:60px;overflow:clip",
        "height:60px;overflow:auto",
        "height:60px;overflow-y:hidden",
        "height:60px;overflow-x:hidden",
        "max-height:60px;overflow:hidden",
        "height:100px;overflow:hidden",
    ] {
        let body = format!(
            "<div style='{clip}'><div style='height:20px'>A</div>\
             <div id=x style='height:30px;break-before:page'>B</div></div>"
        );
        assert_eq!(pages(&body), (1, vec![Some(20)]), "{clip}");
    }
    // Deeper inside the box, and after a box in it.
    let body = "<div style='height:60px;overflow:hidden'><div>\
         <div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div></div>";
    assert_eq!(pages(body), (1, vec![Some(20)]));
    let body = "<div style='height:60px;overflow:hidden'><div style='height:20px'>A</div>\
         <div style='height:30px;break-after:page'>B</div><div id=x>C</div></div>\
         <div>X</div>";
    assert_eq!(pages(body), (1, vec![Some(50)]));
}

#[test]
fn a_clip_box_that_runs_past_the_page_still_breaks() {
    // The box fits a page, but not what is left of the first one.
    let body = "<div style='height:30px'>P</div><div style='height:90px;overflow:hidden'>\
         <div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
    // Without a clip, the box that fits breaks too.
    let body = "<div style='height:60px'><div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
}

#[test]
fn the_rest_of_a_broken_clip_box_that_fits_its_page_breaks_no_more() {
    let body = "<div style='height:150px;overflow:hidden'><div style='height:20px'>A</div>\
         <div style='height:30px;break-before:page'>B</div>\
         <div id=x style='height:30px;break-before:page'>C</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(30)]));
    // The box was moved past the page end by its content before the break.
    let body = "<div style='height:150px;overflow:hidden'><div style='height:120px'>A</div>\
         <div id=x style='height:40px;break-before:page'>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(20)]));
}

#[test]
fn a_break_at_the_edge_of_a_clip_box_falls_outside_it() {
    let body = "<div style='height:20px'>P</div><div style='height:60px;overflow:hidden'>\
         <div id=x style='height:20px;break-before:page'>A</div><div>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
    let body = "<div style='height:60px;overflow:hidden'><div style='height:20px'>A</div>\
         <div style='break-after:page'>B</div></div><div id=x>X</div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
    // Through an element that generates no box, and from a flex item.
    let body = "<div style='height:20px'>P</div><div style='height:60px;overflow:hidden'>\
         <div style='display:contents'><div id=x style='height:20px;break-before:page'>A</div>\
         </div><div>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
    let body = "<div style='height:20px'>P</div>\
         <div style='height:60px;overflow:hidden;display:flex;flex-direction:column'>\
         <div id=x style='height:20px;break-before:page'>A</div><div>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
}

#[test]
fn a_clip_margin_past_the_page_does_not_break_the_box() {
    let body = "<div style='height:60px;overflow:clip;overflow-clip-margin:50px'>\
         <div style='height:20px'>A</div><div id=x style='break-before:page'>B</div></div>";
    assert_eq!(pages(body), (1, vec![Some(20)]));
}

#[test]
fn a_clip_box_with_only_a_min_height_grows_and_breaks() {
    let body = "<div style='min-height:60px;overflow:hidden'><div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div>";
    assert_eq!(pages(body), (2, vec![None, Some(0)]));
}

#[test]
fn a_relative_offset_does_not_move_the_clip_box_past_the_page() {
    // The box fits the page where the flow puts it; the offset only paints
    // it lower.
    for body in [
        "<div style='height:60px;overflow:hidden;position:relative;top:50px'>\
         <div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div>",
        "<div style='position:relative;top:50px'><div style='height:60px;overflow:hidden'>\
         <div style='height:20px'>A</div>\
         <div id=x style='height:30px;break-before:page'>B</div></div></div>",
    ] {
        assert_eq!(pages(body).1.first(), Some(&Some(70)), "{body}");
    }
}
