use super::*;

#[test]
fn builder_requires_explicit_font_bytes() {
    assert!(matches!(
        FontCollectionBuilder::new().build(),
        Err(FontCollectionBuildError::NoFonts)
    ));
}

#[test]
fn builder_rejects_empty_and_invalid_font_sources() {
    assert!(matches!(
        FontCollectionBuilder::new()
            .font_bytes("   ", b"not a font".as_slice())
            .build(),
        Err(FontCollectionBuildError::EmptyFamily)
    ));
    assert!(matches!(
        FontCollectionBuilder::new()
            .font_bytes("Example", b"not a font".as_slice())
            .build(),
        Err(FontCollectionBuildError::FontRejected { family }) if family == "Example"
    ));
}

#[test]
fn builder_registers_consumer_supplied_font_bytes_without_system_fonts() {
    // Use a small valid test font so the registration path is independent
    // of platform font discovery.
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Bundled Test",
            include_bytes!("../../tests/data/NotoSansTest-Regular.ttf").as_slice(),
        )
        .build()
        .expect("bundled bytes register");
    assert!(fonts.is_bundled_only());
    let matched = fonts
        .collection()
        .match_cluster(
            &shodo::font::FontQuery {
                families: vec![shodo::style::FontFamily::Named("Bundled Test".to_owned())],
                ..Default::default()
            },
            // The test font covers a handful of code points, U+0E70 among them.
            "\u{0E70}",
        )
        .expect("the bundled family matches");
    assert!(fonts.collection().font_data(matched.id).is_some());
}
