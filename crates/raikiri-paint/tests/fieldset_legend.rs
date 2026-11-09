//! A `fieldset` paints its block-start border across the middle of a taller
//! rendered legend, leaves the border out where the legend lies and starts
//! its background at the border, as Chromium does (HTML Living Standard,
//! "Rendering", §15.3.13 "The fieldset and legend elements").

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

/// `#` for a green pixel, `b` for a blue one, `.` for a white one and `o`
/// for any other.
fn mark(pixels: &[u8], x: usize, y: usize) -> char {
    let index = (y * WIDTH + x) * 4;
    match [pixels[index], pixels[index + 1], pixels[index + 2]] {
        [0, 128, 0] => '#',
        [0, 0, 255] => 'b',
        [255, 255, 255] => '.',
        _ => 'o',
    }
}

/// The marks of the pixels of row `y` from `x` to `x + len`.
fn row(pixels: &[u8], y: usize, x: usize, len: usize) -> String {
    (x..x + len).map(|x| mark(pixels, x, y)).collect()
}

/// The rows `y` from `from` to `to`, each from `x = 0` across 56px.
fn rows(pixels: &[u8], from: usize, to: usize) -> Vec<String> {
    (from..to).map(|y| row(pixels, y, 0, 56)).collect()
}

#[test]
fn the_border_centres_on_a_taller_legend_and_leaves_a_gap_behind_it() {
    // Chromium: the 10px legend sits at the top of the fieldset, the 4px
    // border runs across its middle (3px down) with a gap where the legend
    // lies, and the background starts at the border. The fieldset is 18px
    // tall: its stated 10px height less the 6px of the legend that reach
    // below the border.
    let pixels = raster(
        "<fieldset style='margin:0;padding:0 4px;border:4px solid green;background:blue;\
         width:40px;height:10px'><legend style='padding:0;width:20px;height:10px'></legend>\
         </fieldset>",
    );
    let top = "########bbbbbbbbbbbbbbbbbbbb############################";
    let side = "####bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb####";
    let bottom = "########################################################";
    let blank = "........................................................";
    let mut expected = vec![blank; 3];
    expected.extend([top; 4]);
    expected.extend([side; 7]);
    expected.extend([bottom; 4]);
    expected.push(blank);
    assert_eq!(rows(&pixels, 0, 19), expected);
}
