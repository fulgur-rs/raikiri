//! A box fixed to the page and placed at its static position along the block
//! axis is drawn from the page that holds that position on, at the same place
//! on each of those pages, and not on the pages before it. A box placed by an
//! inset repeats on every page. Where the box falls is what Chromium prints
//! for the same documents on pages 100px tall.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

/// A document body and the page and top-left pixel of the blue box on each
/// page that draws it.
type Row = (&'static str, &'static [(u32, u32, u32)]);

/// The page and top-left pixel of the blue box on each page of `body` that
/// draws it, laid out on 100x100 pages without margins.
fn blue_box_places_with(body: &str, margin: u32) -> Vec<(u32, u32, u32)> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:{margin}px}}body{{margin:0}}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let mut places = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("the page exists");
        let (doc, cascade, page_box, origin) = page.paint_inputs();
        let mut scene = Scene::new();
        raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
            .unwrap();
        let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
            |out| out.append_scene(scene, Affine::IDENTITY),
            100,
            100,
        );
        let blue = (0..100u32 * 100).find(|&at| {
            let pixel = &pixels[at as usize * 4..at as usize * 4 + 3];
            pixel[0] < 40 && pixel[1] < 40 && pixel[2] > 200
        });
        if let Some(at) = blue {
            places.push((index, at % 100, at / 100));
        }
    }
    places
}

fn blue_box_places(body: &str) -> Vec<(u32, u32, u32)> {
    blue_box_places_with(body, 0)
}

#[test]
fn a_fixed_box_at_its_static_position_is_drawn_from_the_page_that_holds_it() {
    // The static position on the first page, on a later one, after a
    // paragraph pushed to the next page, across a page end, beside a margin
    // and in columns; and a box placed by `top`, which every page draws.
    let rows: &[Row] = &[
        (
            "<div style='height:20px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:200px'></div>",
            &[(0, 40, 20), (1, 40, 20), (2, 40, 20)],
        ),
        (
            "<div style='height:120px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:200px'></div>",
            &[(1, 40, 20), (2, 40, 20), (3, 40, 20)],
        ),
        (
            "<div style='height:220px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:50px'></div>",
            &[(2, 40, 20)],
        ),
        (
            "<div style='height:94px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div>",
            &[(0, 40, 94), (1, 40, 94)],
        ),
        (
            "<div style='height:90px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:5px'></div></div><div style='height:120px'></div>",
            &[(0, 40, 90), (1, 40, 90), (2, 40, 90)],
        ),
        (
            "<div style='height:150px'></div><div style='margin-left:10px'><div style='position:fixed;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:120px'></div>",
            &[(1, 10, 50), (2, 10, 50)],
        ),
        (
            "<div style='height:150px'></div><div style='columns:2;column-gap:0;width:100px'><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:120px'></div>",
            &[(1, 40, 50), (2, 40, 50)],
        ),
        (
            "<div style='height:120px'></div><div><div style='position:fixed;top:5px;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:200px'></div>",
            &[(0, 40, 5), (1, 40, 5), (2, 40, 5), (3, 40, 5)],
        ),
    ];
    for &(body, expected) in rows {
        assert_eq!(blue_box_places(body), expected, "{body}");
    }
}

#[test]
fn the_pages_before_a_fixed_box_leave_it_out_of_their_margins() {
    // With 20px page margins the static position lies 5px into the second
    // page area. The first page does not draw the box in its bottom margin,
    // and every page from the second draws it 5px below its page area's top.
    let body = "<div style='height:65px'></div><div><div style='position:fixed;left:40px;width:4px;height:4px;background:blue'></div><div style='height:30px'></div></div><div style='height:120px'></div>";
    assert_eq!(
        blue_box_places_with(body, 20),
        [(1, 60, 25), (2, 60, 25), (3, 60, 25)]
    );
}
