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
//! - `html_to_png` and `html_to_png_with_render_fonts` need no `ReplacedResolver`
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

use raikiri_html::ParseOptions;
use raikiri_traits::{PageBox, RenderError};

use crate::page_scene::build_page_scene;
use crate::parse_html;

/// Give `dom` the font set `fonts`; without one, its first layout takes the
/// installed fonts.
fn use_fonts(dom: &mut raikiri_dom::Document, fonts: Option<raikiri_html::RenderFonts>) {
    if let Some(fonts) = fonts {
        // shodo does not guarantee deterministic matching during concurrent
        // registration for collections with system-font discovery enabled.
        let collection = fonts.into_collection();
        let bundled_only = collection.is_bundled_only();
        dom.set_font_collection(collection);
        dom.set_ifc_parallel_build(bundled_only);
    }
}

/// Shared implementation for `html_to_png` and its font variant. Without
/// `fonts`, text is laid out with the installed fonts.
///
/// # Errors
/// - `RenderError::Parse(_)` — propagated from `parse_html` (IO / UTF-8 / html5ever)
/// - `RenderError::LimitExceeded` — propagated from `parse_html` when input
///   bytes or parsed DOM nodes exceed their default limits
/// - `RenderError::Layout(_)` — propagated from `layout_single_page` (missing
///   `<body>` / inline layout / taffy internals)
/// - `RenderError::LimitExceeded` — returned when the first page exceeds the
///   shared raster edge or byte budget
pub(crate) fn html_to_png_impl<R: std::io::Read>(
    input: R,
    fonts: Option<raikiri_html::RenderFonts>,
) -> Result<Vec<u8>, RenderError> {
    // Default-equivalent ParseOptions: no extra stylesheets, network, or base URL.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let (mut uncascaded, cascade) = parse_html(input, &opts)?.into_parts();
    use_fonts(&mut uncascaded.dom, fonts);

    let page_box = PageBox::from_page_size(cascade.page.size());
    // `into_parts` takes ownership of the pieces, allowing `&mut` on the DOM
    // alongside `&` on the cascade. `?` converts LayoutError to
    // RenderError::Layout via raikiri-traits' From implementation.
    raikiri_dom::layout_single_page(&mut uncascaded.dom, &cascade, page_box)?;

    // Extract PageScene from the post-layout Document. PageScene::rasterize
    // centralizes the byte-identical raster/encode sequence and still receives
    // the DOM and cascade to reuse the existing paint pipeline. Rebuilding
    // paint through PageDrawables could change bytes, so these parameters
    // remain until rasterize can operate on a true standalone snapshot.
    let dom = &uncascaded.dom;
    let scene = build_page_scene(dom, &cascade, page_box);
    let mut counter_budget = raikiri_dom::CounterSnapshotBudget::default();
    scene.rasterize(dom, &cascade, page_box, &mut counter_budget)
}

/// Rasterize an HTML byte stream to a PNG of the first page (A4 fallback).
///
/// Text is laid out by the inline engine with the installed fonts
/// ([`raikiri_dom::system_font_collection`]). This is the production runtime
/// path.
///
/// # Errors
/// - `RenderError::Parse(_)` — propagated from `parse_html` (IO / UTF-8 / html5ever)
/// - `RenderError::LimitExceeded` — propagated from `parse_html` when input
///   bytes or parsed DOM nodes exceed their default limits
/// - `RenderError::Layout(_)` — propagated from `layout_single_page` (missing
///   `<body>` / inline layout / taffy internals)
/// - `RenderError::LimitExceeded` — returned when the first page exceeds the
///   shared raster edge or byte budget
///
/// The spec §L1118 gives the signature `(html: &str)`; this design instead
/// accepts `impl Read` to match the existing `parse_html<R: Read>` API.
pub fn html_to_png<R: std::io::Read>(input: R) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, None)
}

/// Like [`html_to_png`] with a caller-built font set.
///
/// Text is laid out and drawn with `fonts` only. Build `fonts` with
/// [`FontCollectionBuilder`](crate::FontCollectionBuilder) for output that
/// does not depend on the installed fonts; this is what VRT tests need for
/// cross-machine reproducibility.
///
/// # Errors
/// Same as [`html_to_png`] (`RenderError::Parse` /
/// `RenderError::LimitExceeded` / `RenderError::Layout`).
pub fn html_to_png_with_render_fonts<R: std::io::Read>(
    input: R,
    fonts: raikiri_html::RenderFonts,
) -> Result<Vec<u8>, RenderError> {
    html_to_png_impl(input, Some(fonts))
}

/// Like [`html_to_png`], but fetches, decodes, lays out, and paints `<img>`
/// and inside `list-style-image` markers through `resolver` and `pixel_source`.
///
/// They often refer to the same value (`raikiri_net::ImageResolver` implements
/// both traits), but this function also accepts separate types.
///
/// # Errors
/// In addition to errors from [`html_to_png`], returns `RenderError::Resolver`
/// if `resolver` fails for any image, and `RenderError::LimitExceeded` when
/// the raster exceeds its edge or byte budget. The `ReplacedResolver` contract treats
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
    let base_url = raikiri_html::effective_document_base_url(&uncascaded, None);
    for (element, computed) in cascade.computed.iter().enumerate() {
        if !raikiri_dom::generated_content::inside_marker_in_flow(&cascade, element)
            || cascade
                .pseudo
                .get(&(
                    raikiri_style::StyleNodeId::new(element as u64),
                    raikiri_style::PseudoElem::Marker,
                ))
                .is_some_and(|marker| !marker.content.is_empty())
        {
            continue;
        }
        let raikiri_style::property::BackgroundImage::Url(raw) = &computed.list_style_image else {
            continue;
        };
        let Some(url) = url::Url::parse(raw)
            .ok()
            .or_else(|| base_url.as_ref().and_then(|base| base.join(raw).ok()))
        else {
            continue;
        };
        resolver
            .resolve(raikiri_traits::ResolverRequest::new(&url))
            .map_err(RenderError::Resolver)?;
    }
    uncascaded
        .dom
        .prepare_list_marker_images(&cascade, pixel_source, base_url.as_ref());
    raikiri_dom::layout_single_page_with_resolver(
        &mut uncascaded.dom,
        &cascade,
        page_box,
        resolver,
    )?;
    let dom = &uncascaded.dom;
    let scene = build_page_scene(dom, &cascade, page_box);
    let mut counter_budget = raikiri_dom::CounterSnapshotBudget::default();
    scene.rasterize_with_images(dom, &cascade, page_box, pixel_source, &mut counter_budget)
}

#[cfg(test)]
mod tests;
