//! The canvas background and the root element's box and transform across
//! pages, as the painter draws them.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 128, 0, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Lay out `body` with `css` on 100px square pages and raster every page.
fn pages(body: &str, css: &str) -> Vec<Vec<u8>> {
    let html = format!(
        "<!doctype html><html><head><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0 }} {css}</style></head><body>{body}</body></html>"
    );
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    result
        .pages()
        .map(|page| {
            let (document, cascade, page_box, origin) = page.paint_inputs();
            let mut scene = Scene::new();
            paint_single_page_with_origin_and_page(
                &mut scene,
                document,
                cascade,
                page_box,
                origin,
                page.index(),
                result.page_count(),
                false,
            )
            .expect("paint");
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                100,
                100,
            )
        })
        .collect()
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * 100 + x) * 4;
    rgba[i..i + 4].try_into().expect("pixel")
}

/// Content two pages tall.
const TWO_PAGES: &str = "<div style=\"height: 150px\"></div>";

#[test]
fn a_no_repeat_canvas_image_shows_only_where_the_root_box_reaches() {
    let pages = pages(
        TWO_PAGES,
        "html { background: linear-gradient(red, red) no-repeat; \
         background-size: 100px 50px }",
    );
    assert_eq!(pages.len(), 2);
    assert_eq!(pixel(&pages[0], 50, 25), RED);
    assert_eq!(pixel(&pages[0], 50, 75), WHITE);
    // The second page continues the canvas below the image.
    assert_eq!(pixel(&pages[1], 50, 25), WHITE);
}

#[test]
fn canvas_tiles_continue_from_the_page_before() {
    let pages = pages(
        TWO_PAGES,
        "html { background: linear-gradient(red 50%, blue 50%); \
         background-size: 100px 60px }",
    );
    assert_eq!(pixel(&pages[0], 50, 10), RED);
    assert_eq!(pixel(&pages[0], 50, 40), BLUE);
    assert_eq!(pixel(&pages[0], 50, 70), RED);
    // Flow y 100 is 40px into the second tile: its blue half ends at 120.
    assert_eq!(pixel(&pages[1], 50, 10), BLUE);
    assert_eq!(pixel(&pages[1], 50, 30), RED);
}

#[test]
fn a_fixed_canvas_layer_sits_in_the_page_area_on_every_page() {
    let pages = pages(
        TWO_PAGES,
        "html { margin: 30px; background: linear-gradient(red, red) no-repeat fixed; \
         background-size: 10px 10px }",
    );
    for page in &pages {
        assert_eq!(pixel(page, 5, 5), RED);
        assert_eq!(pixel(page, 35, 35), WHITE);
    }
}

#[test]
fn the_root_transform_moves_the_canvas_images_and_the_content() {
    let pages = pages(
        "<div style=\"margin-top: 20px; width: 10px; height: 10px; background: green\"></div>",
        "html { transform: translateX(30px); \
         background: yellow linear-gradient(red, red) no-repeat; background-size: 10px 10px }",
    );
    let page = &pages[0];
    assert_eq!(pixel(page, 35, 5), RED);
    assert_eq!(pixel(page, 35, 25), GREEN);
    // The canvas color still covers the whole page.
    assert_eq!(pixel(page, 5, 5), YELLOW);
    assert_eq!(pixel(page, 5, 25), YELLOW);
}

#[test]
fn a_singular_root_transform_renders_only_the_canvas_color() {
    let pages = pages(
        "<div style=\"width: 10px; height: 10px; background: green\"></div>",
        "html { transform: scale(0); \
         background: yellow linear-gradient(red, red) no-repeat; background-size: 10px 10px }",
    );
    assert_eq!(pixel(&pages[0], 5, 5), YELLOW);
    assert_eq!(pixel(&pages[0], 50, 50), YELLOW);
}
