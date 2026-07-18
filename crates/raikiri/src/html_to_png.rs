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

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use raikiri_html::ParseOptions;
use raikiri_traits::{PageBox, RenderError};

use crate::parse::parse_html;

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
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
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
    raikiri_dom::layout_single_page(&mut doc.uncascaded.dom, &doc.cascade, page_box)?;

    // PageBox = 793.7008 × 1122.5197 CSS px → 794 × 1123 u32 buffer
    let width = page_box.width.ceil() as u32;
    let height = page_box.height.ceil() as u32;

    // paint_single_page への split-borrow (dom / cascade は render_to_buffer の
    // closure に move されないよう `let` binding で外に出す)
    let dom = &doc.uncascaded.dom;
    let cascade = &doc.cascade;
    let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| raikiri_paint::paint_single_page(scene, dom, cascade, page_box),
        width,
        height,
    );

    Ok(encode_png(&rgba, width, height))
}

/// Encode a premultiplied RGBA8 buffer to PNG bytes via `tiny_skia::Pixmap`.
///
/// The buffer must be exactly `width * height * 4` bytes. Buffer format is
/// premultiplied RGBA8 — the `anyrender_vello_cpu` output convention.
/// `tiny_skia` stores pixmaps in the same format, so encoding is a direct
/// wrap-then-serialize.
///
/// # Panics
///
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization failure (tiny-skia never returns an error for a
///   well-formed pixmap in practice; treated as an invariant violation)
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let pixmap = tiny_skia::Pixmap::from_vec(rgba.to_vec(), size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)");
    pixmap
        .encode_png()
        .expect("encode_png: tiny_skia::Pixmap::encode_png should not fail for a valid pixmap")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG magic bytes: \x89 P N G \r \n \x1A \n (raikiri-vrt tests と同じ pinning)
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

    #[test]
    fn html_to_png_returns_png_bytes_for_hello_world() {
        let html = b"<p style=\"color:red\">Hi</p>";
        let png = html_to_png(&html[..]).expect("html_to_png must succeed");
        assert!(png.len() > 8, "PNG payload should include header + IDAT chunks");
        assert_eq!(
            &png[..8], &PNG_MAGIC,
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
