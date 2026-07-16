//! External consumer が `use raikiri::*;` のみで parse_html → plan (Err) →
//! render_streaming (Err) の chain を書けることを compile + run で pin
//! (raikiri-spike-m1.15 前哨、raikiri-spike-m1.11 で追加)。

use raikiri::*;

#[test]
fn external_consumer_can_reference_all_reexported_types() {
    // 各型が use raikiri::*; だけで名前解決できることを compile で pin。
    // 実際に値を使う必要なし (dead_code lint 抑制のため let _ で消費)。
    let _ = std::marker::PhantomData::<(
        RenderStatus,
        RenderSummary,
        LimitKind,
        DocumentPlan,
        PageSummary,
        PageDefaults,
        PageDefaultsBuilder,
        PageBox,
        PageContext,
        PageFragment,
        PlanConfig,
        PlanConfigBuilder,
        StreamingConfig,
        StreamingConfigBuilder,
        BatchConfig,
        BatchConfigBuilder,
        LookaheadConfig,
        LookaheadConfigBuilder,
        RenderLimits,
        RenderLimitsBuilder,
        LayoutError,
        Symbol,
    )>;
}

/// External consumer が `use raikiri::*;` のみで parse_html → plan (Err) →
/// render_streaming (Err) の chain を書けることを compile + run で pin。
/// (design test #9)
#[test]
fn external_consumer_can_call_parse_plan_render_streaming() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");

    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            unreachable!()
        }
    }
    struct NoopSink;
    impl RenderSink for NoopSink {
        fn accept_page(&mut self, _f: PageFragment) -> Result<(), std::io::Error> {
            unreachable!()
        }
        fn finish_render(&mut self, _s: RenderSummary) -> Result<(), std::io::Error> {
            unreachable!()
        }
    }

    let plan_err = plan(
        &doc,
        PageDefaults::default(),
        &NoopResolver,
        PlanConfig::default(),
    )
    .expect_err("plan stub must Err");
    assert!(matches!(
        plan_err,
        RenderError::Unimplemented {
            feature: "plan",
            ..
        }
    ));

    let mut sink = NoopSink;
    let stream_err = render_streaming(
        &doc,
        PageDefaults::default(),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect_err("render_streaming stub must Err");
    assert!(matches!(
        stream_err,
        RenderError::Unimplemented {
            feature: "render_streaming",
            ..
        }
    ));
}

/// 全 `#[non_exhaustive]` struct が external crate から X::default() / builder
/// で constructable なことを pin (m1.15 acceptance criteria 前哨、design test #10)。
#[test]
fn external_consumer_can_construct_all_non_exhaustive_types() {
    // struct via Default
    let _ = PageDefaults::default();
    let _ = PageBox::default();
    let _ = PageContext::default();
    let _ = PageFragment::default();
    let _ = PlanConfig::default();
    let _ = StreamingConfig::default();
    let _ = BatchConfig::default();
    let _ = LookaheadConfig::default();
    let _ = RenderLimits::default();

    // struct via builder
    let _ = PageDefaults::builder().build();
    let _ = PlanConfig::builder().build();
    let _ = StreamingConfig::builder().build();
    let _ = BatchConfig::builder().build();
    let _ = LookaheadConfig::builder().build();
    let _ = RenderLimits::builder().build();

    // HtmlDocument は parse_html を経由 (private field なので direct construct 不可、
    // これが M2+ で pub use raikiri_dom::Document に置換した際にも同じ制約)
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let _doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
}

/// PageBox の px 単位切替の regression pin (design test #11)。
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

    // PageDefaults の default paper が A4 であることも pin
    assert_eq!(PageDefaults::default().page_box, PageBox::A4);
}
