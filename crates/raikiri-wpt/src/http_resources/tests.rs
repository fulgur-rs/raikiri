use super::*;
use std::collections::HashMap;

use crate::test_http_server::{TestResponse, TestServer};

fn request(url: Url, kind: ResourceKind) -> Request {
    Request {
        url,
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind,
    }
}

#[test]
fn stylesheet_provider_rewrites_css_but_preserves_other_resource_bytes() {
    let css = b"p{background:url(image.png)}";
    let server = TestServer::start(HashMap::from([(
        "/styles/sheet.css",
        TestResponse::ok("text/css", css.to_vec()),
    )]));
    let provider = SystemHttpProvider::new();
    let stylesheet_provider = StylesheetUrlProvider {
        provider: &provider,
    };
    for kind in [
        ResourceKind::ExternalStylesheet,
        ResourceKind::StylesheetImport,
    ] {
        let outcome = stylesheet_provider
            .fetch_one_hop(request(server.url("styles/sheet.css"), kind))
            .unwrap();
        let FetchOutcome::Body(resource) = outcome else {
            panic!("expected CSS response body");
        };
        assert_eq!(
            resource.bytes.as_ref(),
            format!(
                "p{{background:url(\"{}\")}}",
                server.url("styles/image.png")
            )
            .as_bytes()
        );
        assert_eq!(resource.final_url, server.url("styles/sheet.css"));
    }
    let FetchOutcome::Body(resource) = stylesheet_provider
        .fetch_one_hop(request(server.url("styles/sheet.css"), ResourceKind::Font))
        .unwrap()
    else {
        panic!("expected unchanged response body");
    };
    assert_eq!(resource.bytes.as_ref(), css);
    assert_eq!(server.finish(), ["/styles/sheet.css"; 3]);
}

#[test]
fn stylesheet_provider_returns_redirect_without_following_it() {
    let server = TestServer::start(HashMap::from([(
        "/styles/redirect.css",
        TestResponse::redirect("../sheet.css"),
    )]));
    let provider = SystemHttpProvider::new();
    let stylesheet_provider = StylesheetUrlProvider {
        provider: &provider,
    };
    let outcome = stylesheet_provider
        .fetch_one_hop(request(
            server.url("styles/redirect.css"),
            ResourceKind::ExternalStylesheet,
        ))
        .unwrap();
    let FetchOutcome::Redirect { location, status } = outcome else {
        panic!("expected one-hop redirect");
    };
    assert_eq!(status, 302);
    assert_eq!(location, server.url("sheet.css"));
    assert_eq!(server.finish(), ["/styles/redirect.css"]);
}

#[test]
fn failed_resource_requests_keep_the_network_error_or_return_no_font() {
    use raikiri_dom::FontFaceLoader;

    let server = TestServer::start(HashMap::<&str, TestResponse>::new());
    let provider = SystemHttpProvider::new();
    let stylesheet_provider = StylesheetUrlProvider {
        provider: &provider,
    };
    assert!(matches!(
        stylesheet_provider.fetch_one_hop(request(
            server.url("missing.css"),
            ResourceKind::StylesheetImport
        )),
        Err(NetworkError::Http(404))
    ));
    let base_url = server.url("index.html");
    let loader = NetworkFontLoader {
        provider: &provider,
        base_url: &base_url,
    };
    assert_eq!(loader.load("missing.ttf"), None);
    assert_eq!(loader.load("http://["), None);
    assert_eq!(server.finish(), ["/missing.css", "/missing.ttf"]);
}

#[test]
fn invalid_background_url_is_left_unchanged_without_a_fetch() {
    let provider = SystemHttpProvider::new();
    let resolver = ImageResolver::new(provider);
    let base = Url::parse("http://web-platform.test/styles/sheet.css").unwrap();
    let mut image = BackgroundImage::Url("http://[".into());
    prepare_background_image(&mut image, &base, &resolver);
    assert_eq!(image, BackgroundImage::Url("http://[".into()));
}

#[test]
fn generated_content_images_are_prefetched_against_the_base_url() {
    struct Recording(std::cell::RefCell<Vec<String>>);
    impl raikiri_traits::ReplacedResolver for Recording {
        fn resolve(
            &self,
            req: ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            self.0.borrow_mut().push(req.url().to_string());
            Err(raikiri_traits::ResolverError::Decode("not an image".into()))
        }
    }
    let mut doc = raikiri_dom::Document::new();
    let style = doc.append_element(Some(0), "style", Default::default(), None::<&str>);
    doc.append_text(
        style,
        "div::before{content:'a' url(icon.png) url(http://[)}\
         div::after{content:url(https://images.test/after.png)}\
         li::marker{content:url(marker.png)}",
    );
    doc.append_element(Some(0), "div", Default::default(), None::<&str>);
    doc.append_element(Some(0), "li", Default::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let mut cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let resolver = Recording(Default::default());
    let base = Url::parse("http://web-platform.test/css/test.html").unwrap();
    prepare_cascade_images(&mut cascade, &base, &resolver);
    let mut requested = resolver.0.into_inner();
    requested.sort();
    // `::marker` images are not prefetched here, and an unparsable URL is
    // skipped.
    assert_eq!(
        requested,
        [
            "http://web-platform.test/css/icon.png",
            "https://images.test/after.png"
        ]
    );
}

#[test]
fn generated_content_prefetch_stops_at_the_layout_image_limit() {
    struct Counting(std::cell::Cell<usize>);
    impl raikiri_traits::ReplacedResolver for Counting {
        fn resolve(
            &self,
            _: ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            self.0.set(self.0.get() + 1);
            Err(raikiri_traits::ResolverError::Decode("not an image".into()))
        }
    }
    let limit = raikiri_dom::generated_content::MAX_GENERATED_IMAGES;
    let content: String = (0..limit + 8)
        .map(|index| format!(" url(i{index}.png)"))
        .collect();
    let mut doc = raikiri_dom::Document::new();
    let style = doc.append_element(Some(0), "style", Default::default(), None::<&str>);
    doc.append_text(style, format!("div::before{{content:{content}}}"));
    doc.append_element(Some(0), "div", Default::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let mut cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let resolver = Counting(Default::default());
    let base = Url::parse("http://web-platform.test/css/test.html").unwrap();
    prepare_cascade_images(&mut cascade, &base, &resolver);
    assert_eq!(resolver.0.get(), limit);
}
