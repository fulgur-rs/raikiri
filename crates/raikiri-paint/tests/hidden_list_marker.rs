//! A list item whose `visibility` is not `visible` paints no marker, and a
//! marker that is visible by its own style paints in a hidden item.

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

/// A 10×10 red image.
struct Red;
impl ImagePixelSource for Red {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        (url.as_str() == "file:///marker.png").then(|| {
            Arc::new(DecodedImage {
                width: 10,
                height: 10,
                rgba: [255, 0, 0, 255].repeat(100),
            })
        })
    }
}

/// How many pixels of a 100×40 page differ from white, for a list whose one
/// item reads `x` and is styled by `css`, in 10px Ahem on 20px lines with
/// black text. The list has `padding-left: 40px`, so an outside marker fits
/// in the page.
fn painted_pixels(item_style: &str, css: &str) -> usize {
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
             ul {{ display: block; margin: 0; padding: 0 0 0 40px }} \
             li {{ display: list-item; font: 10px/20px Ahem; color: black; {item_style} }} {css}"
        ),
    );
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let ul = doc.append_element(Some(body), "ul", Style::default(), None::<&str>);
    let li = doc.append_element(Some(ul), "li", Style::default(), None::<&str>);
    doc.append_text(li, "x");
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    doc.prepare_list_marker_images(&computed, &Red, None);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    layout_single_page(&mut doc, &computed, page).unwrap();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(&mut scene, &doc, &computed, page, &Red).unwrap();
    let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        100,
        40,
    );
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != [255, 255, 255])
        .count()
}

/// The marker styles to try: a text marker and an image marker, outside and
/// inside.
const MARKERS: [&str; 4] = [
    "list-style: decimal outside",
    "list-style: decimal inside",
    "list-style: url(file:///marker.png) outside",
    "list-style: url(file:///marker.png) inside",
];

#[test]
fn a_visible_item_paints_its_marker() {
    // Control: the marker adds pixels to the item's own text.
    for marker in MARKERS {
        assert!(
            painted_pixels(marker, "") > painted_pixels("list-style: none", ""),
            "{marker}"
        );
    }
}

#[test]
fn a_hidden_or_collapsed_item_paints_no_marker() {
    for marker in MARKERS {
        for visibility in ["hidden", "collapse"] {
            let style = format!("{marker}; visibility: {visibility}");
            assert_eq!(painted_pixels(&style, ""), 0, "{style}");
        }
    }
}

#[test]
fn a_marker_that_is_visible_by_its_own_style_paints_in_a_hidden_item() {
    for marker in MARKERS {
        let style = format!("{marker}; visibility: hidden");
        assert!(
            painted_pixels(&style, "li::marker { visibility: visible }") > 0,
            "{style}"
        );
    }
}
