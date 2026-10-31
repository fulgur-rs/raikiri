//! An absolutely positioned box is laid out whole, so its background is cut
//! to the page area where the box continues from the page before or goes on
//! to the next one.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const LIME: [u8; 3] = [0, 255, 0];

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
