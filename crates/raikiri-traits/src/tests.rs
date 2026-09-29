use super::*;

// ── DOM foundations ─────────────────────────────────────────

#[test]
fn symbol_construct_from_str() {
    let s = Symbol::from("target-1");
    assert_eq!(s.as_str(), "target-1");
}

#[test]
fn nodeid_construct() {
    let n = NodeId::new(42);
    assert_eq!(n.0, 42);
}

// ── Page placeholders ───────────────────────────────────────

#[test]
fn page_placeholders_default_construct() {
    let _ = PageBox::default();
    let _ = PageContext::default();
    let _ = LayoutBuffer::default();
    let _ = TargetRegistry::default();
    let _ = RunningTemplate::default();
    let _ = FormData::default();
}

// ── Trait object safety ─────────────────────────────────────

#[test]
fn dyn_traits_are_object_safe() {
    fn _assert<T: ?Sized>() {}
    _assert::<dyn ReplacedResolver>();
    _assert::<dyn ImagePixelSource>();
    _assert::<dyn NetworkProvider>();
    _assert::<dyn ResourcePolicy>();
    // Strategy traits (LookaheadPolicy / TargetResolver / EmissionPolicy /
    // ReflowPolicy) are passed through generic parameters (the §4
    // `render_with<L,T,E,R>` design), so object safety is not required. Revisit
    //
    // Dom / Element / Node will later gain associated types / GATs
    // and likely become non-object-safe. Do not assert object safety
    // at this stage.
}

// ── AbortController semantic ────────────────────────────────

#[test]
fn abort_controller_default_and_abort() {
    let c = AbortController::new();
    assert!(!c.signal.is_aborted());
    c.abort();
    assert!(c.signal.is_aborted());
}

// ── Resolver placeholders ───────────────────────────────────

#[test]
fn resolver_placeholder_types_default_construct() {
    let _ = IntrinsicBox::default();
}

// ── Error taxonomy ──────────────────────────────────────────

#[test]
fn render_error_is_error_trait() {
    fn _assert<T: std::error::Error>() {}
    _assert::<RenderError>();
}

#[test]
fn exhaustion_policy_default_is_error() {
    assert_eq!(ExhaustionPolicy::default(), ExhaustionPolicy::Error);
}

#[test]
fn target_slot_id_construct() {
    let id = TargetSlotId {
        page_index: 3,
        sequence: 7,
    };
    assert_eq!(id.page_index, 3);
    assert_eq!(id.sequence, 7);
}

// ── Strategy placeholders ───────────────────────────────────

#[test]
fn strategy_placeholders_default_construct() {
    let _ = ProbeContext::default();
    let _ = TargetRequest::default();
}

// ── Config defaults ─────────────────────────────────────────

#[test]
fn all_configs_default_construct() {
    let _ = LookaheadConfig::new();
    let _ = LookaheadConfig::default();
    let _ = RenderLimits::default();
    let _ = RenderLimits::new();
    let _ = LayoutConfig::default();
    let _ = BatchConfig::default();
}

#[test]
fn lookahead_config_defaults() {
    let d = LookaheadConfig::default();
    assert_eq!(d.widow_line_buffer, 2);
    assert_eq!(d.orphan_line_buffer, 2);
    assert_eq!(d.break_avoid_max_subtree_blocks, 20);
    assert_eq!(d.max_container_probe_pages, Some(4));
    assert!(!d.allow_cross_size_lookahead);
}

#[test]
fn render_limits_defaults() {
    let d = RenderLimits::default();
    assert_eq!(d.max_document_pages, Some(10_000));
    assert_eq!(d.max_dom_nodes, Some(1_000_000));
    assert_eq!(d.max_target_slots, Some(100_000));
    assert_eq!(d.max_layout_buffer_entries, Some(10_000));
    assert_eq!(d.max_aggregate_bytes, Some(1_073_741_824));
    // Inherit the former hard-coded 32 MiB cap on parse-time input reads
    // from raikiri-html.
    assert_eq!(d.max_input_bytes, Some(32 * 1024 * 1024));
    // Inherit the former hard-coded value (1024) from raikiri-html's
    // RaikiriTreeSink.
    assert_eq!(d.max_parse_warnings, Some(1024));
}

// ── Config builders ─────────────────────────────────────────

#[test]
fn lookahead_config_builder_roundtrip() {
    let cfg = LookaheadConfig::builder()
        .widow_line_buffer(5)
        .max_container_probe_pages(None)
        .build();
    assert_eq!(cfg.widow_line_buffer, 5);
    assert_eq!(cfg.max_container_probe_pages, None);
    assert_eq!(
        cfg.orphan_line_buffer,
        LookaheadConfig::default().orphan_line_buffer
    );
}

#[test]
fn render_limits_builder_roundtrip() {
    let cfg = RenderLimits::builder()
        .max_document_pages(Some(100))
        .max_dom_nodes(None)
        .build();
    assert_eq!(cfg.max_document_pages, Some(100));
    assert_eq!(cfg.max_dom_nodes, None);
    assert_eq!(
        cfg.max_target_slots,
        RenderLimits::default().max_target_slots
    );
}

#[test]
fn render_limits_max_parse_warnings_builder_roundtrip() {
    let via_builder = RenderLimits::builder().max_parse_warnings(Some(16)).build();
    assert_eq!(via_builder.max_parse_warnings, Some(16));

    // Direct field construction (matching sibling conventions without
    // ergonomic `with_*` methods). Struct-update syntax works within this
    // crate. The external-consumer view of direct assignment is covered by
    // `external_consumer_can_mutate_pub_fields_via_default_shorthand`
    // instead.
    let via_field = RenderLimits {
        max_parse_warnings: Some(16),
        ..RenderLimits::default()
    };
    assert_eq!(via_field.max_parse_warnings, Some(16));

    // The builder also accepts None to disable the cap.
    let unbounded = RenderLimits::builder().max_parse_warnings(None).build();
    assert_eq!(unbounded.max_parse_warnings, None);
}

#[test]
fn streaming_config_builder_roundtrip() {
    let limits = RenderLimits::builder().max_document_pages(Some(50)).build();
    let controller = AbortController::new();
    let cfg = LayoutConfig::builder()
        .limits(limits)
        .signal(Some(controller.signal.clone()))
        .build();
    assert_eq!(cfg.limits.max_document_pages, Some(50));
    assert!(cfg.signal.is_some());
}

#[test]
fn batch_config_builder_roundtrip() {
    let limits = RenderLimits::builder().max_document_pages(Some(30)).build();
    let cfg = BatchConfig::builder().limits(limits).build();
    assert_eq!(cfg.limits.max_document_pages, Some(30));
}

// ── Plan placeholders ───────────────────────────────────────

#[test]
fn plan_placeholder_default_construct() {
    let _ = TargetDefinition::default();
}

// ── Sub-error trait bounds ───────────────────────────

#[test]
fn parse_error_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<ParseError>();
    _assert_display::<ParseError>();
}

#[test]
fn parse_error_io_source_chain() {
    let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
    let pe: ParseError = io_err.into();
    use std::error::Error as _;
    let src = pe.source();
    assert!(
        src.is_some(),
        "ParseError::Io should expose inner io::Error via source()"
    );
}

#[test]
fn cascade_error_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<CascadeError>();
    _assert_display::<CascadeError>();
}

#[test]
fn layout_error_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<LayoutError>();
    _assert_display::<LayoutError>();
}

#[test]
fn render_error_parse_source_chain() {
    use std::error::Error as _;
    let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
    let re: RenderError = RenderError::Parse(ParseError::Io(io_err));
    let src = re.source();
    assert!(
        src.is_some(),
        "RenderError::Parse should expose inner ParseError via source()"
    );
}

#[test]
fn question_mark_propagates_io_through_parse_to_render() {
    fn producer() -> Result<(), RenderError> {
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        let pe: ParseError = io_err.into();
        Err(pe)?;
        Ok(())
    }
    let e = producer().expect_err("expected RenderError");
    match e {
        RenderError::Parse(ParseError::Io(_)) => (),
        other => panic!("expected RenderError::Parse(ParseError::Io(_)), got {other:?}"),
    }
}

// ── Sub-error trait bounds ────────────────────────────

#[test]
fn policy_violation_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<PolicyViolation>();
    _assert_display::<PolicyViolation>();
}

// ── ViolationType::Display ──────────────
// Eliminate Debug-in-Display. Display output is a stability contract, so
// check every variant's string. Also assert representative field values
// to avoid silent changes to auto-derived Debug output when variant
// fields are added.

#[test]
fn violation_type_display_simple_variants() {
    assert_eq!(
        ViolationType::SchemeNotAllowed.to_string(),
        "scheme not allowed"
    );
    assert_eq!(
        ViolationType::HostNotAllowed.to_string(),
        "host not allowed"
    );
    assert_eq!(ViolationType::RedirectDenied.to_string(), "redirect denied");
    assert_eq!(ViolationType::Timeout.to_string(), "timeout exceeded");
    assert_eq!(
        ViolationType::Other.to_string(),
        "unspecified policy violation"
    );
}

#[test]
fn violation_type_display_size_limit_variants() {
    let fetch = ViolationType::FetchTooLarge {
        limit: 1_048_576,
        actual: 2_097_152,
    };
    assert_eq!(
        fetch.to_string(),
        "fetch too large (limit=1048576, actual=2097152)"
    );

    let decoded = ViolationType::DecodedTooLarge {
        limit: 10_000,
        actual: 12_345,
    };
    assert_eq!(
        decoded.to_string(),
        "decoded content too large (limit=10000, actual=12345)"
    );
}

#[test]
fn violation_type_display_string_and_depth_variants() {
    let mime = ViolationType::MimeNotAllowed {
        mime: String::from("application/octet-stream"),
    };
    assert_eq!(
        mime.to_string(),
        "MIME type not allowed: application/octet-stream"
    );

    let rec = ViolationType::RecursionExceeded { depth: 32 };
    assert_eq!(rec.to_string(), "recursion depth exceeded (32)");
}

#[test]
fn policy_violation_display_uses_violation_type_display_not_debug() {
    // Regression-check that PolicyViolation::Display formats violation_type
    // and kind with `{}`. The old implementation used `{:?}`, leaking
    // "FetchTooLarge { limit: .., actual: .. }" and "Image".
    // Also assert the kind after the Display swap.
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::Image,
        url: Url::parse("https://cdn.example.com/big.png").unwrap(),
        violation_type: ViolationType::FetchTooLarge {
            limit: 100,
            actual: 250,
        },
        details: String::from("Content-Length header too big"),
    };
    let s = v.to_string();
    assert!(
        s.contains("fetch too large (limit=100, actual=250)"),
        "PolicyViolation Display must delegate to ViolationType::Display, got: {s}"
    );
    assert!(
        !s.contains("FetchTooLarge {"),
        "PolicyViolation Display must not leak ViolationType Debug format, got: {s}"
    );
    // kind: Display is "image" (lowercase), Debug is "Image" (CamelCase).
    assert!(
        s.contains("(image "),
        "PolicyViolation Display must delegate to ResourceKind::Display, got: {s}"
    );
    assert!(
        !s.contains("Image"),
        "PolicyViolation Display must not leak ResourceKind Debug format, got: {s}"
    );
}

// ── ResourceKind::Display ───────────────
// Complete the Debug-in-Display removal (paired with ViolationType::Display).
// Display output is a stability contract; check all seven variant strings.
// To avoid silent changes to auto-derived Debug output when variants
// are added, pin each using per-variant assert_eq!, as in the
// `violation_type_display_*` pattern.

#[test]
fn resource_kind_display_stylesheet_import() {
    assert_eq!(
        ResourceKind::StylesheetImport.to_string(),
        "stylesheet @import"
    );
}

#[test]
fn resource_kind_display_external_stylesheet() {
    assert_eq!(
        ResourceKind::ExternalStylesheet.to_string(),
        "external stylesheet"
    );
}

#[test]
fn resource_kind_display_image() {
    assert_eq!(ResourceKind::Image.to_string(), "image");
}

#[test]
fn resource_kind_display_font() {
    assert_eq!(ResourceKind::Font.to_string(), "font");
}

#[test]
fn resource_kind_display_svg() {
    assert_eq!(ResourceKind::Svg.to_string(), "SVG");
}

#[test]
fn resource_kind_display_mathml() {
    assert_eq!(ResourceKind::MathML.to_string(), "MathML");
}

#[test]
fn resource_kind_display_other() {
    assert_eq!(ResourceKind::Other.to_string(), "other resource");
}

#[test]
fn render_error_policy_display_is_label_only_source_carries_details() {
    // RenderError::Policy Display follows the label-only convention of sibling
    // arms (Parse / Cascade / Layout / Resolver / Network / Sink / Io): details
    // are available through std::error::Error::source() via PolicyViolation Display.
    // The regression check that violation_type uses Display, not Debug,
    // is covered both by this test's source assertion and the test above
    // `policy_violation_display_uses_violation_type_display_not_debug`
    // (direct PolicyViolation Display).
    use std::error::Error;
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::Font,
        url: Url::parse("https://fonts.example.com/x.woff2").unwrap(),
        violation_type: ViolationType::MimeNotAllowed {
            mime: String::from("text/html"),
        },
        details: String::from("expected font/woff2"),
    };
    let re = RenderError::Policy(Box::new(v));
    // Top-level Display: label only, matches sibling arms convention.
    assert_eq!(re.to_string(), "Resource policy violation");
    // source() surfaces PolicyViolation whose Display carries kind / url /
    // details / violation_type (via ViolationType::Display, not Debug).
    let src = re.source().expect("RenderError::Policy has source");
    let src_s = src.to_string();
    assert!(
        src_s.contains("MIME type not allowed: text/html"),
        "source Display must use ViolationType::Display, got: {src_s}"
    );
    assert!(
        !src_s.contains("MimeNotAllowed {"),
        "source Display must not leak Debug format, got: {src_s}"
    );
}

#[test]
fn network_error_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<NetworkError>();
    _assert_display::<NetworkError>();
}

#[test]
fn network_error_policy_source_chain() {
    use std::error::Error as _;
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::Image,
        url: Url::parse("https://example.com/x.png").unwrap(),
        violation_type: ViolationType::HostNotAllowed,
        details: String::from("host not in allowlist"),
    };
    let ne = NetworkError::PolicyViolation(Box::new(v));
    let src = ne.source();
    assert!(
        src.is_some(),
        "NetworkError::PolicyViolation should expose inner PolicyViolation via source()"
    );
}

#[test]
fn network_error_io_source_chain() {
    use std::error::Error as _;
    let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
    let ne = NetworkError::Io(io_err);
    let src = ne.source();
    assert!(
        src.is_some(),
        "NetworkError::Io should expose inner io::Error via source()"
    );
}

#[test]
fn network_error_io_display_includes_inner_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
    let ne = NetworkError::Io(io_err);
    let s = ne.to_string();
    assert!(
        s.contains("Network I/O error"),
        "Display should preserve prefix, got: {s}"
    );
    assert!(
        s.contains("refused"),
        "Display should include inner io::Error message, got: {s}"
    );
}

#[test]
fn network_error_policy_display_delegates_to_policy_violation() {
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::Image,
        url: Url::parse("https://example.com/x.png").unwrap(),
        violation_type: ViolationType::HostNotAllowed,
        details: String::from("host not in allowlist"),
    };
    let ne = NetworkError::PolicyViolation(Box::new(v));
    let s = ne.to_string();
    assert!(
        s.contains("Network fetch violated policy"),
        "Display should keep Network prefix, got: {s}"
    );
    assert!(
        s.contains("https://example.com/x.png"),
        "Display should include PolicyViolation URL via delegation, got: {s}"
    );
    assert!(
        s.contains("host not in allowlist"),
        "Display should include PolicyViolation details via delegation, got: {s}"
    );
}

#[test]
fn network_error_display_and_source_none_variants() {
    use std::error::Error as _;

    let aborted = NetworkError::Aborted;
    assert_eq!(aborted.to_string(), "Network fetch aborted");
    assert!(aborted.source().is_none(), "Aborted has no inner error");

    let http = NetworkError::Http(503);
    assert_eq!(http.to_string(), "Network HTTP status error: 503");
    assert!(http.source().is_none(), "Http has no inner error");

    let other = NetworkError::Other(String::from("dns lookup failed"));
    assert_eq!(other.to_string(), "Network error: dns lookup failed");
    assert!(other.source().is_none(), "Other has no inner error");
}

#[test]
fn resolver_error_is_error_and_display() {
    fn _assert_error<T: std::error::Error>() {}
    fn _assert_display<T: std::fmt::Display>() {}
    _assert_error::<ResolverError>();
    _assert_display::<ResolverError>();
}

#[test]
fn render_error_network_source_chain() {
    use std::error::Error as _;
    let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
    let re = RenderError::Network(NetworkError::Io(io_err));
    let src = re.source();
    assert!(
        src.is_some(),
        "RenderError::Network should expose inner NetworkError via source()"
    );
}

#[test]
fn render_error_policy_source_chain() {
    use std::error::Error as _;
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::ExternalStylesheet,
        url: Url::parse("https://cdn.example.com/main.css").unwrap(),
        violation_type: ViolationType::MimeNotAllowed {
            mime: String::from("text/plain"),
        },
        details: String::from("expected text/css"),
    };
    let re = RenderError::Policy(Box::new(v));
    let src = re.source();
    assert!(
        src.is_some(),
        "RenderError::Policy should expose inner PolicyViolation via source()"
    );
}

#[test]
fn render_error_network_policy_nested_source_chain() {
    use std::error::Error as _;
    use url::Url;
    let v = PolicyViolation {
        kind: ResourceKind::Image,
        url: Url::parse("http://tracker.example.com/1x1.gif").unwrap(),
        violation_type: ViolationType::SchemeNotAllowed,
        details: String::from("http not allowed in strict mode"),
    };
    let re = RenderError::Network(NetworkError::PolicyViolation(Box::new(v)));
    // depth 1: RenderError → NetworkError
    let inner = re
        .source()
        .expect("RenderError::Network should delegate to NetworkError");
    // depth 2: NetworkError::PolicyViolation → PolicyViolation
    let deep = inner
        .source()
        .expect("NetworkError::PolicyViolation should delegate to PolicyViolation");
    // depth 3: PolicyViolation is a leaf (no inner error)
    assert!(
        deep.source().is_none(),
        "PolicyViolation should be the leaf of the chain"
    );
}

#[test]
fn element_defaults_return_none_or_false() {
    use crate::Element;

    struct BareElement;
    impl Element for BareElement {
        fn tag_name(&self) -> &str {
            "p"
        }
        // inline_style_source / namespace_uri / id / has_class / attr use
        // default implementations. The default attr delegates only "style"
        // to inline_style_source(); all others return None.
    }

    let e = BareElement;
    assert_eq!(e.inline_style_source(), None);
    assert_eq!(e.namespace_uri(), None);
    assert_eq!(e.id(), None);
    assert!(!e.has_class("anything"));
    assert_eq!(e.attr("data-foo"), None);
    // Default attr("style") delegates to inline_style_source (also None by default),
    // so the result is None.
    assert_eq!(e.attr("style"), None);
}

#[test]
fn element_default_id_and_has_class_delegate_to_attr() {
    // Regression-check that overriding only attr() in an implementation
    // makes id() / has_class() / attr("style") follow through their defaults.
    use crate::Element;

    struct AttrOnlyElement;
    impl Element for AttrOnlyElement {
        fn tag_name(&self) -> &str {
            "div"
        }
        fn inline_style_source(&self) -> Option<&str> {
            Some("color:red")
        }
        fn attr(&self, local: &str) -> Option<&str> {
            if local == "style" {
                return self.inline_style_source();
            }
            match local {
                "id" => Some("main"),
                "class" => Some("foo  bar\tbaz"),
                _ => None,
            }
        }
    }

    let e = AttrOnlyElement;
    // id() default → self.attr("id")
    assert_eq!(e.id(), Some("main"));
    // Default has_class() splits attr("class") on ASCII whitespace.
    assert!(e.has_class("foo"));
    assert!(e.has_class("bar"));
    assert!(e.has_class("baz"));
    assert!(!e.has_class("qux"));
    // attr("style") redirects to inline_style_source().
    assert_eq!(e.attr("style"), Some("color:red"));
}

// ── WarningKind extension ────────────────────────────

#[test]
fn warning_kind_html_parse_error_is_constructable() {
    // Regression-check that an external consumer like raikiri-html can construct
    // this variant despite the `#[non_exhaustive]` contract.
    let w = crate::WarningKind::HtmlParseError {
        message: String::from("unexpected end tag"),
    };
    match w {
        crate::WarningKind::HtmlParseError { message } => {
            assert_eq!(message, "unexpected end tag");
        }
        _ => panic!("expected HtmlParseError"),
    }
}

// ── PageBox populate ────────────────────────────────

#[test]
fn page_box_a4_has_expected_dimensions() {
    let a4 = PageBox::A4;
    assert_eq!(a4.width, 793.7008);
    assert_eq!(a4.height, 1122.5197);
}

#[test]
fn page_box_default_is_a4() {
    let default = PageBox::default();
    assert_eq!(default, PageBox::A4);
}

#[test]
fn page_box_external_constructable_via_struct_update_from_a4() {
    // Externally constructible pattern for a `#[non_exhaustive]` pub struct.
    // Change only the width using `..PageBox::A4` as a base.
    let landscape_a4 = PageBox {
        width: 1122.5197,
        height: 793.7008,
        ..PageBox::A4
    };
    assert_eq!(landscape_a4.width, 1122.5197);
    assert_eq!(landscape_a4.height, 793.7008);
}

// ── StylesheetKind trait bounds ──────────────────

#[test]
fn stylesheet_kind_is_copy_send_eq() {
    use crate::StylesheetKind;

    fn assert_send<T: Send>() {}
    fn assert_copy<T: Copy>() {}
    assert_send::<StylesheetKind>();
    assert_copy::<StylesheetKind>();

    let ua = StylesheetKind::UserAgent;
    let ua2 = ua; // Copy
    assert_eq!(ua, ua2);
    assert_ne!(StylesheetKind::UserAgent, StylesheetKind::Author);
    assert_ne!(StylesheetKind::UserAgent, StylesheetKind::User);
    assert_ne!(StylesheetKind::User, StylesheetKind::Author);
}

#[test]
fn layout_config_media_defaults_and_builder() {
    use raikiri_style::{MediaContext, MediaType};
    assert_eq!(LayoutConfig::default().media_context, MediaContext::print());
    let media = MediaContext::with_viewport(MediaType::Print, 261, 161);
    let config = LayoutConfig::builder().media_context(media).build();
    assert_eq!(config.media_context, media);
    assert_eq!(config.media_context.viewport_width(), 261);
    assert_eq!(config.media_context.viewport_height(), 161);
}

#[test]
fn observer_error_preserves_source() {
    use std::error::Error;
    let error = RenderError::Observer(std::io::Error::other("observer failed"));
    assert_eq!(error.source().unwrap().to_string(), "observer failed");
    assert!(error.to_string().contains("Observer"));
}
