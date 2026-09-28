//! Strategy traits — per-render policies that vary between Streaming and Batch.
//!
//! raikiri-dom provides default implementations for each trait (BoundedLookahead,
//! PlaceholderTargetResolver, RegistryTargetResolver, ImmediateEmission,
//! DeferredEmission, AggressiveCommit). Consumers can combine arbitrary
//! strategies via `render_with()` (see §4).

use std::marker::PhantomData;

use crate::page::{PageContext, PageFragment};
use crate::sink::RenderSink;

/// Strategy trait controlling the LayoutBuffer lookahead range.
pub trait LookaheadPolicy {
    /// How many lines ahead to probe for widows / orphans (None = unbounded).
    fn max_widow_orphan_lines(&self) -> Option<usize>;

    /// Maximum blocks in a `break-inside: avoid` subtree (None = unbounded).
    fn max_break_avoid_subtree_blocks(&self) -> Option<usize>;

    /// Flex / grid container probe-layout limit (Finding #2).
    ///
    /// - `None` = unbounded (Batch: lay out the entire container per Fragmentation L3)
    /// - `Some(N)` = at most N pages; delegate to ReflowPolicy beyond that
    fn max_container_probe_pages(&self) -> Option<usize>;

    /// Whether to allow lookahead along the cross-size (block-progression) direction.
    fn allow_cross_size_lookahead(&self) -> bool;
}

/// Target-* resolution mode (placeholder emission / prior registry lookup).
pub trait TargetResolver {
    /// Resolve one target reference.
    fn resolve(&mut self, req: TargetRequest<'_>, ctx: &PageContext) -> ResolvedTarget;
}

/// When to emit a PageFragment (immediately / deferred).
pub trait EmissionPolicy {
    /// Emit one page.
    fn emit(&mut self, page: PageFragment, sink: &mut dyn RenderSink) -> std::io::Result<()>;

    /// Notify that all pages have been emitted.
    fn finish(&mut self, sink: &mut dyn RenderSink) -> std::io::Result<()>;
}

/// Behavior on reaching the probe limit, and room for dirty tracking.
///
/// Currently only `AggressiveCommit` is implemented; `DirtyDeferred` /
/// `FullReflow` remain Future Work (see §4).
pub trait ReflowPolicy {
    /// Whether to commit fallback immediately or defer with a dirty flag at the probe limit.
    fn on_probe_limit(&self, ctx: &ProbeContext) -> ReflowAction;

    /// Memory limit when using dirty tracking.
    fn max_dirty_entries(&self) -> Option<usize>;
}

/// Action returned by ReflowPolicy.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ReflowAction {
    /// Commit fallback immediately; cannot be undone (Streaming preset default).
    CommitWithFallback(ContainerOverflowFallback),
    /// Defer with a dirty flag and reflow with later information (Future Work).
    DeferAsDirty {
        /// Deadline for the deferral.
        deadline: DirtyDeadline,
    },
}

/// Commit fallback behavior at the probe limit.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerOverflowFallback {
    /// Force placement on the next page (recommended).
    ForceBreakBefore,
    /// Simple split ignoring `align-content` and similar properties.
    SimpleFragmentation,
    /// Pack into the current page and overflow.
    OverflowClipping,
    /// fail-loud.
    Error,
}

/// Deadline for DeferAsDirty.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyDeadline {
    /// Defer until the next page is finalized.
    NextPageBoundary,
    /// Defer until the next container appears.
    NextContainerStart,
    /// Defer until the end of the document (used by the Batch preset).
    DocumentEnd,
}

/// Probe context passed to ReflowPolicy (to be populated later).
///
/// Currently an opaque placeholder.
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProbeContext {
    // To be populated later:
    //   pub node_id: NodeId,
    //   pub probed_pages: u32,
    //   pub container_kind: ContainerKind,
    //   ...
}

impl ProbeContext {
    /// Placeholder constructor (not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request passed to TargetResolver.
///
/// Populate the fields when a consumer (raikiri-dom / raikiri-paint) is implemented.
///
/// Currently an opaque placeholder.
#[allow(missing_docs)]
#[derive(Debug)]
#[non_exhaustive]
pub struct TargetRequest<'a> {
    // To be populated later:
    //   pub fragment_id: Symbol,
    //   pub kind: TargetKind,
    //   pub source_page: u32,
    _marker: PhantomData<&'a ()>,
}

impl<'a> TargetRequest<'a> {
    /// Placeholder constructor (not yet populated).
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<'a> Default for TargetRequest<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolved information returned by TargetResolver.
///
/// Populate variants when a consumer (raikiri-dom / raikiri-paint) is implemented.
///
/// Currently uninhabited.
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolvedTarget {
    // To be populated later:
    //   Placeholder { slot_id: TargetSlotId },
    //   Immediate { text: String, kind: TargetKind },
    //   Deferred { fragment_id: Symbol },
    //   Unresolved { fragment_id: Symbol, reason: UnresolvedReason },
}
