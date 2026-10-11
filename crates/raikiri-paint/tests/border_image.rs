//! Border images replace the border styles where they apply.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources, layout, parse_html_with_resources,
};

/// Render `body` on a 50×50 page and return the RGBA pixels.
fn raster(css: &str, body: &str) -> Vec<u8> {
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
    let mut budget = raikiri_dom::CounterSnapshotBudget::default();
    raikiri_paint::paint_single_page_with_origin(
        &mut scene,
        doc,
        cascade,
        page_box,
        origin,
        &mut budget,
    )
    .unwrap();
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
