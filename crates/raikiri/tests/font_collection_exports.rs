//! Font API type identity across the umbrella and HTML entry points.

use raikiri::{FontCollectionBuilder, RenderFonts};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

#[test]
fn the_collection_builder_is_public_and_needs_at_least_one_font() {
    let empty = FontCollectionBuilder::new().build();
    assert!(matches!(
        empty,
        Err(raikiri::FontCollectionBuildError::NoFonts)
    ));
    let fonts: RenderFonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM)
        .build()
        .expect("one font is enough");
    assert!(fonts.is_bundled_only());
    let with_system: RenderFonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM)
        .system_fonts(true)
        .build()
        .expect("one font is enough");
    assert!(!with_system.is_bundled_only());
}

#[test]
fn umbrella_font_exports_are_identical_types() {
    let font: raikiri::BundledFont = raikiri::BundledFont::new("Test", b"font".as_slice());
    let html_font: raikiri_html::BundledFont = font;
    let returned_font: raikiri::BundledFont = html_font;
    assert_eq!(returned_font.family(), "Test");
    assert_eq!(returned_font.bytes(), b"font");

    let builder: raikiri::FontCollectionBuilder = raikiri::FontCollectionBuilder::new();
    let html_builder: raikiri_html::FontCollectionBuilder = builder;
    let returned_builder: raikiri::FontCollectionBuilder = html_builder;
    let error: raikiri::FontCollectionBuildError = match returned_builder.build() {
        Err(error) => error,
        Ok(_) => panic!("expected an empty bundle error"),
    };
    let html_error: raikiri_html::FontCollectionBuildError = error;
    let returned_error: raikiri::FontCollectionBuildError = html_error;
    assert_eq!(returned_error, raikiri::FontCollectionBuildError::NoFonts);
    assert_eq!(
        raikiri::MAX_BUNDLED_FONT_BYTES,
        raikiri_html::MAX_BUNDLED_FONT_BYTES,
    );
    let identity: fn(raikiri::RenderFonts) -> raikiri_html::RenderFonts = |fonts| fonts;
    let back: fn(raikiri_html::RenderFonts) -> raikiri::RenderFonts = |fonts| fonts;
    let _ = (identity, back);
}
