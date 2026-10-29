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
