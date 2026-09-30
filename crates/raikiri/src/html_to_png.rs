//! `html_to_png` — a dogfooding helper that renders HTML to first-page PNG bytes
//! (A4 fallback).
//!
//! This convenience wrapper from spec §L1118 serves VRT (hello-world) tests
//! and examples. For multiple pages, a custom PageBox, or neutral page
//! streaming, consumers should chain `parse_html` and `render_streaming`.
//!
//! # Current contract
//! - PageBox uses the first-page cascade of `@page { size: ... }`, falling
//!   back to `PageBox::A4`. A future
//!   `html_to_png_with(input, PageBox, PageDefaults)` variant may allow a
//!   custom PageBox.
//! - `html_to_png` and `html_to_png_with_fonts` need no `ReplacedResolver`
//!   because they do not support replaced elements. Use
//!   [`html_to_png_with_resolver`] to fetch, decode, and paint `<img>`.
//! - Only the first page is rendered; a future page-stream state machine
//!   will handle clipping overflow on subsequent pages.
//! - Pipeline: `parse_html` → `layout_single_page` → `build_page_scene` →
//!   `PageScene::rasterize` → PNG bytes. The rasterizer preserves the exact
//!   `raikiri_paint::paint_single_page` +
//!   `anyrender::render_to_buffer::<VelloCpuImageRenderer>` + `encode_png`
//!   calls. [`crate::PageScene::rasterize`] centralizes this byte-identical
//!   raster/encode sequence.

use parley::FontContext;
use raikiri_html::ParseOptions;
use raikiri_traits::{PageBox, RenderError};

use crate::page_scene::build_page_scene;
use crate::parse_html;

/// Shared implementation for `html_to_png` and `html_to_png_with_fonts`.
/// Centralizing the VRT path (pinned `FontContext`) and production path
/// (`FontContext::new()`) prevents their layout logic from drifting.
///
/// # Errors
/// - `RenderError::Parse(_)` — propagated from `parse_html` (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — propagated from `layout_single_page` (missing
///   `<body>` / parley shaping / taffy internals)
pub(crate) fn html_to_png_impl<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    // Default-equivalent ParseOptions: no extra stylesheets, network, or base URL.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let (mut uncascaded, cascade) = parse_html(input, &opts)?.into_parts();

    let page_box = PageBox::from_page_size(cascade.page.size());
    // `into_parts` takes ownership of the pieces, allowing `&mut` on the DOM
    // alongside `&` on the cascade. `?` converts LayoutError to
    // RenderError::Layout via raikiri-traits' From implementation.
    raikiri_dom::layout_single_page(&mut uncascaded.dom, &cascade, page_box, font_ctx)?;

    // Extract PageScene from the post-layout Document. PageScene::rasterize
    // centralizes the byte-identical raster/encode sequence and still receives
    // the DOM and cascade to reuse the existing paint pipeline. Rebuilding
    // paint through PageDrawables could change bytes, so these parameters
    // remain until rasterize can operate on a true standalone snapshot.
    let dom = &uncascaded.dom;
    let scene = build_page_scene(dom, &cascade, page_box);
    Ok(scene.rasterize(dom, &cascade, page_box))
}

/// Rasterize an HTML byte stream to a PNG of the first page (A4 fallback).
///
/// Delegate to `html_to_png_impl` with the system font resolver
/// (`FontContext::new()`). This is the production runtime path.
///
/// # Errors
/// - `RenderError::Parse(_)` — propagated from `parse_html` (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — propagated from `layout_single_page` (missing
///   `<body>` / parley shaping / taffy internals)
///
/// The spec §L1118 gives the signature `(html: &str)`; this design instead
/// accepts `impl Read` to match the existing `parse_html<R: Read>` API.
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, FontContext::new())
}

/// Font-aware variant that uses the supplied `FontContext` for layout.
///
/// Intended for VRT tests that need cross-machine reproducibility. When
/// `font_ctx` has been validated by `build_wpt_font_ctx`, this completely
/// bypasses the system font resolver.
///
/// # Scope
/// - Intended for VRT tests; production runtimes should use [`html_to_png`].
/// - May be extended to production consumers once `@font-face` is supported.
///
/// # Errors
/// Same as [`html_to_png`] (`RenderError::Parse` / `RenderError::Layout`).
pub fn html_to_png_with_fonts<R: std::io::Read>(
    input: R,
    font_ctx: FontContext,
) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, font_ctx)
}

/// Like [`html_to_png`], but fetches, decodes, lays out, and paints `<img>`
/// through `resolver` and `pixel_source`.
///
/// They often refer to the same value (`raikiri_net::ImageResolver` implements
/// both traits), but this function also accepts separate types.
///
/// # Errors
/// In addition to errors from [`html_to_png`], returns `RenderError::Resolver`
/// if `resolver` fails for any `<img>`. The `ReplacedResolver` contract treats
/// `Err` as terminal: rendering stops at the first error rather than silently
/// replacing the image with 0×0. Consumers who want a placeholder should
/// return `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })`; see the
/// `raikiri_dom::layout_single_page_with_resolver` docs.
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
