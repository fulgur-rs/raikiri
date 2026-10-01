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

/// The inline engine's font layer and whether it holds bundled fonts only.
type InlineFonts = (shodo::font::FontCollection, bool);

/// The inline engine over the process-wide layer of the installed fonts.
/// That layer loads faces lazily, so its paragraphs are built in sequence.
fn system_inline_fonts() -> Option<InlineFonts> {
    Some((raikiri_dom::system_font_collection(), false))
}

/// Switch the inline engine on for `dom` with `fonts`, or leave every
/// paragraph on the parley path when `fonts` is `None`.
fn enable_inline_engine(dom: &mut raikiri_dom::Document, fonts: Option<InlineFonts>) {
    if let Some((collection, bundled_only)) = fonts {
        dom.enable_inline_formatting(collection, shodo::limits::Limits::default());
        // Only a layer without installed fonts loads every face up front, so
        // only then may paragraphs be built on several threads.
        dom.set_ifc_parallel_build(bundled_only);
    }
}

/// Shared implementation for `html_to_png` and its font variants.
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
    inline_fonts: Option<InlineFonts>,
) -> Result<Vec<u8>, RenderError> {
    // Default-equivalent ParseOptions: no extra stylesheets, network, or base URL.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let (mut uncascaded, cascade) = parse_html(input, &opts)?.into_parts();
    enable_inline_engine(&mut uncascaded.dom, inline_fonts);

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
/// (`FontContext::new()`). This is the production runtime path. Paragraphs
/// are laid out by the inline engine over the installed fonts
/// ([`raikiri_dom::system_font_collection`]); a paragraph it does not
/// support falls back to the parley path.
///
/// # Errors
/// - `RenderError::Parse(_)` — propagated from `parse_html` (IO / UTF-8 / html5ever)
/// - `RenderError::Layout(_)` — propagated from `layout_single_page` (missing
///   `<body>` / parley shaping / taffy internals)
///
/// The spec §L1118 gives the signature `(html: &str)`; this design instead
/// accepts `impl Read` to match the existing `parse_html<R: Read>` API.
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, FontContext::new(), system_inline_fonts())
}

/// Font-aware variant that uses the supplied `FontContext` for layout.
///
/// Every paragraph is laid out by the parley path with `font_ctx`: the
/// inline engine is not used, since it would need a font set built from the
/// same fonts (see [`html_to_png_with_render_fonts`]).
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
    html_to_png_impl(input, font_ctx, None)
}

/// Like [`html_to_png_with_fonts`] with a font set both engines share.
///
/// Paragraphs are laid out by the inline engine with the shodo layer of
/// `fonts`, and a paragraph it does not support falls back to the parley
/// path with the parley context of `fonts`, so the whole page draws from one
/// font set. Build `fonts` with
/// [`FontContextBuilder::build_fonts`](crate::FontContextBuilder::build_fonts)
/// for output that does not depend on the installed fonts.
///
/// # Errors
/// Same as [`html_to_png`] (`RenderError::Parse` / `RenderError::Layout`).
pub fn html_to_png_with_render_fonts<R: std::io::Read>(
    input: R,
    fonts: raikiri_html::RenderFonts,
) -> Result<Vec<u8>, RenderError> {
    let bundled_only = fonts.is_bundled_only();
    let (context, collection) = fonts.into_parts();
    html_to_png_impl(input, context, Some((collection, bundled_only)))
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
    enable_inline_engine(&mut uncascaded.dom, system_inline_fonts());
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
