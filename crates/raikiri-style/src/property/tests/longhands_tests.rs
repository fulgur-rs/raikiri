//! Tests for the `longhands!` declaration table (`property/longhands.rs`).

use super::*;

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
