//! White-box tests for the assembled document, parse, and render entry points.

use super::*;
use raikiri_traits::{
    AbortController, PageDefaults, ParseError, PlanConfig, RenderError, RenderLimits, RenderSink,
    RenderStatus, ReplacedResolver, StreamingConfig,
};

#[cfg(test)]
mod html_document_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = crate::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
        let cascade = build_cascaded(&uncascaded);
        // Construct internal fields directly (pub(crate) fields are accessible in crate-internal tests).
        HtmlDocument {
            uncascaded,
            cascade,
            font_faces: raikiri_style::FontFaceRegistry::new(),
            effective_base_url: None,
        }
    }

    #[test]
    fn consumer_property_cascade_wrapper_accepts_registration() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded =
            crate::parse(&b"<p style=\"bookmark-level: 4\">Hi</p>"[..], &opts).expect("parse");
        let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
        let cascade = build_cascaded_with_consumer_properties(&uncascaded, &registrations);
        assert!(!cascade.computed.is_empty());
    }

    #[test]
    fn html_document_accessors_expose_underlying_types() {
        let doc = hello_world_doc();
        // The accessor has the same identity as the inner field (no separate heap allocation).
        let dom_ref: &raikiri_dom::Document = doc.dom();
        let cascade_ref: &CascadeResult = doc.cascade();
        let sources_ref: &[String] = doc.stylesheet_sources();

        assert!(
            std::ptr::eq(dom_ref, &doc.uncascaded.dom),
            "dom() must return &doc.uncascaded.dom"
        );
        assert!(
            std::ptr::eq(cascade_ref, &doc.cascade),
            "cascade() must return &doc.cascade"
        );
        assert!(
            std::ptr::eq(
                sources_ref.as_ptr(),
                doc.uncascaded.stylesheet_sources.as_ptr()
            ) || (sources_ref.is_empty() && doc.uncascaded.stylesheet_sources.is_empty()),
            "stylesheet_sources() must alias inner Vec"
        );
    }

    #[test]
    fn html_document_into_parts_returns_owned_halves() {
        let doc = hello_world_doc();
        let expected_nodes = doc.dom().node_count();
        let expected_computed = doc.cascade().computed.len();
        let (uncascaded, cascade) = doc.into_parts();
        assert_eq!(uncascaded.dom.node_count(), expected_nodes);
        assert_eq!(cascade.computed.len(), expected_computed);
    }

    #[test]
    fn html_document_cascade_populated_after_construct() {
        let doc = hello_world_doc();
        assert!(
            !doc.cascade().computed.is_empty(),
            "cascade must be populated (build_cascaded produces per-node ComputedValues)"
        );
        // Contract between node_count and cascade.computed.len().
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade.computed.len() must equal document.node_count()"
        );
    }
}

#[cfg(test)]
mod parse_html_tests {
    use super::*;

    #[test]
    fn parse_html_returns_html_document_with_cascade_populated() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html should succeed");
        assert!(
            !doc.cascade().computed.is_empty(),
            "parse_html output must have populated cascade"
        );
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade / dom node_count invariant"
        );
    }

    #[test]
    fn parse_html_propagates_parse_error_from_io() {
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err = parse_html(FailingReader, &opts).expect_err("must fail on reader error");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Io(_))),
            "expected RenderError::Parse(ParseError::Io), got {err:?}"
        );
    }

    #[test]
    fn parse_html_propagates_utf8_error() {
        // 0x80 alone is a UTF-8 continuation byte and invalid UTF-8.
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err =
            parse_html(&[0x80u8, 0x80, 0x80][..], &opts).expect_err("must fail on invalid UTF-8");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Encoding { .. })),
            "expected RenderError::Parse(ParseError::Encoding), got {err:?}"
        );
    }

    #[test]
    fn parse_html_baked_cascade_matches_manual_build_cascaded() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // Check that the two cascade paths produce the same result (parse_html calls
        // build_cascaded internally by contract).
        let via_parse_html = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");
        let via_manual = {
            let uncascaded = crate::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
            build_cascaded(&uncascaded)
        };
        assert_eq!(
            via_parse_html.cascade().computed.len(),
            via_manual.computed.len(),
            "parse_html と手動 build_cascaded で cascade node 数が一致"
        );
    }

    // `HtmlDocument.uncascaded` is `pub(crate)` (not public to consumers).
    // Verifying that `RenderLimits::max_parse_warnings` is actually consulted by
    // `RaikiriTreeSink` during parsing therefore requires a white-box test
    // inside this crate (to access `doc.uncascaded.warnings`).
    // No public consumer can access this field.

    #[test]
    fn parse_html_with_limits_consults_custom_max_parse_warnings() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // `</x>` has no matching start element, so html5ever's error recovery
        // algorithm reports roughly one nonfatal parse error per closing tag.
        // This matches the input shape used in similar raikiri_html tests. With cap=5,
        // last_real_slot=4, allowing four real warnings plus one synthetic warning:
        // five in total (the reserved-last-slot contract; see the docs for
        // `RaikiriTreeSink::parse_error`).
        let malformed = b"</x>".repeat(50);

        let strict_limits = RenderLimits::builder().max_parse_warnings(Some(5)).build();
        let strict_doc = parse_html_with_limits(malformed.as_slice(), &opts, strict_limits)
            .expect("malformed input still recovers");
        assert_eq!(
            strict_doc.uncascaded.warnings.len(),
            5,
            "custom max_parse_warnings=5 must be consulted and yield exactly 4 real + 1 synthetic warning"
        );
        match &strict_doc.uncascaded.warnings.last().expect("cap > 0").kind {
            raikiri_traits::WarningKind::HtmlParseError { message } => {
                assert!(
                    message.contains("5-warning cap"),
                    "last entry must be the synthetic suppression notice naming the runtime cap (5), got: {message:?}"
                );
            }
            other => panic!("expected HtmlParseError, got {other:?}"),
        }

        // By contrast, parsing the same input with the default cap (1024) records
        // more than five warnings. This confirms that the cap is read from RenderLimits
        // rather than the test merely staying under a fixed, small limit.
        let default_doc =
            parse_html_with_limits(malformed.as_slice(), &opts, RenderLimits::default())
                .expect("malformed input still recovers");
        assert!(
            default_doc.uncascaded.warnings.len() > strict_doc.uncascaded.warnings.len(),
            "default cap must allow strictly more warnings than the custom cap=5, got default={} strict={}",
            default_doc.uncascaded.warnings.len(),
            strict_doc.uncascaded.warnings.len()
        );
    }

    #[test]
    fn parse_html_with_limits_zero_max_parse_warnings_records_one_synthetic_entry() {
        // cap=0 has no slot for a real warning, but must still record exactly
        // one synthetic suppression entry when parse errors actually occur,
        // consistent with cap>0's trip-and-record-suppression semantics.
        // Pins that this boundary is reachable via `RenderLimits`, not just
        // via `RaikiriTreeSink::new()` directly.
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let malformed = b"</x>".repeat(50);

        let limits = RenderLimits::builder().max_parse_warnings(Some(0)).build();
        let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
            .expect("malformed input still recovers");
        assert_eq!(
            doc.uncascaded.warnings.len(),
            1,
            "max_parse_warnings=Some(0) must record exactly one synthetic suppression entry, got {}",
            doc.uncascaded.warnings.len()
        );
    }

    #[test]
    fn parse_html_with_limits_none_max_parse_warnings_disables_cap() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // 2,000 `</x>` closing tags reliably reach the default cap (1024),
        // as confirmed by similar tests in raikiri_html.
        let malformed = b"</x>".repeat(2_000);

        let mut limits = RenderLimits::default();
        limits.max_parse_warnings = None;
        let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
            .expect("malformed input still recovers");

        assert!(
            doc.uncascaded.warnings.len() > 1024,
            "max_parse_warnings=None must not cap warnings at the default 1024, got {}",
            doc.uncascaded.warnings.len()
        );
    }
}

#[cfg(test)]
mod stub_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse")
    }

    /// Current focused documents do not contain replaced elements, so this
    /// resolver is expected not to be called.
    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(
            &self,
            _req: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            unreachable!("test document has no replaced element")
        }
    }

    /// Recording sink used to pin page emission and completion ordering.
    #[derive(Default)]
    struct RecordingSink {
        pages: usize,
        summary: Option<raikiri_traits::RenderSummary>,
    }
    impl RenderSink for RecordingSink {
        fn accept_page(
            &mut self,
            _fragment: raikiri_traits::PageFragment,
        ) -> Result<(), std::io::Error> {
            self.pages += 1;
            Ok(())
        }
        fn finish_render(
            &mut self,
            summary: raikiri_traits::RenderSummary,
        ) -> Result<(), std::io::Error> {
            self.summary = Some(summary);
            Ok(())
        }
    }

    #[test]
    fn plan_returns_unimplemented_with_feature_name() {
        let doc = hello_world_doc();
        let err = plan(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            PlanConfig::default(),
        )
        .expect_err("unimplemented plan API must return Err");
        match err {
            RenderError::Unimplemented { feature, .. } => {
                assert_eq!(feature, "plan", "feature must identify plan API");
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn render_streaming_emits_pages_and_finishes_once() {
        let doc = hello_world_doc();
        let mut sink = RecordingSink::default();
        let status = render_streaming(
            &doc,
            PageDefaults::default(),
            StreamingConfig::default(),
            RenderOptions::new(),
            &mut sink,
        )
        .expect("render_streaming should complete");
        let summary = match status {
            RenderStatus::Completed(summary) => summary,
            RenderStatus::Aborted { partial_pages } => {
                panic!("unexpected abort after {partial_pages} pages")
            }
            _ => panic!("unexpected non-exhaustive render status"),
        };
        assert_eq!(sink.pages, 1);
        assert_eq!(summary.total_pages, 1);
        assert_eq!(sink.summary.expect("finish_render").total_pages, 1);
    }

    #[test]
    fn render_options_debug_reports_configured_parts_without_observer_contents() {
        struct Observer;
        impl raikiri_traits::PageEventObserver for Observer {
            fn observe_event(
                &mut self,
                _event: raikiri_traits::PageFragmentEvent,
            ) -> std::io::Result<()> {
                Ok(())
            }
        }
        let resources = RenderResources::new();
        let mut observer = Observer;
        let options = RenderOptions::new()
            .resources(&resources)
            .page_observer(&mut observer);
        let debug = format!("{options:?}");
        assert!(debug.contains("has_page_observer: true"), "{debug}");
        assert!(debug.contains("consumer_property_count: 0"), "{debug}");
        assert!(debug.contains("has_property_observer: false"), "{debug}");
        assert!(debug.contains("RenderResources"), "{debug}");
    }

    #[test]
    fn render_streaming_abort_skips_completion() {
        let doc = hello_world_doc();
        let controller = AbortController::new();
        controller.abort();
        let config = StreamingConfig::builder()
            .signal(Some(controller.signal.clone()))
            .build();
        let mut sink = RecordingSink::default();
        let status = render_streaming(
            &doc,
            PageDefaults::default(),
            config,
            RenderOptions::new(),
            &mut sink,
        )
        .expect("abort is a status, not an error");
        assert!(matches!(status, RenderStatus::Aborted { partial_pages: 0 }));
        assert_eq!(sink.pages, 0);
        assert!(sink.summary.is_none());
    }
}
