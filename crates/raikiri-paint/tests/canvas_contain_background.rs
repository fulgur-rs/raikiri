//! Containment on the root element or on the `<body>` stops the `<body>`'s
//! background from propagating to the canvas (CSS Containment 2 §2): it paints
//! in the `<body>`'s own box and the canvas stays white. The root's own
//! background keeps painting the canvas.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;

const WIDTH: usize = 100;
const HEIGHT: usize = 60;

/// Paint the first page of `html` on a 100×60 page without margins and return
/// the RGBA pixels. The `<body>` is a 40×20 box at (10, 10).
fn raster(html_style: &str, body_style: &str) -> Vec<u8> {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:{WIDTH}px {HEIGHT}px;margin:0}}\
         html{{{html_style}}}body{{margin:10px;width:40px;height:20px;{body_style}}}</style><body></body>"
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
    paint_single_page_with_origin_and_page(
        &mut scene,
        doc,
        cascade,
        page_box,
        origin,
        page.index(),
        result.page_count(),
        false,
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        WIDTH as u32,
        HEIGHT as u32,
    )
}

/// The RGB of the pixel at (`x`, `y`).
fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let index = (y * WIDTH + x) * 4;
    [pixels[index], pixels[index + 1], pixels[index + 2]]
}

const WHITE: [u8; 3] = [255, 255, 255];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// Pixels of the canvas above the body and below it, and one in the body.
fn probes(pixels: &[u8]) -> [[u8; 3]; 3] {
    [
        pixel(pixels, 5, 5),
        pixel(pixels, 80, 50),
        pixel(pixels, 30, 20),
    ]
}

#[test]
fn the_body_background_fills_the_canvas_without_containment() {
    let pixels = raster("", "background: green");
    assert_eq!(probes(&pixels), [GREEN, GREEN, GREEN]);
}

#[test]
fn containment_on_the_body_keeps_its_background_in_its_own_box() {
    for containment in [
        "contain: style",
        "contain: layout",
        "contain: paint",
        "contain: size",
        "container-type: inline-size",
    ] {
        let pixels = raster("", &format!("background: green; {containment}"));
        assert_eq!(probes(&pixels), [WHITE, WHITE, GREEN], "{containment}");
    }
}

#[test]
fn containment_on_the_root_keeps_the_body_background_in_its_own_box() {
    for containment in ["contain: style", "contain: layout", "container-type: size"] {
        let pixels = raster(containment, "background: green");
        assert_eq!(probes(&pixels), [WHITE, WHITE, GREEN], "{containment}");
    }
}

#[test]
fn a_background_image_is_kept_in_the_body_box_too() {
    let pixels = raster(
        "",
        "background-image: linear-gradient(green, green); contain: style",
    );
    assert_eq!(probes(&pixels), [WHITE, WHITE, GREEN]);
}

#[test]
fn the_root_background_keeps_painting_the_canvas_under_containment() {
    let pixels = raster(
        "background: green; contain: style",
        "background: blue; contain: style",
    );
    assert_eq!(probes(&pixels), [GREEN, GREEN, BLUE]);
}
