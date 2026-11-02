//! The marker of a list item is drawn on the page holding the line it aligns
//! with, beside that line, as Chromium draws it. The first line of an item may
//! come after a block with no line, which the marker passes over.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const SIZE: usize = 100;

/// Paint page `index` of `body` on 100×100 pages, in 20px Ahem on 30px lines,
/// and return the RGBA pixels. The list has a 40px start padding and a string
/// marker `M`, an Ahem square 20px wide that stands at x = 16..36.
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}\
         ul{{margin:0;padding:0 0 0 40px;list-style:'M'}}</style><body>{body}</body>"
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
        SIZE as u32,
        SIZE as u32,
    )
}

/// The rows (top to bottom) on which the marker's square is drawn: the rows
/// where the pixel at its center column is black.
fn marker_rows(pixels: &[u8]) -> Vec<usize> {
    (0..SIZE)
        .filter(|&y| pixels[(y * SIZE + 26) * 4..][..3] == [0, 0, 0])
        .collect()
}

/// The same for the body text `z`, which stands at x = 40..60.
fn text_rows(pixels: &[u8]) -> Vec<usize> {
    (0..SIZE)
        .filter(|&y| pixels[(y * SIZE + 50) * 4..][..3] == [0, 0, 0])
        .collect()
}

/// The first line of the item sits after an empty block, within the page: the
/// marker is drawn beside it.
#[test]
fn the_marker_is_beside_the_first_line_after_an_empty_block() {
    let pixels = raster_page("<ul><li><div style='height:65px'></div>z</li></ul>", 0);
    let rows = marker_rows(&pixels);
    assert_eq!(rows.first(), Some(&70));
    assert_eq!(rows, text_rows(&pixels));
}

/// The first line is pushed down to the next page, and the marker goes with
/// it, at the same height, instead of staying on the page of the block.
#[test]
fn the_marker_goes_to_the_page_of_its_first_line() {
    for height in [85, 100] {
        let body = format!("<ul><li><div style='height:{height}px'></div>z</li></ul>");
        let first = raster_page(&body, 0);
        assert!(marker_rows(&first).is_empty(), "page 1 of {height}px");
        let second = raster_page(&body, 1);
        assert_eq!(
            marker_rows(&second).first(),
            Some(&5),
            "page 2 of {height}px"
        );
        assert_eq!(marker_rows(&second), text_rows(&second), "{height}px");
    }
}

/// A first line that fits the first page keeps the marker there, and the
/// item's later pages draw none.
#[test]
fn the_marker_stays_with_a_first_line_on_the_first_page() {
    let body = "<ul><li><div style='height:30px'>y</div><div style='height:50px'></div>z</li></ul>";
    let first = raster_page(body, 0);
    assert_eq!(marker_rows(&first).first(), Some(&5));
    assert!(marker_rows(&raster_page(body, 1)).is_empty());
}
