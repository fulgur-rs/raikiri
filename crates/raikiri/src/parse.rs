//! `parse_html`: HTML byte stream から cascade 済 [`HtmlDocument`] を生成する
//! orchestrator (raikiri-spike-m1.11)。
//!
//! spec §L1060 の pub API 相当。内部 pipeline は
//! `raikiri_html::parse` → [`build_cascaded`] → assemble。M1 では cascade は
//! 常に `Ok` を返すため、`RenderError::Parse` のみが bubble する。

use std::io::Read;

use raikiri_html::ParseOptions;
use raikiri_traits::RenderError;

use crate::{HtmlDocument, build_cascaded};

/// HTML byte stream を parse し、cascade まで完了した [`HtmlDocument`] を返す。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
///
/// M1 では cascade は infallible (m1.23 契約、`.expect` で unwrap)。
///
/// # Example
///
/// ```
/// use raikiri::{parse_html, ParseOptions};
///
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
/// assert!(!doc.cascade().computed.is_empty());
/// ```
#[allow(clippy::result_large_err)]
pub fn parse_html<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<HtmlDocument, RenderError> {
    let uncascaded = raikiri_html::parse(input, options).map_err(RenderError::Parse)?;
    let cascade = build_cascaded(&uncascaded);
    Ok(HtmlDocument {
        uncascaded,
        cascade,
    })
}
