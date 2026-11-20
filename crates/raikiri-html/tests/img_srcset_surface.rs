//! The source an `<img>` shows: its `srcset`, its `<picture>`'s `<source>`
//! elements and its `src`, and the natural size the chosen candidate's
//! density gives it.
//!
//! The candidates and sizes match what Chromium shows on a 200x200 screen.
//! raikiri lays out for print, so `media` and `sizes` are evaluated in the
//! print environment of the layout.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, MediaContext, MediaType,
    PaintRect, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{
    DecodedImage, ImageIntrinsicSize, ImagePixelSource, IntrinsicBox, LayoutConfig, PageDefaults,
    ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use std::sync::Arc;
use url::Url;

/// `a.png` is 40x20, `b.png` 80x40 and `c.png` 120x60.
struct Images;

impl Images {
    fn size(url: &Url) -> (u32, u32) {
        match url.path().rsplit('/').next() {
            Some("a.png") => (40, 20),
            Some("b.png") => (80, 40),
            _ => (120, 60),
        }
    }
}

impl ImagePixelSource for Images {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        let (width, height) = Self::size(url);
        Some(Arc::new(DecodedImage::new(
            width,
            height,
            [0, 128, 0, 255].repeat((width * height) as usize),
        )))
    }

    fn intrinsic_size(&self, url: &Url) -> Option<ImageIntrinsicSize> {
        let (width, height) = Self::size(url);
        Some(ImageIntrinsicSize::new(
            Some(width as f32),
            Some(height as f32),
            Some(width as f32 / height as f32),
        ))
    }
}

impl ReplacedResolver for Images {
    fn resolve(&self, request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        let (width, height) = Self::size(request.url());
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(width as f32, height as f32),
            ResolveDisposition::Ok,
        ))
    }
}

fn lay_out_in(body: &str, media_context: MediaContext) -> DocumentLayout {
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
        .replaced_resolver(&Images)
        .image_pixel_source(&Images);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}body{{margin:0}}img{{display:block}}</style>{body}"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let mut config = LayoutConfig::default();
    config.media_context = media_context;
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        config,
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

/// The file name of the image `body`'s `<img>` shows and the size it is
/// drawn at.
fn shown_in(body: &str, media_context: MediaContext) -> (String, f32, f32) {
    let document = lay_out_in(body, media_context);
    let page = document.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|fragment| page.dom().local_name(fragment.node()) == Some("img"))
        .unwrap();
    let image = page.placed_image(&fragment, &Images).unwrap();
    let PaintRect { width, height, .. } = image.rect;
    let name = image.url.path().rsplit('/').next().unwrap().to_owned();
    (name, width, height)
}

fn shown(body: &str) -> (String, f32, f32) {
    shown_in(body, MediaContext::default())
}

fn image(name: &str, width: f32, height: f32) -> (String, f32, f32) {
    (name.to_owned(), width, height)
}

#[test]
fn an_image_written_for_a_density_is_its_size_divided_by_it() {
    assert_eq!(shown("<img srcset='b.png 2x'>"), image("b.png", 40.0, 20.0));
    assert_eq!(
        shown("<img srcset='b.png 0.5x'>"),
        image("b.png", 160.0, 80.0)
    );
    assert_eq!(
        shown("<img srcset='a.png 0.5x, b.png 0.75x'>"),
        image("b.png", 80.0 / 0.75, 40.0 / 0.75)
    );
    assert_eq!(
        shown("<img srcset='b.png 2x, c.png 3x'>"),
        image("b.png", 40.0, 20.0)
    );
    // A stated width keeps the image's ratio.
    assert_eq!(
        shown("<img srcset='b.png 2x' width='100'>"),
        image("b.png", 100.0, 50.0)
    );
}

#[test]
fn src_is_the_1x_candidate() {
    assert_eq!(
        shown("<img src='a.png' srcset='b.png 2x'>"),
        image("a.png", 40.0, 20.0)
    );
    assert_eq!(
        shown("<img src='a.png' srcset='b.png 1x'>"),
        image("b.png", 80.0, 40.0)
    );
    assert_eq!(
        shown("<img src='a.png' srcset='b.png 2x 3x'>"),
        image("a.png", 40.0, 20.0)
    );
}

#[test]
fn a_width_descriptor_is_divided_by_the_source_size() {
    assert_eq!(
        shown("<img srcset='c.png 120w' sizes='60px'>"),
        image("c.png", 60.0, 30.0)
    );
    assert_eq!(
        shown("<img srcset='a.png 40w, c.png 120w' sizes='60px'>"),
        image("c.png", 60.0, 30.0)
    );
    // Without sizes the image is as wide as the viewport of the media
    // environment.
    let context = MediaContext::with_viewport(MediaType::Print, 240, 200);
    assert_eq!(
        shown_in("<img srcset='c.png 120w'>", context),
        image("c.png", 240.0, 120.0)
    );
    assert_eq!(
        shown_in(
            "<img srcset='c.png 120w' sizes='(min-width: 200px) 60px, 30px'>",
            context
        ),
        image("c.png", 60.0, 30.0)
    );
}

#[test]
fn a_source_of_the_picture_gives_the_candidates() {
    assert_eq!(
        shown("<picture><source srcset='c.png 2x'><img src='a.png'></picture>"),
        image("c.png", 60.0, 30.0)
    );
    assert_eq!(
        shown("<picture><source srcset='c.png 2x' type='image/foo'><img src='a.png'></picture>"),
        image("a.png", 40.0, 20.0)
    );
    assert_eq!(
        shown(
            "<picture><source srcset='c.png 2x' media='(max-width: 1px)'><source srcset='b.png 2x'><img src='a.png'></picture>"
        ),
        image("b.png", 40.0, 20.0)
    );
    // The layout's media is print.
    assert_eq!(
        shown("<picture><source srcset='c.png 2x' media='print'><img src='a.png'></picture>"),
        image("c.png", 60.0, 30.0)
    );
    assert_eq!(
        shown("<picture><source srcset='c.png 2x' media='screen'><img src='a.png'></picture>"),
        image("a.png", 40.0, 20.0)
    );
}
