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
/// で constructable なことを pin (m1.15 acceptance criteria、design test #10)。
///
/// 対象は umbrella `raikiri` から re-export される全 `#[non_exhaustive]` pub
/// struct with `impl Default` (下記 new()-pin test と同じ coverage set)。
#[test]
fn external_consumer_can_construct_all_non_exhaustive_types() {
    // struct via Default — configs
    let _ = PlanConfig::default();
    let _ = StreamingConfig::default();
    let _ = BatchConfig::default();
    let _ = LookaheadConfig::default();
    let _ = RenderLimits::default();

    // struct via Default — paged model
    let _ = PageDefaults::default();
    let _ = PageBox::default();
    let _ = PageContext::default();
    let _ = PageFragment::default();
    let _ = LayoutBuffer::default();
    let _ = TargetRegistry::default();
    let _ = RunningTemplate::default();
    let _ = FormData::default();

    // struct via Default — plan mode / resolver / strategy placeholder shape。
    // これらは M4+ で populate 予定だが Default 契約は今から crate 外に露出。
    let _ = TargetDefinition::default();
    let _ = IntrinsicBox::default();
    let _ = ResolverRequest::default();
    let _ = ProbeContext::default();
    let _ = TargetRequest::default();

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

/// `RuleTree` の read-only accessor chain (`style_rules()` → `declarations()`
/// → `value()`) が external crate から通ることを compile + run で pin
/// (bd raikiri-spike-qzn3、PMO 判断 (B) 可視性を絞る)。
///
/// ⚠️ 本 test が pin するのは **read 経路が届くこと**だけである。「write 経路が
/// 無い」ことは compile する code では表現できないので pin されていない —
/// そちらは compile-fail doctest (bd raikiri-spike-ejia) が別途 pin する:
/// `raikiri_style::rule::Declaration` の struct doc (struct literal /
/// functional-update / clone 後の field 代入、計 3 fence)、
/// `raikiri_style::rule::StyleRule::declarations` の doc (1 fence)、
/// `raikiri_style::ruletree::RuleTree::style_rules` の doc (1 fence)。
/// `Declaration` / `StyleRule` は umbrella `raikiri` から re-export されて
/// いないため、この pin は umbrella 経由ではなく raikiri-style 自身の
/// doctest として存在する (pub(crate) の境界は raikiri-style crate に対して
/// 定義されるものなので、それが自然な置き場所)。
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

// ─────────────────────────────────────────────────────────────────────────
// m1.15: public-api-compile-tests-lookaheadconfig (round 3 review #2 対応)
//
// 設計仕様書 §L681-715 "struct construction pattern" (§M1 Acceptance criteria)
// を external consumer 側で pin する。`#[non_exhaustive]` は crate 外での
// literal construction を封じるため、Consumer は必ず以下 3 pattern のいずれか
// を使わなければならない:
//
//   1. `X::default()` / `X::new()`  (zero-arg constructor)
//   2. `let mut c = X::default(); c.field = v;`  (mutation pattern)
//   3. `X::builder().field_a(v1).field_b(v2).build()`  (fluent builder)
//
// これらの test は runtime 挙動ではなく **compile 契約** を pin する。
// もし将来 `LookaheadConfig::widow_line_buffer` が pub → pub(crate) に落ちる
// / `PageBox::new()` が削除される 等の regression が起きれば、この test は
// compile error になる (= external consumer の API break が CI で捕捉される)。
// ─────────────────────────────────────────────────────────────────────────

/// Pattern 1 の残り半分 (default() は既存 test で pin 済み) — `X::new()` 存在の
/// compile pin。umbrella `raikiri` から re-export される全 `#[non_exhaustive]`
/// pub struct のうち zero-arg `new()` を持つもの全てを対象とする (§L703-704)。
///
/// M4+ で populate 予定の placeholder struct (LayoutBuffer / TargetRegistry /
/// RunningTemplate / FormData / TargetDefinition / IntrinsicBox / ResolverRequest
/// / ProbeContext / TargetRequest) も現時点の zero-arg constructor を pin する
/// ため含める — future populate 時に `new()` signature が非破壊拡張のまま維持
/// されていることを保証する。
///
/// 対象外 (意図的):
/// - `HtmlDocument` は private field で opaque、`parse_html` 経由でのみ construct
/// - `ResolvedIntrinsic` は `#[non_exhaustive]` でないため construction 契約が
///   `struct literal` 経由で crate 外から直接可能、この test の対象外
#[test]
fn external_consumer_can_use_new_constructor_on_all_types() {
    // paged model (raikiri-traits::page)
    let _ = PageDefaults::new();
    let _ = PageBox::new();
    let _ = PageContext::new();
    let _ = PageFragment::new();
    let _ = LayoutBuffer::new();
    let _ = TargetRegistry::new();
    let _ = RunningTemplate::new();
    let _ = FormData::new();

    // render entry point configs (raikiri-traits::config)
    let _ = PlanConfig::new();
    let _ = StreamingConfig::new();
    let _ = BatchConfig::new();
    let _ = LookaheadConfig::new();
    let _ = RenderLimits::new();

    // plan-mode types (raikiri-traits::plan)
    let _ = TargetDefinition::new();

    // resolver types (raikiri-traits::resolver) — ResolverRequest<'a> の 'a は
    // return-type inference で local frame lifetime に落ちる。
    let _ = IntrinsicBox::new();
    let _ = ResolverRequest::new();

    // strategy types (raikiri-traits::strategy) — TargetRequest も lifetime 同上。
    let _ = ProbeContext::new();
    let _ = TargetRequest::new();
}

/// Pattern 2 — mutation pattern (§L705-706 の canonical example) の compile pin。
///
/// `#[non_exhaustive]` 下でも pub field は crate 外から代入可能な状態を保つ
/// 必要がある。この test は `LookaheadConfig`, `RenderLimits`, `PageDefaults`,
/// `PageBox`, `PlanConfig`, `StreamingConfig`, `BatchConfig` の各 pub field
/// に対し `c.field = value` が compile することで、Consumer の runtime tuning
/// 経路を pin する。
#[test]
fn external_consumer_can_mutate_pub_fields_via_default_shorthand() {
    // spec §L705-706 の canonical mutation example そのまま。
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
    // bd raikiri-spike-4kw: Sprint 10 で追加された 6 番目の pub field。
    limits.max_input_bytes = Some(64 * 1024 * 1024);

    let mut page_box = PageBox::default();
    page_box.width = 500.0;
    page_box.height = 700.0;

    let mut page_defaults = PageDefaults::default();
    page_defaults.page_box = PageBox::US_LETTER;

    // Nested config (§4 "対象 struct" list) — inner struct の swap も pin。
    // `initial_registry` は明示的に `Option<TargetRegistry>` に対する
    // `Some(TargetRegistry::new())` で inner type も pin する (単に `None` を
    // 代入するだけでは Option の T が別 type に silently 変わっても検出できない)。
    let mut plan_cfg = PlanConfig::default();
    plan_cfg.lookahead = lookahead.clone();
    plan_cfg.limits = limits.clone();
    plan_cfg.initial_registry = Some(TargetRegistry::new());

    let mut stream_cfg = StreamingConfig::default();
    stream_cfg.lookahead = lookahead.clone();
    stream_cfg.limits = limits.clone();
    stream_cfg.initial_registry = Some(TargetRegistry::new());

    // `BatchConfig` は preset 上 unbounded lookahead 固定 (design §M2 / M6b) の
    // ため `lookahead` field を持たない。limits + initial_registry のみ pin。
    let mut batch_cfg = BatchConfig::default();
    batch_cfg.limits = limits;
    batch_cfg.initial_registry = Some(TargetRegistry::new());

    // consume so compiler は dead_store でなく actual read として扱う。
    let _ = (
        plan_cfg,
        stream_cfg,
        batch_cfg,
        page_defaults,
        page_box,
        lookahead,
    );
}

/// Pattern 3 — builder fluent chain (§L707-708) の compile pin。全 setter が
/// `Self` を返すこと (= `.a(...).b(...).c(...)` が chain 可能) を保証する。
/// もし将来誰かが `&mut Self` に変更すれば borrow-move mismatch で compile
/// error になる。
#[test]
fn external_consumer_can_chain_builder_fluent_setters() {
    // Design task title の LookaheadConfig を代表例として full-field chain。
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

    // RenderLimitsBuilder は全 6 setter を chain (全 method が `Self` を返す
    // regression pin)。bd raikiri-spike-4kw で `max_input_bytes` が 6 番目に追加。
    let limits = RenderLimits::builder()
        .max_document_pages(Some(100))
        .max_dom_nodes(Some(2_000_000))
        .max_target_slots(Some(50_000))
        .max_layout_buffer_entries(Some(5_000))
        .max_aggregate_bytes(Some(512 * 1_024 * 1_024))
        .max_input_bytes(Some(16 * 1_024 * 1_024))
        .build();
    assert_eq!(limits.max_document_pages, Some(100));
    assert_eq!(limits.max_target_slots, Some(50_000));
    assert_eq!(limits.max_input_bytes, Some(16 * 1_024 * 1_024));

    // Cross-struct wiring: LookaheadConfig を PlanConfig / StreamingConfig /
    // BatchConfig に差し込む fluent chain も pin。`initial_registry` は
    // `Option<TargetRegistry>` の inner type も pin するため `Some(...)` 経路を
    // 使う (`None` だけでは inner の T が silently 変わっても検出できない)。
    let _plan = PlanConfig::builder()
        .lookahead(lookahead.clone())
        .limits(limits.clone())
        .initial_registry(Some(TargetRegistry::new()))
        .build();
    // StreamingConfigBuilder は 3 setter (lookahead / limits / initial_registry)
    // 全てを chain 対象に含める。
    let _stream = StreamingConfig::builder()
        .lookahead(lookahead.clone())
        .limits(limits.clone())
        .initial_registry(Some(TargetRegistry::new()))
        .build();
    // BatchConfig は lookahead を持たないため limits + initial_registry chain のみ。
    let _batch = BatchConfig::builder()
        .limits(limits)
        .initial_registry(Some(TargetRegistry::new()))
        .build();
    let _ = lookahead;

    let _defaults = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
}

/// External consumer が `use raikiri::*;` のみで VRT font pin API
/// (`build_wpt_font_ctx`, `FontError`, `FontContext`, `html_to_png_with_fonts`)
/// を chain できることを compile + run で pin (raikiri-spike-e93 roborev
/// round 2 finding — raikiri-dom/parley を direct dep しなくて良い保証)。
#[test]
fn external_consumer_can_reference_vrt_font_pin_api() {
    // 1. 型は全て `use raikiri::*;` で resolve できる
    let _ = std::marker::PhantomData::<(FontContext, FontError)>;

    // 2. build_wpt_font_ctx を呼び出せる (missing dir で DirNotFound Err を expect)
    let bogus = std::path::Path::new("/definitely/does/not/exist/raikiri-spike-e93");
    let err = match build_wpt_font_ctx(bogus) {
        Err(e) => e,
        Ok(_) => panic!("expected Err from missing dir"),
    };
    // FontError variant は raikiri umbrella 経由で pattern match できる
    assert!(matches!(err, FontError::DirNotFound(_)));

    // 3. html_to_png_with_fonts は FontContext を受ける signature、
    //    system font FontContext (fallback) との組み合わせで compile pin
    //    (実 render は system font 経路、determinism 不要な smoke)
    let font_ctx = FontContext::new();
    let _ = html_to_png_with_fonts(&b"<p>x</p>"[..], font_ctx);
}
