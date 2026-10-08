//! Resolved raster pixels and object placement for an independent consumer.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintRect, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_traits::{
    DecodedImage, ImagePixelSource, ImageRasterSize, IntrinsicBox, LayoutConfig, PageDefaults,
    ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::sync::{Arc, Mutex};
use url::Url;

struct Pixels {
    image: Arc<DecodedImage>,
    requests: Mutex<Vec<ImageRasterSize>>,
}

impl Pixels {
    fn new() -> Self {
        Self {
            image: Arc::new(DecodedImage {
                width: 4,
                height: 2,
                rgba: [255, 0, 0, 255].repeat(8),
            }),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl ImagePixelSource for Pixels {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        (!url.path().ends_with("missing.png")).then(|| Arc::clone(&self.image))
    }
    fn get_decoded_at_size(
        &self,
        url: &Url,
        size: ImageRasterSize,
        _limit: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        self.requests.lock().unwrap().push(size);
        self.get_decoded(url)
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

fn lay_out(body: &str, css: &str, pixels: &Pixels) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://images.test/document.html").unwrap())
        .replaced_resolver(pixels)
        .image_pixel_source(pixels);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:10px/10px Ahem}}img{{display:block;width:80px;height:80px}}{css}</style>{body}"
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
    result
}

#[test]
fn raster_images_reuse_pixels_and_the_whole_content_box() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<img src='image.png'>",
        "img{width:100px;border:2px solid black;padding:3px}",
        &pixels,
    );
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.raster_image(&fragment, &pixels).unwrap();
    assert_eq!(image.node, fragment.node());
    assert_eq!(image.url.as_str(), "https://images.test/image.png");
    assert_eq!(image.rect, PaintRect::new(5.0, 5.0, 100.0, 80.0));
    assert_eq!(image.clip.rect, PaintRect::new(5.0, 5.0, 100.0, 80.0));
    assert!(Arc::ptr_eq(&image.pixels, &pixels.image));
    assert_eq!(
        pixels.requests.lock().unwrap().last(),
        Some(&ImageRasterSize {
            width: 100.0,
            height: 80.0
        })
    );
}

#[test]
fn raster_object_fit_uses_the_intrinsic_ratio_and_object_position() {
    for (css, expected) in [
        ("object-fit:fill", PaintRect::new(0.0, 0.0, 80.0, 80.0)),
        ("object-fit:contain", PaintRect::new(0.0, 20.0, 80.0, 40.0)),
        ("object-fit:cover", PaintRect::new(-40.0, 0.0, 160.0, 80.0)),
        ("object-fit:none", PaintRect::new(38.0, 39.0, 4.0, 2.0)),
        (
            "object-fit:scale-down",
            PaintRect::new(38.0, 39.0, 4.0, 2.0),
        ),
        (
            "width:2px;height:2px;object-fit:scale-down",
            PaintRect::new(0.0, 0.5, 2.0, 1.0),
        ),
        (
            "object-fit:contain;object-position:right 4px bottom 5px",
            PaintRect::new(-4.0, 35.0, 80.0, 40.0),
        ),
    ] {
        let pixels = Pixels::new();
        let document = lay_out("<img src='image.png'>", &format!("img{{{css}}}"), &pixels);
        let page = document.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
            .unwrap();
        let image = page.raster_image(&fragment, &pixels).unwrap();
        assert_eq!(image.rect, expected, "{css}");
        assert_eq!(image.clip.rect, fragment.content_rect().unwrap());
    }
}

#[test]
fn ordinary_boxes_hidden_images_and_missing_pixels_have_no_raster_payload() {
    let pixels = Pixels::new();
    let document = lay_out(
        "<div style='width:10px;height:10px'></div><img src='image.png' style='visibility:hidden'><img src='missing.png'>",
        "",
        &pixels,
    );
    let page = document.page(0).unwrap();
    assert!(
        page.fragments()
            .all(|fragment| page.raster_image(&fragment, &pixels).is_none())
    );
}

#[test]
fn raster_content_boxes_use_layout_resolved_percentage_padding() {
    let pixels = Pixels::new();
    let document = lay_out("<img src='image.png'>", "img{padding:10%}", &pixels);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.raster_image(&fragment, &pixels).unwrap();
    assert_eq!(image.rect, PaintRect::new(20.0, 20.0, 80.0, 80.0));
    assert_eq!(image.clip.rect, image.rect);
}
