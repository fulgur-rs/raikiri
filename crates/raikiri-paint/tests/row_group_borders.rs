//! Row group borders are painted by the cells on the edges of the group in
//! the collapsing borders model (CSS 2.1 §17.6.2.1), and not at all in the
//! separated model (§17.6.1). Chromium draws both pages the same way.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const WIDTH: usize = 100;
const HEIGHT: usize = 60;

/// Paint the first page of `body` on a 100×60 page without margins and return
/// the RGBA pixels.
fn raster(body: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}body{{margin:0}}td{{padding:0;width:20px;height:20px}}</style>{body}"
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

/// `#` for a green pixel, `.` for a white one and `o` for anything else.
fn mark(pixels: &[u8], x: usize, y: usize) -> char {
    let index = (y * WIDTH + x) * 4;
    match [pixels[index], pixels[index + 1], pixels[index + 2]] {
        [0, 128, 0] => '#',
        [255, 255, 255] => '.',
        _ => 'o',
    }
}

/// The marks of the pixels of column `x` from `y` to `y + len`.
fn column(pixels: &[u8], x: usize, y: usize, len: usize) -> String {
    (y..y + len).map(|y| mark(pixels, x, y)).collect()
}

#[test]
fn a_collapsed_row_group_border_is_drawn_once_around_its_cells() {
    // Two rows in a group with a 4px border: the border runs down the left
    // edge of the whole group, and nothing is drawn on the line between the
    // rows. The group is 48px tall: two 20px cells plus the 4px border at
    // each end.
    let pixels = raster(
        "<table style='border-collapse:collapse'><tbody style='border:4px solid green'>\
         <tr><td></td></tr><tr><td></td></tr></tbody></table>",
    );
    let edge = format!("{}{}", "#".repeat(48), ".".repeat(12));
    assert_eq!(column(&pixels, 1, 0, 60), edge);
    let inside = format!(
        "{}{}{}{}",
        "#".repeat(4),
        ".".repeat(40),
        "#".repeat(4),
        ".".repeat(12)
    );
    assert_eq!(column(&pixels, 12, 0, 60), inside);
}

#[test]
fn a_separated_row_group_paints_no_border() {
    let pixels = raster(
        "<table style='border-collapse:separate;border-spacing:0'>\
         <tbody style='border:4px solid green'><tr style='border:4px solid green'>\
         <td></td></tr></tbody></table>",
    );
    for x in 0..30 {
        assert_eq!(column(&pixels, x, 0, 30), ".".repeat(30), "column {x}");
    }
}
