//! Boxes broken across columns paint as slices of the whole box.

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

#[test]
fn an_absolute_box_breaks_across_the_columns_and_paints_over_later_blocks() {
    // The 400px child of the absolute box fills four 100px columns, two of
    // them overflow columns, and covers the red block after its containing
    // block.
    let actual = raster(
        "<div style='column-count:2;column-fill:auto;column-gap:0;height:100px;width:50px'><div style='position:relative;width:30px'><div style='position:absolute;top:0;width:5px;height:10px;background:green'><div style='background:green;width:25px;height:400px'></div></div></div><div style='background:red;width:20px;height:140px'></div></div>",
    );
    let expected = raster("<div style='width:100px;height:100px;background:green'></div>");
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn blocks_in_a_containing_block_break_with_its_absolute_boxes() {
    // The containing block starts 30px down the first 40px column. Its red
    // block and the absolute box over it both break at the column end, so
    // the green box covers the red block in both columns.
    let actual = raster(
        "<div style='column-count:2;column-fill:auto;column-gap:0;height:40px;width:100px'><div style='height:30px'></div><div style='position:relative'><div style='height:20px;background:red'></div><div style='position:absolute;top:0;width:100%;height:20px;background:green'></div></div></div>",
    );
    let expected = raster(
        "<div style='position:relative;width:100px;height:40px'><div style='position:absolute;left:0;top:30px;width:50px;height:10px;background:green'></div><div style='position:absolute;left:50px;top:0;width:50px;height:10px;background:green'></div></div>",
    );
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

const NESTED_INNER: &str =
    "column-count:2;column-fill:auto;column-gap:16px;width:50px;height:70px;padding:10px";

#[test]
fn an_absolute_box_wider_than_its_column_continues_past_a_cut_containing_block() {
    // The 280px box breaks into four 70px pieces, the last two in overflow
    // columns after its 110px containing block ends, and shows its full
    // 25px width in each, like an in-flow block of the same size.
    let actual = raster(&format!(
        "<div style='{NESTED_INNER}'><div style='position:relative;height:110px'><div style='position:absolute;top:0;width:25px;height:280px;background:green'></div></div></div>"
    ));
    let expected = raster(&format!(
        "<div style='{NESTED_INNER}'><div style='width:25px;height:280px;background:green'></div></div>"
    ));
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn an_absolute_box_breaks_across_the_columns_of_a_nested_container() {
    let nested = |content: &str| {
        raster(&format!(
            "<div style='column-count:2;column-fill:auto;column-gap:0;width:100px;height:90px'><div style='{NESTED_INNER}'>{content}</div></div>"
        ))
    };
    let actual = nested(
        "<div style='position:relative;height:110px'><div style='position:absolute;top:0;width:25px;height:280px;background:green'></div></div>",
    );
    let expected = nested("<div style='width:25px;height:280px;background:green'></div>");
    assert_eq!(differing_pixels(&actual, &expected), 0);
}

#[test]
fn a_nested_container_taller_than_the_outer_column_resolves_its_gap_in_the_outer_column() {
    // The nested container is 50px wide in the outer column, so a 30% gap
    // is 15px.
    let nested = |gap: &str| {
        raster(&format!(
            "<div style='column-count:2;column-fill:auto;column-gap:0;width:100px;height:50px'><div style='column-count:2;column-fill:auto;column-gap:{gap};width:100%;height:100px'><div style='position:relative;width:100%;height:25px'><div style='position:absolute;top:0;left:0;width:100%;height:200px;background:green'></div></div></div></div>"
        ))
    };
    assert_eq!(differing_pixels(&nested("30%"), &nested("15px")), 0);
}
