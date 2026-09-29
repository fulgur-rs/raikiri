use quote::ToTokens as _;
use syn::Ident;

use super::{Entry, Value, build};
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
    assert_eq!(
        opacity.compute.to_token_stream().to_string(),
        "clamp"
    );
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
    assert!(entries[0].lift.is_none());
    assert_eq!(entries[1].lift.to_token_stream().to_string(), "from_px");
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
