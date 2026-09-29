//! Tests for the `longhands!` declaration table (`property/longhands.rs`).

use super::*;
use cssparser::{Parser, ParserInput};

#[test]
fn longhand_names_are_supported_property_names() {
    for name in LONGHAND_NAMES {
        assert!(
            is_supported_property_name(name),
            "{name} is declared in longhands! but missing from supported_property_names()"
        );
    }
}

#[test]
fn longhand_names_are_lowercase_and_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for name in LONGHAND_NAMES {
        assert_eq!(*name, name.to_ascii_lowercase(), "{name} must be lowercase");
        assert!(seen.insert(*name), "{name} is declared twice");
    }
}

fn parse_decl(name: &str, source: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_value(name, &mut parser)
}

#[test]
fn isolation_is_declared_in_the_table() {
    assert!(LONGHAND_NAMES.contains(&"isolation"));
    assert_eq!(Isolation::INITIAL, Isolation::Auto);
    assert_eq!(
        Isolation::from_css_ident("ISOLATE"),
        Some(Isolation::Isolate)
    );
    assert_eq!(Isolation::Isolate.as_css_str(), "isolate");
    for &variant in Isolation::ALL {
        assert_eq!(
            Isolation::from_css_ident(variant.as_css_str()),
            Some(variant)
        );
    }
}

#[test]
fn table_property_name_lookup_is_ascii_case_insensitive() {
    assert_eq!(
        property_key_for_name("ISOLATION"),
        Some(PropertyKey::Isolation)
    );
    assert_eq!(
        parse_decl("Isolation", "isolate"),
        Some(PropertyValue::Isolation(Isolation::Isolate))
    );
}

#[test]
fn table_property_value_key_round_trips() {
    assert_eq!(
        PropertyValue::Isolation(Isolation::Auto).key(),
        PropertyKey::Isolation
    );
}

#[test]
fn table_property_var_reference_defers_with_its_key() {
    match parse_decl("isolation", "var(--x)") {
        Some(PropertyValue::Deferred(deferred)) => {
            assert_eq!(
                PropertyValue::Deferred(deferred).key(),
                PropertyKey::Isolation
            );
        }
        other => panic!("expected a deferred value, got {other:?}"),
    }
}

#[test]
fn table_keyword_property_rejects_non_keywords() {
    assert_eq!(parse_decl("isolation", ""), None);
    assert_eq!(parse_decl("isolation", "1px"), None);
    assert_eq!(parse_decl("isolation", "bogus"), None);
}
