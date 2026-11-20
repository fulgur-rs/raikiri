//! The width a block-level `::before` or `::after` adds to a shrink-to-fit
//! box.
//!
//! The pseudo-element is a block box among the lines of its parent's
//! paragraph. Like any block child, it adds its own width to the parent's
//! min- and max-content widths (CSS Sizing 3 section 5.1): a definite width
//! stands for its content, and an `auto` one is the width of its content
//! clamped by `min-width` and `max-width`, with its padding, border and
//! margins added. Its content is not measured as text of the parent. The
//! expected boxes are the ones Chromium lays out for the same markup, in an
//! 800px by 600px viewport without a body margin and with Ahem at 10px, where
//! the eight letters of `xxxxxxxx` are 80px wide.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;

/// Every image is 20x50 pixels.
struct Portrait;

impl ImagePixelSource for Portrait {
    fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage::new(20, 50, vec![0; 20 * 50 * 4])))
    }
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border-box width and height of `#s` when `body` is laid out on a page
/// without margins.
fn size_of_s(body: &str) -> [f32; 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(url::Url::parse("https://images.test/document.html").expect("url"))
        .image_pixel_source(&Portrait);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let rect = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some("s"))
        .unwrap_or_else(|| panic!("no fragment for #s in {body}"))
        .rect();
    [rect.width, rect.height]
}

/// `#s` with `host` style, the pseudo-element `pseudo` of `#s` with
/// `content: content; display: block` and `style`, and `text` in `#s`.
fn pseudo_in(host: &str, pseudo: &str, style: &str, content: &str, text: &str) -> String {
    format!(
        "<style>#s::{pseudo} {{ content: \"{content}\"; display: block; {style} }}</style>\
         <span id=s style=\"{host}\">{text}</span>"
    )
}

/// Check that `#s` is `expected` wide and high in every case.
fn check(cases: &[(&str, String, [f32; 2])]) {
    for (what, body, expected) in cases {
        assert_eq!(size_of_s(body), *expected, "{what}: {body}");
    }
}

const FLOAT: &str = "float: left";
const INLINE_BLOCK: &str = "display: inline-block";

#[test]
fn a_definite_width_stands_for_the_content() {
    check(&[
        (
            "a float holding only the box",
            pseudo_in(FLOAT, "before", "width: 40px", "xxxxxxxx", ""),
            [40.0, 10.0],
        ),
        (
            "an inline-block with text after the box",
            pseudo_in(INLINE_BLOCK, "before", "width: 40px", "xxxxxxxx", "ab"),
            [40.0, 20.0],
        ),
        (
            "an ::after",
            pseudo_in(FLOAT, "after", "width: 40px", "xxxxxxxx", "ab"),
            [40.0, 20.0],
        ),
        (
            "the parent's own text is wider",
            pseudo_in(FLOAT, "before", "width: 40px", "xx xx xx xx", "abcdefghij"),
            [100.0, 50.0],
        ),
        (
            "padding and a border",
            pseudo_in(
                FLOAT,
                "before",
                "width: 40px; padding-left: 10px; border-left: 5px solid",
                "xxxxxxxx",
                "a",
            ),
            [55.0, 20.0],
        ),
        (
            "a border-box width",
            pseudo_in(
                FLOAT,
                "before",
                "width: 40px; box-sizing: border-box; padding: 0 5px",
                "xxxxxxxx",
                "a",
            ),
            [40.0, 20.0],
        ),
        (
            "a margin",
            pseudo_in(
                FLOAT,
                "before",
                "width: 40px; margin-left: 20px",
                "xxxxxxxx",
                "a",
            ),
            [60.0, 20.0],
        ),
        (
            "auto margins",
            pseudo_in(
                FLOAT,
                "before",
                "width: 40px; margin: 0 auto",
                "xxxxxxxx",
                "a",
            ),
            [40.0, 20.0],
        ),
    ]);
}

#[test]
fn an_auto_width_is_the_content_clamped_with_its_edges() {
    check(&[
        (
            "a maximum width",
            pseudo_in(FLOAT, "before", "max-width: 30px", "xx xx xx", "ab"),
            [30.0, 40.0],
        ),
        (
            "a maximum width with padding",
            pseudo_in(
                INLINE_BLOCK,
                "before",
                "max-width: 30px; padding-left: 10px",
                "xxxxxxxx",
                "a",
            ),
            [40.0, 20.0],
        ),
        (
            "a minimum width",
            pseudo_in(FLOAT, "before", "min-width: 120px", "xx", "ab"),
            [120.0, 20.0],
        ),
        (
            "padding around the content",
            pseudo_in(FLOAT, "before", "padding: 0 5px", "xx xx", "a"),
            [60.0, 20.0],
        ),
        (
            "a percentage width counts as auto",
            pseudo_in(FLOAT, "before", "width: 50%", "xxxxxxxx", "ab"),
            [80.0, 20.0],
        ),
        (
            "a text-indent of the box's own lines",
            pseudo_in(FLOAT, "before", "text-indent: 15px", "xx xx", "a"),
            [65.0, 20.0],
        ),
        (
            "size containment",
            pseudo_in(FLOAT, "before", "contain: inline-size", "xxxxxxxx", "ab"),
            [20.0, 20.0],
        ),
        (
            "size containment with an intrinsic width",
            pseudo_in(
                FLOAT,
                "before",
                "contain: inline-size; contain-intrinsic-width: 30px",
                "xxxxxxxx",
                "ab",
            ),
            [30.0, 20.0],
        ),
    ]);
}

#[test]
fn the_min_content_width_takes_the_box_apart() {
    check(&[
        (
            "the box's words",
            "<style>#s::before { content: \"xx xx xx\"; display: block }</style>\
             <div id=s style=\"display: inline-block; width: min-content\">a</div>"
                .to_owned(),
            [20.0, 40.0],
        ),
        (
            "a box that does not wrap",
            "<style>#s::before { content: \"xx xx xx\"; display: block; \
             white-space: nowrap }</style>\
             <div id=s style=\"display: inline-block; width: min-content\">a</div>"
                .to_owned(),
            [80.0, 20.0],
        ),
    ]);
}

#[test]
fn a_box_of_an_inline_descendant_counts_too() {
    check(&[(
        "the ::before of a span",
        "<style>.p::before { content: \"xxxxxxxx\"; display: block; width: 40px }</style>\
         <div id=s style=\"float: left\">ab<span class=p>c</span></div>"
            .to_owned(),
        [40.0, 30.0],
    )]);
}

#[test]
fn an_empty_box_or_one_with_an_image_counts_too() {
    check(&[
        (
            "an empty box",
            pseudo_in(FLOAT, "before", "width: 40px; height: 5px", "", "a"),
            [40.0, 15.0],
        ),
        (
            "an empty box ahead of the first letter",
            "<style>#s::first-letter { color: red } \
             #s::before { content: \"\"; display: block; width: 40px }</style>\
             <span id=s style=\"float: left\">abc</span>"
                .to_owned(),
            [40.0, 10.0],
        ),
        (
            "a percentage text-indent counts as zero",
            pseudo_in(FLOAT, "before", "text-indent: 50%", "xx xx", "a"),
            [50.0, 30.0],
        ),
    ]);
    // An image and text: the image is 20px wide.
    let image = |style: &str| {
        format!(
            "<style>#s::before {{ content: url(image.png) \"xxxx\"; display: block; \
             {style} }}</style><span id=s style=\"float: left\">a</span>"
        )
    };
    assert_eq!(size_of_s(&image(""))[0], 60.0);
    assert_eq!(size_of_s(&image("width: 30px"))[0], 30.0);
}

#[test]
fn the_first_letter_and_text_indent_count_as_in_the_lines() {
    // The first letter of the box's content is 30px wide.
    let first_letter = |text: &str| {
        format!(
            "<style>#s::first-letter {{ font-size: 30px }} \
             #s::before {{ content: \"xxxx\"; display: block }}</style>\
             <div id=s style=\"display: inline-block; width: min-content\">{text}</div>"
        )
    };
    assert_eq!(size_of_s(&first_letter("abcdef"))[0], 60.0);
    check(&[
        (
            "the length of a calc() text-indent of the box",
            pseudo_in(
                FLOAT,
                "before",
                "text-indent: calc(10% + 30px)",
                "xxxx",
                "a",
            ),
            [70.0, 20.0],
        ),
        (
            "the length of a calc() text-indent of the parent",
            "<span id=s style=\"float: left; text-indent: calc(10% + 30px)\">ab</span>".to_owned(),
            [50.0, 10.0],
        ),
    ]);
}
