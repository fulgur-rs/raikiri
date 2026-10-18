//! `::first-line` styles reach the text and the inline boxes of the first
//! formatted line (CSS Pseudo 4 section 2.1.2), with or without a
//! `::first-letter`.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

const SIZE: u32 = 50;
const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 128, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Rasterizes `body` in Ahem, where "X" fills its em square: a 10px line of
/// "X" is 100 pixels of its color.
fn raster(body: &str, css: &str) -> Vec<u8> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:{SIZE}px {SIZE}px;margin:0}}body{{margin:0;background:white;font:10px/10px Ahem}}p{{margin:0}}{css}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).unwrap();
    let (doc, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_origin(&mut scene, doc, cascade, page_box, origin)
        .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        SIZE,
        SIZE,
    )
}

fn count(rgba: &[u8], color: [u8; 4]) -> usize {
    rgba.chunks_exact(4).filter(|pixel| *pixel == color).count()
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn the_first_line_takes_the_color_without_a_first_letter() {
    let rgba = raster("<p>X</p>", "p::first-line{color:blue}");
    assert_eq!(count(&rgba, BLUE), 100);
    assert_eq!(count(&rgba, BLACK), 0);
}

#[test]
fn only_the_first_formatted_line_takes_it() {
    let rgba = raster("<p>X<br>X</p>", "p{color:red} p::first-line{color:blue}");
    assert_eq!(count(&rgba, BLUE), 100);
    assert_eq!(count(&rgba, RED), 100);
    assert_eq!(pixel(&rgba, 5, 5), BLUE);
    assert_eq!(pixel(&rgba, 5, 15), RED);
}

#[test]
fn a_wrapped_paragraph_styles_its_first_wrapped_line() {
    let rgba = raster(
        "<p>XX XX</p>",
        "p{width:20px;color:red} p::first-line{color:blue}",
    );
    assert_eq!(count(&rgba, BLUE), 200);
    assert_eq!(count(&rgba, RED), 200);
}

#[test]
fn inline_elements_inherit_it_unless_they_set_their_own() {
    let rgba = raster(
        "<p>A<span>B</span><span class=own>C</span><br>D<span>E</span></p>",
        "p{color:red} p::first-line{color:blue} .own{color:green}",
    );
    assert_eq!(count(&rgba, BLUE), 200);
    assert_eq!(count(&rgba, GREEN), 100);
    assert_eq!(count(&rgba, RED), 200);
}

#[test]
fn an_inline_element_split_over_lines_has_the_style_on_its_first_line_only() {
    let rgba = raster(
        "<p><span>A<br>B</span></p>",
        "p{color:red} p::first-line{color:blue}",
    );
    assert_eq!(pixel(&rgba, 5, 5), BLUE);
    assert_eq!(pixel(&rgba, 5, 15), RED);
}

#[test]
fn borders_in_currentcolor_use_the_first_line_color_on_the_first_line() {
    // The borders of the two pieces of the `span`: its start edge on the
    // first line, its end edge on the second.
    let rgba = raster(
        "<p>A<span>B<br>C</span></p>",
        "p{color:red} p::first-line{color:blue} span{border:1px solid currentColor}",
    );
    assert_eq!(pixel(&rgba, 10, 5), BLUE, "left border of the first piece");
    assert_eq!(pixel(&rgba, 15, 0), BLUE, "top border of the first piece");
    assert_eq!(
        pixel(&rgba, 10, 15),
        RED,
        "right border of the second piece"
    );
    assert_eq!(
        pixel(&rgba, 5, 20),
        RED,
        "bottom border of the second piece"
    );
}

#[test]
fn the_first_line_font_size_changes_the_line() {
    let rgba = raster(
        "<p>X<br>X</p>",
        "p{line-height:normal;color:red} p::first-line{font-size:20px;color:blue}",
    );
    assert_eq!(count(&rgba, BLUE), 400);
    assert_eq!(count(&rgba, RED), 100);
    assert_eq!(pixel(&rgba, 15, 15), BLUE);
    assert_eq!(pixel(&rgba, 5, 25), RED);
}

#[test]
fn a_first_letter_inherits_the_first_line_color() {
    let rgba = raster(
        "<p>XX<br>X</p>",
        "p{color:red} p::first-line{color:blue} p::first-letter{font-size:10px}",
    );
    assert_eq!(count(&rgba, BLUE), 200);
    assert_eq!(count(&rgba, RED), 100);
}

#[test]
fn a_block_around_the_paragraph_gives_its_first_line_over_the_blocks_between() {
    // The inner blocks have a `color` of their own, which is not the color
    // of the first line they are on.
    let rgba = raster(
        "<div class=outer><div class=mid><p>X<br>X</p></div></div>",
        ".outer::first-line{color:green} .mid{color:blue} p{color:red}",
    );
    assert_eq!(pixel(&rgba, 5, 5), GREEN);
    assert_eq!(pixel(&rgba, 5, 15), RED);
}

#[test]
fn generated_content_on_the_first_line_inherits_it() {
    let rgba = raster(
        "<p>A</p>",
        "p{color:red} p::first-line{color:blue} p::before{content:'B'} p::after{content:'C';color:green}",
    );
    assert_eq!(count(&rgba, BLUE), 200);
    assert_eq!(count(&rgba, GREEN), 100);
    assert_eq!(count(&rgba, RED), 0);
}

#[test]
fn a_nested_block_holds_the_first_line_of_its_container() {
    // The first formatted line of `.outer` is the first line of the block
    // inside it; the text after the block is on a later line.
    let rgba = raster(
        "<div class=outer><div>X</div>X</div>",
        ".outer{color:red} .outer::first-line{color:blue}",
    );
    assert_eq!(pixel(&rgba, 5, 5), BLUE);
    assert_eq!(pixel(&rgba, 5, 15), RED);
}
