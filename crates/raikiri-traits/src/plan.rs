//! Output types for `plan()` (dry run).
//!
//! Finding #5: raikiri does not iterate on its own. `plan` returns a one-pass
//! `DocumentPlan`, which the consumer uses as a hint in its own convergence loop
//! and passes back as input.

use crate::error::UnresolvedTarget;
use crate::page::{PageBox, TargetRegistry};

/// `plan()` output (without constructing a paint scene or PaintedBox).
///
/// Uses: fulgur pass-1 reconnaissance, cost estimates, and target convergence hints.
#[derive(Debug)]
pub struct DocumentPlan {
    /// Total number of pages.
    pub total_pages: u32,
    /// A hint: recalculated during rendering (Finding #5).
    pub target_registry: TargetRegistry,
    /// List of target definitions (to be populated in the future).
    pub target_definitions: Vec<TargetDefinition>,
    /// List of unresolved targets.
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// Summary of each page (without box tree or glyph information).
    pub page_summary: Vec<PageSummary>,
}

/// Summary of one page (an element of `plan()` output).
#[derive(Debug)]
pub struct PageSummary {
    /// Zero-indexed page number.
    pub page_index: u32,
    /// Resolved effective `@page` for this page.
    pub page_box: PageBox,
    /// Reason for the page break.
    pub break_reason: BreakReason,
    /// Number of target slots on this page.
    pub target_slot_count: u32,
    /// Number of targets newly defined on this page.
    pub target_definition_count: u32,
    /// Content height of this page (physical pixels).
    pub content_height: f32,
}

/// Reason for a page break, such as `page-break-*` or auto-fill (future variants).
///
/// Currently uninhabited. Planned variants:
///   - `PageBreakBefore { property: BreakProperty }`
///   - `PageBreakAfter { property: BreakProperty }`
///   - `PageBreakInside`
///   - `AutoFill { content_height: f32, page_height: f32 }`
///   - `ExplicitBreakElement { node_id: NodeId }`
#[non_exhaustive]
#[derive(Debug)]
pub enum BreakReason {
    // To be populated in the future.
}

/// Information about a target definition (the source location referenced by `target-counter`, etc.).
///
/// Fields to be populated in the future; currently opaque.
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetDefinition {
    // To be populated in the future:
    //   pub fragment_id: Symbol,
    //   pub page_index: u32,
    //   pub kind: TargetKind,
    //   pub text: String,
    //   ...
}

impl TargetDefinition {
    /// Placeholder constructor (not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}
