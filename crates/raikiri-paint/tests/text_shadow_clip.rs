//! A blurred `text-shadow` is not cut off where its text overflows the
//! paragraph's own width.

use anyrender::Scene;
use raikiri_dom::LayoutStyle as Style;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

struct NoImages;
impl ImagePixelSource for NoImages {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

/// Pixels of a 100×40 page whose block `div`, styled by `div_style`, holds
/// `text` in transparent 10px Ahem on 20px lines. Every glyph is a 10px square
/// at y = 5..15 of its line, with a red shadow blurred by 4px.
fn paint(div_style: &str, text: &str) -> Vec<u8> {
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
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        format!(
            "html, body {{ display: block; margin: 0 }} \
             div {{ display: block; font: 10px/20px Ahem; color: transparent; \
             text-shadow: 0 0 4px red; {div_style} }}"
        ),
    );
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.append_text(div, text);
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &NoImages)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        100,
        40,
    )
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * 100 + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

/// Whether the pixel is clearly shadow red: full red with the blue and green
/// channels mostly gone.
fn is_shadow_red(pixels: &[u8], x: usize, y: usize) -> bool {
    let [red, green, blue] = pixel(pixels, x, y);
    red > 200 && green < 100 && blue < 100
}

#[test]
fn the_shadow_of_text_inside_its_paragraph_is_drawn() {
    // Control: a glyph that fits keeps its shadow.
    let pixels = paint("width: 60px", "AAAAAA");
    assert!(is_shadow_red(&pixels, 55, 10));
}

#[test]
fn the_shadow_of_nowrap_text_that_overflows_is_not_clipped() {
    // The six glyphs fill x = 0..60 of a 30px paragraph; the last one lies
    // more than three blur radii past the paragraph's end.
    let pixels = paint("width: 30px; white-space: nowrap", "AAAAAA");
    assert!(is_shadow_red(&pixels, 55, 10));
}

#[test]
fn the_shadow_of_text_pulled_left_by_a_negative_indent_is_not_clipped() {
    // The first glyph starts 30px left of the paragraph.
    let pixels = paint("margin-left: 30px; width: 40px; text-indent: -30px", "AAAA");
    assert!(is_shadow_red(&pixels, 5, 10));
}

#[test]
fn the_shadow_of_ink_outside_its_advance_box_is_not_clipped() {
    // A letter spacing of minus one em leaves every glyph with no advance,
    // so all four sit at x = 0 and their ink reaches 10px past the end of
    // the zero-width paragraph, farther than the three blur radii the clip
    // grows by.
    let pixels = paint(
        "width: 0; white-space: nowrap; letter-spacing: -10px; text-shadow: 0 0 1px red",
        "AAAA",
    );
    assert!(is_shadow_red(&pixels, 8, 10));
}
