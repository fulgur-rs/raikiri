//! A forced break inside a box whose height does not follow its content.
//! The box fills the rest of the page it breaks on and continues on the next
//! page with the rest of its height, so the content after the break overflows
//! it and the flow after the box stays where its height puts it. A box with
//! `min-height` grows to hold the moved content only past that height. The
//! positions are the ones Chromium prints for the same documents (`page.pdf`
//! with the page size taken from `@page`).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The vertical extent `(top, bottom)` of the first fragment of the element
/// with the id `id` on each page, `None` for a page without any, on pages 100px
/// tall.
fn extents(body: &str, id: &str) -> Vec<Option<(i32, i32)>> {
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
    result
        .pages()
        .map(|page| {
            let dom = page.dom();
            page.fragments()
                .filter(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.y.round() as i32, (rect.y + rect.height).round() as i32)
                })
                .next()
        })
        .collect()
}

/// A box `style` holding a 20px block and, after a forced break, a block
/// `tail` tall, followed by the paragraph `x`.
fn broken_box(style: &str, tail: u32) -> String {
    format!(
        "<div id=box style='{style}'><div style='height:20px'>A</div>\
         <div style='height:{tail}px;break-before:page'>B</div></div><div id=x>X</div>"
    )
}

#[test]
fn a_set_height_keeps_the_flow_after_the_box_in_place() {
    for style in [
        "height:150px",
        "height:150px;overflow:hidden",
        "height:150px;overflow:clip",
        "height:150px;overflow:auto",
    ] {
        let body = broken_box(style, 70);
        assert_eq!(
            extents(&body, "box"),
            [Some((0, 100)), Some((0, 50))],
            "{style}"
        );
        assert_eq!(extents(&body, "x"), [None, Some((50, 70))], "{style}");
    }
}

#[test]
fn content_past_the_set_height_overflows_without_moving_the_flow() {
    // The box ends on the first page, and the block after the break goes on
    // to the next one alone.
    let body = broken_box("height:60px", 70);
    assert_eq!(extents(&body, "box"), [Some((0, 60)), None]);
    assert_eq!(extents(&body, "x"), [Some((60, 80)), None]);
    // A box that starts below other content keeps its end too.
    let body = format!(
        "<div style='height:40px'>P</div>{}",
        broken_box("height:100px", 30)
    );
    assert_eq!(extents(&body, "x"), [None, Some((40, 60))]);
}

#[test]
fn each_forced_break_leaves_the_set_height_in_place() {
    let body = "<div id=box style='height:250px'><div style='height:20px'>A</div>\
         <div style='height:30px;break-before:page'>B</div>\
         <div style='break-before:page'>C</div></div><div id=x>X</div>";
    assert_eq!(extents(body, "x"), [None, None, Some((50, 70))]);
    let body = "<div id=box style='height:150px'><div style='height:20px'>A</div>\
         <div style='height:30px;break-after:page'>B</div><div>C</div></div>\
         <div id=x>X</div>";
    assert_eq!(extents(body, "x"), [None, Some((50, 70))]);
}

#[test]
fn height_bounds_limit_how_far_the_box_grows() {
    // `min-height` holds the moved content until it reaches past it.
    let body = broken_box("min-height:150px", 30);
    assert_eq!(extents(&body, "x"), [None, Some((50, 70))]);
    let body = broken_box("min-height:120px", 70);
    assert_eq!(extents(&body, "box"), [Some((0, 100)), Some((0, 70))]);
    assert_eq!(extents(&body, "x"), [None, Some((70, 90))]);
    // `max-height` caps the height before the break.
    let body = broken_box("height:120px;max-height:80px", 30);
    assert_eq!(extents(&body, "x"), [Some((80, 100)), None]);
}

#[test]
fn a_bounded_box_measures_the_growth_of_the_boxes_inside_it() {
    // The inner box grows with the moved content; the box around it holds
    // that growth past its `min-height`.
    let inner = "<div><div style='height:20px'>A</div>\
         <div style='height:70px;break-before:page'>B</div></div>";
    let body = format!("<div style='min-height:120px'>{inner}</div><div id=x>X</div>");
    assert_eq!(extents(&body, "x"), [None, Some((70, 90))]);
    let body = format!(
        "<div style='min-height:120px'>{inner}<div style='height:10px'>C</div></div>\
         <div id=x>X</div>"
    );
    assert_eq!(extents(&body, "x"), [None, Some((80, 100))]);
    // Bottom padding keeps the inner box's margin inside the outer one.
    let body = format!(
        "<div style='min-height:150px;padding-bottom:10px'>\
         <div style='margin-bottom:15px'>{inner}</div></div><div id=x>X</div>"
    );
    assert_eq!(extents(&body, "x"), [None, None, Some((0, 20))]);
    let body = format!("<div style='max-height:130px'>{inner}</div><div id=x>X</div>");
    assert_eq!(extents(&body, "x"), [None, Some((30, 50))]);
    // A set height around a box that grows keeps its end.
    let body = format!("<div style='height:150px'>{inner}<div>C</div></div><div id=x>X</div>");
    assert_eq!(extents(&body, "x"), [None, Some((50, 70))]);
}

#[test]
fn size_containment_keeps_the_flow_after_the_box_in_place() {
    let body = broken_box("contain:size;contain-intrinsic-size:auto 150px", 70);
    assert_eq!(extents(&body, "x"), [None, Some((50, 70))]);
}

#[test]
fn a_box_holding_its_floats_grows_past_them() {
    let float = "<div style='float:left;width:10px;height:90px'></div>";
    for style in ["display:flow-root", "display:flow-root;min-height:50px"] {
        let body = format!(
            "<div style='{style}'>{float}<div style='height:20px'>A</div>\
             <div style='height:20px;break-before:page'>B</div></div><div id=x>X</div>"
        );
        assert_eq!(extents(&body, "x"), [None, Some((20, 40))], "{style}");
    }
}
