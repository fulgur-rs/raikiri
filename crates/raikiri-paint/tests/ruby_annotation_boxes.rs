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
    lay_out_with_css(ruby, "")
}

fn lay_out_with_css(ruby: &str, extra_css: &str) -> DocumentLayout {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: {WIDTH}px {HEIGHT}px; margin: 0 }} \
         body {{ margin: 8px; font: 20px/20px Ahem }} rt {{ font-size: 10px }} {extra_css}</style>\
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

#[test]
fn an_rtc_paints_its_background_around_the_used_annotation_box() {
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background: yellow; color: transparent\">\
         <rt id=rt>xx</rt></rtc></ruby>",
    );
    let rt = rect_of(&result, "rt");
    assert_eq!(rt, (8, 20, 60, 10));
    let (x, y, width, height) = rect_of(&result, "rtc");
    assert_eq!((x, y, width, height), rt);
    assert_eq!(
        painted(&render_first_page(&result), YELLOW),
        Some(Painted {
            count: width * height,
            x0: x,
            y0: y,
            x1: x + width - 1,
            y1: y + height - 1,
        })
    );
}

#[test]
fn an_rtc_outline_surrounds_its_used_box() {
    let result =
        lay_out("<ruby>AAA<rtc id=rtc style=\"outline: 2px solid red\"><rt>xx</rt></rtc></ruby>");
    let (x, y, width, height) = rect_of(&result, "rtc");
    let pixels = render_first_page(&result);
    let (mid_x, mid_y) = (x + width / 2, y + height / 2);
    for ring in 1..=2 {
        assert_eq!(pixel(&pixels, x - ring, mid_y), RED);
        assert_eq!(pixel(&pixels, mid_x, y - ring), RED);
        assert_eq!(pixel(&pixels, x + width - 1 + ring, mid_y), RED);
    }
    assert_ne!(pixel(&pixels, x - 3, mid_y), RED);
}

#[test]
fn rtc_edges_do_not_change_geometry_or_paint_a_border() {
    let baseline = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background:yellow;color:transparent\"><rt id=rt>xx</rt></rtc></ruby>",
    );
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background:yellow;color:transparent;\
         margin:40px;padding:30px;border:20px solid red;\
         border-image:linear-gradient(red,red) 1;border-radius:30px;\
         background-clip:content-box;background-origin:content-box\"><rt id=rt>xx</rt></rtc></ruby>",
    );
    assert_eq!(rect_of(&result, "rt"), rect_of(&baseline, "rt"));
    assert_eq!(rect_of(&result, "rtc"), rect_of(&baseline, "rtc"));
    assert!(
        render_first_page(&result) == render_first_page(&baseline),
        "RTC edges changed pixels"
    );
    assert_eq!(painted(&render_first_page(&result), RED), None);
}

#[test]
fn one_rtc_background_surrounds_multiple_rt_boxes_without_covering_them() {
    let result = lay_out(
        "<ruby><rb>A</rb><rb>B</rb><rtc id=rtc style=\"background:yellow;color:transparent\">\
         <rt id=a style=\"background:lime\">x</rt><rt id=b>y</rt></rtc></ruby>",
    );
    assert_eq!(rect_of(&result, "rtc"), (8, 20, 40, 10));
    assert_eq!(rect_of(&result, "a"), (8, 20, 20, 10));
    assert_eq!(rect_of(&result, "b"), (28, 20, 20, 10));
    let pixels = render_first_page(&result);
    assert_eq!(painted(&pixels, LIME).unwrap().count, 200);
    assert_eq!(painted(&pixels, YELLOW).unwrap().count, 200);
}

#[test]
fn separate_rtc_levels_paint_in_their_own_positions() {
    let result = lay_out(
        "<ruby>AAA<rtc id=over style=\"background:yellow;color:transparent\"><rt>x</rt></rtc>\
         <rtc id=under style=\"background:blue;color:transparent;ruby-position:under\"><rt>y</rt></rtc></ruby>",
    );
    assert_eq!(rect_of(&result, "over"), (8, 20, 60, 10));
    assert_eq!(rect_of(&result, "under"), (8, 50, 60, 10));
    let pixels = render_first_page(&result);
    assert_eq!(painted(&pixels, YELLOW).unwrap().count, 600);
    assert_eq!(painted(&pixels, BLUE).unwrap().count, 600);
}

#[test]
fn relative_rtc_and_rt_offsets_are_applied_once_after_bounds_are_sized() {
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background:yellow;color:transparent;position:relative;left:7px;top:3px\">\
         <rt id=rt style=\"background:lime;position:relative;left:2px;top:1px\">xx</rt></rtc></ruby>",
    );
    // Fragment rectangles are in layout space, before relative offsets.
    assert_eq!(rect_of(&result, "rtc"), (8, 20, 60, 10));
    assert_eq!(rect_of(&result, "rt"), (8, 20, 60, 10));
    let pixels = render_first_page(&result);
    assert_eq!(pixel(&pixels, 15, 23), YELLOW);
    assert_eq!(pixel(&pixels, 17, 24), LIME);
}

#[test]
fn a_visible_rtc_paints_around_a_hidden_rt() {
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background:yellow\"><rt style=\"visibility:hidden\">xx</rt></rtc></ruby>",
    );
    assert_eq!(rect_of(&result, "rtc"), (8, 20, 60, 10));
    assert_eq!(
        painted(&render_first_page(&result), YELLOW).unwrap().count,
        600
    );
}

#[test]
fn a_hidden_rtc_does_not_paint_its_own_background() {
    let result = lay_out(
        "<ruby>AAA<rtc style=\"background:yellow;visibility:hidden\"><rt style=\"visibility:visible\">xx</rt></rtc></ruby>",
    );
    assert_eq!(painted(&render_first_page(&result), YELLOW), None);
}

#[test]
fn collapsed_annotations_leave_no_rtc_background() {
    let result = lay_out(
        "<ruby>AAA<rtc style=\"background:yellow\"><rt style=\"visibility:collapse\">xx</rt></rtc></ruby>",
    );
    assert_eq!(painted(&render_first_page(&result), YELLOW), None);
}

#[test]
fn an_outer_rtc_includes_participating_nested_ruby_bounds() {
    let result = lay_out(
        "<ruby>AAA<rtc id=outer style=\"background:yellow;color:transparent\"><rt>\
         <ruby>A<rtc id=inner style=\"background:lime\"><rt>x</rt></rtc></ruby>\
         </rt></rtc></ruby>",
    );
    let outer = rect_of(&result, "outer");
    let inner = rect_of(&result, "inner");
    assert_eq!((outer.2, outer.3), (60, 20));
    assert_eq!((inner.2, inner.3), (10, 10));
    assert!(inner.0 >= outer.0 && inner.1 >= outer.1);
    assert!(inner.0 + inner.2 <= outer.0 + outer.2);
    assert!(inner.1 + inner.3 <= outer.1 + outer.3);
    let pixels = render_first_page(&result);
    assert_eq!(painted(&pixels, LIME).unwrap().count, 100);
    assert_eq!(painted(&pixels, YELLOW).unwrap().count, 1100);
}

#[test]
fn vertical_rtc_backgrounds_follow_the_annotation_axes() {
    for mode in ["vertical-lr", "vertical-rl"] {
        let result = lay_out_with_css(
            "<ruby style=\"color:transparent\">AAA<rtc id=rtc style=\"background:yellow\"><rt id=rt>xx</rt></rtc></ruby>",
            &format!("p {{ margin:0; writing-mode:{mode}; height:120px }}"),
        );
        let rtc = rect_of(&result, "rtc");
        assert_eq!((rtc.2, rtc.3), (10, 60));
        assert_eq!(rtc, rect_of(&result, "rt"));
        assert_eq!(
            painted(&render_first_page(&result), YELLOW).unwrap().count,
            600
        );
    }
}

#[test]
fn rtl_rtc_content_keeps_its_background_bounds() {
    let result = lay_out(
        "<ruby dir=rtl>AAA<rtc id=rtc style=\"background:yellow;color:transparent\"><rt>xx</rt></rtc></ruby>",
    );
    assert_eq!(rect_of(&result, "rtc"), (8, 20, 60, 10));
    assert_eq!(
        painted(&render_first_page(&result), YELLOW).unwrap().count,
        600
    );
}

#[test]
fn inter_character_rtc_uses_the_base_container_content_box() {
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style=\"background:yellow;color:transparent;ruby-position:inter-character\"><rt>xx</rt></rtc></ruby>",
    );
    let rect = rect_of(&result, "rtc");
    assert_eq!(rect, (8, 20, 70, 20));
    assert_eq!(
        painted(&render_first_page(&result), YELLOW).unwrap().count,
        200
    );
}

#[test]
fn vertical_rtc_background_has_the_same_base_glyph_occlusion_as_rt_background() {
    for mode in ["vertical-lr", "vertical-rl"] {
        let css = format!("p {{ margin:0; writing-mode:{mode}; height:120px }}");
        let rtc = lay_out_with_css(
            "<ruby>AAA<rtc style=\"background:yellow;color:transparent\"><rt>xx</rt></rtc></ruby>",
            &css,
        );
        let rt = lay_out_with_css(
            "<ruby>AAA<rt style=\"background:yellow;color:transparent\">xx</rt></ruby>",
            &css,
        );
        assert!(render_first_page(&rtc) == render_first_page(&rt), "{mode}");
    }
}

#[test]
fn empty_annotations_do_not_borrow_the_size_of_the_base() {
    for content in ["<rt></rt>", "<rt> </rt>", "<span></span>"] {
        let result = lay_out(&format!(
            "<ruby>AAA<rtc id=rtc style='font-size:10px;background:yellow;color:transparent'>{content}</rtc></ruby>"
        ));
        assert_eq!(rect_of(&result, "rtc"), (38, 20, 0, 10));
        assert_eq!(painted(&render_first_page(&result), YELLOW), None);
    }
}

#[test]
fn anonymous_annotation_content_keeps_its_rtc_identity() {
    for (content, y) in [
        ("<span style='display:contents'>x</span>", 20),
        (
            "<span style='display:inline-block;width:10px;height:10px'></span>",
            22,
        ),
        ("<span style='visibility:hidden'>x</span>", 20),
    ] {
        let result = lay_out(&format!(
            "<ruby>AAA<rtc id=rtc style='font-size:10px;background:yellow;color:transparent'>{content}</rtc></ruby>"
        ));
        assert_eq!(rect_of(&result, "rtc"), (8, y, 60, 10));
        assert_eq!(
            painted(&render_first_page(&result), YELLOW).unwrap().count,
            600
        );
    }
}

#[test]
fn unretained_anonymous_whitespace_and_breaks_do_not_guess_a_container() {
    for content in [" ", "<br>"] {
        let result = lay_out(&format!(
            "<ruby>AAA<rtc style='background:yellow'>{content}</rtc></ruby>"
        ));
        assert_eq!(painted(&render_first_page(&result), YELLOW), None);
    }
}

#[test]
fn mixed_direction_content_uses_the_same_paint_bounds_as_its_rt() {
    let rtc = lay_out(
        "<ruby>AAA<rtc style='font-size:10px;background:yellow;color:transparent'><rt>ab אב cd</rt></rtc></ruby>",
    );
    let rt =
        lay_out("<ruby>AAA<rt style='background:yellow;color:transparent'>ab אב cd</rt></ruby>");
    assert!(render_first_page(&rtc) == render_first_page(&rt));
}

#[test]
fn rtc_background_is_fragmented_when_its_bases_wrap() {
    let result = lay_out_with_css(
        "<ruby><rb>一</rb><rb>二</rb><rb>三</rb><rtc style='background:yellow;color:transparent'>\
         <rt>x</rt><rt>y</rt><rt>z</rt></rtc></ruby>",
        "p { width:40px } rb { white-space:normal }",
    );
    let pixels = render_first_page(&result);
    assert_eq!(painted(&pixels, YELLOW).unwrap().count, 600);
    let mut bands = 0;
    let mut previous = false;
    for row in pixels.chunks_exact(WIDTH * 4) {
        let yellow = row.chunks_exact(4).any(|pixel| pixel[..3] == YELLOW);
        if yellow && !previous {
            bands += 1;
        }
        previous = yellow;
    }
    assert_eq!(bands, 2);
}

#[test]
fn distinct_nested_rtcs_survive_anonymous_root_id_collisions() {
    let result = lay_out(
        "<ruby><ruby><rtc id=inner style='font-size:10px;background:yellow;color:transparent'>x</rtc></ruby>\
         <rtc id=outer style='font-size:10px;background:lime;color:transparent'>\
         <span style='display:inline-block;width:10px;height:10px'></span></rtc></ruby>",
    );
    for id in ["inner", "outer"] {
        let rect = rect_of(&result, id);
        assert!(rect.2 > 0 && rect.3 > 0, "{id}: {rect:?}");
    }
    let pixels = render_first_page(&result);
    assert!(painted(&pixels, YELLOW).is_some());
    assert!(painted(&pixels, LIME).is_some());
}

#[test]
fn a_misparented_rtc_keeps_the_edges_of_its_atomic_fallback() {
    let result = lay_out(
        "<span style='display:ruby-text-container;background:yellow;border:2px solid red'>x</span>",
    );
    assert!(painted(&render_first_page(&result), RED).is_some());
}

#[test]
fn ordinary_inline_overflow_inside_nested_ruby_does_not_expand_rtc_bounds() {
    let result = lay_out(
        "<ruby>AAA<rtc id=rtc style='background:yellow;color:transparent'><rt id=root>\
         <ruby><span id=overflow style='border:20px solid blue'></span><rt id=nested>\
         <span style='display:inline-block;width:10px;height:10px'></span>\
         </rt></ruby></rt></rtc></ruby>",
    );
    let rect = rect_of(&result, "rtc");
    // The retained RT boxes bound y=22..62. The ordinary span's border
    // extends to y=82 without enlarging the annotation container.
    assert_eq!(rect_of(&result, "root"), (8, 52, 60, 10));
    assert_eq!(rect_of(&result, "nested"), (18, 22, 40, 10));
    assert_eq!(rect_of(&result, "overflow"), (18, 32, 40, 50));
    assert_eq!(rect, (8, 22, 60, 40));
}

#[test]
fn positioned_descendants_use_the_same_zero_edge_rtc_box() {
    let build = |edges: &str| {
        lay_out(&format!(
            "<ruby>AAA<rtc style='background:yellow;color:transparent;position:relative;left:7px;top:3px;{edges}'>\
             <rt>xx<span style='position:absolute;left:100%;top:100%;width:3px;height:4px;background:blue'></span></rt>\
             </rtc></ruby>"
        ))
    };
    let plain = render_first_page(&build(""));
    let edges = render_first_page(&build("margin:40px;padding:30px;border:20px solid red"));
    assert!(plain == edges);
    let blue = painted(&plain, BLUE).unwrap();
    assert_eq!(
        (blue.x0, blue.y0, blue.x1, blue.y1, blue.count),
        (75, 33, 77, 36, 12)
    );
}
