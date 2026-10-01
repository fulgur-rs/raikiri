use super::*;

/// PNG magic bytes: \x89 P N G \r \n \x1A \n (same pin as raikiri-vrt tests).
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// Check that `html_to_png` and `html_to_png_with_fonts` delegate to the
/// shared implementation (DRY delegation): `html_to_png` with the installed
/// fonts on both engines, `html_to_png_with_fonts` on the parley path only.
#[test]
fn html_to_png_with_fonts_delegates_to_impl() {
    let input = br#"<p>x</p>"#;
    let a = html_to_png(&input[..]).expect("html_to_png Ok");
    let a_impl = html_to_png_impl(&input[..], FontContext::new(), system_inline_fonts())
        .expect("html_to_png_impl Ok");
    assert_eq!(a, a_impl, "html_to_png must produce byte-identical PNG");
    let b =
        html_to_png_with_fonts(&input[..], FontContext::new()).expect("html_to_png_with_fonts Ok");
    let b_impl =
        html_to_png_impl(&input[..], FontContext::new(), None).expect("html_to_png_impl Ok");
    assert_eq!(
        b, b_impl,
        "html_to_png_with_fonts must produce byte-identical PNG"
    );
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
    let html =
        br#"<html><head><style>@page { size: 300px 50px }</style></head><body>Hi</body></html>"#;
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
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240,
        159, 17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
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
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
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

#[test]
fn a_bundled_layer_builds_in_parallel_and_the_installed_one_does_not() {
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(&mut dom, system_inline_fonts());
    assert!(dom.inline_formatting_enabled());
    assert!(!dom.ifc_parallel_build());
    let bundled = raikiri_html::FontContextBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build_fonts()
        .expect("fonts");
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(&mut dom, Some((bundled.collection().clone(), true)));
    assert!(dom.inline_formatting_enabled());
    assert!(dom.ifc_parallel_build());
}

#[test]
fn no_inline_fonts_leaves_the_document_on_the_parley_path() {
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(&mut dom, None);
    assert!(!dom.inline_formatting_enabled());
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));
