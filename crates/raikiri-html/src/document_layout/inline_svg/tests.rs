use super::*;
mod storage_tests;
use raikiri_svg::{SvgFontRequest, SvgSceneLimit};
use raikiri_traits::vector::FontFace;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// Answers every request with Ahem.
struct Ahem(FontFace);

impl Ahem {
    fn new() -> Self {
        Self(FontFace::new(Arc::new(AHEM), 0))
    }
}

impl SvgFontSource for Ahem {
    fn select(&self, _: &SvgFontRequest) -> Option<FontFace> {
        Some(self.0.clone())
    }

    fn fallback(&self, _: &SvgFontRequest, _: char) -> Option<FontFace> {
        Some(self.0.clone())
    }
}

/// A document over a 40 by 20 viewport with `body` inside the root.
fn document(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"#)
}

const RECT: &str = r##"<rect width="40" height="20" fill="#c00"/>"##;

const MASKED: &str = r##"<mask id="m"><rect width="20" height="20" fill="#fff"/></mask>
    <rect width="40" height="20" fill="#c00" mask="url(#m)"/>"##;

const PATTERNED: &str = r##"<pattern id="p" width="4" height="4" patternUnits="userSpaceOnUse">
        <rect width="2" height="2"/></pattern>
    <rect width="40" height="20" fill="url(#p)"/>"##;

/// Stroked text under `<g transform>`, with `attributes` on the `<text>`.
fn stroked_text(transform: &str, attributes: &str) -> String {
    format!(
        r##"<g transform="{transform}"><text x="2" y="12" font-family="Ahem" font-size="8"
            fill="none" stroke="#006" stroke-width="2" {attributes}>AB</text></g>"##
    )
}

/// Stretches the glyphs of 16 pixels of text over 30, wider than they are high.
const STRETCHED: &str = r#"textLength="30" lengthAdjust="spacingAndGlyphs""#;

fn request<'a>(source: &'a str, fonts: &'a Ahem) -> PictureRequest<'a> {
    PictureRequest {
        source,
        viewport: PaintRect::new(5.0, 6.0, 40.0, 20.0),
        host_opacity: None,
        rendering: SvgRendering::Vector,
        raster_scale: 1.0,
        fonts,
    }
}

fn picture(
    source: &str,
    change: impl FnOnce(&mut PictureRequest<'_>),
) -> Result<ImageContent, SvgSceneError> {
    let fonts = Ahem::new();
    let mut request = request(source, &fonts);
    change(&mut request);
    build_picture(&request)
}

fn is_scene(content: &Result<ImageContent, SvgSceneError>) -> bool {
    matches!(content, Ok(ImageContent::Vector(_)))
}

fn pixels(content: Result<ImageContent, SvgSceneError>) -> Arc<raikiri_traits::DecodedImage> {
    match content {
        Ok(ImageContent::Raster(pixels)) => pixels,
        other => panic!("expected pixels, got {other:?}"),
    }
}

#[test]
fn a_document_in_the_vector_subset_is_a_scene_of_its_viewport() {
    let source = document(RECT);
    let Ok(ImageContent::Vector(scene)) = picture(&source, |_| {}) else {
        panic!("expected a scene");
    };
    assert_eq!(scene.size, (40.0, 20.0));
    assert!(scene.node_count() > 1);
}

#[test]
fn a_root_that_asks_for_pixels_gets_the_raster_at_the_raster_scale() {
    let source = document(RECT);
    let at = |scale| {
        pixels(picture(&source, |request| {
            request.rendering = SvgRendering::Raster;
            request.raster_scale = scale;
        }))
    };
    let (one, two) = (at(1.0), at(2.0));
    assert_eq!((one.width, one.height), (40, 20));
    assert_eq!((two.width, two.height), (80, 40));
    assert_eq!(&two.rgba[..4], &[0xcc, 0, 0, 0xff]);
}

#[test]
fn a_mask_is_drawn_as_pixels_whatever_the_root_asks_for() {
    let source = document(MASKED);
    let raster = pixels(picture(&source, |_| {}));
    assert_eq!((raster.width, raster.height), (40, 20));
    // The mask keeps the left half of the rectangle.
    let alpha = |x: usize| raster.rgba[(10 * 40 + x) * 4 + 3];
    assert_eq!((alpha(10), alpha(30)), (255, 0));
}

const BLURRED: &str = r##"<filter id="f"><feGaussianBlur stdDeviation="2"/></filter>
    <rect x="10" y="5" width="20" height="10" fill="#c00" filter="url(#f)"/>"##;

#[test]
fn a_filter_is_drawn_as_pixels_with_its_effect() {
    let source = document(BLURRED);
    let raster = pixels(picture(&source, |_| {}));
    assert_eq!((raster.width, raster.height), (40, 20));
    let alpha = |x: usize, y: usize| raster.rgba[(y * 40 + x) * 4 + 3];
    // The middle of the rectangle is nearly solid, its edge is soft, and the blur
    // reaches beyond the rectangle.
    assert!(alpha(20, 10) > 240);
    assert!(alpha(10, 10) > 0 && alpha(10, 10) < 255);
    assert!(alpha(8, 10) > 0 && alpha(8, 10) < alpha(10, 10));
}

#[test]
fn a_filter_over_the_budget_of_the_raster_is_an_error_that_says_so() {
    let source = document(&format!(
        "<filter id=\"f\">{}</filter><rect width=\"40\" height=\"20\" filter=\"url(#f)\"/>",
        "<feOffset dx=\"1\"/>".repeat(40)
    ));
    let result = picture(&source, |request| request.raster_scale = 100.0);
    assert!(
        matches!(
            result,
            Err(SvgSceneError::Svg(SvgError::FilterLimitExceeded { .. }))
        ),
        "{result:?}"
    );
}

#[test]
fn a_pattern_is_drawn_as_pixels_at_the_raster_scale() {
    let source = document(PATTERNED);
    let raster = pixels(picture(&source, |request| request.raster_scale = 2.0));
    assert_eq!((raster.width, raster.height), (80, 40));
}

#[test]
fn stroked_glyphs_off_a_similarity_are_drawn_as_pixels() {
    let stretched = document(&stroked_text("none", STRETCHED));
    assert_eq!(pixels(picture(&stretched, |_| {})).width, 40);
}

#[test]
fn text_a_backend_strokes_faithfully_stays_a_scene() {
    // A group carries its transform to the backend, which strokes in the
    // space the group makes, so only the transform of a glyph matters.
    for transform in ["translate(1 1)", "rotate(30)", "scale(2 1)", "skewX(30)"] {
        let source = document(&stroked_text(transform, ""));
        assert!(is_scene(&picture(&source, |_| {})), "{transform}");
    }
    let rotated = document(&stroked_text("none", r#"rotate="30""#));
    assert!(is_scene(&picture(&rotated, |_| {})));
    // Stretched glyphs that are only filled are a glyph run like any other.
    let filled = document(
        r##"<text x="2" y="12" font-family="Ahem" font-size="8" textLength="30"
            lengthAdjust="spacingAndGlyphs">AB</text>"##,
    );
    assert!(is_scene(&picture(&filled, |_| {})));
}

#[test]
fn a_scene_stands_in_when_pixels_are_not_to_be_had() {
    // Nothing can be rasterized at this scale.
    let hopeless = |request: &mut PictureRequest<'_>| request.raster_scale = 1.0e9;
    let stretched = document(&stroked_text("none", STRETCHED));
    assert!(is_scene(&picture(&stretched, hopeless)));
    let source = document(RECT);
    assert!(is_scene(&picture(&source, |request| {
        hopeless(request);
        request.rendering = SvgRendering::Raster;
    })));
}

#[test]
fn a_document_that_has_neither_picture_is_an_error() {
    let hopeless = |request: &mut PictureRequest<'_>| request.raster_scale = 1.0e9;
    let masked = document(MASKED);
    // The raster is the picture of a mask, so its failure is the error.
    assert!(matches!(
        picture(&masked, hopeless),
        Err(SvgSceneError::Svg(SvgError::InvalidViewport))
    ));
}

#[test]
fn a_viewport_without_an_area_has_no_picture_in_either_rendering() {
    let source = document(RECT);
    for rendering in [SvgRendering::Vector, SvgRendering::Raster] {
        let result = picture(&source, |request| {
            request.viewport = PaintRect::new(0.0, 0.0, 0.0, 20.0);
            request.rendering = rendering;
        });
        assert!(
            matches!(result, Err(SvgSceneError::Svg(SvgError::InvalidViewport))),
            "{result:?}"
        );
    }
}

#[test]
fn an_invalid_document_is_an_error_even_for_a_root_that_asks_for_pixels() {
    let result = picture("<svg", |request| request.rendering = SvgRendering::Raster);
    assert!(matches!(result, Err(SvgSceneError::Svg(_))), "{result:?}");
    let external =
        document(r#"<image href="https://example.invalid/a.png" width="4" height="4"/>"#);
    assert!(matches!(
        picture(&external, |request| request.rendering =
            SvgRendering::Raster),
        Err(SvgSceneError::Svg(SvgError::ExternalReference))
    ));
}

/// A document whose one gradient has more stops than a scene permits.
fn too_many_stops() -> String {
    let stops = (0..1025)
        .map(|index| {
            format!(
                r##"<stop offset="{}" stop-color="#c00"/>"##,
                index as f32 / 1024.0
            )
        })
        .collect::<String>();
    document(&format!(
        r##"<linearGradient id="g">{stops}</linearGradient><rect width="40" height="20" fill="url(#g)"/>"##
    ))
}

#[test]
fn a_scene_over_a_budget_is_drawn_as_pixels() {
    let source = too_many_stops();
    assert_eq!(pixels(picture(&source, |_| {})).width, 40);
}

#[test]
fn a_scene_over_a_budget_is_an_error_when_pixels_are_not_to_be_had() {
    let source = too_many_stops();
    let result = picture(&source, |request| request.raster_scale = 1.0e9);
    assert!(
        matches!(
            result,
            Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::GradientStops,
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn the_host_opacity_is_left_to_the_host_around_the_picture() {
    let with_opacity = |source: &str, opacity: Option<f32>, rendering| {
        picture(source, |request| {
            request.host_opacity = opacity;
            request.rendering = rendering;
        })
    };
    let plain = document(RECT);
    let scene = |opacity| match with_opacity(&plain, opacity, SvgRendering::Vector) {
        Ok(ImageContent::Vector(scene)) => scene,
        other => panic!("expected a scene, got {other:?}"),
    };
    assert_eq!(scene(None), scene(Some(0.5)));
    assert_eq!(scene(None), scene(Some(1.0)));
    let raster = pixels(with_opacity(&plain, Some(0.5), SvgRendering::Raster));
    assert_eq!(raster.rgba[3], 255);

    // What the root's descendants inherit from it is theirs, though.
    let inheriting = document(r##"<rect width="40" height="20" fill="#c00" opacity="inherit"/>"##);
    let inherited = |opacity| match with_opacity(&inheriting, opacity, SvgRendering::Vector) {
        Ok(ImageContent::Vector(scene)) => scene,
        other => panic!("expected a scene, got {other:?}"),
    };
    assert_ne!(inherited(None), inherited(Some(0.5)));
    let raster = pixels(with_opacity(&inheriting, Some(0.5), SvgRendering::Raster));
    assert_eq!(raster.rgba[3], 128);
}

#[test]
fn similarity_transforms_keep_angles_and_the_ratio_of_lengths() {
    let transform = |a, b, c, d| PaintTransform::new(a, b, c, d, 3.0, -4.0);
    assert!(is_similarity(transform(1.0, 0.0, 0.0, 1.0)));
    assert!(is_similarity(transform(2.0, 0.0, 0.0, -2.0)));
    assert!(is_similarity(transform(0.0, 3.0, -3.0, 0.0)));
    assert!(!is_similarity(transform(2.0, 0.0, 0.0, 1.0)));
    assert!(!is_similarity(transform(1.0, 0.0, 1.0, 1.0)));
}

fn raster_of(bytes: usize) -> ImageContent {
    ImageContent::Raster(Arc::new(raikiri_traits::DecodedImage::new(
        1,
        1,
        vec![0; bytes],
    )))
}

fn picture_key(source: &str, rendering: SvgRendering) -> PictureKey {
    let prepared = PreparedSvg::new(source.into(), None, rendering);
    PictureKey::new(&prepared, PaintRect::new(0.0, 0.0, 40.0, 20.0))
}

#[test]
fn a_picture_is_kept_for_the_next_request_while_there_is_room_for_it() {
    let cache = InlineSvgCache::new(Arc::default());
    let source = document(RECT);
    let key = picture_key(&source, SvgRendering::Vector);
    assert!(cache.picture(&key).is_none());
    let content = picture(&source, |_| {}).unwrap();
    cache.keep(key.clone(), &content);
    assert_eq!(cache.kept_pictures(), 1);
    let (ImageContent::Vector(kept), ImageContent::Vector(built)) =
        (cache.picture(&key).unwrap(), content)
    else {
        panic!("expected scenes");
    };
    assert!(Arc::ptr_eq(&kept, &built));
}

#[test]
fn a_picture_is_told_by_its_markup_opacity_size_and_rendering_and_not_by_its_place() {
    let source = document(RECT);
    let viewport = PaintRect::new(0.0, 0.0, 40.0, 20.0);
    let key = |source: &str, opacity, viewport, rendering| {
        PictureKey::new(
            &PreparedSvg::new(source.into(), opacity, rendering),
            viewport,
        )
    };
    let base = key(&source, None, viewport, SvgRendering::Vector);
    // The same markup somewhere else on the page is the same picture.
    let elsewhere = PaintRect::new(9.0, -3.0, 40.0, 20.0);
    assert_eq!(base, key(&source, None, elsewhere, SvgRendering::Vector));
    let wider = PaintRect::new(0.0, 0.0, 41.0, 20.0);
    for other in [
        key(&format!("{source} "), None, viewport, SvgRendering::Vector),
        key(&source, Some(0.5), viewport, SvgRendering::Vector),
        key(&source, None, wider, SvgRendering::Vector),
        key(&source, None, viewport, SvgRendering::Raster),
    ] {
        assert_ne!(base, other);
    }
}

#[test]
fn a_raster_is_not_kept_past_the_budget_of_rasters() {
    let cache = InlineSvgCache::new(Arc::default());
    let first = picture_key("a", SvgRendering::Vector);
    let second = picture_key("b", SvgRendering::Vector);
    cache.keep(first.clone(), &raster_of(MAX_KEPT_RASTER_BYTES - 1));
    // One byte of room is left, and the second picture needs more.
    cache.keep(second.clone(), &raster_of(2));
    assert_eq!(cache.kept_pictures(), 1);
    assert!(cache.picture(&second).is_none());
    // A kept picture is not replaced.
    cache.keep(first.clone(), &raster_of(1));
    assert_eq!(cache.kept_pictures(), 1);
    assert!(cache.picture(&first).is_some());
}
