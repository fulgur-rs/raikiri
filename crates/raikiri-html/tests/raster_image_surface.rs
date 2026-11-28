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
            image: Arc::new(DecodedImage::new(4, 2, [255, 0, 0, 255].repeat(8))),
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
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(4.0, 2.0),
            ResolveDisposition::Ok,
        ))
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

#[test]
fn malformed_and_zero_sized_raster_pixels_are_omitted() {
    for (width, height, rgba) in [
        (0, 1, vec![]),
        (1, 0, vec![]),
        (4, 2, vec![255; 31]),
        (4, 2, vec![255; 33]),
        (u32::MAX, u32::MAX, vec![]),
    ] {
        let layout_pixels = Pixels::new();
        let document = lay_out("<img src='image.png'>", "", &layout_pixels);
        let page = document.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
            .unwrap();
        let malformed = Pixels {
            image: Arc::new(DecodedImage::new(width, height, rgba)),
            requests: Mutex::new(Vec::new()),
        };
        assert!(page.raster_image(&fragment, &malformed).is_none());
    }
}

struct SizedPixels {
    natural: raikiri_traits::ImageIntrinsicSize,
    pixels: Option<Arc<DecodedImage>>,
}

impl ImagePixelSource for SizedPixels {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        self.pixels.clone()
    }
    fn intrinsic_size(&self, _url: &Url) -> Option<raikiri_traits::ImageIntrinsicSize> {
        Some(self.natural)
    }
}

#[test]
fn empty_objects_overflowing_placements_and_denied_pixels_are_omitted() {
    let pixels = Pixels::new();
    let ordinary = raikiri_traits::ImageIntrinsicSize::new(Some(4.0), Some(2.0), Some(2.0));
    for (css, natural, decoded) in [
        ("width:0px", ordinary, Some(Arc::clone(&pixels.image))),
        (
            "object-fit:cover",
            raikiri_traits::ImageIntrinsicSize::new(Some(f32::MAX), Some(f32::MIN_POSITIVE), None),
            Some(Arc::clone(&pixels.image)),
        ),
        ("", ordinary, None),
    ] {
        let document = lay_out("<img src='image.png'>", &format!("img{{{css}}}"), &pixels);
        let page = document.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
            .unwrap();
        let source = SizedPixels {
            natural,
            pixels: decoded,
        };
        assert!(page.raster_image(&fragment, &source).is_none(), "{css}");
    }
}

/// The vertical extent `(top, bottom)` of the clip of the image on each page.
fn clipped_extents(css: &str) -> Vec<(f32, f32)> {
    let pixels = Pixels::new();
    let document = lay_out("<div><img src='image.png'></div><p>after</p>", css, &pixels);
    document
        .pages()
        .filter_map(|page| {
            let fragment = page
                .fragments()
                .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))?;
            let image = page.raster_image(&fragment, &pixels).unwrap();
            let clip = image.clip.rect;
            Some((clip.y, clip.y + clip.height))
        })
        .collect()
}

#[test]
fn an_image_cut_by_the_page_end_is_clipped_to_the_page_area_on_each_page() {
    // Chromium prints the 900px image on a 800px page with 60px margins
    // from y=60 to 740 on the first page and from 60 to 280 on the second;
    // nothing is drawn into the page margins.
    assert_eq!(
        clipped_extents("@page{size:600px 800px;margin:60px 40px}img{width:300px;height:900px}"),
        [(60.0, 740.0), (60.0, 280.0)]
    );
    // With a border, the content edge starts below it on the first page and
    // ends above it on the last.
    assert_eq!(
        clipped_extents(
            "@page{size:600px 800px;margin:60px 40px}\
             img{width:300px;height:900px;border:10px solid}"
        ),
        [(70.0, 740.0), (60.0, 290.0)]
    );
}

#[test]
fn an_inline_svg_cut_by_the_page_end_is_clipped_to_the_page_area_on_each_page() {
    // Chromium prints the bordered 900px SVG root like the bordered image:
    // its picture from y=70 to 740 on the first page and from 60 to 290 on
    // the second.
    let pixels = Pixels::new();
    let document = lay_out(
        "<div><svg width=300 height=900 style='display:block;border:10px solid'>\
         <rect width=300 height=900 fill=red /></svg></div><p>after</p>",
        "@page{size:600px 800px;margin:60px 40px}",
        &pixels,
    );
    let clips: Vec<_> = document
        .pages()
        .filter_map(|page| {
            let fragment = page
                .fragments()
                .find(|fragment| page.dom().local_name(fragment.node()) == Some("svg"))?;
            let svg = page.inline_svg(&fragment).unwrap().unwrap();
            assert_eq!(svg.viewport.height, 900.0);
            Some((svg.clip.y, svg.clip.y + svg.clip.height))
        })
        .collect();
    assert_eq!(clips, [(70.0, 740.0), (60.0, 290.0)]);
}

#[test]
fn the_piece_of_a_cut_image_holding_only_its_padding_has_nothing_to_draw() {
    // The content of each ends on the page it starts; the next page holds
    // only its bottom padding, so neither the image nor the SVG picture is
    // drawn there. The SVG root does not fit after the image's padding and
    // starts the third page.
    let pixels = Pixels::new();
    let document = lay_out(
        "<img src='image.png' style='width:300px;height:600px;padding-bottom:300px'>\
         <svg width=300 height=600 style='display:block;padding-bottom:300px'>\
         <rect width=300 height=600 fill=red /></svg>",
        "@page{size:600px 800px;margin:60px 40px}",
        &pixels,
    );
    let drawn: Vec<_> = document
        .pages()
        .map(|page| {
            let (mut image, mut svg) = (None, None);
            for fragment in page.fragments() {
                match page.dom().local_name(fragment.node()) {
                    Some("img") => image = Some(page.raster_image(&fragment, &pixels).is_some()),
                    Some("svg") => svg = Some(page.inline_svg(&fragment).unwrap().is_some()),
                    _ => {}
                }
            }
            (image, svg)
        })
        .collect();
    assert_eq!(
        drawn,
        [
            (Some(true), None),
            (Some(false), None),
            (None, Some(true)),
            (None, Some(false))
        ]
    );
}
