//! Raster objects match independently positioned solid-color page contents.

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
            width: 4,
            height: 2,
            rgba: [255, 0, 0, 255].repeat(8),
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
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;background:white}}img{{display:block;width:80px;height:80px}}</style>{body}"
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
fn relative_image_sources_and_contain_geometry_match_literal_bands() {
    assert_same(
        raster("<img src='image.png' style='object-fit:contain'>"),
        raster(
            "<div style='height:20px'></div><div style='width:80px;height:40px;background:red'></div>",
        ),
    );
}

#[test]
fn image_content_clips_and_host_opacity_match_literal_boxes() {
    assert_same(
        raster(
            "<img src='image.png' style='border:2px solid black;padding:3px;opacity:0.5;object-fit:cover'>",
        ),
        raster(
            "<div style='width:80px;height:80px;border:2px solid black;padding:3px;opacity:0.5'><div style='width:80px;height:80px;background:red'></div></div>",
        ),
    );
}

#[test]
fn percentage_image_padding_uses_the_layout_containing_width() {
    assert_same(
        raster("<img src='image.png' style='padding:10%'>"),
        raster(
            "<div style='width:80px;height:80px;padding:20px'><div style='width:80px;height:80px;background:red'></div></div>",
        ),
    );
}
