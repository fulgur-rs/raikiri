//! Render entry point config (Finding #5 対応: raikiri 内 iteration 廃止)。
//!
//! `plan()` / `render_streaming()` / `render_batch()` の 3 entry point が
//! それぞれ config を受け取り、resource / cost limit を強制。round 4 review #1
//! 対応で `RenderLimits` に昇格 (旧 BatchConfig 限定 から plan / Streaming にも
//! 統一)。

use crate::page::TargetRegistry;

/// 全 entry point (plan / render_streaming / render_batch) が受け取る
/// resource / cost 上限 (Finding #5 + round 4 review #1)。
///
/// 妥当な defaults は fulgur 想定: pages=10_000, nodes=1M, slots=100k,
/// buffer=10k, bytes=1GB (§4 参照)。
///
/// # `max_input_bytes` promotion
///
/// `max_input_bytes` field は、SEC-HIGH `parse_html` unbounded-read DoS を
/// close するために導入された hard-coded 32 MiB input cap を
/// `RenderLimits` 上の configurable field へ昇格させたもの。
///
/// **Migration**: 旧 stopgap を利用していた consumer (`parse_html`
/// / `parse_html_with_limits` を呼ぶ側) は、`RenderLimits::default()` を渡す
/// 限り behavior 不変 (default `Some(32 * 1024 * 1024)` は元の hard-coded
/// 値と一致)。cap を調整したい場合は [`RenderLimitsBuilder::max_input_bytes`]
/// (または field への直接代入)、無効化したい場合は `None` を設定する
/// (**cap 無効化の security 上の含意は `max_input_bytes` field doc を参照**)。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct RenderLimits {
    /// 超過 → `LimitExceeded { kind: Pages }`。
    pub max_document_pages: Option<u32>,
    /// parse 完了後 check。
    pub max_dom_nodes: Option<u64>,
    /// per-doc target 参照数上限。
    pub max_target_slots: Option<u32>,
    /// LayoutBuffer に貯める上限。
    pub max_layout_buffer_entries: Option<u32>,
    /// approximate memory footprint 上限。
    pub max_aggregate_bytes: Option<u64>,
    /// parse 前に読み込む raw input byte 数上限。超過 →
    /// `LimitExceeded { kind: InputBytes }`。
    ///
    /// Default は `Some(32 * 1024 * 1024)` (32 MiB)、旧 stopgap の
    /// hard-coded 値を継承。
    ///
    /// **Security**: `None` は cap を無効化し、SEC-HIGH の `parse_html`
    /// unbounded-read DoS を **再暴露する** (attacker が任意
    /// サイズの HTML を送り込み OOM を誘発可能)。明示的な opt-out としてのみ
    /// 使用し、default (`Some(32 MiB)`) から離れる場合は upstream で別途
    /// bound を設ける前提であること。
    ///
    /// **Semantic**: [`max_aggregate_bytes`](Self::max_aggregate_bytes) は
    /// post-parse の approximate memory footprint (DOM node arena / cascade
    /// table 等の合計) を check する一方、`max_input_bytes` は parse-time の
    /// raw byte stream を check する (parse 開始前に enforce できるので DoS
    /// 対策として直接的、fail-closed 早期返却)。
    pub max_input_bytes: Option<u64>,
    /// HTML parse 中に html5ever が報告する非致命 parse error を warning
    /// として記録する件数の上限。超過すると、以降の parse error は記録され
    /// なくなる代わりに「以降 suppress した」ことを示す synthetic な 1 件の
    /// warning が追加される (silent drop だと consumer が「warning が 1 件も
    /// 無かった」のか「cap に達して drop された」のか区別できないため)。
    ///
    /// Default は `Some(1024)`。
    ///
    /// **Security**: HTML5 のエラー回復アルゴリズムは、malformed な入力の
    /// 小さな token 1 個あたり概ね 1 個の parse error を報告しうる。cap が
    /// 無いと、attacker が任意個数の owned `String` を持つ warning を積ませて
    /// memory を線形に消費させられる (parse error 自体は tree construction を
    /// 止めない non-fatal な事象なので、上限が無いと入力サイズにほぼ比例した
    /// allocation が発生する)。`None` はこの cap を無効化するため、明示的な
    /// opt-out としてのみ使用し、無効化する場合は upstream で別途 bound を
    /// 設ける前提であること。
    ///
    /// **Semantic**: [`max_input_bytes`](Self::max_input_bytes) が生の入力
    /// byte 数 (線形) を check するのに対し、`max_parse_warnings` は同じ入力
    /// サイズでも malformed token の密度によって非線形に増幅しうる出力側の
    /// warning 件数を check する — 短い入力でも極端に高密度な malformed token
    /// 列を送り込めば大量の warning を生成できるため、入力 byte cap だけでは
    /// この増幅を防げない。
    pub max_parse_warnings: Option<usize>,
}

impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_document_pages: Some(10_000),
            max_dom_nodes: Some(1_000_000),
            max_target_slots: Some(100_000),
            max_layout_buffer_entries: Some(10_000),
            max_aggregate_bytes: Some(1_073_741_824), // 1 GB
            // 32 MiB — 元は raikiri-html の parse-time input read に対する
            // hard-coded cap だった値を継承 (behavior 不変)。
            max_input_bytes: Some(32 * 1024 * 1024),
            // 1024 — 元は raikiri-html の RaikiriTreeSink 内 hard-coded const
            // だった値を継承 (behavior 不変)。html5ever のエラー回復アルゴリズム
            // は malformed input の 1 token あたり概ね 1 個の parse error を
            // 報告しうるため、cap が無いと memory 消費が入力サイズにほぼ比例
            // して増加する (field doc の Security note 参照)。
            max_parse_warnings: Some(1024),
        }
    }
}

impl RenderLimits {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> RenderLimitsBuilder {
        RenderLimitsBuilder::default()
    }
}

/// `RenderLimits` の fluent builder。未設定 field は Default 値。
#[derive(Debug, Default, Clone)]
pub struct RenderLimitsBuilder {
    max_document_pages: Option<Option<u32>>,
    max_dom_nodes: Option<Option<u64>>,
    max_target_slots: Option<Option<u32>>,
    max_layout_buffer_entries: Option<Option<u32>>,
    max_aggregate_bytes: Option<Option<u64>>,
    max_input_bytes: Option<Option<u64>>,
    max_parse_warnings: Option<Option<usize>>,
}

impl RenderLimitsBuilder {
    /// `max_document_pages` を設定 (`None` = unbounded)。
    pub fn max_document_pages(mut self, v: Option<u32>) -> Self {
        self.max_document_pages = Some(v);
        self
    }

    /// `max_dom_nodes` を設定。
    pub fn max_dom_nodes(mut self, v: Option<u64>) -> Self {
        self.max_dom_nodes = Some(v);
        self
    }

    /// `max_target_slots` を設定。
    pub fn max_target_slots(mut self, v: Option<u32>) -> Self {
        self.max_target_slots = Some(v);
        self
    }

    /// `max_layout_buffer_entries` を設定。
    pub fn max_layout_buffer_entries(mut self, v: Option<u32>) -> Self {
        self.max_layout_buffer_entries = Some(v);
        self
    }

    /// `max_aggregate_bytes` を設定。
    pub fn max_aggregate_bytes(mut self, v: Option<u64>) -> Self {
        self.max_aggregate_bytes = Some(v);
        self
    }

    /// `max_input_bytes` を設定 (`None` = cap 無効化)。
    pub fn max_input_bytes(mut self, v: Option<u64>) -> Self {
        self.max_input_bytes = Some(v);
        self
    }

    /// `max_parse_warnings` を設定 (`None` = cap 無効化)。
    pub fn max_parse_warnings(mut self, v: Option<usize>) -> Self {
        self.max_parse_warnings = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> RenderLimits {
        let d = RenderLimits::default();
        RenderLimits {
            max_document_pages: self.max_document_pages.unwrap_or(d.max_document_pages),
            max_dom_nodes: self.max_dom_nodes.unwrap_or(d.max_dom_nodes),
            max_target_slots: self.max_target_slots.unwrap_or(d.max_target_slots),
            max_layout_buffer_entries: self
                .max_layout_buffer_entries
                .unwrap_or(d.max_layout_buffer_entries),
            max_aggregate_bytes: self.max_aggregate_bytes.unwrap_or(d.max_aggregate_bytes),
            max_input_bytes: self.max_input_bytes.unwrap_or(d.max_input_bytes),
            max_parse_warnings: self.max_parse_warnings.unwrap_or(d.max_parse_warnings),
        }
    }
}

/// LayoutBuffer の lookahead 幅 config。
///
/// 初期 seed value (blitz 慣習ベース、将来 refine 予定)。
///
/// spec §4 "[対象 struct]" list に含まれるため `#[non_exhaustive]` を付与
/// (round 3 Missing #6 対応)。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LookaheadConfig {
    /// widow 判定のため何行先を bufferするか。
    pub widow_line_buffer: usize,
    /// orphan 判定のため何行前を bufferするか。
    pub orphan_line_buffer: usize,
    /// `break-inside: avoid` subtree の最大 block 数。
    pub break_avoid_max_subtree_blocks: usize,
    /// flex / grid container の probe layout 上限 (`None` = unbounded)。
    pub max_container_probe_pages: Option<usize>,
    /// cross-size 方向の lookahead を許可するか。
    pub allow_cross_size_lookahead: bool,
}

impl Default for LookaheadConfig {
    fn default() -> Self {
        // Initial seed value, refined over time.
        Self {
            widow_line_buffer: 2,
            orphan_line_buffer: 2,
            break_avoid_max_subtree_blocks: 20,
            max_container_probe_pages: Some(4),
            allow_cross_size_lookahead: false,
        }
    }
}

impl LookaheadConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> LookaheadConfigBuilder {
        LookaheadConfigBuilder::default()
    }
}

/// `LookaheadConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct LookaheadConfigBuilder {
    widow_line_buffer: Option<usize>,
    orphan_line_buffer: Option<usize>,
    break_avoid_max_subtree_blocks: Option<usize>,
    max_container_probe_pages: Option<Option<usize>>,
    allow_cross_size_lookahead: Option<bool>,
}

impl LookaheadConfigBuilder {
    /// `widow_line_buffer` を設定。
    pub fn widow_line_buffer(mut self, v: usize) -> Self {
        self.widow_line_buffer = Some(v);
        self
    }

    /// `orphan_line_buffer` を設定。
    pub fn orphan_line_buffer(mut self, v: usize) -> Self {
        self.orphan_line_buffer = Some(v);
        self
    }

    /// `break_avoid_max_subtree_blocks` を設定。
    pub fn break_avoid_max_subtree_blocks(mut self, v: usize) -> Self {
        self.break_avoid_max_subtree_blocks = Some(v);
        self
    }

    /// `max_container_probe_pages` を設定 (`None` = unbounded)。
    pub fn max_container_probe_pages(mut self, v: Option<usize>) -> Self {
        self.max_container_probe_pages = Some(v);
        self
    }

    /// `allow_cross_size_lookahead` を設定。
    pub fn allow_cross_size_lookahead(mut self, v: bool) -> Self {
        self.allow_cross_size_lookahead = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> LookaheadConfig {
        let d = LookaheadConfig::default();
        LookaheadConfig {
            widow_line_buffer: self.widow_line_buffer.unwrap_or(d.widow_line_buffer),
            orphan_line_buffer: self.orphan_line_buffer.unwrap_or(d.orphan_line_buffer),
            break_avoid_max_subtree_blocks: self
                .break_avoid_max_subtree_blocks
                .unwrap_or(d.break_avoid_max_subtree_blocks),
            max_container_probe_pages: self
                .max_container_probe_pages
                .unwrap_or(d.max_container_probe_pages),
            allow_cross_size_lookahead: self
                .allow_cross_size_lookahead
                .unwrap_or(d.allow_cross_size_lookahead),
        }
    }
}

/// `plan()` 用 config (round 4 review #1, #2 対応)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct PlanConfig {
    /// lookahead 設定。
    pub lookahead: LookaheadConfig,
    /// resource / cost 上限 (round 4 review #1)。
    pub limits: RenderLimits,
    /// 反復 chain 用の hint registry (round 4 review #2)。
    pub initial_registry: Option<TargetRegistry>,
}

impl PlanConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PlanConfigBuilder {
        PlanConfigBuilder::default()
    }
}

/// `PlanConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct PlanConfigBuilder {
    lookahead: Option<LookaheadConfig>,
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl PlanConfigBuilder {
    /// `lookahead` を設定。
    pub fn lookahead(mut self, v: LookaheadConfig) -> Self {
        self.lookahead = Some(v);
        self
    }

    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PlanConfig {
        let d = PlanConfig::default();
        PlanConfig {
            lookahead: self.lookahead.unwrap_or(d.lookahead),
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}

/// `render_streaming()` 用 config (round 4 review #1 対応)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct StreamingConfig {
    /// lookahead 設定。
    pub lookahead: LookaheadConfig,
    /// resource / cost 上限。
    pub limits: RenderLimits,
    /// `plan` の結果を hint として渡す (round 4 review #2)。
    pub initial_registry: Option<TargetRegistry>,
    /// Optional cooperative cancellation signal checked before layout and
    /// before each page emission. An aborted render never calls
    /// `RenderSink::finish_render`.
    pub signal: Option<crate::AbortSignal>,
}

impl StreamingConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> StreamingConfigBuilder {
        StreamingConfigBuilder::default()
    }
}

/// `StreamingConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct StreamingConfigBuilder {
    lookahead: Option<LookaheadConfig>,
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
    signal: Option<Option<crate::AbortSignal>>,
}

impl StreamingConfigBuilder {
    /// `lookahead` を設定。
    pub fn lookahead(mut self, v: LookaheadConfig) -> Self {
        self.lookahead = Some(v);
        self
    }

    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Set a cooperative cancellation signal.
    pub fn signal(mut self, v: Option<crate::AbortSignal>) -> Self {
        self.signal = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> StreamingConfig {
        let d = StreamingConfig::default();
        StreamingConfig {
            lookahead: self.lookahead.unwrap_or(d.lookahead),
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
            signal: self.signal.unwrap_or(d.signal),
        }
    }
}

/// `render_batch()` 用 config (round 4 review #1 対応で `max_document_pages` を
/// `limits` に吸収)。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct BatchConfig {
    /// resource / cost 上限。
    pub limits: RenderLimits,
    /// `plan` の結果を hint として渡す。
    pub initial_registry: Option<TargetRegistry>,
}

impl BatchConfig {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> BatchConfigBuilder {
        BatchConfigBuilder::default()
    }
}

/// `BatchConfig` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct BatchConfigBuilder {
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl BatchConfigBuilder {
    /// `limits` を設定。
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// `initial_registry` を設定。
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> BatchConfig {
        let d = BatchConfig::default();
        BatchConfig {
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}
