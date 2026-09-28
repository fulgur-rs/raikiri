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
mod tests {
    use super::*;

    /// PNG magic bytes: \x89 P N G \r \n \x1A \n (same pin as raikiri-vrt tests).
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

    /// Check that `html_to_png_with_fonts` returns the same output as
    /// `html_to_png` when given `FontContext::new()` (DRY delegation).
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

    /// Exercise `<img>` intrinsic-size resolution, layout, pixel decoding,
    /// and painting end to end through `html_to_png_with_resolver`.
    /// The `assert_ne!` below proves the resolver/pixel source actually
    /// paints pixels, not merely that the call succeeds.
    #[test]
    fn html_to_png_with_resolver_paints_an_img_element() {
        use raikiri_net::{FileNetworkProvider, ImageResolver};
        use std::io::Write;

        // 2x1 PNG with red and green pixels (identical bytes to
        // `TINY_PNG` in raikiri-net's image_resolver.rs).
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
