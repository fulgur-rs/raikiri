//! The pictures of SVG `<image>` elements come from the page's image
//! resources: the same policy, network provider, limits and budgets.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use raikiri_html::{RenderResources, RenderSvgImages, ResourceLimits};
use raikiri_svg::{SvgDocument, SvgImageAccess, SvgResources, SvgRootStyle, SvgViewport};
use raikiri_traits::vector::{Group, Node, RasterImage, VectorImage};
use raikiri_traits::{
    AbortController, DecodedImage, FetchOutcome, FetchedResource, ImagePixelSource, NetworkError,
    NetworkProvider, RenderWarning, Request, ResourceKind, ResourcePolicy, ViolationType,
    WarningKind,
};
use url::Url;

const VIEWPORT: SvgViewport = SvgViewport {
    width: 40.0,
    height: 20.0,
};

/// A picture of one color, as PNG bytes.
fn png(color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_raw(2, 2, color.repeat(4))
        .expect("2x2 pixels")
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("PNG encodes");
    out
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// How the double answers a URL.
#[derive(Clone)]
enum Reply {
    Body(Vec<u8>, &'static str),
    Redirect(&'static str),
    Fail,
}

/// Serves a table of URLs and writes down what it was asked.
struct Site {
    replies: Vec<(&'static str, Reply)>,
    asked: Mutex<Vec<String>>,
}

impl Site {
    fn new(replies: Vec<(&'static str, Reply)>) -> Self {
        Self {
            replies,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn pictures(color: [u8; 4]) -> Self {
        Self::new(vec![
            (
                "https://pictures.test/a.png",
                Reply::Body(png(color), "image/png"),
            ),
            (
                "https://pictures.test/b.png",
                Reply::Body(png(color), "image/png"),
            ),
        ])
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl NetworkProvider for Site {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.asked.lock().unwrap().push(request.url.to_string());
        let reply = self
            .replies
            .iter()
            .find(|(url, _)| *url == request.url.as_str())
            .map(|(_, reply)| reply.clone())
            .ok_or(NetworkError::Http(404))?;
        match reply {
            Reply::Body(bytes, content_type) => Ok(FetchOutcome::Body(
                FetchedResource::new(bytes.into(), request.url)
                    .content_type(Some(content_type.into())),
            )),
            Reply::Redirect(location) => Ok(FetchOutcome::Redirect {
                location: Url::parse(location).unwrap(),
                status: 302,
            }),
            Reply::Fail => Err(NetworkError::Other("down".into())),
        }
    }
}

/// Serves the same picture at every URL of a host.
struct Everything(Vec<u8>);

impl NetworkProvider for Everything {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        Ok(FetchOutcome::Body(
            FetchedResource::new(self.0.clone().into(), request.url)
                .content_type(Some("image/png".into())),
        ))
    }
}

#[derive(Default)]
struct Policy {
    denied_hosts: &'static [&'static str],
    denied_schemes: &'static [&'static str],
    denied_redirects: bool,
    mimes: &'static [&'static str],
    /// Leaves no time at all for decoding a picture.
    no_decode_time: bool,
}

impl ResourcePolicy for Policy {
    fn is_scheme_allowed(&self, scheme: &str, _kind: ResourceKind) -> bool {
        !self.denied_schemes.contains(&scheme)
    }
    fn is_host_allowed(&self, host: &str, _kind: ResourceKind) -> bool {
        !self.denied_hosts.contains(&host)
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        !self.denied_redirects
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
        if self.no_decode_time {
            Duration::ZERO
        } else {
            Duration::from_secs(10)
        }
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        self.mimes.iter().map(|mime| (*mime).to_owned()).collect()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

fn document(body: &str) -> SvgDocument {
    SvgDocument::parse_with_images(
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"##)
            .as_bytes(),
        SvgImageAccess::Any,
    )
    .unwrap()
}

fn build(images: &RenderSvgImages<'_, '_>, body: &str) -> VectorImage {
    document(body)
        .scene_with_resources(
            VIEWPORT,
            SvgRootStyle::default(),
            None,
            SvgResources::new().with_images(images),
        )
        .unwrap()
}

fn pictures(group: &Group) -> Vec<&RasterImage> {
    let mut found = Vec::new();
    for child in &group.children {
        match child {
            Node::Group(group) => found.extend(pictures(group)),
            Node::Image(image) => found.push(image),
            _ => {}
        }
    }
    found
}

fn count(scene: &VectorImage) -> usize {
    pictures(&scene.root).len()
}

fn base() -> Option<Url> {
    Some(Url::parse("https://pictures.test/page/index.html").unwrap())
}

fn image_at(href: &str) -> String {
    format!(r##"<image href="{href}" width="10" height="10"/>"##)
}

/// A `data:` URL of `bytes`, percent-encoded.
fn data_url(mime: &str, bytes: &[u8]) -> String {
    let encoded: String = bytes.iter().map(|byte| format!("%{byte:02X}")).collect();
    format!("data:{mime},{encoded}")
}

fn policy_violations(warnings: &[RenderWarning]) -> Vec<&ViolationType> {
    warnings
        .iter()
        .filter_map(|warning| match &warning.kind {
            WarningKind::PolicyWarning { violation } => Some(&violation.violation_type),
            _ => None,
        })
        .collect()
}

fn limit_warnings(warnings: &[RenderWarning]) -> Vec<(u64, u64)> {
    warnings
        .iter()
        .filter_map(|warning| match warning.kind {
            WarningKind::ResourceLimitExceeded { limit, actual, .. } => Some((limit, actual)),
            _ => None,
        })
        .collect()
}

// ── URLs ────────────────────────────────────────────────────

#[test]
fn a_relative_picture_is_fetched_against_the_base_and_drawn() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    let scene = build(&images, &image_at("../a.png"));
    let drawn = pictures(&scene.root);
    assert_eq!(drawn.len(), 1);
    assert_eq!((drawn[0].image.width, drawn[0].image.height), (2, 2));
    assert_eq!(drawn[0].image.rgba[..4], RED);
    assert_eq!(site.asked(), ["https://pictures.test/a.png"]);
    assert!(images.take_warnings().is_empty());
}

#[test]
fn the_base_of_the_resources_serves_a_source_given_none() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new()
        .network_provider(&site)
        .base_url(Url::parse("https://pictures.test/dir/").unwrap());
    let images = resources.svg_images(None);
    assert_eq!(count(&build(&images, &image_at("../b.png"))), 1);
    assert_eq!(site.asked(), ["https://pictures.test/b.png"]);
}

#[test]
fn every_url_is_requested_once_however_many_scenes_ask_for_it() {
    let site = Site::new(vec![
        (
            "https://pictures.test/a.png",
            Reply::Body(png(RED), "image/png"),
        ),
        ("https://pictures.test/down.png", Reply::Fail),
    ]);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    let body = format!(
        "{}{}{}",
        image_at("/a.png"),
        image_at("/down.png"),
        image_at("/a.png#again")
    );
    for _ in 0..3 {
        let scene = build(&images, &body);
        assert_eq!(count(&scene), 2);
    }
    assert_eq!(
        site.asked(),
        [
            "https://pictures.test/a.png",
            "https://pictures.test/down.png"
        ]
    );
    // The failure was said once, by the request that made it.
    let warnings = images.take_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        warnings[0].kind,
        WarningKind::NetworkFallback { .. }
    ));
    assert!(warnings[0].details.starts_with("SVG image: "));
}

#[test]
fn a_denied_host_is_never_requested() {
    let site = Site::pictures(RED);
    let policy = Policy {
        denied_hosts: &["pictures.test"],
        ..Policy::default()
    };
    let resources = RenderResources::new()
        .network_provider(&site)
        .network_policy(&policy);
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    assert!(site.asked().is_empty());
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::HostNotAllowed]
    ));
}

#[test]
fn a_response_over_the_byte_limit_leaves_no_picture() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new()
        .network_provider(&site)
        .resource_limits(ResourceLimits::new().max_resource_bytes(Some(16)));
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    let warnings = images.take_warnings();
    let limits = limit_warnings(&warnings);
    assert_eq!(limits.len(), 1);
    assert_eq!(limits[0].0, 16);
    assert!(limits[0].1 > 16);
}

#[test]
fn a_picture_that_takes_too_long_to_decode_is_dropped() {
    let site = Site::pictures(RED);
    let policy = Policy {
        no_decode_time: true,
        ..Policy::default()
    };
    let resources = RenderResources::new()
        .network_provider(&site)
        .network_policy(&policy);
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::Timeout]
    ));

    // A picture inlined in a `data:` URL has no time to be decoded either.
    let body = image_at(&data_url("image/png", &png(RED)));
    assert_eq!(count(&build(&images, &body)), 0);
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::Timeout]
    ));
}

#[test]
fn the_aggregate_budget_is_shared_by_all_the_pictures() {
    let site = Site::pictures(RED);
    let size = png(RED).len() as u64;
    let resources = RenderResources::new()
        .network_provider(&site)
        .resource_limits(ResourceLimits::new().max_aggregate_resource_bytes(Some(size + size / 2)));
    let images = resources.svg_images(base());
    // The first fits, the second goes over the budget.
    let scene = build(
        &images,
        &format!("{}{}", image_at("/a.png"), image_at("/b.png")),
    );
    assert_eq!(count(&scene), 1);
    assert_eq!(limit_warnings(&images.take_warnings()).len(), 1);
}

#[test]
fn redirects_are_followed_under_the_policy() {
    let site = Site::new(vec![
        (
            "https://pictures.test/old.png",
            Reply::Redirect("https://pictures.test/a.png"),
        ),
        (
            "https://pictures.test/a.png",
            Reply::Body(png(GREEN), "image/png"),
        ),
    ]);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    let scene = build(&images, &image_at("/old.png"));
    assert_eq!(pictures(&scene.root)[0].image.rgba[..4], GREEN);

    let policy = Policy {
        denied_redirects: true,
        ..Policy::default()
    };
    let resources = RenderResources::new()
        .network_provider(&site)
        .network_policy(&policy);
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/old.png"))), 0);
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::RedirectDenied]
    ));
}

#[test]
fn a_failing_request_leaves_no_picture() {
    let site = Site::new(vec![("https://pictures.test/a.png", Reply::Fail)]);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    assert!(matches!(
        images.take_warnings()[0].kind,
        WarningKind::NetworkFallback { .. }
    ));
}

#[test]
fn a_response_that_is_not_a_picture_leaves_no_picture() {
    let site = Site::new(vec![(
        "https://pictures.test/a.png",
        Reply::Body(b"not a picture".to_vec(), "image/png"),
    )]);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    assert!(matches!(
        images.take_warnings()[0].kind,
        WarningKind::ResourceFallback { .. }
    ));
}

#[test]
fn an_svg_document_is_not_a_picture() {
    const NESTED: &[u8] =
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2"/></svg>"##;
    let site = Site::new(vec![
        (
            "https://pictures.test/typed",
            Reply::Body(NESTED.to_vec(), "image/svg+xml"),
        ),
        (
            "https://pictures.test/named.svg",
            Reply::Body(NESTED.to_vec(), "application/octet-stream"),
        ),
    ]);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    let scene = build(
        &images,
        &format!("{}{}", image_at("/typed"), image_at("/named.svg")),
    );
    assert_eq!(count(&scene), 0);
    // The nested document is never part of the scene.
    assert!(scene.root.children.is_empty());
    let warnings = images.take_warnings();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .all(|warning| matches!(warning.kind, WarningKind::ResourceFallback { .. }))
    );
}

#[test]
fn without_a_base_only_absolute_urls_are_requested() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(None);
    let scene = build(
        &images,
        &format!(
            "{}{}",
            image_at("a.png"),
            image_at("https://pictures.test/b.png")
        ),
    );
    assert_eq!(count(&scene), 1);
    assert_eq!(site.asked(), ["https://pictures.test/b.png"]);
    assert!(matches!(
        images.take_warnings()[0].kind,
        WarningKind::ResourceFallback { url: None, .. }
    ));
}

#[test]
fn a_url_that_names_no_resource_leaves_no_picture() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    assert_eq!(
        count(&build(&images, &image_at("mailto:someone@pictures.test"))),
        0
    );
    assert!(site.asked().is_empty());
}

#[test]
fn without_a_network_provider_nothing_is_requested() {
    let resources = RenderResources::new();
    let images = resources.svg_images(base());
    assert_eq!(count(&build(&images, &image_at("/a.png"))), 0);
    assert!(matches!(
        images.take_warnings()[0].kind,
        WarningKind::NetworkFallback { .. }
    ));
}

#[test]
fn an_aborted_signal_stops_the_requests() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new().network_provider(&site);
    let controller = AbortController::new();
    let images = resources
        .svg_images(base())
        .abort_signal(controller.signal.clone());
    controller.abort();
    let body = format!(
        "{}{}",
        image_at("/a.png"),
        image_at(&data_url("image/png", &png(RED)))
    );
    assert_eq!(count(&build(&images, &body)), 0);
    assert!(site.asked().is_empty());
}

#[test]
fn the_pixel_source_of_the_page_can_serve_a_picture() {
    struct Pixels(Arc<DecodedImage>);
    impl ImagePixelSource for Pixels {
        fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
            (url.host_str() == Some("pixels.test")).then(|| Arc::clone(&self.0))
        }
    }
    let pixels = Pixels(Arc::new(DecodedImage::new(1, 1, GREEN.to_vec())));
    let site = Site::pictures(RED);
    let resources = RenderResources::new()
        .network_provider(&site)
        .image_pixel_source(&pixels);
    let images = resources.svg_images(base());
    let scene = build(&images, &image_at("https://pixels.test/mine.png"));
    assert_eq!(pictures(&scene.root)[0].image.rgba, GREEN);
    assert!(site.asked().is_empty());
}

#[test]
fn the_request_limit_is_shared_by_all_the_scenes_of_a_source() {
    let site = Everything(png(RED));
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(base());
    let many: String = (0..256)
        .map(|index| image_at(&format!("/{index}.png")))
        .collect();
    assert_eq!(count(&build(&images, &many)), 256);
    assert!(images.take_warnings().is_empty());
    // The next picture is over the limit, and says so once.
    assert_eq!(count(&build(&images, &image_at("/256.png"))), 0);
    assert_eq!(limit_warnings(&images.take_warnings()), [(256, 257)]);
    assert_eq!(count(&build(&images, &image_at("/257.png"))), 0);
    assert!(images.take_warnings().is_empty());
}

// ── data: URLs ──────────────────────────────────────────────

#[test]
fn a_data_url_picture_is_decoded_without_a_request() {
    let site = Site::pictures(RED);
    let resources = RenderResources::new().network_provider(&site);
    let images = resources.svg_images(None);
    let scene = build(&images, &image_at(&data_url("image/png", &png(GREEN))));
    assert_eq!(pictures(&scene.root)[0].image.rgba[..4], GREEN);
    assert!(site.asked().is_empty());
    assert!(images.take_warnings().is_empty());
}

#[test]
fn a_data_url_picture_is_decoded_once_for_all_the_scenes() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let body = image_at(&data_url("image/png", &png(GREEN)));
    let first = build(&images, &body);
    let second = build(&images, &body);
    // Both scenes hold the one decoded picture.
    assert!(Arc::ptr_eq(
        &pictures(&first.root)[0].image,
        &pictures(&second.root)[0].image
    ));
}

#[test]
fn a_data_url_that_is_not_a_picture_charges_the_budget_once() {
    let junk = b"not a picture".to_vec();
    let resources = RenderResources::new().resource_limits(
        ResourceLimits::new().max_aggregate_resource_bytes(Some(junk.len() as u64)),
    );
    let images = resources.svg_images(None);
    let body = image_at(&data_url("image/png", &junk));
    for _ in 0..3 {
        assert_eq!(count(&build(&images, &body)), 0);
    }
    // One decode failure, and no complaint about the budget.
    let warnings = images.take_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        warnings[0].kind,
        WarningKind::ResourceFallback { .. }
    ));
}

#[test]
fn a_data_url_over_the_byte_limit_leaves_no_picture() {
    let resources =
        RenderResources::new().resource_limits(ResourceLimits::new().max_resource_bytes(Some(16)));
    let images = resources.svg_images(None);
    assert_eq!(
        count(&build(
            &images,
            &image_at(&data_url("image/png", &png(RED)))
        )),
        0
    );
    assert_eq!(limit_warnings(&images.take_warnings()).len(), 1);
}

#[test]
fn the_policy_governs_data_urls_too() {
    let denied = Policy {
        denied_schemes: &["data"],
        ..Policy::default()
    };
    let resources = RenderResources::new().network_policy(&denied);
    let images = resources.svg_images(None);
    let body = image_at(&data_url("image/png", &png(RED)));
    assert_eq!(count(&build(&images, &body)), 0);
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::SchemeNotAllowed]
    ));

    let only_jpeg = Policy {
        mimes: &["image/jpeg"],
        ..Policy::default()
    };
    let resources = RenderResources::new().network_policy(&only_jpeg);
    let images = resources.svg_images(None);
    assert_eq!(count(&build(&images, &body)), 0);
    assert!(matches!(
        policy_violations(&images.take_warnings())[..],
        [ViolationType::MimeNotAllowed { .. }]
    ));
}

#[test]
fn data_urls_share_the_request_limit() {
    let resources = RenderResources::new();
    let images = resources.svg_images(None);
    let distinct = |index: usize| {
        image_at(&data_url(
            "image/png",
            &png([(index % 256) as u8, (index / 256) as u8, 0, 255]),
        ))
    };
    let many: String = (0..256).map(distinct).collect();
    assert_eq!(count(&build(&images, &many)), 256);
    assert!(images.take_warnings().is_empty());
    assert_eq!(count(&build(&images, &distinct(256))), 0);
    assert_eq!(limit_warnings(&images.take_warnings()), [(256, 257)]);
    assert_eq!(count(&build(&images, &distinct(257))), 0);
    assert!(images.take_warnings().is_empty());
    // What was decoded before the limit is still known.
    assert_eq!(count(&build(&images, &distinct(0))), 1);
}

#[test]
fn an_aborted_signal_stops_data_urls_too() {
    let resources = RenderResources::new();
    let controller = AbortController::new();
    let images = resources
        .svg_images(None)
        .abort_signal(controller.signal.clone());
    controller.abort();
    let body = image_at(&data_url("image/png", &png(RED)));
    assert_eq!(count(&build(&images, &body)), 0);
}
