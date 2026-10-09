use super::*;
use std::collections::{HashMap, HashSet};

#[test]
fn map_entries_are_sorted_regardless_of_insertion_order() {
    let mut a = HashMap::new();
    let mut b = HashMap::new();
    for (k, v) in [("--z", 1), ("--a", 2), ("--m", 3)] {
        a.insert(k, v);
    }
    for (k, v) in [("--m", 3), ("--z", 1), ("--a", 2)] {
        b.insert(k, v);
    }
    let canon_a = canonicalize(&format!("{a:?}"));
    assert_eq!(canon_a, canonicalize(&format!("{b:?}")));
    assert_eq!(canon_a, r#"{"--a": 2, "--m": 3, "--z": 1}"#);
}

#[test]
fn struct_fields_keep_their_declared_order() {
    let text = "Point { y: 1, x: 2 }";
    assert_eq!(canonicalize(text), text);
}

#[test]
fn nested_maps_are_sorted_before_their_container() {
    let text = r#"Outer { local: {"b": Some("x"), "a": None}, sets: [{3, 1, 2}] }"#;
    assert_eq!(
        canonicalize(text),
        r#"Outer { local: {"a": None, "b": Some("x")}, sets: [{1, 2, 3}] }"#
    );
}

#[test]
fn literals_are_opaque_to_structure() {
    let text = r#"{"}, {": '{', "a\"b": "c, d"}"#;
    assert_eq!(canonicalize(text), r#"{"a\"b": "c, d", "}, {": '{'}"#);
}

#[test]
fn set_debug_output_is_order_independent() {
    let a: HashSet<u32> = [5, 1, 9].into_iter().collect();
    let b: HashSet<u32> = [9, 5, 1].into_iter().collect();
    assert_eq!(
        canonicalize(&format!("{a:?}")),
        canonicalize(&format!("{b:?}"))
    );
}

#[test]
fn unbalanced_input_is_kept_verbatim() {
    assert_eq!(canonicalize("a } b"), "a } b");
    assert_eq!(canonicalize("a ) b"), "a ) b");
    assert_eq!(canonicalize("{ open"), "{ open");
    assert_eq!(canonicalize("(open"), "(open");
    assert_eq!(canonicalize("\"unterminated"), "\"unterminated");
    assert_eq!(canonicalize("\"trailing escape\\"), "\"trailing escape\\");
}

#[test]
fn struct_fields_rejects_non_struct_text() {
    assert!(struct_fields("two words { a: 1 }").is_none());
    assert!(struct_fields(" { a: 1 }").is_none());
    assert!(struct_fields("V { a: 1").is_none());
}

#[test]
fn struct_fields_split_top_level_only() {
    let (name, fields) = struct_fields("V { a: [1, 2], b: W { c: 3, d: 4 } }").expect("struct");
    assert_eq!(name, "V");
    assert_eq!(fields, vec!["a: [1, 2]", "b: W { c: 3, d: 4 }"]);
    assert!(struct_fields("[1, 2]").is_none());
}
