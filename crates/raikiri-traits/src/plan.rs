//! `plan()` (dry-run) 用の output 型。
//!
//! Finding #5 対応: raikiri は自身で iteration しない。`plan` は 1 pass の
//! `DocumentPlan` を返し、Consumer が自身の convergence loop で hint として
//! 再入力する。

use crate::error::UnresolvedTarget;
use crate::page::{PageBox, TargetRegistry};

/// `plan()` の output (paint scene / PaintedBox 構築なし)。
///
/// 用途: fulgur の pass-1 前哨、cost 見積り、target 収束判定用の hint 生成。
#[derive(Debug)]
pub struct DocumentPlan {
    /// 総ページ数。
    pub total_pages: u32,
    /// hint: render 時は再計算される (Finding #5)。
    pub target_registry: TargetRegistry,
    /// target 定義 list (M4 で populate)。
    pub target_definitions: Vec<TargetDefinition>,
    /// 未解決 target list。
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// 各ページの summary (box tree, glyph 情報なし)。
    pub page_summary: Vec<PageSummary>,
}

/// 1 ページの summary (`plan()` output の要素)。
#[derive(Debug)]
pub struct PageSummary {
    /// 0-indexed page number。
    pub page_index: u32,
    /// このページの実効 `@page` 解決結果。
    pub page_box: PageBox,
    /// なぜこのページで break したか。
    pub break_reason: BreakReason,
    /// このページに含まれる target slot 数。
    pub target_slot_count: u32,
    /// このページで新規定義された target 数。
    pub target_definition_count: u32,
    /// このページの content 高さ (physical pixel)。
    pub content_height: f32,
}

/// `page-break-*` / auto-fill 等、ページ break の理由 (M2 で variant を populate)。
///
/// M1.1 では uninhabited。想定 variant:
///   - `PageBreakBefore { property: BreakProperty }`
///   - `PageBreakAfter { property: BreakProperty }`
///   - `PageBreakInside`
///   - `AutoFill { content_height: f32, page_height: f32 }`
///   - `ExplicitBreakElement { node_id: NodeId }`
#[non_exhaustive]
#[derive(Debug)]
pub enum BreakReason {
    // M2 で populate。
}

/// target-* の定義側情報 (`target-counter` などが参照する source location)。
///
/// M4 で fields を populate。M1.1 では opaque。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetDefinition {
    // M4 で populate:
    //   pub fragment_id: Symbol,
    //   pub page_index: u32,
    //   pub kind: TargetKind,
    //   pub text: String,
    //   ...
}

impl TargetDefinition {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self::default()
    }
}
