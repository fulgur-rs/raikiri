//! Border images replace the border styles where they apply.

use anyrender::{PaintScene, Scene, recording::RenderCommand};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

/// Paint `body` on a 50×50 page.
fn painted(css: &str, body: &str) -> Scene {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0}}{css}</style>{body}"
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
    scene
}

/// Render `body` on a 50×50 page and return the RGBA pixels.
fn raster(css: &str, body: &str) -> Vec<u8> {
    let scene = painted(css, body);
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        50,
        50,
    )
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 3] {
    let index = (y * 50 + x) * 4;
    [pixels[index], pixels[index + 1], pixels[index + 2]]
}

const LIME: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn a_box_border_image_replaces_its_border() {
    let pixels = raster(
        "div{width:20px;height:20px;border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 1}",
        "<div></div>",
    );
    assert_eq!(pixel(&pixels, 2, 15), LIME);
}

#[test]
fn the_root_element_border_image_replaces_its_border() {
    let pixels = raster(
        "html{border:5px solid blue;border-image:linear-gradient(lime,lime) 1}",
        "",
    );
    assert_eq!(pixel(&pixels, 2, 25), LIME);
}

#[test]
fn collapsed_table_cells_keep_their_borders() {
    let pixels = raster(
        "table{border-collapse:collapse}\
         td{padding:0;width:20px;height:20px;border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 1}",
        "<table><tr><td></td></tr></table>",
    );
    let row: Vec<_> = (0..40).map(|x| pixel(&pixels, x, 12)).collect();
    assert!(row.contains(&BLUE), "{row:?}");
    assert!(!row.contains(&LIME), "{row:?}");
}

#[test]
fn a_broken_inline_box_lays_its_border_image_out_over_the_joined_box() {
    let pixels = raster(
        "div{width:30px;font:10px/20px Ahem}\
         span{border:5px solid blue;border-image:linear-gradient(lime,lime) 1}",
        "<div><span>XX XX</span></div>",
    );
    let all: Vec<_> = (0..50)
        .flat_map(|y| (0..50).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&pixels, x, y))
        .collect();
    assert!(all.contains(&LIME));
    assert!(!all.contains(&BLUE));
}

#[test]
fn the_body_border_image_replaces_its_border() {
    let pixels = raster(
        "body{width:20px;height:20px;border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 1}",
        "",
    );
    assert_eq!(pixel(&pixels, 2, 15), LIME);
}

#[test]
fn a_split_first_letter_keeps_its_border_image_outset() {
    // The first letter spans the `<b>` boundary, so it paints as two
    // fragments of one joined box; the outset above the box stays visible.
    let pixels = raster(
        "p{margin:20px 10px;font:10px/10px Ahem}\
         p::first-letter{border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 1 / 5px / 3px}",
        "<p><b>\u{201c}</b>A</p>",
    );
    let all: Vec<_> = (0..50)
        .flat_map(|y| (0..50).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&pixels, x, y))
        .collect();
    assert!(!all.contains(&BLUE));
    // Rows above the border box (y < 15) show the outset.
    let outset_rows: Vec<_> = (0..15)
        .flat_map(|y| (0..50).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&pixels, x, y) == LIME)
        .collect();
    assert!(!outset_rows.is_empty());
}

#[test]
fn spaced_gradient_tiles_leave_their_gaps_empty() {
    // The top edge's tiles are 6px wide in a 22px edge: three tiles with
    // 1px gaps between them.
    let pixels = raster(
        "body{margin:0}div{width:22px;height:0;border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 10 space}",
        "<div></div>",
    );
    let row: Vec<_> = (5..27).map(|x| pixel(&pixels, x, 2)).collect();
    assert!(row.contains(&LIME));
    assert!(row.iter().any(|&color| color != LIME));
    assert!(!row.contains(&BLUE));
}

#[test]
fn a_line_break_cut_shows_no_border_image_side() {
    // The first line's fragment ends at x = 25 with no right edge. A length
    // width would put a 5px image strip at 20..25 if the cut kept it.
    let pixels = raster(
        "div{width:30px;font:10px/20px Ahem;color:transparent}\
         span{border:5px solid blue;\
         border-image:linear-gradient(lime,lime) 1 / 5px}",
        "<div><span>XX XX</span></div>",
    );
    assert_eq!(pixel(&pixels, 2, 10), LIME);
    assert_ne!(pixel(&pixels, 22, 10), LIME);
}

#[test]
fn each_fragment_draws_only_the_joined_border_image_tiles_it_shows() {
    // A 49% slice leaves thin edge slices of the box-sized gradient, so the
    // edges of the joined box repeat many small tiles.
    let fills = |words: usize| {
        let body = format!("<div><span>{}</span></div>", vec!["X"; words].join(" "));
        painted(
            "div{width:10px;font:2px/4px Ahem}\
             span{border:1px solid blue;\
             border-image:linear-gradient(lime,lime) 49% / 1px repeat}",
            &body,
        )
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_)))
        .count()
    };
    let (few, many) = (fills(4), fills(16));
    // The joined box, and so its tile count, grows with the lines. Each
    // fragment repeating every tile of it would make the fills grow with
    // the square of the lines instead.
    assert!(many < few * 4, "{few} fills for 4 lines, {many} for 16");
}
