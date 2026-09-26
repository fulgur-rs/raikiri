use super::*;
use raikiri_traits::{
    FetchOutcome, FetchedResource, ImagePixelSource, ImageRasterSize, NetworkError,
    NetworkProvider, Request, ResourceKind,
};
use std::sync::Mutex;

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
    let cascade = crate::build_cascaded(&uncascaded);
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;

    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts);

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
    let cascade = crate::build_cascaded(&uncascaded);
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = std::collections::HashSet::new();
    let mut attempts = 0;

    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts);
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
fn image_pixel_source_builder_and_font_context_ref() {
    let provider = StubImageProvider;
    let resources = RenderResources::new()
        .image_pixel_source(&provider)
        .font_context(parley::FontContext::new());
    let debug = format!("{resources:?}");
    assert!(debug.contains("has_image_pixel_source: true"), "{debug}");
    let _ = resources.font_context_ref();
}
