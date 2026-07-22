//! Strategy traits — Streaming と Batch で切り替わる per-render policy。
//!
//! raikiri-dom が各 trait の default 実装群 (BoundedLookahead, UnboundedLookahead,
//! PlaceholderTargetResolver, RegistryTargetResolver, ImmediateEmission,
//! DeferredEmission, AggressiveCommit) を提供する。Consumer は
//! `render_with()` 経由で任意 strategy を組み合わせられる (§4 参照)。

use std::marker::PhantomData;

use crate::page::{PageContext, PageFragment};
use crate::sink::RenderSink;

/// LayoutBuffer の lookahead 幅を制御する strategy trait。
pub trait LookaheadPolicy {
    /// widow / orphan 判定のため何行先まで探査するか (None = unbounded)。
    fn max_widow_orphan_lines(&self) -> Option<usize>;

    /// `break-inside: avoid` subtree の最大 block 数 (None = unbounded)。
    fn max_break_avoid_subtree_blocks(&self) -> Option<usize>;

    /// flex / grid container の probe layout 上限 (Finding #2 対応)。
    ///
    /// - `None` = unbounded (Batch: container 全体を Fragmentation L3 準拠に layout)
    /// - `Some(N)` = N ページ相当まで、超えたら ReflowPolicy に委譲
    fn max_container_probe_pages(&self) -> Option<usize>;

    /// cross-size (block-progression direction) 方向の lookahead を許可するか。
    fn allow_cross_size_lookahead(&self) -> bool;
}

/// target-* の解決方式 (placeholder emit / 事前 registry lookup)。
pub trait TargetResolver {
    /// 1 target 参照を resolve。
    fn resolve(&mut self, req: TargetRequest<'_>, ctx: &PageContext) -> ResolvedTarget;
}

/// PageFragment の emit タイミング (immediate / deferred)。
pub trait EmissionPolicy {
    /// 1 ページの emit。
    fn emit(&mut self, page: PageFragment, sink: &mut dyn RenderSink) -> std::io::Result<()>;

    /// 全ページ emit 完了通知。
    fn finish(&mut self, sink: &mut dyn RenderSink) -> std::io::Result<()>;
}

/// probe 限界到達時の挙動、および dirty tracking の余地。
///
/// M1〜M8 は `AggressiveCommit` のみ実装、`DirtyDeferred` / `FullReflow` は
/// Future Work (§4 参照)。
pub trait ReflowPolicy {
    /// probe 限界到達時に "即 fallback commit" するか "dirty flag で defer" するか。
    fn on_probe_limit(&self, ctx: &ProbeContext) -> ReflowAction;

    /// dirty tracking を使う場合の memory 上限。
    fn max_dirty_entries(&self) -> Option<usize>;
}

/// ReflowPolicy が返す action。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ReflowAction {
    /// 即 fallback で commit、取り消し不可 (Streaming preset default)。
    CommitWithFallback(ContainerOverflowFallback),
    /// dirty flag で defer、後続情報で reflow (post-M8 Future Work)。
    DeferAsDirty {
        /// defer の deadline。
        deadline: DirtyDeadline,
    },
}

/// probe 限界時の commit fallback 挙動。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerOverflowFallback {
    /// 次ページに強制配置 (推奨)。
    ForceBreakBefore,
    /// `align-content` 等を無視した単純分割。
    SimpleFragmentation,
    /// 現ページに詰めて overflow。
    OverflowClipping,
    /// fail-loud。
    Error,
}

/// DeferAsDirty の deadline。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyDeadline {
    /// 次のページ確定まで defer。
    NextPageBoundary,
    /// 次の container 出現まで defer。
    NextContainerStart,
    /// document 末尾まで defer (Batch preset で活用)。
    DocumentEnd,
}

/// ReflowPolicy が受け取る probe context (M2 で populate)。
///
/// M1.1 では opaque placeholder。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ProbeContext {
    // M2 で populate:
    //   pub node_id: NodeId,
    //   pub probed_pages: u32,
    //   pub container_kind: ContainerKind,
    //   ...
}

impl ProbeContext {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self::default()
    }
}

/// TargetResolver が受け取る request。
///
/// M6+ consumer (raikiri-dom / raikiri-paint) 実装時に field を populate
/// (raikiri-spike-376 amended taxonomy、2026-07-19 PMO)。
///
/// M1.1〜M5 では opaque placeholder。
#[allow(missing_docs)]
#[derive(Debug)]
#[non_exhaustive]
pub struct TargetRequest<'a> {
    // M4 で populate:
    //   pub fragment_id: Symbol,
    //   pub kind: TargetKind,
    //   pub source_page: u32,
    _marker: PhantomData<&'a ()>,
}

impl<'a> TargetRequest<'a> {
    /// M1.1 placeholder constructor.
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

/// TargetResolver が返す resolved 情報。
///
/// M6+ consumer (raikiri-dom / raikiri-paint) 実装時に variant を populate
/// (raikiri-spike-376 amended taxonomy、2026-07-19 PMO)。
///
/// M1.1〜M5 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolvedTarget {
    // M4 で populate:
    //   Placeholder { slot_id: TargetSlotId },
    //   Immediate { text: String, kind: TargetKind },
    //   Deferred { fragment_id: Symbol },
    //   Unresolved { fragment_id: Symbol, reason: UnresolvedReason },
}
