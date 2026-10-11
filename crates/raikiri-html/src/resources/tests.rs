use super::*;
use raikiri_traits::{
    FetchOutcome, FetchedResource, ImagePixelSource, ImageRasterSize, NetworkError,
    NetworkProvider, Request, ResourceKind,
};
use std::sync::Mutex;

struct BoundedRedirectProbe {
    requests: Mutex<Vec<Url>>,
    redirects: Option<usize>,
    self_loop: bool,
}

impl NetworkProvider for BoundedRedirectProbe {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.url.clone());
        let index = requests.len() - 1;
        // Keep the regression finite even before the adapter is fixed.
        if index >= 32 {
            return Err(NetworkError::Other("probe safety stop".to_owned()));
        }
        if self.redirects == Some(index) {
            return Ok(FetchOutcome::Body(FetchedResource {
                bytes: b"body".as_slice().into(),
                content_type: None,
                final_url: request.url,
                encoding: None,
            }));
        }
        let location = if self.self_loop {
            request.url
        } else {
            Url::parse(&format!("https://redirect.test/{}", index + 1)).unwrap()
        };
        Ok(FetchOutcome::Redirect {
            location,
            status: 302,
        })
    }
}

fn fetch_redirect_probe(
    provider: &BoundedRedirectProbe,
    policy: Option<&dyn ResourcePolicy>,
    kind: ResourceKind,
) -> Result<FetchedResource, NetworkError> {
    ResourceNetworkProvider {
        inner: provider,
        policy,
        limits: ResourceLimits::default(),
        budget: Arc::new(Mutex::new(0)),
    }
    .fetch(Request {
        url: Url::parse("https://redirect.test/0").unwrap(),
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind,
    })
}

#[test]
fn policyless_redirects_bound_cycles_and_unique_url_chains() {
    for kind in [
        ResourceKind::ExternalStylesheet,
        ResourceKind::StylesheetImport,
        ResourceKind::Font,
        ResourceKind::Image,
    ] {
        for self_loop in [false, true] {
            let provider = BoundedRedirectProbe {
                requests: Mutex::new(Vec::new()),
                redirects: None,
                self_loop,
            };
            let error = fetch_redirect_probe(&provider, None, kind).unwrap_err();
            assert!(
                matches!(error, NetworkError::Other(message) if message == "too many redirects")
            );
            assert_eq!(provider.requests.lock().unwrap().len(), 11);
        }
    }
}

#[test]
fn policyless_redirects_preserve_zero_and_exact_limit_success() {
    for redirects in [0, 10] {
        let provider = BoundedRedirectProbe {
            requests: Mutex::new(Vec::new()),
            redirects: Some(redirects),
            self_loop: false,
        };
        let fetched = fetch_redirect_probe(&provider, None, ResourceKind::Font).unwrap();
        assert_eq!(fetched.bytes.as_ref(), b"body");
        assert_eq!(fetched.final_url.path(), format!("/{redirects}"));
        assert_eq!(provider.requests.lock().unwrap().len(), redirects + 1);
    }
}

struct RedirectProbePolicy {
    limit: u32,
    seen: Mutex<Vec<u32>>,
}

impl ResourcePolicy for RedirectProbePolicy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, hop: u32) -> bool {
        self.seen.lock().unwrap().push(hop);
        true
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        self.limit
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

#[test]
fn explicit_redirect_policy_preserves_limits_and_callback_hop_numbers() {
    for (limit, redirects, succeeds) in [
        (0, 1, false),
        (2, 3, false),
        (12, 12, true),
        (u32::MAX, 12, true),
    ] {
        let provider = BoundedRedirectProbe {
            requests: Mutex::new(Vec::new()),
            redirects: Some(redirects),
            self_loop: false,
        };
        let policy = RedirectProbePolicy {
            limit,
            seen: Mutex::new(Vec::new()),
        };
        let result = fetch_redirect_probe(&provider, Some(&policy), ResourceKind::Font);
        let taken = if succeeds {
            assert!(result.is_ok());
            redirects as u32
        } else {
            let NetworkError::PolicyViolation(violation) = result.unwrap_err() else {
                panic!("expected policy rejection");
            };
            assert!(matches!(
                violation.violation_type,
                ViolationType::RedirectDenied
            ));
            assert_eq!(violation.url.path(), format!("/{}", limit + 1));
            limit
        };
        assert_eq!(
            *policy.seen.lock().unwrap(),
            (1..=taken).collect::<Vec<_>>()
        );
        assert_eq!(provider.requests.lock().unwrap().len(), taken as usize + 1);
    }
}

const TWO_COLOR_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#00ff00"/></svg>"##;

#[derive(Default)]
struct SvgNetworkProvider {
    requests: Mutex<Vec<(Url, ResourceKind)>>,
}

impl NetworkProvider for SvgNetworkProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let final_url = request.url.clone();
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((request.url, request.kind));
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: TWO_COLOR_SVG.into(),
            content_type: Some("image/svg+xml".into()),
            final_url,
            encoding: None,
        }))
    }
}

#[test]
fn inside_marker_fetch_is_independent_of_background_preloading() {
    let provider = SvgNetworkProvider::default();
    let base = Url::parse("https://images.test/assets/document.html").unwrap();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .base_url(base.clone());
    let doc = crate::parse_html_with_resources(
        br#"<!doctype html><li style="list-style:inside url(marker.svg);background-image:url(unwanted.svg)">one</li>"#.as_slice(),
        &resources,
    ).unwrap();
    let crate::render::PipelineRun::Completed(output) = crate::render::run_pipeline(
        &doc,
        raikiri_traits::PageDefaults::default(),
        &raikiri_traits::LayoutConfig::default(),
        crate::render::PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
        },
    )
    .unwrap() else {
        panic!("complete pipeline");
    };
    assert_eq!(
        *provider.requests.lock().unwrap(),
        [(base.join("marker.svg").unwrap(), ResourceKind::Image)]
    );
    let item = output
        .cascade
        .computed
        .iter()
        .position(|cv| cv.display == DisplayValue::ListItem)
        .unwrap();
    assert!(output.document.list_marker_image(item).is_some());
}

#[test]
fn relative_background_urls_preload_against_the_document_base() {
    let provider = SvgNetworkProvider::default();
    let base = Url::parse("https://images.test/assets/document.html").unwrap();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .base_url(base.clone());
    let doc = crate::parse_html_with_resources(
        br#"<!doctype html><style>@page { background-image:url(page.svg) }</style><div style="background-image:url(img/element.svg)">one</div><p style="background-image:url(#frag)">two</p><p style="background-image:url('')">three</p>"#.as_slice(),
        &resources,
    ).unwrap();
    let crate::render::PipelineRun::Completed(_) = crate::render::run_pipeline(
        &doc,
        raikiri_traits::PageDefaults::default(),
        &raikiri_traits::LayoutConfig::default(),
        crate::render::PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    )
    .unwrap() else {
        panic!("complete pipeline");
    };
    let element = base.join("img/element.svg").unwrap();
    let page = base.join("page.svg").unwrap();
    assert_eq!(
        *provider.requests.lock().unwrap(),
        [
            (element.clone(), ResourceKind::Image),
            (page.clone(), ResourceKind::Image)
        ]
    );
    assert!(resources.intrinsic_size(&element).is_some());
    assert!(resources.intrinsic_size(&page).is_some());
}

/// Serves one stylesheet and an SVG for every other URL.
#[derive(Default)]
struct StylesheetAndSvgProvider {
    requests: Mutex<Vec<Url>>,
}

impl NetworkProvider for StylesheetAndSvgProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let final_url = request.url.clone();
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.url.clone());
        let (bytes, content_type) = if request.url.path().ends_with(".css") {
            (
                b"div { background-image: url(img/bg.svg) }".to_vec(),
                "text/css",
            )
        } else {
            (TWO_COLOR_SVG.to_vec(), "image/svg+xml")
        };
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes.into(),
            content_type: Some(content_type.into()),
            final_url,
            encoding: None,
        }))
    }
}

#[test]
fn stylesheet_background_urls_resolve_against_the_stylesheet() {
    let provider = StylesheetAndSvgProvider::default();
    let base = Url::parse("https://images.test/doc/index.html").unwrap();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .base_url(base.clone());
    let doc = crate::parse_html_with_resources(
        br#"<!doctype html><link rel=stylesheet href="../css/print.css"><div>one</div>"#.as_slice(),
        &resources,
    )
    .unwrap();
    let crate::render::PipelineRun::Completed(_) = crate::render::run_pipeline(
        &doc,
        raikiri_traits::PageDefaults::default(),
        &raikiri_traits::LayoutConfig::default(),
        crate::render::PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    )
    .unwrap() else {
        panic!("complete pipeline");
    };
    let image = Url::parse("https://images.test/css/img/bg.svg").unwrap();
    assert!(provider.requests.lock().unwrap().contains(&image));
    assert!(resources.intrinsic_size(&image).is_some());
}

#[test]
fn suppressed_marker_images_preserve_the_shared_background_request_budget() {
    let provider = SvgNetworkProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let mut html = String::from(
        r#"<!doctype html><style>
        li { list-style-position:inside }
        .hidden::marker { display:none }
        .none::marker { content:none }
        .text::marker { content:'custom' }
        .empty::marker { content:'' }
        div { background-image:url(https://images.test/element.svg) }
        @page { background-image:url(https://images.test/page.svg) }
        </style>"#,
    );
    let classes = ["hidden", "none", "text", "empty"];
    for index in 0..MAX_BACKGROUND_IMAGE_ATTEMPTS {
        html.push_str(&format!(
            r#"<li class="{}" style="list-style-image:url(https://images.test/suppressed-{index}.svg)"></li>"#,
            classes[index % classes.len()]
        ));
    }
    html.push_str(
        r#"<li style="list-style-image:url(https://images.test/marker.svg)"></li><div></div>"#,
    );
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = crate::parse(html.as_bytes(), &options).unwrap();
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = Default::default();
    let mut attempts = 0;
    resources.preload_list_marker_images(&cascade, None, &warnings, &mut seen, &mut attempts, None);
    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);
    assert_eq!(attempts, 3);
    assert_eq!(
        *provider.requests.lock().unwrap(),
        ["marker.svg", "element.svg", "page.svg"].map(|name| (
            Url::parse(&format!("https://images.test/{name}")).unwrap(),
            ResourceKind::Image
        ))
    );
    assert!(warnings.lock().unwrap().is_empty());
}

#[test]
fn render_preloads_only_effective_marker_images() {
    let provider = SvgNetworkProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let html = br#"<!doctype html><style>
        li { list-style:inside url(https://images.test/unused.svg) }
        .hidden::marker { display:none }
        .none::marker { content:none }
        .text::marker { content:'custom' }
        .empty::marker { content:'' }
        .visible { list-style-image:url(https://images.test/marker.svg) }
        div { background-image:url(https://images.test/element.svg) }
        @page { background-image:url(https://images.test/page.svg) }
        </style><li class="hidden"></li><li class="none"></li>
        <li class="text"></li><li class="empty"></li><li class="visible"></li><div></div>"#;
    let doc = crate::parse_html_with_resources(html.as_slice(), &resources).unwrap();
    let crate::render::PipelineRun::Completed(_) = crate::render::run_pipeline(
        &doc,
        raikiri_traits::PageDefaults::default(),
        &raikiri_traits::LayoutConfig::default(),
        crate::render::PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
        },
    )
    .unwrap() else {
        panic!("complete pipeline");
    };
    assert_eq!(
        *provider.requests.lock().unwrap(),
        ["marker.svg", "element.svg", "page.svg"].map(|name| (
            Url::parse(&format!("https://images.test/{name}")).unwrap(),
            ResourceKind::Image
        ))
    );
}

#[test]
fn relative_inside_and_outside_marker_images_are_fetched_once_before_layout() {
    let provider = SvgNetworkProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let base = Url::parse("https://images.test/assets/document.html").unwrap();
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: Some(base.clone()),
    };
    let html = br#"<!doctype html><style>li {list-style:inside url(marker.svg)}</style><li>one</li><li>two</li><li style="list-style-image:none">text</li><li style="list-style-position:outside;list-style-image:url(outside.svg)">outside</li>"#;
    let mut uncascaded = crate::parse(&html[..], &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.preload_list_marker_images(
        &cascade,
        Some(&base),
        &warnings,
        &mut Default::default(),
        &mut 0,
        None,
    );
    uncascaded
        .dom
        .prepare_list_marker_images(&cascade, &resources, Some(&base));
    assert_eq!(
        *provider.requests.lock().unwrap(),
        ["marker.svg", "outside.svg"].map(|name| (base.join(name).unwrap(), ResourceKind::Image))
    );
    let item = cascade
        .computed
        .iter()
        .position(|cv| cv.display == DisplayValue::ListItem)
        .unwrap();
    let image = uncascaded
        .dom
        .list_marker_image(item)
        .expect("prepared image");
    assert_eq!((image.width, image.height), (2, 1));
    let outside = cascade
        .computed
        .iter()
        .enumerate()
        .find(|(_, cv)| {
            cv.display == DisplayValue::ListItem
                && cv.list_style_position == raikiri_style::ListStylePosition::Outside
        })
        .unwrap()
        .0;
    assert_eq!(
        uncascaded.dom.list_marker_image_url(outside),
        Some(&base.join("outside.svg").unwrap())
    );
    assert!(uncascaded.dom.list_marker_image(outside).is_some());
}

#[test]
fn viewbox_only_svg_inside_marker_uses_one_em_default_size() {
    let resources = RenderResources::new();
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = br#"<!doctype html><li style="font-size:16px;list-style:inside url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 10 10%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22green%22/%3E%3C/svg%3E')">one</li>"#;
    let mut uncascaded = crate::parse(&html[..], &options).unwrap();
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    resources.preload_list_marker_images(
        &cascade,
        None,
        &Arc::new(Mutex::new(Vec::new())),
        &mut Default::default(),
        &mut 0,
        None,
    );
    uncascaded
        .dom
        .prepare_list_marker_images(&cascade, &resources, None);
    let item = cascade
        .computed
        .iter()
        .position(|cv| cv.display == DisplayValue::ListItem)
        .unwrap();
    let image = uncascaded
        .dom
        .list_marker_image(item)
        .expect("rasterized SVG marker");
    assert_eq!((image.width, image.height), (16, 16));
    assert_eq!(
        uncascaded.dom.list_marker_image_size(item),
        Some(ImageRasterSize {
            width: 16.0,
            height: 16.0
        })
    );
}

#[test]
fn preloads_an_absolute_svg_background_once_and_rasterizes_on_demand() {
    let provider = SvgNetworkProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = br#"<!doctype html><style>div { background-image: url("https://images.test/two-color.svg"); }</style><body><div></div><div></div></body>"#;
    let uncascaded = crate::parse(&html[..], &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;

    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);

    let url = Url::parse("https://images.test/two-color.svg").unwrap();
    let intrinsic = resources
        .intrinsic_size(&url)
        .expect("preloaded SVG retains natural metadata");
    let image = resources
        .get_decoded_at_size(
            &url,
            ImageRasterSize {
                width: 4.0,
                height: 2.0,
            },
            None,
        )
        .expect("preloaded SVG rasterizes at the concrete background size");
    let requests = provider
        .requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    assert_eq!(requests.len(), 1, "duplicate background URLs fetch once");
    assert_eq!(requests[0].0, url);
    assert_eq!(requests[0].1, ResourceKind::Image);
    assert_eq!(intrinsic.width, Some(2.0));
    assert_eq!(intrinsic.height, Some(1.0));
    assert_eq!(intrinsic.aspect_ratio, Some(2.0));
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.rgba[8..12], &[0, 255, 0, 255]);
    assert!(
        warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn data_svg_background_is_available_from_the_combined_source_without_network() {
    let resources = RenderResources::new();
    let source = resources
        .image_pixel_source_ref()
        .expect("the combined source exposes its internal cache before preload");
    let data_url = "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='1'%20height='1'%3E%3Crect%20width='1'%20height='1'%20fill='red'/%3E%3C/svg%3E";
    let html = format!(
        "<!doctype html><style>div{{background-image:url(\"{data_url}\")}}</style><body><div></div></body>"
    );
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = crate::parse(html.as_bytes(), &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;

    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);
    let url = Url::parse(data_url).expect("data URL parses");
    let image = source
        .get_decoded_at_size(
            &url,
            ImageRasterSize {
                width: 1.0,
                height: 1.0,
            },
            None,
        )
        .expect("the combined source reads the preloaded data URL");

    assert_eq!(&image.rgba, &[255, 0, 0, 255]);
}

struct AllowAllPolicy;

impl raikiri_traits::ResourcePolicy for AllowAllPolicy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        true
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        10
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

#[test]
fn resource_limits_builders_pin_defaults_and_opt_out() {
    let defaults = ResourceLimits::new();
    assert_eq!(defaults, ResourceLimits::default());
    assert!(defaults.max_resource_bytes.is_some());
    assert!(defaults.max_aggregate_resource_bytes.is_some());

    let disabled = ResourceLimits::new()
        .max_resource_bytes(None)
        .max_aggregate_resource_bytes(None);
    assert_eq!(disabled.max_resource_bytes, None);
    assert_eq!(disabled.max_aggregate_resource_bytes, None);

    let custom = ResourceLimits::new()
        .max_resource_bytes(Some(1024))
        .max_aggregate_resource_bytes(Some(4096));
    assert_eq!(custom.max_resource_bytes, Some(1024));
    assert_eq!(custom.max_aggregate_resource_bytes, Some(4096));
}

#[test]
fn render_resources_builder_chain_surfaces_in_debug() {
    let provider = SvgNetworkProvider::default();
    let policy = AllowAllPolicy;
    let resources = RenderResources::new()
        .stylesheet("p { color: red; }")
        .stylesheet("div { color: blue; }")
        .network_provider(&provider)
        .network_policy(&policy)
        .base_url(Url::parse("https://example.com/base/").unwrap())
        .render_limits(
            raikiri_traits::RenderLimitsBuilder::default()
                .max_document_pages(Some(5))
                .build(),
        )
        .resource_limits(ResourceLimits::new().max_resource_bytes(Some(1024)));
    let debug = format!("{resources:?}");
    assert!(debug.contains("extra_stylesheet_count: 2"), "{debug}");
    assert!(debug.contains("has_network_provider: true"), "{debug}");
    assert!(debug.contains("has_network_policy: true"), "{debug}");
    assert!(debug.contains("example.com"), "{debug}");
}

#[test]
fn render_resources_default_matches_new() {
    let debug_new = format!("{:?}", RenderResources::new());
    let debug_default = format!("{:?}", RenderResources::default());
    assert_eq!(debug_new, debug_default);
}

struct StubImageProvider;

impl raikiri_traits::ReplacedResolver for StubImageProvider {
    fn resolve(
        &self,
        _req: raikiri_traits::ResolverRequest<'_>,
    ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
        Ok(raikiri_traits::ResolvedIntrinsic {
            intrinsic: raikiri_traits::IntrinsicBox::new(2.0, 1.0),
            disposition: raikiri_traits::ResolveDisposition::Ok,
        })
    }
}

impl raikiri_traits::ImagePixelSource for StubImageProvider {
    fn get_decoded(&self, _url: &Url) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
        None
    }
}

#[test]
fn replaced_resolver_builder_retains_resolver_only() {
    let provider = StubImageProvider;
    let resources = RenderResources::new().replaced_resolver(&provider);
    let debug = format!("{resources:?}");
    assert!(debug.contains("has_replaced_resolver: true"), "{debug}");
    assert!(debug.contains("has_image_pixel_source: false"), "{debug}");
}

#[test]
fn replaced_resource_provider_retains_both_roles() {
    let provider = StubImageProvider;
    let resources = RenderResources::new().replaced_resource_provider(&provider);
    let debug = format!("{resources:?}");
    assert!(debug.contains("has_replaced_resolver: true"), "{debug}");
    assert!(debug.contains("has_image_pixel_source: true"), "{debug}");
}

#[test]
fn image_pixel_source_builder_and_font_collection_ref() {
    let provider = StubImageProvider;
    let resources = RenderResources::new().image_pixel_source(&provider);
    let debug = format!("{resources:?}");
    assert!(debug.contains("has_image_pixel_source: true"), "{debug}");
    assert!(resources.font_collection_ref().is_none());
    let resources = resources.fonts(ahem_fonts(false));
    assert!(resources.font_collection_ref().is_some());
}

use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use raikiri_traits::{AbortController, DecodedImage};

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = image::DynamicImage::new_rgba8(width, height);
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("PNG encodes");
    out
}

struct CountingPngProvider {
    calls: AtomicUsize,
    delay: Duration,
    bytes: Vec<u8>,
    seen_signals: Mutex<Vec<bool>>,
}

impl CountingPngProvider {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(0),
            bytes,
            seen_signals: Mutex::new(Vec::new()),
        }
    }

    fn with_delay(bytes: Vec<u8>, delay: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            delay,
            bytes,
            seen_signals: Mutex::new(Vec::new()),
        }
    }
}

impl NetworkProvider for CountingPngProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen_signals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.signal.is_some());
        if request
            .signal
            .as_ref()
            .is_some_and(|signal| signal.is_aborted())
        {
            return Err(NetworkError::Aborted);
        }
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if request
            .signal
            .as_ref()
            .is_some_and(|signal| signal.is_aborted())
        {
            return Err(NetworkError::Aborted);
        }
        let final_url = request.url.clone();
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: self.bytes.clone().into(),
            content_type: Some("image/png".into()),
            final_url,
            encoding: None,
        }))
    }
}

struct ZeroDecodeTimeoutPolicy;

impl raikiri_traits::ResourcePolicy for ZeroDecodeTimeoutPolicy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        true
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        10
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(0)
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

struct TinyDecodedPolicy {
    limit: u64,
}

impl raikiri_traits::ResourcePolicy for TinyDecodedPolicy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        true
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        10
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64> {
        if kind == ResourceKind::Image {
            Some(self.limit)
        } else {
            None
        }
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

#[test]
fn process_budget_try_acquire_bounds_concurrency() {
    let budget = Arc::new(ProcessImageBudget::new(1024, 2));
    assert_eq!(budget.max_retained(), 1024);
    assert_eq!(budget.retained(), 0);
    let first = budget.try_acquire().expect("first permit available");
    let second = budget.try_acquire().expect("second permit available");
    assert!(
        budget.try_acquire().is_none(),
        "third blocks when both held"
    );
    drop(first);
    assert!(budget.try_acquire().is_some(), "release frees one slot");
    drop(second);
}

#[test]
fn process_budget_blocking_acquire_waits_for_release() {
    let budget = Arc::new(ProcessImageBudget::new(1024, 1));
    let held = budget.try_acquire().expect("single slot available");
    let waiting = Arc::clone(&budget);
    let done = Arc::new(AtomicUsize::new(0));
    let done_clone = Arc::clone(&done);
    let handle = std::thread::spawn(move || {
        let _permit = waiting.acquire();
        done_clone.fetch_add(1, Ordering::SeqCst);
    });
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(done.load(Ordering::SeqCst), 0, "blocked while slot held");
    drop(held);
    handle.join().expect("waiter proceeds after release");
    assert_eq!(done.load(Ordering::SeqCst), 1);
}

#[test]
fn same_url_concurrent_fetches_share_one_provider_request() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::with_delay(bytes, Duration::from_millis(150));
    let resources = RenderResources::new().network_provider(&provider);
    let url = Url::parse("https://images.test/shared.png").unwrap();
    let barrier = Arc::new(Barrier::new(4));
    let resources_ref = &resources;
    let warning_counts = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..4 {
            let barrier = Arc::clone(&barrier);
            let url = url.clone();
            handles.push(scope.spawn(move || {
                barrier.wait();
                let warnings = Arc::new(Mutex::new(Vec::new()));
                resources_ref.fetch_background_image(url, &warnings, None);
                warnings
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len()
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("fetch thread joins"))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "concurrent same-URL renders share one provider request"
    );
    assert!(
        warning_counts.iter().all(|count| *count == 0),
        "shared success reports no warnings, got {warning_counts:?}"
    );
    assert!(
        resources
            .cached_background_image(&image_url_without_fragment(&url))
            .is_some(),
        "shared URL is cached once"
    );
}

/// A fresh process budget with the production limits. Tests that hold or
/// inspect decoder slots use their own budget: the process-wide one is shared
/// by every test running in parallel in this binary, so another test's decode
/// could otherwise hold a slot these tests expect to be free.
fn isolated_process_budget() -> Arc<ProcessImageBudget> {
    Arc::new(ProcessImageBudget::new(
        PROCESS_MAX_RETAINED_DECODED_BYTES,
        PROCESS_MAX_CONCURRENT_IMAGE_DECODES,
    ))
}

#[test]
fn unrelated_urls_proceed_without_global_serialization() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::new(bytes);
    let resources = RenderResources::new()
        .network_provider(&provider)
        .with_test_process_budget(isolated_process_budget());
    let held: Vec<_> = (0..3)
        .map(|_| resources.process_budget.try_acquire().expect("slot free"))
        .collect();
    let url_a = Url::parse("https://images.test/a.png").unwrap();
    let url_b = Url::parse("https://images.test/b.png").unwrap();
    let warnings_a = Arc::new(Mutex::new(Vec::new()));
    let warnings_b = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url_a.clone(), &warnings_a, None);
    resources.fetch_background_image(url_b.clone(), &warnings_b, None);
    drop(held);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(
        warnings_a
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
    assert!(
        warnings_b
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
    assert!(resources.cached_background_image(&url_a).is_some());
    assert!(resources.cached_background_image(&url_b).is_some());
}

#[test]
fn process_retained_bound_spans_independent_contexts() {
    let small = png_bytes(1, 1);
    let large = png_bytes(32, 32);
    let provider_small = CountingPngProvider::new(small);
    let provider_large = CountingPngProvider::new(large);
    let shared = Arc::new(ProcessImageBudget::new(200, 4));
    let first = RenderResources::new()
        .network_provider(&provider_small)
        .with_test_process_budget(Arc::clone(&shared));
    let second = RenderResources::new()
        .network_provider(&provider_large)
        .with_test_process_budget(Arc::clone(&shared));
    let first_url = Url::parse("https://images.test/first.png").unwrap();
    let second_url = Url::parse("https://images.test/second.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    first.fetch_background_image(first_url.clone(), &warnings, None);
    assert!(first.cached_background_image(&first_url).is_some());
    let retained_after_first = shared.retained();
    assert!(
        retained_after_first > 0,
        "first insert retains process bytes"
    );
    let warnings_second = Arc::new(Mutex::new(Vec::new()));
    second.fetch_background_image(second_url.clone(), &warnings_second, None);
    assert!(
        second.cached_background_image(&second_url).is_none(),
        "process cap rejects the second independent context"
    );
    let denied = warnings_second
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        denied.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "process-wide denial surfaces as a limit warning, got {denied:?}"
    );
    assert_eq!(shared.retained(), retained_after_first);
}

#[test]
fn cache_insert_paths_cover_per_context_and_replace_branches() {
    let budget = Arc::new(ProcessImageBudget::new(1024 * 1024, 4));
    let mut cache = DecodedImageCache::new(Arc::clone(&budget));
    let url = Url::parse("https://images.test/cache.png").unwrap();
    assert_eq!(cache.prior_bytes(&url), 0);
    assert_eq!(cache.remaining_for(&url), 128 * 1024 * 1024);
    let small = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 0],
    });
    cache
        .insert_raster(
            url.clone(),
            CachedBackgroundSource::Raster(Arc::clone(&small)),
            ImageRasterSize {
                width: 1.0,
                height: 1.0,
            },
            Arc::clone(&small),
        )
        .expect("small insert fits");
    assert_eq!(cache.prior_bytes(&url), 4);
    assert_eq!(budget.retained(), 4);
    let tiny = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![1, 2, 3, 4],
    });
    cache
        .insert_raster(
            url.clone(),
            CachedBackgroundSource::Raster(Arc::clone(&tiny)),
            ImageRasterSize {
                width: 1.0,
                height: 1.0,
            },
            Arc::clone(&tiny),
        )
        .expect("same-size replace succeeds");
    assert_eq!(budget.retained(), 4);
    cache.insert_source(
        url.clone(),
        CachedBackgroundSource::Svg(Arc::new(
            SvgDocument::parse(TWO_COLOR_SVG).expect("fixture SVG parses"),
        )),
    );
    assert_eq!(cache.prior_bytes(&url), 0);
    assert_eq!(budget.retained(), 0);
    cache.insert_source(
        Url::parse("https://images.test/fresh.svg").unwrap(),
        CachedBackgroundSource::Svg(Arc::new(
            SvgDocument::parse(TWO_COLOR_SVG).expect("fixture SVG parses"),
        )),
    );
    assert_eq!(budget.retained(), 0);
    let mut tight = DecodedImageCache {
        entries: HashMap::new(),
        decoded_bytes: 0,
        max_bytes: 10,
        process_budget: Arc::new(ProcessImageBudget::new(1024 * 1024, 4)),
    };
    let big = Arc::new(DecodedImage {
        width: 4,
        height: 4,
        rgba: vec![0; 64],
    });
    assert!(
        tight
            .insert_raster(
                url,
                CachedBackgroundSource::Raster(Arc::clone(&big)),
                ImageRasterSize {
                    width: 4.0,
                    height: 4.0,
                },
                big,
            )
            .is_err(),
        "per-context cap rejects an oversized raster"
    );
}

#[test]
fn reconfigured_clones_keep_cache_and_policy_isolation() {
    let bytes = png_bytes(1, 1);
    let provider_a = CountingPngProvider::new(bytes.clone());
    let provider_b = CountingPngProvider::new(bytes);
    let base = RenderResources::new().network_provider(&provider_a);
    let url = Url::parse("https://images.test/isolated.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    base.fetch_background_image(url.clone(), &warnings, None);
    assert_eq!(provider_a.calls.load(Ordering::SeqCst), 1);
    let reconfigured = base.clone().network_provider(&provider_b);
    let warnings_b = Arc::new(Mutex::new(Vec::new()));
    reconfigured.fetch_background_image(url.clone(), &warnings_b, None);
    assert_eq!(
        provider_b.calls.load(Ordering::SeqCst),
        1,
        "reconfigured clone does not reuse the prior cache"
    );
    assert_eq!(provider_a.calls.load(Ordering::SeqCst), 1);
    assert!(base.cached_background_image(&url).is_some());
    assert!(reconfigured.cached_background_image(&url).is_some());
    struct DenyPolicy;
    impl raikiri_traits::ResourcePolicy for DenyPolicy {
        fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
            false
        }
        fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
            true
        }
        fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
            true
        }
        fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
            10
        }
        fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
            None
        }
        fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
            None
        }
        fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
            Duration::from_secs(10)
        }
        fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
            Duration::from_secs(10)
        }
        fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
            Vec::new()
        }
        fn max_import_depth(&self) -> u32 {
            10
        }
        fn max_svg_recursion_depth(&self) -> u32 {
            10
        }
    }
    let deny = DenyPolicy;
    let denied = base.clone().network_policy(&deny);
    let denied_url = Url::parse("https://images.test/policy.png").unwrap();
    let denied_warnings = Arc::new(Mutex::new(Vec::new()));
    denied.fetch_background_image(denied_url.clone(), &denied_warnings, None);
    assert!(denied.cached_background_image(&denied_url).is_none());
    assert!(
        denied_warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|warning| matches!(warning.kind, WarningKind::PolicyWarning { .. }))
    );
}

#[test]
fn decode_timeout_zero_discards_and_reports_policy_timeout() {
    let bytes = png_bytes(2, 2);
    let provider = CountingPngProvider::new(bytes);
    let policy = ZeroDecodeTimeoutPolicy;
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy)
        .with_test_process_budget(isolated_process_budget());
    let url = Url::parse("https://images.test/timeout.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            &warning.kind,
            WarningKind::PolicyWarning { violation }
                if matches!(violation.violation_type, ViolationType::Timeout)
        )),
        "zero decode timeout surfaces as a Timeout policy warning, got {seen:?}"
    );
    assert!(
        resources.process_budget.try_acquire().is_some(),
        "timed-out work releases its decoder slot"
    );
}

#[test]
fn tiny_decoded_policy_reports_decoded_too_large() {
    let bytes = png_bytes(8, 8);
    let provider = CountingPngProvider::new(bytes);
    let policy = TinyDecodedPolicy { limit: 10 };
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy);
    let url = Url::parse("https://images.test/too-large.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "policy decoded cap surfaces as a limit warning, got {seen:?}"
    );
}

#[test]
fn invalid_raster_bytes_report_fallback_without_timeout() {
    let provider = CountingPngProvider::new(b"not an image".to_vec());
    let resources = RenderResources::new().network_provider(&provider);
    let url = Url::parse("https://images.test/broken.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "undecodable bytes surface as a fallback, got {seen:?}"
    );
}

#[test]
fn svg_input_limit_and_parse_failure_report_warnings() {
    let oversized = vec![b'a'; 33 * 1024 * 1024];
    let provider = CountingPngProvider::new(oversized);
    let resources = RenderResources::new()
        .network_provider(&provider)
        .resource_limits(
            ResourceLimits::new()
                .max_resource_bytes(None)
                .max_aggregate_resource_bytes(None),
        );
    let big_url = Url::parse("https://images.test/big.svg").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(big_url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&big_url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "oversized SVG input surfaces as a limit, got {seen:?}"
    );
    let broken_provider = CountingPngProvider::new(b"not svg".to_vec());
    let broken_resources = RenderResources::new().network_provider(&broken_provider);
    let broken_url = Url::parse("https://images.test/broken.svg").unwrap();
    let broken_warnings = Arc::new(Mutex::new(Vec::new()));
    broken_resources.fetch_background_image(broken_url.clone(), &broken_warnings, None);
    assert!(
        broken_resources
            .cached_background_image(&broken_url)
            .is_none()
    );
    let broken_seen = broken_warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        broken_seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "unparsable SVG surfaces as a fallback, got {broken_seen:?}"
    );
}

#[test]
fn svg_zero_timeout_reports_timeout_after_parse() {
    let provider = SvgNetworkProvider::default();
    let policy = ZeroDecodeTimeoutPolicy;
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy);
    let url = Url::parse("https://images.test/timeout.svg").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            &warning.kind,
            WarningKind::PolicyWarning { violation }
                if matches!(violation.violation_type, ViolationType::Timeout)
        )),
        "SVG parse exceeding zero timeout reports Timeout, got {seen:?}"
    );
}

#[test]
fn aborted_signal_skips_provider_and_reports_fallback() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::new(bytes);
    let resources = RenderResources::new().network_provider(&provider);
    let controller = AbortController::new();
    controller.abort();
    let url = Url::parse("https://images.test/aborted.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, Some(&controller.signal));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "aborted fetch surfaces as a fallback, got {seen:?}"
    );
}

#[test]
fn abort_during_fetch_reports_abort_without_caching() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::with_delay(bytes, Duration::from_millis(200));
    let resources = RenderResources::new().network_provider(&provider);
    let controller = AbortController::new();
    let url = Url::parse("https://images.test/abort-mid.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            resources.fetch_background_image(url.clone(), &warnings, Some(&controller.signal));
        });
        std::thread::sleep(Duration::from_millis(50));
        controller.abort();
        handle.join().expect("aborted fetch joins");
    });
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        !seen.is_empty(),
        "mid-fetch abort still records a warning, got {seen:?}"
    );
}

#[test]
fn abort_during_decode_wait_reports_abort() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::with_delay(bytes, Duration::from_millis(50));
    let resources = RenderResources::new()
        .network_provider(&provider)
        .with_test_process_budget(isolated_process_budget());
    let held = resources.process_budget.try_acquire().expect("slot free");
    let held2 = resources.process_budget.try_acquire().expect("slot free");
    let held3 = resources.process_budget.try_acquire().expect("slot free");
    let held4 = resources.process_budget.try_acquire().expect("slot free");
    let controller = AbortController::new();
    let signal = controller.signal.clone();
    let url = Url::parse("https://images.test/abort-wait.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        scope.spawn(|| {
            resources.fetch_background_image(url.clone(), &warnings, Some(&signal));
        });
        std::thread::sleep(Duration::from_millis(20));
        controller.abort();
        drop(held);
        drop(held2);
        drop(held3);
        drop(held4);
    });
    assert!(resources.cached_background_image(&url).is_none());
}

#[test]
fn signal_is_forwarded_to_provider() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::new(bytes);
    let resources = RenderResources::new().network_provider(&provider);
    let controller = AbortController::new();
    let url = Url::parse("https://images.test/signal.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, Some(&controller.signal));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let seen = provider
        .seen_signals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        seen,
        vec![true],
        "background requests carry the abort signal"
    );
    assert!(resources.cached_background_image(&url).is_some());
}

#[test]
fn svg_raster_timeout_returns_none_without_caching_raster() {
    let provider = SvgNetworkProvider::default();
    let policy = ZeroDecodeTimeoutPolicy;
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy);
    let url = Url::parse("https://images.test/raster-timeout.svg").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = br#"<!doctype html><style>div { background-image: url("https://images.test/raster-timeout.svg"); }</style><body><div></div></body>"#;
    let uncascaded = crate::parse(&html[..], &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;
    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);
    assert!(resources.cached_background_image(&url).is_none());
    let direct_provider = SvgNetworkProvider::default();
    let direct = RenderResources::new().network_provider(&direct_provider);
    let direct_warnings = Arc::new(Mutex::new(Vec::new()));
    let mut direct_seen = std::collections::HashSet::new();
    let mut direct_attempts = 0;
    direct.preload_background_images(
        &cascade,
        &direct_warnings,
        &mut direct_seen,
        &mut direct_attempts,
        None,
    );
    let raster = direct.get_decoded_at_size(
        &url,
        ImageRasterSize {
            width: 4.0,
            height: 2.0,
        },
        None,
    );
    assert!(raster.is_some(), "without a zero timeout SVG rasterizes");
    let timed = resources.get_decoded_at_size(
        &url,
        ImageRasterSize {
            width: 4.0,
            height: 2.0,
        },
        None,
    );
    assert!(
        timed.is_none(),
        "zero decode timeout refuses the paint-time raster"
    );
}

#[test]
fn preload_abort_breaks_without_provider_calls() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::new(bytes);
    let resources = RenderResources::new().network_provider(&provider);
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = br#"<!doctype html><style>div { background-image: url("https://images.test/preload-abort.png"); }</style><body><div></div></body>"#;
    let uncascaded = crate::parse(&html[..], &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let controller = AbortController::new();
    controller.abort();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;
    resources.preload_background_images(
        &cascade,
        &warnings,
        &mut seen,
        &mut attempts,
        Some(&controller.signal),
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_provider_reports_network_fallback() {
    let resources = RenderResources::new();
    let url = Url::parse("https://images.test/no-provider.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter()
            .any(|warning| matches!(warning.kind, WarningKind::NetworkFallback { .. })),
        "missing provider surfaces as NetworkFallback, got {seen:?}"
    );
}

#[test]
fn fetch_policy_denial_replicates_to_waiters() {
    struct DenyFetchPolicy;
    impl raikiri_traits::ResourcePolicy for DenyFetchPolicy {
        fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
            true
        }
        fn is_host_allowed(&self, _host: &str, _kind: ResourceKind) -> bool {
            true
        }
        fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
            false
        }
        fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
            0
        }
        fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
            Some(1)
        }
        fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
            None
        }
        fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
            Duration::from_secs(10)
        }
        fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
            Duration::from_secs(10)
        }
        fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
            Vec::new()
        }
        fn max_import_depth(&self) -> u32 {
            10
        }
        fn max_svg_recursion_depth(&self) -> u32 {
            10
        }
    }
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::with_delay(bytes, Duration::from_millis(100));
    let policy = DenyFetchPolicy;
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy);
    let url = Url::parse("https://images.test/denied.png").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let resources_ref = &resources;
    let all = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let barrier = Arc::clone(&barrier);
            let url = url.clone();
            handles.push(scope.spawn(move || {
                barrier.wait();
                let warnings = Arc::new(Mutex::new(Vec::new()));
                resources_ref.fetch_background_image(url, &warnings, None);
                warnings
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("denied fetch joins"))
            .collect::<Vec<_>>()
    });
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    for warnings in &all {
        assert!(
            warnings.iter().any(|warning| matches!(
                warning.kind,
                WarningKind::ResourceLimitExceeded {
                    kind: ResourceKind::Image,
                    ..
                }
            )),
            "both owner and waiter report the limit, got {warnings:?}"
        );
    }
}

struct FailingProvider;

impl NetworkProvider for FailingProvider {
    fn fetch_one_hop(&self, _request: Request) -> Result<FetchOutcome, NetworkError> {
        Err(NetworkError::Other("boom".to_owned()))
    }
}

#[test]
fn failing_provider_reports_network_fallback() {
    let provider = FailingProvider;
    let resources = RenderResources::new().network_provider(&provider);
    let url = Url::parse("https://images.test/failing.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter()
            .any(|warning| matches!(warning.kind, WarningKind::NetworkFallback { .. })),
        "failing provider surfaces as NetworkFallback, got {seen:?}"
    );
}

#[test]
fn sequential_second_fetch_hits_fast_path_without_provider_call() {
    let bytes = png_bytes(1, 1);
    let provider = CountingPngProvider::new(bytes);
    let resources = RenderResources::new().network_provider(&provider);
    let url = Url::parse("https://images.test/sequential.png").unwrap();
    let first_warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &first_warnings, None);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let second_warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &second_warnings, None);
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "cached second fetch issues no provider request"
    );
    assert!(
        second_warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn data_abort_before_decode_reports_fallback() {
    let resources = RenderResources::new();
    let controller = AbortController::new();
    controller.abort();
    let data_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let url = Url::parse(data_url).expect("data URL parses");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, Some(&controller.signal));
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                ..
            }
        )),
        "aborted data decode surfaces as a fallback, got {seen:?}"
    );
}

#[test]
fn cache_smaller_replace_releases_process_bytes() {
    let budget = Arc::new(ProcessImageBudget::new(1024 * 1024, 4));
    let mut cache = DecodedImageCache::new(Arc::clone(&budget));
    let url = Url::parse("https://images.test/shrink.png").unwrap();
    let big = Arc::new(DecodedImage {
        width: 4,
        height: 4,
        rgba: vec![0; 64],
    });
    cache
        .insert_raster(
            url.clone(),
            CachedBackgroundSource::Raster(Arc::clone(&big)),
            ImageRasterSize {
                width: 4.0,
                height: 4.0,
            },
            Arc::clone(&big),
        )
        .expect("big insert fits");
    assert_eq!(budget.retained(), 64);
    let small = Arc::new(DecodedImage {
        width: 1,
        height: 1,
        rgba: vec![0; 4],
    });
    cache
        .insert_raster(
            url.clone(),
            CachedBackgroundSource::Raster(Arc::clone(&small)),
            ImageRasterSize {
                width: 1.0,
                height: 1.0,
            },
            small,
        )
        .expect("smaller replace fits");
    assert_eq!(budget.retained(), 4);
    assert_eq!(cache.prior_bytes(&url), 4);
}

#[test]
fn concurrent_inserts_bound_process_retained_at_insert() {
    let shared = Arc::new(ProcessImageBudget::new(100, 4));
    let first_bytes = png_bytes(4, 4);
    let second_bytes = png_bytes(4, 4);
    let first_provider = CountingPngProvider::with_delay(first_bytes, Duration::from_millis(80));
    let second_provider = CountingPngProvider::with_delay(second_bytes, Duration::from_millis(80));
    let first = RenderResources::new()
        .network_provider(&first_provider)
        .with_test_process_budget(Arc::clone(&shared));
    let second = RenderResources::new()
        .network_provider(&second_provider)
        .with_test_process_budget(Arc::clone(&shared));
    let first_url = Url::parse("https://images.test/concurrent-a.png").unwrap();
    let second_url = Url::parse("https://images.test/concurrent-b.png").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let first_ref = &first;
    let second_ref = &second;
    std::thread::scope(|scope| {
        let barrier_a = Arc::clone(&barrier);
        let handle_a = scope.spawn(move || {
            barrier_a.wait();
            let warnings = Arc::new(Mutex::new(Vec::new()));
            first_ref.fetch_background_image(first_url, &warnings, None);
            warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        let barrier_b = Arc::clone(&barrier);
        let handle_b = scope.spawn(move || {
            barrier_b.wait();
            let warnings = Arc::new(Mutex::new(Vec::new()));
            second_ref.fetch_background_image(second_url, &warnings, None);
            warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        let first_warnings = handle_a.join().expect("first concurrent joins");
        let second_warnings = handle_b.join().expect("second concurrent joins");
        let total_cached = usize::from(
            first
                .cached_background_image(
                    &Url::parse("https://images.test/concurrent-a.png").unwrap(),
                )
                .is_some(),
        ) + usize::from(
            second
                .cached_background_image(
                    &Url::parse("https://images.test/concurrent-b.png").unwrap(),
                )
                .is_some(),
        );
        assert!(
            total_cached >= 1,
            "at least one concurrent insert retains, got {total_cached}"
        );
        let _ = (first_warnings, second_warnings);
    });
    assert!(
        shared.retained() <= 100,
        "process retained never exceeds the cap, got {}",
        shared.retained()
    );
}

#[test]
fn invalid_bytes_with_zero_timeout_reports_timeout() {
    let provider = CountingPngProvider::new(b"not an image".to_vec());
    let policy = ZeroDecodeTimeoutPolicy;
    let resources = RenderResources::new()
        .network_provider(&provider)
        .network_policy(&policy);
    let url = Url::parse("https://images.test/timeout-decode.png").unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    resources.fetch_background_image(url.clone(), &warnings, None);
    assert!(resources.cached_background_image(&url).is_none());
    let seen = warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        seen.iter().any(|warning| matches!(
            &warning.kind,
            WarningKind::PolicyWarning { violation }
                if matches!(violation.violation_type, ViolationType::Timeout)
        )),
        "decode failure under zero timeout reports Timeout, got {seen:?}"
    );
}

#[test]
fn svg_raster_zero_timeout_via_direct_cache_returns_none() {
    let policy = ZeroDecodeTimeoutPolicy;
    let resources = RenderResources::new().network_policy(&policy);
    let url = Url::parse("https://images.test/direct-timeout.svg").unwrap();
    resources.cache_image_source(
        url.clone(),
        CachedBackgroundSource::Svg(Arc::new(
            SvgDocument::parse(TWO_COLOR_SVG).expect("fixture SVG parses"),
        )),
    );
    let raster = resources.get_decoded_at_size(
        &url,
        ImageRasterSize {
            width: 4.0,
            height: 2.0,
        },
        None,
    );
    assert!(
        raster.is_none(),
        "zero timeout refuses the paint-time SVG raster"
    );
    let allow = AllowAllPolicy;
    let allowed = RenderResources::new().network_policy(&allow);
    allowed.cache_image_source(
        url.clone(),
        CachedBackgroundSource::Svg(Arc::new(
            SvgDocument::parse(TWO_COLOR_SVG).expect("fixture SVG parses"),
        )),
    );
    let ok = allowed.get_decoded_at_size(
        &url,
        ImageRasterSize {
            width: 4.0,
            height: 2.0,
        },
        None,
    );
    assert!(ok.is_some(), "without a zero timeout SVG rasterizes");
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn ahem_fonts(system_fonts: bool) -> crate::RenderFonts {
    crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .system_fonts(system_fonts)
        .build()
        .expect("fonts")
}

#[test]
fn the_default_resources_use_the_system_layer_for_the_inline_engine() {
    let resources = RenderResources::new();
    let layer = resources.inline_engine_fonts();
    assert_eq!(
        layer.layer_handle().id(),
        raikiri_dom::system_font_collection().layer_handle().id()
    );
}

#[test]
fn a_font_set_is_the_layer_of_the_inline_engine() {
    let fonts = ahem_fonts(false);
    let expected = fonts.collection().layer_handle().id();
    let resources = RenderResources::new().fonts(fonts);
    assert_eq!(
        resources.inline_engine_fonts().layer_handle().id(),
        expected
    );
    assert_eq!(
        resources
            .font_collection_ref()
            .map(|layer| layer.layer_handle().id()),
        Some(expected)
    );
}

#[test]
fn border_images_of_elements_and_pseudo_elements_are_preloaded() {
    let provider = SvgNetworkProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let html = br#"<!doctype html><style>
        div { border-image:url(https://images.test/element.svg) 1 }
        div::before { content:''; border-image-source:url(https://images.test/before.svg) }
        </style><div></div>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = crate::parse(html.as_slice(), &options).unwrap();
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = Default::default();
    let mut attempts = 0;
    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);
    assert_eq!(
        *provider.requests.lock().unwrap(),
        ["element.svg", "before.svg"].map(|name| (
            Url::parse(&format!("https://images.test/{name}")).unwrap(),
            ResourceKind::Image
        ))
    );
}
