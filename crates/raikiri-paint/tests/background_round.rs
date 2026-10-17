//! `background-repeat: round` on one axis, with the other axis `auto`.

use anyrender::Scene;
use raikiri_dom::{Document, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;
use taffy::Style;

/// A 10×10 image in four quadrants: red and blue on top, green and black
/// below.
struct Quadrants;
impl ImagePixelSource for Quadrants {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        if url.as_str() != "file:///quadrants.png" {
            return None;
        }
        let mut rgba = Vec::with_capacity(10 * 10 * 4);
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
        Some(Arc::new(DecodedImage {
            width: 10,
            height: 10,
            rgba,
        }))
    }
}

/// Pixels of a 100×80 page whose block, sized and styled by `block_style`,
/// holds nothing but a background.
fn paint(block_style: &str) -> Vec<u8> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!("display:block;{block_style}").as_str()),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &Quadrants)
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

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 255, 0];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

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
