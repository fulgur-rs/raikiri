//! Native generated decorations match independently placed literal rectangles.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource, LayoutConfig, PageDefaults};
use std::sync::Arc;

struct NoPixels;
impl ImagePixelSource for NoPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}body{{margin:0;background:white;font:20px/20px Ahem;color:transparent}}{css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    assert_eq!(result.page_count(), 1);
    let page = result.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page_with_images(
        &mut scene,
        document,
        cascade,
        page_box,
        &NoPixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        100,
        100,
    )
}

fn compare(body: &str, css: &str, after_x: u32, after_y: u32) {
    let actual = raster(body, css);
    let expected = raster(
        &format!(
            "<div style='position:absolute;left:0;top:0;width:20px;height:20px;background:red'></div><div style='position:absolute;left:{after_x}px;top:{after_y}px;width:20px;height:20px;background:blue'></div>"
        ),
        "",
    );
    assert_eq!(
        actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count(),
        0
    );
}

#[test]
fn inline_generated_backgrounds_match_literal_rectangles() {
    compare(
        "<div>A</div>",
        "div::before{content:'B';background:red}div::after{content:'C';background:blue}",
        40,
        0,
    );
}

#[test]
fn block_generated_backgrounds_match_literal_rectangles() {
    compare(
        "<div>A</div>",
        "div::before{display:block;content:'B';background:red}div::after{display:block;content:'C';background:blue}",
        0,
        40,
    );
}
