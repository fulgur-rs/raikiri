//! A dashed or dotted border around a box with rounded corners is laid out
//! along the curve (CSS Backgrounds 3 §5.2): one pattern runs around the whole
//! border, from where the upper-left corner ends, instead of one per side. The
//! expected pixels are the ones Chromium draws for the same boxes.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const WIDTH: usize = 140;
const HEIGHT: usize = 90;

/// Paint the first page of `body` on a 140×90 page without margins and return
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

/// A box of `width` × `height` content pixels at (10, 10) with `style`.
fn boxed(width: usize, height: usize, style: &str) -> Vec<u8> {
    raster(&format!(
        "<div style='position:absolute;left:10px;top:10px;width:{width}px;height:{height}px;{style}'></div>"
    ))
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

#[test]
fn dashes_start_where_the_upper_left_corner_ends() {
    // A 4px dashed border around 100 × 50 content pixels, with corners of 12px:
    // the corner is rounded off, and the first dash begins on the top edge where
    // the arc of the upper-left corner ends, 12px from the left.
    let pixels = boxed(100, 50, "border:4px dashed green;border-radius:12px");
    assert_eq!(row(&pixels, 10, 10, 5), ".....", "the corner is cut off");
    assert_eq!(column(&pixels, 10, 10, 5), ".....", "the corner is cut off");
    assert_eq!(
        row(&pixels, 11, 22, 44),
        "########...o#######o...o#######o...o#######o"
    );
}

#[test]
fn dots_are_round_and_one_pattern_runs_around_the_border() {
    // 4px dots along the top edge, about 8px apart.
    let pixels = boxed(100, 50, "border:4px dotted green;border-radius:12px");
    assert_eq!(
        row(&pixels, 11, 10, 9),
        ".........",
        "the corner is cut off"
    );
    assert_eq!(row(&pixels, 11, 21, 2), "##");
    assert_eq!(row(&pixels, 11, 24, 4), "....");
    assert_eq!(row(&pixels, 11, 29, 2), "##");
    assert_eq!(row(&pixels, 11, 33, 3), "...");
    // A dot is round: the pixels at its corners are not covered.
    assert_ne!(mark(&pixels, 20, 10), '#');
}

#[test]
fn wide_dots_are_spaced_along_the_curve() {
    let pixels = boxed(100, 50, "border:6px dotted green;border-radius:14px");
    assert_eq!(row(&pixels, 12, 22, 4), "####");
    assert_eq!(row(&pixels, 12, 27, 6), "......");
    assert_eq!(row(&pixels, 12, 34, 4), "####");
    assert_eq!(row(&pixels, 12, 40, 5), ".....");
    assert_eq!(row(&pixels, 12, 46, 5), "#####");
}

#[test]
fn thin_dots_are_square_and_exactly_twice_the_thickness_apart() {
    // 2px dots are not fitted to the border: they stay 4px apart all the way.
    let pixels = boxed(100, 50, "border:2px dotted green;border-radius:12px");
    assert_eq!(row(&pixels, 10, 22, 40), "##..".repeat(10));
}

#[test]
fn a_square_corner_keeps_the_start_at_the_edge_of_the_box() {
    // With a square upper-left corner the border starts on the top edge, and
    // the dashes after the first one are those of the rounded border.
    let pixels = boxed(
        100,
        50,
        "border:4px dashed green;border-radius:0 12px 12px 12px",
    );
    assert_eq!(
        row(&pixels, 11, 20, 37),
        "....o#######o...o#######o...o#######o"
    );
}

#[test]
fn a_border_too_short_for_two_dashes_is_solid() {
    // The centerline of this circle is 25px long, less than two 16px dashes, so
    // the border is a plain ring.
    let dashed = boxed(0, 0, "border:8px dashed green;border-radius:8px");
    let solid = boxed(0, 0, "border:8px solid green;border-radius:8px");
    assert!(dashed == solid);
    assert_eq!(mark(&dashed, 18, 18), '#');
}

#[test]
fn a_border_between_two_and_three_dashes_long_has_two_shorter_dashes() {
    // The 38px centerline of this circle holds two dashes of 12.5px and two gaps
    // of 6px: the ring is open at the lower right and at the upper left.
    let pixels = boxed(4, 4, "border:8px dashed green;border-radius:10px");
    assert_eq!(mark(&pixels, 21, 11), '#');
    assert_eq!(mark(&pixels, 27, 20), '#');
    assert_eq!(mark(&pixels, 14, 26), '#');
    assert_eq!(mark(&pixels, 11, 20), '#');
    assert_eq!(mark(&pixels, 24, 26), '.');
    assert_eq!(mark(&pixels, 14, 14), '.');
    assert_eq!(mark(&pixels, 20, 20), '.', "the hole in the middle");
}

#[test]
fn a_double_border_with_rounded_corners_keeps_its_two_bands() {
    let pixels = boxed(100, 50, "border:6px double green;border-radius:12px");
    assert_eq!(column(&pixels, 50, 8, 12), "..##..##....");
    assert_eq!(row(&pixels, 30, 8, 12), "..##..##....");
}
