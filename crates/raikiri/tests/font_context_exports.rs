//! Font API type identity across the umbrella and HTML entry points.

#[test]
fn umbrella_font_exports_are_identical_types() {
    let font: raikiri::BundledFont = raikiri::BundledFont::new("Test", b"font".as_slice());
    let html_font: raikiri_html::BundledFont = font;
    let returned_font: raikiri::BundledFont = html_font;
    assert_eq!(returned_font.family(), "Test");
    assert_eq!(returned_font.bytes(), b"font");

    let builder: raikiri::FontContextBuilder = raikiri::FontContextBuilder::new();
    let html_builder: raikiri_html::FontContextBuilder = builder;
    let returned_builder: raikiri::FontContextBuilder = html_builder;
    let error: raikiri::FontContextBuildError = match returned_builder.build() {
        Err(error) => error,
        Ok(_) => panic!("expected an empty bundle error"),
    };
    let html_error: raikiri_html::FontContextBuildError = error;
    let returned_error: raikiri::FontContextBuildError = html_error;
    assert_eq!(returned_error, raikiri::FontContextBuildError::NoFonts);
    assert_eq!(
        raikiri::MAX_BUNDLED_FONT_BYTES,
        raikiri_html::MAX_BUNDLED_FONT_BYTES,
    );
}

#[test]
fn umbrella_render_fonts_are_the_html_type() {
    let error = match raikiri::FontContextBuilder::new().build_fonts() {
        Err(error) => error,
        Ok(_) => panic!("expected an empty bundle error"),
    };
    assert_eq!(error, raikiri::FontContextBuildError::NoFonts);
    let identity: fn(raikiri::RenderFonts) -> raikiri_html::RenderFonts = |fonts| fonts;
    let back: fn(raikiri_html::RenderFonts) -> raikiri::RenderFonts = |fonts| fonts;
    let _ = (identity, back);
}
