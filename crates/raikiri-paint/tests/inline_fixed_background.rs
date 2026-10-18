//! `background-attachment: fixed` on inline boxes is positioned in the page
//! area, not in the inline box.

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

/// A 2×1 image, red on the left and blue on the right.
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

/// A layer that stretches the image over its positioning area once: red on
/// the left half, blue on the right half.
const STRETCHED: &str = "background-image:url(file:///two-color.png);\
                         background-size:100% 100%;background-repeat:no-repeat";

/// The layer positioned in the 100×80 page area.
fn fixed() -> String {
    format!("{STRETCHED};background-attachment:fixed")
}

/// Pixels of a 100×80 page whose block, styled `block_style`, holds the
/// elements that `build` appends, with 10px Ahem on 20px lines. The block is
/// 40px wide, so `AAAA<br>AAAA` fills two lines of 40px.
fn paint(block_style: &str, build: impl FnOnce(&mut Document, usize)) -> Vec<u8> {
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
        Some(
            format!(
                "display:block;width:40px;margin-left:30px;{block_style};\
                 font:10px/20px Ahem;color:transparent"
            )
            .as_str(),
        ),
    );
    build(&mut doc, root);
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

/// `AAAA<br>AAAA` in a span styled `style` appended to `parent`; the span.
fn two_lines(doc: &mut Document, parent: usize, style: &str) -> usize {
    let span = doc.append_element(Some(parent), "span", Style::default(), Some(style));
    doc.append_text(span, "AAAA");
    doc.append_element(Some(span), "br", Style::default(), None::<&str>);
    doc.append_text(span, "AAAA");
    span
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * 100 + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

/// Whether the pixel is closer to the image's red end than to its blue end.
/// Smoothing blends the two colors between the centers of the image's pixels,
/// so only the side a pixel leans to tells where the image was placed.
fn leans_red(pixels: &[u8], x: usize, y: usize) -> bool {
    let [red, _, blue] = pixel(pixels, x, y);
    red > blue
}

/// Assert that the two 40px line fragments, at x = 30..70, show the page's
/// image: red left of x = 50 and blue right of it, on both lines.
fn assert_page_positioned(pixels: &[u8]) {
    for y in [5, 25] {
        assert!(leans_red(pixels, 35, y), "left of the page's middle, y={y}");
        assert!(!leans_red(pixels, 65, y), "right of it, y={y}");
    }
}

/// Assert that the fragments show the image laid over their joined 80px box,
/// as `scroll` does: red on the first line and blue on the second.
fn assert_box_positioned(pixels: &[u8]) {
    for x in [35, 65] {
        assert!(leans_red(pixels, x, 5), "first line, x={x}");
        assert!(!leans_red(pixels, x, 25), "second line, x={x}");
    }
}

#[test]
fn a_fixed_layer_of_an_inline_box_is_positioned_in_the_page_area() {
    let pixels = paint("", |doc, root| {
        two_lines(doc, root, &format!("display:inline;{}", fixed()));
    });
    assert_page_positioned(&pixels);
}

#[test]
fn a_scroll_layer_of_an_inline_box_stays_in_the_joined_box() {
    let pixels = paint("", |doc, root| {
        two_lines(
            doc,
            root,
            &format!("display:inline;{STRETCHED};background-attachment:scroll"),
        );
    });
    assert_box_positioned(&pixels);
}

#[test]
fn a_cloned_fixed_layer_is_still_positioned_in_the_page_area() {
    // The image is 150px wide in the page area, which places its middle at
    // x = 75. Laid over a cloned fragment's own 40px box, it would be 60px
    // wide and have its middle at x = 60, left of the fragment's right end.
    let pixels = paint("", |doc, root| {
        two_lines(
            doc,
            root,
            &format!(
                "display:inline;box-decoration-break:clone;{};background-size:150% 100%",
                fixed()
            ),
        );
    });
    for y in [5, 25] {
        for x in [35, 65] {
            assert!(leans_red(&pixels, x, y), "x={x} y={y}");
        }
    }
}

#[test]
fn a_fixed_layer_in_a_transformed_block_stays_in_the_inline_box() {
    // A transform makes its element the containing block of the fixed
    // descendants, so a `fixed` layer in it is positioned like a `scroll` one.
    let pixels = paint("transform:translateX(0px)", |doc, root| {
        two_lines(doc, root, &format!("display:inline;{}", fixed()));
    });
    assert_box_positioned(&pixels);
}

#[test]
fn a_fixed_layer_in_a_filtered_inline_box_stays_in_the_inline_box() {
    // A filter also makes its element the containing block of fixed
    // descendants, the box with the layer included.
    let pixels = paint("", |doc, root| {
        two_lines(
            doc,
            root,
            &format!("display:inline;filter:opacity(1);{}", fixed()),
        );
    });
    assert_box_positioned(&pixels);
}

#[test]
fn a_fixed_layer_inside_a_filtered_inline_element_stays_in_the_inline_box() {
    let pixels = paint("", |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;filter:opacity(1)"),
        );
        two_lines(doc, outer, &format!("display:inline;{}", fixed()));
    });
    assert_box_positioned(&pixels);
}

#[test]
fn a_fixed_layer_inside_a_plain_inline_element_is_positioned_in_the_page_area() {
    let pixels = paint("", |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;color:transparent"),
        );
        two_lines(doc, outer, &format!("display:inline;{}", fixed()));
    });
    assert_page_positioned(&pixels);
}

#[test]
fn a_fixed_layer_of_an_anonymous_table_cell_paragraph_is_positioned_in_the_page_area() {
    // Text beside a table row's cells is wrapped in an anonymous cell, whose
    // lines are painted as a paragraph of their own.
    let pixels = paint("display:table", |doc, root| {
        let row = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:table-row"),
        );
        two_lines(doc, row, &format!("display:inline;{}", fixed()));
    });
    assert_page_positioned(&pixels);
}

#[test]
fn a_fixed_layer_of_a_block_level_pseudo_element_is_positioned_in_the_page_area() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(
        style,
        format!(
            "html, body {{ display: block; margin: 0 }} \
             div::before {{ content: ''; display: block; width: 40px; height: 20px; \
             margin-left: 30px; {}; background-size: 150% 100% }}",
            fixed()
        ),
    );
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    doc.append_text(div, "A");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 80.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &TwoColor)
        .unwrap();
    let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        100,
        80,
    );
    // The image is 150px wide in the page area, so its middle is at x = 75;
    // laid over the pseudo-element's 40px box it would be 60px wide and have
    // its middle at x = 60.
    for x in [35, 65] {
        assert!(leans_red(&pixels, x, 10), "x={x}");
    }
}
