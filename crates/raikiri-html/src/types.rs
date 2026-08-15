//! Public data types produced/consumed by raikiri-html.

use raikiri_dom::Document;
use raikiri_traits::{NetworkProvider, RenderWarning};
use url::Url;

/// Parse phase の出力。cascade 前の DOM + inline/external stylesheet source
/// 集約結果 + parse warnings。
///
/// `stylesheet_sources` には `<head>` 内 `<style>` element の text content
/// に加えて、`<head>` 内 `<link rel="stylesheet">` を `ParseOptions::network`
/// 経由で fetch した CSS text も Author stylesheet として集約される
/// (`parse.rs::parse_with_sink` が
/// `TreeSink::finish()` 後に追記する — fetch は I/O を伴うため Sink 実装内では
/// 行わない)。fetch 失敗や `network` 未提供時は `warnings` に記録されるのみで
/// parse 全体は継続する。順序についての scope 制限は
/// `parse.rs::fetch_external_stylesheets` の doc を参照。
/// `warnings` は html5ever tokenizer 由来の非致命 parse error を
/// [`raikiri_traits::WarningKind::HtmlParseError`] variant で、外部
/// stylesheet の fetch 失敗を [`raikiri_traits::WarningKind::NetworkFallback`] /
/// [`raikiri_traits::WarningKind::PolicyWarning`] variant で保持する。
#[derive(Debug)]
pub struct UncascadedDocument {
    /// DOM tree (raikiri-dom arena)。
    pub dom: Document,
    /// `<head>` 内 `<style>` element の text content (document order)、および
    /// 同じく `<head>` 内で fetch に成功した `<link rel="stylesheet">` の CSS
    /// text (fetch 成功順に後続で追記、struct doc 参照)。いずれも Author
    /// origin として cascade に統合される想定 (raikiri umbrella crate の
    /// `build_cascaded` が消費)。
    pub stylesheet_sources: Vec<String>,
    /// html5ever が報告した非致命 parse error を warning として保持。
    /// 上位の orchestrator (raikiri umbrella crate) が `Document` を経由し
    /// `RenderSummary.warnings` に merge する想定。
    pub warnings: Vec<RenderWarning>,
    /// HTML5 quirks mode 判定 (html5ever の QuirksMode をミラーした
    /// raikiri-native enum)。cascade が selector 挙動 / 特別ルールで
    /// 参照する予定。
    pub quirks_mode: raikiri_traits::QuirksMode,
}

/// Parse に渡す option 群。
///
/// `extra_stylesheets` は `parse_with_sink` が `Document::add_stylesheet`
/// (`StylesheetKind::Author`) 経由で消費する。`network` / `base_url` は
/// `parse_with_sink` が `<head>` 内
/// `<link rel="stylesheet">` の fetch に消費する
/// (`parse.rs::fetch_external_stylesheets`)。Replaced element (`<img>` 等) の
/// 外部 resource fetch はこの task の scope 外、引き続き未消費。
pub struct ParseOptions<'a> {
    /// Consumer が cascade 時に追加供給する CSS 文字列列 (fulgur の内部 UA CSS 等)。
    pub extra_stylesheets: &'a [&'a str],
    /// Replaced element や `<link rel="stylesheet">` 等の外部 resource
    /// fetch に用いる provider。`None` の場合 `<link rel="stylesheet">` は
    /// fetch されず無視される (外部 stylesheet 機能は opt-in)。
    /// Replaced element の fetch はこの provider を受け取るのみで
    /// まだ未消費 (scope 外)。
    pub network: Option<&'a dyn NetworkProvider>,
    /// Relative URL の resolution base。`<link rel="stylesheet" href="...">`
    /// が相対 URL の場合の解決に使う (`Url::join`)。`None` かつ `href` が
    /// 相対 URL の場合、その `<link>` は解決不能として fetch されない。
    pub base_url: Option<Url>,
}
