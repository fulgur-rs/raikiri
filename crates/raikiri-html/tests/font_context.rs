//! Consumer-supplied font construction and document layout contracts.

use parley::fontique::GenericFamily;
use raikiri_html::{
    BundledFont, FontContextBuildError, FontContextBuilder, FragmentKind, LayoutConfig,
    LayoutOptions, LayoutStatus, MAX_BUNDLED_FONT_BYTES, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

const FONT: &[u8] = include_bytes!("data/NotoSansTest-Regular.ttf");

#[test]
fn font_input_errors_are_preserved() {
    assert!(matches!(
        FontContextBuilder::new().build(),
        Err(FontContextBuildError::NoFonts)
    ));
    assert!(matches!(
        FontContextBuilder::new()
            .font_bytes("   ", b"bad".as_slice())
            .build(),
        Err(FontContextBuildError::EmptyFamily)
    ));
    assert!(matches!(
        FontContextBuilder::new()
            .font_bytes(" Test ", b"".as_slice())
            .build(),
        Err(FontContextBuildError::EmptyFont { family }) if family == "Test"
    ));
    assert!(matches!(
        FontContextBuilder::new()
            .font_bytes(" Test ", b"bad".as_slice())
            .build(),
        Err(FontContextBuildError::FontRejected { family }) if family == "Test"
    ));
}

#[test]
fn oversized_font_is_rejected_before_registration() {
    assert_eq!(MAX_BUNDLED_FONT_BYTES, 100 * 1024 * 1024);
    let bytes = vec![0; MAX_BUNDLED_FONT_BYTES as usize + 1];
    assert!(matches!(
        FontContextBuilder::new().font_bytes("Test", bytes).build(),
        Err(FontContextBuildError::FontTooLarge { family, limit, actual })
            if family == "Test" && limit == MAX_BUNDLED_FONT_BYTES
                && actual == MAX_BUNDLED_FONT_BYTES + 1
    ));
}

#[test]
fn generic_fallback_uses_only_bundle_order() {
    let mut context = FontContextBuilder::new()
        .font(BundledFont::new("First", FONT))
        .font_bytes("Second", FONT)
        .build()
        .expect("bundled fonts register");
    let first = context.collection.family_id("First").unwrap();
    let second = context.collection.family_id("Second").unwrap();
    assert_ne!(first, second);
    for generic in [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
    ] {
        assert_eq!(
            context
                .collection
                .generic_families(generic)
                .collect::<Vec<_>>(),
            [first, second],
        );
    }
}

#[test]
fn html_font_context_lays_out_text_deterministically() {
    let html = b"<html><head><style>body { margin: 0; font-family: 'Bundled Test'; font-size: 16px; }</style></head><body>Hello bundled font</body></html>";
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let context = FontContextBuilder::new()
            .font_bytes("Bundled Test", FONT)
            .build()
            .expect("bundled font registers");
        let resources = RenderResources::new().font_context(context);
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
