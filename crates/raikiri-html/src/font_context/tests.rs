use super::*;

#[test]
fn builder_requires_explicit_font_bytes() {
    assert!(matches!(
        FontContextBuilder::new().build(),
        Err(FontContextBuildError::NoFonts)
    ));
}

#[test]
fn builder_rejects_empty_and_invalid_font_sources() {
    assert!(matches!(
        FontContextBuilder::new()
            .font_bytes("   ", b"not a font".as_slice())
            .build(),
        Err(FontContextBuildError::EmptyFamily)
    ));
    assert!(matches!(
        FontContextBuilder::new()
            .font_bytes("Example", b"not a font".as_slice())
            .build(),
        Err(FontContextBuildError::FontRejected { family }) if family == "Example"
    ));
}

#[test]
fn builder_registers_consumer_supplied_font_bytes_without_system_fonts() {
    // Use a small valid test font so the registration path is independent
    // of platform font discovery.
    let mut context = FontContextBuilder::new()
        .font_bytes(
            "Bundled Test",
            include_bytes!("../../tests/data/NotoSansTest-Regular.ttf").as_slice(),
        )
        .build()
        .expect("bundled bytes register");
    assert!(context.collection.family_id("Bundled Test").is_some());
}
