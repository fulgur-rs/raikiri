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

#[test]
fn transformed_canvas_images_stay_inside_the_canvas() {
    let pages = pages(
        "",
        "@page { margin: 20px } html { height: 100%; transform: rotate(30deg); \
         background: linear-gradient(red, red) }",
    );
    assert_eq!(pixel(&pages[0], 50, 50), RED);
    // The page margins are outside the canvas.
    assert_eq!(pixel(&pages[0], 5, 5), WHITE);
    assert_eq!(pixel(&pages[0], 95, 95), WHITE);
}

#[test]
fn page_area_clips_stay_put_under_the_root_transform() {
    let pages = pages(
        "<div style=\"height: 10px; background: green\"></div>",
        "@page { margin: 20px } html { transform: translateX(-20px) }",
    );
    // The box moves left, but the page margin still clips its background.
    assert_eq!(pixel(&pages[0], 10, 25), WHITE);
    assert_eq!(pixel(&pages[0], 30, 25), GREEN);
    assert_eq!(pixel(&pages[0], 70, 25), WHITE);
}

#[test]
fn page_area_text_clips_stay_put_under_the_root_transform() {
    // Text clipped to a page area half as wide as the page keeps that clip
    // in page coordinates when the root moves it.
    let pages = pages(
        "MMMMMMMM",
        "@page { margin: 0 30px } html { transform: translateX(-30px) } \
         body { display: flex; font-size: 20px; line-height: 20px }",
    );
    let page = &pages[0];
    for y in 0..20 {
        for x in 0..30 {
            assert_eq!(pixel(page, x, y), WHITE, "ink in the margin at ({x}, {y})");
        }
    }
    let inked = (0..20).any(|y| (30..70).any(|x| pixel(page, x, y) != WHITE));
    assert!(inked, "the text paints in the page area");
}

/// The root's `display: contents` computes to `block` (CSS Display 3 §2.8),
/// so its opacity applies to the page content like any block root's.
#[test]
fn a_contents_root_is_as_translucent_as_a_block_root() {
    let content = "<div style=\"width: 100px; height: 100px; background: red\"></div>";
    let block = pages(content, "html { display: block; opacity: .5 }");
    let contents = pages(content, "html { display: contents; opacity: .5 }");
    let (block, contents) = (pixel(&block[0], 50, 50), pixel(&contents[0], 50, 50));
    assert!((100..200).contains(&block[1]), "{block:?}");
    assert_eq!(contents, block);
}

// The root element's own border is drawn on its box, which the root's margin
// and transform place; the pixel positions were measured in Chromium.

#[test]
fn the_root_border_sits_inside_the_root_margin() {
    let pages = pages(
        "<div style=\"height: 60px; background: green\"></div>",
        "html { margin: 10px; border: 5px solid red }",
    );
    let page = &pages[0];
    assert_eq!(pixel(page, 2, 2), WHITE);
    assert_eq!(pixel(page, 2, 50), WHITE);
    assert_eq!(pixel(page, 12, 50), RED);
    assert_eq!(pixel(page, 87, 50), RED);
    assert_eq!(pixel(page, 50, 12), RED);
    assert_eq!(pixel(page, 50, 77), RED);
    assert_eq!(pixel(page, 50, 50), GREEN);
    assert_eq!(pixel(page, 50, 85), WHITE);
    assert_eq!(pixel(page, 95, 50), WHITE);
}

#[test]
fn the_root_border_moves_with_the_root_transform() {
    let pages = pages(
        "<div style=\"height: 40px; background: green\"></div>",
        "html { width: 60px; border: 5px solid red; transform: translateX(30px) }",
    );
    let page = &pages[0];
    assert_eq!(pixel(page, 12, 20), WHITE);
    assert_eq!(pixel(page, 32, 20), RED);
    assert_eq!(pixel(page, 50, 20), GREEN);
    assert_eq!(pixel(page, 97, 20), RED);
    assert_eq!(pixel(page, 50, 2), RED);
    assert_eq!(pixel(page, 50, 47), RED);
    assert_eq!(pixel(page, 50, 60), WHITE);
}

#[test]
fn a_singular_root_transform_hides_the_root_border() {
    let pages = pages(
        "<div style=\"height: 40px; background: green\"></div>",
        "html { border: 5px solid red; transform: scale(0) }",
    );
    for (x, y) in [(2, 2), (50, 2), (2, 20), (50, 20), (97, 20), (50, 47)] {
        assert_eq!(pixel(&pages[0], x, y), WHITE, "({x}, {y})");
    }
}

#[test]
fn the_root_border_runs_down_every_page_and_closes_on_the_last() {
    let pages = pages(
        "<div style=\"height: 250px; background: green\"></div>",
        "html { border: 5px solid red }",
    );
    assert_eq!(pages.len(), 3);
    // The top edge opens the first page only, and the sides run on.
    assert_eq!(pixel(&pages[0], 50, 2), RED);
    assert_eq!(pixel(&pages[0], 50, 97), GREEN);
    assert_eq!(pixel(&pages[1], 50, 2), GREEN);
    for page in &pages {
        assert_eq!(pixel(page, 2, 30), RED);
        assert_eq!(pixel(page, 97, 30), RED);
    }
    // The bottom edge closes the last page where the root ends.
    assert_eq!(pixel(&pages[2], 50, 57), RED);
    assert_eq!(pixel(&pages[2], 50, 80), WHITE);
    assert_eq!(pixel(&pages[2], 2, 80), WHITE);
}

#[test]
fn the_root_top_margin_and_border_are_left_at_the_page_break() {
    let pages = pages(
        "<div style=\"height: 150px; background: green\"></div>",
        "html { margin-top: 30px; border: 5px solid red }",
    );
    assert_eq!(pages.len(), 2);
    assert_eq!(pixel(&pages[0], 50, 10), WHITE);
    assert_eq!(pixel(&pages[0], 50, 32), RED);
    assert_eq!(pixel(&pages[0], 50, 50), GREEN);
    // The margin and the border at the top do not repeat on the next page.
    assert_eq!(pixel(&pages[1], 50, 2), GREEN);
    assert_eq!(pixel(&pages[1], 50, 87), RED);
    assert_eq!(pixel(&pages[1], 50, 95), WHITE);
}
