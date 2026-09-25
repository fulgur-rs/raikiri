use super::*;
use raikiri_traits::{ReplacedResolver, ResolverRequest};

struct StaticBytesProvider(&'static [u8]);
impl raikiri_traits::NetworkProvider for StaticBytesProvider {
    fn fetch_one_hop(
        &self,
        _request: raikiri_traits::Request,
    ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
        Ok(raikiri_traits::FetchOutcome::Body(
            raikiri_traits::FetchedResource {
                bytes: self.0.to_vec().into(),
                content_type: Some("image/png".into()),
                final_url: url::Url::parse("file:///fixture.png").unwrap(),
                encoding: None,
            },
        ))
    }
}

/// A hand-encoded 2x1 RGBA8 PNG (red, green), generated once via the
/// `png` crate's own encoder in a throwaway script — not checked-in
/// tooling, just a fixed byte constant.
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159,
    17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

const TWO_COLOR_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##;

struct StaticSvgProvider(&'static [u8]);

impl raikiri_traits::NetworkProvider for StaticSvgProvider {
    fn fetch_one_hop(
        &self,
        request: raikiri_traits::Request,
    ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
        Ok(raikiri_traits::FetchOutcome::Body(
            raikiri_traits::FetchedResource {
                bytes: self.0.to_vec().into(),
                content_type: Some("image/svg+xml".into()),
                final_url: request.url,
                encoding: None,
            },
        ))
    }
}

#[test]
fn resolve_returns_intrinsic_size_from_decoded_png() {
    let resolver = ImageResolver::new(StaticBytesProvider(TINY_PNG));
    let url = url::Url::parse("file:///fixture.png").unwrap();
    let result = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    assert_eq!(
        (result.intrinsic.width, result.intrinsic.height),
        (2.0, 1.0)
    );
}

#[test]
fn resolves_svg_intrinsics_and_rasterizes_at_the_requested_size() {
    use raikiri_traits::{ImagePixelSource, ImageRasterSize};

    let resolver = ImageResolver::new(StaticSvgProvider(TWO_COLOR_SVG));
    let url = url::Url::parse("https://images.test/two-color.svg").unwrap();
    let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    let natural = resolver
        .intrinsic_size(&url)
        .expect("SVG metadata is cached");
    let decoded = resolver
        .get_decoded_at_size(
            &url,
            ImageRasterSize {
                width: 4.0,
                height: 2.0,
            },
            None,
        )
        .expect("SVG rasterizes at the requested size");

    assert_eq!(
        (resolved.intrinsic.width, resolved.intrinsic.height),
        (2.0, 1.0)
    );
    assert_eq!(natural.width, Some(2.0));
    assert_eq!(natural.height, Some(1.0));
    assert_eq!(natural.aspect_ratio, Some(2.0));
    assert_eq!((decoded.width, decoded.height), (4, 2));
    assert_eq!(&decoded.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&decoded.rgba[8..12], &[0, 255, 0, 255]);
}

#[test]
fn resolves_percent_encoded_data_svg_with_ratio_only_intrinsics() {
    use raikiri_traits::ImagePixelSource;

    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="4" height="2" fill="blue"/></svg>"#;
    let encoded = source
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let url = url::Url::parse(&format!("data:image/svg+xml,{encoded}")).unwrap();
    let resolver = ImageResolver::new(RejectingProvider);
    let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    let natural = resolver
        .intrinsic_size(&url)
        .expect("SVG metadata is cached");

    assert_eq!(resolved.intrinsic.aspect_ratio, Some(2.0));
    assert_eq!(natural.width, None);
    assert_eq!(natural.height, None);
    assert_eq!(natural.aspect_ratio, Some(2.0));
}

#[test]
fn get_decoded_after_resolve_returns_cached_pixels() {
    use raikiri_traits::ImagePixelSource;
    let resolver = ImageResolver::new(StaticBytesProvider(TINY_PNG));
    let url = url::Url::parse("file:///fixture.png").unwrap();
    resolver.resolve(ResolverRequest::new(&url)).unwrap();
    let decoded = resolver
        .get_decoded(&url)
        .expect("should be cached after resolve");
    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]); // red px, green px
}

/// Encodes an 8-bit PNG of `color` from `data` and hands it to a fresh
/// resolver, returning that resolver plus the fixture URL. The bytes are
/// produced by `png::Encoder` at test time (same approach as the
/// 16-bit-depth PNG coverage below) rather than hand-crafted, so the
/// fixtures stay valid if the `png` crate's output details change.
fn resolver_for_encoded(
    color: png::ColorType,
    width: u32,
    height: u32,
    data: &[u8],
    palette: Option<Vec<u8>>,
) -> (ImageResolver<StaticBytesProvider>, url::Url) {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some(palette) = palette {
            encoder.set_palette(palette);
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    (
        ImageResolver::new(StaticBytesProvider(Box::leak(buf.into_boxed_slice()))),
        url::Url::parse("file:///fixture.png").unwrap(),
    )
}

/// `ColorType::Rgb` is the most common real-world PNG color type, and its
/// arm does real 3→4 byte index arithmetic. Pin both the expansion and
/// the opaque alpha it synthesizes.
#[test]
fn decodes_rgb_png_to_rgba8_with_opaque_alpha() {
    use raikiri_traits::ImagePixelSource;

    // 2x1: red, then blue.
    let (resolver, url) =
        resolver_for_encoded(png::ColorType::Rgb, 2, 1, &[255, 0, 0, 0, 0, 255], None);
    let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    assert_eq!(
        (resolved.intrinsic.width, resolved.intrinsic.height),
        (2.0, 1.0)
    );

    let decoded = resolver.get_decoded(&url).expect("cached after resolve");
    assert_eq!(
        decoded.rgba,
        vec![255, 0, 0, 255, 0, 0, 255, 255],
        "each 3-byte RGB pixel must expand to RGBA with alpha=255"
    );
}

/// `ColorType::Grayscale` expands 1 byte to 4, replicating the gray value
/// across R/G/B and synthesizing an opaque alpha.
#[test]
fn decodes_grayscale_png_to_rgba8_with_replicated_channels() {
    use raikiri_traits::ImagePixelSource;

    // 3x1: black, mid-gray, white.
    let (resolver, url) =
        resolver_for_encoded(png::ColorType::Grayscale, 3, 1, &[0, 128, 255], None);
    resolver.resolve(ResolverRequest::new(&url)).unwrap();

    let decoded = resolver.get_decoded(&url).expect("cached after resolve");
    assert_eq!(
        decoded.rgba,
        vec![0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255],
        "gray value must land in R, G and B, with alpha=255"
    );
}

/// `ColorType::GrayscaleAlpha` expands 2 bytes to 4, replicating the gray
/// value and carrying the source alpha through rather than forcing 255.
#[test]
fn decodes_grayscale_alpha_png_to_rgba8_preserving_alpha() {
    use raikiri_traits::ImagePixelSource;

    // 2x1: opaque mid-gray, then half-transparent white.
    let (resolver, url) = resolver_for_encoded(
        png::ColorType::GrayscaleAlpha,
        2,
        1,
        &[128, 255, 255, 128],
        None,
    );
    resolver.resolve(ResolverRequest::new(&url)).unwrap();

    let decoded = resolver.get_decoded(&url).expect("cached after resolve");
    assert_eq!(
        decoded.rgba,
        vec![128, 128, 128, 255, 255, 255, 255, 128],
        "source alpha must be carried through, not replaced by 255"
    );
}

/// The generic decoder expands indexed PNG pixels to the shared RGBA8
/// contract instead of exposing palette indices to the paint path.
#[test]
fn decodes_indexed_palette_png_to_rgba8() {
    use raikiri_traits::ImagePixelSource;

    // 2x1 image of palette entries 0 and 1 (red, blue).
    let (resolver, url) = resolver_for_encoded(
        png::ColorType::Indexed,
        2,
        1,
        &[0, 1],
        Some(vec![255, 0, 0, 0, 0, 255]),
    );

    resolver.resolve(ResolverRequest::new(&url)).unwrap();
    let decoded = resolver.get_decoded(&url).expect("cached after resolve");
    assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
}

/// Higher-depth PNG input is down-converted by the generic decoder while
/// preserving the invariant that paint receives exactly RGBA8 bytes.
#[test]
fn decodes_16_bit_depth_png_to_rgba8() {
    use raikiri_traits::ImagePixelSource;

    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0, 255, 0, 255, 0, 255, 0, 255])
            .unwrap();
    }

    let resolver = ImageResolver::new(StaticBytesProvider(Box::leak(buf.into_boxed_slice())));
    let url = url::Url::parse("file:///sixteen-bit.png").unwrap();
    resolver.resolve(ResolverRequest::new(&url)).unwrap();
    let decoded = resolver.get_decoded(&url).expect("cached after resolve");
    assert_eq!((decoded.width, decoded.height), (1, 1));
    assert_eq!(decoded.rgba.len(), 4);
}

/// The common non-PNG formats enabled in Cargo.toml all flow through the
/// same RGBA8 normalization path. The URL suffix is intentionally not
/// involved in format detection.
#[test]
fn decodes_common_non_png_formats_to_rgba8() {
    use std::io::Cursor;

    let source = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 255, 0]).unwrap();
    for (format, suffix) in [
        (image::ImageFormat::Jpeg, "fixture.bin"),
        (image::ImageFormat::Gif, "fixture.dat"),
        (image::ImageFormat::WebP, "fixture.unknown"),
    ] {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(source.clone())
            .write_to(&mut encoded, format)
            .unwrap_or_else(|error| panic!("encode {format:?}: {error}"));
        let resolver = ImageResolver::new(StaticBytesProvider(Box::leak(
            encoded.into_inner().into_boxed_slice(),
        )));
        let url = url::Url::parse(&format!("file:///{suffix}")).unwrap();
        resolver.resolve(ResolverRequest::new(&url)).unwrap();
        let decoded = resolver.get_decoded(&url).expect("cached after resolve");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba.len(), 2 * 4);
        assert!(decoded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}

struct RejectingProvider;

impl raikiri_traits::NetworkProvider for RejectingProvider {
    fn fetch_one_hop(
        &self,
        _request: raikiri_traits::Request,
    ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
        Err(raikiri_traits::NetworkError::Other(
            "data URL must not reach the provider".into(),
        ))
    }
}

#[test]
fn non_data_provider_errors_are_propagated() {
    let resolver = ImageResolver::new(RejectingProvider);
    let url = url::Url::parse("file:///fixture.png").unwrap();
    let error = resolver.resolve(ResolverRequest::new(&url)).unwrap_err();
    assert!(matches!(
        error,
        raikiri_traits::ResolverError::Network(raikiri_traits::NetworkError::Other(message))
            if message == "data URL must not reach the provider"
    ));
}

#[test]
fn decodes_base64_data_url_without_calling_network_provider() {
    let resolver = ImageResolver::new(RejectingProvider);
    let url = url::Url::parse(concat!(
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEElEQVR4nGP5z8Dw",
        "nxFIAAAQIQMDHl0gUAAAAABJRU5ErkJggg==",
    ))
    .unwrap();
    let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    assert_eq!(
        (resolved.intrinsic.width, resolved.intrinsic.height),
        (2.0, 1.0)
    );
    assert!(resolver.get_decoded(&url).is_some());
}

#[test]
fn decodes_percent_encoded_data_url_without_calling_network_provider() {
    let encoded = TINY_PNG
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let url = url::Url::parse(&format!("data:image/png,{encoded}")).unwrap();
    let resolver = ImageResolver::new(RejectingProvider);
    let resolved = resolver.resolve(ResolverRequest::new(&url)).unwrap();
    assert_eq!(
        (resolved.intrinsic.width, resolved.intrinsic.height),
        (2.0, 1.0)
    );
}

#[test]
fn invalid_data_url_is_reported_as_a_decode_error() {
    let resolver = ImageResolver::new(RejectingProvider);
    let url = url::Url::parse("data:image/png;base64,not-valid").unwrap();
    let error = resolver.resolve(ResolverRequest::new(&url)).unwrap_err();
    assert!(matches!(
        error,
        raikiri_traits::ResolverError::Decode(message)
            if message.contains("invalid data URL payload")
    ));
}
