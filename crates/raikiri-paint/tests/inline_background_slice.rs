//! Background images of inline boxes split across lines.

use anyrender::Scene;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;
use taffy::Style;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// A 2×1 image: red on the left, blue on the right.
struct TwoColor;
impl ImagePixelSource for TwoColor {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        (url.as_str() == "file:///two-color.png").then(|| {
            Arc::new(DecodedImage {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
            })
        })
    }
}

/// Pixels of a 100×80 page whose 40px-wide block holds `AAAA<br>AAAA` in a
/// span styled `span_style`, with 10px Ahem on 20px lines.
fn paint(span_style: &str) -> Vec<u8> {
    let mut doc = Document::new();
    doc.set_font_collection(
        build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: AHEM.to_vec(),
            }],
            false,
        )
        .unwrap(),
    );
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:40px;font:10px/20px Ahem;color:transparent"),
    );
    let span = doc.append_element(Some(root), "span", Style::default(), Some(span_style));
    doc.append_text(span, "AAAA");
    doc.append_element(Some(span), "br", Style::default(), None::<&str>);
    doc.append_text(span, "AAAA");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &TwoColor)
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
fn a_background_image_is_laid_out_over_the_joined_line_fragments() {
    let pixels = paint(
        "display:inline;background-image:url(file:///two-color.png);\
         background-size:100% 100%;background-repeat:no-repeat",
    );
    // The joined box is 80px wide: the red pixel fills its left end, the
    // first line's fragment, and the blue one its right end, the second's.
    // Smoothing blends them between the two pixel centers.
    for x in [2, 10] {
        assert_eq!(pixel(&pixels, x, 5), [255, 0, 0], "first line at x={x}");
    }
    for x in [30, 37] {
        assert_eq!(pixel(&pixels, x, 25), [0, 0, 255], "second line at x={x}");
    }
    let [red, _, blue] = pixel(&pixels, 37, 5);
    assert!(red > blue, "the first fragment ends left of the seam");
    let [red, _, blue] = pixel(&pixels, 2, 25);
    assert!(blue > red, "the second fragment starts right of the seam");
}

#[test]
fn a_cloned_fragment_lays_out_the_image_over_its_own_box() {
    let pixels = paint(
        "display:inline;box-decoration-break:clone;\
         background-image:url(file:///two-color.png);\
         background-size:100% 100%;background-repeat:no-repeat",
    );
    // Each 40px fragment shows the whole image: red on its left end, blue
    // on its right end.
    for y in [5, 25] {
        assert_eq!(pixel(&pixels, 2, y), [255, 0, 0], "left end at y={y}");
        assert_eq!(pixel(&pixels, 37, y), [0, 0, 255], "right end at y={y}");
    }
}
