//! External-consumer coverage for page-margin boxes that show an image with
//! `content: url()` (CSS Content 3 §2.2): the render pipeline loads the image
//! and the box lays it out in its line.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, MarginBox, Page,
    PageMarginBoxSlot, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// A 20x10 red PNG.
const RED: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAKCAIAAAA7N+mxAAAAFUlEQVR4nGP4z8BANiJf56jmEaMZADFExzl02osRAAAAAElFTkSuQmCC";
/// A 30x15 blue PNG.
const BLUE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAB4AAAAPCAIAAAB82OjLAAAAGklEQVR4nGNgYPhPMzRq9KjRo0aPGj3SjAYAkEnATh6FXQYAAAAASUVORK5CYII=";

fn lay_out(css: &str, body: &str) -> DocumentLayout {
    let html = format!(
        r#"<!doctype html><style>
          @page {{ size: 300px 200px; margin: 40px }}
          html {{ font: 10px/10px Ahem }}
          body, p {{ margin: 0 }}
          {css}
        </style>{body}"#
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    result
}

fn top_left(page: &Page<'_>) -> MarginBox {
    page.margin_boxes()
        .into_iter()
        .find(|margin_box| margin_box.slot == PageMarginBoxSlot::TopLeft)
        .expect("a top-left margin box")
}

/// The URL and border box of each content image of `margin_box`.
fn images(margin_box: &MarginBox) -> Vec<(String, [f32; 4])> {
    margin_box
        .content_images
        .iter()
        .map(|image| {
            let rect = image.rect;
            (image.url.clone(), [rect.x, rect.y, rect.width, rect.height])
        })
        .collect()
}

#[test]
fn a_margin_box_lays_out_its_image_at_the_size_of_the_image() {
    let layout = lay_out(
        &format!("@page {{ @top-left {{ content: 'ab' url({RED}); vertical-align: top }} }}"),
        "<p>body</p>",
    );
    let pages: Vec<_> = layout.pages().collect();
    let margin_box = top_left(&pages[0]);
    assert_eq!(margin_box.content, "ab");
    // The image is 20x10 and follows the 20px of 'ab'.
    assert_eq!(
        images(&margin_box),
        [(RED.to_owned(), [60.0, 0.0, 20.0, 10.0])]
    );
}

#[test]
fn a_margin_box_whose_image_cannot_be_loaded_shows_the_rest_of_its_content() {
    let layout = lay_out(
        "@page { @top-left { content: 'ab' url(data:image/png;base64,AAAA); vertical-align: top } }",
        "<p>body</p>",
    );
    let pages: Vec<_> = layout.pages().collect();
    let margin_box = top_left(&pages[0]);
    assert_eq!(margin_box.content, "ab");
    assert!(margin_box.content_images.is_empty());
    assert!(margin_box.text.is_some());
}

#[test]
fn each_page_lays_out_the_images_of_its_own_margin_boxes() {
    // The first page shows the red image and the second page the blue one,
    // which only the second page's margin boxes name.
    let layout = lay_out(
        &format!(
            "@page {{ @top-left {{ content: url({BLUE}); vertical-align: top }} }} \
             @page :first {{ @top-left {{ content: url({RED}); vertical-align: top }} }}"
        ),
        "<p style='break-after: page'>one</p><p>two</p>",
    );
    let pages: Vec<_> = layout.pages().collect();
    assert_eq!(pages.len(), 2);
    assert_eq!(
        images(&top_left(&pages[0])),
        [(RED.to_owned(), [40.0, 0.0, 20.0, 10.0])]
    );
    assert_eq!(
        images(&top_left(&pages[1])),
        [(BLUE.to_owned(), [40.0, 0.0, 30.0, 15.0])]
    );
}
