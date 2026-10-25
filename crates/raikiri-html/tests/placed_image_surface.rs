//! Images placed on a page as pixels or as scenes: `placed_image`,
//! `placed_marker`, `placed_generated_image` and `image_content`.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, GeneratedKind, ImageContent, LayoutOptions,
    LayoutStatus, PaintEvent, PaintRect, RenderResources, SvgRendering, layout,
    parse_html_with_resources,
};
use raikiri_traits::vector::VectorImage;
use raikiri_traits::{
    DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, ImageVectorRequest,
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::sync::{Arc, Mutex};
use url::Url;

/// Pictures that end in `.svg` have a scene (when the double is asked to give
/// one) and pixels; every other picture has pixels only.
struct Source {
    pixels: Arc<DecodedImage>,
    scenes: bool,
    pixel_requests: Mutex<Vec<ImageRasterSize>>,
    scene_requests: Mutex<Vec<ImageRasterSize>>,
}

impl Source {
    fn new(scenes: bool) -> Self {
        Self {
            pixels: Arc::new(DecodedImage::new(4, 2, [255, 0, 0, 255].repeat(8))),
            scenes,
            pixel_requests: Mutex::new(Vec::new()),
            scene_requests: Mutex::new(Vec::new()),
        }
    }

    fn pixel_requests(&self) -> Vec<ImageRasterSize> {
        self.pixel_requests.lock().unwrap().clone()
    }

    fn scene_requests(&self) -> Vec<ImageRasterSize> {
        self.scene_requests.lock().unwrap().clone()
    }
}

impl ImagePixelSource for Source {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::clone(&self.pixels))
    }

    fn intrinsic_size(&self, _url: &Url) -> Option<ImageIntrinsicSize> {
        Some(ImageIntrinsicSize::new(Some(4.0), Some(2.0), Some(2.0)))
    }

    fn get_decoded_at_size(
        &self,
        _url: &Url,
        size: ImageRasterSize,
        _limit: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        self.pixel_requests.lock().unwrap().push(size);
        Some(Arc::clone(&self.pixels))
    }

    fn get_vector(&self, url: &Url, request: &ImageVectorRequest) -> Option<Arc<VectorImage>> {
        self.scene_requests.lock().unwrap().push(request.size);
        (self.scenes && url.path().ends_with(".svg"))
            .then(|| Arc::new(VectorImage::new((request.size.width, request.size.height))))
    }
}

impl ReplacedResolver for Source {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(4.0, 2.0),
            ResolveDisposition::Ok,
        ))
    }
}

fn lay_out(
    body: &str,
    css: &str,
    source: &Source,
    options: impl for<'a> FnOnce(LayoutOptions<'a, 'a>) -> LayoutOptions<'a, 'a>,
) -> DocumentLayout {
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
        .replaced_resolver(source)
        .image_pixel_source(source);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0;font:10px/10px Ahem}}img{{display:block;width:80px;height:40px}}{css}</style>{body}"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        options(LayoutOptions::new().resources(&resources)),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

fn size(width: f32, height: f32) -> ImageRasterSize {
    ImageRasterSize { width, height }
}

fn scene_size(content: &ImageContent) -> Option<(f32, f32)> {
    match content {
        ImageContent::Vector(scene) => Some(scene.size),
        _ => None,
    }
}

#[test]
fn an_svg_image_is_a_scene_of_its_object_box() {
    let source = Source::new(true);
    let document = lay_out("<img src='a.svg'>", "", &source, |options| options);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &source).unwrap();
    assert_eq!(image.node, fragment.node());
    assert_eq!(image.url.as_str(), "https://images.test/a.svg");
    assert_eq!(image.rect, PaintRect::new(0.0, 0.0, 80.0, 40.0));
    assert_eq!(image.clip.rect, image.rect);
    assert_eq!(image.clip_owner, Some(fragment.node()));
    assert_eq!(scene_size(&image.content), Some((80.0, 40.0)));
    assert_eq!(source.scene_requests(), vec![size(80.0, 40.0)]);
    assert!(source.pixel_requests().is_empty(), "no pixels were needed");
}

#[test]
fn the_scene_follows_the_object_fit_box_not_the_element_box() {
    let source = Source::new(true);
    let document = lay_out(
        "<img src='a.svg'>",
        "img{height:80px;object-fit:contain}",
        &source,
        |options| options,
    );
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &source).unwrap();
    // The 4:2 picture is as wide as the 80px element and half as tall.
    assert_eq!(image.rect, PaintRect::new(0.0, 20.0, 80.0, 40.0));
    assert_eq!(image.clip.rect, PaintRect::new(0.0, 0.0, 80.0, 80.0));
    assert_eq!(scene_size(&image.content), Some((80.0, 40.0)));
}

#[test]
fn a_raster_image_is_pixels() {
    let source = Source::new(true);
    let document = lay_out("<img src='a.png'>", "", &source, |options| options);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &source).unwrap();
    let ImageContent::Raster(pixels) = &image.content else {
        panic!("expected pixels, got {:?}", image.content)
    };
    assert!(Arc::ptr_eq(pixels, &source.pixels));
}

#[test]
fn an_image_the_source_has_no_scene_for_is_pixels() {
    let source = Source::new(false);
    let document = lay_out("<img src='a.svg'>", "", &source, |options| options);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &source).unwrap();
    assert!(matches!(image.content, ImageContent::Raster(_)));
    assert_eq!(source.scene_requests(), vec![size(80.0, 40.0)]);
    assert_eq!(source.pixel_requests(), vec![size(80.0, 40.0)]);
}

#[test]
fn the_element_decides_between_a_scene_and_pixels() {
    let source = Source::new(true);
    let document = lay_out(
        "<img id=plain src='a.svg'>\
         <div style='--raikiri-svg-rendering: raster'><img id=raster src='a.svg'>\
         <img id=vector src='a.svg' style='--raikiri-svg-rendering: vector'></div>",
        "",
        &source,
        |options| options,
    );
    let page = document.page(0).unwrap();
    let content = |id: &str| {
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().attr(fragment.node(), "id") == Some(id))
            .unwrap();
        page.placed_image(&fragment, &source).unwrap().content
    };
    assert!(matches!(content("plain"), ImageContent::Vector(_)));
    assert!(matches!(content("raster"), ImageContent::Raster(_)));
    assert!(matches!(content("vector"), ImageContent::Vector(_)));
}

#[test]
fn the_layout_default_decides_for_an_element_that_says_nothing() {
    let source = Source::new(true);
    let document = lay_out(
        "<img id=plain src='a.svg'><img id=vector src='a.svg' style='--raikiri-svg-rendering: vector'>",
        "",
        &source,
        |options| options.svg_rendering(SvgRendering::Raster),
    );
    let page = document.page(0).unwrap();
    let content = |id: &str| {
        let fragment = page
            .fragments()
            .find(|fragment| page.dom().attr(fragment.node(), "id") == Some(id))
            .unwrap();
        page.placed_image(&fragment, &source).unwrap().content
    };
    assert!(matches!(content("plain"), ImageContent::Raster(_)));
    assert!(matches!(content("vector"), ImageContent::Vector(_)));
}

#[test]
fn pixels_are_asked_for_at_the_raster_scale() {
    let source = Source::new(true);
    let document = lay_out(
        "<img src='a.svg' style='--raikiri-svg-rendering: raster'>",
        "",
        &source,
        |options| options.svg_raster_scale(2.5),
    );
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &source).unwrap();
    assert!(matches!(image.content, ImageContent::Raster(_)));
    // The box is still 80 x 40; only the pixels follow the scale.
    assert_eq!(image.rect, PaintRect::new(0.0, 0.0, 80.0, 40.0));
    assert_eq!(source.pixel_requests(), vec![size(200.0, 100.0)]);
}

#[test]
fn the_pixel_api_stays_pixels_and_follows_the_raster_scale() {
    let source = Source::new(true);
    let document = lay_out("<img src='a.svg'>", "", &source, |options| {
        options.svg_raster_scale(2.0)
    });
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.raster_image(&fragment, &source).unwrap();
    assert!(Arc::ptr_eq(&image.pixels, &source.pixels));
    assert!(source.scene_requests().is_empty(), "no scene was asked for");
    assert_eq!(source.pixel_requests(), vec![size(160.0, 80.0)]);
}

#[test]
fn an_image_the_consumer_places_gets_its_content_by_the_owner() {
    let source = Source::new(true);
    let document = lay_out(
        "<div id=plain></div><div id=raster style='--raikiri-svg-rendering: raster'></div>",
        "",
        &source,
        |options| options.svg_raster_scale(3.0),
    );
    let page = document.page(0).unwrap();
    let owner = |id: &str| {
        page.fragments()
            .map(|fragment| fragment.node())
            .find(|&node| page.dom().attr(node, "id") == Some(id))
            .unwrap()
    };
    let url = Url::parse("https://images.test/tile.svg").unwrap();
    let vector = page
        .image_content(owner("plain"), &url, size(10.0, 5.0), &source)
        .unwrap();
    assert_eq!(scene_size(&vector), Some((10.0, 5.0)));
    let raster = page
        .image_content(owner("raster"), &url, size(10.0, 5.0), &source)
        .unwrap();
    assert!(matches!(raster, ImageContent::Raster(_)));
    assert_eq!(source.pixel_requests(), vec![size(30.0, 15.0)]);
}

#[test]
fn a_marker_image_is_a_scene_in_its_marker_box() {
    let source = Source::new(true);
    let document = lay_out(
        "<ul><li>A</li></ul>",
        "li{list-style-image:url(marker.svg)}",
        &source,
        |options| options,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let owner = events
        .iter()
        .find_map(|event| match event {
            PaintEvent::MarkerImage(owner) => Some(*owner),
            _ => None,
        })
        .expect("a marker image");
    let placed = page.placed_marker(owner, &source).unwrap();
    let pixels = page.raster_marker(owner, &source).unwrap();
    assert_eq!(placed.rect, pixels.rect);
    assert_eq!(placed.clip_owner, pixels.clip_owner);
    assert_eq!(
        scene_size(&placed.content),
        Some((placed.rect.width, placed.rect.height))
    );
}

#[test]
fn a_content_image_is_a_scene_in_its_object_box() {
    let source = Source::new(true);
    let document = lay_out(
        "<p>A</p>",
        "p::before{content:url(icon.svg)}",
        &source,
        |options| options,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let image = page
        .paint_order_for_text_runs(&runs)
        .iter()
        .find_map(|event| match event {
            PaintEvent::GeneratedImage(image) => Some(*image),
            _ => None,
        })
        .expect("a generated image");
    let placed = page.placed_generated_image(image, &source).unwrap();
    let pixels = page.raster_generated_image(image, &source).unwrap();
    assert_eq!(placed.rect, pixels.rect);
    assert_eq!(placed.clip.rect, pixels.clip.rect);
    assert_eq!(placed.clip_owner, pixels.clip_owner);
    assert_eq!(
        scene_size(&placed.content),
        Some((placed.rect.width, placed.rect.height))
    );
}

#[test]
fn a_content_image_follows_the_rendering_its_pseudo_element_names() {
    let source = Source::new(true);
    let document = lay_out(
        "<p id=a>A</p><p id=b>B</p><p id=c>C</p>",
        "#a::before{content:url(a.svg);--raikiri-svg-rendering:raster}\
         #b{--raikiri-svg-rendering:raster}#b::after{content:url(b.svg);--raikiri-svg-rendering:vector}\
         #c{--raikiri-svg-rendering:raster}#c::before{content:url(c.svg)}",
        &source,
        |options| options,
    );
    let page = document.page(0).unwrap();
    let runs = page.text_runs();
    let events = page.paint_order_for_text_runs(&runs);
    let content = |id: &str, kind: GeneratedKind| {
        let image = events
            .iter()
            .find_map(|event| match event {
                PaintEvent::GeneratedImage(image)
                    if image.kind == kind && page.dom().attr(image.owner, "id") == Some(id) =>
                {
                    Some(*image)
                }
                _ => None,
            })
            .expect("a generated image");
        page.placed_generated_image(image, &source).unwrap().content
    };
    // The pseudo-element says raster where its element says nothing, ...
    assert!(matches!(
        content("a", GeneratedKind::Before),
        ImageContent::Raster(_)
    ));
    // ... says vector where its element says raster, ...
    assert!(matches!(
        content("b", GeneratedKind::After),
        ImageContent::Vector(_)
    ));
    // ... and inherits from its element where it says nothing.
    assert!(matches!(
        content("c", GeneratedKind::Before),
        ImageContent::Raster(_)
    ));
}

#[test]
fn an_image_without_a_source_answer_is_not_placed() {
    struct Nothing;
    impl ImagePixelSource for Nothing {
        fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
            None
        }
    }
    let source = Source::new(true);
    let document = lay_out("<img src='a.svg'>", "", &source, |options| options);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    assert!(page.placed_image(&fragment, &Nothing).is_none());
    let url = Url::parse("https://images.test/a.svg").unwrap();
    assert!(
        page.image_content(fragment.node(), &url, size(8.0, 4.0), &Nothing)
            .is_none()
    );

    // A source that knows the size of the picture but has neither a scene nor
    // pixels for it places nothing either.
    struct SizedOnly;
    impl ImagePixelSource for SizedOnly {
        fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
            None
        }
        fn intrinsic_size(&self, _url: &Url) -> Option<ImageIntrinsicSize> {
            Some(ImageIntrinsicSize::new(Some(4.0), Some(2.0), Some(2.0)))
        }
    }
    assert!(page.placed_image(&fragment, &SizedOnly).is_none());
}
