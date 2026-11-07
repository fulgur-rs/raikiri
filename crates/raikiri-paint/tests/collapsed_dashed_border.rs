//! A dashed border in a table with collapsed borders is drawn one segment per
//! cell: the dashes start again at the edge of each cell, and a segment starts
//! and ends with a dash, as Chromium draws it (CSS Tables 3 §5.2: the winning
//! border of each edge segment is painted by the cell that owns it).

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const WIDTH: usize = 100;
const HEIGHT: usize = 40;

/// Paint the first page of `body` on a 100×40 page without margins and return
/// the RGBA pixels.
fn raster(body: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}body{{margin:0}}</style>{body}"
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
    let page = result.page(0).expect("the page exists");
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        WIDTH as u32,
        HEIGHT as u32,
    )
}

/// `#` for a green pixel, `.` for a white one and `o` for a partly covered one.
fn mark(pixels: &[u8], x: usize, y: usize) -> char {
    let index = (y * WIDTH + x) * 4;
    match [pixels[index], pixels[index + 1], pixels[index + 2]] {
        [0, 128, 0] => '#',
        [255, 255, 255] => '.',
        _ => 'o',
    }
}

/// The marks of the pixels of row `y` from `x` to `x + len`.
fn row(pixels: &[u8], y: usize, x: usize, len: usize) -> String {
    (x..x + len).map(|x| mark(pixels, x, y)).collect()
}

/// The marks of the pixels of column `x` from `y` to `y + len`.
fn column(pixels: &[u8], x: usize, y: usize, len: usize) -> String {
    (y..y + len).map(|y| mark(pixels, x, y)).collect()
}

/// A row with a 2px dashed green border around two cells, 30px and 22px wide
/// with the half of the border on each side.
const TWO_CELLS: &str = "<table style='border-collapse:collapse'>\
    <tr style='border:2px dashed green'>\
    <td style='width:28px;height:10px;padding:0'></td>\
    <td style='width:20px;padding:0'></td></tr></table>";

#[test]
fn the_dashes_start_again_in_each_cell() {
    let pixels = raster(TWO_CELLS);
    // The first cell is 30px wide: four dashes with a 2px gap between two. The
    // second is 22px wide: three dashes. The last dash of the first cell and
    // the first of the second meet at x = 30. One pattern over the 52px of
    // the row would put its dashes at other places.
    let expected = "######..######..######..############..######..######";
    assert_eq!(row(&pixels, 0, 0, 52), expected, "the top side");
    assert_eq!(row(&pixels, 1, 0, 52), expected, "the top side, second row");
    assert_eq!(row(&pixels, 12, 0, 52), expected, "the bottom side");
    assert_eq!(
        row(&pixels, 13, 0, 52),
        expected,
        "the bottom side, second row"
    );
}

#[test]
fn a_segment_starts_and_ends_with_a_dash() {
    let pixels = raster(TWO_CELLS);
    // The left and the right side of the row are 14px long with the corners.
    let expected = "#####o..o#####";
    assert_eq!(column(&pixels, 0, 0, 14), expected, "the left side");
    assert_eq!(column(&pixels, 50, 0, 14), expected, "the right side");
}
