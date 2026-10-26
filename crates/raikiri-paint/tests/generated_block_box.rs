//! Boxes of block-level `::before` and `::after` painted by the built-in
//! painter.

use anyrender::Scene;
use raikiri_dom::{BundledFace, Document, build_bundled_font_collection, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// A 1×1 red image.
struct Red;
impl ImagePixelSource for Red {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage::new(1, 1, vec![255, 0, 0, 255])))
    }
}

/// Pixels of a 100×80 page holding `<div>A</div>` with the style sheet
/// `css`, on 20px lines of 10px Ahem whose glyphs are transparent.
fn paint(css: &str) -> Vec<u8> {
    paint_with_root(css, None)
}

/// `paint`, with the root element styled `root_css`.
fn paint_with_root(css: &str, root_css: Option<&str>) -> Vec<u8> {
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
    let html = doc.append_element(Some(0), "html", root_css);
    let head = doc.append_element(Some(html), "head", None::<&str>);
    let style = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(
        style,
        format!(
            "html, body {{ display: block; margin: 0 }} \
             div {{ display: block; font: 10px/20px Ahem; color: transparent }} {css}"
        ),
    );
    let body = doc.append_element(Some(html), "body", None::<&str>);
    let div = doc.append_element(Some(body), "div", None::<&str>);
    doc.append_text(div, "A");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &Red).unwrap();
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
fn an_empty_block_after_paints_its_background_over_its_whole_box() {
    let pixels = paint("div::after { content: ''; display: block; height: 4px; background: red }");
    // The bar spans the paragraph's width below its line.
    for x in [1, 50, 98] {
        assert_eq!(pixel(&pixels, x, 22), [255, 0, 0], "bar at x={x}");
    }
    assert_eq!(pixel(&pixels, 50, 18), [255, 255, 255], "above the bar");
    assert_eq!(pixel(&pixels, 50, 25), [255, 255, 255], "below the bar");
}

#[test]
fn a_block_before_with_text_paints_its_border_box_not_its_text() {
    let pixels = paint(
        "div::before { content: 'B'; display: block; width: 40px; \
         border: 2px solid lime; background: blue }",
    );
    // The border box is 44px wide and 24px tall: a 20px line inside a 2px
    // border.
    assert_eq!(
        pixel(&pixels, 30, 10),
        [0, 0, 255],
        "background past the text"
    );
    assert_eq!(pixel(&pixels, 0, 10), [0, 255, 0], "left border");
    assert_eq!(pixel(&pixels, 43, 10), [0, 255, 0], "right border");
    assert_eq!(pixel(&pixels, 20, 0), [0, 255, 0], "top border");
    assert_eq!(pixel(&pixels, 20, 23), [0, 255, 0], "bottom border");
    assert_eq!(pixel(&pixels, 50, 10), [255, 255, 255], "beside the box");
    assert_eq!(
        pixel(&pixels, 20, 30),
        [255, 255, 255],
        "the element's own line"
    );
}

#[test]
fn a_hidden_block_before_paints_nothing() {
    let pixels = paint(
        "div::before { content: ''; display: block; height: 10px; background: red; \
         visibility: hidden }",
    );
    assert_eq!(pixel(&pixels, 50, 5), [255, 255, 255]);
}

#[test]
fn a_block_before_under_a_translucent_first_line_stays_below_the_text() {
    let pixels = paint(
        "div { color: black } div::first-line { opacity: 0.5 } \
         div::before { content: 'B'; display: block; background: blue }",
    );
    // The box goes with its first line's paint, and is still painted over
    // its whole width.
    let [red, _, blue] = pixel(&pixels, 50, 10);
    assert_eq!(blue, 255);
    assert!(red < 255, "the box is painted: {red}");
}

#[test]
fn a_transformed_block_before_with_an_image_moves_its_whole_box() {
    let pixels = paint(
        "div::before { content: url(file:///red.png); display: block; background: blue; \
         transform: translateX(50px) }",
    );
    // The 100px-wide box moves right by half its width.
    assert_eq!(
        pixel(&pixels, 10, 0),
        [255, 255, 255],
        "left of the moved box"
    );
    assert_eq!(pixel(&pixels, 80, 0)[2], 255, "the moved box");
}

#[test]
fn a_block_before_without_width_paints_nothing() {
    let pixels = paint(
        "div::before { content: ''; display: block; width: 0; height: 10px; \
         border-left: 0 solid; background: red }",
    );
    assert_eq!(pixel(&pixels, 0, 5), [255, 255, 255]);
}

#[test]
fn a_page_along_x_paints_the_block_box_of_its_horizontal_paragraph() {
    let pixels = paint_with_root(
        "div { writing-mode: horizontal-tb; width: 60px } \
         div::before { content: ''; display: block; height: 10px; background: red }",
        Some("writing-mode: vertical-rl"),
    );
    // The paragraph sits at the right of the page, where a vertical flow
    // starts.
    assert_eq!(pixel(&pixels, 70, 5), [255, 0, 0]);
}

/// The paragraph is 60px wide and 50px tall with a 20px column of text.
const VERTICAL_PARAGRAPH: &str = "div { width: 60px; height: 50px } div::before { content: ''; display: block; width: 8px; \
     background: red } div::after { content: ''; display: block; width: 6px; background: lime }";

#[test]
fn a_vertical_rl_paragraph_paints_its_block_boxes_from_the_right() {
    let pixels = paint(&format!(
        "div {{ writing-mode: vertical-rl }} {VERTICAL_PARAGRAPH}"
    ));
    // The block start is the right: the box before the text takes its 8px
    // from the paragraph's right edge, the 20px column of text follows, and
    // the box after it takes 6px at its left.
    for y in [0, 25, 49] {
        assert_eq!(pixel(&pixels, 56, y), [255, 0, 0], "before at y={y}");
        assert_eq!(pixel(&pixels, 29, y), [0, 255, 0], "after at y={y}");
    }
    assert_eq!(pixel(&pixels, 50, 25), [255, 255, 255], "the text column");
    assert_eq!(pixel(&pixels, 56, 55), [255, 255, 255], "below the box");
    assert_eq!(pixel(&pixels, 20, 25), [255, 255, 255], "left of the box");
}

#[test]
fn a_vertical_lr_paragraph_paints_its_block_boxes_from_the_left() {
    let pixels = paint(&format!(
        "div {{ writing-mode: vertical-lr }} {VERTICAL_PARAGRAPH}"
    ));
    for y in [0, 25, 49] {
        assert_eq!(pixel(&pixels, 4, y), [255, 0, 0], "before at y={y}");
        assert_eq!(pixel(&pixels, 31, y), [0, 255, 0], "after at y={y}");
    }
    assert_eq!(pixel(&pixels, 18, 25), [255, 255, 255], "the text column");
    assert_eq!(pixel(&pixels, 4, 55), [255, 255, 255], "below the box");
    assert_eq!(pixel(&pixels, 40, 25), [255, 255, 255], "right of the box");
}

#[test]
fn a_vertical_block_box_keeps_its_border_on_the_sides_it_has() {
    let pixels = paint(
        "div { writing-mode: vertical-lr; width: 60px; height: 50px } \
         div::before { content: ''; display: block; width: 10px; margin: 5px 0 0 6px; \
         border-left: 2px solid lime; border-top: 2px solid blue; background: red }",
    );
    // The box is 12px wide, from x=6, and spans 45px down from y=5: its
    // physical left border is its block-start edge, its top border its
    // inline-start one.
    assert_eq!(pixel(&pixels, 7, 30), [0, 255, 0], "left border");
    assert_eq!(pixel(&pixels, 12, 5), [0, 0, 255], "top border");
    assert_eq!(pixel(&pixels, 12, 30), [255, 0, 0], "background");
    assert_eq!(pixel(&pixels, 12, 2), [255, 255, 255], "above the margin");
    assert_eq!(pixel(&pixels, 3, 30), [255, 255, 255], "left margin");
}
