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
    assert!(format!("{fonts:?}").contains("bundled_only: true"));
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

#[test]
fn builder_names_the_first_rejected_font_after_a_valid_font() {
    let result = FontCollectionBuilder::new()
        .font_bytes(
            "Valid",
            include_bytes!("../../tests/data/NotoSansTest-Regular.ttf").as_slice(),
        )
        .font_bytes("  First invalid  ", b"not a font".as_slice())
        .font_bytes("Second invalid", b"also not a font".as_slice())
        .build();
    assert!(matches!(
        result,
        Err(FontCollectionBuildError::FontRejected { family }) if family == "First invalid"
    ));
}

#[test]
fn builder_preserves_registration_order_for_generic_fallback() {
    use shodo::style::{FontFamily, GenericFamily};

    let bytes = include_bytes!("../../tests/data/NotoSansTest-Regular.ttf").as_slice();
    for system_fonts in [false, true] {
        for names in [["First", "Second"], ["Second", "First"]] {
            let fonts = FontCollectionBuilder::new()
                .font_bytes(names[0], bytes)
                .font_bytes(names[1], bytes)
                .system_fonts(system_fonts)
                .build()
                .expect("both bundled families register");
            assert_eq!(fonts.is_bundled_only(), !system_fonts);
            let matched = |family| {
                fonts
                    .collection()
                    .match_cluster(
                        &shodo::font::FontQuery {
                            families: vec![family],
                            ..Default::default()
                        },
                        "\u{0E70}",
                    )
                    .expect("the test font covers the cluster")
                    .id
            };
            let first = matched(FontFamily::Named(names[0].to_owned()));
            let second = matched(FontFamily::Named(names[1].to_owned()));
            assert_ne!(first, second);
            for generic in [
                GenericFamily::Serif,
                GenericFamily::SansSerif,
                GenericFamily::Monospace,
                GenericFamily::SystemUi,
                GenericFamily::Cursive,
                GenericFamily::Fantasy,
            ] {
                assert_eq!(matched(FontFamily::Generic(generic)), first);
            }
        }
    }
}
