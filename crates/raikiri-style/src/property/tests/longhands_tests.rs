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

#[test]
fn object_fit_is_declared_through_a_parse_fn() {
    assert!(LONGHAND_NAMES.contains(&"object-fit"));
    assert_eq!(
        property_key_for_name("object-fit"),
        Some(PropertyKey::ObjectFit)
    );
    assert_eq!(
        parse_decl("object-fit", "contain"),
        Some(PropertyValue::ObjectFit(ObjectFit::Contain))
    );
    assert_eq!(parse_decl("object-fit", "bogus"), None);
    assert_eq!(
        PropertyValue::ObjectFit(ObjectFit::Fill).key(),
        PropertyKey::ObjectFit
    );
}

#[test]
fn longhand_value_pat_matches_table_variants_only() {
    use crate::property::longhand_value_pat;
    let table = [
        PropertyValue::Isolation(Isolation::Isolate),
        PropertyValue::ObjectFit(ObjectFit::Contain),
    ];
    for value in &table {
        assert!(matches!(value, longhand_value_pat!()), "{value:?}");
    }
    let other = PropertyValue::Opacity(0.5);
    assert!(!matches!(&other, longhand_value_pat!()));
}

#[test]
fn every_table_entry_has_a_sample_of_its_own_variant() {
    // `sample_for` in the page cascade tests is built from the same table;
    // here we only pin that a sample maps back to the declared key.
    for (name, value) in crate::property::longhand_samples() {
        assert_eq!(
            property_key_for_name(name),
            Some(value.key()),
            "sample for {name} does not round-trip through its key"
        );
    }
}

#[test]
fn table_structs_start_at_the_declared_initial_values() {
    let specified = SpecifiedTable::initial();
    assert_eq!(specified.isolation, Isolation::Auto);
    assert_eq!(specified.object_fit, ObjectFit::Fill);
    let computed = ComputedTable::initial();
    assert_eq!(computed.isolation, Isolation::Auto);
    assert_eq!(computed.object_fit, ObjectFit::Fill);
}

#[test]
fn table_apply_sets_the_matching_field() {
    let mut table = SpecifiedTable::initial();
    table.apply(PropertyValue::Isolation(Isolation::Isolate));
    table.apply(PropertyValue::ObjectFit(ObjectFit::Cover));
    assert_eq!(table.isolation, Isolation::Isolate);
    assert_eq!(table.object_fit, ObjectFit::Cover);
}

#[test]
fn table_absolutize_copies_as_specified_values() {
    let mut table = SpecifiedTable::initial();
    table.apply(PropertyValue::Isolation(Isolation::Isolate));
    let computed = table.absolutize();
    assert_eq!(computed.isolation, Isolation::Isolate);
    assert_eq!(computed.object_fit, ObjectFit::Fill);
}

#[test]
fn non_inherited_table_fields_reset_to_initial_in_a_child() {
    let mut parent = ComputedTable::initial();
    parent.isolation = Isolation::Isolate;
    let child = SpecifiedTable::inherit_from(&parent);
    assert_eq!(child.isolation, Isolation::Auto);
}

#[test]
fn values_expose_table_fields_through_deref() {
    let mut specified = crate::specified::SpecifiedValues::initial();
    specified.isolation = Isolation::Isolate;
    assert_eq!(specified.longhands.isolation, Isolation::Isolate);
    let computed = crate::computed::ComputedValues::initial();
    assert_eq!(computed.isolation, Isolation::Auto);
    assert_eq!(computed.longhands.object_fit, ObjectFit::Fill);
}
