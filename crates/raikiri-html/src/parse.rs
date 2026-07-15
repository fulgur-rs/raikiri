//! Public parse entrypoints.

use std::io::Read;

use html5ever::driver::{parse_document, ParseOpts};
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeSink;
use raikiri_traits::ParseError;

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
    _options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError>
where
    R: Read,
    S: TreeSink<Handle = usize, Output = UncascadedDocument>,
{
    let mut buf = String::new();
    input.read_to_string(&mut buf).map_err(|e| {
        // read_to_string は invalid UTF-8 で InvalidData を返す。
        if e.kind() == std::io::ErrorKind::InvalidData {
            ParseError::Encoding {
                label: String::from("utf-8"),
                reason: e.to_string(),
            }
        } else {
            ParseError::Io(e)
        }
    })?;

    let parser = parse_document(sink, ParseOpts::default());
    Ok(parser.one(buf.as_str()))
}
