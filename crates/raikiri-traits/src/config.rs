//! Render entry-point configuration (Finding #5: remove iteration inside raikiri).
//!
//! The three entry points, `plan()`, `render_streaming()`, and `render_batch()`,
//! each accept configuration and enforce resource and cost limits. Round 4 review #1
//! promoted these limits to `RenderLimits` (formerly limited to BatchConfig, now
//! shared by plan and Streaming).

use raikiri_style::{CascadeLimits, RuleTreeLimits};

use crate::page::TargetRegistry;

/// Resource and cost limits accepted by all entry points (plan /
/// render_streaming / render_batch; Finding #5 + round 4 review #1).
///
/// Defaults suitable for fulgur: pages=10_000, nodes=1M, slots=100k,
/// buffer=10k, bytes=1GB (see §4).
///
/// # `max_input_bytes` promotion
///
/// The `max_input_bytes` field replaces the hard-coded 32 MiB input cap
/// introduced to close the SEC-HIGH unbounded-read DoS in `parse_html`
/// with a configurable field on `RenderLimits`.
///
/// **Migration**: Consumers of the old stopgap (`parse_html` /
/// `parse_html_with_limits` callers) retain the same behavior when passing
/// `RenderLimits::default()` (its default `Some(32 * 1024 * 1024)` matches
/// the former hard-coded value). To adjust the cap, use [`RenderLimitsBuilder::max_input_bytes`]
/// (or assign the field directly); to disable it, set `None`
/// (**see the `max_input_bytes` field docs for the security implications**).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct RenderLimits {
    /// Exceeding this yields `LimitExceeded { kind: Pages }`.
    pub max_document_pages: Option<u32>,
    /// Checked after parsing completes.
    pub max_dom_nodes: Option<u64>,
    /// Maximum number of per-document target references.
    pub max_target_slots: Option<u32>,
    /// Maximum entries stored in LayoutBuffer.
    pub max_layout_buffer_entries: Option<u32>,
    /// Maximum approximate memory footprint. No stage consults it yet; the
    /// cascade's result has its own bound,
    /// [`Self::max_cascade_output_bytes`].
    pub max_aggregate_bytes: Option<u64>,
    /// Maximum raw input bytes read before parsing. Exceeding it yields
    /// `LimitExceeded { kind: InputBytes }`.
    ///
    /// Defaults to `Some(32 * 1024 * 1024)` (32 MiB), inherited from
    /// the previous hard-coded stopgap.
    ///
    /// **Security**: `None` disables the cap and **re-exposes** the SEC-HIGH
    /// unbounded-read DoS in `parse_html` (an attacker could send HTML of arbitrary
    /// size and trigger OOM). Use it only as an explicit opt-out;
    /// if you depart from the default (`Some(32 MiB)`), provide another
    /// upstream bound.
    ///
    /// **Semantic**: [`max_aggregate_bytes`](Self::max_aggregate_bytes) checks
    /// approximate post-parse memory usage (the DOM node arena, cascade
    /// table, etc.), whereas `max_input_bytes` checks the parse-time raw
    /// byte stream (enforceable before parsing for direct DoS protection
    /// and fail-closed early return).
    pub max_input_bytes: Option<u64>,
    /// Maximum number of non-fatal parse errors reported by html5ever during
    /// HTML parsing that are recorded as warnings. Once exceeded, subsequent
    /// parse errors are not recorded; instead, one synthetic warning indicates
    /// that later errors were suppressed (so consumers can distinguish
    /// no warnings from warnings dropped after the cap was reached).
    ///
    /// Defaults to `Some(1024)`.
    ///
    /// **Security**: HTML5 error recovery can report roughly one parse error
    /// per small malformed token. Without a cap, an attacker can accumulate
    /// arbitrarily many warnings with owned `String`s and consume memory
    /// linearly (parse errors do not stop tree construction, so allocations
    /// can scale approximately with input size). `None` disables this cap;
    /// use it only as an explicit opt-out and provide another upstream
    /// bound if disabling it.
    ///
    ///
    /// **Semantic**: [`max_input_bytes`](Self::max_input_bytes) checks the raw
    /// input byte count (linear), while `max_parse_warnings` checks output-side
    /// warning count, which can grow nonlinearly with the density of malformed
    /// tokens even for a fixed input size. A short but densely malformed input
    /// can create many warnings; the input-byte cap alone cannot prevent
    /// this amplification.
    pub max_parse_warnings: Option<usize>,
    /// The most candidate declarations one element may collect in the
    /// cascade; see
    /// [`CascadeLimits::max_candidates_per_element`](raikiri_style::CascadeLimits::max_candidates_per_element),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: CascadeCandidatesPerElement }`.
    pub max_cascade_candidates_per_element: Option<u32>,
    /// The most candidate declarations the cascade may collect over all
    /// elements; see
    /// [`CascadeLimits::max_declarations_visited`](raikiri_style::CascadeLimits::max_declarations_visited),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: CascadeDeclarations }`.
    pub max_cascade_declarations: Option<u64>,
    /// The most selector tests the cascade may run; see
    /// [`CascadeLimits::max_selector_tests`](raikiri_style::CascadeLimits::max_selector_tests),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: CascadeSelectorTests }`.
    pub max_cascade_selector_tests: Option<u64>,
    /// The most bytes of candidate declarations the cascade's result may keep;
    /// see
    /// [`CascadeLimits::max_retained_bytes`](raikiri_style::CascadeLimits::max_retained_bytes),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: CascadeRetainedBytes }`.
    pub max_cascade_retained_bytes: Option<u64>,
    /// The most bytes the cascade's result may hold: the computed values of
    /// every node and pseudo-element, and the candidates it keeps, counted
    /// before they are allocated, and the heap the values hold, counted once
    /// each node is resolved; see
    /// [`CascadeLimits::max_output_bytes`](raikiri_style::CascadeLimits::max_output_bytes),
    /// whose default this takes and which admits [`Self::max_dom_nodes`]
    /// nodes with two pseudo-element styles per element. Exceeding it yields
    /// `LimitExceeded { kind: CascadeOutputBytes }`.
    ///
    /// Each cascade run is bounded on its own. The heap the computed values
    /// hold of their own, such as the strings `var()` substitution builds,
    /// counts too; a value a node inherits counts once.
    pub max_cascade_output_bytes: Option<u64>,
    /// The most rules counted while the document's stylesheets are parsed
    /// into its rule tree; see
    /// [`RuleTreeLimits::max_rules`](raikiri_style::RuleTreeLimits::max_rules),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: StyleRules }`.
    pub max_style_rules: Option<u64>,
    /// The most selectors counted while the document's stylesheets are
    /// parsed; see
    /// [`RuleTreeLimits::max_selectors`](raikiri_style::RuleTreeLimits::max_selectors),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: StyleSelectors }`.
    pub max_style_selectors: Option<u64>,
    /// The most declarations counted while the document's stylesheets are
    /// parsed, after shorthand expansion; see
    /// [`RuleTreeLimits::max_declarations`](raikiri_style::RuleTreeLimits::max_declarations),
    /// whose default this takes. Exceeding it yields
    /// `LimitExceeded { kind: StyleDeclarations }`.
    pub max_style_declarations: Option<u64>,
}

impl Default for RenderLimits {
    fn default() -> Self {
        let cascade = CascadeLimits::default();
        let rule_tree = RuleTreeLimits::default();
        Self {
            max_document_pages: Some(10_000),
            max_dom_nodes: Some(1_000_000),
            max_target_slots: Some(100_000),
            max_layout_buffer_entries: Some(10_000),
            max_aggregate_bytes: Some(1_073_741_824), // 1 GB
            // 32 MiB — inherited from the former hard-coded cap on parse-time input
            // reads in raikiri-html (unchanged behavior).
            max_input_bytes: Some(32 * 1024 * 1024),
            // 1024 — inherited from the former hard-coded constant in raikiri-html's
            // RaikiriTreeSink (unchanged behavior). html5ever error recovery can
            // report roughly one parse error per malformed token; without a cap,
            // memory consumption could grow roughly in proportion to input size
            // (see the Security note in the field docs).
            max_parse_warnings: Some(1024),
            max_cascade_candidates_per_element: cascade.max_candidates_per_element,
            max_cascade_declarations: cascade.max_declarations_visited,
            max_cascade_selector_tests: cascade.max_selector_tests,
            max_cascade_retained_bytes: cascade.max_retained_bytes,
            max_cascade_output_bytes: cascade.max_output_bytes,
            max_style_rules: rule_tree.max_rules,
            max_style_selectors: rule_tree.max_selectors,
            max_style_declarations: rule_tree.max_declarations,
        }
    }
}

impl RenderLimits {
    /// Shortcut equivalent to `Default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a fluent builder.
    pub fn builder() -> RenderLimitsBuilder {
        RenderLimitsBuilder::default()
    }

    /// The limits of a cascade run within these limits: the `max_cascade_*`
    /// fields.
    pub fn cascade_limits(&self) -> CascadeLimits {
        let mut limits = CascadeLimits::default();
        limits.max_candidates_per_element = self.max_cascade_candidates_per_element;
        limits.max_declarations_visited = self.max_cascade_declarations;
        limits.max_selector_tests = self.max_cascade_selector_tests;
        limits.max_retained_bytes = self.max_cascade_retained_bytes;
        limits.max_output_bytes = self.max_cascade_output_bytes;
        limits
    }

    /// The limits of a rule tree built within these limits: the `max_style_*`
    /// fields.
    pub fn rule_tree_limits(&self) -> RuleTreeLimits {
        let mut limits = RuleTreeLimits::default();
        limits.max_rules = self.max_style_rules;
        limits.max_selectors = self.max_style_selectors;
        limits.max_declarations = self.max_style_declarations;
        limits
    }
}

/// Fluent builder for `RenderLimits`. Unset fields use their default values.
#[derive(Debug, Default, Clone)]
pub struct RenderLimitsBuilder {
    max_document_pages: Option<Option<u32>>,
    max_dom_nodes: Option<Option<u64>>,
    max_target_slots: Option<Option<u32>>,
    max_layout_buffer_entries: Option<Option<u32>>,
    max_aggregate_bytes: Option<Option<u64>>,
    max_input_bytes: Option<Option<u64>>,
    max_parse_warnings: Option<Option<usize>>,
    max_cascade_candidates_per_element: Option<Option<u32>>,
    max_cascade_declarations: Option<Option<u64>>,
    max_cascade_selector_tests: Option<Option<u64>>,
    max_cascade_retained_bytes: Option<Option<u64>>,
    max_cascade_output_bytes: Option<Option<u64>>,
    max_style_rules: Option<Option<u64>>,
    max_style_selectors: Option<Option<u64>>,
    max_style_declarations: Option<Option<u64>>,
}

impl RenderLimitsBuilder {
    /// Set `max_document_pages` (`None` = unbounded).
    pub fn max_document_pages(mut self, v: Option<u32>) -> Self {
        self.max_document_pages = Some(v);
        self
    }

    /// Set `max_dom_nodes`.
    pub fn max_dom_nodes(mut self, v: Option<u64>) -> Self {
        self.max_dom_nodes = Some(v);
        self
    }

    /// Set `max_target_slots`.
    pub fn max_target_slots(mut self, v: Option<u32>) -> Self {
        self.max_target_slots = Some(v);
        self
    }

    /// Set `max_layout_buffer_entries`.
    pub fn max_layout_buffer_entries(mut self, v: Option<u32>) -> Self {
        self.max_layout_buffer_entries = Some(v);
        self
    }

    /// Set `max_aggregate_bytes`.
    pub fn max_aggregate_bytes(mut self, v: Option<u64>) -> Self {
        self.max_aggregate_bytes = Some(v);
        self
    }

    /// Set `max_input_bytes` (`None` disables the cap).
    pub fn max_input_bytes(mut self, v: Option<u64>) -> Self {
        self.max_input_bytes = Some(v);
        self
    }

    /// Set `max_parse_warnings` (`None` disables the cap).
    pub fn max_parse_warnings(mut self, v: Option<usize>) -> Self {
        self.max_parse_warnings = Some(v);
        self
    }

    /// Set `max_cascade_candidates_per_element` (`None` disables the limit).
    pub fn max_cascade_candidates_per_element(mut self, v: Option<u32>) -> Self {
        self.max_cascade_candidates_per_element = Some(v);
        self
    }

    /// Set `max_cascade_declarations` (`None` disables the limit).
    pub fn max_cascade_declarations(mut self, v: Option<u64>) -> Self {
        self.max_cascade_declarations = Some(v);
        self
    }

    /// Set `max_cascade_selector_tests` (`None` disables the limit).
    pub fn max_cascade_selector_tests(mut self, v: Option<u64>) -> Self {
        self.max_cascade_selector_tests = Some(v);
        self
    }

    /// Set `max_cascade_retained_bytes` (`None` disables the limit).
    pub fn max_cascade_retained_bytes(mut self, v: Option<u64>) -> Self {
        self.max_cascade_retained_bytes = Some(v);
        self
    }

    /// Set `max_cascade_output_bytes` (`None` disables the limit).
    pub fn max_cascade_output_bytes(mut self, v: Option<u64>) -> Self {
        self.max_cascade_output_bytes = Some(v);
        self
    }

    /// Set `max_style_rules` (`None` disables the limit).
    pub fn max_style_rules(mut self, v: Option<u64>) -> Self {
        self.max_style_rules = Some(v);
        self
    }

    /// Set `max_style_selectors` (`None` disables the limit).
    pub fn max_style_selectors(mut self, v: Option<u64>) -> Self {
        self.max_style_selectors = Some(v);
        self
    }

    /// Set `max_style_declarations` (`None` disables the limit).
    pub fn max_style_declarations(mut self, v: Option<u64>) -> Self {
        self.max_style_declarations = Some(v);
        self
    }

    /// Build; unset fields use their default values.
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
            max_cascade_candidates_per_element: self
                .max_cascade_candidates_per_element
                .unwrap_or(d.max_cascade_candidates_per_element),
            max_cascade_declarations: self
                .max_cascade_declarations
                .unwrap_or(d.max_cascade_declarations),
            max_cascade_selector_tests: self
                .max_cascade_selector_tests
                .unwrap_or(d.max_cascade_selector_tests),
            max_cascade_retained_bytes: self
                .max_cascade_retained_bytes
                .unwrap_or(d.max_cascade_retained_bytes),
            max_cascade_output_bytes: self
                .max_cascade_output_bytes
                .unwrap_or(d.max_cascade_output_bytes),
            max_style_rules: self.max_style_rules.unwrap_or(d.max_style_rules),
            max_style_selectors: self.max_style_selectors.unwrap_or(d.max_style_selectors),
            max_style_declarations: self
                .max_style_declarations
                .unwrap_or(d.max_style_declarations),
        }
    }
}

/// Configuration for the LayoutBuffer lookahead range.
///
/// Initial seed values (based on blitz conventions; to be refined).
///
/// Listed among the structs in spec §4, so it is `#[non_exhaustive]`
/// (round 3 Missing #6).
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LookaheadConfig {
    /// Number of lines to buffer ahead for widow detection.
    pub widow_line_buffer: usize,
    /// Number of lines to buffer behind for orphan detection.
    pub orphan_line_buffer: usize,
    /// Maximum number of blocks in a `break-inside: avoid` subtree.
    pub break_avoid_max_subtree_blocks: usize,
    /// Probe-layout limit for flex / grid containers (`None` = unbounded).
    pub max_container_probe_pages: Option<usize>,
    /// Whether to allow lookahead in the cross-size direction.
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
    /// Shortcut equivalent to `Default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a fluent builder.
    pub fn builder() -> LookaheadConfigBuilder {
        LookaheadConfigBuilder::default()
    }
}

/// Fluent builder for `LookaheadConfig`.
#[derive(Debug, Default, Clone)]
pub struct LookaheadConfigBuilder {
    widow_line_buffer: Option<usize>,
    orphan_line_buffer: Option<usize>,
    break_avoid_max_subtree_blocks: Option<usize>,
    max_container_probe_pages: Option<Option<usize>>,
    allow_cross_size_lookahead: Option<bool>,
}

impl LookaheadConfigBuilder {
    /// Set `widow_line_buffer`.
    pub fn widow_line_buffer(mut self, v: usize) -> Self {
        self.widow_line_buffer = Some(v);
        self
    }

    /// Set `orphan_line_buffer`.
    pub fn orphan_line_buffer(mut self, v: usize) -> Self {
        self.orphan_line_buffer = Some(v);
        self
    }

    /// Set `break_avoid_max_subtree_blocks`.
    pub fn break_avoid_max_subtree_blocks(mut self, v: usize) -> Self {
        self.break_avoid_max_subtree_blocks = Some(v);
        self
    }

    /// Set `max_container_probe_pages` (`None` = unbounded).
    pub fn max_container_probe_pages(mut self, v: Option<usize>) -> Self {
        self.max_container_probe_pages = Some(v);
        self
    }

    /// Set `allow_cross_size_lookahead`.
    pub fn allow_cross_size_lookahead(mut self, v: bool) -> Self {
        self.allow_cross_size_lookahead = Some(v);
        self
    }

    /// Build; unset fields use their default values.
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

/// Configuration for document layout and page production.
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct LayoutConfig {
    /// Lookahead settings.
    pub lookahead: LookaheadConfig,
    /// Resource and cost limits.
    pub limits: RenderLimits,
    /// An optional registry hint for target resolution.
    pub initial_registry: Option<TargetRegistry>,
    /// Media type and output dimensions used to evaluate layout-time media queries.
    /// Defaults to the nominal print page box in [`raikiri_style::MediaContext::print`].
    /// Supply the selected paper size with [`raikiri_style::MediaContext::with_viewport`]
    /// when it differs; page defaults, authored sizes, and margins do not change
    /// this environment. Element styles, page styles, and font faces share it.
    pub media_context: raikiri_style::MediaContext,
    /// Optional cooperative cancellation signal checked before layout and
    /// during page production.
    pub signal: Option<crate::AbortSignal>,
}

impl LayoutConfig {
    /// Shortcut equivalent to `Default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a fluent builder.
    pub fn builder() -> LayoutConfigBuilder {
        LayoutConfigBuilder::default()
    }
}

/// Fluent builder for [`LayoutConfig`].
#[derive(Debug, Default, Clone)]
pub struct LayoutConfigBuilder {
    lookahead: Option<LookaheadConfig>,
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
    signal: Option<Option<crate::AbortSignal>>,
    media_context: Option<raikiri_style::MediaContext>,
}

impl LayoutConfigBuilder {
    /// Set `lookahead`.
    pub fn lookahead(mut self, v: LookaheadConfig) -> Self {
        self.lookahead = Some(v);
        self
    }

    /// Set `limits`.
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// Set `initial_registry`.
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Set the media type and viewport for layout-time media queries.
    pub fn media_context(mut self, v: raikiri_style::MediaContext) -> Self {
        self.media_context = Some(v);
        self
    }

    /// Set a cooperative cancellation signal.
    pub fn signal(mut self, v: Option<crate::AbortSignal>) -> Self {
        self.signal = Some(v);
        self
    }

    /// Build; unset fields use their default values.
    pub fn build(self) -> LayoutConfig {
        let d = LayoutConfig::default();
        LayoutConfig {
            lookahead: self.lookahead.unwrap_or(d.lookahead),
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
            signal: self.signal.unwrap_or(d.signal),
            media_context: self.media_context.unwrap_or(d.media_context),
        }
    }
}

/// Configuration for `render_batch()` (round 4 review #1 moved
/// `max_document_pages` into `limits`).
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct BatchConfig {
    /// Resource and cost limits.
    pub limits: RenderLimits,
    /// Pass the result of `plan` as a hint.
    pub initial_registry: Option<TargetRegistry>,
}

impl BatchConfig {
    /// Shortcut equivalent to `Default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a fluent builder.
    pub fn builder() -> BatchConfigBuilder {
        BatchConfigBuilder::default()
    }
}

/// Fluent builder for `BatchConfig`.
#[derive(Debug, Default, Clone)]
pub struct BatchConfigBuilder {
    limits: Option<RenderLimits>,
    initial_registry: Option<Option<TargetRegistry>>,
}

impl BatchConfigBuilder {
    /// Set `limits`.
    pub fn limits(mut self, v: RenderLimits) -> Self {
        self.limits = Some(v);
        self
    }

    /// Set `initial_registry`.
    pub fn initial_registry(mut self, v: Option<TargetRegistry>) -> Self {
        self.initial_registry = Some(v);
        self
    }

    /// Build; unset fields use their default values.
    pub fn build(self) -> BatchConfig {
        let d = BatchConfig::default();
        BatchConfig {
            limits: self.limits.unwrap_or(d.limits),
            initial_registry: self.initial_registry.unwrap_or(d.initial_registry),
        }
    }
}
