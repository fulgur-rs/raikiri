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
