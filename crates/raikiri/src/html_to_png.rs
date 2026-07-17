//! `html_to_png` — dogfooding helper: HTML → single-page A4 PNG bytes (raikiri-spike-m1.14)。
//!
//! spec §L1118 の convenience wrapper。VRT (m1.14 hello-world) / examples 用途。
//! Consumer が multi-page / custom PageBox / streaming を要する場合は
//! `parse_html` + `render_streaming` (M2+) を chain する。
//!
//! # M1 契約
//! - PageBox は `PageBox::A4` 固定 (spec §M1.6)。custom PageBox は M2+ で
//!   `html_to_png_with(input, PageBox, PageDefaults)` variant を追加予定
//! - `ReplacedResolver` 不要 (M1 replaced element 非対応)
//! - 単一ページのみ。overflow の 2 ページ目 clip は M2 pagestream-state-machine
//! - 内部 pipeline: `parse_html` → `layout_single_page` → `paint_single_page`
//!   → `anyrender::render_to_buffer::<VelloCpuImageRenderer>` → `encode_png`

use raikiri_traits::RenderError;

/// HTML byte stream を単一 A4 ページの PNG に raster する。
///
/// # Errors
/// - `RenderError::Parse(_)` — `parse_html` からの伝播 (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — `layout_single_page` からの伝播 (`<body>` 欠落 /
///   parley shape / taffy internal)
///
/// spec §L1118 の signature literal は `(html: &str)` だが、既存 `parse_html<R: Read>`
/// と signature を統一するため `impl Read` を採用 (m1.14 design 決定)。
#[allow(clippy::result_large_err)]
pub fn html_to_png<R: std::io::Read>(_input: R) -> Result<Vec<u8>, RenderError> {
    todo!("m1.14 Task 2 で実装")
}
