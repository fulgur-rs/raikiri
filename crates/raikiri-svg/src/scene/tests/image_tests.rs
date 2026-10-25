//! Pictures in scenes: taken from the caller's source, placed by the group
//! the parser wraps them in, and bounded.

use super::*;
use crate::{SvgImageAccess, SvgImageSource};
use raikiri_traits::DecodedImage;
use raikiri_traits::vector::RasterImage;
use std::sync::{Arc, Mutex};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Two pixels by two: red and green over blue and white.
fn quad() -> Arc<DecodedImage> {
    Arc::new(DecodedImage::new(2, 2, [RED, GREEN, BLUE, WHITE].concat()))
}

/// Answers from a table of URLs, answers every `data:` URL with one picture,
/// and writes down what it was asked.
struct Pictures {
    urls: Vec<(&'static str, Option<Arc<DecodedImage>>)>,
    data: Option<Arc<DecodedImage>>,
    asked: Mutex<Vec<String>>,
}

impl Pictures {
    fn new() -> Self {
        Self {
            urls: Vec::new(),
            data: None,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn url(mut self, href: &'static str, image: Option<Arc<DecodedImage>>) -> Self {
        self.urls.push((href, image));
        self
    }

    fn data(mut self, image: Arc<DecodedImage>) -> Self {
        self.data = Some(image);
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl SvgImageSource for Pictures {
    fn url(&self, href: &str) -> Option<Arc<DecodedImage>> {
        self.asked.lock().unwrap().push(format!("url {href}"));
        self.urls
            .iter()
            .find(|(url, _)| *url == href)
            .and_then(|(_, image)| image.clone())
    }

    fn data(&self, mime: &str, bytes: &[u8]) -> Option<Arc<DecodedImage>> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("data {mime} {bytes:?}"));
        self.data.clone()
    }
}

fn parse(access: SvgImageAccess, body: &str) -> SvgDocument {
    SvgDocument::parse_with_images(
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg"
                     xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="20">{body}</svg>"##
        )
        .as_bytes(),
        access,
    )
    .unwrap()
}

fn build(
    document: &SvgDocument,
    pictures: &Pictures,
    limits: SceneLimits,
) -> Result<VectorImage, SvgSceneError> {
    document.build_scene(
        VIEWPORT,
        style(1.0),
        None,
        limits,
        SvgResources::new().with_images(pictures),
    )
}

fn scene(access: SvgImageAccess, body: &str, pictures: &Pictures) -> VectorImage {
    build(&parse(access, body), pictures, SceneLimits::default()).unwrap()
}

fn rasters(group: &Group) -> Vec<&RasterImage> {
    let mut found = Vec::new();
    for child in &group.children {
        match child {
            Node::Group(group) => found.extend(rasters(group)),
            Node::Image(image) => found.push(image),
            _ => {}
        }
    }
    found
}

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * 40 + x) * 4;
    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
}

/// Draws `picture` into `target` as the test renderer draws a scene.
pub(super) fn draw_image(
    target: &mut tiny_skia::Pixmap,
    picture: &RasterImage,
    transform: tiny_skia::Transform,
) {
    let pixels = &picture.image;
    // The pictures here are opaque, so straight and premultiplied agree.
    let source = tiny_skia::Pixmap::from_vec(
        pixels.rgba.clone(),
        tiny_skia::IntSize::from_wh(pixels.width, pixels.height).unwrap(),
    )
    .unwrap();
    target.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &tiny_skia::PixmapPaint {
            quality: if picture.smooth {
                tiny_skia::FilterQuality::Bilinear
            } else {
                tiny_skia::FilterQuality::Nearest
            },
            ..tiny_skia::PixmapPaint::default()
        },
        transform,
        None,
    );
}

// ── Drawing ─────────────────────────────────────────────────

#[test]
fn a_picture_is_drawn_into_the_box_of_its_element() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="quad.png" x="10" y="5" width="20" height="10"
                   preserveAspectRatio="none" style="image-rendering:pixelated"/>"##,
        &pictures,
    );
    let drawn = render(&image);
    assert_eq!(pixel(&drawn, 14, 7), RED);
    assert_eq!(pixel(&drawn, 24, 7), GREEN);
    assert_eq!(pixel(&drawn, 14, 12), BLUE);
    assert_eq!(pixel(&drawn, 24, 12), WHITE);
    assert_eq!(pixel(&drawn, 5, 7), [0, 0, 0, 0]);
    assert_eq!(pixel(&drawn, 34, 12), [0, 0, 0, 0]);
    assert_eq!(pictures.asked(), ["url quad.png"]);
}

#[test]
fn a_pixelated_picture_draws_like_the_rectangles_of_its_pixels() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="quad.png" x="10" y="5" width="20" height="10"
                   preserveAspectRatio="none" style="image-rendering:pixelated"/>"##,
        &pictures,
    );
    let rectangles = document(
        r##"<rect x="10" y="5" width="10" height="5" fill="#f00"/>
            <rect x="20" y="5" width="10" height="5" fill="#0f0"/>
            <rect x="10" y="10" width="10" height="5" fill="#00f"/>
            <rect x="20" y="10" width="10" height="5" fill="#fff"/>"##,
    );
    let raster = rectangles
        .rasterize_at_css_pixel_scale(VIEWPORT, style(1.0), None)
        .unwrap();
    assert_close(&render(&image), &raster.rgba, 1);
}

#[test]
fn meet_centers_the_picture_in_its_box() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    // The square picture fits the height of a box twice as wide as it is high.
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="quad.png" x="0" y="0" width="40" height="20"
                   style="image-rendering:pixelated"/>"##,
        &pictures,
    );
    let drawn = render(&image);
    assert_eq!(pixel(&drawn, 5, 5), [0, 0, 0, 0]);
    assert_eq!(pixel(&drawn, 15, 5), RED);
    assert_eq!(pixel(&drawn, 25, 5), GREEN);
    assert_eq!(pixel(&drawn, 15, 15), BLUE);
    assert_eq!(pixel(&drawn, 25, 15), WHITE);
    assert_eq!(pixel(&drawn, 35, 5), [0, 0, 0, 0]);
}

#[test]
fn slice_fills_the_box_and_clips_what_overflows_it() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="quad.png" x="10" y="5" width="20" height="10"
                   preserveAspectRatio="xMidYMid slice" style="image-rendering:pixelated"/>"##,
        &pictures,
    );
    let drawn = render(&image);
    // The picture is 20 by 20 around the box, and the box shows its middle.
    assert_eq!(pixel(&drawn, 15, 3), [0, 0, 0, 0]);
    assert_eq!(pixel(&drawn, 15, 7), RED);
    assert_eq!(pixel(&drawn, 25, 7), GREEN);
    assert_eq!(pixel(&drawn, 15, 12), BLUE);
    assert_eq!(pixel(&drawn, 25, 12), WHITE);
    assert_eq!(pixel(&drawn, 15, 17), [0, 0, 0, 0]);
    assert!(groups(&image.root).iter().any(|group| group.clip.is_some()));
}

#[test]
fn the_rendering_property_decides_whether_scaling_smooths() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    for (rendering, smooth) in [
        ("auto", true),
        ("optimizeQuality", true),
        ("smooth", true),
        ("optimizeSpeed", false),
        ("crisp-edges", false),
        ("pixelated", false),
    ] {
        let image = scene(
            SvgImageAccess::Any,
            &format!(
                r##"<image href="quad.png" width="20" height="10" style="image-rendering:{rendering}"/>"##
            ),
            &pictures,
        );
        let found = rasters(&image.root);
        assert_eq!(found.len(), 1, "{rendering}");
        assert_eq!(found[0].smooth, smooth, "{rendering}");
    }
}

#[test]
fn a_picture_follows_the_opacity_and_the_transform_around_it() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<g opacity="0.5" transform="translate(4 2)">
                <image href="quad.png" width="20" height="10" preserveAspectRatio="none"
                       style="image-rendering:pixelated"/>
            </g>"##,
        &pictures,
    );
    assert!(groups(&image.root).iter().any(|group| group.opacity == 0.5));
    let drawn = render(&image);
    assert_eq!(pixel(&drawn, 8, 4), [255, 0, 0, 128]);
    assert_eq!(pixel(&drawn, 3, 4), [0, 0, 0, 0]);
}

#[test]
fn a_hidden_picture_is_not_drawn() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="quad.png" width="20" height="10" visibility="hidden"/>"##,
        &pictures,
    );
    assert!(rasters(&image.root).is_empty());
    assert_eq!(image, VectorImage::new((40.0, 20.0)));
}

// ── What reaches the source ─────────────────────────────────

#[test]
fn a_url_reaches_the_source_as_the_document_wrote_it() {
    let pictures = Pictures::new()
        .url("a.png", Some(quad()))
        .url("dir/b.png?x=1", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="  a.png " width="10" height="10"/>
            <image xlink:href="dir/b.png?x=1" x="10" width="10" height="10"/>"##,
        &pictures,
    );
    assert_eq!(rasters(&image.root).len(), 2);
    assert_eq!(pictures.asked(), ["url a.png", "url dir/b.png?x=1"]);
}

#[test]
fn fragments_and_empty_references_name_nothing_and_are_not_asked() {
    let pictures = Pictures::new();
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="#elsewhere" width="10" height="10"/>
            <image href="" width="10" height="10"/>
            <image width="10" height="10"/>"##,
        &pictures,
    );
    assert!(rasters(&image.root).is_empty());
    assert!(pictures.asked().is_empty());
}

#[test]
fn a_data_url_reaches_the_source_decoded() {
    let pictures = Pictures::new().data(quad());
    let image = scene(
        SvgImageAccess::Data,
        r##"<image href="data:image/png;base64,AQID" width="20" height="10"/>
            <image href="DATA:image/jpeg,%04%05" x="20" width="20" height="10"/>"##,
        &pictures,
    );
    assert_eq!(rasters(&image.root).len(), 2);
    assert_eq!(
        pictures.asked(),
        ["data image/png [1, 2, 3]", "data image/jpeg [4, 5]"]
    );
}

#[test]
fn a_nested_svg_document_never_reaches_the_source() {
    let pictures = Pictures::new().data(quad()).url("nested.svg", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<image width="10" height="10"
                   href="data:image/svg+xml;utf8,&lt;svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/&gt;"/>"##,
        &pictures,
    );
    assert!(rasters(&image.root).is_empty());
    assert!(pictures.asked().is_empty());
}

#[test]
fn a_repeated_reference_is_asked_once_and_shares_its_pixels() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let image = scene(
        SvgImageAccess::Any,
        r##"<defs><image id="i" href="quad.png" width="10" height="10"/></defs>
            <use href="#i"/><use href="#i" x="10"/><use href="#i" x="20"/>
            <image href="quad.png" y="10" width="10" height="10"/>"##,
        &pictures,
    );
    let found = rasters(&image.root);
    assert_eq!(found.len(), 4);
    assert!(
        found
            .iter()
            .all(|picture| Arc::ptr_eq(&picture.image, &found[0].image))
    );
    assert_eq!(pictures.asked(), ["url quad.png"]);
}

#[test]
fn equal_data_urls_are_asked_once_and_different_ones_apart() {
    let pictures = Pictures::new().data(quad());
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="first.png" width="10" height="10"/>
            <image href="data:image/png;base64,AQID" width="10" height="10"/>
            <image href="data:image/png;base64,AQID" x="10" width="10" height="10"/>
            <image href="data:image/png;base64,AQIE" x="20" width="10" height="10"/>
            <image href="data:image/gif;base64,AQID" x="30" width="10" height="10"/>"##,
        &pictures,
    );
    // The first picture is not one the source has.
    assert_eq!(rasters(&image.root).len(), 4);
    assert_eq!(
        pictures.asked(),
        [
            "url first.png",
            "data image/png [1, 2, 3]",
            "data image/png [1, 2, 4]",
            "data image/gif [1, 2, 3]",
        ]
    );
}

#[test]
fn pictures_the_source_does_not_give_are_left_out() {
    let broken = Pictures::new().url("gone.png", None);
    let image = scene(
        SvgImageAccess::Any,
        r##"<rect width="5" height="5" fill="#123"/>
            <image href="gone.png" width="10" height="10"/>"##,
        &broken,
    );
    assert!(rasters(&image.root).is_empty());
    assert_eq!(paths(&image.root).len(), 1);

    // A picture whose pixels do not fit its size, or that has none, is a
    // broken picture too.
    let malformed = Pictures::new()
        .url(
            "short.png",
            Some(Arc::new(DecodedImage::new(2, 2, vec![0; 12]))),
        )
        .url(
            "empty.png",
            Some(Arc::new(DecodedImage::new(0, 0, Vec::new()))),
        );
    let image = scene(
        SvgImageAccess::Any,
        r##"<image href="short.png" width="10" height="10"/>
            <image href="empty.png" width="10" height="10"/>"##,
        &malformed,
    );
    assert!(rasters(&image.root).is_empty());
}

#[test]
fn a_document_without_a_picture_source_has_no_pictures() {
    let document = parse(
        SvgImageAccess::Any,
        r##"<rect width="5" height="5"/><image href="quad.png" width="10" height="10"/>"##,
    );
    let image = document.scene(VIEWPORT, style(1.0)).unwrap();
    assert!(rasters(&image.root).is_empty());
    assert_eq!(paths(&image.root).len(), 1);
    // Fonts alone do not give it any either.
    let with_fonts = document
        .scene_with_resources(
            VIEWPORT,
            style(1.0),
            None,
            SvgResources::new().with_fonts(&text_tests::Fonts::none()),
        )
        .unwrap();
    assert_eq!(with_fonts, image);
}

#[test]
fn pictures_and_text_come_from_their_own_sources() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let fonts = text_tests::Fonts::ahem();
    let document = parse(
        SvgImageAccess::Any,
        r##"<image href="quad.png" width="10" height="10"/>
            <text x="12" y="16" font-family="Ahem" font-size="10">AB</text>"##,
    );
    let image = document
        .scene_with_resources(
            VIEWPORT,
            style(1.0),
            None,
            SvgResources::new()
                .with_fonts(&fonts)
                .with_images(&pictures),
        )
        .unwrap();
    assert_eq!(rasters(&image.root).len(), 1);
    assert!(
        image
            .root
            .children
            .iter()
            .any(|node| matches!(node, Node::Text(_)))
    );
}

// ── Budgets ─────────────────────────────────────────────────

#[test]
fn the_pixels_of_a_scene_are_bounded_and_each_picture_counts_once() {
    let pictures = Pictures::new()
        .url("a.png", Some(quad()))
        .url("b.png", Some(quad()));
    let one_picture = SceneLimits {
        image_bytes: 16,
        ..SceneLimits::default()
    };
    // One picture of 16 bytes, shown three times, fits.
    let repeated = parse(
        SvgImageAccess::Any,
        r##"<image href="a.png" width="10" height="10"/>
            <image href="a.png" x="10" width="10" height="10"/>
            <image href="a.png" x="20" width="10" height="10"/>"##,
    );
    assert_eq!(
        rasters(&build(&repeated, &pictures, one_picture).unwrap().root).len(),
        3
    );
    // A second picture does not, and nothing is asked for after it. A
    // picture the scene would not draw counts as well, because it is held.
    let two = parse(
        SvgImageAccess::Any,
        r##"<image href="a.png" width="10" height="10"/>
            <image href="b.png" x="10" width="10" height="10" visibility="hidden"/>
            <image href="c.png" x="20" width="10" height="10"/>"##,
    );
    let pictures = pictures.url("c.png", Some(quad()));
    let before = pictures.asked().len();
    let error = build(&two, &pictures, one_picture).unwrap_err();
    assert_eq!(pictures.asked()[before..], ["url a.png", "url b.png"]);
    assert!(matches!(
        error,
        SvgSceneError::LimitExceeded {
            limit: SvgSceneLimit::ImageBytes,
            max: 16
        }
    ));
    assert!(!error.falls_back_to_raster());
    assert_eq!(
        error.to_string(),
        "SVG scene exceeds the limit of 16 picture bytes"
    );
    // The default allows the output size of the raster path.
    assert_eq!(SceneLimits::default().image_bytes, 32 * 1024 * 1024);
}

#[test]
fn the_pictures_a_document_may_ask_for_are_bounded() {
    let pictures = Pictures::new();
    let images = |count: usize| {
        (0..count)
            .map(|index| format!(r##"<image href="p{index}.png" width="1" height="1"/>"##))
            .collect::<String>()
    };
    let at_the_limit = parse(SvgImageAccess::Any, &images(256));
    assert!(build(&at_the_limit, &pictures, SceneLimits::default()).is_ok());
    assert_eq!(pictures.asked().len(), 256);

    // The document asks for two more, and the source is not asked again once
    // the scene is an error.
    let over = parse(SvgImageAccess::Any, &images(258));
    let pictures = Pictures::new();
    let error = build(&over, &pictures, SceneLimits::default()).unwrap_err();
    assert_eq!(pictures.asked().len(), 256);
    assert!(matches!(
        error,
        SvgSceneError::LimitExceeded {
            limit: SvgSceneLimit::Images,
            max: 256
        }
    ));
    assert_eq!(
        error.to_string(),
        "SVG scene exceeds the limit of 256 pictures"
    );
}

#[test]
fn a_picture_counts_toward_the_node_budget() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let document = parse(
        SvgImageAccess::Any,
        r##"<image href="quad.png" width="10" height="10"/>"##,
    );
    let at_most = |nodes| SceneLimits {
        nodes,
        ..SceneLimits::default()
    };
    // The root, the wrapper group and the picture.
    assert_eq!(
        build(&document, &pictures, at_most(3))
            .unwrap()
            .node_count(),
        3
    );
    assert!(matches!(
        build(&document, &pictures, at_most(2)),
        Err(SvgSceneError::LimitExceeded {
            limit: SvgSceneLimit::Nodes,
            ..
        })
    ));
}

// ── The raster path and the fallback ────────────────────────

#[test]
fn a_document_with_pictures_is_not_rasterized_or_exported() {
    let document = parse(
        SvgImageAccess::Any,
        r##"<image href="quad.png" width="10" height="10"/>"##,
    );
    assert!(matches!(
        document.rasterize(VIEWPORT, style(1.0), None),
        Err(SvgError::UnsupportedImages)
    ));
    assert_eq!(
        SvgError::UnsupportedImages.to_string(),
        "SVG images are drawn by vector scenes only, not by the raster path"
    );
    assert!(matches!(
        document.styled_source(VIEWPORT, style(1.0)),
        Err(SvgError::ExternalReference)
    ));
}

#[test]
fn a_document_with_pictures_and_a_pattern_asks_for_the_raster_which_cannot_draw_it() {
    let pictures = Pictures::new().url("quad.png", Some(quad()));
    let document = parse(
        SvgImageAccess::Any,
        r##"<defs><pattern id="p" width="2" height="2" patternUnits="userSpaceOnUse">
                <rect width="1" height="1"/></pattern></defs>
            <image href="quad.png" width="10" height="10"/>
            <rect x="12" width="10" height="10" fill="url(#p)"/>"##,
    );
    let error = build(&document, &pictures, SceneLimits::default()).unwrap_err();
    assert!(error.falls_back_to_raster());
    assert!(matches!(
        document.rasterize(VIEWPORT, style(1.0), None),
        Err(SvgError::UnsupportedImages)
    ));
}
