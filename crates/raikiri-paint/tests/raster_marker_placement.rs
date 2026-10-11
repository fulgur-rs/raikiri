//! Raster markers match independently positioned solid-color page contents.

use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    DecodedImage, ImagePixelSource, IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver,
    ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::sync::Arc;

struct Pixels;
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage {
            width: 8,
            height: 4,
            rgba: [255, 0, 0, 255].repeat(32),
        }))
    }
}
impl ReplacedResolver for Pixels {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(4.0, 2.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

fn raster(body: &str) -> Vec<u8> {
    let pixels = Pixels;
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(url::Url::parse("https://images.test/document.html").unwrap())
        .replaced_resolver(&pixels)
        .image_pixel_source(&pixels);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;background:white}}body{{font:20px/20px Ahem}}ul{{padding:0;margin:0}}li{{margin-left:20px;list-style-image:url(marker.png);color:transparent}}</style>{body}"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
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
        &pixels,
        &mut raikiri_dom::CounterSnapshotBudget::default(),
    )
    .unwrap();
    anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |out| out.append_scene(scene, Affine::IDENTITY),
        200,
        200,
    )
}

fn assert_same(actual: Vec<u8>, expected: Vec<u8>) {
    let differences = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differences, 0);
}

#[test]
fn outside_relative_png_and_opacity_match_literal_pixels() {
    assert_same(
        raster("<ul><li style='opacity:.5'>A</li></ul>"),
        raster(
            "<div style='position:absolute;left:8px;top:0;width:8px;height:4px;background:red;opacity:.5'></div>",
        ),
    );
}

#[test]
fn inside_atomic_png_matches_literal_pixels() {
    assert_same(
        raster("<ul><li style='list-style-position:inside'>A</li></ul>"),
        raster(
            "<div style='position:absolute;left:20px;top:12px;width:8px;height:4px;background:red'></div>",
        ),
    );
}

#[test]
fn translated_png_markers_match_literal_pixels() {
    assert_same(
        raster("<ul style='transform:translate(5px,3px)'><li>A</li></ul>"),
        raster(
            "<div style='position:absolute;left:13px;top:3px;width:8px;height:4px;background:red'></div>",
        ),
    );
    assert_same(
        raster(
            "<ul style='transform:translate(5px,3px)'><li style='list-style-position:inside'>A</li></ul>",
        ),
        raster(
            "<div style='position:absolute;left:25px;top:15px;width:8px;height:4px;background:red'></div>",
        ),
    );
}
