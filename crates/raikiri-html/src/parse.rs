//! Public parse entrypoints.

use std::borrow::Cow;
use std::io::Read;

use html5ever::driver::{ParseOpts, parse_document};
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeSink;
use raikiri_traits::{ParseError, StylesheetKind};

use crate::sink::RaikiriTreeSink;
use crate::types::{ParseOptions, UncascadedDocument};

/// HTML を parse し [`UncascadedDocument`] を返す。cascade 前の DOM +
/// inline `<style>` 抽出 + parse warning が含まれる。
///
/// `input` は UTF-8 の byte stream として扱う。Read 失敗は
/// [`ParseError::Io`]、UTF-8 として invalid な入力は [`ParseError::Encoding`]
/// を返す (M1 spike scope、encoding_rs 導入は M2+ で予定)。
///
/// # Example
///
/// ```
/// use raikiri_html::{parse, ParseOptions};
/// use raikiri_traits::Dom;
///
/// let html = b"<html><body>Hi</body></html>";
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse(&html[..], &opts).unwrap();
/// // Parse succeeded; dom has root
/// assert_eq!(doc.dom.root_id().0, 0);
/// ```
pub fn parse<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    parse_with_sink(input, RaikiriTreeSink::new(), options)
}

/// Consumer-supplied sink 経由で parse する。Consumer wrapper は
/// `type Output = UncascadedDocument` を宣言し、`finish(self)` で inner
/// sink の finish 結果を bubble させる契約。
///
/// (Task 8 で impl。Task 4 段階では public export のみ配置。)
pub fn parse_with_sink<R, S>(
    mut input: R,
    sink: S,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError>
where
    R: Read,
    S: TreeSink<Handle = usize, Output = UncascadedDocument>,
{
    // 2-step: reader failures → Io、UTF-8 conversion failures → Encoding。
    // read_to_string の InvalidData 一括分類 (reader が非-encoding 由来で
    // InvalidData を返すケース) を防ぐ。
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).map_err(ParseError::Io)?;
    let buf = String::from_utf8(bytes).map_err(|e| ParseError::Encoding {
        label: String::from("utf-8"),
        reason: e.to_string(),
    })?;

    let parser = parse_document(sink, ParseOpts::default());
    let mut doc = parser.one(buf.as_str());

    // spec §M1.4a: 既定 UA CSS を Document に注入 (raikiri-spike-m1.22)
    doc.dom.add_stylesheet(
        Cow::Borrowed(crate::ua::MINIMAL_UA_CSS),
        StylesheetKind::UserAgent,
    );

    // Consumer 提供の extra_stylesheets を Author として追加 (spec §M1
    // ParseOptions::extra_stylesheets の実 consume 経路)
    for extra in options.extra_stylesheets {
        doc.dom
            .add_stylesheet(Cow::Owned((*extra).to_string()), StylesheetKind::Author);
    }

    Ok(doc)
}
