//! `html_to_png` — dogfooding helper: HTML → first-page PNG bytes (A4 fallback)。
//!
//! spec §L1118 の convenience wrapper。VRT (hello-world) / examples 用途。
//! Consumer が multi-page / custom PageBox / neutral page streaming を要する場合は
//! Parses HTML, builds a page scene, and rasterizes it to PNG.
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
mod tests;
