//! Render error taxonomy + status + summary types.
//!
//! `RenderError` is terminal: it marks where rendering stopped. Consumer
//! fallbacks are represented by returning `Ok(fallback)`, and raikiri
//! records them in `RenderSummary.warnings` (Finding #1, new review).

use url::Url;

use crate::dom::{NodeId, Symbol};
use crate::net::NetworkError;
use crate::page::TargetRegistry;
use crate::policy::{PolicyViolation, ResourceKind};
use crate::resolver::ResolverError;

// CSS cascade error taxonomy is owned by raikiri-style (Stylo pattern).
// Re-exported here so `RenderError::Cascade`
// and `impl From<CascadeError> for RenderError` below (which reference
// `CascadeError` by unqualified path) keep the same identity, and downstream
// consumers observing `raikiri_traits::CascadeError` (raikiri umbrella's
// re-export in `crates/raikiri/src/lib.rs`) are unchanged.
pub use raikiri_style::CascadeError;
use raikiri_style::CascadeLimitKind;

/// Terminal render error. Every variant means rendering stopped at that point.
///
/// Finding #10 (structured error taxonomy). Round 4 review #1 unified
/// limit errors as `LimitExceeded`.
#[non_exhaustive]
#[derive(Debug)]
pub enum RenderError {
    /// HTML parse error.
    Parse(ParseError),
    /// CSS parse / cascade error.
    Cascade(CascadeError),
    /// Layout error.
    Layout(LayoutError),
    /// A consumer resolver returned Err.
    Resolver(ResolverError),
    /// A consumer network provider returned Err.
    Network(NetworkError),
    /// Resource policy violation.
    ///
    /// Boxed for the same reason as `NetworkError::PolicyViolation`
    /// (`PolicyViolation` is large).
    Policy(Box<PolicyViolation>),
    /// Exceeded a `RenderLimits` limit (fail-fast; round 4 review #1
    /// subsumed the former `PageLimitExceeded` under `kind: Pages`).
    LimitExceeded {
        /// Which limit was exceeded.
        kind: LimitKind,
        /// Configured limit.
        limit: u64,
        /// Observed value.
        actual: u64,
    },
    /// A consumer property observer returned an IO error.
    Observer(std::io::Error),
    /// Inconsistent configuration (such as invalid BatchConfig.initial_registry).
    Configuration(String),
    /// target-* failed to converge within `max_target_iterations` (round 6 review #5;
    /// only when the consumer selects `ExhaustionPolicy::Error`).
    TargetDidNotConverge {
        /// Number of iterations performed.
        iterations: u32,
    },
    /// Page geometry kept changing after the bounded scheduled layout passes.
    /// No pages are emitted when this terminal status is returned, because the
    /// page-local rectangles and resolved page metadata would otherwise come
    /// from different pagination schedules.
    PageGeometryDidNotConverge {
        /// Number of scheduled layout passes attempted.
        iterations: u32,
    },
    /// Other `std::io::Error`-family error.
    Io(std::io::Error),

    /// Call to an unavailable API. This variant will be **removed** when the
    /// implementation is complete (documented as a breaking change in release notes).
    /// Consumers need match it only while the API is unimplemented; afterwards
    /// they can remove the arm. `feature` identifies the API (`"plan"`, `"render_streaming"`, etc.).
    Unimplemented {
        /// Name of the unimplemented API.
        feature: &'static str,
        /// Migration hint for consumers.
        migration_hint: &'static str,
    },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(_) => write!(f, "HTML parse error"),
            Self::Cascade(_) => write!(f, "CSS cascade error"),
            Self::Layout(_) => write!(f, "Layout error"),
            Self::Resolver(_) => write!(f, "Replaced-element resolver error"),
            Self::Network(_) => write!(f, "Network provider error"),
            Self::Policy(_) => write!(f, "Resource policy violation"),
            Self::LimitExceeded {
                kind,
                limit,
                actual,
            } => {
                write!(
                    f,
                    "Render limit exceeded: {kind:?} (limit={limit}, actual={actual})"
                )
            }
            Self::Observer(_) => write!(f, "Observer returned I/O error"),
            Self::Configuration(msg) => write!(f, "Configuration error: {msg}"),
            Self::TargetDidNotConverge { iterations } => {
                write!(f, "target-* did not converge in {iterations} iterations")
            }
            Self::PageGeometryDidNotConverge { iterations } => {
                write!(
                    f,
                    "page geometry did not converge in {iterations} iterations"
                )
            }
            Self::Io(_) => write!(f, "I/O error"),
            Self::Unimplemented {
                feature,
                migration_hint,
            } => {
                write!(
                    f,
                    "{feature} is not implemented yet (hint: {migration_hint})"
                )
            }
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            Self::Cascade(e) => Some(e),
            Self::Layout(e) => Some(e),
            Self::Resolver(e) => Some(e),
            Self::Network(e) => Some(e),
            Self::Policy(v) => Some(&**v),
            Self::Observer(e) | Self::Io(e) => Some(e),
            Self::LimitExceeded { .. }
            | Self::Configuration(_)
            | Self::TargetDidNotConverge { .. }
            | Self::PageGeometryDidNotConverge { .. }
            | Self::Unimplemented { .. } => None,
        }
    }
}

impl From<ParseError> for RenderError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

/// A passed cascade limit becomes a passed [`RenderLimits`](crate::RenderLimits)
/// limit, the one it is configured from, as the layout limits below do. Any
/// other cascade error stays one.
impl From<CascadeError> for RenderError {
    fn from(e: CascadeError) -> Self {
        if let CascadeError::LimitExceeded {
            kind,
            limit,
            actual,
        } = e
            && let Some(kind) = render_limit_kind(kind)
        {
            return Self::LimitExceeded {
                kind,
                limit,
                actual,
            };
        }
        Self::Cascade(e)
    }
}

/// The [`LimitKind`] of the [`RenderLimits`](crate::RenderLimits) field a
/// cascade limit is configured from.
fn render_limit_kind(kind: CascadeLimitKind) -> Option<LimitKind> {
    Some(match kind {
        CascadeLimitKind::CandidatesPerElement => LimitKind::CascadeCandidatesPerElement,
        CascadeLimitKind::DeclarationsVisited => LimitKind::CascadeDeclarations,
        CascadeLimitKind::SelectorTests => LimitKind::CascadeSelectorTests,
        CascadeLimitKind::RetainedBytes => LimitKind::CascadeRetainedBytes,
        CascadeLimitKind::OutputBytes => LimitKind::CascadeOutputBytes,
        CascadeLimitKind::StyleRules => LimitKind::StyleRules,
        CascadeLimitKind::StyleSelectors => LimitKind::StyleSelectors,
        CascadeLimitKind::StyleDeclarations => LimitKind::StyleDeclarations,
        // cov:ignore: every current kind is mapped above; a kind added later
        // stays a cascade error until it is mapped here
        _ => return None,
    })
}

impl From<LayoutError> for RenderError {
    fn from(e: LayoutError) -> Self {
        match e {
            LayoutError::Resolver(re) => Self::Resolver(re),
            LayoutError::PageLimitExceeded { limit, actual } => Self::LimitExceeded {
                kind: LimitKind::Pages,
                limit: u64::from(limit),
                actual,
            },
            LayoutError::CounterSnapshotLimitExceeded { limit, actual } => Self::LimitExceeded {
                kind: LimitKind::CounterSnapshots,
                limit,
                actual,
            },
            other => Self::Layout(other),
        }
    }
}

/// Classification of exceeded limits (round 4 review #1).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Exceeded the maximum width or height of one raster buffer.
    RasterEdge,
    /// Exceeded the RGBA8 byte budget for one page.
    RasterPageBytes,
    /// Exceeded the cumulative RGBA8 byte budget for all pages in a document.
    RasterDocumentBytes,
    /// Exceeded `max_document_pages`.
    Pages,
    /// Exceeded `max_dom_nodes`.
    DomNodes,
    /// Exceeded `max_target_slots`.
    TargetSlots,
    /// Exceeded `max_layout_buffer_entries`.
    LayoutBufferEntries,
    /// Exceeded `max_aggregate_bytes` (approximate post-parse memory footprint,
    /// including DOM node arena, cascade table, etc.).
    AggregateBytes,
    /// Exceeded `max_input_bytes`.
    ///
    /// Semantic distinction from [`AggregateBytes`](Self::AggregateBytes):
    /// `InputBytes` checks the **parse-time** raw input byte stream, allowing
    /// fail-closed early return (`parse_html_with_limits` enforces it when reading).
    /// `AggregateBytes` measures approximate **post-parse** memory usage.
    /// Thus `InputBytes` more directly prevents an attacker from inducing OOM
    /// by sending huge HTML.
    InputBytes,
    /// Exceeded the hard cumulative estimated-memory budget for counter snapshots.
    CounterSnapshots,
    /// Exceeded `max_cascade_candidates_per_element`.
    CascadeCandidatesPerElement,
    /// Exceeded `max_cascade_declarations`.
    CascadeDeclarations,
    /// Exceeded `max_cascade_selector_tests`.
    CascadeSelectorTests,
    /// Exceeded `max_cascade_retained_bytes`.
    CascadeRetainedBytes,
    /// Exceeded `max_cascade_output_bytes`.
    CascadeOutputBytes,
    /// Exceeded `max_style_rules`.
    StyleRules,
    /// Exceeded `max_style_selectors`.
    StyleSelectors,
    /// Exceeded `max_style_declarations`.
    StyleDeclarations,
}

/// Render completion summary (Finding #4 completion protocol).
#[derive(Debug, Clone)]
pub struct RenderSummary {
    /// Total number of pages.
    pub total_pages: u32,
    /// Final target-* registry (used by the consumer as a patch-table base).
    pub target_registry: TargetRegistry,
    /// List of unresolved targets.
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// List of emitted target slots.
    pub emitted_target_slots: Vec<EmittedSlotInfo>,
    /// Items whose hints differ from actual values (Finding #5; used by consumers to decide convergence).
    pub target_discrepancies: Vec<TargetDiscrepancy>,
    /// Warnings about consumer fallback usage, policy violations, etc. (Finding #1, new review).
    pub warnings: Vec<RenderWarning>,
}

/// Render warning (fallback usage / policy warning / unresolved target, etc.).
#[derive(Debug, Clone)]
pub struct RenderWarning {
    /// Warning kind.
    pub kind: WarningKind,
    /// Related DOM node (optional, on element-level warnings).
    pub node_id: Option<NodeId>,
    /// Human-readable details.
    pub details: String,
}

/// Warning kind. The five base variants in §4 plus `HtmlParseError`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum WarningKind {
    /// A consumer resolver returned a fallback (`Ok(fallback_intrinsic)`).
    ResolverFallback {
        /// Target fragment ID.
        fragment_id: Symbol,
    },
    /// The intended complete fetch for this URL was unavailable, so the resource
    /// was degraded. One variant covers two dispositions:
    ///
    /// - A consumer network provider explicitly returned replacement content
    ///   (an Ok disposition, as for `ResolverFallback`; however,
    ///   `NetworkProvider::fetch` has no disposition channel yet, so
    ///   this route is currently unused).
    /// - The fetch returned `Err` (timeout / I/O / HTTP error, etc.),
    ///   and the caller continued without the resource rather than failing.
    ///   In this case, no content was applied.
    ///
    /// Current implementations produce only the Err disposition;
    /// the Ok disposition is unreachable until its channel is implemented.
    /// Nevertheless, **consumers must not infer from this variant alone
    /// that any content was applied**. `RenderWarning::details` is free-form
    /// human-readable text, not a structured disposition contract.
    /// Its current "fetch failed"-like text is written manually by the caller
    /// (`raikiri-html`) and is not guaranteed by this variant. Once the
    /// Ok disposition is implemented and both forms can be produced,
    /// a way to distinguish them must be designed separately (either
    /// structure the details as a contract or split the variants).
    ///
    /// The warning does not encode which disposition occurred.
    NetworkFallback {
        /// Target URL.
        url: Url,
    },
    /// Consumer `on_violation` treated a policy violation as Warn.
    PolicyWarning {
        /// Triggered violation.
        violation: PolicyViolation,
    },
    /// A target-* reference was not found and rendered as fallback_text.
    UnresolvedTarget {
        /// Target fragment ID.
        fragment_id: Symbol,
    },
    /// target-* did not converge within `max_target_iterations`, but the consumer
    /// selected `ExhaustionPolicy::BestEffort` and rendered best-effort
    /// (round 6 review #5).
    TargetConvergenceExhausted {
        /// Number of exhausted iterations.
        iterations: u32,
    },
    /// The html5ever tokenizer reported a non-fatal parse error (for example,
    /// while recovering malformed HTML). Following Stylo/blitz responsibility
    /// boundaries, raikiri-html downgrades it to a warning and continues rendering.
    /// The orchestrator merges Document warnings into `RenderSummary.warnings`.
    HtmlParseError {
        /// Diagnostic message returned by html5ever (Cow<'static, str> converted to String).
        message: String,
    },
    /// A resource could not be used, but rendering continued with a fallback.
    ResourceFallback {
        /// Resource category.
        kind: ResourceKind,
        /// Absolute resource URL when one was available.
        url: Option<Url>,
    },
    /// A response or aggregate resource byte limit was reached and the resource
    /// was skipped. The caller can distinguish this from a provider failure.
    ResourceLimitExceeded {
        /// Resource category.
        kind: ResourceKind,
        /// Configured byte limit.
        limit: u64,
        /// Observed response or aggregate byte count.
        actual: u64,
    },
    /// The positioned glyph runs of a paragraph are not reported because its
    /// text is placed by geometry they do not model yet (for example a
    /// vertical writing mode or lines split across columns). The paragraph
    /// is still laid out and painted; `RenderWarning::node_id` names its root
    /// and `RenderWarning::details` the reason.
    TextRunsOmitted,
    /// The paint order of a subtree is listed with less structure than the
    /// painter uses (for example columns without their column clips). The
    /// subtree's events are still listed; `RenderWarning::node_id` names the
    /// subtree root and `RenderWarning::details` the reason.
    PaintOrderApproximated,
}

/// Behavior when the consumer convergence loop exhausts `max_target_iterations`
/// (round 6 review #5; disallows silently continuing).
///
/// This contract concerns consumer-side iteration; raikiri's `plan()` / `render_*`
/// APIs do not consume it (consumers use it in their own loops).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExhaustionPolicy {
    /// Return non-convergence upstream as an error (conservative default).
    #[default]
    Error,
    /// Render using the last registry and always record
    /// `WarningKind::TargetConvergenceExhausted` in `summary.warnings`.
    BestEffort,
}

/// Details of an unresolved target (Finding #4 completion protocol).
#[derive(Debug, Clone)]
pub struct UnresolvedTarget {
    /// Unique slot identifier.
    pub slot_id: TargetSlotId,
    /// Unresolved fragment ID.
    pub fragment_id: Symbol,
    /// Reason it remains unresolved.
    pub reason: UnresolvedReason,
}

/// Reason an UnresolvedTarget remains unresolved (Finding #4).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// Fragment ID has no definition anywhere.
    NotFound,
    /// Treated as an error by consumer policy.
    ConsumerRejected,
    // ConvergenceFailed removed (raikiri does not iterate internally; Finding #5).
}

/// Details of an emitted target slot.
#[derive(Debug, Clone)]
pub struct EmittedSlotInfo {
    /// Unique slot identifier.
    pub slot_id: TargetSlotId,
    /// Target fragment ID.
    pub fragment_id: Symbol,
    /// Target kind.
    pub kind: TargetKind,
}

/// Unique slot identifier (used as a key in the consumer patch table).
///
/// `(page_index, sequence)` is unique in decode order and guarantees
/// byte-identical identifiers (Finding #4 completion protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetSlotId {
    /// Zero-indexed page number of this page.
    pub page_index: u32,
    /// Sequential number within the page (target-* occurrence order, zero-indexed).
    pub sequence: u32,
}

/// Kind of target-* (target-counter / target-text / target-string, etc.).
///
/// Populate variants as target-* support is implemented. Currently uninhabited.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    // Populate later:
    //   Counter,
    //   Text,
    //   String,
    //   Element,
}

/// Item whose hint differs from its actual value (consumer convergence; Finding #5).
#[derive(Debug, Clone)]
pub struct TargetDiscrepancy {
    /// Target fragment ID.
    pub fragment_id: Symbol,
    /// Page predicted at the hint stage (`None` if absent).
    pub hinted_page: Option<u32>,
    /// Page in the actual render.
    pub actual_page: u32,
    /// Text content at the hint stage (`None` if absent).
    pub hinted_text: Option<String>,
    /// Text content in the actual render.
    pub actual_text: String,
}

/// Terminal error during HTML parsing (originates in raikiri-html).
///
/// **Same responsibility boundary as Stylo/blitz**: non-fatal tokenizer
/// parse errors (recoverable malformed HTML) become [`RenderWarning`]s
/// aggregated into the summary inside raikiri-html; this enum excludes them.
/// Its variants are genuine terminal errors that halt rendering.
///
/// `Io` / `Encoding` are populated. html5ever-specific error variants
/// remain crate-private in raikiri-html and can be added here when needed
/// using `#[non_exhaustive]`.
#[non_exhaustive]
#[derive(Debug)]
pub enum ParseError {
    /// Failed to read from the input source (`std::io::Read`).
    Io(std::io::Error),
    /// Failed to convert byte stream to text (encoding label detection or conversion error).
    Encoding {
        /// Detected or specified encoding label (for example, "utf-8" or "shift_jis").
        label: String,
        /// Human-readable description of the failure.
        reason: String,
    },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(_) => write!(f, "HTML source read error"),
            Self::Encoding { label, reason } => {
                write!(f, "HTML source encoding error ({label}): {reason}")
            }
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Encoding { .. } => None,
        }
    }
}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// NB: `CascadeError` enum + `Display` + `Error` impls used to live here.
// Ownership moved to `raikiri-style::error`
// (Stylo pattern — style owns its cascade error taxonomy). The re-export at
// the top of this file preserves the `raikiri_traits::CascadeError` name path.

/// Terminal layout error (originates in raikiri-dom + taffy).
///
/// **Same responsibility boundary**: taffy-specific error types remain
/// internal to raikiri-dom; this enum signals only its explicit fail-hard cases.
///
#[non_exhaustive]
#[derive(Debug)]
pub enum LayoutError {
    /// An unrecoverable internal error occurred in raikiri-dom (such as a taffy
    /// internal error). raikiri-dom constructs the detailed logged message.
    Internal {
        /// Human-readable failure details (constructed by raikiri-dom).
        message: String,
    },
    /// Failed to resolve a replaced element such as `<img>`
    /// (`ReplacedResolver::resolve` returned `Err`).
    Resolver(ResolverError),
    /// The inline formatting engine was required for every paragraph and a
    /// paragraph uses something it does not lay out.
    IfcUnsupported {
        /// DOM node id of the element or text the refusal is about.
        node: usize,
        /// What the inline engine does not lay out.
        reason: &'static str,
    },
    /// A paragraph exceeded a resource limit of the inline formatting engine
    /// (its text length, item count or nesting depth).
    IfcLimitExceeded {
        /// DOM node id of the paragraph root.
        node: usize,
        /// Which limit, with its configured and observed values.
        limit: String,
    },
    /// A document's body subtree exceeded the maximum supported layout depth.
    TreeDepthLimitExceeded {
        /// Maximum number of nested layout elements accepted.
        limit: usize,
        /// First element depth rejected by the layout preflight.
        actual: usize,
    },
    /// Counter snapshots exceeded their hard cumulative estimated-memory budget.
    CounterSnapshotLimitExceeded {
        /// Maximum estimated bytes allowed for the operation.
        limit: u64,
        /// Estimated cumulative bytes after the rejected snapshot.
        actual: u64,
    },
    /// One layout pass would exceed its fragment, break-flow, or column-projection work cap.
    FragmentLimitExceeded {
        /// Separate maximum for retained fragments and estimated additional
        /// break-flow measurement and column-projection work units in one pass.
        limit: usize,
    },
    /// Pagination would produce more pages than the configured limit.
    PageLimitExceeded {
        /// Maximum number of pages allowed.
        limit: u32,
        /// First page count that exceeded the limit.
        actual: u64,
    },
    /// Pagination was cancelled by its caller.
    Aborted,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal { message } => write!(f, "Layout internal error: {message}"),
            Self::Resolver(e) => write!(f, "Layout resolver error: {e}"),
            Self::IfcUnsupported { node, reason } => {
                write!(f, "Layout inline engine refused node {node}: {reason}")
            }
            Self::IfcLimitExceeded { node, limit } => {
                write!(f, "Layout inline engine limit at node {node}: {limit}")
            }
            Self::TreeDepthLimitExceeded { limit, actual } => write!(
                f,
                "Layout tree depth limit exceeded: {actual} elements (limit {limit})"
            ),
            Self::CounterSnapshotLimitExceeded { limit, actual } => write!(
                f,
                "Layout counter snapshot limit exceeded: {actual} bytes (limit {limit})"
            ),
            Self::FragmentLimitExceeded { limit } => {
                write!(
                    f,
                    "Layout fragment or break-flow work limit exceeded: {limit}"
                )
            }
            Self::PageLimitExceeded { limit, actual } => {
                write!(
                    f,
                    "Layout page limit exceeded: {actual} pages (limit {limit})"
                )
            }
            Self::Aborted => write!(f, "Layout aborted"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
mod tests;
