use std::collections::BTreeSet;

use super::supported_property_names;
use crate::property::is_supported_property_name;

/// `parse_value`'s dispatch match, `crates/raikiri-style/src/property/parse/mod.rs`:
/// the scanned block starts right after `match normalized_name.as_str() {`
/// and ends at that match's closing arm and braces (`_ => None,` / `}` /
/// `}`, at the 8/4/0-space indents the source uses for them).
const PARSE_MOD_SOURCE: &str = include_str!("../parse/mod.rs");
const PARSE_VALUE_START_MARKER: &str = "match normalized_name.as_str() {\n";
const PARSE_VALUE_END_MARKER: &str = "\n        _ => None,\n    }\n}";

/// `property_key_for_name`'s match, `crates/raikiri-style/src/property/types.rs`:
/// the scanned block starts right after `Some(match normalized_name.as_str() {`
/// and ends at that match's closing arm and braces (`_ => return None,` /
/// `})` / `}`).
const TYPES_SOURCE: &str = include_str!("../types.rs");
const PROPERTY_KEY_START_MARKER: &str = "Some(match normalized_name.as_str() {\n";
const PROPERTY_KEY_END_MARKER: &str = "\n        _ => return None,\n    })\n}";

/// Every `"name"` (or `"a" | "b" | ...`) that is the pattern of a match arm
/// on `line` -- i.e. `line`, once its leading whitespace is trimmed, starts
/// with a quoted string and reaches `=>` before anything else follows. This
/// is what a `^\s*((?:"[a-z-]+"\s*\|\s*)*"[a-z-]+")\s*=>` regex would match
/// per line; every arm in both matches this test scans is written on one
/// line, so a per-line scan is exact without pulling in a regex dependency
/// this crate otherwise has no use for.
fn arm_names_in_line(line: &str) -> Vec<&str> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('"') {
        return Vec::new();
    }
    let Some(arrow) = trimmed.find("=>") else {
        return Vec::new();
    };
    trimmed[..arrow]
        .split('|')
        .filter_map(|part| {
            let part = part.trim();
            part.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        })
        .collect()
}

/// Every match-arm property name inside the block between `start_marker`
/// and `end_marker` in `source`.
fn arm_names_in_block(source: &str, start_marker: &str, end_marker: &str) -> BTreeSet<String> {
    let after_start = source
        .find(start_marker)
        .map(|i| &source[i + start_marker.len()..])
        .unwrap_or_else(|| panic!("start marker not found: {start_marker:?}"));
    let block = after_start
        .find(end_marker)
        .map(|i| &after_start[..i])
        .unwrap_or_else(|| panic!("end marker not found: {end_marker:?}"));
    block
        .lines()
        .flat_map(arm_names_in_line)
        .map(str::to_owned)
        .collect()
}

fn parse_value_dispatch_names() -> BTreeSet<String> {
    arm_names_in_block(
        PARSE_MOD_SOURCE,
        PARSE_VALUE_START_MARKER,
        PARSE_VALUE_END_MARKER,
    )
}

fn property_key_for_name_names() -> BTreeSet<String> {
    arm_names_in_block(
        TYPES_SOURCE,
        PROPERTY_KEY_START_MARKER,
        PROPERTY_KEY_END_MARKER,
    )
}

#[test]
fn every_parse_value_dispatch_arm_is_listed() {
    let names = parse_value_dispatch_names();
    assert!(
        names.len() > 100,
        "extraction found too few names to be real: {names:?}"
    );
    let supported: BTreeSet<&str> = supported_property_names().iter().copied().collect();
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !supported.contains(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "supported_property_names() is missing names parse_value dispatches on: {missing:?}"
    );
}

#[test]
fn every_property_key_for_name_arm_is_listed() {
    let names = property_key_for_name_names();
    assert!(
        names.len() > 100,
        "extraction found too few names to be real: {names:?}"
    );
    let supported: BTreeSet<&str> = supported_property_names().iter().copied().collect();
    let missing: Vec<&String> = names
        .iter()
        .filter(|n| !supported.contains(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "supported_property_names() is missing names property_key_for_name recognizes: {missing:?}"
    );
}

/// Soundness: every listed name really is one `parse_value` recognizes.
///
/// Probing `parse_value` with each property's "typical value" (e.g. the
/// CSS-wide keyword `initial`) is not a sound check here: `parse_value`
/// does not generally accept CSS-wide keywords itself (see `parse/mod.rs`,
/// e.g. the `z-index` and `float` dispatch arms' doc comments --
/// `inherit`/`initial`/... are usually resolved one layer up, in the
/// cascade, not per property; `counter-reset` and `border-radius` are the
/// exceptions that do parse `inherit` directly), so no single probe value
/// parses successfully across the ~220 unrelated grammars this list
/// covers.
///
/// Instead, each name is checked two ways: [`is_supported_property_name`]
/// is a real runtime call into `property_key_for_name` (true for every name
/// except the handful of shorthands documented on
/// [`supported_property_names`] that have no `PropertyKey` of their own);
/// for exactly those few, this falls back to the same dispatch-match scan
/// the drift tests above use -- a name is "recognized by `parse_value`"
/// exactly when it has a quoted arm in that match, which is the fact
/// establishing soundness for them (an arm that never actually parses any
/// value would be a bug caught by this crate's per-property parser tests,
/// not a naming drift this list could paper over).
#[test]
fn every_listed_name_is_recognized() {
    let dispatch_names = parse_value_dispatch_names();
    let missing: Vec<&&str> = supported_property_names()
        .iter()
        .filter(|n| !is_supported_property_name(n) && !dispatch_names.contains(**n))
        .collect();
    assert!(
        missing.is_empty(),
        "supported_property_names() lists a name parse_value does not dispatch on: {missing:?}"
    );
}

#[test]
fn supported_property_names_is_sorted_lowercase_and_excludes_custom_properties() {
    let names = supported_property_names();
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        names,
        sorted.as_slice(),
        "supported_property_names() must be sorted"
    );
    for name in names {
        assert_eq!(
            *name,
            name.to_ascii_lowercase(),
            "supported_property_names() must be lowercase: {name}"
        );
        assert!(
            !name.starts_with("--"),
            "supported_property_names() must exclude custom properties: {name}"
        );
    }
}
