//! Checks on the corpus tables themselves.

use std::collections::BTreeSet;

use crate::property::{
    is_supported_property_name, property_key_for_name, supported_property_names,
};

use super::DOMAINS;
use super::excluded::{ALIASES_COVERED_BY_TARGET, EXCLUDED, KEYLESS};

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

/// Every name `parse_value` dispatches on is classified exactly once: in the
/// corpus, in [`EXCLUDED`], in [`ALIASES_COVERED_BY_TARGET`] (only names that
/// share their target's parse arm), or, for names without a `PropertyKey`,
/// in [`KEYLESS`]. Names are compared by name, not by key: an alias with a
/// parse arm of its own (`page-break-before`, logical margins) is not covered
/// by its target, and neither is a shorthand that shares a longhand's key
/// (`text-wrap` shares `PropertyKey::TextWrap` with `text-wrap-mode`).
#[test]
fn every_supported_name_is_in_the_corpus_or_classified() {
    let corpus: BTreeSet<&str> = corpus_names().collect();
    for name in &corpus {
        assert!(
            property_key_for_name(name).is_some(),
            "{name}: corpus entry without a PropertyKey"
        );
    }

    let mut classified: BTreeSet<&str> = corpus.clone();
    let mut classify = |name: &'static str, list: &str| {
        assert!(
            is_supported_property_name(name),
            "{list} {name}: unsupported"
        );
        assert!(classified.insert(name), "{list} {name}: classified twice");
    };
    for (name, reason) in EXCLUDED {
        assert!(!reason.is_empty(), "excluded {name}: no reason");
        assert!(
            property_key_for_name(name).is_some(),
            "excluded {name}: no PropertyKey (list it in KEYLESS)"
        );
        classify(name, "excluded");
    }
    for (name, reason) in KEYLESS {
        assert!(!reason.is_empty(), "keyless {name}: no reason");
        classify(name, "keyless");
    }
    let excluded: BTreeSet<&str> = EXCLUDED.iter().map(|(name, _)| *name).collect();
    for (alias, target) in ALIASES_COVERED_BY_TARGET {
        assert!(
            corpus.contains(target) || excluded.contains(target),
            "alias {alias}: target {target} is neither in the corpus nor excluded"
        );
        assert_eq!(
            property_key_for_name(alias),
            property_key_for_name(target),
            "alias {alias}: key differs from {target}"
        );
        classify(alias, "alias");
    }

    let supported: BTreeSet<&str> = supported_property_names().iter().copied().collect();
    let missing: Vec<_> = supported.difference(&classified).collect();
    assert!(
        missing.is_empty(),
        "neither in the corpus nor classified: {missing:?}"
    );

    let keyless: BTreeSet<&str> = supported
        .iter()
        .copied()
        .filter(|name| property_key_for_name(name).is_none())
        .collect();
    let listed: BTreeSet<&str> = KEYLESS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        listed, keyless,
        "KEYLESS must list exactly the keyless names"
    );
}
