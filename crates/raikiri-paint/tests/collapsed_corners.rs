//! Where collapsed borders meet, the strongest of the lines takes the whole
//! junction square instead of each cell cutting its corner along the
//! diagonal: the widest line, then the one with the stronger style, then the
//! one from the more specific kind of box (cell, row, row group, column,
//! column group, table), and then from the box earlier in the table. Between
//! two lines of one box, the line after the junction wins, and a horizontal
//! line before a vertical one. Chromium paints these pages the same way.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const SIZE: usize = 80;

/// Paint `body` on an 80×80 page without margins, with 20px square cells, and
/// return the RGBA pixels.
fn raster(body: &str) -> Vec<u8> {
    raster_table("", body)
}

/// Paint a table with the extra `style` and the rows of `body`, as [`raster`].
fn raster_table(style: &str, body: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{SIZE}px {SIZE}px;margin:0}}body{{margin:0}}\
         td{{padding:0;width:20px;height:20px}}</style>\
         <table style='border-collapse:collapse;{style}'>{body}</table>"
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
        SIZE as u32,
        SIZE as u32,
    )
}

/// The color of the 4px junction square at (`x`, `y`): `R`, `G`, `B` or `Y`
/// when the whole square has that color, and `o` when it is mixed.
fn junction(pixels: &[u8], x: usize, y: usize) -> char {
    let mut colors = (0..4)
        .flat_map(|dy| (0..4).map(move |dx| ((y + dy) * SIZE + x + dx) * 4))
        .map(
            |index| match [pixels[index], pixels[index + 1], pixels[index + 2]] {
                [255, 0, 0] => 'R',
                [0, 128, 0] => 'G',
                [0, 0, 255] => 'B',
                [255, 255, 0] => 'Y',
                _ => 'o',
            },
        );
    let first = colors.next().unwrap();
    if colors.all(|color| color == first) {
        first
    } else {
        'o'
    }
}

/// The junctions of the grid of 4px lines 24px apart, row by row.
fn junctions(pixels: &[u8], columns: usize, rows: usize) -> Vec<String> {
    (0..rows)
        .map(|row| {
            (0..columns)
                .map(|column| junction(pixels, column * 24, row * 24))
                .collect()
        })
        .collect()
}

#[test]
fn a_junction_goes_to_the_cell_earliest_in_the_table() {
    let pixels = raster(
        "<tr><td style='border:4px solid red'></td><td style='border:4px solid green'></td></tr>\
         <tr><td style='border:4px solid blue'></td><td style='border:4px solid yellow'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 3, 3), ["RRG", "RRG", "BBY"]);
}

#[test]
fn a_junction_between_row_groups_goes_to_the_earlier_group() {
    let pixels = raster(
        "<tbody style='border:4px solid red'><tr><td></td><td></td></tr></tbody>\
         <tbody style='border:4px solid blue'><tr><td></td><td></td></tr></tbody>",
    );
    assert_eq!(junctions(&pixels, 2, 3), ["RR", "RR", "BB"]);
}

#[test]
fn a_cell_corner_goes_to_one_of_its_own_sides() {
    // The bottom side wins its corners over the left and right ones, and the
    // right side wins the top right corner over the top one.
    let pixels = raster(
        "<tr><td style='border:4px solid green;border-bottom-color:red;\
         border-right-color:blue'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 2, 2), ["GB", "RR"]);
    // The top side wins the top left corner over the left one.
    let pixels = raster(
        "<tr><td style='border:4px solid green;border-top-color:red;\
         border-left-color:blue'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 2, 2), ["RG", "GG"]);
}

#[test]
fn a_cell_border_wins_a_junction_over_its_row() {
    let pixels = raster(
        "<tr style='border:4px solid red'><td style='border-bottom:4px solid blue'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 2, 2), ["RR", "BB"]);
}

#[test]
fn a_missing_side_leaves_the_junction_to_the_cell_above() {
    let pixels = raster(
        "<tr><td style='border:4px solid red;border-bottom:none'></td></tr>\
         <tr><td style='border:4px solid blue;border-top-color:yellow'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 2, 3), ["RR", "RR", "BB"]);
}

#[test]
fn the_more_specific_box_wins_a_junction() {
    // In each table the blue border comes from the more specific box, once on
    // the top side and once on the left one.
    for body in [
        "<tbody style='border-left:4px solid red'><tr><td style='border-top:4px solid blue'></td></tr></tbody>",
        "<tbody style='border-top:4px solid red'><tr style='border-left:4px solid blue'><td></td></tr></tbody>",
        "<col style='border-left:4px solid red'><tr style='border-top:4px solid blue'><td></td></tr>",
        "<col style='border-top:4px solid red'><tr style='border-left:4px solid blue'><td></td></tr>",
        "<colgroup style='border-left:4px solid red'><col style='border-top:4px solid blue'></colgroup><tr><td></td></tr>",
    ] {
        let pixels = raster(body);
        assert_eq!(junction(&pixels, 0, 0), 'B', "{body}");
    }
}

#[test]
fn a_right_to_left_table_mirrors_its_junctions() {
    let pixels = raster_table(
        "direction:rtl",
        "<tr><td style='border:4px solid red'></td><td style='border:4px solid green'></td></tr>\
         <tr><td style='border:4px solid blue'></td><td style='border:4px solid yellow'></td></tr>",
    );
    // The first cell is on the right.
    assert_eq!(junctions(&pixels, 3, 3), ["GRR", "GRR", "YBB"]);
}

#[test]
fn no_line_runs_through_a_spanning_cell() {
    let pixels = raster(
        "<tr><td rowspan=2 style='border:4px solid red'></td><td style='border:4px solid green'></td></tr>\
         <tr><td style='border:4px solid blue'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 3, 3), ["RRG", "RRG", "RRB"]);
}

#[test]
fn a_cell_painted_later_keeps_the_junction_it_does_not_own() {
    // The cells of the second row are positioned, so they paint after the
    // first row, with their backgrounds; the junctions still show the lines
    // that own them.
    let pixels = raster(
        "<tr><td style='border:4px solid red'></td><td style='border:4px solid green'></td></tr>\
         <tr><td style='position:relative;background:white;border:4px solid blue'></td>\
         <td style='position:relative;background:white;border:4px solid yellow'></td></tr>",
    );
    assert_eq!(junctions(&pixels, 3, 3), ["RRG", "RRG", "BBY"]);
}
