//! `html_to_png` — dogfooding helper: HTML → first-page PNG bytes (A4 fallback)。
//!
//! spec §L1118 の convenience wrapper。VRT (hello-world) / examples 用途。
//! Consumer が multi-page / custom PageBox / neutral page streaming を要する場合は
//! `parse_html` + `render_streaming` を chain する。
//!
//! # 現状の契約
//! - PageBox は `@page { size: ... }` の first-page cascadeを優先し、未指定時は
//!   `PageBox::A4`。custom PageBox は将来
//!   `html_to_png_with(input, PageBox, PageDefaults)` variant を追加予定
//! - `html_to_png` / `html_to_png_with_fonts` は `ReplacedResolver` 不要
//!   (replaced element 非対応)。`<img>` を実際に fetch→decode→paint する
//!   経路は [`html_to_png_with_resolver`] を使う
//! - 単一ページのみ。overflow の 2 ページ目 clip は将来の pagestream state
//!   machine で対応
//! - 内部 pipeline:
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
use crate::parse_html;

/// `html_to_png` / `html_to_png_with_fonts` の共通実装。VRT test 経路 (pinned
/// `FontContext`) と production 経路 (`FontContext::new()`) の layout logic を
/// 1 箇所に集約し、drift を構造的に防止する。
///
/// # Errors
/// - `RenderError::Parse(_)` — `parse_html` からの伝播 (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — `layout_single_page` からの伝播 (`<body>` 欠落 /
///   parley shape / taffy internal)
pub(crate) fn html_to_png_impl<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    // ParseOptions は default 相当 (extra stylesheet / network / base_url 不要)
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let (mut uncascaded, cascade) = parse_html(input, &opts)?.into_parts();

    let page_box = PageBox::from_page_size(cascade.page.size());
    // `into_parts` で所有権ごと分解するので、dom の `&mut` と cascade の `&` を
    // 同時に取れる。`?` は raikiri-traits の `From<LayoutError> for RenderError`
    // で LayoutError → RenderError::Layout に自動変換される。
    raikiri_dom::layout_single_page(&mut uncascaded.dom, &cascade, page_box, font_ctx)?;

    // post-layout Document から PageScene snapshot を
    // 抽出し、byte-identical な raster + encode triple は PageScene::rasterize に
    // 集約された。dom / cascade は rasterize に thread されて既存 paint pipeline
    // が verbatim reuse される (PageDrawables 経由 paint 再導出は
    // byte-identical を破るため defer、rasterize が真の snapshot に至る
    // までの過渡形として dom + cascade を param に受ける)。
    let dom = &uncascaded.dom;
    let scene = build_page_scene(dom, &cascade, page_box);
    Ok(scene.rasterize(dom, &cascade, page_box))
}

/// HTML byte stream を単一の first page (A4 fallback) の PNG に raster する。
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
/// と signature を統一するため `impl Read` を採用 (design 決定)。
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, FontContext::new())
}

/// Font-aware 版。渡された `FontContext` がそのまま layout に使われる。
///
/// cross-machine 決定性が必要な VRT test 向け。
/// `font_ctx` が check 済み (`build_wpt_font_ctx` 経由) の場合、system font
/// resolver は完全 bypass される。
///
/// # Scope
/// - VRT test 向け。production runtime は既存 [`html_to_png`] を使う
/// - 将来 `@font-face` 対応時に production consumer にも展開検討
///
/// # Errors
/// [`html_to_png`] と同じ (`RenderError::Parse` / `RenderError::Layout`)。
pub fn html_to_png_with_fonts<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, font_ctx)
}

/// [`html_to_png`] と同一だが、`resolver`/`pixel_source` 経由で `<img>` を
/// 実際に fetch→decode→layout→paint する。
///
/// `resolver`/`pixel_source` は同一の値を指すことが多い (例:
/// `raikiri_net::ImageResolver` は両方の trait を実装する) が、この関数は
/// それを要求しない — 別々の型でもよい。
///
/// # Errors
/// [`html_to_png`] と同じ、加えて `RenderError::Resolver` — `resolver` が
/// いずれかの `<img>` の resolve に失敗した場合。`ReplacedResolver` の
/// 契約上 `Err` は terminal なので、最初の失敗で render 全体が停止する
/// (握りつぶして 0×0 にはしない)。placeholder への degrade を望む Consumer
/// は `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })` を返す —
/// `raikiri_dom::layout_single_page_with_resolver` の doc 参照。
pub fn html_to_png_with_resolver<R, I>(
    input: impl std::io::Read,
    resolver: &R,
    pixel_source: &I,
) -> Result<Vec<u8>, RenderError>
where
    R: raikiri_traits::ReplacedResolver,
    I: raikiri_traits::ImagePixelSource,
{
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let (mut uncascaded, cascade) = parse_html(input, &opts)?.into_parts();
    let page_box = PageBox::from_page_size(cascade.page.size());
    raikiri_dom::layout_single_page_with_resolver(
        &mut uncascaded.dom,
        &cascade,
        page_box,
        FontContext::new(),
        resolver,
    )?;
    let dom = &uncascaded.dom;
    let scene = build_page_scene(dom, &cascade, page_box);
    Ok(scene.rasterize_with_images(dom, &cascade, page_box, pixel_source))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG magic bytes: \x89 P N G \r \n \x1A \n (raikiri-vrt tests と同じ pinning)
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

    /// `html_to_png_with_fonts` が `html_to_png` と同じ output を返す
    /// (`FontContext::new()` を渡した場合)。DRY delegate 経路の regression check。
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
    fn html_to_png_uses_first_page_size_descriptor_for_png_dimensions() {
        let html = br#"<html><head><style>@page { size: 300px 50px }</style></head><body>Hi</body></html>"#;
        let png = html_to_png(&html[..]).expect("custom @page size should render");
        assert!(png.len() >= 24, "PNG must contain the IHDR header");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (300, 50));
    }

    /// `html_to_png_with_resolver` が `<img>` の intrinsic size resolve →
    /// layout → decode 済み pixel の実描画までを end-to-end で通す
    /// (resolver/pixel-source の配線が実際に paint するところまで証明する —
    /// 単に呼び出しが成功するだけでは検出できない、下の `assert_ne!` 参照)。
    #[test]
    fn html_to_png_with_resolver_paints_an_img_element() {
        use raikiri_net::{FileNetworkProvider, ImageResolver};
        use std::io::Write;

        // 2x1 PNG, red then green pixel (raikiri-net の image_resolver.rs
        // `TINY_PNG` と同じバイト列)。
        const PNG_BYTES: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207,
            192, 240, 159, 17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68,
            174, 66, 96, 130,
        ];
        let mut tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        tmp.write_all(PNG_BYTES).unwrap();
        let url = url::Url::from_file_path(tmp.path()).unwrap();

        let html = format!(
            r#"<html><head><style>img {{ width: 20px; height: 20px }}</style></head><body><img src="{url}"></body></html>"#
        );

        let resolver = ImageResolver::new(FileNetworkProvider);
        let png = html_to_png_with_resolver(html.as_bytes(), &resolver, &resolver)
            .expect("should render with a resolved <img>");

        assert_eq!(&png[0..8], &PNG_MAGIC);

        // Magic bytes alone would pass even if resolve/decode/paint silently
        // no-op'd (unresolved `<img>` just paints nothing, per
        // `raikiri_traits::ImagePixelSource::get_decoded` doc). Compare
        // against the resolver-less path (same HTML/geometry) to prove the
        // decoded pixels actually reached the canvas.
        let without_resolver = html_to_png(html.as_bytes()).expect("baseline render");
        assert_ne!(
            png, without_resolver,
            "resolver path must paint different pixels than the no-resolver baseline"
        );
    }

    /// A `ReplacedResolver::resolve()` `Err` is terminal: it must fail the
    /// whole render as `RenderError::Resolver`, not be swallowed into a
    /// silently unsized `<img>`. Graceful degradation is the Consumer's job,
    /// expressed as `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })`
    /// (see `raikiri_traits::ReplacedResolver`'s doc), so raikiri must not
    /// perform it on the Consumer's behalf.
    #[test]
    fn html_to_png_with_resolver_propagates_a_terminal_resolver_error() {
        use raikiri_traits::{
            DecodedImage, ImagePixelSource, ReplacedResolver, ResolvedIntrinsic, ResolverError,
            ResolverRequest,
        };
        use std::sync::Arc;

        struct AlwaysErrResolver;
        impl ReplacedResolver for AlwaysErrResolver {
            fn resolve(
                &self,
                _req: ResolverRequest<'_>,
            ) -> Result<ResolvedIntrinsic, ResolverError> {
                Err(ResolverError::Decode("simulated decode failure".into()))
            }
        }
        impl ImagePixelSource for AlwaysErrResolver {
            fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
                // Unreachable: the render fails before paint. Returning None
                // keeps this honest rather than fabricating pixels.
                None
            }
        }

        // The `src` must parse as an absolute URL — a relative one is skipped
        // before `resolve()` is ever called, which would make this vacuous.
        let html = br#"<html><body><img src="file:///nonexistent-fixture.png"></body></html>"#;
        let resolver = AlwaysErrResolver;
        let err = html_to_png_with_resolver(&html[..], &resolver, &resolver)
            .expect_err("a resolver Err must fail the render");
        assert!(
            matches!(err, RenderError::Resolver(ResolverError::Decode(_))),
            "expected RenderError::Resolver(Decode(_)), got {err:?}"
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
