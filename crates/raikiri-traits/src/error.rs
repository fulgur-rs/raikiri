//! Render error taxonomy + status + summary types.
//!
//! `RenderError` は terminal error (rendering が停止した場所を表す)。Consumer
//! 側 fallback は `Ok(fallback)` を返すことで表現し、raikiri は
//! `RenderSummary.warnings` に記録する (Finding #1 新 review 対応)。

use url::Url;

use crate::dom::{NodeId, Symbol};
use crate::net::NetworkError;
use crate::page::TargetRegistry;
use crate::policy::PolicyViolation;
use crate::resolver::ResolverError;

/// Terminal render error。すべての variant は "rendering がそこで停止した" を意味。
///
/// Finding #10 対応 (構造化 error taxonomy)。round 4 review #1 対応で
/// `LimitExceeded` に統一。
#[non_exhaustive]
#[derive(Debug)]
pub enum RenderError {
    /// HTML parse エラー。
    Parse(ParseError),
    /// CSS parse / cascade エラー。
    Cascade(CascadeError),
    /// Layout エラー。
    Layout(LayoutError),
    /// Consumer の resolver が Err を返した。
    Resolver(ResolverError),
    /// Consumer の network が Err を返した。
    Network(NetworkError),
    /// Resource policy 違反。
    Policy(PolicyViolation),
    /// `RenderLimits` の各種 limit 超過 (fail-fast、round 4 review #1 対応で
    /// 旧 `PageLimitExceeded` を `kind: Pages` で吸収)。
    LimitExceeded {
        /// どの limit を超過したか。
        kind: LimitKind,
        /// 設定された limit 値。
        limit: u64,
        /// 観測された実 value。
        actual: u64,
    },
    /// Consumer の sink method (accept_page / finish_render) が Err を返した。
    Sink(std::io::Error),
    /// Config 不整合 (BatchConfig.initial_registry が不正 等)。
    Configuration(String),
    /// target-* が `max_target_iterations` 内に収束しなかった (round 6 review #5
    /// 対応、Consumer が `ExhaustionPolicy::Error` を選択した場合のみ発生)。
    TargetDidNotConverge {
        /// 実行された iteration 数。
        iterations: u32,
    },
    /// その他 `std::io::Error` 系。
    Io(std::io::Error),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(_) => write!(f, "HTML parse error"),
            Self::Cascade(_) => write!(f, "CSS cascade error"),
            Self::Layout(_) => write!(f, "Layout error"),
            Self::Resolver(_) => write!(f, "Replaced-element resolver error"),
            Self::Network(_) => write!(f, "Network provider error"),
            Self::Policy(v) => write!(f, "Resource policy violation: {:?}", v.violation_type),
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
            Self::Sink(_) => write!(f, "Sink returned I/O error"),
            Self::Configuration(msg) => write!(f, "Configuration error: {msg}"),
            Self::TargetDidNotConverge { iterations } => {
                write!(f, "target-* did not converge in {iterations} iterations")
            }
            Self::Io(_) => write!(f, "I/O error"),
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sink(e) | Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Limit exceeded の分類 (round 4 review #1 対応)。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// `max_document_pages` 超過。
    Pages,
    /// `max_dom_nodes` 超過。
    DomNodes,
    /// `max_target_slots` 超過。
    TargetSlots,
    /// `max_layout_buffer_entries` 超過。
    LayoutBufferEntries,
    /// `max_aggregate_bytes` 超過。
    AggregateBytes,
}

/// AbortSignal による graceful shutdown を error と別カテゴリで表現。
/// `render_*` は `Result<RenderStatus, RenderError>` を返す。
#[non_exhaustive]
#[derive(Debug)]
pub enum RenderStatus {
    /// 全ページ emit 完了、`finish_render` も成功。
    Completed(RenderSummary),
    /// AbortSignal による中断。直前まで emit 済み、`finish_render` は呼ばれない。
    Aborted {
        /// 中断前に commit されたページ数。
        partial_pages: u32,
    },
}

/// Render 完了 summary (Finding #4 completion protocol)。
#[derive(Debug)]
pub struct RenderSummary {
    /// 総ページ数。
    pub total_pages: u32,
    /// target-* の最終 registry (Consumer が patch table の base に利用)。
    pub target_registry: TargetRegistry,
    /// 未解決 target list。
    pub unresolved_targets: Vec<UnresolvedTarget>,
    /// emit 済み target slot list。
    pub emitted_target_slots: Vec<EmittedSlotInfo>,
    /// hint と actual の乖離を検知した項目 (Finding #5 対応、Consumer 収束判定用)。
    pub target_discrepancies: Vec<TargetDiscrepancy>,
    /// Consumer's fallback usage / policy violation 等の警告 (Finding #1 新 review 対応)。
    pub warnings: Vec<RenderWarning>,
}

/// Render 警告 (fallback usage / policy warning / unresolved target 等)。
#[derive(Debug)]
pub struct RenderWarning {
    /// 警告 kind。
    pub kind: WarningKind,
    /// 関連 DOM node (option、element-level warning に付く)。
    pub node_id: Option<NodeId>,
    /// 人間可読な詳細。
    pub details: String,
}

/// 警告 kind。§4 の 5 variant を再現。
#[non_exhaustive]
#[derive(Debug)]
pub enum WarningKind {
    /// Consumer の resolver が fallback を返した (`Ok(fallback_intrinsic)`)。
    ResolverFallback {
        /// 対象 fragment id。
        fragment_id: Symbol,
    },
    /// Consumer の network が fallback を返した。
    NetworkFallback {
        /// 対象 URL。
        url: Url,
    },
    /// Policy violation を Consumer の on_violation が Warn 扱いにした。
    PolicyWarning {
        /// 発火した違反。
        violation: PolicyViolation,
    },
    /// target-* 参照先が見つからず fallback_text で描画された。
    UnresolvedTarget {
        /// 対象 fragment id。
        fragment_id: Symbol,
    },
    /// target-* が `max_target_iterations` 内に収束しなかったが、Consumer が
    /// `ExhaustionPolicy::BestEffort` を選択したため best-effort render された
    /// (round 6 review #5 対応)。
    TargetConvergenceExhausted {
        /// 尽くした iteration 数。
        iterations: u32,
    },
}

/// Consumer の convergence loop が `max_target_iterations` を尽くしたときの挙動
/// (round 6 review #5 対応、silent 続行を禁じる)。
///
/// Consumer 側 iteration に関する契約なので、raikiri の `plan()` / `render_*`
/// API 内では消費されない (Consumer が自身の loop で参照する)。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExhaustionPolicy {
    /// 未収束を error として上流に返す (保守的 default)。
    Error,
    /// 最後の registry で render、`WarningKind::TargetConvergenceExhausted` を
    /// 必ず `summary.warnings` に記録。
    BestEffort,
}

impl Default for ExhaustionPolicy {
    fn default() -> Self {
        Self::Error
    }
}

/// 未解決 target の詳細 (Finding #4 completion protocol)。
#[derive(Debug, Clone)]
pub struct UnresolvedTarget {
    /// slot 一意識別子。
    pub slot_id: TargetSlotId,
    /// 未解決の fragment id。
    pub fragment_id: Symbol,
    /// 未解決の理由。
    pub reason: UnresolvedReason,
}

/// UnresolvedTarget の理由 (Finding #4)。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// fragment id がどこにも定義されていない。
    NotFound,
    /// Consumer 側 policy でエラー扱い。
    ConsumerRejected,
    // ConvergenceFailed は削除 (raikiri 内 iteration しないため、Finding #5)
}

/// emit 済み target slot の詳細。
#[derive(Debug, Clone)]
pub struct EmittedSlotInfo {
    /// slot 一意識別子。
    pub slot_id: TargetSlotId,
    /// 対象 fragment id。
    pub fragment_id: Symbol,
    /// target 種別。
    pub kind: TargetKind,
}

/// slot の一意識別子 (Consumer が patch table の key に使う)。
///
/// (page_index, sequence) は decode 順で unique、byte-identical 保証あり
/// (Finding #4 completion protocol)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetSlotId {
    /// このページの 0-indexed page number。
    pub page_index: u32,
    /// ページ内での通し番号 (target-* 出現順、0-indexed)。
    pub sequence: u32,
}

/// target-* の種別 (target-counter / target-text / target-string 等)。
///
/// M4 target-* で variant を populate。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    // M4 で populate:
    //   Counter,
    //   Text,
    //   String,
    //   Element,
}

/// hint と actual の乖離を検知した項目 (Consumer 収束判定用、Finding #5)。
#[derive(Debug, Clone)]
pub struct TargetDiscrepancy {
    /// 対象 fragment id。
    pub fragment_id: Symbol,
    /// hint 段階で予告された page (無い場合は None)。
    pub hinted_page: Option<u32>,
    /// 実 render での page。
    pub actual_page: u32,
    /// hint 段階のテキスト content (無い場合は None)。
    pub hinted_text: Option<String>,
    /// 実 render のテキスト content。
    pub actual_text: String,
}

/// HTML parse 段階の error。M1.2 error propagation task で variant populate。
///
/// M1.1 では uninhabited。html5ever error / doctype mismatch / io error 等を
/// M1.2 で追加。
#[non_exhaustive]
#[derive(Debug)]
pub enum ParseError {
    // M1.2 で populate:
    //   Io(std::io::Error),
    //   Html5ever(String),
    //   ...
}

/// CSS parse / cascade 段階の error。M1.4 css-cascade-basic task で variant populate。
///
/// M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum CascadeError {
    // M1.4 で populate:
    //   Parse(String),
    //   InvalidValue(String),
    //   ...
}

/// Layout 段階の error。M1.6 layout-single-page task で variant populate。
///
/// M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug)]
pub enum LayoutError {
    // M1.6 で populate:
    //   TaffyError(String),
    //   ...
}
