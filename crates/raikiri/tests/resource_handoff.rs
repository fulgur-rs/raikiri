//! Consumer-facing resource-handoff integration tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raikiri::{
    Bytes, DecodedImage, FetchedResource, FontContextBuilder, ImagePixelSource, IntrinsicBox,
    NetworkError, NetworkProvider, PageDefaults, PageFragment, RenderError, RenderOptions,
    RenderResources, RenderSink, RenderStatus, RenderSummary, ReplacedResolver, Request,
    ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest, ResourceKind,
    ResourceLimits, ResourcePolicy, StreamingConfig, Url, WarningKind, parse_html_with_resources,
    render_streaming,
};

/// Parse with `resources`, then render with the same resource handoff.
#[allow(clippy::result_large_err)]
fn parse_and_render<R: std::io::Read>(
    input: R,
    defaults: PageDefaults,
    resources: &RenderResources<'_>,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    let doc = parse_html_with_resources(input, resources)?;
    render_streaming(
        &doc,
        defaults,
        config,
        RenderOptions::new().resources(resources),
        sink,
    )
}

#[derive(Default)]
struct PageCollector {
    pages: Vec<PageFragment>,
}

impl RenderSink for PageCollector {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn finish_render(&mut self, _summary: RenderSummary) -> std::io::Result<()> {
        Ok(())
    }
}

/// [`parse_and_render`] into a collector that retains every emitted page.
#[allow(clippy::result_large_err)]
fn parse_and_collect_pages<R: std::io::Read>(
    input: R,
    defaults: PageDefaults,
    resources: &RenderResources<'_>,
    config: StreamingConfig,
) -> Result<(Vec<PageFragment>, RenderStatus), RenderError> {
    let mut sink = PageCollector::default();
    let status = parse_and_render(input, defaults, resources, config, &mut sink)?;
    Ok((sink.pages, status))
}

#[derive(Default)]
struct CountingNetwork {
    calls: AtomicUsize,
}

impl NetworkProvider for CountingNetwork {
    fn fetch(&self, _request: Request) -> Result<FetchedResource, NetworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(NetworkError::Aborted)
    }
}

#[derive(Default)]
struct FixedResolver {
    seen: Mutex<Vec<Url>>,
}

impl ReplacedResolver for FixedResolver {
    fn resolve(&self, request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        self.seen.lock().unwrap().push(request.url().clone());
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(48.0, 24.0),
            disposition: ResolveDisposition::Fallback {
                reason: "test placeholder".into(),
            },
        })
    }
}

impl ImagePixelSource for FixedResolver {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 0],
        }))
    }
}

#[derive(Default)]
struct CapturingSink {
    pages: usize,
    summary: Option<RenderSummary>,
}

impl RenderSink for CapturingSink {
    fn accept_page(&mut self, _page: raikiri::PageFragment) -> std::io::Result<()> {
        self.pages += 1;
        Ok(())
    }

    fn finish_render(&mut self, summary: RenderSummary) -> std::io::Result<()> {
        self.summary = Some(summary);
        Ok(())
    }
}

fn bundled_test_font() -> raikiri::FontContext {
    // This render fixture has no text; it verifies the bundled-context handoff.
    FontContextBuilder::new()
        .font_bytes(
            "Bundled Handoff Test",
            include_bytes!("data/NotoSansTest-Regular.ttf").as_slice(),
        )
        .build()
        .expect("font bytes register")
}

#[test]
fn bundled_no_network_resources_share_base_resolver_and_summary() {
    let network = CountingNetwork::default();
    let resolver = FixedResolver::default();
    let base_url = Url::parse("https://example.test/books/chapter.html").unwrap();
    let resources = RenderResources::new()
        .base_url(base_url)
        .network_provider(&network)
        .font_context(bundled_test_font())
        .replaced_resource_provider(&resolver);
    let mut sink = CapturingSink::default();

    let status = parse_and_render(
        &b"<html><body style='font-family: \"Bundled Handoff Test\"'>A<img src='../images/cover.png'></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("resource-aware render succeeds without network access");

    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };
    assert_eq!(sink.pages, 1);
    assert_eq!(summary.total_pages, 1);
    assert_eq!(network.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        resolver.seen.lock().unwrap().as_slice(),
        [Url::parse("https://example.test/images/cover.png").unwrap()]
    );
    assert!(
        resources
            .image_pixel_source_ref()
            .unwrap()
            .get_decoded(&Url::parse("https://example.test/images/cover.png").unwrap())
            .is_some()
    );
    assert!(summary.warnings.iter().any(|warning| matches!(
        &warning.kind,
        WarningKind::ResourceFallback {
            kind: ResourceKind::Image,
            url: Some(url),
        } if url.as_str() == "https://example.test/images/cover.png"
    )));
}

#[test]
fn bundled_no_network_page_output_is_deterministic_across_runs() {
    let network = CountingNetwork::default();
    let resolver = FixedResolver::default();
    let resources = RenderResources::new()
        .base_url(Url::parse("https://example.test/books/chapter.html").unwrap())
        .network_provider(&network)
        .font_context(bundled_test_font())
        .replaced_resource_provider(&resolver);
    let mut outputs = Vec::new();

    for _ in 0..3 {
        let (pages, status) = parse_and_collect_pages(
            &b"<html><body style='font-family: \"Bundled Handoff Test\"'>A<img src='../images/cover.png'></body></html>"[..],
            PageDefaults::default(),
            &resources,
            StreamingConfig::default(),
        )
        .expect("bundled no-network render succeeds");
        let RenderStatus::Completed(summary) = status else {
            panic!("expected a complete render");
        };
        assert_eq!(summary.total_pages, 1);
        outputs.push(pages);
    }

    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1], outputs[2]);
    assert_eq!(network.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn html_base_element_overrides_configured_fallback_for_replaced_resources() {
    let resolver = FixedResolver::default();
    let resources = RenderResources::new()
        .base_url(Url::parse("https://example.test/books/chapter.html").unwrap())
        .replaced_resource_provider(&resolver);
    let mut sink = CapturingSink::default();

    parse_and_render(
        &b"<html><head><base href='/assets/'></head><body><img src='cover.png'></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("document base is valid");

    assert_eq!(
        resolver.seen.lock().unwrap().as_slice(),
        [Url::parse("https://example.test/assets/cover.png").unwrap()]
    );
}

#[test]
fn batch_collection_uses_the_resource_aware_page_path() {
    let resources = RenderResources::new();
    let (pages, status) = parse_and_collect_pages(
        &b"<html><body><div></div></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
    )
    .expect("batch collection succeeds");

    assert_eq!(pages.len(), 1);
    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };
    assert_eq!(summary.total_pages, 1);
}

struct OversizedStylesheetProvider {
    requests: Mutex<Vec<Request>>,
}

impl NetworkProvider for OversizedStylesheetProvider {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(FetchedResource {
            bytes: Bytes::from_static(b"12345"),
            content_type: Some("text/css".into()),
            final_url: request.url,
            encoding: None,
        })
    }
}

#[test]
fn response_limits_are_reported_in_render_summary() {
    let network = OversizedStylesheetProvider {
        requests: Mutex::new(Vec::new()),
    };
    let resources = RenderResources::new()
        .base_url(Url::parse("https://example.test/docs/index.html").unwrap())
        .network_provider(&network)
        .resource_limits(
            ResourceLimits::new()
                .max_resource_bytes(Some(4))
                .max_aggregate_resource_bytes(Some(16)),
        );
    let mut sink = CapturingSink::default();

    let status = parse_and_render(
        &b"<html><head><link rel='stylesheet' href='main.css'></head><body><p>Hi</p></body></html>"
            [..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("an oversized stylesheet is a non-fatal fallback");
    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };

    let requests = network.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.as_str(),
        "https://example.test/docs/main.css"
    );
    assert!(summary.warnings.iter().any(|warning| matches!(
        warning.kind,
        WarningKind::ResourceLimitExceeded {
            kind: ResourceKind::ExternalStylesheet,
            limit: 4,
            actual: 5,
        }
    )));
}

struct OneByteProvider {
    requests: Mutex<Vec<Request>>,
}

impl NetworkProvider for OneByteProvider {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(FetchedResource {
            bytes: Bytes::from_static(b"x"),
            content_type: Some("text/css".into()),
            final_url: request.url,
            encoding: None,
        })
    }
}

#[test]
fn aggregate_response_budget_is_shared_by_parse_and_font_loading() {
    let network = OneByteProvider {
        requests: Mutex::new(Vec::new()),
    };
    let resources = RenderResources::new()
        .base_url(Url::parse("https://example.test/book/page.html").unwrap())
        .network_provider(&network)
        .stylesheet("@font-face { font-family: 'Web'; src: url('font.ttf'); }")
        .resource_limits(
            ResourceLimits::new()
                .max_resource_bytes(Some(4))
                .max_aggregate_resource_bytes(Some(1)),
        );
    let mut sink = CapturingSink::default();

    let status = parse_and_render(
        &b"<html><head><link rel='stylesheet' href='main.css'></head><body></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("the second resource falls back after aggregate budget exhaustion");
    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };

    let requests = network.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].kind, ResourceKind::ExternalStylesheet);
    assert_eq!(requests[1].kind, ResourceKind::Font);
    assert!(summary.warnings.iter().any(|warning| matches!(
        warning.kind,
        WarningKind::ResourceLimitExceeded {
            kind: ResourceKind::Font,
            limit: 1,
            actual: 2,
        }
    )));
}

struct ResourceTestPolicy {
    hosts_allowed: bool,
    decoded_image_limit: Option<u64>,
}

impl ResourcePolicy for ResourceTestPolicy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
        self.hosts_allowed
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        false
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        0
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64> {
        if kind == ResourceKind::Image {
            self.decoded_image_limit
        } else {
            None
        }
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(1)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(1)
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        1
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        1
    }
}

struct SuccessfulImageProvider;

impl ReplacedResolver for SuccessfulImageProvider {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(1.0, 1.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

impl ImagePixelSource for SuccessfulImageProvider {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 0],
        }))
    }
}

#[test]
fn decoded_image_limit_is_reported_and_paint_source_refuses_the_image() {
    let policy = ResourceTestPolicy {
        hosts_allowed: true,
        decoded_image_limit: Some(3),
    };
    let provider = SuccessfulImageProvider;
    let resources = RenderResources::new()
        .base_url(Url::parse("https://example.test/book/page.html").unwrap())
        .network_policy(&policy)
        .replaced_resource_provider(&provider);
    let mut sink = CapturingSink::default();

    let status = parse_and_render(
        &b"<html><body><img src='image.png'></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("oversized decoded pixels fall back");
    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };

    assert!(summary.warnings.iter().any(|warning| matches!(
        warning.kind,
        WarningKind::ResourceLimitExceeded {
            kind: ResourceKind::Image,
            limit: 3,
            actual: 4,
        }
    )));
    assert!(
        resources
            .image_pixel_source_ref()
            .unwrap()
            .get_decoded(&Url::parse("https://example.test/book/image.png").unwrap())
            .is_none()
    );
}

#[test]
fn network_policy_applies_to_stylesheet_font_and_replaced_resource_fetches() {
    let network = CountingNetwork::default();
    let resolver = FixedResolver::default();
    let resources = RenderResources::new()
        .base_url(Url::parse("https://blocked.test/book/page.html").unwrap())
        .stylesheet(
            "@import url('reset.css'); @font-face { font-family: 'Web'; src: url('font.ttf'); } body { font-family: 'Web'; }",
        )
        .network_provider(&network)
        .network_policy(&ResourceTestPolicy {
            hosts_allowed: false,
            decoded_image_limit: None,
        })
        .replaced_resource_provider(&resolver);
    let mut sink = CapturingSink::default();

    let status = parse_and_render(
        &b"<html><head><link rel='stylesheet' href='main.css'></head><body><p>Hi</p><img src='image.png'></body></html>"[..],
        PageDefaults::default(),
        &resources,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("denied resources fall back without aborting render");
    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };

    assert_eq!(network.calls.load(Ordering::SeqCst), 0);
    assert!(resolver.seen.lock().unwrap().is_empty());
    assert!(
        resources
            .image_pixel_source_ref()
            .unwrap()
            .get_decoded(&Url::parse("https://blocked.test/book/image.png").unwrap())
            .is_none()
    );
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| matches!(warning.kind, WarningKind::PolicyWarning { .. }))
    );
    assert!(summary.warnings.iter().any(|warning| matches!(
        &warning.kind,
        WarningKind::PolicyWarning { violation }
            if violation.kind == ResourceKind::Image
                && violation.url.as_str() == "https://blocked.test/book/image.png"
    )));
    assert!(summary.warnings.iter().any(|warning| matches!(
        &warning.kind,
        WarningKind::PolicyWarning { violation }
            if violation.kind == ResourceKind::StylesheetImport
                && violation.url.as_str() == "https://blocked.test/book/reset.css"
    )));
    let denied_font = summary
        .warnings
        .iter()
        .find_map(|warning| match &warning.kind {
            WarningKind::PolicyWarning { violation } if violation.kind == ResourceKind::Font => {
                Some(violation)
            }
            _ => None,
        })
        .expect("font policy denial is reported");
    assert_eq!(
        denied_font.url.as_str(),
        "https://blocked.test/book/font.ttf"
    );
    assert!(summary.warnings.iter().any(|warning| matches!(
        warning.kind,
        WarningKind::ResourceFallback {
            kind: ResourceKind::Font,
            ..
        }
    )));
}
