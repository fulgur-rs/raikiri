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

// CSS cascade error taxonomy is owned by raikiri-style (Stylo pattern).
// Re-exported here so `RenderError::Cascade`
// and `impl From<CascadeError> for RenderError` below (which reference
// `CascadeError` by unqualified path) keep the same identity, and downstream
// consumers observing `raikiri_traits::CascadeError` (raikiri umbrella's
// re-export at `crates/raikiri/src/lib.rs:51`) are unchanged.
pub use raikiri_style::CascadeError;

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

    /// stub 段階の API に対する call。実装完了時にこの variant は
    /// **削除される** (breaking change として release notes に明記)。Consumer
    /// は stub 期間中のみ pattern match し、実装完了時に arm 削除でよい。
    /// `feature` は呼ばれた stub API の識別 (`"plan"`, `"render_streaming"` 等)。
    Unimplemented {
        /// stub の API 名。
        feature: &'static str,
        /// Consumer 向け migration hint。
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
            Self::Sink(_) => write!(f, "Sink returned I/O error"),
            Self::Configuration(msg) => write!(f, "Configuration error: {msg}"),
            Self::TargetDidNotConverge { iterations } => {
                write!(f, "target-* did not converge in {iterations} iterations")
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
            Self::Policy(v) => Some(v),
            Self::Sink(e) | Self::Io(e) => Some(e),
            Self::LimitExceeded { .. }
            | Self::Configuration(_)
            | Self::TargetDidNotConverge { .. }
            | Self::Unimplemented { .. } => None,
        }
    }
}

impl From<ParseError> for RenderError {
    fn from(e: ParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<CascadeError> for RenderError {
    fn from(e: CascadeError) -> Self {
        Self::Cascade(e)
    }
}

impl From<LayoutError> for RenderError {
    fn from(e: LayoutError) -> Self {
        match e {
            LayoutError::Resolver(re) => Self::Resolver(re),
            other => Self::Layout(other),
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
    /// `max_aggregate_bytes` 超過 (post-parse の approximate memory footprint、
    /// DOM node arena / cascade table 等の合計)。
    AggregateBytes,
    /// `max_input_bytes` 超過。
    ///
    /// [`AggregateBytes`](Self::AggregateBytes) との semantic 分離: `InputBytes`
    /// は **parse-time** の raw input byte stream を pin する fail-closed 早期
    /// 返却用 (`parse_html_with_limits` が read 段階で enforce)。`AggregateBytes`
    /// は **post-parse** の approximate memory footprint。ゆえに attacker が
    /// 巨大 HTML を送りつけて OOM を誘発する DoS 対策としては `InputBytes` の
    /// 方が直接的。
    InputBytes,
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
#[derive(Debug, Clone)]
pub struct RenderWarning {
    /// 警告 kind。
    pub kind: WarningKind,
    /// 関連 DOM node (option、element-level warning に付く)。
    pub node_id: Option<NodeId>,
    /// 人間可読な詳細。
    pub details: String,
}

/// 警告 kind。§4 の 5 base variant + 追加された `HtmlParseError`。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum WarningKind {
    /// Consumer の resolver が fallback を返した (`Ok(fallback_intrinsic)`)。
    ResolverFallback {
        /// 対象 fragment id。
        fragment_id: Symbol,
    },
    /// この URL の resource について、意図した完全な fetch が得られず degrade
    /// した。二種類の disposition を一つの variant に包含する:
    ///
    /// - Consumer の network provider が明示的に代替 content を返した場合
    ///   (`ResolverFallback` と同様の Ok-disposition。ただし
    ///   `NetworkProvider::fetch` の戻り値には現時点で disposition channel
    ///   が無く、この経路は未使用)。
    /// - fetch 自体が `Err` になり (timeout / I/O error / HTTP error 等)、
    ///   呼び出し側がそれを fatal にせず該当 resource なしで処理を継続した
    ///   場合。この場合 content は一切適用されていない。
    ///
    /// 現時点の実装で実際に生成されるのは Err-disposition のみ
    /// (Ok-disposition は channel 自体が未実装のため到達不能)。それでも
    /// **consumer はこの variant だけから「何らかの content が適用された」
    /// と推論してはならない**。ただし `RenderWarning::details` は人間可読な
    /// 自由記述であり、disposition を判別するための構造化された contract
    /// ではない — 現状 details に "fetch failed" 等 Err らしく読める文字列
    /// が入っているのは呼び出し側 (`raikiri-html`) が手で書いているからに
    /// 過ぎず、この variant 自体が保証する形式ではない。将来
    /// Ok-disposition が実装されこの variant から両方の disposition が
    /// 実際に生成されるようになる時点で、判別手段 (details を構造化した
    /// contract にする、または別 variant に分離する) を別途設計する必要が
    /// ある。
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
    /// html5ever tokenizer が非致命 parse error を報告した (malformed HTML を
    /// recover した場合等)。Stylo/blitz と同じ責務境界: raikiri-html 内で
    /// warning に降格し、rendering は継続する。orchestrator が Document →
    /// `RenderSummary.warnings` に merge する。
    HtmlParseError {
        /// html5ever が返した診断メッセージ (Cow<'static, str> を String 化)。
        message: String,
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
/// target-* 対応時に variant を populate。現時点では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    // 将来 populate 予定:
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

/// HTML parse 段階の terminal error (raikiri-html crate 内で発生)。
///
/// **Stylo/blitz と同じ責務境界**: html5ever tokenizer 由来の non-fatal
/// parse error (recoverable な malformed HTML) は raikiri-html 内で
/// [`RenderWarning`] として summary に集約し、この enum には含めない。
/// この enum の variant は rendering を halt させる真の terminal error のみ。
///
/// `Io` / `Encoding` の 2 variant を populate 済み。html5ever 固有の
/// error variant は raikiri-html 側で crate-private に扱い、必要になった
/// 時点で `#[non_exhaustive]` の恩恵で追加する。
#[non_exhaustive]
#[derive(Debug)]
pub enum ParseError {
    /// Input source (`std::io::Read`) からの read 失敗。
    Io(std::io::Error),
    /// Byte stream → text の変換に失敗 (encoding label 検出 or 変換 error)。
    Encoding {
        /// 検出または指定された encoding label (例: "utf-8", "shift_jis")。
        label: String,
        /// 失敗理由の人間可読な description。
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

/// Layout 段階の terminal error (raikiri-dom + taffy が発生源)。
///
/// **同じ責務境界**: taffy 固有 error 型は raikiri-dom 内部に閉じ込め、
/// この enum は raikiri-dom が明示的に fail-hard を選択した場合の signal のみ。
///
#[non_exhaustive]
#[derive(Debug)]
pub enum LayoutError {
    /// raikiri-dom 内で回復不能な内部 error が発生した (taffy internal
    /// error 等)。詳細メッセージは raikiri-dom 内部で log + message として構成。
    Internal {
        /// 人間可読な失敗詳細 (raikiri-dom 内部で構成)。
        message: String,
    },
    /// `<img>` 等 replaced element の resolve が失敗した
    /// (`ReplacedResolver::resolve` が `Err` を返した)。
    Resolver(ResolverError),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal { message } => write!(f, "Layout internal error: {message}"),
            Self::Resolver(e) => write!(f, "Layout resolver error: {e}"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
mod unimplemented_variant_tests {
    use super::*;

    #[test]
    fn unimplemented_display_includes_feature_and_hint() {
        let err = RenderError::Unimplemented {
            feature: "plan",
            migration_hint: "pagination 実装後に populate予定",
        };
        let s = format!("{err}");
        assert!(
            s.contains("plan"),
            "display must include feature: got {s:?}"
        );
        assert!(
            s.contains("pagination"),
            "display must include hint: got {s:?}"
        );
        assert!(
            s.contains("not implemented"),
            "display must include 'not implemented': got {s:?}"
        );
    }

    #[test]
    fn unimplemented_source_is_none() {
        use std::error::Error;
        let err = RenderError::Unimplemented {
            feature: "render_streaming",
            migration_hint: "hint",
        };
        assert!(err.source().is_none(), "Unimplemented has no inner cause");
    }

    #[test]
    fn layout_error_resolver_variant_converts_to_render_error_resolver() {
        let re = ResolverError::Decode("bad PNG".into());
        let le = LayoutError::Resolver(re);
        let render_err: RenderError = le.into();
        assert!(matches!(render_err, RenderError::Resolver(_)));
    }
}
