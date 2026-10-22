//! How a background image is tiled: `background-repeat: round` on one axis
//! with the other axis `auto`, and along axes that repeat or do not.

use anyrender::Scene;
use raikiri_dom::{Document, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, ImageRasterSize, PageBox};
use std::sync::{Arc, Mutex};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// Serves two 10×10 images: `file:///quadrants.png`, in four quadrants (red
/// and blue on top, green and black below), and `file:///green.png`, solid
/// green.
struct Quadrants;
impl ImagePixelSource for Quadrants {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        let mut rgba = Vec::with_capacity(10 * 10 * 4);
        match url.as_str() {
            "file:///quadrants.png" => {
                for y in 0..10 {
                    for x in 0..10 {
                        rgba.extend_from_slice(match (y < 5, x < 5) {
                            (true, true) => &[255, 0, 0, 255],
                            (true, false) => &[0, 0, 255, 255],
                            (false, true) => &[0, 255, 0, 255],
                            (false, false) => &[0, 0, 0, 255],
                        });
                    }
                }
            }
            "file:///green.png" => {
                for _ in 0..10 * 10 {
                    rgba.extend_from_slice(&[0, 255, 0, 255]);
                }
            }
            _ => return None,
        }
        Some(Arc::new(DecodedImage {
            width: 10,
            height: 10,
            rgba,
        }))
    }
}

/// `Quadrants` that remembers the raster size each request asked for, as an
/// SVG source would be asked to rasterize at it.
struct Recording(Mutex<Vec<(f32, f32)>>);
impl ImagePixelSource for Recording {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        Quadrants.get_decoded(url)
    }

    fn get_decoded_at_size(
        &self,
        url: &url::Url,
        size: ImageRasterSize,
        _max_output_bytes: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        self.0.lock().unwrap().push((size.width, size.height));
        self.get_decoded(url)
    }
}

/// Pixels of a 100×80 page whose block, sized and styled by `block_style`,
/// holds nothing but a background.
fn paint(block_style: &str) -> Vec<u8> {
    paint_with(block_style, &Quadrants)
}

fn paint_with(block_style: &str, source: &dyn ImagePixelSource) -> Vec<u8> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    doc.append_element(
        Some(body),
        "div",
        Some(format!("display:block;{block_style}").as_str()),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, source)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        100,
        80,
    )
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * 100 + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

#[test]
fn a_round_width_keeps_the_auto_height_in_proportion() {
    // 24px holds 2.4 tiles, so two tiles of 12px fill it, and the auto height
    // follows to 12px; the tile does not repeat down.
    let pixels = paint(
        "width:24px;height:40px;background-image:url(file:///quadrants.png);\
         background-repeat:round no-repeat",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 9, 2), BLUE);
    assert_eq!(pixel(&pixels, 14, 2), RED, "the second tile");
    // The tile is 12px tall, not the 10px of the image.
    assert_eq!(pixel(&pixels, 2, 11), GREEN);
    assert_eq!(pixel(&pixels, 9, 10), BLACK);
    assert_eq!(
        pixel(&pixels, 2, 14),
        WHITE,
        "below the single row of tiles"
    );
}

#[test]
fn a_round_height_keeps_the_auto_width_in_proportion() {
    let pixels = paint(
        "width:40px;height:24px;background-image:url(file:///quadrants.png);\
         background-repeat:no-repeat round",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 11, 2), BLUE);
    assert_eq!(pixel(&pixels, 2, 14), RED, "the second row of tiles");
    // The tile is 12px wide, not the 10px of the image.
    assert_eq!(pixel(&pixels, 14, 2), WHITE, "right of the single column");
}

#[test]
fn an_explicit_size_on_the_other_axis_is_not_rescaled() {
    let pixels = paint(
        "width:24px;height:40px;background-image:url(file:///quadrants.png);\
         background-size:auto 10px;background-repeat:round no-repeat",
    );
    // The height is not `auto`, so it stays 10px while the width rounds to 12px.
    assert_eq!(pixel(&pixels, 2, 9), GREEN);
    assert_eq!(pixel(&pixels, 2, 11), WHITE);
    assert_eq!(pixel(&pixels, 9, 2), BLUE);
}

#[test]
fn the_image_is_rasterized_at_the_final_rounded_tile_size() {
    // A size-aware source such as an SVG draws into the viewport it is given,
    // so it must be asked for the 12px tile that ends up on the page, not for
    // the 10px image width or a height scaled from it.
    let source = Recording(Mutex::new(Vec::new()));
    paint_with(
        "width:24px;height:40px;background-image:url(file:///quadrants.png);\
         background-repeat:round no-repeat",
        &source,
    );
    assert_eq!(*source.0.lock().unwrap(), [(12.0, 12.0)]);
    let source = Recording(Mutex::new(Vec::new()));
    paint_with(
        "width:24px;height:40px;background-image:url(file:///quadrants.png);\
         background-repeat:round round",
        &source,
    );
    // Both axes round: 40px holds four 10px tiles, so only the width moves.
    assert_eq!(*source.0.lock().unwrap(), [(12.0, 10.0)]);
}

#[test]
fn a_tile_far_smaller_than_a_pixel_leaves_no_seams() {
    // Each tile of this solid image covers a fifth of a pixel. Painted tile by
    // tile, the antialiased edges of neighboring tiles would let the white
    // page show through; the repeated image fills every pixel.
    let pixels = paint(
        "width:50px;height:40px;background-image:url(file:///green.png);\
         background-size:0.2px 0.2px",
    );
    for y in 0..40 {
        for x in 0..50 {
            assert_eq!(pixel(&pixels, x, y), GREEN, "at ({x}, {y})");
        }
    }
    assert_eq!(pixel(&pixels, 50, 0), WHITE, "right of the box");
    assert_eq!(pixel(&pixels, 0, 40), WHITE, "below the box");
}

#[test]
fn repeat_x_tiles_a_single_row() {
    let pixels = paint(
        "width:40px;height:30px;background-image:url(file:///quadrants.png);\
         background-repeat:repeat-x",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 7, 2), BLUE);
    assert_eq!(pixel(&pixels, 12, 2), RED, "the second tile");
    assert_eq!(pixel(&pixels, 37, 7), BLACK, "the last tile fills the box");
    assert_eq!(pixel(&pixels, 2, 7), GREEN);
    assert_eq!(pixel(&pixels, 2, 12), WHITE, "below the single row");
}

#[test]
fn repeat_y_tiles_a_single_column() {
    let pixels = paint(
        "width:30px;height:40px;background-image:url(file:///quadrants.png);\
         background-repeat:repeat-y",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 2, 7), GREEN);
    assert_eq!(pixel(&pixels, 2, 12), RED, "the second tile");
    assert_eq!(pixel(&pixels, 7, 37), BLACK, "the last tile fills the box");
    assert_eq!(pixel(&pixels, 12, 2), WHITE, "right of the single column");
}

#[test]
fn repeat_on_both_axes_fills_the_box() {
    let pixels = paint(
        "width:30px;height:30px;background-image:url(file:///quadrants.png);\
         background-repeat:repeat",
    );
    assert_eq!(pixel(&pixels, 12, 12), RED);
    assert_eq!(pixel(&pixels, 27, 12), BLUE);
    assert_eq!(pixel(&pixels, 22, 27), GREEN);
    assert_eq!(pixel(&pixels, 30, 0), WHITE, "right of the box");
}

#[test]
fn a_repeating_axis_beside_a_spaced_one_keeps_the_gaps() {
    // 35px holds three 10px tiles, with the remaining 5px shared between two
    // gaps, so the rows start at 0, 12.5 and 25.
    let pixels = paint(
        "width:40px;height:35px;background-image:url(file:///quadrants.png);\
         background-repeat:repeat space",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 12, 2), RED, "repeats along x");
    assert_eq!(
        pixel(&pixels, 2, 11),
        WHITE,
        "the gap between the first rows"
    );
    assert_eq!(pixel(&pixels, 2, 16), RED, "the second row");
    assert_eq!(pixel(&pixels, 2, 27), RED, "the third row");
}

#[test]
fn a_repeating_axis_beside_a_spaced_one_keeps_the_gaps_across() {
    let pixels = paint(
        "width:35px;height:40px;background-image:url(file:///quadrants.png);\
         background-repeat:space repeat",
    );
    assert_eq!(pixel(&pixels, 2, 2), RED);
    assert_eq!(pixel(&pixels, 2, 12), RED, "repeats along y");
    assert_eq!(
        pixel(&pixels, 11, 2),
        WHITE,
        "the gap between the first columns"
    );
    assert_eq!(pixel(&pixels, 16, 2), RED, "the second column");
    assert_eq!(pixel(&pixels, 27, 2), RED, "the third column");
}
