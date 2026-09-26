use super::*;
use raikiri_traits::{FetchOutcome, NetworkError, NetworkProvider};
use std::sync::atomic::{AtomicUsize, Ordering};

struct RejectingProvider;

impl NetworkProvider for RejectingProvider {
    fn fetch_one_hop(&self, _request: Request) -> Result<FetchOutcome, NetworkError> {
        Err(NetworkError::Other("provider was called".into()))
    }
}

#[test]
fn base64_data_url_is_decoded_without_calling_provider() {
    let resolver = ResourceLoader::new(RejectingProvider);
    let url = Url::parse(concat!(
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEElEQVR4nGP5z8Dw",
        "nxFIAAAQIQMDHl0gUAAAAABJRU5ErkJggg==",
    ))
    .unwrap();

    let decoded = resolver.load(&url).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 1));
}

/// A hand-encoded 2x1 RGBA8 PNG (red, green).
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159,
    17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn percent_encoded_data_url_is_decoded_without_calling_provider() {
    let encoded = TINY_PNG
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    let resolver = ResourceLoader::new(RejectingProvider);
    let url = Url::parse(&format!("data:image/png,{encoded}")).unwrap();

    let decoded = resolver.load(&url).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 1));
}

#[test]
fn malformed_data_url_is_a_decode_error() {
    let resolver = ResourceLoader::new(RejectingProvider);
    let url = Url::parse("data:image/png;base64,not-valid").unwrap();
    let error = resolver.load(&url).unwrap_err();

    assert!(matches!(
        error,
        ResolverError::Decode(message)
            if message.contains("invalid data URL payload")
    ));
}

#[test]
fn invalid_data_url_image_bytes_are_decode_errors() {
    let resolver = ResourceLoader::new(RejectingProvider);
    let url = Url::parse("data:image/png,not-an-image").unwrap();
    let error = resolver.load(&url).unwrap_err();

    assert!(matches!(
        error,
        ResolverError::Decode(message)
            if message.contains("data URL image decode failed")
    ));
}

struct RecordingProvider {
    fetch_count: AtomicUsize,
    bytes: &'static [u8],
}

impl NetworkProvider for RecordingProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        assert_eq!(request.method, Method::Get);
        assert!(matches!(request.kind, ResourceKind::Image));
        self.fetch_count.fetch_add(1, Ordering::Relaxed);
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: self.bytes.to_vec().into(),
            content_type: Some("image/png".into()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

#[test]
fn fetched_invalid_image_bytes_are_decode_errors() {
    let resolver = ResourceLoader::new(RecordingProvider {
        fetch_count: AtomicUsize::new(0),
        bytes: b"not an image",
    });
    let url = Url::parse("file:///fixture.png").unwrap();
    let error = resolver.load(&url).unwrap_err();

    assert!(matches!(error, ResolverError::Decode(message) if !message.is_empty()));
}

#[test]
fn resource_loader_caches_successful_decodes() {
    let provider = RecordingProvider {
        fetch_count: AtomicUsize::new(0),
        bytes: TINY_PNG,
    };
    let resolver = ResourceLoader::new(provider);
    let url = Url::parse("file:///fixture.png").unwrap();

    let first = resolver.load(&url).unwrap();
    let second = resolver.load(&url).unwrap();

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.width, 2);
    assert_eq!(first.height, 1);
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 1);
    assert!(resolver.cached(&url).is_some());
}

#[test]
fn provider_errors_remain_network_errors() {
    let resolver = ResourceLoader::new(RejectingProvider);
    let url = Url::parse("file:///fixture.png").unwrap();
    let error = resolver.load(&url).unwrap_err();
    assert!(matches!(
        error,
        ResolverError::Network(NetworkError::Other(_))
    ));
}

fn tiny_decoded() -> DecodedImage {
    DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    }
}

#[test]
fn raster_source_rasterize_respects_output_limit() {
    let source = ImageSource::raster(Arc::new(tiny_decoded()));
    let size = ImageRasterSize {
        width: 2.0,
        height: 1.0,
    };
    assert!(source.rasterize(size, None).is_some());
    assert!(source.rasterize(size, Some(8)).is_some());
    assert_eq!(source.rasterize(size, Some(7)), None);
}

#[test]
fn raster_source_reports_decoded_byte_len() {
    let source = ImageSource::raster(Arc::new(tiny_decoded()));
    assert_eq!(source.decoded_byte_len(), Some(8));
}

const TWO_COLOR_SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><rect width=\"4\" height=\"2\" fill=\"red\"/></svg>";

#[test]
fn svg_source_rasterize_caches_second_call() {
    let document = SvgDocument::parse(TWO_COLOR_SVG).expect("parse svg");
    let source = ImageSource::svg(document);
    assert_eq!(source.decoded_byte_len(), None);
    let size = ImageRasterSize {
        width: 4.0,
        height: 2.0,
    };
    let first = source.rasterize(size, None).expect("rasterize");
    assert_eq!((first.width, first.height), (4, 2));
    assert!(source.cached_raster().is_some());
    let second = source.rasterize(size, None).expect("cached rasterize");
    assert_eq!(second.rgba, first.rgba);
}
