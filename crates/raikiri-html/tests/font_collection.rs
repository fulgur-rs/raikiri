//! Consumer-supplied font construction and document layout contracts.

use raikiri_html::{
    FontCollectionBuildError, FontCollectionBuilder, FragmentKind, LayoutConfig, LayoutOptions,
    LayoutStatus, MAX_BUNDLED_FONT_BYTES, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const FONT: &[u8] = include_bytes!("data/NotoSansTest-Regular.ttf");

#[test]
fn font_input_errors_are_preserved() {
    assert!(matches!(
        FontCollectionBuilder::new().build(),
        Err(FontCollectionBuildError::NoFonts)
    ));
    assert!(matches!(
        FontCollectionBuilder::new()
            .font_bytes("   ", b"bad".as_slice())
            .build(),
        Err(FontCollectionBuildError::EmptyFamily)
    ));
    assert!(matches!(
        FontCollectionBuilder::new()
            .font_bytes(" Test ", b"".as_slice())
            .build(),
        Err(FontCollectionBuildError::EmptyFont { family }) if family == "Test"
    ));
    assert!(matches!(
        FontCollectionBuilder::new()
            .font_bytes(" Test ", b"bad".as_slice())
            .build(),
        Err(FontCollectionBuildError::FontRejected { family }) if family == "Test"
    ));
}

#[test]
fn oversized_font_is_rejected_before_registration() {
    assert_eq!(MAX_BUNDLED_FONT_BYTES, 100 * 1024 * 1024);
    let bytes = vec![0; MAX_BUNDLED_FONT_BYTES as usize + 1];
    assert!(matches!(
        FontCollectionBuilder::new().font_bytes("Test", bytes).build(),
        Err(FontCollectionBuildError::FontTooLarge { family, limit, actual })
            if family == "Test" && limit == MAX_BUNDLED_FONT_BYTES
                && actual == MAX_BUNDLED_FONT_BYTES + 1
    ));
}

#[test]
fn html_font_collection_lays_out_text_deterministically() {
    let html = b"<html><head><style>body { margin: 0; font-family: 'Bundled Test'; font-size: 16px; }</style></head><body>Hello bundled font</body></html>";
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let fonts = FontCollectionBuilder::new()
            .font_bytes("Bundled Test", FONT)
            .build()
            .expect("bundled font registers");
        let resources = RenderResources::new().fonts(fonts);
        let document = parse_html_with_resources(html.as_slice(), &resources).unwrap();
        let status = layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap();
        let LayoutStatus::Completed(document_layout) = status else {
            panic!("expected complete layout");
        };
        assert!(document_layout.pages().any(|page| {
            page.fragments().any(|fragment| {
                fragment.kind() == FragmentKind::Text
                    && fragment.line_range().is_some_and(|range| !range.is_empty())
            })
        }));
        outputs.push(
            document_layout
                .pages()
                .map(|page| {
                    (
                        page.index(),
                        page.geometry(),
                        page.fragments()
                            .map(|fragment| {
                                (fragment.kind(), fragment.rect(), fragment.line_range())
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(outputs[0], outputs[1]);
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

#[test]
fn a_built_font_set_resolves_the_family_and_the_generics_to_the_bundle() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    // The shodo side resolves the authored family and a generic to the bundle.
    for family in [
        shodo::style::FontFamily::Named("Ahem".to_owned()),
        shodo::style::FontFamily::Generic(shodo::style::GenericFamily::SansSerif),
    ] {
        let query = shodo::font::FontQuery {
            families: vec![family.clone()],
            ..Default::default()
        };
        let matched = fonts
            .collection()
            .match_cluster(&query, "a")
            .unwrap_or_else(|| panic!("{family:?} resolves"));
        let data = fonts.collection().font_data(matched.id).expect("font data");
        assert_eq!(data.data.as_ref(), AHEM, "{family:?}");
    }
}

#[test]
fn a_built_font_set_reports_whether_installed_fonts_are_excluded() {
    let bundled = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    assert!(bundled.is_bundled_only());
    let with_system = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .system_fonts(true)
        .build()
        .expect("fonts");
    assert!(!with_system.is_bundled_only());
}

/// Bytes of the face shodo resolves for `family` and `text`.
fn shodo_face_bytes(
    fonts: &raikiri_html::RenderFonts,
    family: shodo::style::FontFamily,
    text: &str,
) -> Option<Vec<u8>> {
    let query = shodo::font::FontQuery {
        families: vec![family],
        ..Default::default()
    };
    let matched = fonts.collection().match_cluster(&query, text)?;
    let data = fonts.collection().font_data(matched.id)?;
    Some(data.data.as_ref().to_vec())
}

fn ahem_then_noto() -> raikiri_html::RenderFonts {
    FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .font_bytes("Noto Sans Test", FONT.to_vec())
        .build()
        .expect("fonts")
}

#[test]
fn a_bundled_family_resolves_to_its_bytes() {
    let fonts = ahem_then_noto();
    // Noto Sans Test covers a handful of code points; U+0E70 is one of them.
    for (name, bytes, text) in [("Ahem", AHEM, "a"), ("Noto Sans Test", FONT, "\u{0E70}")] {
        let shodo = shodo_face_bytes(
            &fonts,
            shodo::style::FontFamily::Named(name.to_owned()),
            text,
        )
        .expect("shodo resolves the family");
        assert_eq!(shodo, bytes, "{name}: shodo");
    }
}

#[test]
fn every_generic_resolves_to_the_first_registered_face() {
    use shodo::style::GenericFamily as G;
    let fonts = ahem_then_noto();
    for generic in [
        G::Serif,
        G::SansSerif,
        G::Monospace,
        G::SystemUi,
        G::Cursive,
        G::Fantasy,
    ] {
        // Both faces cover the space, so only the generic's order decides.
        let shodo = shodo_face_bytes(&fonts, shodo::style::FontFamily::Generic(generic), " ")
            .expect("shodo resolves the generic");
        assert_eq!(shodo, AHEM, "{generic:?}: shodo");
    }
}

/// `bytes` with the big-endian `value` written at `offset` of the OS/2 table.
fn with_os2_field(bytes: &[u8], offset: usize, value: u16) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let tables = u16::from_be_bytes([out[4], out[5]]) as usize;
    let os2 = (0..tables)
        .map(|index| 12 + index * 16)
        .find(|&record| &out[record..record + 4] == b"OS/2")
        .map(|record| {
            u32::from_be_bytes([
                out[record + 8],
                out[record + 9],
                out[record + 10],
                out[record + 11],
            ]) as usize
        })
        .expect("an OS/2 table");
    out[os2 + offset..os2 + offset + 2].copy_from_slice(&value.to_be_bytes());
    out
}

/// OS/2 `usWeightClass`, `usWidthClass` and `fsSelection` offsets.
const WEIGHT_CLASS: usize = 4;
const WIDTH_CLASS: usize = 6;
const FS_SELECTION: usize = 62;

#[test]
fn the_faces_of_one_family_resolve_by_weight_width_and_style() {
    let bold = with_os2_field(AHEM, WEIGHT_CLASS, 700);
    // Width class 3 is condensed (75%); fsSelection bit 0 is italic.
    let condensed = with_os2_field(AHEM, WIDTH_CLASS, 3);
    let italic = with_os2_field(AHEM, FS_SELECTION, 1);
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Quad", AHEM.to_vec())
        .font_bytes("Quad", bold.clone())
        .font_bytes("Quad", condensed.clone())
        .font_bytes("Quad", italic.clone())
        .build()
        .expect("fonts");
    let cases = [
        ("regular", 400.0_f32, 100.0_f32, false, AHEM),
        ("bold", 700.0, 100.0, false, bold.as_slice()),
        ("condensed", 400.0, 75.0, false, condensed.as_slice()),
        ("italic", 400.0, 100.0, true, italic.as_slice()),
    ];
    for (name, weight, width, is_italic, expected) in cases {
        let query = shodo::font::FontQuery {
            families: vec![shodo::style::FontFamily::Named("Quad".to_owned())],
            weight,
            width,
            style: if is_italic {
                shodo::style::FontStyle::Italic
            } else {
                shodo::style::FontStyle::Normal
            },
            ..Default::default()
        };
        let matched = fonts
            .collection()
            .match_cluster(&query, "a")
            .expect("shodo matches a face");
        let shodo = fonts.collection().font_data(matched.id).expect("data");
        assert!(
            shodo.data.as_ref() == expected,
            "shodo picks another face for {name}"
        );
        assert!(!matched.embolden, "no synthesized bold for {name}");
        assert_eq!(matched.skew, None, "no synthesized slant for {name}");
    }
}

#[test]
fn a_face_with_an_out_of_range_weight_class_is_still_accepted() {
    // A weight class outside the CSS range does not make the face unusable.
    let heavy = with_os2_field(AHEM, WEIGHT_CLASS, 1200);
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Heavy", heavy)
        .build();
    assert!(fonts.is_ok(), "{fonts:?}");
}
