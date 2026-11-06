//! A box that a page break cuts paints only inside the page area: the
//! background, shadows and border of an absolutely positioned box, which is
//! laid out whole, are cut to it where the box continues from the page before
//! or goes on to the next one, and the outline of any box is cut to it, with a
//! bar along the cut edge where a break cuts the box (Chromium's way).

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const LIME: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// Paint page `index` of `body`, laid out on 50×50 pages with a 10px margin
/// (a 30×30 page area), and return the RGBA pixels.
fn raster_page(body: &str, index: u32) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:10px}}body{{margin:0}}</style>{body}"
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
    let page = result.page(index).expect("the page exists");
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        50,
        50,
    )
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let index = (y * 50 + x) * 4;
    [pixels[index], pixels[index + 1], pixels[index + 2]]
}

/// A box 80px tall in the 30px page area, so on the first three pages.
const TALL: &str = "<div style='position:absolute;top:0;left:0;width:30px;background:lime'>\
                    <div style='height:80px'></div></div>";

#[test]
fn a_box_that_goes_on_to_the_next_page_is_cut_at_the_page_area() {
    let pixels = raster_page(TALL, 0);
    assert_eq!(pixel(&pixels, 20, 25), LIME);
    assert_ne!(pixel(&pixels, 20, 45), LIME, "the bottom margin");
}

#[test]
fn a_box_that_goes_on_to_the_next_page_and_comes_from_the_page_before_is_cut_on_both_sides() {
    let pixels = raster_page(TALL, 1);
    assert_eq!(pixel(&pixels, 20, 25), LIME);
    assert_ne!(pixel(&pixels, 20, 5), LIME, "the top margin");
    assert_ne!(pixel(&pixels, 20, 45), LIME, "the bottom margin");
}

#[test]
fn a_box_that_comes_from_the_page_before_is_cut_at_the_top_of_the_page_area() {
    let pixels = raster_page(TALL, 2);
    assert_eq!(pixel(&pixels, 20, 15), LIME);
    assert_ne!(pixel(&pixels, 20, 5), LIME, "the top margin");
}

#[test]
fn a_box_in_the_top_margin_of_the_first_page_is_not_cut() {
    let pixels = raster_page(
        "<div style='position:absolute;top:-5px;left:0;width:30px;height:20px;background:lime'></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 20, 7), LIME, "the top margin");
    assert_eq!(pixel(&pixels, 20, 20), LIME);
}

#[test]
fn a_fixed_box_in_the_margin_is_not_cut() {
    let pixels = raster_page(
        "<div style='position:fixed;top:-5px;left:0;width:30px;height:80px;background:lime'></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 20, 7), LIME, "the top margin");
}

/// An absolutely positioned box with a 2px border, 80px of content and no
/// background: 84px tall, so on the first three pages (30px page area).
const TALL_BORDERED: &str = "<div style='position:absolute;top:0;left:0;width:26px;border:2px solid blue'>\
                             <div style='height:80px'></div></div>";

#[test]
fn the_border_of_a_box_that_goes_on_to_the_next_page_is_cut_at_the_page_area() {
    let pixels = raster_page(TALL_BORDERED, 0);
    assert_eq!(pixel(&pixels, 11, 25), BLUE, "the left border");
    assert_eq!(pixel(&pixels, 25, 10), BLUE, "the top border");
    assert_ne!(pixel(&pixels, 11, 45), BLUE, "the bottom margin");
}

#[test]
fn the_border_of_a_box_that_comes_from_the_page_before_and_goes_on_is_cut_on_both_sides() {
    let pixels = raster_page(TALL_BORDERED, 1);
    assert_eq!(pixel(&pixels, 11, 25), BLUE, "the left border");
    assert_ne!(pixel(&pixels, 11, 5), BLUE, "the top margin");
    assert_ne!(pixel(&pixels, 11, 45), BLUE, "the bottom margin");
    assert_ne!(pixel(&pixels, 25, 10), BLUE, "there is no top border here");
}

#[test]
fn the_border_of_a_box_that_comes_from_the_page_before_is_cut_at_the_top_of_the_page_area() {
    let pixels = raster_page(TALL_BORDERED, 2);
    assert_eq!(pixel(&pixels, 11, 15), BLUE, "the left border");
    assert_eq!(pixel(&pixels, 25, 32), BLUE, "the bottom border");
    assert_ne!(pixel(&pixels, 11, 5), BLUE, "the top margin");
}

#[test]
fn the_border_of_a_box_with_a_background_is_cut_at_the_page_area() {
    let pixels = raster_page(
        "<div style='position:absolute;top:0;left:0;width:26px;border:2px solid blue;background:lime'>\
         <div style='height:80px'></div></div>",
        1,
    );
    assert_eq!(pixel(&pixels, 11, 25), BLUE, "the left border");
    assert_eq!(pixel(&pixels, 20, 25), LIME);
    assert_ne!(pixel(&pixels, 11, 45), BLUE, "the bottom margin");
}

#[test]
fn the_border_of_a_block_that_a_page_break_cuts_stays_in_the_page_area() {
    let body = "<div style='width:22px;height:80px;border:4px solid blue'></div>";
    let first = raster_page(body, 0);
    assert_eq!(first[(11 * 50 + 20) * 4..][..3], BLUE, "the top border");
    assert_ne!(pixel(&first, 13, 45), BLUE, "the bottom margin");
    let middle = raster_page(body, 1);
    assert_eq!(pixel(&middle, 13, 25), BLUE, "the left border");
    assert_ne!(pixel(&middle, 13, 5), BLUE, "the top margin");
    assert_ne!(pixel(&middle, 13, 45), BLUE, "the bottom margin");
    assert_ne!(pixel(&middle, 20, 11), BLUE, "there is no top border here");
}

/// The three pages of a box with a 4px outline that a page break cuts: the
/// ring follows the box on the sides, and where the break cuts the box it is a
/// bar along the edge of the page area, inside it.
fn assert_outline_follows_page_breaks(body: &str) {
    let first = raster_page(body, 0);
    assert_eq!(pixel(&first, 13, 25), BLUE, "the left ring");
    assert_eq!(
        pixel(&first, 20, 37),
        BLUE,
        "the bar at the bottom of the page area"
    );
    assert_ne!(
        pixel(&first, 20, 7),
        BLUE,
        "the top ring lies in the margin"
    );
    assert_ne!(pixel(&first, 20, 45), BLUE, "the bottom margin");
    let middle = raster_page(body, 1);
    assert_eq!(pixel(&middle, 13, 25), BLUE, "the left ring");
    assert_eq!(
        pixel(&middle, 20, 11),
        BLUE,
        "the bar at the top of the page area"
    );
    assert_eq!(
        pixel(&middle, 20, 37),
        BLUE,
        "the bar at the bottom of the page area"
    );
    assert_ne!(pixel(&middle, 13, 5), BLUE, "the top margin");
    assert_ne!(pixel(&middle, 13, 45), BLUE, "the bottom margin");
    let last = raster_page(body, 2);
    assert_eq!(
        pixel(&last, 20, 11),
        BLUE,
        "the bar at the top of the page area"
    );
    assert_eq!(pixel(&last, 20, 31), BLUE, "the bottom ring below the box");
    assert_ne!(pixel(&last, 20, 37), BLUE, "nothing below the box");
}

#[test]
fn the_outline_of_an_absolutely_positioned_box_follows_the_page_breaks() {
    assert_outline_follows_page_breaks(
        "<div style='position:absolute;top:0;left:6px;width:18px;outline:4px solid blue'>\
         <div style='height:80px'></div></div>",
    );
}

#[test]
fn the_outline_of_a_block_follows_the_page_breaks() {
    assert_outline_follows_page_breaks(
        "<div style='margin-left:6px;width:18px;height:80px;outline:4px solid blue'></div>",
    );
}

#[test]
fn an_outline_is_cut_at_the_sides_of_the_page_area() {
    // The box is at x 30 to 36 with an 8px outline: its right ring lies in the
    // right margin from x 40.
    let pixels = raster_page(
        "<div style='position:absolute;top:0;left:20px;width:6px;height:10px;outline:8px solid blue'></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 25, 15), BLUE, "the left ring");
    assert_eq!(pixel(&pixels, 38, 25), BLUE, "the bottom ring");
    assert_ne!(pixel(&pixels, 41, 25), BLUE, "the right margin");
    assert_ne!(pixel(&pixels, 30, 5), BLUE, "the top margin");
}

#[test]
fn the_border_of_a_box_in_the_top_margin_of_the_first_page_is_not_cut() {
    let pixels = raster_page(
        "<div style='position:absolute;top:-5px;left:0;width:26px;height:10px;border:2px solid blue'></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 20, 6), BLUE, "the top border, in the margin");
}

#[test]
fn a_border_in_the_side_margin_is_cut_at_the_page_area() {
    // The box spans x 4 to 38: its left border (x 4 and 5) lies in the left
    // margin, its right border (x 36 and 37) in the page area.
    for body in [
        "<div style='margin-left:-6px;width:30px;height:10px;border:2px solid blue'></div>",
        "<div style='position:absolute;top:0;left:-6px;width:30px;height:10px;border:2px solid blue'></div>",
    ] {
        let pixels = raster_page(body, 0);
        assert_ne!(pixel(&pixels, 5, 15), BLUE, "{body}: the left margin");
        assert_eq!(pixel(&pixels, 37, 15), BLUE, "{body}: the right border");
    }
}

#[test]
fn the_outline_of_a_relatively_positioned_box_is_cut_but_has_no_break_bars() {
    // The box is moved down to y 35 to 45, across the bottom of the page area
    // (y 40) without a page break cutting it.
    let pixels = raster_page(
        "<div style='position:relative;top:25px;margin-left:6px;width:18px;height:10px;outline:4px solid blue'></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 20, 32), BLUE, "the top ring");
    assert_eq!(pixel(&pixels, 13, 37), BLUE, "the left ring");
    assert_ne!(
        pixel(&pixels, 20, 37),
        BLUE,
        "no bar along the page area edge"
    );
    assert_ne!(pixel(&pixels, 13, 43), BLUE, "the bottom margin");
}

#[test]
fn an_outline_with_a_negative_offset_follows_the_page_breaks() {
    let pixels = raster_page(
        "<div style='position:absolute;top:0;left:6px;width:18px;outline:4px solid blue;outline-offset:-2px'>\
         <div style='height:80px'></div></div>",
        0,
    );
    assert_eq!(pixel(&pixels, 15, 25), BLUE, "the left ring");
    assert_eq!(
        pixel(&pixels, 20, 37),
        BLUE,
        "the bar at the bottom of the page area"
    );
    assert_ne!(pixel(&pixels, 20, 45), BLUE, "the bottom margin");
}
