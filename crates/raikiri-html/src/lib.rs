//! raikiri-html — html5ever wrapper (`RaikiriTreeSink`) + [`parse`] /
//! [`parse_with_sink`] entrypoints。cascade 前の [`UncascadedDocument`] を produce
//! する薄い parser layer (責務は parse だけ、cascade / layout は含まない)。

mod parse;
mod sink;
mod types;
pub mod ua;

pub use parse::{parse, parse_with_sink};
pub use sink::RaikiriTreeSink;
pub use types::{ParseOptions, UncascadedDocument};
pub use ua::MINIMAL_UA_CSS;

#[cfg(test)]
#[allow(clippy::needless_lifetimes, clippy::collapsible_if)]
mod tests {
    use super::*;
    use raikiri_traits::{Dom, Element, Node, NodeKind};

    fn empty_options<'a>() -> ParseOptions<'a> {
        ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        }
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

        // html > body > p > "Hello" の tree を assert。
        // 注意: html5ever tokenizer は text を複数 AppendText に分割する可能性
        // があるため、Text children を concat して assert する (M1 では
        // adjacent Text の auto-merge を実装しない spike 上限)。
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
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:red}")]
        );
    }

    #[test]
    fn parse_skips_link_stylesheet_when_no_network_provider() {
        // raikiri-spike-5z86.6: 外部 <link rel="stylesheet"> の fetch は
        // `ParseOptions::network` が `Some` の時のみ行われる opt-in 機能。
        // `network: None` (empty_options()) の場合は href の解決すら試みず、
        // stylesheet_sources は空のまま — Consumer が network capability を
        // 渡さない既存 caller の挙動は不変。
        let html =
            br#"<html><head><link rel="stylesheet" href="foo.css"></head><body>x</body></html>"#;
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
        // element. The assertion below targets only what raikiri-spike-5z86.6
        // could plausibly add: no fetch-related warning without a provider.
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

    /// raikiri-spike-5z86.6: `<link rel="stylesheet">` fetch → CSS text →
    /// `UncascadedDocument.stylesheet_sources` の wiring を、mock
    /// `NetworkProvider` を使って end-to-end で検証する (実 CSS parsing /
    /// cascade 統合は raikiri-style / raikiri umbrella crate 側、ここでは
    /// raikiri-html の責務である「検出 → fetch → doc.stylesheet_sources
    /// への統合」までを見る)。
    mod external_link_stylesheet_fetch_tests {
        use super::*;
        use raikiri_traits::{
            FetchedResource, NetworkError, NetworkProvider, PolicyViolation, Request, ResourceKind,
            ViolationType, WarningKind,
        };

        /// `NetworkProvider` that echoes the resolved request URL back as
        /// the CSS body comment (`/* <url> */`). Doubles as both a
        /// content-producer (fetch succeeded) and an assertion probe (which
        /// URL raikiri-html actually resolved and requested) without extra
        /// interior-mutability bookkeeping.
        struct EchoUrlProvider;

        impl NetworkProvider for EchoUrlProvider {
            fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
                Ok(FetchedResource {
                    bytes: bytes::Bytes::from(format!("/* {} */", request.url).into_bytes()),
                    content_type: Some("text/css".to_string()),
                    final_url: request.url,
                    encoding: None,
                })
            }
        }

        /// `NetworkProvider` that returns a successful, empty-body response
        /// (verifies the `extract_inline_stylesheets`-style empty-body
        /// guard: an empty fetched stylesheet must not be pushed).
        struct EmptyBodyProvider;

        impl NetworkProvider for EmptyBodyProvider {
            fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
                Ok(FetchedResource {
                    bytes: bytes::Bytes::new(),
                    content_type: Some("text/css".to_string()),
                    final_url: request.url,
                    encoding: None,
                })
            }
        }

        /// `NetworkProvider` whose `fetch` always fails with a caller-chosen
        /// error (constructed fresh per call since `NetworkError` isn't
        /// `Clone`).
        struct AlwaysErrorProvider(fn() -> NetworkError);

        impl NetworkProvider for AlwaysErrorProvider {
            fn fetch(&self, _request: Request) -> Result<FetchedResource, NetworkError> {
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
            fn fetch(&self, _request: Request) -> Result<FetchedResource, NetworkError> {
                panic!("fetch must not be called for a <link> that is not a stylesheet reference");
            }
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
                uncascaded.stylesheet_sources,
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
            // gate, raikiri-spike-84y) — so this is the one case that
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
                uncascaded.stylesheet_sources,
                vec![String::from("/* https://example.test/a.css */")],
                "the comment must not prevent the <link> after it from being fetched"
            );
        }

        #[test]
        fn parse_resolves_relative_href_against_base_url() {
            let provider = EchoUrlProvider;
            let base =
                url::Url::parse("https://example.test/dir/page.html").expect("valid base url");
            let opts = ParseOptions {
                extra_stylesheets: &[],
                network: Some(&provider as &dyn NetworkProvider),
                base_url: Some(base),
            };
            let html =
                br#"<html><head><link rel="stylesheet" href="style.css"></head><body>x</body></html>"#;
            let uncascaded = parse(&html[..], &opts).expect("parse ok");
            assert_eq!(
                uncascaded.stylesheet_sources,
                vec![String::from("/* https://example.test/dir/style.css */")]
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
                uncascaded.stylesheet_sources,
                vec![
                    String::from("/* https://example.test/a.css */"),
                    String::from("/* https://example.test/b.css */"),
                ]
            );
        }

        #[test]
        fn parse_appends_external_stylesheets_after_inline_style_sources() {
            // 既知の scope 制限 (parse.rs::fetch_external_stylesheets doc
            // 参照): 同一 <head> 内で <style> と <link> が混在する場合、
            // <link> は常に全ての <style> の後ろに追記される (真の
            // document-order interleave ではない)。
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
                uncascaded.stylesheet_sources,
                vec![
                    String::from("p{color:red}"),
                    String::from("/* https://example.test/a.css */"),
                ],
                "external stylesheet is appended after inline <style> sources (known scope limitation)"
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
            // CSS parser — reviewer-spec finding, bd raikiri-spike-5z86.6).
            let provider = PanicIfCalledProvider;
            let opts = ParseOptions {
                extra_stylesheets: &[],
                network: Some(&provider as &dyn NetworkProvider),
                base_url: Some(
                    url::Url::parse("https://example.test/page.html").expect("valid base url"),
                ),
            };
            let html =
                br#"<html><head><link rel="stylesheet" href="   "></head><body>x</body></html>"#;
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
                uncascaded.stylesheet_sources,
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
                "details should carry the underlying NetworkError message, got: {}",
                warning.details
            );
        }

        #[test]
        fn parse_records_policy_warning_on_policy_violation() {
            let provider = AlwaysErrorProvider(|| {
                NetworkError::PolicyViolation(PolicyViolation {
                    kind: ResourceKind::ExternalStylesheet,
                    url: url::Url::parse("https://blocked.test/a.css").expect("valid url"),
                    violation_type: ViolationType::HostNotAllowed,
                    details: "host not on allow-list".to_string(),
                })
            });
            let opts = ParseOptions {
                extra_stylesheets: &[],
                network: Some(&provider as &dyn NetworkProvider),
                base_url: None,
            };
            let html = br#"<html><head><link rel="stylesheet" href="https://blocked.test/a.css"></head><body>x</body></html>"#;
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

        // 明確に html5ever が非致命 parse error を報告する input:
        // </p> だけの closing tag は "unexpected end tag" を trigger する。
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
        // Codex Security finding raikiri-spike-g9vr: html5ever は malformed
        // input の 1 token あたり概ね 1 parse_error を報告するため、cap が
        // 無いと attacker-controlled 個数の RenderWarning (owned String 持ち)
        // が積み上がる (DoS)。100,000 個の `</x>` で 100,001 warnings /
        // RSS 線形増加を実測済み。
        let opts = empty_options();

        let small = b"</x>".repeat(2_000);
        let small_count = parse(small.as_slice(), &opts)
            .expect("parse recovers")
            .warnings
            .len();

        assert!(
            small_count <= crate::sink::MAX_HTML_PARSE_WARNINGS,
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

        /// Consumer wrapper example: RaikiriTreeSink を丸ごと delegate するだけの
        /// 透過 sink。sanitize / rewrite の hook point としては何もしない。
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
            fn append_doctype_to_document(
                &self,
                name: StrTendril,
                pid: StrTendril,
                sid: StrTendril,
            ) {
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
            inner: RaikiriTreeSink::new(),
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
        // raikiri-spike-84y contract rewrite (旧 84y 前: pseudo-tag "#comment"
        // Element を strip する契約 — 84y で `NodeData::Comment` variant として
        // 恒久 tree 内保持 + `mark_in_document_flags` step 2 で
        // IS_IN_DOCUMENT bit clear + Element でないので cascade/paint の Element
        // gate で skip、の 2 段 gate に置換)。
        //
        // 旧 test 名 `parse_strips_comment_nodes_from_dom_tree` は "#comment"
        // pseudo-tag Element の非存在を scan していたが、84y 後は Comment が
        // Element でない → as_element() == None → scan は自動で "見つからない"
        // → vacuously pass するため active positive assertion に rewrite する
        // (advisor 指摘: run-and-see-pass に頼らない coverage)。
        use raikiri_traits::{Dom, Node};

        let html = b"<html><body><!-- a comment --><p>hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;

        // (a) arena に必ず 1 個以上 Comment kind の node が存在する。
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

        // (b) Comment は as_element() == None (Two-way invariant)。
        assert!(
            comment_node.as_element().is_none(),
            "NodeKind::Comment must project as_element() == None (Two-way invariant)"
        );

        // (c) sink.finish() 後 mark_in_document_flags は Comment の
        //     IS_IN_DOCUMENT bit を clear している (advisor step-6 (i))。
        assert!(
            !comment_node.is_in_document(),
            "Comment's IS_IN_DOCUMENT bit must be cleared after parse"
        );

        // (d) Comment は tree 内に persist している (body の children に含まれる)。
        //     旧挙動: strip 済で detach されていたため、body の直接子は <p> のみ
        //     だった。新挙動: body の children = `[Comment, <p>]` (source order)。
        //     NB: `Dom::child_ids` と `TraversePartialTree::child_ids` の
        //     inherent-method ambiguity 回避のため UFCS で trait を明示する。
        //     raw arena children を見たいので Dom (unfiltered) を選択、次段の
        //     taffy filter test は TraversePartialTree を明示する。
        let body_id = find_first_by_tag(doc, "body").expect("body exists");
        let body_kids: Vec<usize> = Dom::child_ids(doc, body_id)
            .map(|id| id.0 as usize)
            .collect();
        assert!(
            body_kids.contains(&comment_id),
            "Comment node must persist as a child of body (not physically stripped); body kids = {body_kids:?}"
        );

        // (e) taffy layout tree からは leak しない。TaffyChildIter は
        //     is_in_document() filter (raikiri-dom/src/taffy_impl.rs) を
        //     経由するため、body の taffy child_count = 1 (`<p>` only)。
        //     この behaviour は既に `taffy_child_ids_and_count_filter_out_template_descendants`
        //     で pin されているが、Comment/PI 経路の独立 regression として
        //     ここでも assert する。
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

    #[test]
    fn parse_survives_table_foster_parenting() {
        // <table> 直下 text の foster parenting は html5ever が
        // append_before_sibling(AppendText(...)) を trigger する典型 case。
        // panic せず parse が完走することのみ verify。
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
            uncascaded.stylesheet_sources,
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
        // appear in stylesheet_sources. Minimum m1.3 fix (skip template subtree
        // during extraction). Full template-fragment isolation tracked as bd-xno.
        let html = b"<html><head>\
                     <template><style>p{color:red}</style></template>\
                     <style>p{color:blue}</style>\
                     </head><body><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // Only the top-level <style> should appear.
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:blue}")]
        );
    }

    #[test]
    fn parse_marks_template_descendants_out_of_document() {
        // raikiri-spike-37c: <template> element 自身は flat tree の一員なので
        // is_in_document()=true、その descendants (子孫の element / text) は
        // false であることを parse 経路の bit populate で pin する。
        //
        // 現在の sink には mark_in_document_flags phase が無いため、default
        // true が clear されず descendant も true になる → 失敗する failing test。
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
        // raikiri-spike-37c: normal HTML (template 無し) を parse すると全 node が
        // is_in_document()=true。default true が保たれる regression pin。
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
        // raikiri-spike-xno Part 2: sink が `<template>` を作った時 fragment
        // root を eager allocate し、template element の `template_contents`
        // slot に arena index を wire する。html5ever は以降
        // `get_template_contents(template_handle)` の戻り値を append parent と
        // して使うため、template contents は fragment root の子として積まれ
        // (template element 自身の children は空)、Document root からは
        // reachable でなくなる。この smoke test は次を pin する:
        //
        // 1. template_contents は Some(idx) を返し、idx != template arena index
        //    (別 arena slot に fragment root が実在する)
        // 2. template element の arena children は空 (children は fragment root
        //    へ流れた: 旧 M1 挙動では template 直下に <span> が居た)
        // 3. fragment root の arena children に <span> が含まれる (reshape 到達点)
        // 4. 37c invariant: template 自身は is_in_document()=true、fragment root
        //    と <span>、その text は is_in_document()=false (Document root から
        //    reachable でないため mark_in_document_flags で clear される)
        let html = b"<html><body><template><span>x</span></template></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let doc = &uncascaded.dom;

        // template element を linear scan で拾う (get_template_contents に相当
        // する raikiri-traits API は無いため、arena 直参照で node.template_contents()
        // を読む)。
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

        // (1) fragment root は template element と別 arena slot に居る。
        assert_ne!(
            frag_root_id, template_id,
            "fragment root must be a distinct arena node, not the template element itself"
        );

        // (2) template element 自身の arena children は空 (reshape で全ての
        // contents が fragment root に付いた)。
        assert!(
            template_node.children.is_empty(),
            "template element's own arena children must be empty (contents belong to fragment root); got {:?}",
            template_node.children
        );

        // (3) fragment root は NodeKind::DocumentFragment として存在する
        //     (raikiri-spike-84y — 旧: "#document-fragment" pseudo-tag Element)。
        //     as_element() == None、tag_name() == None (pseudo-tag pollution 廃止)、
        //     しかし children slot は使えて <span> を保持する。
        let frag_root = doc
            .get_node(frag_root_id)
            .expect("fragment root should exist in arena");
        assert_eq!(
            frag_root.kind(),
            NodeKind::DocumentFragment,
            "fragment root must be NodeKind::DocumentFragment (84y contract, replaces '#document-fragment' pseudo-tag)"
        );
        assert_eq!(
            frag_root.tag_name(),
            None,
            "fragment root must not carry a tag_name (Two-way invariant: non-Element kind → tag_name None)"
        );
        // NodeRef 経由でも as_element() == None を confirm (dom_impl surface で
        // Two-way invariant が保たれることを end-to-end で pin)。
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

        // (4) 37c invariant: template 自身は in_document、fragment root と
        // <span> はどちらも out-of-document (Document root から reachable
        // でないため `mark_in_document_flags` step 2 が set しない)。
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
        // <span> の text child も out-of-document。
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
        // raikiri-spike-37c: 深いネスト (template > div > span > text) でも
        // in_document bit が subtree 全体に伝播する。single-pass DFS で
        // in_template state が正しく引き継がれることを pin。
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
        // raikiri-spike-37c: cascade が template subtree を skip する silent bug fix
        // regression pin。詳細な cascaded map の shape reflection は raikiri-style
        // 内部の unit test で担保するのが正道 (未存在なら Task 4 で追加)、この
        // integration test は "parse → cascade の chain が template 内 element を
        // 触っても error / panic しない" ことと、bit populate が cascade 呼び出し
        // 前後で保たれることを pin する。
        //
        // 追加 pin (roborev-equivalent advisor 指摘): resolve_inheritance の
        // is_in_document() gate 実装ミスは `cascade.computed.len() ==
        // dom.node_count()` という m1.23 contract (raikiri/src/lib.rs
        // `html_document_cascade_populated_after_construct` が非-template
        // document でのみ pin していた) を template を含む document で破り得る
        // — raikiri-dom::layout::preshape_text / raikiri-paint::text::draw_text_node
        // は node_id で `cascade.computed[idx]` に直接 index するため、破れると
        // OOB panic に繋がる。TestDoc 経由の raikiri-style 内部 unit test は
        // is_in_document() が常に true な default 実装のため、この contract
        // 破れを検出できない (parse 経由で実際に bit が false になる document
        // でのみ再現する) — 本 integration test がそのカバレッジを担う。
        let html = b"<html><head><style>p { color: red }</style></head><body>\
                     <p>outer</p>\
                     <template><p id=\"inner\">inner</p></template>\
                     </body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let tree = raikiri_style::build_rule_tree(&uncascaded.dom);
        let cascade = raikiri_style::cascade(&uncascaded.dom, &tree)
            .expect("cascade must not error / panic on template subtree");

        // m1.23 contract: cascade.computed.len() == dom.node_count() でなければ
        // ならない — たとえ template 子孫が cascade gate で skip されても、
        // index 契約 (raikiri-dom / raikiri-paint が node_id で直接 index) を
        // 破ってはいけない。
        assert_eq!(
            cascade.computed.len(),
            uncascaded.dom.node_count(),
            "cascade.computed.len() must equal node_count() even with template descendants (m1.23 contract)"
        );

        // cascade 呼び出し後も inner <p> は out-of-document のまま (cascade が bit
        // を触ることは無いという contract の pin)。
        //
        // roborev job 293 L1 finding: 加えて outer <p> と inner <p> の ComputedValues
        // を実際に検証する。gate が動いていれば outer には `p { color: red }` rule
        // が適用され CssColor { r:255, g:0, b:0 } となり、inner には rule が適用
        // されず initial (CssColor::BLACK = { r:0, g:0, b:0 }) が残る。もし cascade
        // gate を両方削除したら inner にも red rule が届き BLACK ではなくなるため、
        // この assert 対で gate 動作が本当に発火していることを pin する。
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
        // bd raikiri-spike-cmd3 spec-lens finding: no test anywhere pinned
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
        // on `raikiri-style` directly, so this crate can pin it today
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
        // bd raikiri-spike-5z86.3: HTML LS §phrasing-content-3's
        // `a:link, a:visited { color: #0000EE; text-decoration: underline; }`
        // is approximated here as `a[href] { color: #0000EE; text-decoration:
        // underline; }` (no `:link`/`:visited` — Non-Goal, see the UA rule's
        // comment in `minimal.css`; `:link` itself additionally requires an
        // `href` attribute per HTML LS §selector-link, gated here via
        // `[href]`). Same "survives real parse+cascade, not just literal
        // text" concern as the `hr` test above. Also pins the `[href]` gate
        // itself: a bare `<a id="anchor">` with no `href` (e.g. a fragment
        // target, not a link at all per spec) must stay at CSS-initial
        // (spec-lens/debt-lens finding on the original unconditional `a { }`
        // rule, bd raikiri-spike-5z86.3 review round).
        use raikiri_style::Origin;
        use raikiri_style::property::{CssColor, TextDecoration};

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
            computed.text_decoration,
            TextDecoration::Underline,
            "a[href]'s UA rule text-decoration: underline must reach computed.text_decoration through real parse+cascade"
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
            bare_computed.text_decoration,
            TextDecoration::None,
            "a without href must stay at CSS-initial text-decoration, not the a[href] UA rule's underline"
        );
    }

    #[test]
    fn parse_ignores_body_style_in_m1_scope() {
        // 設計仕様書 §6 MVP: <head> 内 <style> のみ登録。<body> 内 <style> は
        // position-aware semantics を要するため defer。
        let html = b"<html><head><style>p{color:red}</style></head>\
                     <body><style>p{color:blue}</style><p>x</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        // Only <head> style should be extracted.
        assert_eq!(
            uncascaded.stylesheet_sources,
            vec![String::from("p{color:red}")]
        );
    }

    // ── Attribute / namespace wiring (raikiri-spike-blg) ────────

    #[test]
    fn parse_wires_style_attribute_to_inline_style_source() {
        // real HTML `<p style="color:red">` → cascade が消費できる
        // inline_style_source が populate される (end-to-end verify)。
        let html = b"<html><body><p style=\"color:red\">Hi</p></body></html>";
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p_elem = p_node.as_element().expect("p is element");
        assert_eq!(p_elem.inline_style_source(), Some("color:red"));
        // `style` は attributes には積まれない (Node.inline_style 側に分離)。
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
        // HTML default namespace は fast path として None を返す。
        assert_eq!(p.namespace_uri(), None);
    }

    #[test]
    fn parse_wires_svg_namespace_uri() {
        // html5ever は <svg> 内 element を automatically SVG namespace に置く。
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
        // trait contract: `style=""` は inline_style_source が None。
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
        // Defensive: html5ever は tokenizer 段で duplicate attr を dedupe する
        // が (§13.2.5.32)、raikiri-html sink 単体が受け取る Vec<Attribute> が
        // duplicate を含む可能性を排除しない (external consumer が TreeSink を
        // wrap して重複 attr を注入する scenario も含む)。この test は sink
        // 単体を driver に見立てて "style を 2 回渡すと最初 (color:red) が勝つ"
        // 挙動を pin する。
        use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use markup5ever::{LocalName, Namespace};

        let sink = RaikiriTreeSink::new();
        let name = QualName::new(
            None,
            Namespace::from("http://www.w3.org/1999/xhtml"),
            LocalName::from("p"),
        );
        let attr = |v: &str| Attribute {
            name: QualName::new(None, Namespace::from(""), LocalName::from("style")),
            value: StrTendril::from(v),
        };
        // sink に "style=color:red" と "style=color:blue" を順に渡す。
        let attrs = vec![attr("color:red"), attr("color:blue")];
        let idx = sink.create_element(name, attrs, ElementFlags::default());
        // Document の Handle は root。attach しないと finish 前に見つからないため
        // append 経由で root child にする。
        sink.append(
            &sink.get_document(),
            html5ever::interface::NodeOrText::AppendNode(idx),
        );
        let uncascaded = sink.finish();

        let p_id = find_first_by_tag(&uncascaded.dom, "p").expect("p exists");
        let p_node = uncascaded.dom.node(p_id).expect("p node exists");
        let p = p_node.as_element().expect("p is element");
        // first-wins (regression pin for wire_side_tables)。
        assert_eq!(p.inline_style_source(), Some("color:red"));
    }

    // ── MathML annotation-xml integration point (raikiri-spike-eil) ─────
    //
    // HTML5 §13.2.5 tree construction: MathML `annotation-xml` element は
    // `encoding` attribute の value が ASCII case-insensitive で `text/html`
    // または `application/xhtml+xml` と一致するときに HTML integration point
    // となる。tree construction algorithm の branch 判定に使う中心 predicate。

    /// Helper: annotation-xml element を指定 namespace + encoding で作成し、
    /// `is_mathml_annotation_xml_integration_point` の返り値を返す。
    fn probe_annotation_xml_integration_point(
        ns_uri: &str,
        local: &str,
        encoding: Option<&str>,
    ) -> bool {
        use html5ever::interface::{Attribute, ElementFlags, QualName, TreeSink};
        use html5ever::tendril::StrTendril;
        use markup5ever::{LocalName, Namespace};

        let sink = RaikiriTreeSink::new();
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
        // spec は text/html と application/xhtml+xml の 2 種のみ integration point。
        // application/xml / image/svg+xml / 空文字列 は non-match。
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
        // Defense in depth: annotation-xml でも MathML namespace 以外では
        // integration point ではない (spec の主語が "MathML annotation-xml")。
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
        // annotation-xml 以外の MathML 要素は integration point ではない。
        assert!(!probe_annotation_xml_integration_point(
            "http://www.w3.org/1998/Math/MathML",
            "mi",
            Some("text/html"),
        ));
    }

    #[test]
    fn annotation_xml_duplicate_encoding_attribute_uses_first_value() {
        // HTML §13.2.5.32 duplicate attribute → ignore later occurrences。
        // sink_first_wins_on_duplicate_style_attribute と同じ first-wins 契約を
        // integration point 判定でも守る (later match が earlier non-match を
        // 上書きしないことを pin する)。
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
            let sink = RaikiriTreeSink::new();
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
            let sink = RaikiriTreeSink::new();
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

    // End-to-end parse: annotation-xml integration point の判定は tree
    // construction algorithm の branch を切り替えるので、parse 経由でも
    // "子要素の namespace が予想通りか" で観測できる。
    // - encoding=text/html → HTML integration point 発動 → 子は HTML namespace
    //   (raikiri-dom fast path で namespace_uri() = None)
    // - encoding 不在 → 通常の MathML foreign content → 子は MathML namespace

    #[test]
    fn parse_annotation_xml_integration_point_inherits_html_namespace_for_children() {
        // annotation-xml encoding=text/html は HTML integration point。中の
        // 未知要素 <foo> は HTML namespace として解釈されるべき。
        let html =
            br#"<math><annotation-xml encoding="text/html"><foo>x</foo></annotation-xml></math>"#;
        let opts = empty_options();
        let uncascaded = parse(&html[..], &opts).expect("parse ok");
        let foo_id = find_first_by_tag(&uncascaded.dom, "foo").expect("foo exists");
        let foo_node = uncascaded.dom.node(foo_id).expect("foo node exists");
        let foo = foo_node.as_element().expect("foo is element");
        assert_eq!(
            foo.namespace_uri(),
            None,
            "child inside HTML integration point should be HTML (None fast path)"
        );
    }

    #[test]
    fn parse_annotation_xml_non_integration_wraps_children_in_mathml_namespace() {
        // annotation-xml (encoding 不在) は integration point ではない。中の
        // 未知要素 <foo> は MathML foreign content として MathML namespace で解釈される。
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
        // raikiri-spike-84y contract rewrite (旧: 100 個の "#comment" pseudo-tag
        // Element が strip されるか、を "#comment" tag の非存在で確認)。
        // 84y 後は Comment kind node が 100 個 arena に存在し、taffy child_count
        // からは 100 個すべて filter され、body の taffy child は <p> の 1 個のみ、
        // という bulk invariant を positive に pin する。旧 form は Comment が
        // Element でないため as_element() == None → scan は空振り → vacuously
        // pass するため content 保証にならない (advisor).
        use raikiri_traits::{Dom, Node};

        // 100 comments under body — verifies mark_in_document_flags handles bulk
        // correctly (旧 retain_children ベース bulk strip の 代替 stress test)。
        let mut html = String::from("<html><head></head><body>");
        for i in 0..100 {
            html.push_str(&format!("<!-- comment {i} -->"));
        }
        html.push_str("<p>x</p></body></html>");
        let opts = empty_options();
        let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
        let doc = &uncascaded.dom;

        // (a) arena 中の Comment node の総数 == 100 (persist されている)。
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
        // (b) すべての Comment の IS_IN_DOCUMENT bit は clear されている
        //     (bulk mark_in_document_flags 契約)。
        assert_eq!(
            in_document_comments, 0,
            "all 100 Comment nodes must have IS_IN_DOCUMENT cleared"
        );

        // (c) body の taffy child_count == 1 (<p> only)、100 個の Comment は
        //     TaffyChildIter の is_in_document filter で完全に除去される
        //     (attacker-controlled bulk stress でも leak しない = defense-in-depth)。
        let body_id = find_first_by_tag(doc, "body").expect("body exists");
        use taffy::TraversePartialTree;
        let body_taffy_id = taffy::NodeId::from(body_id.0 as usize);
        assert_eq!(
            <raikiri_dom::Document as TraversePartialTree>::child_count(doc, body_taffy_id),
            1,
            "taffy body child_count must be 1 (only <p>), 100 comments filtered"
        );
    }

    // ── UA CSS bundle (M1.4a、raikiri-spike-m1.22) ──────────────

    #[test]
    fn minimal_ua_css_covers_required_display_block_selectors() {
        // spec §M1.4a Scope: html, body, div, p, h1-h6 が display: block を持つ。
        //
        // 各 tag について、rule 行の存在を検査する — 「行を trim_start した後
        // `{tag}` で始まり、その直後 whitespace を挟んで `{` が来る」ケースだけ
        // 選択子と扱う。素の contains() だと `p` が comment 内の `Appendix` /
        // `paragraph` / `display` の一部に match してしまう (roborev job 217 low
        // 対応)。
        // bd raikiri-spike-5z86.1: article/section/nav/aside/header/footer/
        // main/figure/figcaption/blockquote 追加 (block-level sectioning /
        // grouping elements)。cascade まで通した非-vacuous な検証は
        // `crates/raikiri/tests/build_cascaded.rs`
        // `sectioning_and_grouping_elements_are_display_block_via_ua_css` 側。
        // bd raikiri-spike-5z86.2: ol/ul/li 追加 (block-level list
        // treatment、marker/list-style は Epic 4 へ defer)。li は spec の
        // `display: list-item` が raikiri-style で未実装のため display:
        // block に fallback、list-item 実装は bd raikiri-spike-uhzy で
        // track (詳細は minimal.css のコメント参照)。cascade まで通した
        // 非-vacuous な検証は `crates/raikiri/tests/build_cascaded.rs`
        // `list_elements_are_display_block_via_ua_css` 側。
        // bd raikiri-spike-5z86.5: hr 追加 (display: block は §flow-content-3
        // (15.3.3) の flow-content グループ側の rule に相乗り。border/color/
        // margin の hr 固有 rule は別 group、詳細は minimal.css のコメント
        // 参照)。cascade まで通した非-vacuous な検証は
        // `crates/raikiri/tests/build_cascaded.rs`
        // `hr_is_display_block_border_inset_and_margin_via_ua_css` 側。
        // bd raikiri-spike-xhgn: hgroup 追加 (article/aside/nav/section と
        // 同じ §sections-and-headings (15.3.6) selector group の一員、
        // 5z86.1 の scope からは漏れていた)。cascade まで通した非-vacuous
        // な検証は `crates/raikiri/tests/build_cascaded.rs`
        // `sectioning_and_grouping_elements_are_display_block_via_ua_css`
        // 側 (既存 loop に追加)。
        // bd raikiri-spike-cfbo: address/center/listing/plaintext/search/xmp
        // 追加 (§flow-content-3 (15.3.3) の display:block selector の残り、
        // bd raikiri-spike-5z86.1 の audit で未追跡と判明した7要素のうち6つ)。
        // center/listing/plaintext/xmp は HTML LS §16.2 上は
        // "entirely obsolete" 分類だが、その分類は authoring conformance の
        // 話であって UA rendering の話ではない (詳細は minimal.css のコメント
        // 参照)。dialog (7要素目) はこのループには含めない —
        // まだ本ファイルに rule 自体が無い (deliberately deferred、詳細は
        // minimal.css のコメント参照)。当初 `dialog { display: none; }` /
        // `dialog[open] { display: block; }` の2 rule ペアで追加していたが、
        // reviewer:spec が regression を発見し amend で削除した:
        // `raikiri_dom::ElementRef::attr()` が空文字列属性値を `None` に
        // 正規化する bug (bd raikiri-spike-kxki) により、canonical form の
        // 素の `<dialog open>` では `dialog[open]` が発火せず、無条件の
        // `dialog { display: none; }` だけが効いてしまう —
        // rule 追加前 (no-rule → CSS-initial `inline`、box type は誤りだが
        // content は見える) より悪化する (`display: none` で content が
        // 完全に不可視になる) regression だったため。bd raikiri-spike-wezw
        // (kxki 解消後に再導入する followup) で追跡する。cascade まで通した非-vacuous な検証は
        // 追加した6要素分は
        // `crates/raikiri/tests/build_cascaded.rs`
        // `flow_content_3_residue_elements_are_display_block_via_ua_css` 側。
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
        // spec 参照コメントが含まれていること (CSS 2.1 App.D 由来の cleanroom 印)
        assert!(
            MINIMAL_UA_CSS.contains("CSS 2.1 App.D"),
            "MINIMAL_UA_CSS should contain spec reference comments",
        );
        // display: block declaration が含まれていること (直接文字列で確認)
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
    fn parse_injects_extra_stylesheets_as_author() {
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

        let author_entries: Vec<&str> = doc
            .dom
            .stylesheets()
            .filter(|(_, k)| *k == StylesheetKind::Author)
            .map(|(s, _)| s)
            .collect();
        assert_eq!(author_entries.len(), 2);
        assert_eq!(author_entries[0], extra_a);
        assert_eq!(author_entries[1], extra_b);
    }
}
