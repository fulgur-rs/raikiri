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
//! - 内部 pipeline (Sprint 23 raikiri-spike-bkkm):
//!   `parse_html` → `layout_single_page` → `build_page_scene` →
//!   `PageScene::rasterize` (`raikiri_paint::paint_single_page` +
//!   `anyrender::render_to_buffer::<VelloCpuImageRenderer>` + `encode_png` を
//!   内部で verbatim call) → PNG bytes
//!   raster / encode の byte-identical triple は [`crate::PageScene::rasterize`]
//!   に集約された (byte-identical 契約 = 同 triple 呼び出しの verbatim 維持)。

use parley::FontContext;
use raikiri_html::ParseOptions;
use raikiri_traits::{PageBox, RenderError};

use crate::page_scene::build_page_scene;
use crate::parse::parse_html;

/// `html_to_png` / `html_to_png_with_fonts` の共通実装。VRT test 経路 (pinned
/// `FontContext`) と production 経路 (`FontContext::new()`) の layout logic を
/// 1 箇所に集約し、drift を構造的に防止する (raikiri-spike-e93 Task 6)。
///
/// # Errors
/// - `RenderError::Parse(_)` — `parse_html` からの伝播 (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — `layout_single_page` からの伝播 (`<body>` 欠落 /
///   parley shape / taffy internal)
#[allow(clippy::result_large_err)]
pub(crate) fn html_to_png_impl<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    // ParseOptions は default 相当 (M1 fixture は extra stylesheet / network / base_url 不要)
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut doc = parse_html(input, &opts)?;

    let page_box = PageBox::A4;
    // pub(crate) field を crate-internal から split-borrow。accessor 経由だと
    // `&mut self` が要求されて cascade への同時参照が壊れるが、field 直接なら OK。
    // `?` は raikiri-traits の `From<LayoutError> for RenderError` (error.rs:137-140)
    // で LayoutError → RenderError::Layout に自動変換される。
    raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box, font_ctx)?;

    // Sprint 23 raikiri-spike-bkkm: post-layout Document から PageScene snapshot を
    // 抽出し、byte-identical な raster + encode triple は PageScene::rasterize に
    // 集約された。dom / cascade は rasterize に thread されて既存 paint pipeline
    // が verbatim reuse される (Finding 1: Sprint 23 では PageDrawables 経由 paint
    // 再導出は byte-identical を破るため defer、rasterize が真の snapshot に至る
    // までの過渡形として dom + cascade を param に受ける)。
    let dom = &doc.uncascaded.dom;
    let cascade = &doc.cascade;
    let scene = build_page_scene(dom, cascade, page_box);
    Ok(scene.rasterize(dom, cascade, page_box))
}

/// HTML byte stream を単一 A4 ページの PNG に raster する (M1.14 pub API)。
///
/// System font resolver 経由 (`FontContext::new()`) で `html_to_png_impl` に
/// delegate する。production runtime 用の経路。
///
/// # Errors
/// - `RenderError::Parse(_)` — `parse_html` からの伝播 (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — `layout_single_page` からの伝播 (`<body>` 欠落 /
///   parley shape / taffy internal)
///
/// spec §L1118 の signature literal は `(html: &str)` だが、既存 `parse_html<R: Read>`
/// と signature を統一するため `impl Read` を採用 (m1.14 design 決定)。
#[allow(clippy::result_large_err)]
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, FontContext::new())
}

/// Font-aware 版。渡された `FontContext` がそのまま layout に使われる。
///
/// cross-machine 決定性が必要な VRT test 向け (raikiri-spike-e93)。
/// `font_ctx` が pin 済み (`build_wpt_font_ctx` 経由) の場合、system font
/// resolver は完全 bypass される。
///
/// # M1 scope
/// - VRT test 向け。production runtime は既存 [`html_to_png`] を使う
/// - M4+ で `@font-face` 対応時に production consumer にも展開検討
///
/// # Errors
/// [`html_to_png`] と同じ (`RenderError::Parse` / `RenderError::Layout`)。
#[allow(clippy::result_large_err)]
pub fn html_to_png_with_fonts<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, font_ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG magic bytes: \x89 P N G \r \n \x1A \n (raikiri-vrt tests と同じ pinning)
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

    /// `html_to_png_with_fonts` が `html_to_png` と同じ output を返す
    /// (`FontContext::new()` を渡した場合)。DRY delegate 経路の regression pin。
    #[test]
    fn html_to_png_with_fonts_delegates_to_impl() {
        let input = br#"<p>x</p>"#;
        let a = html_to_png(&input[..]).expect("html_to_png Ok");
        let b = html_to_png_with_fonts(&input[..], FontContext::new())
            .expect("html_to_png_with_fonts Ok");
        assert_eq!(a, b, "delegate path must produce byte-identical PNG");
    }

    #[test]
    fn html_to_png_returns_png_bytes_for_hello_world() {
        let html = b"<p style=\"color:red\">Hi</p>";
        let png = html_to_png(&html[..]).expect("html_to_png must succeed");
        assert!(
            png.len() > 8,
            "PNG payload should include header + IDAT chunks"
        );
        assert_eq!(
            &png[..8],
            &PNG_MAGIC,
            "output must start with PNG magic bytes; got {:?}",
            &png[..8]
        );
    }

    #[test]
    fn html_to_png_propagates_parse_error_from_io() {
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let err = html_to_png(FailingReader).expect_err("must fail on reader error");
        assert!(
            matches!(err, RenderError::Parse(raikiri_traits::ParseError::Io(_))),
            "expected RenderError::Parse(ParseError::Io), got {err:?}"
        );
    }
}
