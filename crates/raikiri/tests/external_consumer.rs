//! External consumers can parse and inspect document layout through umbrella exports.

use raikiri::*;

#[test]
fn external_consumer_can_reference_all_reexported_types() {
    // Compile-check that every type resolves with only use raikiri::*;.
    // Consume the values with let _ to avoid dead_code warnings.
    let _ = std::marker::PhantomData::<(
        RenderSummary,
        LimitKind,
        DocumentPlan,
        PageSummary,
        PageDefaults,
        PageDefaultsBuilder,
        PageBox,
        PageContext,
        LayoutConfig,
        LayoutConfigBuilder,
        BatchConfig,
        BatchConfigBuilder,
        LookaheadConfig,
        LookaheadConfigBuilder,
        RenderLimits,
        RenderLimitsBuilder,
        LayoutError,
        Symbol,
        CascadeLimits,
        CascadeOptions,
        CascadeLimitKind,
    )>;
}

/// Parse and inspect a completed layout through umbrella exports.
#[test]
fn external_consumer_can_call_parse_and_layout() {
    let doc = parse_html(
        b"<p>Hi</p>".as_slice(),
        &ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .unwrap() else {
        panic!("expected completed layout")
    };
    assert_eq!(result.page_count(), 1);
    assert!(result.page(0).unwrap().fragments().next().is_some());
}

/// Check that every `#[non_exhaustive]` struct can be constructed from an
/// external crate through X::default() or a builder (design test #10).
///
/// This covers all public `#[non_exhaustive]` structs re-exported by `raikiri`
/// with `impl Default`, the same set covered by the new()-check below.
///
/// Initially, `raikiri_style::property::Border` was excluded because it had
/// neither `Default` nor `new()` nor any public constructor; E0639 also
/// forbade struct literals, leaving no way to obtain a `Border` value from
/// raikiri. After adding `impl Default for Border` and `Border::new()`, this
/// test includes it in the coverage below.
#[test]
fn external_consumer_can_construct_all_non_exhaustive_types() {
    // struct via Default — configs

    let _ = LayoutConfig::default();
    let _ = BatchConfig::default();
    let _ = LookaheadConfig::default();
    let _ = RenderLimits::default();

    // struct via Default — paged model
    let _ = PageDefaults::default();
    let _ = PageBox::default();
    let _ = PageContext::default();

    let _ = LayoutBuffer::default();
    let _ = TargetRegistry::default();
    let _ = RunningTemplate::default();
    let _ = FormData::default();

    // Construct a raikiri-style value via Default: CSS Backgrounds 3 initial
    // values are width=medium(3px), style=none, and color=currentcolor.
    let _ = Border::default();

    // Construct plan-mode, resolver, and strategy placeholder structs via
    // Default; the external contract exists before their fields are populated.
    let _ = TargetDefinition::default();
    let _ = IntrinsicBox::default();
    let _ = ProbeContext::default();
    let _ = TargetRequest::default();

    // struct via builder
    let _ = PageDefaults::builder().build();

    let _ = LayoutConfig::builder().build();
    let _ = BatchConfig::builder().build();
    let _ = LookaheadConfig::builder().build();
    let _ = RenderLimits::builder().build();

    // Construct HtmlDocument via parse_html: its private field prevents direct
    // construction, even if it later re-exports raikiri_dom::Document.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let _doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
}

/// Compile and run the read-only `RuleTree` accessor chain
/// (`style_rules()` → `declarations()` → `value()`) from an external crate.
/// This pins the chosen visibility boundary.
///
/// This test checks only that the **read path is accessible**. Compilable
/// code cannot prove that a write path does not exist. Separate compile-fail
/// doctests check that restriction: three fences in the
/// `raikiri_style::rule::Declaration` struct docs (literal, struct update,
/// and assignment after clone), one in the
/// `raikiri_style::rule::StyleRule::declarations` docs, and one in the
/// `raikiri_style::ruletree::RuleTree::style_rules` docs.
/// `Declaration` and `StyleRule` are not re-exported by umbrella `raikiri`,
/// so their doctests belong to raikiri-style, whose `pub(crate)` boundary
/// they enforce.
#[test]
fn external_consumer_reads_rule_tree_through_readonly_accessors() {
    let mut tree = RuleTree::empty();
    assert!(tree.style_rules().is_empty(), "empty() は 0 rule");

    tree.add_stylesheet("p { color: red; }", Origin::Author);

    let rules = tree.style_rules();
    assert_eq!(rules.len(), 1, "type selector 1 rule が入る");

    let decls = rules[0].declarations();
    assert_eq!(decls.len(), 1);
    assert!(
        matches!(decls[0].value(), PropertyValue::Color(_)),
        "accessor 経由で読めた value は color 宣言"
    );
}

/// Regression check for PageBox switching to px units (design test #11).
#[test]
fn pagedefaults_us_letter_and_a4_have_expected_px_values() {
    assert!(
        (PageBox::A4.width - 793.7008).abs() < 0.001,
        "A4.width = 793.7008 px"
    );
    assert!(
        (PageBox::A4.height - 1122.5197).abs() < 0.001,
        "A4.height = 1122.5197 px"
    );
    assert_eq!(PageBox::US_LETTER.width, 816.0);
    assert_eq!(PageBox::US_LETTER.height, 1056.0);

    // Also verify that PageDefaults uses A4 as the default paper.
    assert_eq!(PageDefaults::default().page_box, PageBox::A4);
}

// ─────────────────────────────────────────────────────────────────────────
// public-api-compile-tests-lookaheadconfig (round 3 review #2)
//
// Check the design spec §L681-715 "struct construction patterns" acceptance
// criteria from an external consumer. Because `#[non_exhaustive]` prevents
// struct literals outside the crate, consumers need one of three patterns:
//
//   1. `X::default()` / `X::new()`  (zero-arg constructor)
//   2. `let mut c = X::default(); c.field = v;`  (mutation pattern)
//   3. `X::builder().field_a(v1).field_b(v2).build()`  (fluent builder)
//
// These tests pin **compile-time contracts**, not runtime behavior. If
// `LookaheadConfig::widow_line_buffer` becomes pub(crate), or `PageBox::new()`
// disappears, this test fails to compile and CI catches the consumer API break.
// ─────────────────────────────────────────────────────────────────────────

/// Compile-check the other half of pattern 1: zero-argument `X::new()`.
/// (The existing test covers default().) Cover every re-exported public
/// `#[non_exhaustive]` struct that provides zero-argument `new()` (§L703-704).
///
/// Include future-facing placeholder structs (LayoutBuffer / TargetRegistry /
/// RunningTemplate / FormData / TargetDefinition / IntrinsicBox / ResolverRequest
/// / ProbeContext / TargetRequest). This pins their current constructor
/// signatures as they gain fields without breaking consumers.
///
/// Deliberate exclusions:
/// - `HtmlDocument` is opaque with private fields; use `parse_html`.
/// - `ResolvedIntrinsic` is not `#[non_exhaustive]`, so external consumers
///   may construct it with a struct literal.
///
/// `raikiri_style::property::Border` originally lacked both `new()` and
/// `Default`, and E0639 forbade struct literals. After adding `Border::new()`
/// (= `Self::default()`), the coverage below includes it.
#[test]
fn external_consumer_can_use_new_constructor_on_all_types() {
    // paged model (raikiri-traits::page)
    let _ = PageDefaults::new();
    let _ = PageBox::new();
    let _ = PageContext::new();

    let _ = LayoutBuffer::new();
    let _ = TargetRegistry::new();
    let _ = RunningTemplate::new();
    let _ = FormData::new();

    // render entry point configs (raikiri-traits::config)

    let _ = LayoutConfig::new();
    let _ = BatchConfig::new();
    let _ = LookaheadConfig::new();
    let _ = RenderLimits::new();

    // raikiri-style value type.
    let _ = Border::new();

    // plan-mode types (raikiri-traits::plan)
    let _ = TargetDefinition::new();

    // Resolver types (raikiri-traits::resolver): return-type inference binds
    // ResolverRequest<'a>'s lifetime to the local stack frame.
    let _ = IntrinsicBox::new(1.0, 1.0);
    let url = url::Url::parse("https://example.com/image.png").unwrap();
    let _ = ResolverRequest::new(&url);

    // Strategy types (raikiri-traits::strategy): TargetRequest has the same lifetime behavior.
    let _ = ProbeContext::new();
    let _ = TargetRequest::new();
}

/// Compile-check pattern 2, the mutation example from §L705-706.
///
/// Public fields must remain assignable from outside the crate despite
/// `#[non_exhaustive]`. Assigning `c.field = value` on the public fields of
/// `LookaheadConfig`, `RenderLimits`, `PageDefaults`, `PageBox`, `PlanConfig`,
/// `StreamingConfig`, `BatchConfig`, and `Border` checks the consumer's
/// runtime-tuning path.
#[test]
fn external_consumer_can_mutate_pub_fields_via_default_shorthand() {
    // The canonical mutation example from spec §L705-706.
    let mut lookahead = LookaheadConfig::default();
    lookahead.widow_line_buffer = 5;
    lookahead.orphan_line_buffer = 3;
    lookahead.break_avoid_max_subtree_blocks = 40;
    lookahead.max_container_probe_pages = Some(8);
    lookahead.allow_cross_size_lookahead = true;

    let mut limits = RenderLimits::default();
    limits.max_document_pages = Some(50_000);
    limits.max_dom_nodes = Some(5_000_000);
    limits.max_target_slots = Some(200_000);
    limits.max_layout_buffer_entries = Some(20_000);
    limits.max_aggregate_bytes = Some(4 * 1_073_741_824);
    limits.max_input_bytes = Some(64 * 1024 * 1024);
    limits.max_parse_warnings = Some(512);
    limits.max_cascade_candidates_per_element = Some(4096);
    limits.max_cascade_declarations = Some(1 << 20);
    limits.max_cascade_selector_tests = None;
    limits.max_cascade_retained_bytes = Some(16 * 1024 * 1024);
    limits.max_cascade_output_bytes = Some(4 * 1_073_741_824);

    // The cascade limits are configured the same way.
    let mut cascade = CascadeOptions::default();
    cascade.limits = limits.cascade_limits();
    cascade.limits.max_output_bytes = Some(1 << 30);
    let _ = CascadeLimits::default();

    let mut page_box = PageBox::default();
    page_box.width = 500.0;
    page_box.height = 700.0;

    let mut page_defaults = PageDefaults::default();
    page_defaults.page_box = PageBox::US_LETTER;

    // Assign directly to all three raikiri-style value fields: width,
    // style, and color. This also checks typed construction through the
    // `BorderStyle` and `BorderColor` umbrella re-exports added alongside them.
    let mut border = Border::default();
    border.width = Length::Px(2.0);
    border.style = BorderStyle::Dashed;
    border.color = BorderColor::Resolved(CssColor {
        r: 0,
        g: 128,
        b: 255,
        a: 255,
    });

    // Nested config (§4 struct list): also pin swapping the inner structs.
    // For `initial_registry`, assign `Some(TargetRegistry::new())` to check the
    // inner type of `Option<TargetRegistry>`; assigning only `None` would not
    // detect a silent change to the Option's type parameter.

    let mut stream_cfg = LayoutConfig::default();
    stream_cfg.lookahead = lookahead.clone();
    stream_cfg.limits = limits.clone();
    stream_cfg.initial_registry = Some(TargetRegistry::new());

    // `BatchConfig` fixes lookahead to unbounded, so it has no `lookahead`
    // field. Check only limits and initial_registry.
    let mut batch_cfg = BatchConfig::default();
    batch_cfg.limits = limits;
    batch_cfg.initial_registry = Some(TargetRegistry::new());

    // Consume the value so the compiler sees a real read, not a dead store.
    let _ = (
        stream_cfg,
        batch_cfg,
        page_defaults,
        page_box,
        lookahead,
        border,
        cascade,
    );
}

/// Compile-check pattern 3, the fluent builder chain from §L707-708.
/// Each setter must return `Self`, allowing `.a(...).b(...).c(...)`.
/// A future change to `&mut Self` would fail to compile due to a
/// borrow/move mismatch.
#[test]
fn external_consumer_can_chain_builder_fluent_setters() {
    // Use LookaheadConfig from the design task title for a full-field chain.
    let lookahead = LookaheadConfig::builder()
        .widow_line_buffer(4)
        .orphan_line_buffer(2)
        .break_avoid_max_subtree_blocks(30)
        .max_container_probe_pages(Some(6))
        .allow_cross_size_lookahead(true)
        .build();
    assert_eq!(lookahead.widow_line_buffer, 4);
    assert_eq!(lookahead.orphan_line_buffer, 2);
    assert!(lookahead.allow_cross_size_lookahead);

    // Chain all twelve RenderLimitsBuilder setters, pinning their `Self`
    // return types.
    let limits = RenderLimits::builder()
        .max_document_pages(Some(100))
        .max_dom_nodes(Some(2_000_000))
        .max_target_slots(Some(50_000))
        .max_layout_buffer_entries(Some(5_000))
        .max_aggregate_bytes(Some(512 * 1_024 * 1_024))
        .max_input_bytes(Some(16 * 1_024 * 1_024))
        .max_parse_warnings(Some(256))
        .max_cascade_candidates_per_element(Some(1_024))
        .max_cascade_declarations(Some(1 << 24))
        .max_cascade_selector_tests(Some(1 << 26))
        .max_cascade_retained_bytes(None)
        .max_cascade_output_bytes(Some(2 * 1_073_741_824))
        .build();
    assert_eq!(limits.max_document_pages, Some(100));
    assert_eq!(limits.max_target_slots, Some(50_000));
    assert_eq!(limits.max_input_bytes, Some(16 * 1_024 * 1_024));
    assert_eq!(limits.max_parse_warnings, Some(256));
    assert_eq!(limits.max_cascade_candidates_per_element, Some(1_024));
    assert_eq!(limits.max_cascade_declarations, Some(1 << 24));
    assert_eq!(limits.max_cascade_selector_tests, Some(1 << 26));
    assert_eq!(limits.max_cascade_retained_bytes, None);
    assert_eq!(limits.max_cascade_output_bytes, Some(2 * 1_073_741_824));

    // Cross-struct integration: also pin the fluent chains that insert
    // LookaheadConfig into LayoutConfig / BatchConfig. Pass `Some(...)` to
    // `initial_registry` to check the inner type of `Option<TargetRegistry>`;
    // `None` alone cannot detect a silent change to the type parameter.
    // Chain all four LayoutConfigBuilder setters: lookahead, limits,
    // initial_registry, and signal.
    let abort_controller = AbortController::new();
    let _stream = LayoutConfig::builder()
        .lookahead(lookahead.clone())
        .limits(limits.clone())
        .initial_registry(Some(TargetRegistry::new()))
        .signal(Some(abort_controller.signal.clone()))
        .build();
    // BatchConfig has no lookahead; chain only limits and initial_registry.
    let _batch = BatchConfig::builder()
        .limits(limits)
        .initial_registry(Some(TargetRegistry::new()))
        .build();
    let _ = lookahead;

    let _defaults = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// A consumer builds a font set from its own font bytes and renders with
/// it, using only `use raikiri::*;`.
#[test]
fn external_consumer_renders_with_its_own_fonts() {
    // The font types are nameable through the umbrella.
    let _ = std::marker::PhantomData::<(FontCollection, FontCollectionBuildError)>;
    let fonts: RenderFonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM)
        .build()
        .expect("one font is enough");
    let html = br#"<html><body style="margin:0"><div style="font-family:Ahem;font-size:50px;line-height:50px">X</div></body></html>"#;
    let png = html_to_png_with_render_fonts(&html[..], fonts).expect("render");
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().expect("png header");
    let mut buffer = vec![0; reader.output_buffer_size().expect("buffer size")];
    let info = reader.next_frame(&mut buffer).expect("png frame");
    // Ahem's "X" is a black 1em square: the middle of its 50px box is ink.
    let pixel = ((25 * info.width + 25) * 4) as usize;
    assert_eq!(&buffer[pixel..pixel + 3], &[0, 0, 0]);
}

/// The render font set (`FontCollectionBuilder`, `RenderFonts`,
/// `html_to_png_with_render_fonts`, `RenderResources::fonts`) is nameable
/// with only a `raikiri` dependency.
#[test]
fn render_fonts_are_reachable_from_the_facade() {
    let _ = std::marker::PhantomData::<RenderFonts>;
    let built: Result<RenderFonts, FontCollectionBuildError> = FontCollectionBuilder::new().build();
    assert!(matches!(built, Err(FontCollectionBuildError::NoFonts)));
    type Input = std::io::Cursor<&'static [u8]>;
    type RenderWithFonts = fn(Input, RenderFonts) -> Result<Vec<u8>, RenderError>;
    let _: RenderWithFonts = html_to_png_with_render_fonts::<Input>;
    let _: fn(RenderResources<'static>, RenderFonts) -> RenderResources<'static> =
        RenderResources::fonts;
    assert!(!RenderResources::new().inline_engine_parallel_build());
}
