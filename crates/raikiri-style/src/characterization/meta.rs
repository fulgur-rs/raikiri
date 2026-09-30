//! Checks on the corpus tables themselves.

use std::collections::BTreeSet;

use crate::property::is_supported_property_name;

use super::DOMAINS;

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
