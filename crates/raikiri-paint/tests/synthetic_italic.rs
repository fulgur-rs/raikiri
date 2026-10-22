//! A synthetic italic (`font-style: italic` over a face without an italic
//! design) leans to the right, like an italic in a browser: the glyph's top is
//! shifted right of its bottom.

use anyrender::Scene;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const WIDTH: usize = 100;
const HEIGHT: usize = 100;

struct NoImages;
impl ImagePixelSource for NoImages {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

/// Pixels of a 100×100 page whose block `div`, styled by `div_style`, holds
/// `text` in red 20px Ahem on 40px lines. Every upright glyph is a 20px square
/// at y = 10..30 of its line, standing on a baseline at y = 26.
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
    let html = doc.append_element(Some(0), "html", None::<&str>);
    let head = doc.append_element(Some(html), "head", None::<&str>);
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        format!(
            "html, body {{ display: block; margin: 0 }} \
             div {{ display: block; font: 20px/40px Ahem; color: red; \
             margin-left: 30px; {div_style} }}"
        ),
    );
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let div = doc.append_element(Some(body), "div", None::<&str>);
    doc.append_text(div, text);
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = WIDTH as f32;
    page.height = HEIGHT as f32;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &NoImages)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        WIDTH as u32,
        HEIGHT as u32,
    )
}

/// The first and last x of the clearly red pixels on row `y`.
fn red_extent(pixels: &[u8], y: usize) -> (usize, usize) {
    let red: Vec<usize> = (0..WIDTH)
        .filter(|&x| {
            let at = (y * WIDTH + x) * 4;
            pixels[at] > 200 && pixels[at + 1] < 100 && pixels[at + 2] < 100
        })
        .collect();
    (
        *red.first().unwrap_or_else(|| panic!("no red on row {y}")),
        *red.last().unwrap(),
    )
}

#[test]
fn an_upright_glyph_is_the_control() {
    let pixels = paint("", "A");
    assert_eq!(red_extent(&pixels, 11), (30, 49));
    assert_eq!(red_extent(&pixels, 29), (30, 49));
}

#[test]
fn a_synthetic_italic_leans_right() {
    let pixels = paint("font-style: italic", "A");
    // The glyph turns about its baseline, 15px below row 11 and 3px above
    // row 29, so the top moves right by about 4px and the bottom left by 1px.
    let (top, _) = red_extent(&pixels, 11);
    let (bottom, _) = red_extent(&pixels, 29);
    assert!(top >= 33, "top starts at {top}");
    assert!(bottom <= 30, "bottom starts at {bottom}");
}

#[test]
fn a_synthetic_oblique_leans_right() {
    let pixels = paint("font-style: oblique", "A");
    let (top, _) = red_extent(&pixels, 11);
    let (bottom, _) = red_extent(&pixels, 29);
    assert!(top >= 33, "top starts at {top}");
    assert!(bottom <= 30, "bottom starts at {bottom}");
}

/// The first and last y of the clearly red pixels in column `x`.
fn red_column_extent(pixels: &[u8], x: usize) -> (usize, usize) {
    let red: Vec<usize> = (0..HEIGHT)
        .filter(|&y| {
            let at = (y * WIDTH + x) * 4;
            pixels[at] > 200 && pixels[at + 1] < 100 && pixels[at + 2] < 100
        })
        .collect();
    (
        *red.first()
            .unwrap_or_else(|| panic!("no red in column {x}")),
        *red.last().unwrap(),
    )
}

const VERTICAL: &str = "writing-mode: vertical-rl; width: 40px; height: 60px; ";

#[test]
fn an_upright_glyph_in_vertical_text_is_the_control() {
    let pixels = paint(&format!("{VERTICAL} text-orientation: upright"), "A");
    assert_eq!(red_extent(&pixels, 1), red_extent(&pixels, 18));
}

#[test]
fn a_synthetic_italic_leans_right_in_upright_vertical_text() {
    // An upright glyph keeps its horizontal slant.
    let pixels = paint(
        &format!("{VERTICAL} text-orientation: upright; font-style: italic"),
        "A",
    );
    // The glyph fills y = 0..20 of its column.
    let (top, _) = red_extent(&pixels, 1);
    let (bottom, _) = red_extent(&pixels, 18);
    assert!(top > bottom + 3, "top starts at {top}, bottom at {bottom}");
}

#[test]
fn a_synthetic_italic_leans_along_a_sideways_glyph() {
    // A sideways glyph turned clockwise has its top on the right and leans
    // toward its own right, which is down the page: the right edge starts
    // lower than the left one.
    let pixels = paint(&format!("{VERTICAL} font-style: italic"), "A");
    let (left, _) = red_column_extent(&pixels, 41);
    let (right, _) = red_column_extent(&pixels, 58);
    assert!(right > left + 3, "left starts at {left}, right at {right}");
}
