//! Page-related neutral model types.
//!
//! ここに集めた型は §5 (Pipeline), §7 (GCPM), §9 (PageBox), §11 (Paint) が
//! authoritative なので、M1.1 では opaque placeholder として置き、後続の
//! task が field / method を段階的に populate する。
//!
//! 入力/構築対象の struct は `#[non_exhaustive]` + `impl Default` + `pub fn new()` を持ち、
//! external consumer crate から `X::new()` / `X::default()` で construct 可能
//! ([`TargetInfo`] は `TargetRegistry::register` の input として consumer 側で構築)。
//! Output-only snapshot 型 (現状 [`PendingResolution`] のみ — registry 内部で
//! populate されて API 返り値経由で consumer に届くのみ) は consumer 側で直接
//! construct しないため Default / new を要件外とする (raikiri-spike-bsi Option C
//! wall/traits merge で確立、`#[non_exhaustive]` は全 public struct に維持)。

mod target;

pub use target::{
    PendingResolution, ResolveOutcome, TargetInfo, TargetRegistry, resolve_content_component,
};

use crate::dom::{NodeId, Symbol};
use raikiri_style::property::{
    ContentComponent, ContentPart, ContentTextKeyword, CounterStyle, StringFetchMode,
};
#[cfg(test)]
use smol_str::SmolStr;
use url::Url;

/// PageFragment — 1 ページの painted output (glyph run / decoration / target slot 含む)。
/// M1.7 paint-basic + M2 pagestream で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageFragment {
    // M1.7 / M2 で populate:
    //   pub page_index: u32,
    //   pub page_box: PageBox,
    //   pub items: Vec<PaintedBoxItem>,
    //   pub target_slots: Vec<TargetSlot>,
    //   ...
}

impl PageFragment {
    /// Construct an empty PageFragment. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// PageBox — @page rule 解決結果 (size, margins, margin box slots)。
/// M1.6 layout-single-page で width / height + `A4` / `US_LETTER` const を populate。
/// margins / margin_boxes は M4 で populate。
///
/// **単位 = CSS px** (1 CSS px = 1/96 in in print context per CSS Values L4 §6.2
/// "Absolute Lengths" <https://www.w3.org/TR/css-values-4/#absolute-lengths>)。
/// pt / mm / in への換算は Consumer 責務。
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageBox {
    /// Page 幅 (CSS px)。
    pub width: f32,
    /// Page 高 (CSS px)。
    pub height: f32,
    // M4 で populate:
    //   pub margins: Margins,
    //   pub margin_boxes: [Option<MarginBox>; 16],
}

impl PageBox {
    /// A4 portrait: 210×297 mm = **793.70 × 1122.52 px** (@ 96 DPI anchor)。
    /// CSS Paged Media Level 3 §7 default size。
    pub const A4: PageBox = PageBox {
        width: 793.7008,   // 210mm × 96/25.4
        height: 1122.5197, // 297mm × 96/25.4
    };

    /// US Letter portrait: 8.5×11 in = **816 × 1056 px** ちょうど。
    pub const US_LETTER: PageBox = PageBox {
        width: 816.0,
        height: 1056.0,
    };

    /// Construct a `PageBox` = `A4`。`#[non_exhaustive]` の下でも安定した
    /// zero-arg constructor を残すため保持。
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for PageBox {
    fn default() -> Self {
        Self::A4
    }
}

/// Consumer が render 開始時に渡す page-level default 値。M1 は paper size
/// のみを持つ最小 shape。M2+ で margin / orientation / named pages 等を追加予定。
///
/// 全 field は CSS px 単位 (`PageBox` 参照)。pt/mm/in 換算は Consumer 責務。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageDefaults {
    /// Default paper サイズ (`@page size` で override しない場合の initial value)。
    /// 既定 = A4。
    pub page_box: PageBox,
}

impl PageDefaults {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PageDefaultsBuilder {
        PageDefaultsBuilder::default()
    }
}

/// `PageDefaults` の fluent builder。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct PageDefaultsBuilder {
    page_box: Option<PageBox>,
}

impl PageDefaultsBuilder {
    /// `page_box` を設定。
    pub fn page_box(mut self, v: PageBox) -> Self {
        self.page_box = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PageDefaults {
        PageDefaults {
            page_box: self.page_box.unwrap_or_default(),
        }
    }
}

/// PageContext — GCPM runtime state (counter tree, named string 4-snapshot,
/// running bindings)。M4 GCPM で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageContext {
    // M4 で populate。
}

impl PageContext {
    /// Construct an empty PageContext. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// LayoutBuffer — widow / orphan / break-inside / container probe lookahead
/// buffer の中立モデル。実装は raikiri-dom 側 (§5 参照)。
/// M2 layoutbuffer-skeleton で populate。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LayoutBuffer {
    // M2 で populate。
}

impl LayoutBuffer {
    /// Construct an empty LayoutBuffer. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

// `TargetRegistry` — target-* placeholder emit + resolve の runtime registry。
// design doc §7.2 canonical shape、raikiri-spike-bsi (Sprint 15 dom-3 Wave 1)
// で raikiri-dom `pub(crate)` shadow を Option C で本 crate に merge、canonical
// impl は sibling `target` submodule。この module では re-export のみ。

/// RunningTemplate — `position: running(name)` の template 登録。
/// M4 で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RunningTemplate {
    // M4 で populate。
}

impl RunningTemplate {
    /// Construct an empty RunningTemplate. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// FormData — application/x-www-form-urlencoded body の中立モデル。
/// Consumer 側 network 実装で参照 (Body::Form(FormData))。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FormData {
    // 将来 populate:
    //   pub pairs: Vec<(String, String)>,
}

impl FormData {
    /// Construct an empty FormData. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Stable identifier for a running-element template registered via
/// `position: running(name)`.
///
/// design doc §7.0 line 1903-1905 "shared types → raikiri-traits" scope。
/// [`GcpmDirective::RegisterRunning`] (§7.1 line 1918) が field で参照する。
///
/// Wraps a subtree-root [`NodeId`]。`position: running(name)` された element の
/// subtree root は arena 内で per-element unique なので、そのまま stable な
/// per-template key として使える (canonical: 内 raikiri-dom
/// `RunningTemplateStore` の keying rationale と同 — raikiri-spike-96u.3 で
/// pinned)。newtype で `NodeId` の他用途との mix を防ぐ。
///
/// (raikiri-spike-96u.4 populate; wall/traits.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunningTemplateId(pub NodeId);

impl RunningTemplateId {
    /// Construct from a subtree-root [`NodeId`] (canonical: per-element unique).
    pub fn new(subtree_root: NodeId) -> Self {
        Self(subtree_root)
    }
}

/// Resolved `<content-list>` value used as the source of a `string-set` snapshot.
///
/// design doc §7.1 line 1917 `StringSet { name: Symbol, source: ContentSource }`。
/// raikiri-style cascade が `string-set: name <content-list>` declaration を
/// resolve し、raikiri-dom Phase B walk が §2.7.2 の 4-snapshot mode
/// (start / first / last / first-except) で [`PageContext`] named-string state
/// にコピーする際の source shape。
///
/// M6+ 側で consumer (raikiri-style bridge) が [`Vec<ContentValueItem>`] から
/// 構築する。`items` は public field で `..Default::default()` の struct-update
/// syntax でも construct 可 (sibling [`PageBox`] pattern)。
///
/// (raikiri-spike-96u.4 populate; wall/traits.)
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentSource {
    /// Ordered resolved items making up the content-list.
    pub items: Vec<ContentValueItem>,
}

impl ContentSource {
    /// Construct from a resolved item list.
    pub fn new(items: Vec<ContentValueItem>) -> Self {
        Self { items }
    }
}

/// GCPM producing directive emitted by raikiri-style cascade
/// (`counter-increment` / `counter-reset` / `counter-set` / `string-set` /
/// `position: running(name)` 等)、raikiri-dom Phase B walk が [`PageContext`]
/// と [`TargetRegistry`] を更新するために消費する (design doc §7.1 line
/// 1907-1920 — "Producing directive"、raikiri-style emit → raikiri-dom apply)。
///
/// **Producing side** of §7 GCPM: Phase B walk は running counter tree /
/// named-string 4-snapshot / running bindings / TargetRegistry を mutate する。
///
/// **`#[non_exhaustive]` semantics** — sibling [`ContentValueItem`] と同じ
/// forward-compat 契約: enum-level `#[non_exhaustive]` は downstream `match`
/// に `_ =>` arm を強制するが、既存 variant の tuple/struct constructor 呼び出しは
/// block しない。既存 variant の payload **type** 変更は downstream の
/// constructor を compile-break させる。
///
/// (raikiri-spike-96u.4 populate; wall/traits — M1.1〜M5 の uninhabited placeholder
/// を design doc §7.1 line 1913-1920 の canonical 6 variant に置き換え。)
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcpmDirective {
    /// `counter-increment: name delta` — 指定 counter `name` を `delta` だけ増分
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-increment>)。
    CounterIncrement {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Increment amount (spec default 1、`counter-increment: name -3` で負値)。
        delta: i32,
    },
    /// `counter-reset: name value` — 指定 counter `name` を `value` に reset
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#propdef-counter-reset>)。
    CounterReset {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Reset value (spec default 0)。
        value: i32,
    },
    /// `counter-set: name value` — 現要素で counter `name` を `value` に set
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-set>)。
    CounterSet {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Set value (spec default 0)。
        value: i32,
    },
    /// `string-set: name <content-list>` — resolve 済 content-list を named-string
    /// `name` の 4-snapshot state (start / first / last / first-except) に snapshot
    /// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
    StringSet {
        /// Named-string identifier。
        name: Symbol,
        /// Resolved source content-list ([`ContentSource`])。
        source: ContentSource,
    },
    /// `position: running(name)` — 現 subtree を running template として `name`
    /// 下に登録し、`template_id` (subtree root wrapped in [`RunningTemplateId`])
    /// を key として [`RunningTemplate`] pool に格納
    /// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)。
    RegisterRunning {
        /// Running-template name (custom-ident)。
        name: Symbol,
        /// Template identifier (subtree root wrapped)。
        template_id: RunningTemplateId,
    },
    /// In-document fragment id を [`TargetRegistry`] に登録
    /// (`target-counter()` / `target-counters()` / `target-text()` の後続解決用)
    /// (design doc §7.2 TargetRegistry)。
    RegisterTarget {
        /// Fragment id (通常は element `id` attribute 値の Symbol view)。
        fragment_id: Symbol,
    },
}

/// resolved `content` property の item — [`GcpmDirective`] の "consuming"
/// counterpart (design doc §7.1 line 1923-1937 — "Consuming directive"、
/// raikiri-style cascade emit → raikiri-dom paint-time resolve で concrete
/// string に変換)。
///
/// **`#[non_exhaustive]` semantics** — sibling [`GcpmDirective`] および
/// [`raikiri_style::property::ContentComponent`] と同じ forward-compat 契約。
///
/// **`TargetCounters::sep`** — design doc §7.1 line 1935 の field 名は `sep`。
///   [`raikiri_style::property::ContentComponent::TargetCounters::separator`]
///   は `separator`。design doc に verbatim 従い `sep` を使う。[`TryFrom`] impl
///   で `separator → sep` を map。
///
/// # `Element` variant — [`TryFrom<ContentComponent>`] impl gap
///
/// `ContentValueItem::Element` は design doc §7.1 line 1931 の
/// canonical variant だが、`ContentComponent::Element` は raikiri-style に
/// 未実装 (spinout: bd raikiri-spike-6z0 scope/css-engine)。ゆえに [`TryFrom`]
/// impl 経路では現在到達不能で、raikiri-dom 内部の running-template pipeline
/// producer が直接 construct する。6z0 land 後に bridge arm を追加。
///
/// (raikiri-spike-96u.4 populate; wall/traits — M1.1〜M5 の uninhabited placeholder
/// を design doc §7.1 line 1926-1937 の canonical 10 variant に置き換え。)
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueItem {
    /// bare `<string>` literal (`content: "hello"`)。
    Literal(String),
    /// `counter(name, style?)` (CSS Lists 3 §4.7
    /// <https://www.w3.org/TR/css-lists-3/#counter-functions>)。
    Counter {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `counters(name, separator, style?)` (CSS Lists 3 §4.7)。
    Counters {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Separator string (nested counter stack join)。
        separator: String,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `string(name, mode?)` (CSS Content 3 §2.7.2
    /// <https://www.w3.org/TR/css-content-3/#string-function>)。
    String {
        /// Named-string identifier。
        name: Symbol,
        /// Fetch mode (spec default `first`)。
        fetch: StringFetchMode,
    },
    /// `element(name)` (CSS GCPM 3 §1.2.2
    /// <https://www.w3.org/TR/css-gcpm-3/#element-syntax>)。runtime resolve は
    /// raikiri-dom の [`RunningTemplate`] pool 引きから。
    ///
    /// **From-impl gap**: `ContentComponent::Element` は未実装 (bd raikiri-spike-6z0)。
    /// See type-level docstring "Element variant" section。
    Element {
        /// Running-template name (custom-ident)。
        name: Symbol,
    },
    /// `content(part?)` (CSS GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>)。keyword 省略時は
    /// [`ContentPart::Content`] (spec default)。
    Content {
        /// Element の string value のどの部分を挿入するか。
        part: ContentPart,
    },
    /// `attr(name)` (CSS Content 3 §2.1
    /// <https://www.w3.org/TR/css-content-3/#strings>)。
    Attr {
        /// Attribute name (null-namespace、CSS Content 3 §2.1)。
        name: Symbol,
    },
    /// `target-counter(url, name, style?)` (CSS Content 3 §2.6.1
    /// <https://www.w3.org/TR/css-content-3/#target-counter>)。
    TargetCounter {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `target-counters(url, name, separator, style?)` (CSS Content 3 §2.6.2
    /// <https://www.w3.org/TR/css-content-3/#target-counters>)。
    ///
    /// field 名は design doc §7.1 line 1935 に verbatim (`sep`)。
    TargetCounters {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Separator string (design doc verbatim `sep`)。
        sep: String,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `target-text(url, part?)` (CSS Content 3 §2.6.3
    /// <https://www.w3.org/TR/css-content-3/#target-text>)。
    TargetText {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// どの部分を挿入するか (`content` は要素の string value 全体を指す
        /// keyword)。
        part: ContentPart,
    },
}

/// [`TryFrom<ContentComponent> for ContentValueItem`] の failure taxonomy。
///
/// raikiri-style は `url` crate に依存しない leaf crate なので target-* variant
/// の URL は raw [`String`] で保持され、bridge 側で [`Url::parse`] する。
/// parse 失敗と、raikiri-style 側に新 variant が landing した際の
/// forward-compat 検知の 2 case を分離する。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueConvertError {
    /// target-* variant の URL 文字列が [`Url::parse`] で fail した。
    InvalidUrl(url::ParseError),
    /// raikiri-style [`ContentComponent`] に本 crate 未対応 variant が新規追加された
    /// (cross-crate `#[non_exhaustive]` の catch-all)。
    ///
    /// stable Rust では cross-crate `#[non_exhaustive]` に対して compile-time
    /// 全 variant enforce できないため、runtime error でハンドリング。raikiri-style
    /// 側追加時に本 crate の [`TryFrom`] arm を extend する discipline。
    UnsupportedVariant,
}

impl core::fmt::Display for ContentValueConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "invalid URL in target-* content component: {e}"),
            Self::UnsupportedVariant => f.write_str(
                "unsupported ContentComponent variant (raikiri-style ahead of raikiri-traits, extend TryFrom arms)",
            ),
        }
    }
}

impl std::error::Error for ContentValueConvertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUrl(e) => Some(e),
            Self::UnsupportedVariant => None,
        }
    }
}

impl From<url::ParseError> for ContentValueConvertError {
    fn from(e: url::ParseError) -> Self {
        Self::InvalidUrl(e)
    }
}

/// [`ContentTextKeyword`] → [`ContentPart`] mapping (`content(keyword)` の
/// keyword を bridge)。keyword 集合は spec spelling が異なる: GCPM 3 §1.1.1.1
/// の `content()` と CSS Content 3 §2.6.3 の `target-text()` は同じ概念に
/// 異なる keyword spelling を当てている (`text` vs `content`)。両者を
/// [`ContentPart`] 側の value 集合に集約するときは意味の対応で紐付ける
/// (spec の "default" 宣言には依らない — CSS Content 3 §2.6.3 は
/// `target-text()` の第 2 引数省略時の値を規定していない。GCPM 3 §1.1.1.1
/// は `text` を "the default value" と述べているが、同 section は grammar に
/// `?` が無く [第 2 引数が構文上 optional でない]、かつ "default をどう
/// 定義するか" 自体が未解決の WG issue として残っており、TR 上安定した根拠
/// ではない):
/// - [`ContentTextKeyword::Text`] → [`ContentPart::Content`] (どちらも
///   「要素の string value 全体」を指す)
/// - [`ContentTextKeyword::Before`] → [`ContentPart::Before`]
/// - [`ContentTextKeyword::After`] → [`ContentPart::After`]
/// - [`ContentTextKeyword::FirstLetter`] → [`ContentPart::FirstLetter`]
fn content_text_keyword_to_content_part(
    kw: ContentTextKeyword,
) -> Result<ContentPart, ContentValueConvertError> {
    Ok(match kw {
        ContentTextKeyword::Text => ContentPart::Content,
        ContentTextKeyword::Before => ContentPart::Before,
        ContentTextKeyword::After => ContentPart::After,
        ContentTextKeyword::FirstLetter => ContentPart::FirstLetter,
        // cov:ignore: cross-crate `#[non_exhaustive]` catch-all — stable Rust
        // requires the `_` arm for exhaustive matching on ContentTextKeyword
        // defined in raikiri-style; unreachable until raikiri-style adds a
        // new variant. Treat it like any other unsupported cross-crate variant
        // rather than silently changing its semantics to the spec default.
        _ => return Err(ContentValueConvertError::UnsupportedVariant),
    })
}

impl TryFrom<ContentComponent> for ContentValueItem {
    type Error = ContentValueConvertError;

    /// [`raikiri_style::property::ContentComponent`] → [`ContentValueItem`]
    /// canonical taxonomy 変換 (raikiri-spike-376 amended)。
    ///
    /// 変換対象 9 variant (Element は raikiri-style 側未実装、bd raikiri-spike-6z0
    /// で raikiri-style 側に生えたら arm 追加):
    ///
    /// - [`ContentComponent::Literal`] → [`ContentValueItem::Literal`] (`String` に変換)
    /// - [`ContentComponent::Counter`] → [`ContentValueItem::Counter`]
    ///   (`name: SmolStr` → [`Symbol::new`])
    /// - [`ContentComponent::Counters`] → [`ContentValueItem::Counters`]
    /// - [`ContentComponent::String`] → [`ContentValueItem::String`]
    /// - [`ContentComponent::Attr`] → [`ContentValueItem::Attr`]
    /// - [`ContentComponent::TargetCounter`] → [`ContentValueItem::TargetCounter`]
    ///   (URL は [`Url::parse`]、失敗時 [`ContentValueConvertError::InvalidUrl`])
    /// - [`ContentComponent::TargetCounters`] → [`ContentValueItem::TargetCounters`]
    ///   (`separator` → `sep` field 名変換、design doc §7.1 line 1935 verbatim)
    /// - [`ContentComponent::TargetText`] → [`ContentValueItem::TargetText`]
    /// - [`ContentComponent::Content`] → [`ContentValueItem::Content`]
    ///   ([`ContentTextKeyword`] → [`ContentPart`] mapping)
    fn try_from(cc: ContentComponent) -> Result<Self, Self::Error> {
        Ok(match cc {
            ContentComponent::Literal(s) => Self::Literal(s.into()),
            ContentComponent::Counter { name, style } => Self::Counter {
                name: Symbol::new(name),
                style,
            },
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => Self::Counters {
                name: Symbol::new(name),
                separator,
                style,
            },
            ContentComponent::String { name, fetch } => Self::String {
                name: Symbol::new(name),
                fetch,
            },
            ContentComponent::Attr { name } => Self::Attr {
                name: Symbol::new(name),
            },
            ContentComponent::TargetCounter { url, name, style } => Self::TargetCounter {
                url: Url::parse(&url)?,
                name: Symbol::new(name),
                style,
            },
            ContentComponent::TargetCounters {
                url,
                name,
                separator,
                style,
            } => Self::TargetCounters {
                url: Url::parse(&url)?,
                name: Symbol::new(name),
                sep: separator,
                style,
            },
            ContentComponent::TargetText { url, part } => Self::TargetText {
                url: Url::parse(&url)?,
                part,
            },
            ContentComponent::Content { keyword } => Self::Content {
                part: content_text_keyword_to_content_part(keyword)?,
            },
            // cov:ignore: cross-crate `#[non_exhaustive]` catch-all — stable
            // Rust requires the `_` arm for exhaustive matching on
            // ContentComponent defined in raikiri-style; unreachable until
            // raikiri-style adds a variant this crate has not yet mirrored
            // (e.g. bd raikiri-spike-6z0 will land `ContentComponent::Element`
            // and this arm's coverage window opens for one iter). Reporting
            // `UnsupportedVariant` at runtime is the fail-closed discipline
            // for raikiri-style landing ahead of raikiri-traits — see
            // type-level docstring "Element variant" section.
            _ => return Err(ContentValueConvertError::UnsupportedVariant),
        })
    }
}

#[cfg(test)]
mod pagebox_px_baseline_tests {
    use super::*;

    #[test]
    fn a4_dimensions_match_css_px_conversion() {
        // 210mm × 297mm を CSS px (1/96 in) 換算:
        //   width  = 210mm × 96/25.4 ≈ 793.7008
        //   height = 297mm × 96/25.4 ≈ 1122.5197
        assert!(
            (PageBox::A4.width - 793.7008).abs() < 0.001,
            "A4.width should be ~793.7008 px, got {}",
            PageBox::A4.width
        );
        assert!(
            (PageBox::A4.height - 1122.5197).abs() < 0.001,
            "A4.height should be ~1122.5197 px, got {}",
            PageBox::A4.height
        );
    }

    #[test]
    fn us_letter_dimensions_match_exact_integers() {
        // 8.5in × 11in @ 96 DPI = 816 × 1056 px exactly
        assert_eq!(PageBox::US_LETTER.width, 816.0);
        assert_eq!(PageBox::US_LETTER.height, 1056.0);
    }

    #[test]
    fn pagebox_default_is_a4() {
        assert_eq!(PageBox::default(), PageBox::A4);
    }
}

#[cfg(test)]
mod pagedefaults_tests {
    use super::*;

    #[test]
    fn pagedefaults_default_uses_a4() {
        let d = PageDefaults::default();
        assert_eq!(d.page_box, PageBox::A4);
    }

    #[test]
    fn pagedefaults_new_is_default() {
        assert_eq!(
            PageDefaults::new().page_box,
            PageDefaults::default().page_box
        );
    }

    #[test]
    fn pagedefaults_builder_sets_page_box() {
        let d = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
        assert_eq!(d.page_box, PageBox::US_LETTER);
    }

    #[test]
    fn pagedefaults_builder_default_matches_pagedefaults_default() {
        let via_builder = PageDefaults::builder().build();
        let via_default = PageDefaults::default();
        assert_eq!(via_builder.page_box, via_default.page_box);
    }
}

#[cfg(test)]
mod gcpm_directive_populate_tests {
    //! GcpmDirective canonical 6 variant construction pins (raikiri-spike-96u.4)。
    //!
    //! design doc §7.1 line 1913-1920 verbatim shape。variant 追加 / rename /
    //! payload type 変更で fail、`#[non_exhaustive]` catch-all は無し
    //! (crate-local match は non_exhaustive の enforce 外)。

    use super::*;

    #[test]
    fn counter_increment_construct_and_payload_visible() {
        let d = GcpmDirective::CounterIncrement {
            name: Symbol::new("chapter"),
            delta: 1,
        };
        match d {
            GcpmDirective::CounterIncrement { name, delta } => {
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(delta, 1);
            }
            other => panic!("expected CounterIncrement, got {other:?}"),
        }
    }

    #[test]
    fn counter_reset_construct_negative_value() {
        let d = GcpmDirective::CounterReset {
            name: Symbol::new("section"),
            value: -3,
        };
        match d {
            GcpmDirective::CounterReset { name, value } => {
                assert_eq!(name.as_str(), "section");
                assert_eq!(value, -3);
            }
            other => panic!("expected CounterReset, got {other:?}"),
        }
    }

    #[test]
    fn counter_set_construct_zero_value() {
        let d = GcpmDirective::CounterSet {
            name: Symbol::new("page"),
            value: 0,
        };
        match d {
            GcpmDirective::CounterSet { name, value } => {
                assert_eq!(name.as_str(), "page");
                assert_eq!(value, 0);
            }
            other => panic!("expected CounterSet, got {other:?}"),
        }
    }

    #[test]
    fn string_set_carries_content_source_items() {
        let source = ContentSource::new(vec![ContentValueItem::Literal(String::from("Ch. "))]);
        let d = GcpmDirective::StringSet {
            name: Symbol::new("heading"),
            source: source.clone(),
        };
        match d {
            GcpmDirective::StringSet { name, source: s } => {
                assert_eq!(name.as_str(), "heading");
                assert_eq!(s, source);
                assert_eq!(s.items.len(), 1);
            }
            other => panic!("expected StringSet, got {other:?}"),
        }
    }

    #[test]
    fn register_running_carries_template_id() {
        let template_id = RunningTemplateId::new(NodeId::new(42));
        let d = GcpmDirective::RegisterRunning {
            name: Symbol::new("header"),
            template_id,
        };
        match d {
            GcpmDirective::RegisterRunning {
                name,
                template_id: tid,
            } => {
                assert_eq!(name.as_str(), "header");
                assert_eq!(tid, template_id);
                assert_eq!(tid.0, NodeId::new(42));
            }
            other => panic!("expected RegisterRunning, got {other:?}"),
        }
    }

    #[test]
    fn register_target_carries_fragment_id() {
        let d = GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        };
        match d {
            GcpmDirective::RegisterTarget { fragment_id } => {
                assert_eq!(fragment_id.as_str(), "intro");
            }
            other => panic!("expected RegisterTarget, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod content_value_item_populate_tests {
    //! ContentValueItem canonical 10 variant construction pins (raikiri-spike-96u.4)。
    //!
    //! design doc §7.1 line 1926-1937 verbatim shape。

    use super::*;

    #[test]
    fn literal_string_payload() {
        let c = ContentValueItem::Literal(String::from("hello"));
        match c {
            ContentValueItem::Literal(s) => {
                assert_eq!(s.as_str(), "hello");
            }
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn counter_default_style_is_decimal() {
        let c = ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::Counter { name, style } => {
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected Counter, got {other:?}"),
        }
    }

    #[test]
    fn counters_separator_payload() {
        let c = ContentValueItem::Counters {
            name: Symbol::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::Counters {
                name,
                separator,
                style,
            } => {
                assert_eq!(name.as_str(), "section");
                assert_eq!(separator, ".");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected Counters, got {other:?}"),
        }
    }

    #[test]
    fn string_default_fetch_mode_is_first() {
        let c = ContentValueItem::String {
            name: Symbol::new("running-title"),
            fetch: StringFetchMode::default(),
        };
        match c {
            ContentValueItem::String { name, fetch } => {
                assert_eq!(name.as_str(), "running-title");
                assert_eq!(fetch, StringFetchMode::First);
            }
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn element_variant_construct() {
        let c = ContentValueItem::Element {
            name: Symbol::new("header"),
        };
        match c {
            ContentValueItem::Element { name } => {
                assert_eq!(name.as_str(), "header");
            }
            other => panic!("expected Element, got {other:?}"),
        }
    }

    #[test]
    fn content_default_part_is_content() {
        let c = ContentValueItem::Content {
            part: ContentPart::default(),
        };
        match c {
            ContentValueItem::Content { part } => {
                assert_eq!(part, ContentPart::Content);
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn attr_variant_construct() {
        let c = ContentValueItem::Attr {
            name: Symbol::new("data-title"),
        };
        match c {
            ContentValueItem::Attr { name } => {
                assert_eq!(name.as_str(), "data-title");
            }
            other => panic!("expected Attr, got {other:?}"),
        }
    }

    #[test]
    fn target_counter_url_payload() {
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetCounter {
            url: url.clone(),
            name: Symbol::new("chapter"),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::TargetCounter {
                url: u,
                name,
                style,
            } => {
                assert_eq!(u, url);
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected TargetCounter, got {other:?}"),
        }
    }

    #[test]
    fn target_counters_sep_field_name_matches_design_doc() {
        // design doc §7.1 line 1935: `sep: String` (not `separator`)。
        // Regression pin for the deliberate field-name deviation from
        // ContentComponent::TargetCounters (which uses `separator`).
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetCounters {
            url: url.clone(),
            name: Symbol::new("section"),
            sep: String::from("."),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::TargetCounters {
                url: u,
                name,
                sep,
                style,
            } => {
                assert_eq!(u, url);
                assert_eq!(name.as_str(), "section");
                assert_eq!(sep, ".");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected TargetCounters, got {other:?}"),
        }
    }

    #[test]
    fn target_text_default_part_is_content() {
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetText {
            url: url.clone(),
            part: ContentPart::default(),
        };
        match c {
            ContentValueItem::TargetText { url: u, part } => {
                assert_eq!(u, url);
                assert_eq!(part, ContentPart::Content);
            }
            other => panic!("expected TargetText, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod content_component_bridge_tests {
    //! [`TryFrom<ContentComponent> for ContentValueItem`] roundtrip pins
    //! (raikiri-spike-96u.4、canonical taxonomy conversion per raikiri-spike-376
    //! amended)。
    //!
    //! Coverage: 9 of 10 [`ContentValueItem`] variants — [`Element`] は
    //! [`ContentComponent::Element`] 未実装 (bd raikiri-spike-6z0) のため
    //! bridge 経路では現在到達不能。variant 追加時に arm を extend する。

    use super::*;

    #[test]
    fn literal_bridge_converts_smol_str_to_string() {
        let cc = ContentComponent::Literal(SmolStr::new("hello"));
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Literal");
        assert_eq!(cvi, ContentValueItem::Literal(String::from("hello")));
    }

    #[test]
    fn counter_bridge_wraps_name_in_symbol() {
        let cc = ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Counter");
        assert_eq!(
            cvi,
            ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn counters_bridge_preserves_separator_field_name() {
        // ContentComponent::Counters は `separator`、ContentValueItem::Counters も
        // `separator` (design doc §7.1 line 1929 verbatim)。両者同名なので
        // straight mapping。
        let cc = ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Counters");
        assert_eq!(
            cvi,
            ContentValueItem::Counters {
                name: Symbol::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn string_bridge_preserves_fetch_mode() {
        let cc = ContentComponent::String {
            name: SmolStr::new("title"),
            fetch: StringFetchMode::Last,
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for String");
        assert_eq!(
            cvi,
            ContentValueItem::String {
                name: Symbol::new("title"),
                fetch: StringFetchMode::Last,
            }
        );
    }

    #[test]
    fn attr_bridge_wraps_name_in_symbol() {
        let cc = ContentComponent::Attr {
            name: SmolStr::new("data-title"),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Attr");
        assert_eq!(
            cvi,
            ContentValueItem::Attr {
                name: Symbol::new("data-title"),
            }
        );
    }

    #[test]
    fn target_counter_bridge_parses_url() {
        let cc = ContentComponent::TargetCounter {
            url: String::from("https://example.com/#foo"),
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetCounter {
                url: expected_url,
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn target_counters_bridge_maps_separator_to_sep() {
        // ContentComponent::TargetCounters は `separator`、design doc §7.1
        // line 1935 の ContentValueItem::TargetCounters は `sep` — bridge
        // で名称変換される。
        let cc = ContentComponent::TargetCounters {
            url: String::from("https://example.com/#foo"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetCounters {
                url: expected_url,
                name: Symbol::new("section"),
                sep: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn target_text_bridge_parses_url() {
        let cc = ContentComponent::TargetText {
            url: String::from("https://example.com/#foo"),
            part: ContentPart::Before,
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetText {
                url: expected_url,
                part: ContentPart::Before,
            }
        );
    }

    #[test]
    fn content_bridge_maps_text_keyword_to_content_part() {
        // ContentTextKeyword::Text (GCPM 3 §1.1.1.1) →
        // ContentPart::Content (CSS Content 3 §2.6.3): 両者とも「要素の
        // string value 全体」を指す同一概念への canonical mapping
        // (spec の "default" 宣言には依らない — 詳細は
        // content_text_keyword_to_content_part の doc comment 参照)。
        let cc = ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Content");
        assert_eq!(
            cvi,
            ContentValueItem::Content {
                part: ContentPart::Content,
            }
        );
    }

    #[test]
    fn content_bridge_maps_before_after_first_letter_verbatim() {
        // 非-default keyword は同名の ContentPart 値に mapping。
        for (kw, expected) in [
            (ContentTextKeyword::Before, ContentPart::Before),
            (ContentTextKeyword::After, ContentPart::After),
            (ContentTextKeyword::FirstLetter, ContentPart::FirstLetter),
        ] {
            let cvi = ContentValueItem::try_from(ContentComponent::Content { keyword: kw })
                .expect("infallible for Content");
            assert_eq!(
                cvi,
                ContentValueItem::Content { part: expected },
                "keyword {kw:?} should map to part {expected:?}"
            );
        }
    }

    #[test]
    fn target_counter_bridge_returns_invalid_url_error() {
        // Invalid URL (relative URL に base 無し) は InvalidUrl error。
        let cc = ContentComponent::TargetCounter {
            url: String::from("not a url"),
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_counters_bridge_propagates_url_parse_error() {
        let cc = ContentComponent::TargetCounters {
            url: String::from("not a url"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_text_bridge_propagates_url_parse_error() {
        let cc = ContentComponent::TargetText {
            url: String::from("not a url"),
            part: ContentPart::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn convert_error_display_and_source_chain() {
        use std::error::Error as _;
        // InvalidUrl は source() で url::ParseError を surface。
        let parse_err = Url::parse("not a url").expect_err("invalid URL");
        let err = ContentValueConvertError::InvalidUrl(parse_err);
        assert!(err.to_string().contains("invalid URL"));
        assert!(err.source().is_some());

        // UnsupportedVariant は source() none、Display で origin cite。
        let uv = ContentValueConvertError::UnsupportedVariant;
        assert!(uv.to_string().contains("unsupported ContentComponent"));
        assert!(uv.source().is_none());
    }
}

#[cfg(test)]
mod content_source_tests {
    use super::*;

    #[test]
    fn content_source_default_is_empty() {
        let cs = ContentSource::default();
        assert!(cs.items.is_empty());
    }

    #[test]
    fn content_source_new_wraps_vec() {
        let items = vec![
            ContentValueItem::Literal(String::from("Ch. ")),
            ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::default(),
            },
        ];
        let cs = ContentSource::new(items.clone());
        assert_eq!(cs.items, items);
    }

    #[test]
    fn content_source_struct_update_from_default() {
        // #[non_exhaustive] public struct の consumer construct pattern
        // (sibling PageBox の struct-update pattern 継承)。
        let cs = ContentSource {
            items: vec![ContentValueItem::Literal(String::from("hello"))],
            ..Default::default()
        };
        assert_eq!(cs.items.len(), 1);
    }
}

#[cfg(test)]
mod running_template_id_tests {
    use super::*;

    #[test]
    fn running_template_id_wraps_node_id() {
        let id = RunningTemplateId::new(NodeId::new(42));
        assert_eq!(id.0, NodeId::new(42));
    }

    #[test]
    fn running_template_id_copy_hash_eq_derives() {
        use std::collections::HashMap;
        let id1 = RunningTemplateId::new(NodeId::new(1));
        let id2 = id1; // Copy
        assert_eq!(id1, id2);
        // Hash + Eq — HashMap key として使える (raikiri-dom
        // RunningTemplateStore.parsed_templates keying rationale)。
        let mut m: HashMap<RunningTemplateId, &'static str> = HashMap::new();
        m.insert(id1, "template-1");
        assert_eq!(m.get(&id2), Some(&"template-1"));
    }

    #[test]
    fn running_template_id_ord_derives() {
        let id1 = RunningTemplateId::new(NodeId::new(1));
        let id2 = RunningTemplateId::new(NodeId::new(2));
        assert!(id1 < id2);
    }
}
