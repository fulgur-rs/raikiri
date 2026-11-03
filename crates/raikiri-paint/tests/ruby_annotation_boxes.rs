//! The boxes inside a ruby annotation paint like the boxes of any other
//! inline content: the `ruby-text` box and the inline elements in it draw
//! their background, border and outline where the layout puts them.
//!
//! Each fixture is laid out through `raikiri_html::layout` and its first page
//! rendered to pixels. A box is found by the pixels of its (opaque) colour,
//! and compared with the rectangle of its fragment on the page.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderFonts, RenderResources, layout, parse_html_with_resources,
};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const WIDTH: usize = 400;
const HEIGHT: usize = 200;

const YELLOW: [u8; 3] = [255, 255, 0];
const RED: [u8; 3] = [255, 0, 0];
const LIME: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn fonts() -> RenderFonts {
    FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts")
}

/// A page with a paragraph holding `ruby`, in 20px Ahem with 10px
/// annotations.
fn lay_out(ruby: &str) -> DocumentLayout {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: {WIDTH}px {HEIGHT}px; margin: 0 }} \
         body {{ margin: 8px; font: 20px/20px Ahem }} rt {{ font-size: 10px }}</style>\
         <p>{ruby}</p>"
    );
    let resources = RenderResources::new().fonts(fonts());
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
    result
}

fn render_first_page(result: &DocumentLayout) -> Vec<u8> {
    let page = result.pages().next().expect("a page");
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = anyrender::Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, document, cascade, page_box, origin)
        .expect("paint");
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| {
            use anyrender::PaintScene;
            out.append_scene(scene, kurbo::Affine::IDENTITY);
        },
        WIDTH as u32,
        HEIGHT as u32,
    )
}

/// The pixels of exactly one colour: how many, and the first and last
/// column and row they are in.
#[derive(Debug, PartialEq)]
struct Painted {
    count: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

fn painted(pixels: &[u8], rgb: [u8; 3]) -> Option<Painted> {
    let mut found: Option<Painted> = None;
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        if pixel[..3] != rgb {
            continue;
        }
        let (x, y) = (index % WIDTH, index / WIDTH);
        let found = found.get_or_insert(Painted {
            count: 0,
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        });
        found.count += 1;
        found.x0 = found.x0.min(x);
        found.y0 = found.y0.min(y);
        found.x1 = found.x1.max(x);
        found.y1 = found.y1.max(y);
    }
    found
}

/// The border box `(x, y, width, height)` of the element with the `id`. The
/// fixtures place every box on whole pixels, so the box can be compared with
/// the pixels it paints.
fn rect_of(result: &DocumentLayout, id: &str) -> (usize, usize, usize, usize) {
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let rect = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| fragment.rect())
        .unwrap_or_else(|| panic!("no fragment for #{id}"));
    for edge in [rect.x, rect.y, rect.width, rect.height] {
        assert_eq!(edge.fract(), 0.0, "#{id} is {rect:?}");
    }
    (
        rect.x as usize,
        rect.y as usize,
        rect.width as usize,
        rect.height as usize,
    )
}

/// The colour of the pixel at `(x, y)`.
fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let at = (y * WIDTH + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2]]
}

/// The background of a `ruby-text` box fills exactly the box the layout
/// gives the annotation.
#[test]
fn the_background_of_an_rt_fills_its_box() {
    // Transparent text leaves the whole background visible.
    let result = lay_out(
        "<ruby>AAA<rt id=rt style=\"background: yellow; color: transparent\">xx</rt></ruby>",
    );
    let (x, y, width, height) = rect_of(&result, "rt");
    assert_eq!((width, height), (60, 10), "the annotation spans its base");
    let yellow = painted(&render_first_page(&result), YELLOW).expect("a background");
    assert_eq!(
        yellow,
        Painted {
            count: width * height,
            x0: x,
            y0: y,
            x1: x + width - 1,
            y1: y + height - 1,
        }
    );
}

/// An outline on a `ruby-text` box is a ring of the outline's width around
/// the box, outside of it. The line of the base below may paint over its
/// bottom side, so that side is left out.
#[test]
fn the_outline_of_an_rt_surrounds_its_box() {
    let result = lay_out("<ruby>AAA<rt id=rt style=\"outline: 2px solid red\">xx</rt></ruby>");
    let (x, y, width, height) = rect_of(&result, "rt");
    let pixels = render_first_page(&result);
    let (mid_x, mid_y) = (x + width / 2, y + height / 2);
    for ring in 1..=2 {
        assert_eq!(pixel(&pixels, x - ring, mid_y), RED, "left, {ring}px out");
        assert_eq!(pixel(&pixels, mid_x, y - ring), RED, "top, {ring}px out");
        assert_eq!(
            pixel(&pixels, x + width - 1 + ring, mid_y),
            RED,
            "right, {ring}px out"
        );
    }
    // The ring stops at its width, and the box itself is not painted.
    assert_ne!(pixel(&pixels, x - 3, mid_y), RED);
    assert_ne!(pixel(&pixels, x + width / 2, y + 1), RED);
}

/// An inline element in an annotation is painted where its fragment is: its
/// border follows the edge of the fragment, and its background fills the
/// padding box inside that.
#[test]
fn an_inline_element_in_an_annotation_paints_its_background_and_border() {
    // Transparent text leaves the whole background visible, and an annotation
    // as wide as its base has no space to spread, which keeps the element on
    // whole pixels.
    let result = lay_out(
        "<ruby>AAA<rt style=\"color: transparent; font-size: 20px\">x\
         <span id=sp style=\"background: lime; border: 1px solid blue\">y</span>x</rt></ruby>",
    );
    let pixels = render_first_page(&result);
    let lime = painted(&pixels, LIME).expect("a background");
    let blue = painted(&pixels, BLUE).expect("a border");
    let (x, y, width, height) = rect_of(&result, "sp");
    assert_eq!(
        (blue.x0, blue.y0, blue.x1),
        (x, y, x + width - 1),
        "the border is the outer edge of the fragment"
    );
    // The line of the base starts at the bottom border, so the last row of
    // the border and of the padding box may be hidden under it.
    assert!(
        lime.y1 + 2 >= y + height - 1 && lime.y1 < y + height - 1,
        "{lime:?}"
    );
    assert_eq!(
        (lime.x0, lime.y0, lime.x1),
        (x + 1, y + 1, x + width - 2),
        "the padding box lies inside the border"
    );
    assert_eq!(
        lime.count,
        (lime.x1 - lime.x0 + 1) * (lime.y1 - lime.y0 + 1),
        "the padding box is filled"
    );
}
