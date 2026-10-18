//! Where page-margin boxes sit among the other things a page shows (CSS Page 3
//! §3.1): the page background is below everything, the canvas background, the
//! page border and the document contents form one layer at level zero, and a
//! margin box goes behind that layer with a negative `z-index` and in front of
//! it otherwise.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

/// Raster the only page of a 100px square page with 20px margins, a yellow
/// canvas, a green page background and a red 20px tall block at the top of
/// the page area (y = 20 to 40), with `page_css` added to the `@page` rule.
fn page(page_css: &str) -> Vec<u8> {
    let html = format!(
        "<!doctype html><html><head><style>\
         @page {{ size: 100px 100px; margin: 20px; background: green; {page_css} }} \
         html {{ background: yellow }} body {{ margin: 0 }}\
         </style></head><body><div style=\"height: 20px; background: red\"></div></body></html>"
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
    assert_eq!(result.page_count(), 1);
    let page = result.pages().next().expect("a page");
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
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * 100 + x) * 4;
    rgba[i..i + 4].try_into().expect("pixel")
}

/// A blue box from y = 30 to 50, which reaches 20px into the page area: over
/// the lower half of the red block and over the canvas below it.
fn overlapping_box(z_index: &str) -> String {
    format!(
        "@top-center {{ content: ''; background: blue; height: 20px; \
         margin-bottom: -30px; {z_index} }}"
    )
}

#[test]
fn a_margin_box_is_in_front_of_the_document_by_default() {
    let page = page(&overlapping_box(""));
    // Only the document.
    assert_eq!(pixel(&page, 50, 25), RED);
    // The box over the block, and over the canvas.
    assert_eq!(pixel(&page, 50, 35), BLUE);
    assert_eq!(pixel(&page, 50, 45), BLUE);
}

#[test]
fn a_margin_box_with_a_z_index_of_zero_or_more_is_in_front_of_the_document() {
    for z_index in ["z-index: 0", "z-index: 5"] {
        let page = page(&overlapping_box(z_index));
        assert_eq!(pixel(&page, 50, 35), BLUE, "{z_index}");
        assert_eq!(pixel(&page, 50, 45), BLUE, "{z_index}");
    }
}

#[test]
fn a_margin_box_with_a_negative_z_index_is_behind_the_canvas_and_the_document() {
    let page = page(&overlapping_box("z-index: -1"));
    assert_eq!(pixel(&page, 50, 25), RED);
    // The block, then the canvas, hide the box.
    assert_eq!(pixel(&page, 50, 35), RED);
    assert_eq!(pixel(&page, 50, 45), YELLOW);
}

#[test]
fn a_margin_box_with_a_negative_z_index_is_in_front_of_the_page_background() {
    // The left column lies in the page margin, where only the page
    // background is behind it.
    let page = page("@left-middle { content: ''; background: blue; width: 20px; z-index: -1 }");
    assert_eq!(pixel(&page, 10, 50), BLUE);
    // Beside it, the page margin shows the page background.
    assert_eq!(pixel(&page, 10, 10), [0, 128, 0, 255]);
}
