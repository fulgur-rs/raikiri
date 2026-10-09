use super::*;
use raikiri_traits::{Dom, Element, Node, NodeKind};

fn empty_options<'a>() -> ParseOptions<'a> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

fn stylesheet_texts(doc: &UncascadedDocument) -> Vec<String> {
    doc.stylesheet_sources
        .iter()
        .map(|sheet| {
            sheet
                .parts
                .iter()
                .map(|part| part.source.as_str())
                .collect()
        })
        .collect()
}

fn find_first_by_tag<'a>(
    doc: &'a raikiri_dom::Document,
    target: &str,
) -> Option<raikiri_traits::NodeId> {
    fn recur(
        doc: &raikiri_dom::Document,
        id: raikiri_traits::NodeId,
        target: &str,
    ) -> Option<raikiri_traits::NodeId> {
        let node = doc.node(id)?;
        if let Some(el) = node.as_element() {
            if el.tag_name() == target {
                return Some(id);
            }
        }
        for c in doc.child_ids(id) {
            if let Some(hit) = recur(doc, c, target) {
                return Some(hit);
            }
        }
        None
    }
    recur(doc, doc.root_id(), target)
}

#[test]
fn parse_hello_world_produces_expected_tree() {
    let html = b"<html><body><p>Hello</p></body></html>";
    let options = empty_options();
    let uncascaded = parse(&html[..], &options).expect("parse ok");

    // Assert the html > body > p > "Hello" tree.
    // Note: the html5ever tokenizer may split text into multiple AppendText calls,
    // so concatenate Text children before asserting. Adjacent Text nodes are not
    // auto-merged yet; this is a current implementation limit.
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p element exists");
    let mut collected = String::new();
    for c in uncascaded.dom.child_ids(p_id) {
        let child = uncascaded.dom.node(c).unwrap();
        assert_eq!(
            child.kind(),
            NodeKind::Text,
            "expected only Text children under <p>, got {:?}",
            child.kind()
        );
        collected.push_str(child.text_content().unwrap_or(""));
    }
    assert_eq!(collected, "Hello");

    // parse-only path: no stylesheet
    assert!(uncascaded.stylesheet_sources.is_empty());
}

#[test]
fn parse_extracts_style_element_content() {
    let html = b"<html><head><style>p{color:red}</style></head><body><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![String::from("p{color:red}")]
    );
}

#[test]
fn parse_skips_link_stylesheet_when_no_network_provider() {
    // Fetching external <link rel="stylesheet"> resources is opt-in and happens
    // only when `ParseOptions::network` is `Some`. With `network: None`
    // (empty_options()), even href resolution is skipped and stylesheet_sources
    // remains empty. Existing callers that provide no network capability keep
    // the same behavior.
    let html = br#"<html><head><link rel="stylesheet" href="foo.css"></head><body>x</body></html>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    // cov:ignore: assert! message args only evaluate when the condition
    // is false; this assertion passes in every run.
    assert!(
        uncascaded.stylesheet_sources.is_empty(),
        "external link stylesheets must not be fetched without a NetworkProvider"
    );
    // NB: doesn't assert `warnings.is_empty()` — this minimal fixture (no
    // `<!DOCTYPE html>`) already triggers an unrelated html5ever
    // `HtmlParseError` ("Unexpected token", spec-conformant per HTML5
    // §13.2.6.4.1 initial insertion mode) regardless of the `<link>`
    // element. The assertion below targets only what the external
    // stylesheet fetch feature could plausibly add: no fetch-related
    // warning without a provider.
    // cov:ignore: assert! message args only evaluate when the condition
    // is false; this assertion passes in every run.
    assert!(
        !uncascaded.warnings.iter().any(|w| matches!(
            &w.kind,
            raikiri_traits::WarningKind::NetworkFallback { .. }
                | raikiri_traits::WarningKind::PolicyWarning { .. }
        )),
        "no NetworkProvider means no fetch attempt, so no fetch-related warning either, got: {:?}",
        uncascaded.warnings
    );
}

/// `<link rel="stylesheet">` fetch → CSS text →
/// Test the integration of `UncascadedDocument.stylesheet_sources`
/// end to end using a mock `NetworkProvider`. CSS parsing and cascade
/// integration belong to raikiri-style and the raikiri umbrella crate; this
/// test covers raikiri-html’s responsibility: detection, fetching, and
/// incorporation into doc.stylesheet_sources.
mod external_link_stylesheet_fetch_tests {
    use super::*;
    use raikiri_traits::{
        FetchOutcome, FetchedResource, NetworkError, NetworkProvider, PolicyViolation, Request,
        ResourceKind, ViolationType, WarningKind,
    };

    /// `NetworkProvider` that echoes the resolved request URL back as
    /// the CSS body comment (`/* <url> */`). Doubles as both a
    /// content-producer (fetch succeeded) and an assertion probe (which
    /// URL raikiri-html actually resolved and requested) without extra
    /// interior-mutability bookkeeping.
    struct EchoUrlProvider;

    impl NetworkProvider for EchoUrlProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: bytes::Bytes::from(format!("/* {} */", request.url).into_bytes()),
                content_type: Some("text/css".to_string()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    struct ImportProvider {
        requests: std::sync::Mutex<Vec<(String, ResourceKind)>>,
    }

    impl NetworkProvider for ImportProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let requested = request.url.to_string();
            self.requests
                .lock()
                .unwrap()
                .push((requested.clone(), request.kind));
            let (css, final_url) = match requested.as_str() {
                "https://page.example/css/inline.css" => (
                    ".inline { color: red }",
                    "https://page.example/css/inline.css",
                ),
                "https://page.example/css/main.css" => (
                    "@import \"nested.css\" screen; .root { color: green }",
                    "https://cdn.example/assets/main.css",
                ),
                "https://cdn.example/assets/nested.css" => (
                    ".nested { color: blue }",
                    "https://cdn.example/assets/nested.css",
                ),
                _ => return Err(NetworkError::Other("not found".to_owned())),
            };
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: bytes::Bytes::from(css),
                content_type: Some("text/css".to_owned()),
                final_url: url::Url::parse(final_url).unwrap(),
                encoding: None,
            }))
        }
    }

    struct MediaSheetProvider;

    impl NetworkProvider for MediaSheetProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let css = match request.url.path() {
                "/print.css" => {
                    "@font-face { font-family: 'PrintFont'; src: url('/p.ttf'); } \
                     @layer base { p { display: inline } } \
                     @page { margin: 1in }"
                }
                "/screen.css" => {
                    "@font-face { font-family: 'ScreenFont'; src: url('/s.ttf'); } \
                     span { display: block }"
                }
                _ => return Err(NetworkError::Other("not found".to_owned())),
            };
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: bytes::Bytes::from(css),
                content_type: Some("text/css".to_owned()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    #[test]
    fn link_media_attribute_guards_the_fetched_stylesheet() {
        let provider = MediaSheetProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(url::Url::parse("https://page.example/index.html").unwrap()),
        };
        let html = br#"<html><head>
                <link rel="stylesheet" href="print.css" media="print">
                <style>em { display: block }</style>
                <link rel="stylesheet" href="screen.css" media="screen and (min-width: 1px)">
                </head><body><p>x</p><span>y</span></body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(uncascaded.stylesheet_sources.len(), 3);
        assert_eq!(
            uncascaded
                .stylesheet_sources
                .iter()
                .map(|sheet| sheet.media.clone())
                .collect::<Vec<_>>(),
            [
                Some("print".to_owned()),
                None,
                Some("screen and (min-width: 1px)".to_owned()),
            ]
        );

        let tree = crate::cascade::build_rule_tree(&uncascaded);
        let print = raikiri_style::MediaContext::print();
        let screen = raikiri_style::MediaContext::screen();
        let print_faces = tree.font_faces_for(&print);
        let screen_faces = tree.font_faces_for(&screen);
        assert!(print_faces.get("PrintFont").is_some());
        assert!(print_faces.get("ScreenFont").is_none());
        assert!(screen_faces.get("PrintFont").is_none());
        assert!(screen_faces.get("ScreenFont").is_some());
        // The layered rule and `@page` survive inside the print-only sheet.
        assert_eq!(tree.page_rules.len(), 1);

        let display = |context: &raikiri_style::MediaContext, tag: &str| {
            let id = find_first_by_tag(&uncascaded.dom, tag).unwrap();
            let result =
                raikiri_style::cascade_with_media_context(&uncascaded.dom, &tree, context).unwrap();
            result.computed[id.0 as usize].display
        };
        use raikiri_style::DisplayValue;
        assert_eq!(display(&print, "p"), DisplayValue::Inline);
        assert_eq!(display(&screen, "p"), DisplayValue::Block);
        assert_eq!(display(&print, "span"), DisplayValue::Inline);
        assert_eq!(display(&screen, "span"), DisplayValue::Block);
    }

    #[test]
    fn parse_expands_inline_and_external_imports_with_request_kinds_and_redirect_base() {
        let provider = ImportProvider {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(url::Url::parse("https://page.example/css/page.html").unwrap()),
        };
        let html = br#"<html><head>
                <style>@import "inline.css"; .inline { color: red }</style>
                <link rel="stylesheet" href="main.css">
                </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");

        assert_eq!(uncascaded.stylesheet_sources.len(), 2);
        assert!(stylesheet_texts(&uncascaded)[0].contains(".inline { color: red }"));
        assert!(stylesheet_texts(&uncascaded)[1].contains(".nested { color: blue }"));
        assert!(
            uncascaded.stylesheet_sources[1]
                .parts
                .iter()
                .any(|part| part.media.iter().any(|media| media == "screen"))
        );
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                (
                    "https://page.example/css/inline.css".to_owned(),
                    ResourceKind::StylesheetImport,
                ),
                (
                    "https://page.example/css/main.css".to_owned(),
                    ResourceKind::ExternalStylesheet,
                ),
                (
                    "https://cdn.example/assets/nested.css".to_owned(),
                    ResourceKind::StylesheetImport,
                ),
            ]
        );
    }

    /// `NetworkProvider` that returns a successful, empty-body response
    /// (verifies the `extract_inline_stylesheets`-style empty-body
    /// guard: an empty fetched stylesheet must not be pushed).
    struct EmptyBodyProvider;

    impl NetworkProvider for EmptyBodyProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: bytes::Bytes::new(),
                content_type: Some("text/css".to_string()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    /// `NetworkProvider` whose `fetch` always fails with a caller-chosen
    /// error (constructed fresh per call since `NetworkError` isn't
    /// `Clone`).
    struct AlwaysErrorProvider(fn() -> NetworkError);

    impl NetworkProvider for AlwaysErrorProvider {
        fn fetch_one_hop(&self, _request: Request) -> Result<FetchOutcome, NetworkError> {
            Err((self.0)())
        }
    }

    /// `NetworkProvider` whose `fetch` panics if invoked — used to
    /// assert a `<link>` was correctly *not* recognized as an external
    /// stylesheet reference (rel-token / type-attribute / href-missing
    /// gating), i.e. fetch must not even be attempted.
    struct PanicIfCalledProvider;

    impl NetworkProvider for PanicIfCalledProvider {
        // cov:ignore: this fn's body must never execute — that is
        // exactly what every test using this mock asserts. See the
        // type's doc comment above.
        fn fetch_one_hop(&self, _request: Request) -> Result<FetchOutcome, NetworkError> {
            panic!("fetch must not be called for a <link> that is not a stylesheet reference");
        }
    }

    #[test]
    fn parse_resolves_imports_in_head_source_order() {
        let provider = ImportProvider {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(url::Url::parse("https://page.example/css/page.html").unwrap()),
        };
        let html = br#"<html><head>
                <link rel="stylesheet" href="main.css">
                <style>@import "inline.css";</style>
                </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");

        assert!(stylesheet_texts(&uncascaded)[0].contains(".nested { color: blue }"));
        assert!(stylesheet_texts(&uncascaded)[1].contains(".inline { color: red }"));
        assert_eq!(
            *provider.requests.lock().unwrap(),
            vec![
                (
                    "https://page.example/css/main.css".to_owned(),
                    ResourceKind::ExternalStylesheet,
                ),
                (
                    "https://cdn.example/assets/nested.css".to_owned(),
                    ResourceKind::StylesheetImport,
                ),
                (
                    "https://page.example/css/inline.css".to_owned(),
                    ResourceKind::StylesheetImport,
                ),
            ]
        );
    }

    #[test]
    fn parse_fetches_external_stylesheet_with_absolute_href() {
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://example.test/a.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://example.test/a.css */")]
        );
    }

    #[test]
    fn parse_skips_comment_node_and_still_finds_link_stylesheet_after_it() {
        // Exercises collect_external_stylesheet_hrefs's `is_in_document()`
        // gate for real: unlike <template> contents (routed through a
        // detached fragment root that a plain child_ids() DFS never
        // reaches at all, per raikiri-dom::Document::mark_in_document_flags
        // doc comment), a <!-- comment --> node *is* reachable via normal
        // child_ids() traversal from <head> and still gets its
        // IS_IN_DOCUMENT bit cleared (Comment/ProcessingInstruction kind
        // gate) — so this is the one case that
        // genuinely walks into the `if !node.is_in_document() { continue }`
        // branch during a real parse.
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <!-- a comment between head children -->
                           <link rel="stylesheet" href="https://example.test/a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // cov:ignore: assert_eq! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://example.test/a.css */")],
            "the comment must not prevent the <link> after it from being fetched"
        );
    }

    #[test]
    fn parse_resolves_relative_href_against_base_url() {
        let provider = EchoUrlProvider;
        let base = url::Url::parse("https://example.test/dir/page.html").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base),
        };
        let html =
            br#"<html><head><link rel="stylesheet" href="style.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://example.test/dir/style.css */")]
        );
    }

    #[test]
    fn parse_resolves_link_href_against_base_element_when_present() {
        // HTML Standard §4.2.7 "The base element": a <base href> in
        // <head> overrides options.base_url as the base for resolving
        // every subsequent relative URL in the document, including
        // <link href> — even though options.base_url ("the page's own
        // URL") points at a completely different host.
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <base href="https://cdn.example/">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://cdn.example/a.css */")],
            "<link href> must resolve against the <base> override, not options.base_url"
        );
    }

    #[test]
    fn parse_resolves_relative_base_href_against_options_base_url() {
        // The <base>'s own href can itself be relative per spec — it is
        // resolved against options.base_url (document's fallback base
        // URL) first, and *that* result becomes the effective base for
        // <link href>.
        let provider = EchoUrlProvider;
        let base_url =
            url::Url::parse("https://page.example/dir/page.html").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <base href="/assets/">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://page.example/assets/a.css */")]
        );
    }

    #[test]
    fn parse_uses_first_base_element_in_document_order() {
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <base href="https://first.example/">
                           <base href="https://second.example/">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://first.example/a.css */")],
            "the first <base> in document order wins, later ones are ignored"
        );
    }

    #[test]
    fn parse_applies_base_override_to_a_link_that_precedes_it_in_source_order() {
        // Known scope divergence from the HTML Standard's actual
        // processing model (documented on
        // `parse.rs::fetch_external_stylesheets`, the bullet on the relative
        // source order of <base> and <link>): per spec, a browser's streaming parser
        // fetches each <link>'s resource against the document base URL
        // *at the moment the <link> is inserted*, so a <link> before
        // the <base> in source order should resolve against
        // options.base_url, unaffected by a <base> that appears later.
        // This crate instead fetches every <head> <link> in one
        // post-parse pass using the document's *final* base URL, so a
        // <link> before <base> is (incorrectly, relative to spec, but
        // intentionally per the single-pass architecture) still
        // affected by the override. This test pins that as a known,
        // deliberate behavior rather than an untested edge case.
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <link rel="stylesheet" href="a.css">
                           <base href="https://cdn.example/">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://cdn.example/a.css */")]
        );
    }

    #[test]
    fn parse_skips_base_element_with_no_href_attribute_in_document_order_search() {
        // A <base> with no href attribute at all doesn't "count" (it
        // can't override anything), so the search continues past it to
        // the next <base> in document order.
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <base>
                           <base href="https://cdn.example/">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://cdn.example/a.css */")]
        );
    }

    #[test]
    fn parse_falls_back_to_options_base_url_when_base_href_is_empty() {
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/dir/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <base href="">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://page.example/dir/a.css */")],
            "an empty <base href> must not shadow a real base further down; \
                 options.base_url is used as if there were no <base> at all"
        );
    }

    #[test]
    fn parse_falls_back_to_options_base_url_when_base_href_is_whitespace_only() {
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/dir/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <base href="   ">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://page.example/dir/a.css */")]
        );
    }

    #[test]
    fn parse_falls_back_to_options_base_url_when_base_href_is_data_scheme() {
        // Per the base element's "frozen base URL" algorithm, a
        // <base href> that resolves to a `data:` or `javascript:` URL
        // is explicitly excluded from ever becoming the document base
        // URL — it falls back to the document's fallback base URL
        // (`options.base_url` here), exactly as if no <base> with a
        // usable href were present.
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/dir/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <base href="data:text/html,ignored">
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://page.example/dir/a.css */")]
        );
    }

    #[test]
    fn parse_skips_base_element_inside_template() {
        // <template> contents are inert per spec (mirrors
        // `parse_skips_link_stylesheet_inside_template_element` above) —
        // a <base> nested inside <template> in <head> must not be
        // treated as the document's base element.
        let provider = EchoUrlProvider;
        let base_url = url::Url::parse("https://page.example/").expect("valid base url");
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(base_url),
        };
        let html = br#"<html><head>
                           <template><base href="https://cdn.example/"></template>
                           <link rel="stylesheet" href="a.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://page.example/a.css */")],
            "a <base> inert inside <template> must not override options.base_url"
        );
    }

    #[test]
    fn parse_skips_relative_href_without_base_url() {
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html =
            br#"<html><head><link rel="stylesheet" href="style.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            uncascaded.stylesheet_sources.is_empty(),
            "relative href without base_url is unresolvable and must not be fetched"
        );
    }

    #[test]
    fn parse_preserves_document_order_across_multiple_link_stylesheets() {
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <link rel="stylesheet" href="https://example.test/a.css">
                           <link rel="stylesheet" href="https://example.test/b.css">
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![
                String::from("/* https://example.test/a.css */"),
                String::from("/* https://example.test/b.css */"),
            ]
        );
    }

    #[test]
    fn parse_preserves_document_order_between_inline_and_external_sources() {
        // `<style>` and `<link>` share the stylesheet_sources bucket, so
        // same-specificity rules retain their original head order.
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <link rel="stylesheet" href="https://example.test/a.css">
                           <style>p{color:red}</style>
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // cov:ignore: assert_eq! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![
                String::from("/* https://example.test/a.css */"),
                String::from("p{color:red}"),
            ],
            "stylesheet sources must retain the original head order"
        );
    }

    #[test]
    fn parse_preserves_source_order_across_head_and_body_styles() {
        // Head `<style>` / `<link>` / `<style>` entries are interleaved at
        // their original positions, then body styles continue in document
        // order. This is the source order consumed by the umbrella cascade.
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <style>p{color:red}</style>
                           <link rel="stylesheet" href="https://example.test/head.css">
                           <style>p{color:blue}</style>
                           </head><body>
                           <style>p{color:green}</style>
                           <section><style>p{color:purple}</style></section>
                           <p>x</p>
                           </body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![
                String::from("p{color:red}"),
                String::from("/* https://example.test/head.css */"),
                String::from("p{color:blue}"),
                String::from("p{color:green}"),
                String::from("p{color:purple}"),
            ]
        );
    }

    #[test]
    fn parse_skips_link_stylesheet_inside_template_element() {
        // <template> contents are inert per spec (mirrors
        // `parse_skips_style_inside_template_element` for <style>) —
        // a <link rel=stylesheet> nested inside <template> in <head>
        // must not be fetched at all.
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head>
                           <template><link rel="stylesheet" href="https://example.test/a.css"></template>
                           </head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_skips_non_stylesheet_rel_without_fetching() {
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="icon" href="https://example.test/favicon.ico"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_skips_non_css_type_attribute_without_fetching() {
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" type="application/rss+xml" href="https://example.test/feed"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_skips_titled_alternate_stylesheet_without_fetching() {
        // CSSOM "add a CSS style sheet" only unsets the disabled flag
        // for a titled alternate stylesheet when it matches the page's
        // preferred/selected stylesheet set — a concept this crate does
        // not track (`is_stylesheet_link` doc). Proves the `title`
        // attribute actually reaches `is_stylesheet_link` through
        // `collect_external_stylesheet_hrefs`'s `el.attr("title")` call,
        // not just that the predicate itself is correct in isolation.
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="alternate stylesheet" title="High Contrast" href="https://example.test/high-contrast.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_ignores_href_missing_link_stylesheet() {
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_ignores_whitespace_only_href_link_stylesheet() {
        // `href`'s HTML attribute type is "valid non-empty URL
        // potentially surrounded by spaces" — a value that's only
        // spaces is not a valid non-empty URL and must be skipped, not
        // resolved (`Url::join("   ")` on a base URL resolves to that
        // *base URL itself*, which would otherwise cause raikiri to
        // fetch the page's own URL and feed the resulting HTML to the
        // CSS parser).
        let provider = PanicIfCalledProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(
                url::Url::parse("https://example.test/page.html").expect("valid base url"),
            ),
        };
        let html = br#"<html><head><link rel="stylesheet" href="   "></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
    }

    #[test]
    fn parse_trims_surrounding_whitespace_from_a_real_href() {
        // The trim in the fix above must not reject a legitimate href
        // that merely has incidental surrounding whitespace (a common
        // authoring artifact) — only whitespace-*only* values are
        // skipped.
        let provider = EchoUrlProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="  https://example.test/a.css  "></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(
            stylesheet_texts(&uncascaded),
            vec![String::from("/* https://example.test/a.css */")]
        );
    }

    #[test]
    fn parse_ignores_empty_fetched_stylesheet_body() {
        let provider = EmptyBodyProvider;
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://example.test/empty.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            uncascaded.stylesheet_sources.is_empty(),
            "empty-body fetch response must not push an empty stylesheet source"
        );
        // Empty body is a *successful* fetch (Ok(FetchedResource { bytes:
        // empty, .. })), not a failure — no fetch-related warning either.
        // (Doesn't assert overall `warnings.is_empty()`: this minimal
        // fixture, like the sibling test above, independently triggers an
        // unrelated html5ever "Unexpected token" HtmlParseError from
        // missing `<!DOCTYPE html>`.)
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            !uncascaded.warnings.iter().any(|w| matches!(
                &w.kind,
                WarningKind::NetworkFallback { .. } | WarningKind::PolicyWarning { .. }
            )),
            "empty body is a successful fetch, not a failure: got {:?}",
            uncascaded.warnings
        );
    }

    #[test]
    fn parse_records_network_fallback_warning_on_fetch_failure() {
        let provider = AlwaysErrorProvider(|| NetworkError::Http(404));
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://example.test/missing.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            uncascaded.stylesheet_sources.is_empty(),
            "failed fetch must not contribute a stylesheet source"
        );
        let warning = uncascaded
            .warnings
            .iter()
            .find(|w| matches!(&w.kind, WarningKind::NetworkFallback { .. }))
            .expect("expected a NetworkFallback warning");
        match &warning.kind {
            WarningKind::NetworkFallback { url } => {
                assert_eq!(url.as_str(), "https://example.test/missing.css");
            }
            // cov:ignore: defensive "unexpected variant" arm, unreachable
            // while production code matches this test's expectation.
            other => panic!("expected NetworkFallback, got {other:?}"),
        }
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            warning.details.contains("404"),
            "details should carry the safe HTTP status summary, got: {}",
            warning.details
        );
        // `WarningKind::NetworkFallback` covers two dispositions (content
        // substituted vs. fetch failed with nothing applied), but
        // `RenderWarning::details` is documented as unstructured
        // free-form prose, not a discrimination contract. The status is
        // retained for useful diagnostics while arbitrary provider error
        // text is intentionally omitted to prevent credential/token leaks.
        // cov:ignore: assert! message args only evaluate when the
        // condition is false; this assertion passes in every run.
        assert!(
            warning.details.contains("fetch failed"),
            "details should read as a failure (no content applied), not a \
                 substitution, got: {}",
            warning.details
        );
    }

    #[test]
    fn parse_redacts_credentials_from_external_stylesheet_warnings() {
        let provider = AlwaysErrorProvider(|| NetworkError::Http(403));
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://user:secret@example.test/missing.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let warning = uncascaded
            .warnings
            .iter()
            .find(|w| matches!(&w.kind, WarningKind::NetworkFallback { .. }))
            .expect("expected a NetworkFallback warning");
        let WarningKind::NetworkFallback { url } = &warning.kind else {
            unreachable!("filtered above");
        };
        assert_eq!(url.as_str(), "https://example.test/missing.css");
        assert!(!warning.details.contains("user:secret"));
    }

    #[test]
    fn parse_does_not_copy_provider_error_text_into_warnings() {
        let provider = AlwaysErrorProvider(|| {
            NetworkError::Other("request https://user:secret@example.test/token".to_owned())
        });
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://example.test/a.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let warning = uncascaded
            .warnings
            .iter()
            .find(|w| matches!(&w.kind, WarningKind::NetworkFallback { .. }))
            .expect("expected a NetworkFallback warning");
        assert!(!warning.details.contains("user:secret"));
        assert!(!warning.details.contains("/token"));
        assert!(
            warning
                .details
                .contains("provider returned a network error")
        );
    }

    #[test]
    fn parse_keeps_failed_inline_import_and_records_warning() {
        let provider = AlwaysErrorProvider(|| NetworkError::Http(404));
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: Some(url::Url::parse("https://example.test/page.html").unwrap()),
        };
        let html = br#"<html><head><style>
                @import "missing.css";
                p { color: blue }
            </style></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert_eq!(uncascaded.stylesheet_sources.len(), 1);
        assert!(stylesheet_texts(&uncascaded)[0].contains(r#"@import "missing.css";"#));
        assert!(stylesheet_texts(&uncascaded)[0].contains("p { color: blue }"));
        assert!(
            uncascaded
                .warnings
                .iter()
                .any(|warning| matches!(&warning.kind, WarningKind::NetworkFallback { .. }))
        );
    }

    #[test]
    fn parse_records_policy_warning_on_policy_violation() {
        let provider = AlwaysErrorProvider(|| {
            NetworkError::PolicyViolation(Box::new(PolicyViolation {
                kind: ResourceKind::ExternalStylesheet,
                url: url::Url::parse("https://user:secret@blocked.test/a.css").expect("valid url"),
                violation_type: ViolationType::HostNotAllowed,
                details: "blocked https://user:secret@blocked.test/token".to_string(),
            }))
        });
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: Some(&provider as &dyn NetworkProvider),
            base_url: None,
        };
        let html = br#"<html><head><link rel="stylesheet" href="https://user:secret@blocked.test/a.css"></head><body>x</body></html>"#;
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        assert!(uncascaded.stylesheet_sources.is_empty());
        let warning = uncascaded
            .warnings
            .iter()
            .find(|w| matches!(&w.kind, WarningKind::PolicyWarning { .. }))
            .expect("expected a PolicyWarning warning");
        match &warning.kind {
            WarningKind::PolicyWarning { violation } => {
                assert_eq!(violation.kind, ResourceKind::ExternalStylesheet);
                assert!(matches!(
                    violation.violation_type,
                    ViolationType::HostNotAllowed
                ));
                assert_eq!(violation.url.as_str(), "https://blocked.test/a.css");
                assert_eq!(violation.details, "network policy denied the request");
                assert!(!warning.details.contains("user:secret"));
            }
            // cov:ignore: defensive "unexpected variant" arm, unreachable
            // while production code matches this test's expectation.
            other => panic!("expected PolicyWarning, got {other:?}"),
        }
    }
}

#[test]
fn parse_records_html_parse_warning_on_malformed_input() {
    use raikiri_traits::WarningKind;

    // Input that reliably causes html5ever to report a nonfatal parse error:
    // a lone </p> closing tag triggers "unexpected end tag".
    let html = b"</p>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse recovers");

    assert!(
        uncascaded
            .warnings
            .iter()
            .any(|w| matches!(&w.kind, WarningKind::HtmlParseError { .. })),
        "expected at least one HtmlParseError warning, got: {:?}",
        uncascaded
            .warnings
            .iter()
            .map(|w| &w.kind)
            .collect::<Vec<_>>()
    );
}

#[test]
fn parse_caps_html_parse_warnings_and_does_not_grow_past_cap() {
    // html5ever reports roughly one parse_error per token of malformed input.
    // Without a cap, an attacker could accumulate an unbounded number of
    // RenderWarning entries (each owning a String), causing a DoS.
    let opts = empty_options();
    let default_cap = raikiri_traits::RenderLimits::default()
        .max_parse_warnings
        .expect("default max_parse_warnings must be Some");

    let small = b"</x>".repeat(2_000);
    let small_count = parse(small.as_slice(), &opts)
        .expect("parse recovers")
        .warnings
        .len();

    assert!(
        small_count <= default_cap,
        "parse warnings must be capped, got {small_count}"
    );

    // 8x more malformed input must not produce more warnings than the cap
    // (proves the bound holds, not just a coincidental small-input count).
    let large = b"</x>".repeat(16_000);
    let large_count = parse(large.as_slice(), &opts)
        .expect("parse recovers")
        .warnings
        .len();

    assert_eq!(
        small_count, large_count,
        "warning count must plateau at the cap regardless of input size"
    );
}

#[test]
fn raikiri_tree_sink_new_consults_custom_max_parse_warnings_cap() {
    // Verify that the parameter to `RaikiriTreeSink::new` directly sets the
    // `parse_error` cap, without going through the default `parse()` path.
    //
    // With cap=5, last_real_slot=4: record only four real warnings, then
    // replace the fifth with a synthetic entry indicating further suppression
    // (the reserved-last-slot contract; see `RaikiriTreeSink::parse_error` docs).
    // The 50 `</x>` tags yield far more raw parse errors than this cap
    // (the same input shape as `parse_caps_html_parse_warnings_and_does_not_grow_past_cap`,
    // which also confirms that a cap of 1024 bounds the count).
    use raikiri_traits::WarningKind;

    let opts = empty_options();
    let malformed = b"</x>".repeat(50);

    let sink = RaikiriTreeSink::new(Some(5));
    let uncascaded = parse_with_sink(malformed.as_slice(), sink, &opts).expect("parse recovers");
    assert_eq!(
        uncascaded.warnings.len(),
        5,
        "custom cap=5 must yield exactly 4 real + 1 synthetic warning"
    );
    match &uncascaded.warnings.last().expect("cap > 0").kind {
        WarningKind::HtmlParseError { message } => {
            assert!(
                message.contains("5-warning cap"),
                "last entry must be the synthetic suppression notice naming the runtime cap (5), got: {message:?}"
            );
        }
        other => panic!("expected HtmlParseError, got {other:?}"),
    }
}

#[test]
fn raikiri_tree_sink_new_zero_cap_records_one_synthetic_suppression_entry() {
    // cap=0 has no slot for a real warning, but must still record exactly
    // one synthetic suppression entry on the first parse_error call
    // (rather than silently recording nothing), so that "0 parse errors
    // occurred" and "N>=1 parse errors occurred but all were suppressed"
    // remain distinguishable -- consistent with cap>0's
    // trip-and-record-suppression semantics.
    use raikiri_traits::WarningKind;

    let opts = empty_options();
    let malformed = b"</x>".repeat(50);

    let sink = RaikiriTreeSink::new(Some(0));
    let uncascaded = parse_with_sink(malformed.as_slice(), sink, &opts).expect("parse recovers");
    assert_eq!(
        uncascaded.warnings.len(),
        1,
        "cap=0 must record exactly one synthetic suppression entry, got {}",
        uncascaded.warnings.len()
    );
    match &uncascaded.warnings[0].kind {
        WarningKind::HtmlParseError { message } => {
            assert!(
                message.contains("suppressed"),
                "expected a suppression message, got {message:?}"
            );
        }
        other => panic!("expected HtmlParseError, got {other:?}"),
    }
}

#[test]
fn raikiri_tree_sink_new_zero_cap_records_nothing_when_no_parse_errors_occur() {
    // Companion to the above: cap=0 with a well-formed document that
    // never calls parse_error must still record zero warnings -- the
    // synthetic entry is only pushed in response to an actual
    // suppressed parse error, not unconditionally at cap=0.
    let opts = empty_options();
    // `<!DOCTYPE html>` is required here -- a fragment lacking it
    // already triggers an unrelated html5ever "Unexpected token"
    // HtmlParseError (spec-conformant per HTML5 §13.2.6.4.1 initial
    // insertion mode) regardless of this test's own concern, which
    // would make the assertion below fail for a reason unrelated to
    // cap=0's suppression behavior (see the NB comment elsewhere in
    // this file documenting the same fixture-shape pitfall).
    let well_formed = b"<!DOCTYPE html><html><head></head><body><p>ok</p></body></html>";
    let sink = RaikiriTreeSink::new(Some(0));
    let uncascaded = parse_with_sink(well_formed.as_slice(), sink, &opts).expect("parse ok");
    assert!(
        uncascaded.warnings.is_empty(),
        "cap=0 with no parse errors must record zero warnings, got {}",
        uncascaded.warnings.len()
    );
}

#[test]
fn raikiri_tree_sink_new_cap_of_one_yields_only_the_synthetic_entry() {
    // With cap=1, last_real_slot=0 leaves no slot for real warnings.
    // The first parse_error call immediately records the synthetic entry.
    use raikiri_traits::WarningKind;

    let opts = empty_options();
    let malformed = b"</x>".repeat(50);

    let sink = RaikiriTreeSink::new(Some(1));
    let uncascaded = parse_with_sink(malformed.as_slice(), sink, &opts).expect("parse recovers");
    assert_eq!(
        uncascaded.warnings.len(),
        1,
        "cap=1 must yield exactly 1 warning (the synthetic entry, zero real slots)"
    );
    match &uncascaded.warnings[0].kind {
        WarningKind::HtmlParseError { message } => {
            assert!(
                message.contains("1-warning cap"),
                "the single entry must be the synthetic suppression notice, got: {message:?}"
            );
        }
        other => panic!("expected HtmlParseError, got {other:?}"),
    }
}

#[test]
fn raikiri_tree_sink_new_none_disables_warning_cap() {
    let opts = empty_options();
    let malformed = b"</x>".repeat(2_000);

    let sink = RaikiriTreeSink::new(None);
    let uncascaded = parse_with_sink(malformed.as_slice(), sink, &opts).expect("parse recovers");
    assert!(
        uncascaded.warnings.len() > 1024,
        "None must disable the warning cap entirely, got {}",
        uncascaded.warnings.len()
    );
}

#[test]
fn parse_returns_encoding_error_on_invalid_utf8() {
    use raikiri_traits::ParseError;

    let bad: &[u8] = &[0xFF, 0xFE, 0xFF, b'<', b'p', b'>'];
    let opts = empty_options();
    let err = parse(bad, &opts).expect_err("invalid utf-8 should fail");
    match err {
        ParseError::Encoding { label, reason } => {
            assert_eq!(label, "utf-8");
            assert!(!reason.is_empty(), "reason should be populated");
        }
        other => panic!("expected Encoding, got {other:?}"),
    }
}

#[test]
fn parse_returns_io_error_on_read_failure() {
    use raikiri_traits::ParseError;
    use std::io::{Error, ErrorKind, Read};

    struct FailingReader;
    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(Error::other("boom"))
        }
    }

    let opts = empty_options();
    let err = parse(FailingReader, &opts).expect_err("read failure should fail");
    match err {
        ParseError::Io(inner) => {
            assert_eq!(inner.kind(), ErrorKind::Other);
            assert_eq!(inner.to_string(), "boom");
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn parse_with_sink_via_transparent_wrapper() {
    use html5ever::interface::{Attribute, ElementFlags, NodeOrText, QualName, TreeSink};
    use html5ever::tendril::StrTendril;
    use html5ever::tree_builder::QuirksMode;
    use std::borrow::Cow;
    use std::cell::Ref;

    /// Consumer wrapper example: a transparent sink that delegates everything
    /// to RaikiriTreeSink. It does nothing at the sanitize/rewrite hook point.
    struct TransparentSink {
        inner: RaikiriTreeSink,
        observed_elements: std::cell::Cell<u32>,
    }

    impl TreeSink for TransparentSink {
        type Handle = usize;
        type Output = UncascadedDocument;
        type ElemName<'a> = Ref<'a, QualName>;

        fn finish(self) -> UncascadedDocument {
            self.inner.finish()
        }
        fn parse_error(&self, msg: Cow<'static, str>) {
            self.inner.parse_error(msg)
        }
        fn get_document(&self) -> usize {
            self.inner.get_document()
        }
        fn elem_name<'a>(&'a self, t: &'a usize) -> Ref<'a, QualName> {
            self.inner.elem_name(t)
        }
        fn create_element(
            &self,
            name: QualName,
            attrs: Vec<Attribute>,
            flags: ElementFlags,
        ) -> usize {
            self.observed_elements.set(self.observed_elements.get() + 1);
            self.inner.create_element(name, attrs, flags)
        }
        fn create_comment(&self, text: StrTendril) -> usize {
            self.inner.create_comment(text)
        }
        fn create_pi(&self, target: StrTendril, data: StrTendril) -> usize {
            self.inner.create_pi(target, data)
        }
        fn append(&self, parent: &usize, child: NodeOrText<usize>) {
            self.inner.append(parent, child)
        }
        fn append_based_on_parent_node(&self, e: &usize, p: &usize, c: NodeOrText<usize>) {
            self.inner.append_based_on_parent_node(e, p, c)
        }
        fn append_doctype_to_document(&self, name: StrTendril, pid: StrTendril, sid: StrTendril) {
            self.inner.append_doctype_to_document(name, pid, sid)
        }
        fn get_template_contents(&self, t: &usize) -> usize {
            self.inner.get_template_contents(t)
        }
        fn same_node(&self, x: &usize, y: &usize) -> bool {
            self.inner.same_node(x, y)
        }
        fn set_quirks_mode(&self, mode: QuirksMode) {
            self.inner.set_quirks_mode(mode)
        }
        fn append_before_sibling(&self, s: &usize, n: NodeOrText<usize>) {
            self.inner.append_before_sibling(s, n)
        }
        fn add_attrs_if_missing(&self, t: &usize, a: Vec<Attribute>) {
            self.inner.add_attrs_if_missing(t, a)
        }
        fn remove_from_parent(&self, t: &usize) {
            self.inner.remove_from_parent(t)
        }
        fn reparent_children(&self, n: &usize, p: &usize) {
            self.inner.reparent_children(n, p)
        }
        fn is_mathml_annotation_xml_integration_point(&self, h: &usize) -> bool {
            self.inner.is_mathml_annotation_xml_integration_point(h)
        }
    }

    let html = b"<html><body><p>Hi</p></body></html>";
    let opts = empty_options();
    let sink = TransparentSink {
        inner: RaikiriTreeSink::default(),
        observed_elements: std::cell::Cell::new(0),
    };
    // observed_elements is inside sink and moves into parse_with_sink.
    // We can't inspect it post-parse; instead we assert the returned Document
    // has the expected tree (proves finish() bubble ok).
    let uncascaded = parse_with_sink(&html[..], sink, &opts).expect("parse ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let text = uncascaded.dom.node(p_id).unwrap();
    let kids: Vec<_> = uncascaded.dom.child_ids(p_id).collect();
    assert_eq!(kids.len(), 1);
    assert_eq!(
        uncascaded.dom.node(kids[0]).unwrap().text_content(),
        Some("Hi")
    );
    let _ = text;
}

#[test]
fn parse_persists_comment_node_as_comment_variant_with_cleared_in_document_bit() {
    // Contract rewrite: previously, pseudo-tag "#comment" Elements were stripped.
    // Now `NodeData::Comment` nodes persist in the tree. In step 2,
    // `mark_in_document_flags` clears IS_IN_DOCUMENT; the cascade/paint Element
    // gate also skips them because they are not Elements. These two gates
    // replace the old stripping contract.
    //
    // The old test `parse_strips_comment_nodes_from_dom_tree` scanned for an
    // absent "#comment" pseudo-tag Element. With the new contract, Comment is
    // not an Element, so as_element() == None and the scan would pass vacuously.
    // Use positive assertions instead to guarantee coverage without relying
    // on simply running the old test and seeing a pass.
    use raikiri_traits::{Dom, Node};

    let html = b"<html><body><!-- a comment --><p>hi</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let doc = &uncascaded.dom;

    // (a) At least one Comment-kind node exists in the arena.
    let mut comment_ids: Vec<usize> = Vec::new();
    for i in 0..doc.node_count() {
        let n = doc.node(raikiri_traits::NodeId::new(i as u64)).unwrap();
        if n.kind() == NodeKind::Comment {
            comment_ids.push(i);
        }
    }
    assert_eq!(
        comment_ids.len(),
        1,
        "expected exactly 1 Comment node in arena after parse"
    );
    let comment_id = comment_ids[0];
    let comment_node = doc
        .node(raikiri_traits::NodeId::new(comment_id as u64))
        .unwrap();

    // (b) A Comment has as_element() == None (the two-way invariant).
    assert!(
        comment_node.as_element().is_none(),
        "NodeKind::Comment must project as_element() == None (Two-way invariant)"
    );

    // (c) After sink.finish(), mark_in_document_flags has cleared the
    //     Comment node’s IS_IN_DOCUMENT bit.
    assert!(
        !comment_node.is_in_document(),
        "Comment's IS_IN_DOCUMENT bit must be cleared after parse"
    );

    // (d) The Comment persists in the tree, among the body’s children.
    //     Previously it was detached by stripping, leaving only <p> as a
    //     direct child. Now the body children are `[Comment, <p>]` in source order.
    //     Use UFCS to disambiguate the `Dom::child_ids` and
    //     `TraversePartialTree::child_ids` methods. Choose the unfiltered Dom
    //     to inspect raw arena children; the next taffy filter test explicitly
    //     uses TraversePartialTree.
    let body_id = find_first_by_tag(doc, "body").expect("body exists");
    let body_kids: Vec<usize> = Dom::child_ids(doc, body_id)
        .map(|id| id.0 as usize)
        .collect();
    assert!(
        body_kids.contains(&comment_id),
        "Comment node must persist as a child of body (not physically stripped); body kids = {body_kids:?}"
    );

    // (e) The Comment does not leak into the taffy layout tree. TaffyChildIter
    //     uses the is_in_document() filter (raikiri-dom/src/taffy_impl.rs),
    //     so the body’s taffy child_count is 1 (only `<p>`). This behavior is
    //     already checked by `taffy_child_ids_and_count_filter_out_template_descendants`,
    //     but assert it here independently for the Comment/PI path as a
    //     regression check.
    use taffy::TraversePartialTree;
    let body_taffy_id = taffy::NodeId::from(body_id.0 as usize);
    assert_eq!(
        <raikiri_dom::Document as TraversePartialTree>::child_count(doc, body_taffy_id),
        1,
        "taffy tree must not leak Comment into body's child count"
    );
    let taffy_body_children: Vec<taffy::NodeId> =
        <raikiri_dom::Document as TraversePartialTree>::child_ids(doc, body_taffy_id).collect();
    assert!(
        !taffy_body_children.contains(&taffy::NodeId::from(comment_id)),
        "taffy body children must not include Comment"
    );
}

#[test]
fn parse_captures_quirks_mode_for_missing_doctype() {
    use raikiri_traits::QuirksMode;

    // Missing <!DOCTYPE html> triggers full quirks mode per HTML5 spec.
    let html = b"<html><body>x</body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(uncascaded.quirks_mode, QuirksMode::Quirks);
}

#[test]
fn parse_captures_no_quirks_for_standards_doctype() {
    use raikiri_traits::QuirksMode;

    let html = b"<!DOCTYPE html><html><body>x</body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(uncascaded.quirks_mode, QuirksMode::NoQuirks);
}

/// End-to-end regression, doctype-less (quirks-triggering) side: html5ever's
/// quirks-mode detection (`UncascadedDocument.quirks_mode`, asserted by
/// `parse_captures_quirks_mode_for_missing_doctype` above) must reach
/// `raikiri_dom::Document` itself (`RaikiriTreeSink::finish` calling
/// `Document::set_quirks_mode`) and, through `impl raikiri_style::StyleDom
/// for Document`, `StyleDom::quirks_mode()` — the accessor
/// `raikiri-style`'s cascade actually reads for id/class selector
/// ASCII-case-folding (CSS Selectors L4,
/// <https://www.w3.org/TR/selectors-4/#the-css-qualified-name>'s "In
/// quirks mode, ... matching of the ID and class attributes for the
/// purposes of selector matching must be done in an ASCII case-insensitive
/// manner"). Previously `Document` carried no quirks-mode field at all,
/// so this path always fell back to `StyleDom::quirks_mode`'s `NoQuirks`
/// default regardless of the parsed document's real doctype — meaning
/// `raikiri-style`'s quirks-mode ASCII-fold matching logic, though
/// correctly implemented, was unreachable from any real parsed document.
///
/// Asserting `StyleDom::quirks_mode()` alone would only prove the value
/// arrives at the accessor, not that cascade reads it and changes
/// matching behavior — so this test drives a real `build_rule_tree` +
/// `cascade` over an uppercase `class`/`id` attribute matched by a
/// lowercase selector, and checks the *computed style*, mirroring the
/// real-parse-survives-to-cascade idiom the `hr` / `a[href]` UA-rule
/// tests above use for UA CSS.
#[test]
fn parse_wires_quirks_mode_through_document_to_cascade_ascii_fold() {
    use raikiri_style::property::CssColor;
    use raikiri_style::{Origin, StyleDom, StyleQuirksMode};
    use raikiri_traits::QuirksMode;

    let html = b"<html><head><style>.foo { color: #00ff00 } \
                     #bar { color: #0000ff }</style></head><body>\
                     <p class=\"FOO\">a</p><span id=\"BAR\">b</span></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");

    // Accessor-level: the value reaches Document and StyleDom.
    assert_eq!(uncascaded.quirks_mode, QuirksMode::Quirks);
    assert_eq!(uncascaded.dom.quirks_mode(), QuirksMode::Quirks);
    assert_eq!(
        StyleDom::quirks_mode(&uncascaded.dom),
        StyleQuirksMode::Quirks
    );

    // Behavior-level: cascade actually reads it and ASCII-folds
    // `.foo`/`#bar` against `class="FOO"`/`id="BAR"`.
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    let span_id = find_first_by_tag(&uncascaded.dom, "span")
        .expect("<span> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].color,
        CssColor {
            r: 0,
            g: 0xFF,
            b: 0,
            a: 255,
        },
        "quirks mode: .foo must ASCII-fold-match class=\"FOO\" through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[span_id].color,
        CssColor {
            r: 0,
            g: 0,
            b: 0xFF,
            a: 255,
        },
        "quirks mode: #bar must ASCII-fold-match id=\"BAR\" through real parse+cascade"
    );
}

/// Standards-mode counterpart of
/// `parse_wires_quirks_mode_through_document_to_cascade_ascii_fold`: an
/// explicit `<!DOCTYPE html>` must reach `StyleDom::quirks_mode()` as
/// `NoQuirks` end-to-end, and — the claim that actually matters —
/// cascade's id/class matching must stay case-sensitive, so the same
/// `.foo`/`#bar` selectors must NOT match `class="FOO"`/`id="BAR"` and
/// the elements must stay at CSS-initial `color` (black). This is the
/// load-bearing half: it is the only assertion that can distinguish
/// "cascade read the real (`NoQuirks`) value" from "cascade fell back
/// to `StyleDom::quirks_mode`'s `NoQuirks` default and happened to
/// agree" — both bugs would incorrectly produce this document's Quirks
/// counterpart matching case-sensitively too, but only a real bug (not
/// integration `Document::quirks_mode` at all) would make *this* document's
/// case stay unmatched by coincidence, so the pair of tests together
/// is what actually pins the integration.
#[test]
fn parse_wires_no_quirks_through_document_to_cascade_case_sensitive() {
    use raikiri_style::property::CssColor;
    use raikiri_style::{Origin, StyleDom, StyleQuirksMode};
    use raikiri_traits::QuirksMode;

    let html = b"<!DOCTYPE html><html><head><style>.foo { color: #00ff00 } \
                     #bar { color: #0000ff }</style></head><body>\
                     <p class=\"FOO\">a</p><span id=\"BAR\">b</span></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");

    assert_eq!(uncascaded.quirks_mode, QuirksMode::NoQuirks);
    assert_eq!(uncascaded.dom.quirks_mode(), QuirksMode::NoQuirks);
    assert_eq!(
        StyleDom::quirks_mode(&uncascaded.dom),
        StyleQuirksMode::NoQuirks
    );

    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    let span_id = find_first_by_tag(&uncascaded.dom, "span")
        .expect("<span> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].color,
        CssColor::BLACK,
        "no-quirks mode: .foo must NOT match class=\"FOO\" (case-sensitive) through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[span_id].color,
        CssColor::BLACK,
        "no-quirks mode: #bar must NOT match id=\"BAR\" (case-sensitive) through real parse+cascade"
    );
}

#[test]
fn parse_survives_table_foster_parenting() {
    // Foster parenting of text directly under <table> is a typical case
    // that triggers append_before_sibling(AppendText(...)) in html5ever.
    // Only verify that parsing completes without panicking.
    let html = b"<table>stray text<tr><td>x</td></tr></table>";
    let opts = empty_options();
    let _ = parse(&html[..], &opts).expect("parse should not panic");
}

#[test]
fn parse_survives_deeply_nested_html() {
    // Build ~5000 nested <div> — recursive walker would stack overflow.
    // Iterative walker completes fine.
    let mut html = String::new();
    let depth = 5000;
    for _ in 0..depth {
        html.push_str("<div>");
    }
    html.push_str("hello");
    for _ in 0..depth {
        html.push_str("</div>");
    }
    let opts = empty_options();
    let _ = parse(html.as_bytes(), &opts).expect("parse ok — walkers must handle deep nesting");
}

#[test]
fn parse_maps_reader_invaliddata_to_io_not_encoding() {
    use raikiri_traits::ParseError;
    use std::io::{Error, ErrorKind, Read};

    /// Reader that returns InvalidData with an I/O-shaped message
    /// (not a UTF-8 error). Under the old read_to_string mapping this
    /// would be misclassified as ParseError::Encoding; the new
    /// read_to_end + from_utf8 path correctly reports ParseError::Io.
    struct BadKindReader;
    impl Read for BadKindReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(Error::new(ErrorKind::InvalidData, "disk sector unreadable"))
        }
    }

    let opts = empty_options();
    let err = parse(BadKindReader, &opts).expect_err("reader failure should error");
    assert!(
        matches!(err, ParseError::Io(_)),
        "InvalidData from reader must map to ParseError::Io, not Encoding (got {err:?})"
    );
}

#[test]
fn parse_extracts_multiple_style_blocks_in_document_order() {
    // Multiple <style> blocks — verify source-order preserved so cascade
    // tie-breaking (equal specificity → last-wins) works correctly.
    let html = b"<html><head>\
                     <style>p{color:red}</style>\
                     <style>p{color:blue}</style>\
                     <style>p{color:green}</style>\
                     </head><body><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![
            String::from("p{color:red}"),
            String::from("p{color:blue}"),
            String::from("p{color:green}"),
        ]
    );
}

#[test]
fn parse_skips_style_inside_template_element() {
    // <template> contents are inert per spec — <style> inside must not
    // appear in stylesheet_sources. Minimal fix (skip template subtree
    // during extraction). Full template-fragment isolation is a
    // separate, still-open task.
    let html = b"<html><head>\
                     <template><style>p{color:red}</style></template>\
                     <style>p{color:blue}</style>\
                     </head><body><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    // Only the top-level <style> should appear.
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![String::from("p{color:blue}")]
    );
}

#[test]
fn stylesheet_collection_sees_style_appended_directly_under_template() {
    // A `<style>` appended directly under a `<template>` element (the DOM
    // `appendChild` shape scripts use) is an ordinary light-DOM child: it
    // stays in-document, so head stylesheet collection must see it. A
    // `<style>` in the detached contents fragment stays inert and must not
    // be collected.
    use crate::sink::{HeadStylesheetSource, collect_head_stylesheet_sources};

    let mut doc = raikiri_dom::Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", taffy::Style::default(), None::<&str>);
    let tmpl = doc.append_element(
        Some(head),
        "template",
        taffy::Style::default(),
        None::<&str>,
    );
    let real_style = doc.append_element(Some(tmpl), "style", taffy::Style::default(), None::<&str>);
    doc.append_text(real_style, "p{color:green}");
    let frag = doc.allocate_template_fragment_root(tmpl);
    let inert_style =
        doc.append_element(Some(frag), "style", taffy::Style::default(), None::<&str>);
    doc.append_text(inert_style, "p{color:red}");
    doc.mark_in_document_flags();

    let sources = collect_head_stylesheet_sources(&doc);
    assert_eq!(
        sources.len(),
        1,
        "only the real <template> child <style> must be collected, not the fragment one"
    );
    match &sources[0] {
        HeadStylesheetSource::Inline { node_id } => assert_eq!(
            node_id.0, real_style as u64,
            "the collected <style> must be the real child"
        ),
        HeadStylesheetSource::External { .. } => {
            panic!("a text <style> must collect as Inline, not External")
        }
    }
}

#[test]
fn parse_marks_template_descendants_out_of_document() {
    // The <template> element itself belongs to the flat tree, so
    // is_in_document() is true. Check that parsing populates the bits for its
    // descendants (elements and text) as false.
    //
    // Without a mark_in_document_flags phase in the sink, the default true
    // bits on descendants would remain set, causing this test to fail.
    let html = b"<html><head></head><body>\
                     <template><p id=\"inner\">hi</p></template>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let doc = &uncascaded.dom;

    let mut saw_template = false;
    let mut saw_inner_p = false;
    let mut saw_inner_text = false;
    for id_u in 0..doc.node_count() {
        let id = raikiri_traits::NodeId::new(id_u as u64);
        let n = doc.node(id).expect("in-range");
        if let Some(el) = n.as_element() {
            match el.tag_name() {
                "template" => {
                    assert!(
                        n.is_in_document(),
                        "template element itself must be in document"
                    );
                    saw_template = true;
                }
                "p" if el.id() == Some("inner") => {
                    assert!(
                        !n.is_in_document(),
                        "<p> inside <template> must be out of document"
                    );
                    saw_inner_p = true;
                }
                _ => {}
            }
        }
        if n.text_content() == Some("hi") {
            assert!(
                !n.is_in_document(),
                "text inside <template> must be out of document"
            );
            saw_inner_text = true;
        }
    }
    assert!(saw_template, "template element should exist in parsed tree");
    assert!(
        saw_inner_p,
        "<p id=inner> should exist inside template subtree"
    );
    assert!(
        saw_inner_text,
        "'hi' text should exist inside template subtree"
    );
}

#[test]
fn parse_marks_body_children_in_document() {
    // Parsing normal HTML without a template leaves is_in_document() true
    // for every node. This regression check pins the default true value.
    let html = b"<html><head></head><body><p>hi</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let doc = &uncascaded.dom;
    for id_u in 0..doc.node_count() {
        let id = raikiri_traits::NodeId::new(id_u as u64);
        let n = doc.node(id).expect("in-range");
        assert!(
            n.is_in_document(),
            "node {} ({:?}) expected in_document",
            id_u,
            n.kind()
        );
    }
}

#[test]
fn parse_wires_template_contents_to_detached_fragment_root() {
    // When the sink creates a `<template>`, it eagerly allocates a fragment
    // root and wires its arena index into the template element’s
    // `template_contents` slot. html5ever then uses the result of
    // `get_template_contents(template_handle)` as the append parent, placing
    // contents under the fragment root rather than the template element. The
    // contents are no longer reachable from the Document root. Check:
    //
    // 1. template_contents returns Some(idx), with idx != template arena index
    //    (the fragment root has its own arena slot).
    // 2. The template element has no arena children: contents went to the
    //    fragment root (previously <span> sat directly under the template).
    // 3. The fragment root’s arena children include <span> (the reshape result).
    // 4. The template itself has is_in_document()=true; the fragment root,
    //    <span>, and its text have is_in_document()=false because they are not
    //    reachable from the Document root and mark_in_document_flags clears them.
    let html = b"<html><body><template><span>x</span></template></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let doc = &uncascaded.dom;

    // Find the template element by linear scan. raikiri-traits has no API
    // equivalent to get_template_contents, so read node.template_contents()
    // directly from the arena.
    let mut template_id: Option<usize> = None;
    let mut span_id: Option<usize> = None;
    for id_u in 0..doc.node_count() {
        let id = raikiri_traits::NodeId::new(id_u as u64);
        let n = doc.node(id).expect("in-range");
        if let Some(el) = n.as_element() {
            match el.tag_name() {
                "template" => template_id = Some(id_u),
                "span" => span_id = Some(id_u),
                _ => {}
            }
        }
    }
    let template_id = template_id.expect("<template> element should exist in arena");
    let span_id = span_id.expect("<span> element should exist in arena");

    let template_node = doc.get_node(template_id).expect("template node in-range");
    let frag_root_id = template_node
        .template_contents()
        .expect("template_contents slot must be populated by sink");

    // (1) The fragment root occupies a different arena slot from the template.
    assert_ne!(
        frag_root_id, template_id,
        "fragment root must be a distinct arena node, not the template element itself"
    );

    // (2) The template itself has no arena children; reshaping attached all
    //     contents to the fragment root.
    assert!(
        template_node.children.is_empty(),
        "template element's own arena children must be empty (contents belong to fragment root); got {:?}",
        template_node.children
    );

    // (3) The fragment root exists as NodeKind::DocumentFragment, not the
    //     former "#document-fragment" pseudo-tag Element. Its as_element()
    //     and tag_name() return None, while its children slot holds <span>.
    let frag_root = doc
        .get_node(frag_root_id)
        .expect("fragment root should exist in arena");
    assert_eq!(
        frag_root.kind(),
        NodeKind::DocumentFragment,
        "fragment root must be NodeKind::DocumentFragment (replaces the old '#document-fragment' pseudo-tag)"
    );
    assert_eq!(
        frag_root.tag_name(),
        None,
        "fragment root must not carry a tag_name (Two-way invariant: non-Element kind → tag_name None)"
    );
    // Also confirm as_element() == None through NodeRef, pinning the
    // two-way invariant end to end at the dom_impl surface.
    {
        let frag_ref = doc
            .node(raikiri_traits::NodeId::new(frag_root_id as u64))
            .expect("fragment root NodeRef");
        assert!(
            frag_ref.as_element().is_none(),
            "fragment root as_element() must be None (kind = DocumentFragment ⇒ Element downcast fails)"
        );
    }
    assert!(
        frag_root.children.contains(&span_id),
        "fragment root children must include the <span>; got {:?}",
        frag_root.children
    );

    // (4) The template is in-document; the fragment root and <span> are not.
    //     They are unreachable from the Document root, so step 2 of
    //     `mark_in_document_flags` does not set their bits.
    assert!(
        template_node.is_in_document(),
        "template element itself must be in_document"
    );
    assert!(
        !frag_root.is_in_document(),
        "fragment root must be out-of-document (detached from Document root)"
    );
    let span_node = doc.get_node(span_id).expect("span in-range");
    assert!(
        !span_node.is_in_document(),
        "<span> under fragment root must be out-of-document"
    );
    // The <span> text child is also out-of-document.
    for &c in &span_node.children {
        let child = doc.get_node(c).expect("span child in-range");
        assert!(
            !child.is_in_document(),
            "text under <span> in template must be out-of-document"
        );
    }
}

#[test]
fn parse_marks_nested_template_descendants_out_of_document() {
    // Even with deep nesting (template > div > span > text), the
    // in_document bit propagates across the entire subtree. Pin that a
    // single-pass DFS carries the in_template state correctly.
    let html = b"<html><body>\
                     <template><div><span>x</span></div></template>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let doc = &uncascaded.dom;
    for id_u in 0..doc.node_count() {
        let id = raikiri_traits::NodeId::new(id_u as u64);
        let n = doc.node(id).expect("in-range");
        if let Some(el) = n.as_element() {
            match el.tag_name() {
                "div" | "span" => assert!(
                    !n.is_in_document(),
                    "<{}> inside <template> must be out of document",
                    el.tag_name()
                ),
                _ => {}
            }
        }
        if n.text_content() == Some("x") {
            assert!(
                !n.is_in_document(),
                "text 'x' inside <template> must be out of document"
            );
        }
    }
}

#[test]
fn parse_then_cascade_skips_template_descendants() {
    // Regression check for the silent bug where cascade skipped the template
    // subtree. Detailed checks of the cascaded map’s shape belong in
    // raikiri-style unit tests (add them separately if missing). Here, check
    // that parsing then cascading can visit an element inside a template
    // without error or panic, and that its bits stay unchanged across cascade.
    //
    // Additional check: a broken is_in_document() gate in the cascade walk
    // may violate the `cascade.computed.len() == dom.node_count()` contract.
    // `html_document_cascade_populated_after_construct` in raikiri/src/lib.rs
    // previously checked this only for documents without templates. Later
    // phases index `cascade.computed[idx]` directly by node_id; a mismatch
    // would panic out of bounds. raikiri-style unit tests using TestDoc
    // cannot catch this: its default is_in_document() is always true. Only
    // a parsed document can produce false bits; this integration test covers
    // that case.
    let html = b"<html><head><style>p { color: red }</style></head><body>\
                     <p>outer</p>\
                     <template><p id=\"inner\">inner</p></template>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree)
        .expect("cascade must not error / panic on template subtree");

    // Contract: cascade.computed.len() must equal dom.node_count(). Even
    // when the cascade gate skips template descendants, preserve the indexing
    // contract: raikiri-dom and raikiri-paint index directly by node_id.
    assert_eq!(
        cascade.computed.len(),
        uncascaded.dom.node_count(),
        "cascade.computed.len() must equal node_count() even with template descendants"
    );

    // After cascade, the inner <p> remains out-of-document: cascade must
    // not change the bit.
    //
    // Check the actual ComputedValues of both the outer and inner <p>.
    // If the gate works, `p { color: red }` applies to the outer <p>, yielding
    // CssColor { r:255, g:0, b:0 }, while the inner <p> keeps its initial
    // CssColor::BLACK = { r:0, g:0, b:0 }. If both cascade gates are removed,
    // the inner <p> also becomes red. These paired assertions prove that the
    // gate actually runs.
    use raikiri_style::property::CssColor;
    const RED: CssColor = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };

    let mut outer_p_id: Option<usize> = None;
    let mut inner_p_id: Option<usize> = None;
    for id_u in 0..uncascaded.dom.node_count() {
        let id = raikiri_traits::NodeId::new(id_u as u64);
        let n = uncascaded.dom.node(id).unwrap();
        if let Some(el) = n.as_element()
            && el.tag_name() == "p"
        {
            if el.id() == Some("inner") {
                assert!(
                    !n.is_in_document(),
                    "inner <p> should remain out of document after cascade"
                );
                inner_p_id = Some(id_u);
            } else {
                outer_p_id = Some(id_u);
            }
        }
    }
    let outer_p_id = outer_p_id.expect("outer <p> should exist");
    let inner_p_id = inner_p_id.expect("<p id=inner> should exist inside template");

    // Index into cascade.computed for both nodes must not panic (proves
    // out`idx` was still written for the inert node despite the gate).
    let outer_cv = &cascade.computed[outer_p_id];
    let inner_cv = &cascade.computed[inner_p_id];

    assert_eq!(
        outer_cv.color, RED,
        "outer <p> should have red rule applied (in-document, rule matches)"
    );
    assert_eq!(
        inner_cv.color,
        CssColor::BLACK,
        "inner <p> should keep initial color (cascade gate skips template descendants)"
    );
}

#[test]
fn hr_ua_rule_overflow_hidden_survives_real_parse_and_cascade() {
    // No test anywhere pinned
    // that `hr`'s new `overflow: hidden;` UA rule (HTML LS
    // §the-hr-element-2) actually survives real cssparser parsing and
    // cascade, as opposed to just being literal text in
    // `MINIMAL_UA_CSS` (`minimal_ua_css_covers_required_display_block_selectors`
    // above only textually scans for `display: block`, not `overflow`).
    // The `raikiri` umbrella's `build_cascaded.rs` sibling test
    // (`hr_is_display_block_border_inset_and_margin_via_ua_css`)
    // explicitly skips asserting `overflow` because `OverflowValue`/
    // `OverflowXY` aren't re-exported at the umbrella root yet
    // (`wall/umbrella`, correctly deferred) — but `raikiri-html` depends
    // on `raikiri-style` directly, so this crate can check it today
    // without crossing that wall.
    use raikiri_style::Origin;
    use raikiri_style::property::OverflowValue;

    let html = b"<html><body><hr></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let hr_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "hr")
        })
        .expect("<hr> should exist");
    let overflow = cascade.computed[hr_id].overflow;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        overflow.x,
        OverflowValue::Hidden,
        "hr's UA rule overflow: hidden must reach computed.overflow.x through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        overflow.y,
        OverflowValue::Hidden,
        "hr's UA rule overflow: hidden must reach computed.overflow.y through real parse+cascade"
    );
}

#[test]
fn a_ua_rule_color_and_text_decoration_survives_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3's
    // `a:link, a:visited { color: #0000EE; text-decoration: underline; }`
    // is approximated here as `a[href] { color: #0000EE; text-decoration:
    // underline; }` (no `:link`/`:visited` — Non-Goal, see the UA rule's
    // comment in `minimal.css`; `:link` itself additionally requires an
    // `href` attribute per HTML LS §selector-link, gated here via
    // `[href]`). Same "survives real parse+cascade, not just literal
    // text" concern as the `hr` test above. Also pins the `[href]` gate
    // itself: a bare `<a id="anchor">` with no `href` (e.g. a fragment
    // target, not a link at all per spec) must stay at CSS-initial
    // (the original unconditional `a { }` rule had this gap).
    use raikiri_style::Origin;
    use raikiri_style::property::{CssColor, TextDecorationLine};

    let html = b"<html><body><a href=\"x\">link</a><a id=\"anchor\">bare</a></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let a_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "a" && el.attr("href").is_some())
        })
        .expect("<a href> should exist");
    let bare_a_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "a" && el.attr("href").is_none())
        })
        .expect("<a> without href should exist");
    let computed = &cascade.computed[a_id];
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        computed.color,
        CssColor {
            r: 0x00,
            g: 0x00,
            b: 0xEE,
            a: 255,
        },
        "a[href]'s UA rule color: #0000EE must reach computed.color through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        computed.text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "a[href]'s UA rule text-decoration: underline must reach computed.text_decoration_line through real parse+cascade"
    );

    // HTML LS §selector-link: an `a` with no `href` is not `:link` at
    // all — must NOT pick up the UA rule's color/text-decoration.
    let bare_computed = &cascade.computed[bare_a_id];
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        bare_computed.color,
        CssColor::BLACK,
        "a without href must stay at CSS-initial color, not the a[href] UA rule's #0000EE"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        bare_computed.text_decoration_line,
        TextDecorationLine::NONE,
        "a without href must stay at CSS-initial text-decoration, not the a[href] UA rule's underline"
    );
}

#[test]
fn a_href_empty_value_ua_rule_color_and_text_decoration_survives_real_parse_and_cascade() {
    // HTML LS §selector-link: "All a elements that have an href
    // attribute ... must match ... :link" — an empty `href=""` still
    // satisfies "have an href attribute" (presence, not a non-empty
    // value, is the gate), so `<a href="">` is spec-`:link` and must
    // pick up the same `a[href]` UA rule as a non-empty href. This is
    // the `[href]` attribute-presence-selector counterpart to the
    // `a[href]` test above, isolating the previously-broken case:
    // `Component::AttributeInNoNamespaceExists` matching depends on
    // `elem.attr("href").is_some()` distinguishing "present with empty
    // value" from "absent" — real DOM `attr()` now does.
    use raikiri_style::Origin;
    use raikiri_style::property::{CssColor, TextDecorationLine};

    let html = b"<html><body><a href=\"\">empty href link</a></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let a_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "a")
        })
        .expect("<a> should exist");
    let computed = &cascade.computed[a_id];
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        computed.color,
        CssColor {
            r: 0x00,
            g: 0x00,
            b: 0xEE,
            a: 255,
        },
        "a[href='']'s UA rule color: #0000EE must reach computed.color through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        computed.text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "a[href='']'s UA rule text-decoration: underline must reach computed.text_decoration_line through real parse+cascade"
    );
}

#[test]
fn lang_wildcard_selector_does_not_match_explicit_empty_lang_attribute_via_real_dom() {
    // HTML LS §3.2.6.2 "determine the language of a node": an explicit
    // `lang=""` resolves to "the primary language is unknown" — a
    // terminal state, distinct from "no lang attribute at all" (see
    // `raikiri_style::cascade::effective_language`'s doc). CSS Selectors
    // L4 §7.2, bikeshed source `selectors-4/Overview.bs`
    // `#the-lang-pseudo`, verbatim: "For this purpose, a wildcard
    // language range (\"*\") does not match elements whose language is
    // not tagged (e.g. `lang=\"\"`), but does match elements whose
    // language is tagged as undetermined (`lang=und`)." — an element
    // whose resolved content language is the empty string must NOT
    // match the wildcard range specifically (see the sibling test
    // below for the literal empty-string range `:lang("")`, which the
    // same quote's next sentence says MUST match this case).
    //
    // This can only be exercised through the real DOM: `TestDoc` (the
    // mock `raikiri-style` uses for its own unit tests) still collapses
    // `lang=""` to attribute-absent by design, so it can never produce
    // the `Some("")` resolved-language state this regression is about.
    // Only `raikiri-dom::dom_impl::ElementRef`, which tracks attribute
    // presence independent of value, can.
    //
    // The wildcard range must be **quoted** (`:lang("*")`, not bare
    // `:lang(*)`) — `*` is a CSS delimiter token, not a valid `<ident>`
    // character, so the unquoted form fails to parse as a `:lang()`
    // argument (verified empirically: an unquoted `:lang(*)` rule here
    // is simply dropped as an invalid selector, so this test uses the
    // quoted form that's this crate's `:lang()` argument parser
    // actually accepts, per `expect_ident_or_string()`).
    use raikiri_style::property::CssColor;

    let html = b"<html><body>\
                     <style>:lang(\"*\") { color: #FF0000; }</style>\
                     <div lang=\"ja\">has lang</div>\
                     <div lang=\"\">empty lang</div>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let find_div_with_lang = |lang_value: &str| {
        (0..uncascaded.dom.node_count())
            .find(|&id_u| {
                uncascaded
                    .dom
                    .node(raikiri_traits::NodeId::new(id_u as u64))
                    .unwrap()
                    .as_element()
                    .is_some_and(|el| el.tag_name() == "div" && el.attr("lang") == Some(lang_value))
            })
            .unwrap_or_else(|| panic!("<div lang=\"{lang_value}\"> should exist"))
    };

    let has_lang_id = find_div_with_lang("ja");
    let empty_lang_id = find_div_with_lang("");

    // Positive control: a real, non-empty lang must match :lang("*") —
    // without this, a silently-dropped/unparsed rule would make the
    // negative assertion below pass vacuously.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[has_lang_id].color,
        CssColor {
            r: 0xFF,
            g: 0x00,
            b: 0x00,
            a: 255,
        },
        ":lang(\"*\") must match an element with a real, non-empty lang attribute"
    );
    // The regression under test: explicit lang="" must NOT match
    // :lang("*").
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[empty_lang_id].color,
        CssColor::BLACK,
        "explicit lang=\"\" (HTML LS: primary language unknown) must not match :lang(\"*\")"
    );
}

#[test]
fn lang_empty_string_range_matches_explicit_empty_lang_attribute_via_real_dom() {
    // Complements the wildcard test above with the other half of the
    // same spec sentence (CSS Selectors L4 §7.2, bikeshed source
    // `selectors-4/Overview.bs` `#the-lang-pseudo`, verbatim,
    // immediately following the wildcard sentence quoted there): "A
    // language range consisting of an empty string (`:lang(\"\")`)
    // matches (only) elements whose language is not tagged." — unlike
    // the wildcard range `"*"`, the literal empty-string range `""`
    // MUST match an element whose resolved content language is the
    // empty string (explicit `lang=""`). Real-DOM specifically for the
    // explicit-`lang=\"\"` case: `TestDoc`'s `attr()` mock collapses any
    // empty-value attribute to `None` (`raikiri_style::test_dom`'s
    // `TestElementRef::attr` doc), so it can never observe an explicit
    // `lang=""` as present — only `raikiri-dom::dom_impl::ElementRef`,
    // which tracks attribute presence independent of value, can.
    //
    // `<body>` carries an explicit non-empty `lang=\"fr\"` (rather than
    // being left untagged like the wildcard test above) to isolate what
    // this test actually exercises. Left untagged, `<html>`/`<body>`
    // would themselves resolve to the empty content language via
    // `raikiri_style::cascade::effective_language`'s exhausted-chain
    // terminal case — so if the real-DOM empty-value-attribute bug this
    // test guards against ever regressed (making `<div lang=\"\">`'s own
    // attribute read back as absent), that div would still fall through
    // to the ancestor walk and land on the *same* empty-string result
    // via `<body>`'s own absence, making the assertion below pass for
    // the wrong reason. With `<body lang=\"fr\">`, that fallback path
    // resolves to `\"fr\"` instead, so the assertion below only passes
    // if the div's own explicit `lang=\"\"` attribute was read
    // correctly.
    //
    // `background-color`, not `color`: `color` is inherited (CSS
    // Cascade 5 §7.2 <https://www.w3.org/TR/css-cascade-5/#inheriting>,
    // same citation `raikiri_style::computed::ComputedValues`'s own doc
    // uses), so a `color: red` rule matched on an ancestor would reach
    // the `lang=\"ja\"` div through ordinary inheritance regardless of
    // whether `:lang(\"\")` matched that div itself — the same pitfall
    // `raikiri_style::cascade`'s own
    // `root_pseudo_class_matches_the_document_root_element_only` test
    // comment documents. `background-color` is not inherited (CSS
    // Backgrounds 3 §2.2), so it isolates each div's own match status.
    use raikiri_style::property::CssColor;

    let html = b"<html><body lang=\"fr\">\
                     <style>:lang(\"\") { background-color: #FF0000; }</style>\
                     <div lang=\"ja\">has lang</div>\
                     <div lang=\"\">empty lang</div>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let find_div_with_lang = |lang_value: &str| {
        (0..uncascaded.dom.node_count())
            .find(|&id_u| {
                uncascaded
                    .dom
                    .node(raikiri_traits::NodeId::new(id_u as u64))
                    .unwrap()
                    .as_element()
                    .is_some_and(|el| el.tag_name() == "div" && el.attr("lang") == Some(lang_value))
            })
            .unwrap_or_else(|| panic!("<div lang=\"{lang_value}\"> should exist"))
    };

    let has_lang_id = find_div_with_lang("ja");
    let empty_lang_id = find_div_with_lang("");

    // Negative control: a real, non-empty lang must NOT match
    // :lang("") — without this, an over-broad match (e.g. a bug that
    // treated "" as matching everything) would make the assertion
    // below pass vacuously.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[has_lang_id].background_color,
        CssColor::TRANSPARENT,
        ":lang(\"\") must not match an element with a real, non-empty lang attribute"
    );
    // The regression under test: explicit lang="" MUST match
    // :lang("") — the element's language is "not tagged" per the
    // quoted spec text, which is exactly what :lang("") selects for.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[empty_lang_id].background_color,
        CssColor {
            r: 0xFF,
            g: 0x00,
            b: 0x00,
            a: 255,
        },
        "explicit lang=\"\" (HTML LS: primary language unknown) must match :lang(\"\")"
    );
}

#[test]
fn phrasing_content_ua_rules_survive_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3's
    //   b, strong { font-weight: bolder; }
    //   big { font-size: larger; }
    //   small { font-size: smaller; }
    //   mark { background: yellow; color: black; }
    //   ins, u { text-decoration: underline; }
    // all landed. The 5-element `cite, dfn, em, i, var { font-style:
    // italic; }` rule also landed — see
    // `cite_dfn_em_i_var_font_style_ua_rule_survives_real_parse_and_cascade`
    // below for its dedicated real-parse+cascade coverage. sub/sup's two rules
    // (`vertical-align: sub`/`super`) also landed — covered separately
    // by `sub_sup_ua_rules_survive_real_parse_and_cascade` below. Same
    // "survives real parse+cascade, not just literal text in
    // MINIMAL_UA_CSS" concern as the hr / `a[href]` tests above.
    use raikiri_style::Origin;
    use raikiri_style::property::{CssColor, TextDecorationLine};

    let html = b"<html><body><strong>x</strong><b>x</b><big>x</big><small>x</small>\
                     <mark>x</mark><ins>x</ins><u>x</u></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let strong_id = find_first_by_tag(&uncascaded.dom, "strong")
        .expect("<strong> should exist")
        .0 as usize;
    let b_id = find_first_by_tag(&uncascaded.dom, "b")
        .expect("<b> should exist")
        .0 as usize;
    let big_id = find_first_by_tag(&uncascaded.dom, "big")
        .expect("<big> should exist")
        .0 as usize;
    let small_id = find_first_by_tag(&uncascaded.dom, "small")
        .expect("<small> should exist")
        .0 as usize;
    let mark_id = find_first_by_tag(&uncascaded.dom, "mark")
        .expect("<mark> should exist")
        .0 as usize;
    let ins_id = find_first_by_tag(&uncascaded.dom, "ins")
        .expect("<ins> should exist")
        .0 as usize;
    let u_id = find_first_by_tag(&uncascaded.dom, "u")
        .expect("<u> should exist")
        .0 as usize;

    // strong, b { font-weight: bolder } — CSS Fonts 4 §2.2.1's
    // relative-weight table resolves an inherited 400 (the CSS-initial
    // `normal`) to 700.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[strong_id].font_weight, 700.0,
        "strong's UA rule font-weight: bolder must resolve to 700 against the inherited initial 400 through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[b_id].font_weight, 700.0,
        "b's UA rule font-weight: bolder must resolve to 700 against the inherited initial 400 through real parse+cascade"
    );

    // big { font-size: larger } — simple-ratio (1.2) branch applied
    // against the inherited initial 16px.
    let big_font_size = cascade.computed[big_id].font_size.0;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (big_font_size - 16.0 * 1.2).abs() < 0.0001,
        "big's UA rule font-size: larger must resolve to 16px * 1.2 through real parse+cascade, got {big_font_size}"
    );

    // small { font-size: smaller } — simple-ratio (1.2) branch applied
    // against the inherited initial 16px.
    let small_font_size = cascade.computed[small_id].font_size.0;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (small_font_size - 16.0 / 1.2).abs() < 0.0001,
        "small's UA rule font-size: smaller must resolve to 16px / 1.2 through real parse+cascade, got {small_font_size}"
    );

    // mark { background-color: yellow; color: black; } (background-color
    // longhand substituted for the unimplemented `background` shorthand,
    // see minimal.css comment).
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[mark_id].background_color,
        CssColor {
            r: 0xFF,
            g: 0xFF,
            b: 0x00,
            a: 255,
        },
        "mark's UA rule background-color: yellow must reach computed.background_color through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[mark_id].color,
        CssColor::BLACK,
        "mark's UA rule color: black must reach computed.color through real parse+cascade"
    );

    // ins, u { text-decoration: underline; }
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[ins_id].text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "ins's UA rule text-decoration: underline must reach computed.text_decoration_line through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[u_id].text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "u's UA rule text-decoration: underline must reach computed.text_decoration_line through real parse+cascade"
    );
}

#[test]
fn preformatted_ua_rule_disables_text_autospace() {
    // CSS Text 4 Appendix C sets `text-spacing: none` on preformatted
    // elements; minimal.css lands its `text-autospace` longhand.
    use raikiri_style::Origin;
    use raikiri_style::property::TextAutospace;

    let html = b"<html><body><p>x</p><pre>x</pre><code>x</code><kbd>x</kbd>\
                     <samp>x</samp><tt>x</tt><listing>x</listing><xmp>x</xmp>\
                     </body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
    let autospace = |tag: &str| {
        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        cascade.computed[id].text_autospace
    };

    assert_eq!(autospace("p"), TextAutospace::Normal);
    for tag in ["pre", "code", "kbd", "samp", "tt", "listing", "xmp"] {
        assert_eq!(autospace(tag), TextAutospace::NoAutospace, "<{tag}>");
    }
}

#[test]
fn del_s_strike_and_abbr_acronym_ua_rules_survive_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3's
    //   del, s, strike { text-decoration: line-through; }
    //   `abbr[title], acronym[title] { text-decoration: dotted underline; }`
    // Same "survives real parse+cascade, not just literal text in
    // MINIMAL_UA_CSS" concern as the sibling UA rule tests above. A
    // separate test rather than folding into
    // `phrasing_content_ua_rules_survive_real_parse_and_cascade` above,
    // to keep this addition to `minimal.css`'s `ins, u` group
    // self-contained.
    use raikiri_style::Origin;
    use raikiri_style::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

    let html = b"<html><body><del>x</del><s>x</s><strike>x</strike>\
                     <abbr title=\"x\">x</abbr><abbr>x</abbr>\
                     <acronym title=\"x\">x</acronym></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let del_id = find_first_by_tag(&uncascaded.dom, "del")
        .expect("<del> should exist")
        .0 as usize;
    let s_id = find_first_by_tag(&uncascaded.dom, "s")
        .expect("<s> should exist")
        .0 as usize;
    let strike_id = find_first_by_tag(&uncascaded.dom, "strike")
        .expect("<strike> should exist")
        .0 as usize;
    let acronym_id = find_first_by_tag(&uncascaded.dom, "acronym")
        .expect("<acronym> should exist")
        .0 as usize;
    let abbr_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "abbr" && el.attr("title").is_some())
        })
        .expect("<abbr title> should exist") as usize;

    // del, s, strike { text-decoration: line-through; }
    for (name, id) in [("del", del_id), ("s", s_id), ("strike", strike_id)] {
        assert_eq!(
            cascade.computed[id].text_decoration_line,
            TextDecorationLine::LINE_THROUGH,
            "{name}'s UA rule text-decoration: line-through must reach \
                 computed.text_decoration_line through real parse+cascade"
        );
    }

    // `abbr[title], acronym[title] { text-decoration: dotted underline; }`
    // — both line and style components must land, and color stays at
    // its own initial (`currentcolor`, the shorthand didn't mention it).
    // Both selector branches are exercised independently (not just the
    // shared declaration via one tag) in case an attribute-selector
    // match behaves differently per tag name.
    assert_eq!(
        cascade.computed[abbr_id].text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "abbr[title]'s UA rule text-decoration: dotted underline must \
             reach computed.text_decoration_line through real parse+cascade"
    );
    assert_eq!(
        cascade.computed[abbr_id].text_decoration_style,
        TextDecorationStyle::Dotted,
        "abbr[title]'s UA rule text-decoration: dotted underline must \
             reach computed.text_decoration_style through real parse+cascade"
    );
    assert_eq!(
        cascade.computed[abbr_id].text_decoration_color,
        TextDecorationColor::CurrentColor
    );
    assert_eq!(
        cascade.computed[acronym_id].text_decoration_line,
        TextDecorationLine::UNDERLINE,
        "acronym[title]'s UA rule text-decoration: dotted underline must \
             reach computed.text_decoration_line through real parse+cascade"
    );
    assert_eq!(
        cascade.computed[acronym_id].text_decoration_style,
        TextDecorationStyle::Dotted,
        "acronym[title]'s UA rule text-decoration: dotted underline must \
             reach computed.text_decoration_style through real parse+cascade"
    );
    assert_eq!(
        cascade.computed[acronym_id].text_decoration_color,
        TextDecorationColor::CurrentColor
    );

    // HTML LS §phrasing-content-3's `[title]` gate: an `abbr` with no
    // `title` attribute is not matched by `abbr[title]` at all — must
    // stay at CSS-initial, same "attribute gate must actually gate"
    // concern as the `a[href]` test above.
    let bare_abbr_id = (0..uncascaded.dom.node_count())
        .find(|&id_u| {
            uncascaded
                .dom
                .node(raikiri_traits::NodeId::new(id_u as u64))
                .unwrap()
                .as_element()
                .is_some_and(|el| el.tag_name() == "abbr" && el.attr("title").is_none())
        })
        .expect("<abbr> without title should exist");
    assert_eq!(
        cascade.computed[bare_abbr_id].text_decoration_line,
        TextDecorationLine::NONE,
        "abbr without title must stay at CSS-initial text-decoration, \
             not the abbr[title] UA rule's dotted underline"
    );
}

#[test]
fn sub_sup_ua_rules_survive_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3:
    //   sub { vertical-align: sub; }
    //   sup { vertical-align: super; }
    //   sub, sup { line-height: normal; font-size: smaller; }
    // (`minimal.css`'s sub/sup comment has the full per-declaration
    // rationale, including why `vertical-align` is scoped to
    // `sub`/`super` only.)
    //
    // `line-height: normal` is also the CSS-initial value, so asserting
    // it directly on a bare `<sub>`/`<sup>` under an all-initial
    // ancestor chain would pass whether or not the UA rule actually
    // fired. Wrapping both in an author-styled `line-height: 3`
    // container makes the assertion discriminating instead: CSS
    // Cascading L4 has any declared value (any origin) beat an
    // inherited one, so the UA rule's `normal` must win over the
    // inherited `3` on `sub`/`sup`, while a plain `<span>` sibling with
    // no matching UA rule keeps the inherited `3` (negative control).
    use raikiri_style::Origin;
    use raikiri_style::property::VerticalAlign;
    use raikiri_style::resolve::ComputedLineHeight;

    let html = b"<html><body><div style=\"line-height: 3\">\
                     <sub>x</sub><sup>x</sup><span>x</span></div></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let sub_id = find_first_by_tag(&uncascaded.dom, "sub")
        .expect("<sub> should exist")
        .0 as usize;
    let sup_id = find_first_by_tag(&uncascaded.dom, "sup")
        .expect("<sup> should exist")
        .0 as usize;
    let span_id = find_first_by_tag(&uncascaded.dom, "span")
        .expect("<span> should exist")
        .0 as usize;

    // sub { vertical-align: sub; } / sup { vertical-align: super; }
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[sub_id].vertical_align,
        VerticalAlign::Sub,
        "sub's UA rule vertical-align: sub must reach computed.vertical_align through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[sup_id].vertical_align,
        VerticalAlign::Super,
        "sup's UA rule vertical-align: super must reach computed.vertical_align through real parse+cascade"
    );

    // sub, sup { line-height: normal; ... } must override the inherited
    // line-height: 3 from the wrapper div.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[sub_id].line_height,
        ComputedLineHeight::Normal,
        "sub's UA rule line-height: normal must override the inherited line-height: 3 through real parse+cascade"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[sup_id].line_height,
        ComputedLineHeight::Normal,
        "sup's UA rule line-height: normal must override the inherited line-height: 3 through real parse+cascade"
    );
    // negative control: a sibling with no matching UA rule keeps the
    // inherited line-height: 3 — proves the wrapper's declaration
    // actually propagates, so the two overrides above are
    // discriminating and not vacuously true.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[span_id].line_height,
        ComputedLineHeight::Number(3.0),
        "span (no UA rule) must inherit line-height: 3 from the wrapper div"
    );

    // sub, sup { font-size: smaller; } — same simple-ratio (1.2) branch
    // as `small` above, applied against the inherited initial 16px
    // (the wrapper div's line-height declaration does not affect
    // font-size).
    let sub_font_size = cascade.computed[sub_id].font_size.0;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (sub_font_size - 16.0 / 1.2).abs() < 0.0001,
        "sub's UA rule font-size: smaller must resolve to 16px / 1.2 through real parse+cascade, got {sub_font_size}"
    );
    let sup_font_size = cascade.computed[sup_id].font_size.0;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (sup_font_size - 16.0 / 1.2).abs() < 0.0001,
        "sup's UA rule font-size: smaller must resolve to 16px / 1.2 through real parse+cascade, got {sup_font_size}"
    );
}

#[test]
fn cite_dfn_em_i_var_font_style_ua_rule_survives_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3's 5-element
    //   cite, dfn, em, i, var { font-style: italic; }
    // — now combined with §flow-content-3's separate `address` rule
    // into one physical `address, cite, dfn, em, i, var` CSS rule (see
    // minimal.css's comment on that rule), but this test covers only
    // the 5 elements this spec rule itself names; `address`'s coverage
    // is `address_font_style_ua_rule_survives_real_parse_and_cascade`
    // below. Same "survives real parse+cascade, not just literal text
    // in MINIMAL_UA_CSS" concern as the hr / `a[href]` /
    // `phrasing_content_ua_rules_survive_real_parse_and_cascade` tests
    // above — a separate test function (rather than folded into that
    // one) so this rule's coverage doesn't collide with concurrent
    // edits to the same shared test.
    use raikiri_style::Origin;
    use raikiri_style::property::FontStyle;

    let html = b"<html><body><cite>x</cite><dfn>x</dfn><em>x</em><i>x</i>\
                     <var>x</var><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    for tag in ["cite", "dfn", "em", "i", "var"] {
        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            cascade.computed[id].font_style,
            FontStyle::Italic,
            "{tag}'s UA rule font-style: italic must reach computed.font_style through real parse+cascade"
        );
    }

    // Contrast: an element the UA rule does not target must stay at
    // CSS-initial `normal` (HTML LS §phrasing-content-3's selector is
    // exactly the 5 elements above, not every element).
    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].font_style,
        FontStyle::Normal,
        "p must stay at CSS-initial font-style: normal, not the cite/dfn/em/i/var UA rule's italic"
    );
}

#[test]
fn address_font_style_ua_rule_survives_real_parse_and_cascade() {
    // HTML LS §flow-content-3's separate `address { font-style: italic; }`
    // rule (distinct from §phrasing-content-3's cite/dfn/em/i/var rule
    // above, though both specify the same declaration) lands folded into
    // the same physical CSS rule as cite/dfn/em/i/var — see minimal.css's
    // comment on that rule for why. Same "survives real parse+cascade,
    // not just literal text in MINIMAL_UA_CSS" concern as
    // `cite_dfn_em_i_var_font_style_ua_rule_survives_real_parse_and_cascade`
    // above — a separate test function, mirroring that test's own
    // separate-function convention so this rule's coverage doesn't
    // collide with concurrent edits to a shared test.
    use raikiri_style::Origin;
    use raikiri_style::property::FontStyle;

    let html = b"<html><body><address>x</address><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    let address_id = find_first_by_tag(&uncascaded.dom, "address")
        .expect("<address> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[address_id].font_style,
        FontStyle::Italic,
        "address's UA rule font-style: italic must reach computed.font_style through real parse+cascade"
    );

    // Contrast: an element the UA rule does not target must stay at
    // CSS-initial `normal` (HTML LS §flow-content-3's address rule
    // targets only address, not every element).
    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].font_style,
        FontStyle::Normal,
        "p must stay at CSS-initial font-style: normal, not address's UA rule's italic"
    );
}

#[test]
fn code_kbd_samp_tt_ua_rule_survives_real_parse_and_cascade() {
    // HTML LS §phrasing-content-3's
    //   code, kbd, samp, tt { font-family: monospace; }
    // Same "survives real parse+cascade, not just literal text in
    // MINIMAL_UA_CSS" concern as the hr / `a[href]` /
    // `cite_dfn_em_i_var_font_style_ua_rule_survives_real_parse_and_cascade`
    // tests above — a separate test function, mirroring those sibling
    // tests' own separate-function convention.
    use raikiri_style::Origin;

    let html = b"<html><body><code>x</code><kbd>x</kbd><samp>x</samp>\
                     <tt>x</tt><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    for tag in ["code", "kbd", "samp", "tt"] {
        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            cascade.computed[id].font_family[0].to_string(),
            "monospace",
            "{tag}'s UA rule font-family: monospace must reach computed.font_family through real parse+cascade"
        );
    }

    // Contrast: an element the UA rule does not target must stay at
    // CSS-initial `serif` (HTML LS §phrasing-content-3's selector is
    // exactly the 4 elements above, not every element).
    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].font_family[0].to_string(),
        "serif",
        "p must stay at CSS-initial font-family: serif, not the code/kbd/samp/tt UA rule's monospace"
    );
}

#[test]
fn listing_plaintext_pre_xmp_font_family_and_white_space_survives_real_parse_and_cascade() {
    // HTML LS §flow-content-3's
    //   listing, plaintext, pre, xmp { font-family: monospace; white-space: pre; }
    // Both declarations land in MINIMAL_UA_CSS — same test shape as
    // `code_kbd_samp_tt_ua_rule_survives_real_parse_and_cascade` above,
    // extended with a `white_space` assertion alongside `font_family`
    // now that `white-space` parses (CSS Text 3 §3, `raikiri_style::property::WhiteSpace`).
    //
    // Each tag is parsed in its own isolated document rather than one
    // shared document like the sibling test above: html5ever's tokenizer
    // switches to the PLAINTEXT state on a `<plaintext>` start tag, after
    // which every remaining byte of the document becomes literal text —
    // no further element, including a following `<pre>`/`<xmp>`/`<p>`,
    // would parse as an element at all.
    use raikiri_style::Origin;
    use raikiri_style::property::WhiteSpace;

    for tag in ["pre", "listing", "plaintext", "xmp"] {
        let html = format!("<html><body><{tag}>x</{tag}></body></html>");
        let opts = empty_options();
        let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
        let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            cascade.computed[id].font_family[0].to_string(),
            "monospace",
            "{tag}'s UA rule font-family: monospace must reach computed.font_family through real parse+cascade"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            cascade.computed[id].white_space,
            WhiteSpace::Pre,
            "{tag}'s UA rule white-space: pre must reach computed.white_space through real parse+cascade"
        );
    }

    // Contrast: an element the UA rule does not target must stay at
    // CSS-initial `serif` / `normal` (HTML LS §flow-content-3's selector
    // is exactly the 4 elements above, not every element).
    let html = "<html><body><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p")
        .expect("<p> should exist")
        .0 as usize;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].font_family[0].to_string(),
        "serif",
        "p must stay at CSS-initial font-family: serif, not the pre/listing/plaintext/xmp UA rule's monospace"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        cascade.computed[p_id].white_space,
        WhiteSpace::Normal,
        "p must stay at CSS-initial white-space: normal, not the pre/listing/plaintext/xmp UA rule's pre"
    );
}

#[test]
fn margin_block_and_inline_ua_rules_survive_real_parse_and_cascade() {
    // HTML LS §flow-content-3's
    //   blockquote, figure, listing, p, plaintext, pre, xmp {
    //     margin-block: 1em;
    //   }
    //   blockquote, figure { margin-inline: 40px; }
    // land in MINIMAL_UA_CSS verbatim, using the CSS Logical Properties
    // and Values 1 §4.2 shorthands directly. This test pins that both
    // rules reach `computed.margin.*` through real cssparser parsing
    // and cascade, not just literal text in `MINIMAL_UA_CSS` — same
    // "survives real parse+cascade" concern as the `hr` test above.
    // 1em resolves to 16px against the UA-default inherited 16px
    // font-size; margin-inline's 40px is a plain absolute length with
    // no such resolution step.
    //
    // margin-left/margin-right are asserted per tag: 40px for
    // blockquote/figure (the only two the margin-inline rule targets,
    // resolved from the LTR-default `direction: ltr` inline-start/end
    // pair to left/right respectively) and CSS-initial 0px for the
    // other five, which margin-inline does not touch at all.
    //
    // Each tag is parsed in its own isolated document, same reason as
    // `listing_plaintext_pre_xmp_font_family_and_white_space_survives_real_parse_and_cascade`
    // above: html5ever's tokenizer switches to the PLAINTEXT state on a
    // `<plaintext>` start tag, after which no further element in the
    // same document would parse.
    //
    // Every fixture below carries an explicit `<!DOCTYPE html>` to force
    // standards mode. Without it these fixtures would parse in quirks
    // mode (`parse_captures_quirks_mode_for_missing_doctype` pins that
    // a missing doctype does exactly this), and six of the seven tags
    // (all but figure) are on HTML LS §15.3.9 "Margin collapsing
    // quirks"'s "elements with default margins" list: as each tag's
    // only, substantial (non-blank) child of body, §15.3.9 would
    // require `margin-block-start` (this crate's `margin-top`) to be
    // UA-zeroed in quirks mode — a separate, currently-unimplemented
    // rule (see minimal.css's comment on this UA rule). Forcing
    // standards mode here keeps this test pinned to margin-block
    // substitution alone, not an unrelated quirks-mode interaction.
    use raikiri_style::{ComputedLengthPercentageOrAuto, Origin};

    for tag in [
        "blockquote",
        "figure",
        "listing",
        "p",
        "plaintext",
        "pre",
        "xmp",
    ] {
        let html = format!("<!DOCTYPE html><html><body><{tag}>x</{tag}></body></html>");
        let opts = empty_options();
        let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
        let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        let margin = cascade.computed[id].margin;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "{tag}'s UA rule margin-block: 1em must reach computed.margin.top (16px at \
                 default 16px font-size) through real parse+cascade"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "{tag}'s UA rule margin-block: 1em must reach computed.margin.bottom (16px \
                 at default 16px font-size) through real parse+cascade"
        );
        // blockquote/figure additionally carry `margin-inline: 40px`;
        // the other five tags stay at CSS-initial 0 on the inline axis.
        let expect_inline = if matches!(tag, "blockquote" | "figure") {
            ComputedLengthPercentageOrAuto::Px(40.0)
        } else {
            ComputedLengthPercentageOrAuto::Px(0.0)
        };
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            margin.left, expect_inline,
            "{tag}'s computed.margin.left must reflect the margin-inline UA rule \
                 (40px for blockquote/figure, CSS-initial 0 otherwise)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            margin.right, expect_inline,
            "{tag}'s computed.margin.right must reflect the margin-inline UA rule \
                 (40px for blockquote/figure, CSS-initial 0 otherwise)"
        );
    }

    // Contrast: an element the UA rule does not target must stay at
    // CSS-initial margin 0 on every side (HTML LS §flow-content-3's
    // margin-block selector is exactly the 7 elements above, not every
    // block-level element — e.g. div gets display: block from a
    // separate rule but no margin rule at all).
    let html = "<!DOCTYPE html><html><body><div>x</div></body></html>";
    let opts = empty_options();
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
    let div_id = find_first_by_tag(&uncascaded.dom, "div")
        .expect("<div> should exist")
        .0 as usize;
    let div_margin = cascade.computed[div_id].margin;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        (
            div_margin.top,
            div_margin.right,
            div_margin.bottom,
            div_margin.left
        ),
        (
            ComputedLengthPercentageOrAuto::Px(0.0),
            ComputedLengthPercentageOrAuto::Px(0.0),
            ComputedLengthPercentageOrAuto::Px(0.0),
            ComputedLengthPercentageOrAuto::Px(0.0),
        ),
        "div must stay at CSS-initial margin 0 on every side, not the \
             blockquote/figure/listing/p/plaintext/pre/xmp UA rule's 1em top/bottom"
    );
}

#[test]
fn parse_extracts_body_style_after_head_style() {
    let html = b"<html><head><style>p{color:red}</style></head>\
                     <body><style>p{color:blue}</style><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![String::from("p{color:red}"), String::from("p{color:blue}")]
    );
}

#[test]
fn parse_extracts_body_style_without_explicit_head() {
    let html = b"<html><body><style>p{color:green}</style><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![String::from("p{color:green}")]
    );
}

#[test]
fn parse_skips_body_style_inside_template_element() {
    let html = b"<html><body><template><style>p{color:red}</style></template>                     <style>p{color:blue}</style><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![String::from("p{color:blue}")]
    );
}

#[test]
fn parse_includes_svg_and_html_styles_in_host_stylesheet_sources() {
    let html = b"<html><body><svg><style>circle{fill:red}</style></svg>                     <math><style>p{color:orange}</style></math>                     <style>p{color:green}</style><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    assert_eq!(
        stylesheet_texts(&uncascaded),
        vec![
            String::from("circle{fill:red}"),
            String::from("p{color:green}"),
        ]
    );
}

// ── Attribute / namespace integration ────────

#[test]
fn parse_wires_style_attribute_to_inline_style_source() {
    // Real HTML `<p style="color:red">` populates inline_style_source
    // for cascade to consume (end-to-end verification).
    let html = b"<html><body><p style=\"color:red\">Hi</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let p_node = uncascaded.dom.node(p_id).expect("p node exists");
    let p_elem = p_node.as_element().expect("p is element");
    assert_eq!(p_elem.inline_style_source(), Some("color:red"));
    // `style` is stored separately in Node.inline_style, not in attributes.
    assert_eq!(p_elem.attr("style"), Some("color:red"));
}

#[test]
fn parse_wires_id_class_and_data_attributes() {
    let html =
        br#"<html><body><div id="main" class="foo bar baz" data-x="42"></div></body></html>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let div_id = find_first_by_tag(&uncascaded.dom, "div").expect("div exists");
    let div_node = uncascaded.dom.node(div_id).expect("div node exists");
    let div = div_node.as_element().expect("div is element");
    assert_eq!(div.id(), Some("main"));
    assert!(div.has_class("foo"));
    assert!(div.has_class("bar"));
    assert!(div.has_class("baz"));
    assert!(!div.has_class("qux"));
    assert_eq!(div.attr("data-x"), Some("42"));
}

#[test]
fn parse_treats_html_elements_namespace_uri_as_none() {
    let html = b"<html><body><p>hi</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let p_node = uncascaded.dom.node(p_id).expect("p node exists");
    let p = p_node.as_element().expect("p is element");
    // The optimized path returns None for the default HTML namespace.
    assert_eq!(p.namespace_uri(), None);
}

#[test]
fn parse_wires_svg_namespace_uri() {
    // html5ever automatically puts elements within <svg> in the SVG namespace.
    let html = br#"<html><body><svg><g></g></svg></body></html>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let svg_id = find_first_by_tag(&uncascaded.dom, "svg").expect("svg exists");
    let g_id = find_first_by_tag(&uncascaded.dom, "g").expect("g exists");
    let svg_node = uncascaded.dom.node(svg_id).expect("svg node exists");
    let svg = svg_node.as_element().expect("svg is element");
    assert_eq!(svg.namespace_uri(), Some("http://www.w3.org/2000/svg"));
    let g_node = uncascaded.dom.node(g_id).expect("g node exists");
    let g = g_node.as_element().expect("g is element");
    assert_eq!(g.namespace_uri(), Some("http://www.w3.org/2000/svg"));
}

#[test]
fn parse_serializes_inline_svg_with_qualified_attributes() {
    let html = br##"<!doctype html><html><body><svg viewBox="0 0 4 2"><defs><g id="shape"><rect width="4" height="2" fill="red"></rect></g></defs><use xlink:href="#shape"></use></svg></body></html>"##;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let svg_id = find_first_by_tag(&uncascaded.dom, "svg").expect("svg exists");
    let source = uncascaded
        .dom
        .serialize_svg_subtree(svg_id.0 as usize)
        .expect("SVG subtree serializes")
        .expect("SVG root produces XML source");

    assert!(source.contains("viewBox=\"0 0 4 2\""));
    assert!(source.contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""));
    assert!(source.contains("xlink:href=\"#shape\""));
    assert!(source.contains("width=\"4\""));
    assert!(source.contains("height=\"2\""));
    assert!(source.contains("fill=\"red\""));
}

#[test]
fn parse_missing_style_attribute_leaves_inline_style_none() {
    let html = b"<html><body><p>x</p></body></html>";
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let p_node = uncascaded.dom.node(p_id).expect("p node exists");
    let p = p_node.as_element().expect("p is element");
    assert_eq!(p.inline_style_source(), None);
}

#[test]
fn parse_empty_style_attribute_normalizes_to_none() {
    // Trait contract: `style=""` gives inline_style_source == None.
    let html = br#"<html><body><p style=""></p></body></html>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let p_node = uncascaded.dom.node(p_id).expect("p node exists");
    let p = p_node.as_element().expect("p is element");
    assert_eq!(p.inline_style_source(), None);
}

#[test]
fn sink_first_wins_on_duplicate_style_attribute() {
    // Defense in depth: html5ever deduplicates attributes during tokenization
    // (§13.2.5.32), but the raikiri-html sink may still receive a
    // Vec<Attribute> with duplicates. An external consumer might wrap TreeSink
    // and inject duplicate attributes. Drive the sink directly to check that
    // when style is supplied twice, the first value (color:red) wins.
    use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
    use html5ever::tendril::StrTendril;
    use markup5ever::{LocalName, Namespace};

    let sink = RaikiriTreeSink::default();
    let name = QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from("p"),
    );
    let attr = |v: &str| Attribute {
        name: QualName::new(None, Namespace::from(""), LocalName::from("style")),
        value: StrTendril::from(v),
    };
    // Pass "style=color:red" then "style=color:blue" to the sink.
    let attrs = vec![attr("color:red"), attr("color:blue")];
    let idx = sink.create_element(name, attrs, ElementFlags::default());
    // The Document Handle is the root. Append the node as a root child
    // so finish can find it.
    sink.append(
        &sink.get_document(),
        html5ever::interface::NodeOrText::AppendNode(idx),
    );
    let uncascaded = sink.finish();

    let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
    let p_node = uncascaded.dom.node(p_id).expect("p node exists");
    let p = p_node.as_element().expect("p is element");
    // First wins (regression check for wire_side_tables).
    assert_eq!(p.inline_style_source(), Some("color:red"));
}

// ── MathML annotation-xml integration point ─────
//
// HTML5 §13.2.5 tree construction: a MathML `annotation-xml` element
// is an HTML integration point when its `encoding` attribute equals
// `text/html` or `application/xhtml+xml`, ignoring ASCII case. This is
// the central predicate for branch selection in tree construction.

/// Helper: create an annotation-xml element with the given namespace
/// and encoding, then return `is_mathml_annotation_xml_integration_point`.
fn probe_annotation_xml_integration_point(
    ns_uri: &str,
    local: &str,
    encoding: Option<&str>,
) -> bool {
    use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
    use html5ever::tendril::StrTendril;
    use markup5ever::{LocalName, Namespace};

    let sink = RaikiriTreeSink::default();
    let name = QualName::new(None, Namespace::from(ns_uri), LocalName::from(local));
    let attrs = encoding
        .map(|v| {
            vec![Attribute {
                name: QualName::new(None, Namespace::from(""), LocalName::from("encoding")),
                value: StrTendril::from(v),
            }]
        })
        .unwrap_or_default();
    let idx = sink.create_element(name, attrs, ElementFlags::default());
    sink.is_mathml_annotation_xml_integration_point(&idx)
}

#[test]
fn annotation_xml_with_text_html_encoding_is_integration_point() {
    assert!(probe_annotation_xml_integration_point(
        "http://www.w3.org/1998/Math/MathML",
        "annotation-xml",
        Some("text/html"),
    ));
}

#[test]
fn annotation_xml_encoding_comparison_is_ascii_case_insensitive() {
    // spec: "ASCII case-insensitive match"
    for v in [
        "Text/HTML",
        "TEXT/HTML",
        "text/HTML",
        "Application/XHTML+XML",
    ] {
        assert!(
            probe_annotation_xml_integration_point(
                "http://www.w3.org/1998/Math/MathML",
                "annotation-xml",
                Some(v),
            ),
            "encoding={v:?} should match (ASCII case-insensitive)"
        );
    }
}

#[test]
fn annotation_xml_with_application_xhtml_xml_encoding_is_integration_point() {
    assert!(probe_annotation_xml_integration_point(
        "http://www.w3.org/1998/Math/MathML",
        "annotation-xml",
        Some("application/xhtml+xml"),
    ));
}

#[test]
fn annotation_xml_with_unrelated_encoding_is_not_integration_point() {
    // The spec lists only text/html and application/xhtml+xml as integration
    // points. application/xml, image/svg+xml, and the empty string do not match.
    for v in ["application/xml", "image/svg+xml", "", "text/plain"] {
        assert!(
            !probe_annotation_xml_integration_point(
                "http://www.w3.org/1998/Math/MathML",
                "annotation-xml",
                Some(v),
            ),
            "encoding={v:?} should not match"
        );
    }
}

#[test]
fn annotation_xml_without_encoding_attribute_is_not_integration_point() {
    assert!(!probe_annotation_xml_integration_point(
        "http://www.w3.org/1998/Math/MathML",
        "annotation-xml",
        None,
    ));
}

#[test]
fn non_mathml_annotation_xml_is_not_integration_point() {
    // Defense in depth: annotation-xml is not an integration point outside
    // the MathML namespace (the spec says "MathML annotation-xml").
    assert!(!probe_annotation_xml_integration_point(
        "http://www.w3.org/2000/svg",
        "annotation-xml",
        Some("text/html"),
    ));
    assert!(!probe_annotation_xml_integration_point(
        "http://www.w3.org/1999/xhtml",
        "annotation-xml",
        Some("text/html"),
    ));
}

#[test]
fn non_annotation_xml_mathml_element_is_not_integration_point() {
    // Other MathML elements are not integration points.
    assert!(!probe_annotation_xml_integration_point(
        "http://www.w3.org/1998/Math/MathML",
        "mi",
        Some("text/html"),
    ));
}

#[test]
fn annotation_xml_duplicate_encoding_attribute_uses_first_value() {
    // HTML §13.2.5.32: ignore later occurrences of a duplicate attribute.
    // Apply the same first-wins contract as
    // sink_first_wins_on_duplicate_style_attribute to the integration-point
    // predicate: a later match must not override an earlier nonmatch.
    use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
    use html5ever::tendril::StrTendril;
    use markup5ever::{LocalName, Namespace};

    let encoding_attr = |v: &str| Attribute {
        name: QualName::new(None, Namespace::from(""), LocalName::from("encoding")),
        value: StrTendril::from(v),
    };

    // Case A: first=text/html (match), second=application/xml (non-match)
    // → first wins → integration point (true)
    {
        let sink = RaikiriTreeSink::default();
        let name = QualName::new(
            None,
            Namespace::from("http://www.w3.org/1998/Math/MathML"),
            LocalName::from("annotation-xml"),
        );
        let idx = sink.create_element(
            name,
            vec![encoding_attr("text/html"), encoding_attr("application/xml")],
            ElementFlags::default(),
        );
        assert!(
            sink.is_mathml_annotation_xml_integration_point(&idx),
            "first encoding=text/html should win over later encoding=application/xml"
        );
    }

    // Case B: first=application/xml (non-match), second=text/html (match)
    // → first wins → NOT integration point (false)
    {
        let sink = RaikiriTreeSink::default();
        let name = QualName::new(
            None,
            Namespace::from("http://www.w3.org/1998/Math/MathML"),
            LocalName::from("annotation-xml"),
        );
        let idx = sink.create_element(
            name,
            vec![encoding_attr("application/xml"), encoding_attr("text/html")],
            ElementFlags::default(),
        );
        assert!(
            !sink.is_mathml_annotation_xml_integration_point(&idx),
            "later encoding=text/html must not override earlier non-match"
        );
    }
}

// End-to-end parsing: the annotation-xml integration-point predicate
// changes the branch in the tree construction algorithm. Observe it by
// checking the child element’s namespace:
// - encoding=text/html activates an HTML integration point: HTML namespace
//   (the optimized raikiri-dom namespace_uri() returns None).
// - Without encoding: normal MathML foreign content and MathML namespace.

#[test]
fn parse_annotation_xml_integration_point_inherits_html_namespace_for_children() {
    // annotation-xml with encoding=text/html is an HTML integration point.
    // Its unknown <foo> child must be interpreted in the HTML namespace.
    let html =
        br#"<math><annotation-xml encoding="text/html"><foo>x</foo></annotation-xml></math>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let foo_id = find_first_by_tag(&uncascaded.dom, "foo").expect("foo exists");
    let foo_node = uncascaded.dom.node(foo_id).expect("foo node exists");
    let foo = foo_node.as_element().expect("foo is element");
    // cov:ignore: assertion text is only evaluated when this test fails
    assert_eq!(
        foo.namespace_uri(),
        None,
        "child inside HTML integration point should be HTML (None optimized path)"
    );
}

#[test]
fn parse_annotation_xml_non_integration_wraps_children_in_mathml_namespace() {
    // annotation-xml without encoding is not an integration point. Its
    // unknown <foo> child belongs to MathML foreign content and namespace.
    let html = br#"<math><annotation-xml><foo>x</foo></annotation-xml></math>"#;
    let opts = empty_options();
    let uncascaded = parse(&html[..], &opts).expect("parse ok");
    let foo_id = find_first_by_tag(&uncascaded.dom, "foo").expect("foo exists");
    let foo_node = uncascaded.dom.node(foo_id).expect("foo node exists");
    let foo = foo_node.as_element().expect("foo is element");
    assert_eq!(
        foo.namespace_uri(),
        Some("http://www.w3.org/1998/Math/MathML"),
        "child inside non-integration MathML should stay in MathML namespace"
    );
}

#[test]
fn parse_persists_bulk_comments_and_filters_them_from_taffy_child_count() {
    // Contract rewrite: the old test checked whether 100 "#comment"
    // pseudo-tag Elements were stripped by scanning for that tag’s absence.
    // Now 100 Comment-kind nodes persist in the arena, but all 100 are
    // filtered from taffy child_count, leaving only <p> as a taffy child of
    // body. Assert this bulk invariant positively. The old Element scan
    // would pass vacuously because Comments return as_element() == None.
    use raikiri_traits::{Dom, Node};

    // 100 comments under body — verifies mark_in_document_flags handles bulk
    // Stress-test the replacement for the old retain_children-based bulk strip.
    let mut html = String::from("<html><head></head><body>");
    for i in 0..100 {
        html.push_str(&format!("<!-- comment {i} -->"));
    }
    html.push_str("<p>x</p></body></html>");
    let opts = empty_options();
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
    let doc = &uncascaded.dom;

    // (a) Exactly 100 Comment nodes persist in the arena.
    let mut comment_count = 0usize;
    let mut in_document_comments = 0usize;
    for i in 0..doc.node_count() {
        let n = doc.node(raikiri_traits::NodeId::new(i as u64)).unwrap();
        if n.kind() == NodeKind::Comment {
            comment_count += 1;
            if n.is_in_document() {
                in_document_comments += 1;
            }
        }
    }
    assert_eq!(
        comment_count, 100,
        "expected 100 Comment nodes to persist in the arena"
    );
    // (b) The IS_IN_DOCUMENT bit is clear on every Comment
    //     (the bulk mark_in_document_flags contract).
    assert_eq!(
        in_document_comments, 0,
        "all 100 Comment nodes must have IS_IN_DOCUMENT cleared"
    );

    // (c) The body has exactly one taffy child, <p>; the
    //     TaffyChildIter is_in_document filter removes all 100 Comments.
    //     No leak occurs even for attacker-controlled bulk input (defense in depth).
    let body_id = find_first_by_tag(doc, "body").expect("body exists");
    use taffy::TraversePartialTree;
    let body_taffy_id = taffy::NodeId::from(body_id.0 as usize);
    assert_eq!(
        <raikiri_dom::Document as TraversePartialTree>::child_count(doc, body_taffy_id),
        1,
        "taffy body child_count must be 1 (only <p>), 100 comments filtered"
    );
}

// ── UA CSS bundle ──────────────

#[test]
fn minimal_ua_css_covers_required_display_block_selectors() {
    // Scope: html, body, div, p, and h1-h6 have display: block.
    //
    // For each tag, check that a rule line starts with `{tag}` after trim_start,
    // followed by whitespace and `{`. A plain contains() check would falsely
    // match `p` inside `Appendix`, `paragraph`, or `display` in comments.
    //
    // article/section/nav/aside/header/footer/
    // Add main/figure/figcaption/blockquote (block-level sectioning and
    // grouping elements), alongside article/section/nav/aside/header/footer.
    // For a non-vacuous cascade test, see `crates/raikiri/tests/build_cascaded.rs`
    // and `sectioning_and_grouping_elements_are_display_block_via_ua_css`.
    // Add ol/ul/li (block-level list treatment and the foundation for
    // `display: list-item` and markers). For a non-vacuous cascade test, see
    // `crates/raikiri/tests/build_cascaded.rs` and
    // `list_elements_use_list_item_display_via_ua_css`.
    // Add hr. Its display: block comes from the §flow-content-3 (15.3.3)
    // flow-content rule; separate rules handle hr’s border/color/margin (see
    // minimal.css comments). For a non-vacuous cascade test, see
    // `crates/raikiri/tests/build_cascaded.rs` and
    // `hr_is_display_block_border_inset_and_margin_via_ua_css`.
    // Add hgroup: it shares the §sections-and-headings (15.3.6) selector
    // group with article/aside/nav/section but was omitted from the original
    // scope. For a non-vacuous cascade test, see the existing loop in
    // `crates/raikiri/tests/build_cascaded.rs`,
    // `sectioning_and_grouping_elements_are_display_block_via_ua_css`.
    // Add address/center/listing/plaintext/search/xmp: six of the seven
    // previously untracked display:block selectors in §flow-content-3
    // (15.3.3). HTML LS §16.2 classifies center/listing/plaintext/xmp as
    // "entirely obsolete", but that is about authoring conformance, not UA
    // rendering (see minimal.css comments). For a non-vacuous cascade test of
    // these six, see `crates/raikiri/tests/build_cascaded.rs` and
    // `flow_content_3_residue_elements_are_display_block_via_ua_css`.
    // The seventh element, dialog, is excluded from this loop (see below).
    for tag in [
        "html",
        "body",
        "div",
        "p",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "article",
        "section",
        "nav",
        "aside",
        "hgroup",
        "header",
        "footer",
        "main",
        "figure",
        "figcaption",
        "blockquote",
        "ol",
        "ul",
        "li",
        "hr",
        "address",
        "center",
        "listing",
        "plaintext",
        "pre",
        "search",
        "xmp",
    ] {
        let has_rule = MINIMAL_UA_CSS.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix(tag)
                .map(|rest| rest.trim_start().starts_with('{'))
                .unwrap_or(false)
        });
        assert!(
            has_rule,
            "MINIMAL_UA_CSS is missing selector rule `{tag} {{ … }}`",
        );
    }
    // The display of dialog depends on its open attribute, via the `dialog`
    // and `dialog[open]` specificity pair (see minimal.css comments). It does
    // not meet this loop’s unconditional display: block contract. Check both
    // selector-rule prefixes directly here instead. For a non-vacuous cascade
    // test, see `crates/raikiri/tests/build_cascaded.rs` and
    // `dialog_display_reflects_open_attribute_via_ua_css`.
    let has_selector_rule = |selector: &str| {
        MINIMAL_UA_CSS.lines().any(|line| {
            line.trim_start()
                .strip_prefix(selector)
                .map(|rest| rest.trim_start().starts_with('{'))
                .unwrap_or(false)
        })
    };
    assert!(
        has_selector_rule("dialog"),
        "MINIMAL_UA_CSS is missing selector rule `dialog {{ … }}`",
    );
    assert!(
        has_selector_rule("dialog[open]"),
        "MINIMAL_UA_CSS is missing selector rule `dialog[open] {{ … }}`",
    );
    // `dir` directionality mapping (HTML LS Rendering §15.3.5, see
    // minimal.css comment): both `[dir]:dir(...)` rules must be present.
    // Non-vacuous cascade behavior is pinned end-to-end in
    // `crates/raikiri/tests/build_cascaded.rs`
    // (`div_direction_reflects_dir_attribute_via_ua_css`).
    assert!(
        has_selector_rule("[dir]:dir(ltr)"),
        "MINIMAL_UA_CSS is missing selector rule `[dir]:dir(ltr) {{ … }}`",
    );
    assert!(
        has_selector_rule("[dir]:dir(rtl)"),
        "MINIMAL_UA_CSS is missing selector rule `[dir]:dir(rtl) {{ … }}`",
    );
    // Check for the spec-reference comment, a sign of independent implementation based on CSS 2.1 App.D.
    assert!(
        MINIMAL_UA_CSS.contains("CSS 2.1 App.D"),
        "MINIMAL_UA_CSS should contain spec reference comments",
    );
    // Check for a display: block declaration directly as text.
    assert!(
        MINIMAL_UA_CSS.contains("display: block"),
        "MINIMAL_UA_CSS should declare display: block",
    );
}

#[test]
fn parse_injects_default_ua_stylesheet_into_document() {
    use raikiri_traits::StylesheetKind;

    let html = b"<html><body><p>Hi</p></body></html>";
    let opts = empty_options();
    let doc = parse(&html[..], &opts).expect("parse ok");

    let ua_entries: Vec<&str> = doc
        .dom
        .stylesheets()
        .filter(|(_, k)| *k == StylesheetKind::UserAgent)
        .map(|(s, _)| s)
        .collect();
    assert_eq!(ua_entries.len(), 1, "exactly one UA CSS entry expected");
    assert_eq!(ua_entries[0], MINIMAL_UA_CSS);
}

#[test]
fn parse_retains_extra_stylesheets_as_user() {
    use raikiri_traits::StylesheetKind;

    let html = b"<html><body></body></html>";
    let extra_a = "a { color: red }";
    let extra_b = "b { color: blue }";
    let opts = ParseOptions {
        extra_stylesheets: &[extra_a, extra_b],
        network: None,
        base_url: None,
    };
    let doc = parse(&html[..], &opts).expect("parse ok");

    // extra_stylesheets are tagged StylesheetKind::User. Previously they
    // were mixed into Author; the separate variant prevents that regression.
    let user_entries: Vec<&str> = doc
        .user_stylesheet_sources
        .iter()
        .flat_map(|sheet| sheet.parts.iter().map(|part| part.source.as_str()))
        .collect();
    assert_eq!(user_entries.len(), 2);
    assert_eq!(user_entries[0], extra_a);
    assert_eq!(user_entries[1], extra_b);

    // Also confirm that no entries reach Author (two did before retagging).
    let author_entries: Vec<&str> = doc
        .dom
        .stylesheets()
        .filter(|(_, k)| *k == StylesheetKind::Author)
        .map(|(s, _)| s)
        .collect();
    // cov:ignore: same false positive as the `img_width_attribute_*` tests in
    // raikiri-style/src/cascade.rs (message-format branch of assert! only
    // executes on failure).
    assert!(
        author_entries.is_empty(),
        "extra_stylesheets should no longer tag as Author"
    );
}

#[test]
fn img_defaults_to_inline_block_via_ua_css() {
    use raikiri_style::Origin;

    let mut doc = raikiri_dom::Document::new();
    let html = doc.append_element(Some(0), "html", taffy::Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", taffy::Style::default(), None::<&str>);
    let img = doc.append_element(Some(body), "img", taffy::Style::default(), None::<&str>);

    let mut tree = raikiri_style::build_rule_tree(&doc);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&doc, &tree).expect("cascade ok");

    assert_eq!(
        cascade.computed[img].display,
        raikiri_style::property::DisplayValue::InlineBlock
    );
}

#[test]
fn table_ua_rule_sets_a_2px_border_spacing() {
    // HTML LS §15.3.8: `table { border-spacing: 2px }`. The CSS initial
    // value is 0, so the spacing comes only from the UA rule.
    use raikiri_style::Origin;
    use raikiri_style::resolve::ComputedLength;

    let spacing = |html: &str, tag: &str| {
        let uncascaded = parse(html.as_bytes(), &empty_options()).expect("parse ok");
        let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        let bs = &cascade.computed[id].border_spacing;
        (bs.horizontal, bs.vertical)
    };
    let table =
        "<!doctype html><html><body><table><tr><td>a</td></tr></table><div>b</div></body></html>";
    assert_eq!(
        spacing(table, "table"),
        (ComputedLength(2.0), ComputedLength(2.0))
    );
    // Elements outside a table keep the initial 0.
    assert_eq!(
        spacing(table, "div"),
        (ComputedLength(0.0), ComputedLength(0.0))
    );
    // `border-collapse` is inherited; the UA rule restates `separate` so a
    // table nested in a collapsing table keeps its own spacing.
    let nested = "<!doctype html><html><body><table style=\"border-collapse: collapse\"><tr><td>\
        <table id=inner><tr><td>a</td></tr></table></td></tr></table></body></html>";
    let uncascaded = parse(nested.as_bytes(), &empty_options()).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
    let inner = (0..cascade.computed.len())
        .find(|&id| {
            find_first_by_tag(&uncascaded.dom, "table").map(|n| n.0 as usize) != Some(id)
                && cascade.computed[id].display == raikiri_style::property::DisplayValue::Table
        })
        .expect("inner table");
    assert_eq!(
        cascade.computed[inner].border_collapse,
        raikiri_style::property::BorderCollapseValue::Separate
    );
    // Author style wins over the UA rule.
    let author = "<!doctype html><html><body><table style=\"border-spacing: 0 4px\"><tr><td>a</td></tr></table></body></html>";
    assert_eq!(
        spacing(author, "table"),
        (ComputedLength(0.0), ComputedLength(4.0))
    );
}

#[test]
fn heading_ua_rules_set_weight_size_and_block_margins_per_level() {
    // HTML LS §15.3.6 sections-and-headings, through real parse+cascade:
    // every heading is bold; level n gets its own font size and block
    // margins, the margins in em of the heading's own font size.
    use raikiri_style::Origin;
    use raikiri_style::resolve::ComputedLengthPercentageOrAuto;

    // Standards mode: the quirks-mode margin-collapsing quirk would zero
    // the first heading's top margin (HTML LS §15.3.9).
    let html =
        b"<!doctype html><html><body><h1>a</h1><h2>b</h2><h3>c</h3><h4>d</h4><h5>e</h5><h6>f</h6></body></html>";
    let uncascaded = parse(&html[..], &empty_options()).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");

    // (tag, font-size factor, margin-block in em)
    let levels = [
        ("h1", 2.00, 0.67),
        ("h2", 1.50, 0.83),
        ("h3", 1.17, 1.00),
        ("h4", 1.00, 1.33),
        ("h5", 0.83, 1.67),
        ("h6", 0.67, 2.33),
    ];
    for (tag, size, margin) in levels {
        let id = find_first_by_tag(&uncascaded.dom, tag)
            .unwrap_or_else(|| panic!("<{tag}> should exist"))
            .0 as usize;
        let cv = &cascade.computed[id];
        assert_eq!(cv.font_weight, 700.0, "{tag} weight");
        let font_size = cv.font_size.px();
        assert!(
            (font_size - 16.0 * size).abs() < 1e-3,
            "{tag} size {font_size}"
        );
        for (side, value) in [("top", cv.margin.top), ("bottom", cv.margin.bottom)] {
            let ComputedLengthPercentageOrAuto::Px(px) = value else {
                panic!("{tag} margin-{side} is not a length: {value:?}");
            };
            assert!(
                (px - font_size * margin).abs() < 1e-3,
                "{tag} margin-{side} {px}"
            );
        }
    }

    // Author style wins over the UA rules.
    let html = b"<!doctype html><html><body><h1 style=\"font-weight: normal; margin: 0\">g</h1></body></html>";
    let uncascaded = parse(&html[..], &empty_options()).expect("parse ok");
    let mut tree = raikiri_style::build_rule_tree(&uncascaded.dom);
    tree.add_stylesheet(MINIMAL_UA_CSS, Origin::UserAgent);
    let cascade = raikiri_style::cascade(&uncascaded.dom, &tree).expect("cascade ok");
    let id = find_first_by_tag(&uncascaded.dom, "h1")
        .expect("<h1> should exist")
        .0 as usize;
    let cv = &cascade.computed[id];
    assert_eq!(cv.font_weight, 400.0);
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
    assert!(
        (cv.font_size.px() - 32.0).abs() < 1e-3,
        "the size still comes from the UA rule"
    );
}
