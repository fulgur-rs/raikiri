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
fn resource_loader_bounds_distinct_entries_without_evicting_cache_hits() {
    let resolver = ResourceLoader::new(RecordingProvider {
        fetch_count: AtomicUsize::new(0),
        bytes: TINY_PNG,
    });
    let first_url = Url::parse("https://images.test/0.png").unwrap();
    let first = resolver.load(&first_url).unwrap();
    for index in 1..256 {
        let url = Url::parse(&format!("https://images.test/{index}.png")).unwrap();
        resolver.load(&url).unwrap();
    }
    let overflow_url = Url::parse("https://images.test/overflow.png").unwrap();
    assert!(matches!(
        resolver.load(&overflow_url),
        Err(ResolverError::Decode(_))
    ));
    assert!(resolver.cached(&overflow_url).is_none());
    assert!(Arc::ptr_eq(&first, &resolver.load(&first_url).unwrap()));
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 256);
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
    let source = ImageSource::raster(Arc::new(tiny_decoded()), test_budget(MAX_CACHE_BYTES));
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
    let source = ImageSource::raster(Arc::new(tiny_decoded()), test_budget(MAX_CACHE_BYTES));
    assert_eq!(source.decoded_byte_len(), Some(8));
}

const TWO_COLOR_SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><rect width=\"4\" height=\"2\" fill=\"red\"/></svg>";

#[test]
fn svg_source_rasterize_caches_second_call() {
    let document = SvgDocument::parse(TWO_COLOR_SVG).expect("parse svg");
    let source = ImageSource::svg(document, TWO_COLOR_SVG.into(), test_budget(MAX_CACHE_BYTES));
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

fn test_budget(limit: u64) -> Arc<Mutex<CacheBudget>> {
    Arc::new(Mutex::new(CacheBudget { used: 0, limit }))
}

#[test]
fn aggregate_pixel_and_key_budget_is_checked_before_cache_admission() {
    let first_url = Url::parse("https://images.test/a.png").unwrap();
    let second_url = Url::parse("https://images.test/b.png").unwrap();
    let budget = (first_url.as_str().len() + second_url.as_str().len() + 15) as u64;
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        budget,
        256,
    );
    let first = resolver.load(&first_url).unwrap();
    assert!(resolver.load(&second_url).is_err());
    assert!(resolver.cached_source(&second_url).is_none());
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        first_url.as_str().len() as u64 + 8
    );
    assert!(Arc::ptr_eq(&first, &resolver.load(&first_url).unwrap()));
}

#[test]
fn exact_budget_allows_cache_hits_but_rejects_another_url_before_fetch() {
    let url = Url::parse("https://images.test/a.png").unwrap();
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        url.as_str().len() as u64 + 8,
        256,
    );
    let first = resolver.load(&url).unwrap();
    assert!(Arc::ptr_eq(&first, &resolver.load(&url).unwrap()));
    assert!(
        resolver
            .load(&Url::parse("https://images.test/b.png").unwrap())
            .is_err()
    );
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 1);
}

#[test]
fn data_urls_share_the_byte_budget_and_failed_decode_does_not_charge_it() {
    let encoded = TINY_PNG
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let url = Url::parse(&format!("data:image/png,{encoded}")).unwrap();
    let resolver =
        ResourceLoader::with_limits(RejectingProvider, url.as_str().len() as u64 + 8, 256);
    assert!(
        resolver
            .load(&Url::parse("data:image/png,invalid").unwrap())
            .is_err()
    );
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    let first = resolver.load(&url).unwrap();
    assert!(
        resolver
            .load(&Url::parse(&format!("{url}#second")).unwrap())
            .is_err()
    );
    assert!(Arc::ptr_eq(&first, &resolver.load(&url).unwrap()));
}

#[test]
fn svg_source_and_late_rasterization_share_the_aggregate_budget() {
    let url = Url::parse("https://images.test/a.svg").unwrap();
    let source_bytes = (url.as_str().len() + TWO_COLOR_SVG.len()) as u64;
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TWO_COLOR_SVG,
        },
        source_bytes + 32,
        256,
    );
    let source = resolver.load_source(&url).unwrap();
    assert_eq!(resolver.budget.lock().unwrap().used, source_bytes);
    let first = source
        .rasterize(
            ImageRasterSize {
                width: 4.0,
                height: 2.0,
            },
            None,
        )
        .unwrap();
    assert_eq!(resolver.budget.lock().unwrap().used, source_bytes + 32);
    assert!(
        source
            .rasterize(
                ImageRasterSize {
                    width: 5.0,
                    height: 2.0
                },
                None
            )
            .is_none()
    );
    assert!(Arc::ptr_eq(&first, &source.cached_raster().unwrap()));
    let smaller = source
        .rasterize(
            ImageRasterSize {
                width: 2.0,
                height: 2.0,
            },
            None,
        )
        .unwrap();
    assert_eq!(smaller.rgba.len(), 16);
    assert_eq!(resolver.budget.lock().unwrap().used, source_bytes + 16);
    assert!(
        source
            .rasterize(
                ImageRasterSize {
                    width: 2.0,
                    height: 2.0
                },
                Some(15)
            )
            .is_none()
    );
    assert!(
        source
            .rasterize(
                ImageRasterSize {
                    width: 4.0,
                    height: 2.0
                },
                Some(31)
            )
            .is_none()
    );
    assert!(Arc::ptr_eq(&smaller, &source.cached_raster().unwrap()));
    assert_eq!(resolver.budget.lock().unwrap().used, source_bytes + 16);
}

#[test]
fn svg_sources_cannot_evade_the_budget_by_remaining_unrasterized() {
    let url = Url::parse("https://images.test/a.svg").unwrap();
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TWO_COLOR_SVG,
        },
        (url.as_str().len() + TWO_COLOR_SVG.len() - 1) as u64,
        256,
    );
    assert!(resolver.load_source(&url).is_err());
    assert!(resolver.cached_source(&url).is_none());
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
}

#[test]
fn svg_use_expansion_preserves_pixels_after_source_only_admission() {
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><defs><g id=\"g0\"><rect width=\"4\" height=\"2\" fill=\"red\"/></g>",
    );
    for depth in 1..=8 {
        let previous = depth - 1;
        svg.push_str(&format!(
            "<g id=\"g{depth}\"><use href=\"#g{previous}\"/><use href=\"#g{previous}\"/></g>"
        ));
    }
    svg.push_str("</defs><use href=\"#g8\"/></svg>");
    let encoded = svg.bytes().map(|b| format!("%{b:02X}")).collect::<String>();
    let url = Url::parse(&format!("data:image/svg+xml,{encoded}")).unwrap();
    let source_bytes = (url.as_str().len() + svg.len()) as u64;
    let resolver = ResourceLoader::with_limits(RejectingProvider, source_bytes + 32, 256);
    let source = resolver.load_source(&url).unwrap();
    assert_eq!(source.intrinsic.width, Some(4.0));
    assert_eq!(source.intrinsic.height, Some(2.0));
    let size = ImageRasterSize {
        width: 4.0,
        height: 2.0,
    };
    let first = source.rasterize(size, None).unwrap();
    assert!(
        first
            .rgba
            .chunks_exact(4)
            .all(|pixel| pixel == [255, 0, 0, 255])
    );
    assert!(Arc::ptr_eq(&first, &source.rasterize(size, None).unwrap()));
    assert!(
        resolver
            .load_source(&Url::parse(&format!("{url}#second")).unwrap())
            .is_err()
    );
    assert!(Arc::ptr_eq(&source, &resolver.load_source(&url).unwrap()));
}

#[test]
fn concurrent_duplicate_misses_fetch_and_charge_once() {
    let url = Url::parse("https://images.test/a.png").unwrap();
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        url.as_str().len() as u64 + 8,
        256,
    );
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                barrier.wait();
                assert!(resolver.load(&url).is_ok());
            });
        }
    });
    assert_eq!(resolver.network.fetch_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        resolver.budget.lock().unwrap().used,
        url.as_str().len() as u64 + 8
    );
}

#[test]
fn concurrent_distinct_misses_cannot_exceed_the_byte_budget() {
    let cost = "https://images.test/00.png".len() as u64 + 8;
    let resolver = ResourceLoader::with_limits(
        RecordingProvider {
            fetch_count: AtomicUsize::new(0),
            bytes: TINY_PNG,
        },
        2 * cost,
        256,
    );
    let successes = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for index in 0..8 {
            let resolver = &resolver;
            let successes = &successes;
            scope.spawn(move || {
                let url = Url::parse(&format!("https://images.test/{index:02}.png")).unwrap();
                if resolver.load(&url).is_ok() {
                    successes.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    assert_eq!(successes.load(Ordering::Relaxed), 2);
    assert_eq!(resolver.budget.lock().unwrap().used, 2 * cost);
    assert_eq!(resolver.cache.lock().unwrap().len(), 2);
}

struct OversizedImageProvider;

impl NetworkProvider for OversizedImageProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: vec![b'x'; MAX_SVG_BYTES + 1].into(),
            content_type: Some("image/png".into()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

#[test]
fn oversized_encoded_images_are_rejected_without_cache_admission() {
    let resolver = ResourceLoader::new(OversizedImageProvider);
    let url = Url::parse("https://images.test/large.png").unwrap();
    assert!(
        matches!(resolver.load(&url), Err(ResolverError::Decode(message)) if message.contains("input exceeds"))
    );
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    assert!(resolver.cached_source(&url).is_none());

    let data = Url::parse(&format!("data:image/png,{}", "x".repeat(MAX_SVG_BYTES + 1))).unwrap();
    assert!(
        matches!(resolver.load(&data), Err(ResolverError::Decode(message)) if message.contains("input exceeds"))
    );
    assert_eq!(resolver.budget.lock().unwrap().used, 0);
    assert!(resolver.cached_source(&data).is_none());
}
