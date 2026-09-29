use quote::ToTokens as _;
use syn::Ident;

use super::{Entry, Lift, Value, build, is_css_name};
use crate::diag::Errors;
use crate::parse::parse_block;

fn build_str(src: &str, hand_written: &[&str]) -> (Vec<Entry>, Vec<String>) {
    let mut errors = Errors::default();
    let raw = parse_block(src.parse().expect("test input tokenizes"), &mut errors);
    let hand_written: Vec<Ident> = hand_written
        .iter()
        .map(|name| Ident::new(name, proc_macro2::Span::call_site()))
        .collect();
    let entries = build(raw, &hand_written, &mut errors);
    let messages = errors.into_vec().iter().map(|e| e.to_string()).collect();
    (entries, messages)
}

fn text(tokens: Option<&proc_macro2::TokenStream>) -> String {
    tokens.map(|t| t.to_string()).unwrap_or_default()
}

#[test]
fn keyword_entry_defaults() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "text-wrap-mode" => TextWrapMode { keywords: [Wrap, NoWrap, Pre2 = "pre-2"], initial: Wrap, inherited: yes },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    let entry = &entries[0];
    assert_eq!(entry.field, "text_wrap_mode");
    assert!(entry.inherited);
    let Value::Keywords(list) = &entry.value else {
        panic!("expected keywords");
    };
    let spellings: Vec<_> = list.iter().map(|k| k.css.value()).collect();
    assert_eq!(spellings, ["wrap", "no-wrap", "pre-2"]);
    assert_eq!(text(entry.initial.as_ref()), "TextWrapMode :: Wrap");
    // The sample defaults to the first keyword that is not the initial one.
    assert_eq!(text(entry.sample.as_ref()), "TextWrapMode :: NoWrap");
    assert!(entry.compute.is_none());
    assert!(entry.computed_ty.is_none());
    assert!(entry.name_listed);
}

#[test]
fn default_sample_skips_an_initial_value_that_is_not_the_first_keyword() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A { keywords: [X, Y], initial: A::X, inherited: no },
        /// Docs.
        "b" => B { keywords: [X, Y], initial: Y, inherited: no },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(text(entries[0].sample.as_ref()), "A :: Y");
    assert_eq!(text(entries[1].sample.as_ref()), "B :: X");
}

#[test]
fn parsed_entry_defaults() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: ObjectFit::Contain },
        /// Docs.
        "opacity" => Opacity: f32 { initial: 1.0, inherited: no, parse: p, compute: clamp, sample: 2.0, field: alpha },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    let object_fit = &entries[0];
    let Value::Parsed { ty, parse } = &object_fit.value else {
        panic!("expected a parsed value");
    };
    // The value type defaults to the variant name.
    assert_eq!(ty.to_token_stream().to_string(), "ObjectFit");
    assert!(parse.is_some());
    // A bare identifier is an associated item of the value type; a path is
    // used as written.
    assert_eq!(text(object_fit.initial.as_ref()), "< ObjectFit > :: Fill");
    assert_eq!(text(object_fit.sample.as_ref()), "ObjectFit :: Contain");

    let opacity = &entries[1];
    assert_eq!(opacity.field, "alpha");
    assert_eq!(text(opacity.initial.as_ref()), "1.0");
    assert_eq!(opacity.compute.to_token_stream().to_string(), "clamp");
}

#[test]
fn computed_via_sets_the_type_hook_and_lift() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A: L { initial: L::Z, inherited: yes, parse: p, computed: Px via to_px, sample: L::O },
        /// Docs.
        "b" => B: L { initial: L::Z, inherited: no, parse: p, computed: Px via to_px, lift: from_px, sample: L::O },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(entries[0].computed_ty.to_token_stream().to_string(), "Px");
    assert_eq!(entries[0].compute.to_token_stream().to_string(), "to_px");
    assert!(matches!(entries[0].lift, Lift::Into));
    let Lift::Path(lift) = &entries[1].lift else {
        panic!("expected `lift: from_px`");
    };
    assert_eq!(lift.to_token_stream().to_string(), "from_px");
}

#[test]
fn a_failed_entry_is_still_declared_with_its_usable_parts() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A { keywords: [X, Y], initial: Q, inherited: no, animatable: yes },
        "#,
        &[],
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].initial.is_none());
    assert_eq!(text(entries[0].sample.as_ref()), "A :: X");
}

#[test]
fn duplicates_drop_the_later_entry_except_for_names() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A { keywords: [X, Y], initial: X, inherited: no },
        /// Docs.
        "a" => B { keywords: [X, Y], initial: X, inherited: no },
        /// Docs.
        "c" => A { keywords: [X, Y], initial: X, inherited: no },
        /// Docs.
        "d" => D { keywords: [X, Y], initial: X, inherited: no, field: a },
        /// Docs.
        "color" => Color { keywords: [X, Y], initial: X, inherited: no },
        "#,
        &["Color"],
    );
    assert_eq!(errors.len(), 4, "{errors:?}");
    let variants: Vec<_> = entries.iter().map(|e| e.variant.to_string()).collect();
    assert_eq!(variants, ["A", "B"]);
    assert!(entries[0].name_listed);
    assert!(!entries[1].name_listed);
}

#[test]
fn css_names_are_well_formed_identifiers() {
    for good in ["opacity", "object-fit", "-webkit-line-clamp", "x2", "a-2b"] {
        assert!(is_css_name(good), "{good}");
    }
    for bad in [
        "",
        "-",
        "--",
        "a-",
        "a--b",
        "--custom",
        "2d",
        "-2d",
        "Object-fit",
        "a_b",
        "a b",
    ] {
        assert!(!is_css_name(bad), "{bad}");
    }
}

#[test]
fn a_keywords_initial_written_as_a_path_drives_the_default_sample() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Isolation::Isolate, inherited: no },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(text(entries[0].initial.as_ref()), "Isolation :: Isolate");
    assert_eq!(text(entries[0].sample.as_ref()), "Isolation :: Auto");
}

#[test]
fn acronym_variants_get_readable_default_fields() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "html-mode" => HTMLMode { keywords: [On, Off], initial: On, inherited: no },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(entries[0].field, "html_mode");
}

#[test]
fn a_malformed_key_is_not_reported_again_by_related_rules() {
    // A malformed `computed:` with `lift:`, and a malformed `compute:` with
    // `computed: .. via ..`: one error each.
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A: L { initial: L::Z, inherited: no, parse: p, computed: Px, lift: l, sample: L::O },
        /// Docs.
        "b" => B: L { initial: L::Z, inherited: no, parse: p, compute: |x| x, computed: Px via to_px, sample: L::O },
        /// Docs.
        "c" => C: L { initial: L::Z, inherited: no, parse: p, computed: Px via to_px, lift: |x| x, sample: L::O },
        "#,
        &[],
    );
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(matches!(entries[2].lift, Lift::Broken));
}

#[test]
fn reserved_alias_names_are_rejected_as_value_types() {
    let (_, errors) = build_str(
        r#"
        /// Docs.
        "a" => A: Specified { initial: X, inherited: no, parse: p, sample: Y },
        /// Docs.
        "b" => B: f32 { initial: 1.0, inherited: no, parse: p, computed: Computed via c, sample: 2.0 },
        /// Docs.
        "c" => Property { keywords: [X, Y], initial: X, inherited: no },
        /// Docs.
        "d" => D: crate::Specified { initial: X, inherited: no, parse: p, sample: Y },
        "#,
        &[],
    );
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(
        errors
            .iter()
            .all(|e| e.contains("would resolve to the entry's own type alias"))
    );
}

#[test]
fn a_duplicate_name_across_blocks_is_reported() {
    let mut errors = Errors::default();
    let mut raw = parse_block(
        r#"/// Docs.
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },"#
            .parse()
            .expect("tokenizes"),
        &mut errors,
    );
    raw.extend(parse_block(
        r#"/// Docs.
        "isolation" => Other { keywords: [Auto, Isolate], initial: Auto, inherited: no },"#
            .parse()
            .expect("tokenizes"),
        &mut errors,
    ));
    let entries = build(raw, &[], &mut errors);
    let messages: Vec<String> = errors.into_vec().iter().map(|e| e.to_string()).collect();
    assert_eq!(
        messages,
        ["longhand \"isolation\" is already declared by the `Isolation` entry"]
    );
    assert_eq!(entries.len(), 2);
    assert!(!entries[1].name_listed);
}

fn derive_names(entry: &Entry) -> Vec<String> {
    entry
        .derives
        .iter()
        .map(|p| p.to_token_stream().to_string())
        .collect()
}

#[test]
fn derive_appends_paths_and_default_marks_the_initial_keyword() {
    let (entries, errors) = build_str(
        r#"
        /// Docs.
        "a" => A { keywords: [X, Y, Z], derive: [Hash, Default], initial: Y, inherited: no },
        /// Docs.
        "b" => B { keywords: [X, Y], derive: [core::default::Default, PartialOrd, Ord], initial: B::Y, inherited: no },
        /// Docs.
        "c" => C { keywords: [X, Y], derive: [Hash], initial: X, inherited: no },
        /// Docs.
        "d" => D { keywords: [X, Y], initial: X, inherited: no },
        "#,
        &[],
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(derive_names(&entries[0]), ["Hash", "Default"]);
    assert_eq!(
        entries[0].default_keyword.as_ref().map(ToString::to_string),
        Some("Y".to_owned())
    );
    // A path-written initial names its keyword by the last segment.
    assert_eq!(
        derive_names(&entries[1]),
        ["core :: default :: Default", "PartialOrd", "Ord"]
    );
    assert_eq!(
        entries[1].default_keyword.as_ref().map(ToString::to_string),
        Some("Y".to_owned())
    );
    // Without `Default`, no keyword is marked.
    assert_eq!(derive_names(&entries[2]), ["Hash"]);
    assert!(entries[2].default_keyword.is_none());
    assert!(entries[3].derives.is_empty());
    assert!(entries[3].default_keyword.is_none());
}

#[test]
fn derive_mistakes_are_reported_once_each() {
    let (entries, errors) = build_str(
        r#"
        /// Fixed derives and repeats are dropped.
        "a" => A { keywords: [X, Y], derive: [Clone, Hash, core::hash::Hash, std::fmt::Debug], initial: X, inherited: no },
        /// Not a keywords entry.
        "b" => B: f32 { derive: [Hash], initial: 1.0, inherited: no, parse: p, sample: 2.0 },
        /// `Default` with an initial that is not a keyword.
        "c" => C { keywords: [X, Y], derive: [Default], initial: C::Q, inherited: no },
        /// An unresolved initial is reported once, by `initial:`.
        "d" => D { keywords: [X, Y], derive: [Default], initial: Q, inherited: no },
        /// A missing initial is reported once, as missing.
        "e" => E { keywords: [X, Y], derive: [Default], inherited: no },
        "#,
        &[],
    );
    assert_eq!(errors.len(), 7, "{errors:?}");
    assert!(
        errors[0].starts_with("`Clone` is always derived"),
        "{errors:?}"
    );
    assert_eq!(errors[1], "`Hash` is listed twice in `derive:`");
    assert!(
        errors[2].starts_with("`Debug` is always derived"),
        "{errors:?}"
    );
    assert!(
        errors[3].starts_with("`derive:` only applies to the enum a `keywords:` entry generates"),
        "{errors:?}"
    );
    assert!(
        errors[4].starts_with("`Default` marks the `initial:` keyword"),
        "{errors:?}"
    );
    assert!(
        errors[5].starts_with("`initial: Q` is not one of"),
        "{errors:?}"
    );
    assert!(errors[6].contains("is missing `initial"), "{errors:?}");
    assert_eq!(derive_names(&entries[0]), ["Hash"]);
    assert!(entries[1].derives.is_empty());
    // `Default` without a keyword to mark is dropped, so the only error is
    // the one above (rustc would otherwise also reject the derive).
    for entry in &entries[2..] {
        assert!(entry.derives.is_empty());
        assert!(entry.default_keyword.is_none());
    }
}
