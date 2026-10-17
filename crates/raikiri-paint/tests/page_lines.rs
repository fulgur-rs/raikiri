//! A page along the y axis paints only the lines layout puts on it.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

/// Each page of `body`, on 100px square paper with 20px top and bottom
/// margins, so each page area is 60px high.
fn raster(body: &str) -> Vec<Vec<u8>> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:20px 0}}\
         body{{margin:0;font:10px/10px Ahem;color:green}}</style>{body}"
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
    result
        .pages()
        .map(|page| {
            let (doc, cascade, page_box, origin) = page.paint_inputs();
            let mut scene = Scene::new();
            raikiri_paint::paint_single_page_with_origin(
                &mut scene, doc, cascade, page_box, origin,
            )
            .unwrap();
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |out| out.append_scene(scene, Affine::IDENTITY),
                100,
                100,
            )
        })
        .collect()
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    rgba[(y * 100 + x) * 4..][..4].try_into().unwrap()
}

const GREEN: [u8; 4] = [0, 128, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn the_lines_of_neighbouring_pages_stay_out_of_the_margins() {
    // Ten 10px lines of one paragraph run over two pages; the lines of the
    // other page lie just past each page area, in the margin.
    let pages = raster("<div style='width:10px'>a a a a a a a a a a</div>");
    assert_eq!(pages.len(), 2);
    assert_eq!(pixel(&pages[0], 5, 75), GREEN);
    assert_eq!(pixel(&pages[0], 5, 85), WHITE);
    assert_eq!(pixel(&pages[1], 5, 15), WHITE);
    assert_eq!(pixel(&pages[1], 5, 25), GREEN);
}

#[test]
fn a_line_after_a_forced_break_stays_on_its_page() {
    // The break moves the text after the inner block to the second page,
    // right past the first page area, while their parent spans both pages.
    let pages = raster("<div>a<div style='break-after:page'>a</div>a</div>");
    assert_eq!(pages.len(), 2);
    assert_eq!(pixel(&pages[0], 5, 35), GREEN);
    assert_eq!(pixel(&pages[0], 5, 85), WHITE);
    assert_eq!(pixel(&pages[1], 5, 25), GREEN);
}

#[test]
fn a_line_taller_than_the_page_area_continues_on_the_next_page() {
    // The 80px line overflows the 60px page area of the first page, which
    // keeps it; its last 20px paint at the top of the second page area,
    // above the 10px line that starts there.
    let pages = raster("<div style='font:80px/80px Ahem'>a</div><div>a</div>");
    assert_eq!(pages.len(), 2);
    assert_eq!(pixel(&pages[0], 5, 75), GREEN);
    assert_eq!(pixel(&pages[1], 5, 35), GREEN);
    assert_eq!(pixel(&pages[1], 5, 55), WHITE);
}
