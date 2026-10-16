//! A nested column container continued in later outer columns paints its
//! border and background as one box sliced at each outer column end.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;

struct NoPixels;
impl ImagePixelSource for NoPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

const WIDTH: u16 = 180;
const HEIGHT: u16 = 100;

fn raster(body: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}body{{margin:0;background:white}}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    assert_eq!(document.page_count(), 1);
    let page = document.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene, document, cascade, page_box, &NoPixels,
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        u32::from(WIDTH),
        u32::from(HEIGHT),
    )
}

fn differing_pixels(actual: &[u8], expected: &[u8]) -> usize {
    actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

const OUTER: &str = "width:170px;height:60px;column-count:2;column-gap:10px;column-fill:auto";

#[test]
fn a_continued_nested_container_has_no_border_at_the_outer_column_break() {
    // The 110px box fills the 60px first column, top border included, and
    // ends 50px down the second, bottom border included.
    let actual = raster(&format!(
        "<div style='{OUTER}'><div style='column-count:1;background:green;border:solid blue;border-width:5px 0'><div style='height:100px'></div></div></div>"
    ));
    let expected = raster(
        "<div style='position:absolute;left:0;top:0;width:80px;height:55px;background:green;border-top:5px solid blue'></div>\
         <div style='position:absolute;left:90px;top:0;width:80px;height:45px;background:green;border-bottom:5px solid blue'></div>",
    );
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn a_middle_row_box_of_a_continued_nested_container_has_no_block_border() {
    // The 150px box fills the 60px first and second columns and ends 40px
    // down the third.
    let actual = raster(
        "<div style='width:260px;height:60px;column-count:3;column-gap:10px;column-fill:auto'><div style='column-count:1;background:green;border:solid blue;border-width:5px 0'><div style='height:140px'></div></div></div>",
    );
    let expected = raster(
        "<div style='position:absolute;left:0;top:0;width:80px;height:55px;background:green;border-top:5px solid blue'></div>\
         <div style='position:absolute;left:90px;top:0;width:80px;height:60px;background:green'></div>\
         <div style='position:absolute;left:180px;top:0;width:80px;height:35px;background:green;border-bottom:5px solid blue'></div>",
    );
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn cloned_decorations_draw_both_borders_on_every_row_box() {
    let actual = raster(&format!(
        "<div style='{OUTER}'><div style='column-count:1;background:green;border:solid blue;border-width:5px 0;box-decoration-break:clone'><div style='height:100px'></div></div></div>"
    ));
    let expected = raster(
        "<div style='position:absolute;left:0;top:0;width:80px;height:50px;background:green;border:solid blue;border-width:5px 0'></div>\
         <div style='position:absolute;left:90px;top:0;width:80px;height:40px;background:green;border:solid blue;border-width:5px 0'></div>",
    );
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn each_row_box_of_a_sliced_nested_container_draws_a_whole_outline() {
    let actual = raster(&format!(
        "<div style='{OUTER}'><div style='column-count:1;outline:5px solid blue;outline-offset:-5px'><div style='height:110px'></div></div></div>"
    ));
    let expected = raster(
        "<div style='position:absolute;left:0;top:0;width:70px;height:50px;border:5px solid blue'></div>\
         <div style='position:absolute;left:90px;top:0;width:70px;height:40px;border:5px solid blue'></div>",
    );
    assert_eq!(differing_pixels(&actual, &expected), 0);
}
