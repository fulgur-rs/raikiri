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

/// An 8x4 red image. Without `sized`, the source reports no natural size, so
/// layout gives the marker images no size and paint takes their natural size
/// from the decoded pixels.
struct Pixels {
    sized: bool,
}
impl ImagePixelSource for Pixels {
    fn intrinsic_size(&self, url: &url::Url) -> Option<raikiri_traits::ImageIntrinsicSize> {
        if !self.sized {
            return None;
        }
        let image = self.get_decoded(url)?;
        Some(raikiri_traits::ImageIntrinsicSize::new(
            Some(image.width as f32),
            Some(image.height as f32),
            None,
        ))
    }

    fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage::new(
            8,
            4,
            [255, 0, 0, 255].repeat(32),
        )))
    }
}
impl ReplacedResolver for Pixels {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(4.0, 2.0),
            ResolveDisposition::Ok,
        ))
    }
}

fn raster(body: &str) -> Vec<u8> {
    raster_with(body, "url(marker.png)", true)
}

/// Rasters `body` with `marker` as the list items' `list-style-image`.
fn raster_with(body: &str, marker: &str, sized: bool) -> Vec<u8> {
    let pixels = Pixels { sized };
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
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;background:white}}body{{font:20px/20px Ahem}}ul{{padding:0;margin:0}}li{{margin-left:20px;list-style-image:{marker};color:transparent}}</style>{body}"
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
    raikiri_paint::paint_single_page_with_images(&mut scene, document, cascade, page_box, &pixels)
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
    // Chromium 152: Ahem 20px/20px has a 16px baseline, and the
    // 4px image bottom meets it, so its top is 12px. The outside
    // image ends 7px before the item's border box.
    assert_same(
        raster("<ul><li style='opacity:.5'>A</li></ul>"),
        raster(
            "<div style='position:absolute;left:5px;top:12px;width:8px;height:4px;background:red;opacity:.5'></div>",
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
            "<div style='position:absolute;left:10px;top:15px;width:8px;height:4px;background:red'></div>",
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

fn red_pixels(raster: &[u8]) -> usize {
    raster
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50)
        .count()
}

#[test]
fn an_unsized_image_set_marker_paints_at_its_natural_size() {
    // The 8x4 image written for 2x has a 4x2 natural size.
    let body = "<ul><li>A</li></ul>";
    assert_eq!(red_pixels(&raster_with(body, "url(marker.png)", false)), 32);
    assert_eq!(
        red_pixels(&raster_with(body, "image-set(url(marker.png) 2x)", false)),
        8
    );
}
