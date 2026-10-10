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
    let canon_a = canonicalize(&format!("{a:?}"), None);
    assert_eq!(canon_a, canonicalize(&format!("{b:?}"), None));
    assert_eq!(canon_a, r#"{"--a": 2, "--m": 3, "--z": 1}"#);
}

#[test]
fn struct_fields_keep_their_declared_order() {
    let text = "Point { y: 1, x: 2 }";
    assert_eq!(canonicalize(text, None), text);
}

#[test]
fn nested_maps_are_sorted_before_their_container() {
    let text = r#"Outer { local: {"b": Some("x"), "a": None}, sets: [{3, 1, 2}] }"#;
    assert_eq!(
        canonicalize(text, None),
        r#"Outer { local: {"a": None, "b": Some("x")}, sets: [{1, 2, 3}] }"#
    );
}

#[test]
fn literals_are_opaque_to_structure() {
    let text = r#"{"}, {": '{', "a\"b": "c, d"}"#;
    assert_eq!(canonicalize(text, None), r#"{"a\"b": "c, d", "}, {": '{'}"#);
}

#[test]
fn set_debug_output_is_order_independent() {
    let a: HashSet<u32> = [5, 1, 9].into_iter().collect();
    let b: HashSet<u32> = [9, 5, 1].into_iter().collect();
    assert_eq!(
        canonicalize(&format!("{a:?}"), None),
        canonicalize(&format!("{b:?}"), None)
    );
}

#[test]
fn unbalanced_input_is_kept_verbatim() {
    assert_eq!(canonicalize("a } b", None), "a } b");
    assert_eq!(canonicalize("a ) b", None), "a ) b");
    assert_eq!(canonicalize("{ open", None), "{ open");
    assert_eq!(canonicalize("(open", None), "(open");
    assert_eq!(canonicalize("\"unterminated", None), "\"unterminated");
    assert_eq!(
        canonicalize("\"trailing escape\\", None),
        "\"trailing escape\\"
    );
}

#[test]
fn struct_fields_rejects_non_struct_text() {
    assert!(struct_fields("two words { a: 1 }").is_none());
    assert!(struct_fields(" { a: 1 }").is_none());
    assert!(struct_fields("V { a: 1").is_none());
}

#[test]
fn fields_of_a_dropped_struct_are_left_out_at_any_depth() {
    let text = r#"Outer { env: Env { local: {"b": 1, "a": 2}, has_parent: true }, x: Some(Inner { y: 1, env: Env { local: {} } }), envs: [Env { local: {} }] }"#;
    // Only fields go: a literal in a list, which is no field, stays.
    assert_eq!(
        canonicalize(text, Some("Env")),
        "Outer { x: Some(Inner { y: 1 }), envs: [Env { local: {} }] }"
    );
    // A struct left without fields keeps its braces.
    assert_eq!(
        canonicalize("Outer { env: Env { a: 1 } }", Some("Env")),
        "Outer {}"
    );
    // Only a literal of that very name, and nothing in a string.
    let kept = r#"Outer { a: EnvLike { b: 1 }, c: "env: Env { d }" }"#;
    assert_eq!(canonicalize(kept, Some("Env")), kept);
    // Without a name, every field stays.
    assert!(canonicalize(text, None).starts_with(r#"Outer { env: Env { local: {"a": 2, "b": 1}"#));
}

#[test]
fn struct_fields_split_top_level_only() {
    let (name, fields) = struct_fields("V { a: [1, 2], b: W { c: 3, d: 4 } }").expect("struct");
    assert_eq!(name, "V");
    assert_eq!(fields, vec!["a: [1, 2]", "b: W { c: 3, d: 4 }"]);
    assert!(struct_fields("[1, 2]").is_none());
}
