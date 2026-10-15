//! Size containment (CSS Containment 2 §3.1, §3.2): a box with `contain:
//! size` is sized as if it had no content, in both axes, and one with
//! `contain: inline-size` in its inline axis only. The content is still laid
//! out inside it and overflows.
//!
//! Fragment rects are in page coordinates, so they include the 20px page
//! margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    DecodedImage, ImagePixelSource, IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver,
    ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::sync::Arc;
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const PAGE: &str = "@page { size: 300px 200px; margin: 20px } body { margin: 0; font: 20px/1 Ahem } p { margin: 0 }";

/// Every image is 8x4 pixels, a 2:1 aspect ratio.
struct Pixels(Arc<DecodedImage>);

impl ImagePixelSource for Pixels {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::clone(&self.0))
    }
}

impl ReplacedResolver for Pixels {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(8.0, 4.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

/// The border box `(page, x, y, width, height)` of one fragment.
type Fragment = (u32, f32, f32, f32, f32);

/// The border box of every fragment of `#id`, and the page count.
fn fragments(body: &str, id: &str) -> (Vec<Fragment>, u32) {
    let html = format!("<!DOCTYPE html><style>{PAGE}</style>{body}");
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let pixels = Pixels(Arc::new(DecodedImage {
        width: 8,
        height: 4,
        rgba: [0, 0, 255, 255].repeat(32),
    }));
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://size.test/document.html").expect("url"))
        .replaced_resolver(&pixels)
        .image_pixel_source(&pixels);
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
    let mut found = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("page");
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                found.push((index, rect.x, rect.y, rect.width, rect.height));
            }
        }
    }
    assert!(!found.is_empty(), "no fragment for {id} in {html}");
    (found, result.page_count())
}

/// The border box of the first fragment of `#id`.
fn first(body: &str, id: &str) -> (f32, f32, f32, f32) {
    let (found, _) = fragments(body, id);
    let (_, x, y, width, height) = found[0];
    (x, y, width, height)
}

#[test]
fn size_containment_sizes_an_auto_height_block_as_if_empty() {
    for contain in ["size", "strict", "size layout"] {
        let body = format!(
            "<div id=c style=\"contain: {contain}; padding-top: 3px\"><p>XX</p><p>XX</p></div>\
             <div id=next style=\"height: 10px\"></div>"
        );
        // The block still fills its container's width.
        assert_eq!(first(&body, "c"), (20.0, 20.0, 260.0, 3.0), "{contain}");
        // The content overflows instead of pushing the next box down.
        assert_eq!(first(&body, "next"), (20.0, 23.0, 260.0, 10.0), "{contain}");
    }
}

#[test]
fn sizing_properties_still_size_a_size_contained_box() {
    let body = "<div id=a style=\"contain: size; height: 50px\"><p>XX</p></div>\
                <div id=b style=\"contain: size; min-height: 30px\"><p>XX</p><p>XX</p></div>";
    assert_eq!(first(body, "a").3, 50.0);
    assert_eq!(first(body, "b").3, 30.0);
}

#[test]
fn shrink_to_fit_boxes_with_size_containment_have_no_content_width() {
    for display in ["inline-block", "inline-flex", "inline-grid"] {
        let body = format!(
            "<div><span id=c style=\"display: {display}; contain: size; padding: 2px\">XX</span></div>"
        );
        let (_, _, width, height) = first(&body, "c");
        assert_eq!((width, height), (4.0, 4.0), "{display}");
    }
    let body = "<div id=f style=\"float: left; contain: size\">XX</div>";
    let (_, _, width, height) = first(body, "f");
    assert_eq!((width, height), (0.0, 0.0));
}

#[test]
fn a_size_contained_grid_keeps_its_explicit_tracks() {
    let body = "<div id=g style=\"display: inline-grid; contain: size; \
                grid-template-columns: 30px 40px; grid-template-rows: 15px\">\
                <div>XXXXXXXX</div></div>";
    let (_, _, width, height) = first(body, "g");
    assert_eq!((width, height), (70.0, 15.0));
}

#[test]
fn inline_size_containment_only_sizes_the_inline_axis() {
    let body = "<div><span id=c style=\"display: inline-block; contain: inline-size\">\
                <p>XX</p><p>XX</p></span></div>";
    let (_, _, width, height) = first(body, "c");
    // No content width, but its two lines still give it its height.
    assert_eq!((width, height), (0.0, 40.0));
    let body = "<div id=c style=\"contain: inline-size\"><p>XX</p><p>XX</p></div>";
    assert_eq!(first(body, "c"), (20.0, 20.0, 260.0, 40.0));
}

#[test]
fn size_containment_applies_to_blockified_inline_elements() {
    // Floats, absolutely positioned boxes and flex items are block-level
    // whatever their `display`.
    for (body, id) in [
        (
            "<span id=s style=\"float: left; contain: size\">XX</span>",
            "s",
        ),
        (
            "<div style=\"position: relative\"><span id=s style=\"position: absolute; contain: size\">XX</span></div>",
            "s",
        ),
        (
            "<div style=\"display: flex; align-items: start\"><span id=s style=\"contain: size\">XX</span></div>",
            "s",
        ),
    ] {
        let (_, _, width, height) = first(body, id);
        assert_eq!((width, height), (0.0, 0.0), "{body}");
    }
}

#[test]
fn size_containment_applies_to_blockified_rubies() {
    for body in [
        "<ruby id=r style=\"float: left; contain: size\">XX<rt>YY</rt></ruby>",
        "<div style=\"position: relative\"><ruby id=r style=\"position: absolute; contain: size\">XX<rt>YY</rt></ruby></div>",
        "<div style=\"display: flex; align-items: start\"><ruby id=r style=\"contain: size\">XX<rt>YY</rt></ruby></div>",
    ] {
        let (_, _, width, height) = first(body, "r");
        assert_eq!((width, height), (0.0, 0.0), "{body}");
    }
}

#[test]
fn a_size_contained_box_is_not_a_definite_percentage_basis() {
    // The empty-sized height of an auto-height box does not turn a
    // percentage height inside it into a size.
    let body = "<div style=\"contain: size\"><div id=c style=\"height: 50%\">XX</div></div>";
    assert_eq!(first(body, "c").3, 20.0);
}

#[test]
fn size_contained_replaced_elements_have_no_natural_aspect_ratio() {
    // An 8x4 image keeps neither its natural size nor its ratio.
    let body = "<p><img id=i src=a.png></p>";
    assert_eq!(first(body, "i").2, 8.0);
    let body = "<p><img id=i style=\"contain: size\" src=a.png></p>";
    let (_, _, width, height) = first(body, "i");
    assert_eq!((width, height), (0.0, 0.0));
    let body = "<img id=i style=\"display: block; contain: size; width: 100px\" src=a.png>";
    let (_, _, width, height) = first(body, "i");
    assert_eq!((width, height), (100.0, 0.0));
    // Inline-size containment keeps the natural height.
    let body = "<p><img id=i style=\"contain: inline-size\" src=a.png></p>";
    let (_, _, width, height) = first(body, "i");
    assert_eq!((width, height), (0.0, 4.0));
}

#[test]
fn size_containment_does_not_apply_to_inline_boxes() {
    let body = "<div><span id=s style=\"contain: size\">XX</span></div>";
    assert_eq!(first(body, "s").2, 40.0);
}

#[test]
fn a_paginated_size_contained_box_keeps_its_size() {
    // The content of the 10px box runs onto later pages, but the box stays
    // 10px tall on the first page and the next box follows it directly.
    let body = "<div id=c style=\"contain: size; height: 10px\">\
                <div style=\"height: 400px\"></div></div>\
                <div id=next style=\"height: 10px\"></div>";
    let (found, pages) = fragments(body, "c");
    assert_eq!(found, vec![(0, 20.0, 20.0, 260.0, 10.0)]);
    let (next, _) = fragments(body, "next");
    assert_eq!(next, vec![(0, 20.0, 30.0, 260.0, 10.0)]);
    // The overflowing content still counts toward the page count.
    assert!(pages >= 3, "pages {pages}");
    // Without a height the box is empty-sized; its content still runs over
    // the page break without growing it.
    let body = "<div id=c style=\"contain: size; padding-top: 4px\">\
                <p>XX</p><div style=\"height: 400px\"></div></div>\
                <div id=next style=\"height: 10px\"></div>";
    let (found, pages) = fragments(body, "c");
    assert_eq!(found, vec![(0, 20.0, 20.0, 260.0, 4.0)]);
    let (next, _) = fragments(body, "next");
    assert_eq!(next, vec![(0, 20.0, 24.0, 260.0, 10.0)]);
    assert!(pages >= 3, "pages {pages}");
}

#[test]
fn size_containment_applies_to_table_captions() {
    // A caption is not an internal table box.
    let body = "<table><caption id=c style=\"contain: size\">XX</caption>\
                <tr><td style=\"width: 100px\">X</td></tr></table>";
    assert_eq!(first(body, "c").3, 0.0);
}

#[test]
fn inline_size_containment_follows_a_vertical_writing_mode() {
    // The inline axis of a vertical box is its physical height: the two
    // lines still give it its width.
    for mode in ["vertical-rl", "vertical-lr"] {
        let body = format!(
            "<div><span id=c style=\"display: inline-block; writing-mode: {mode}; \
             contain: inline-size\"><p>XX</p><p>XX</p></span></div>"
        );
        let (_, _, width, height) = first(&body, "c");
        assert_eq!((width, height), (40.0, 0.0), "{mode}");
    }
    // An inherited writing mode counts as well.
    let body = "<div style=\"writing-mode: vertical-rl\"><span id=c style=\"display: inline-block; \
                contain: inline-size\"><p>XX</p><p>XX</p></span></div>";
    let (_, _, width, height) = first(body, "c");
    assert_eq!((width, height), (40.0, 0.0));
}

#[test]
fn a_vertical_box_keeps_its_logical_size_under_containment() {
    // `inline-size` sizes the physical height of a vertical box, and its
    // own sizing properties still apply under containment.
    let body = "<div><span id=c style=\"display: inline-block; writing-mode: vertical-rl; \
                contain: inline-size; inline-size: 100px\"><p>XX</p><p>XX</p></span></div>";
    let (_, _, width, height) = first(body, "c");
    assert_eq!((width, height), (40.0, 100.0));
}
