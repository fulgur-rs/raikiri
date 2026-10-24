use super::*;
use crate::cascade::rollback::Rollback;
use crate::property::PropertyValue;

const SPREAD_KEYWORDS: [CssWideKeyword; 3] = [
    CssWideKeyword::Initial,
    CssWideKeyword::Inherit,
    CssWideKeyword::Unset,
];

fn keys(keyword: CssWideKeyword) -> Vec<PropertyKey> {
    declarations(keyword)
        .iter()
        .map(|declaration| declaration.key)
        .collect()
}

#[test]
fn every_spread_keyword_resets_the_same_properties_once() {
    let initial = keys(CssWideKeyword::Initial);
    for keyword in SPREAD_KEYWORDS {
        let keys = keys(keyword);
        assert_eq!(keys, initial, "{keyword:?}");
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "{keyword:?} repeats a key");
        for declaration in declarations(keyword) {
            assert_eq!(
                declaration.rollback,
                Rollback::None,
                "{:?}",
                declaration.key
            );
            assert!(!declaration.important, "{:?}", declaration.key);
            assert!(declaration.logical.is_none(), "{:?}", declaration.key);
        }
    }
}

#[test]
fn the_spread_covers_ordinary_properties_and_leaves_the_exempt_ones() {
    let keys = keys(CssWideKeyword::Inherit);
    for key in [
        PropertyKey::Color,
        PropertyKey::Display,
        PropertyKey::Width,
        PropertyKey::MarginTop,
        PropertyKey::PaddingLeft,
        PropertyKey::BorderTopWidth,
        PropertyKey::BorderRadiusTopLeft,
        PropertyKey::FontSize,
        PropertyKey::LineHeight,
        PropertyKey::WritingMode,
        PropertyKey::Quotes,
        PropertyKey::Page,
        PropertyKey::Fill,
        PropertyKey::Stroke,
    ] {
        assert!(keys.contains(&key), "{key:?} is reset by all");
    }
    // `direction` and `unicode-bidi` are exempt (CSS Cascade 4 section 3.1),
    // and the rest are not properties of their own.
    for key in [
        PropertyKey::Direction,
        PropertyKey::UnicodeBidi,
        PropertyKey::All,
        PropertyKey::Custom,
        PropertyKey::InlineSize,
        PropertyKey::BlockSize,
        PropertyKey::MinInlineSize,
        PropertyKey::MinBlockSize,
        PropertyKey::MaxInlineSize,
        PropertyKey::MaxBlockSize,
    ] {
        assert!(!keys.contains(&key), "{key:?} is not spread");
    }
}

#[test]
fn a_property_takes_the_keyword_under_its_own_name() {
    // `background` sorts before `background-color` and expands to the same
    // key; the marker that stays names the longhand.
    for keyword in SPREAD_KEYWORDS {
        let color = declarations(keyword)
            .iter()
            .find(|declaration| declaration.key == PropertyKey::BackgroundColor)
            .expect("background-color is spread");
        match &color.value {
            PropertyValue::Deferred(marker) => {
                assert_eq!(&*marker.property, "background-color");
                assert_eq!(marker.css_wide_keyword(), Some(keyword));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn the_rollback_keywords_spread_nothing() {
    assert!(declarations(CssWideKeyword::Revert).is_empty());
    assert!(declarations(CssWideKeyword::RevertLayer).is_empty());
}

#[test]
fn only_a_known_few_properties_refuse_the_keywords() {
    // A property that does not accept the keywords alone is not reset by
    // `all` either. `text-align-all` has no computed field, so the keywords
    // have no effect there.
    let mut refused: Vec<&str> = Vec::new();
    for name in supported_property_names() {
        let css = format!("{name}: initial");
        let mut input = ParserInput::new(&css);
        let mut parser = Parser::new(&mut input);
        let declarations =
            parse_declaration_block_within(&mut parser, &[], usize::MAX).unwrap_or_default();
        if declarations.is_empty() {
            refused.push(name);
        }
    }
    assert_eq!(refused, ["text-align-all"]);
}
