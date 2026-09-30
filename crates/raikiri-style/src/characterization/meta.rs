//! Checks on the corpus tables themselves.

use std::collections::BTreeSet;

use crate::property::{
    PropertyKey, is_supported_property_name, property_key_for_name, supported_property_names,
};

use super::DOMAINS;
use super::excluded::EXCLUDED;

fn corpus_names() -> impl Iterator<Item = &'static str> {
    DOMAINS
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|entry| entry.name))
}

#[test]
fn every_corpus_property_is_supported() {
    let unsupported: Vec<_> = corpus_names()
        .filter(|name| !is_supported_property_name(name))
        .collect();
    assert!(unsupported.is_empty(), "unsupported names: {unsupported:?}");
}

#[test]
fn corpus_has_no_duplicate_property_or_sample() {
    let mut seen = BTreeSet::new();
    let duplicates: Vec<_> = corpus_names().filter(|name| !seen.insert(*name)).collect();
    assert!(duplicates.is_empty(), "duplicate entries: {duplicates:?}");

    for (_, entries) in DOMAINS {
        for entry in *entries {
            let mut seen = BTreeSet::new();
            let duplicates: Vec<_> = entry
                .all_samples()
                .into_iter()
                .filter(|sample| !seen.insert(sample.clone()))
                .collect();
            assert!(
                duplicates.is_empty(),
                "{}: duplicate samples {duplicates:?}",
                entry.name
            );
        }
    }
}

#[test]
fn domains_are_unique_and_sorted() {
    let names: Vec<_> = DOMAINS.iter().map(|(name, _)| *name).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted);
}

/// Every property with a `PropertyKey` is either in the corpus or listed in
/// [`EXCLUDED`] with a reason. Properties are compared by key, so an alias
/// (`word-wrap`, `page-break-before`, logical names, ...) counts as its
/// target; names without a key (shorthands that fully expand while parsing)
/// are not checked.
#[test]
fn every_keyed_property_is_in_the_corpus_or_excluded() {
    let key_of = |name: &str| property_key_for_name(name);
    let corpus_keys: Vec<PropertyKey> = corpus_names()
        .map(|name| key_of(name).unwrap_or_else(|| panic!("{name}: no PropertyKey")))
        .collect();

    let mut excluded_keys = Vec::new();
    for (name, reason) in EXCLUDED {
        assert!(
            is_supported_property_name(name),
            "excluded {name}: unsupported"
        );
        assert!(!reason.is_empty(), "excluded {name}: no reason");
        let key = key_of(name).unwrap_or_else(|| panic!("excluded {name}: no PropertyKey"));
        assert!(
            !corpus_keys.contains(&key),
            "{name} is both in the corpus and excluded"
        );
        assert!(!excluded_keys.contains(&key), "{name}: key excluded twice");
        excluded_keys.push(key);
    }

    let missing: BTreeSet<&str> = supported_property_names()
        .iter()
        .copied()
        .filter(|name| {
            key_of(name)
                .is_some_and(|key| !corpus_keys.contains(&key) && !excluded_keys.contains(&key))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "neither in the corpus nor excluded: {missing:?}"
    );
}
