//! Public data types produced/consumed by raikiri-html.

use raikiri_dom::Document;
use raikiri_traits::{NetworkProvider, RenderWarning};
use url::Url;

/// Parse phase の出力。cascade 前の DOM + inline stylesheet 抽出結果 +
/// parse warnings。
///
/// M1 では `<style>` element の中身のみ `stylesheet_sources` に集約する
/// (external `<link rel="stylesheet">` は M2 network integration で扱う)。
/// `warnings` は html5ever tokenizer 由来の非致命 parse error を
/// [`raikiri_traits::WarningKind::HtmlParseError`] variant で保持する。
#[derive(Debug)]
pub struct UncascadedDocument {
    /// DOM tree (raikiri-dom arena)。
    pub dom: Document,
    /// `<style>` element の text content をそのまま抽出したもの。
    /// M2 で `Vec<StylesheetSource>` (Inline / External enum) に昇格予定。
    pub stylesheet_sources: Vec<String>,
    /// html5ever が報告した非致命 parse error を warning として保持。
    /// M1.5+ orchestrator が `Document` を経由し
    /// `RenderSummary.warnings` に merge する。
    pub warnings: Vec<RenderWarning>,
    /// HTML5 quirks mode 判定 (html5ever の QuirksMode をミラーした
    /// raikiri-native enum)。m1.4 cascade が selector 挙動 / 特別ルール
    /// で参照する予定。
    pub quirks_mode: raikiri_traits::QuirksMode,
}

/// Parse に渡す option 群。
///
/// M1.3 では **どの field も raikiri-html 内では読まれない** (`extra_stylesheets`
/// は M1.4 raikiri-style::cascade が消費、`network` / `base_url` は M2 network
/// integration で消費)。API 型 shape の pre-landing のみ。
pub struct ParseOptions<'a> {
    /// Consumer が cascade 時に追加供給する CSS 文字列列 (fulgur の内部 UA CSS 等)。
    pub extra_stylesheets: &'a [&'a str],
    /// Replaced element や `<link rel="stylesheet">` 等の外部 resource
    /// fetch に用いる provider (M2 で consumption)。
    pub network: Option<&'a dyn NetworkProvider>,
    /// Relative URL の resolution base (M2 で consumption)。
    pub base_url: Option<Url>,
}
