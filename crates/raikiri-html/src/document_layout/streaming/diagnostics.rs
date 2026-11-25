//! Optional operation counts for streaming performance measurements.

/// Streaming work through the final layout, enabled by `streaming-diagnostics`.
///
/// Node counts describe source arena entries, including detached nodes, rather
/// than heap bytes. Snapshots occur at checkpoints; finishing consumes the
/// parser's document without taking another snapshot. Resume attempts include
/// retries caused by late content, and can exceed the number of checkpoints.
// cov:ignore: value-only declaration; LLVM maps derived operations to their call sites. Counter updates are tested in streaming.rs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct StreamingDiagnostics {
    /// Compact table inputs attempted at the first checkpoint.
    pub table_inputs: u32,
    /// Finished inputs with a replayable table structure.
    pub table_structures: u32,
    /// Replayable tables accepted by the stylesheet check.
    pub table_styles: u32,
    /// Accepted tables with a complete pagination plan.
    pub table_plans: u32,
    /// Wall time spent restoring and laying out windows to measure columns.
    pub table_measure_time: std::time::Duration,
    /// Wall time spent validating page boundaries, including any tentative painting.
    pub table_plan_time: std::time::Duration,
    /// Portion of planning spent assembling and laying out the document windows.
    pub table_window_layout_time: std::time::Duration,
    /// Portion of planning spent in the sink's tentative painting callback.
    pub table_stage_time: std::time::Duration,
    /// Wall time spent restoring, laying out, and delivering planned windows.
    pub table_delivery_time: std::time::Duration,
    /// Pages delivered from a table whose body rows were spooled to disk.
    pub table_window_pages: u32,
    /// Nodes retained by the parser when parsing finished.
    pub parser_nodes: u64,
    /// Checkpoint snapshots taken after a body candidate was created.
    pub snapshot_calls: u64,
    /// Sum of the source node counts of those snapshots.
    pub snapshot_nodes: u64,
    /// Source node clone operations while preparing those snapshots.
    /// Arena growth can also relocate existing storage; allocator traffic is
    /// measured separately rather than included in this count.
    pub snapshot_copied_nodes: u64,
    /// Layout calls that tried to resume after delivered pages.
    pub resume_attempts: u64,
    /// Layout calls that accepted the requested resume point.
    pub resumed_layouts: u64,
}
