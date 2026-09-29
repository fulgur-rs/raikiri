use quote::ToTokens as _;

use super::{ComputedSpec, RawEntry, Slot, parse_block};
use crate::diag::Errors;

fn parse(src: &str) -> (Vec<RawEntry>, Vec<String>) {
    let mut errors = Errors::default();
    let tokens = src.parse().expect("test input tokenizes");
    let entries = parse_block(tokens, &mut errors);
    let messages = errors.into_vec().iter().map(|e| e.to_string()).collect();
    (entries, messages)
}

fn tokens<T: quote::ToTokens>(slot: &Slot<T>) -> String {
    slot.value()
        .map(|v| v.to_token_stream().to_string())
        .unwrap_or_default()
}

#[test]
fn parses_the_short_forms() {
    let (entries, errors) = parse(
        r#"
        /// CSS Compositing 1 §3.4.2
        "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no },
        /// CSS Images 3 §5.1
        "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain },
        /// CSS Color 4 §3.3
        "opacity" => Opacity: f32 { initial: 1.0, inherited: no, parse: parse_opacity_value,
                                    compute: clamp_opacity, sample: 2.0 }
        "#,
    );
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(entries.len(), 3);

    let isolation = &entries[0];
    assert_eq!(isolation.name.value(), "isolation");
    assert_eq!(isolation.variant, "Isolation");
    assert_eq!(isolation.attrs.len(), 1);
    assert!(isolation.value_ty.is_none());
    let keywords = isolation.keywords.value().expect("keywords");
    assert_eq!(keywords.len(), 2);
    assert_eq!(keywords[1].ident, "Isolate");
    assert!(keywords[1].css.is_none());
    assert_eq!(tokens(&isolation.initial), "Auto");
    assert_eq!(isolation.inherited.value(), Some(&false));
    assert!(isolation.sample.is_absent());

    let object_fit = &entries[1];
    assert_eq!(tokens(&object_fit.parse), "parse_object_fit");
    assert_eq!(tokens(&object_fit.sample), "Contain");

    let opacity = &entries[2];
    assert_eq!(opacity.value_ty.to_token_stream().to_string(), "f32");
    assert_eq!(tokens(&opacity.compute), "clamp_opacity");
    assert_eq!(tokens(&opacity.initial), "1.0");
    assert!(!opacity.had_errors);
}

#[test]
fn accepts_keys_in_any_order_and_optional_commas() {
    let (entries, errors) = parse(
        r#"
        "a" => A { inherited: yes, initial: X, keywords: [X, Y = "why",], }
        "#,
    );
    assert_eq!(errors, Vec::<String>::new());
    let keywords = entries[0].keywords.value().expect("keywords");
    assert_eq!(
        keywords[1].css.as_ref().map(|s| s.value()).as_deref(),
        Some("why")
    );
    assert_eq!(entries[0].inherited.value(), Some(&true));
}

#[test]
fn keywords_keep_their_doc_comments() {
    let (entries, errors) = parse(
        r#"
        "a" => A { keywords: [
            /// The first.
            X,
            Y,
        ] }
        "#,
    );
    assert_eq!(errors, Vec::<String>::new());
    let keywords = entries[0].keywords.value().expect("keywords");
    assert_eq!(keywords[0].attrs.len(), 1);
    assert!(keywords[1].attrs.is_empty());
}

#[test]
fn parses_computed_forms() {
    let (entries, errors) = parse(
        r#"
        "a" => A: Length { computed: as_specified, field: a_field, lift: f },
        "b" => B: Length { computed: ComputedLength<'static, u8> via absolutize, lift: crate::lift_b },
        "#,
    );
    assert_eq!(errors, Vec::<String>::new());
    assert!(matches!(
        entries[0].computed.value(),
        Some(ComputedSpec::AsSpecified)
    ));
    assert_eq!(tokens(&entries[0].field), "a_field");
    let Some(ComputedSpec::Via { ty, hook }) = entries[1].computed.value() else {
        panic!("expected `via`");
    };
    assert_eq!(
        ty.to_token_stream().to_string(),
        "ComputedLength < 'static , u8 >"
    );
    assert_eq!(hook.to_token_stream().to_string(), "absolutize");
    assert_eq!(tokens(&entries[1].lift), "crate :: lift_b");
}

#[test]
fn an_unknown_key_is_one_error_and_the_rest_is_kept() {
    let (entries, errors) = parse(
        r#"
        "a" => A { initial: X, animatable: yes(1, 2), inherited: no, sample: Y },
        "b" => B { initial: X, inherited: no },
        "#,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("unknown key `animatable`"));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].inherited.value(), Some(&false));
    assert_eq!(tokens(&entries[0].sample), "Y");
    assert!(entries[0].had_errors);
    assert!(!entries[1].had_errors);
}

#[test]
fn a_malformed_value_marks_the_key_invalid() {
    let (entries, errors) = parse(r#""a" => A { initial: , inherited: maybe, sample: 1 }"#);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(matches!(entries[0].initial, Slot::Invalid(_)));
    assert!(matches!(entries[0].inherited, Slot::Invalid(_)));
    assert_eq!(tokens(&entries[0].sample), "1");
}

#[test]
fn a_missing_comma_between_keys_resumes_at_the_next_key() {
    let (entries, errors) = parse(r#""a" => A { initial: X inherited: no }"#);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("separate keys with `,`"), "{errors:?}");
    assert_eq!(entries[0].inherited.value(), Some(&false));
}

#[test]
fn a_broken_head_skips_to_the_next_entry() {
    let (entries, errors) = parse(
        r#"
        "a" = A { initial: X },
        /// Docs.
        "b" => B { initial: X },
        "c" => { initial: X },
        "d" => D { initial: X }
        "#,
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
    let names: Vec<_> = entries.iter().map(|e| e.name.value()).collect();
    assert_eq!(names, ["b", "d"]);
    assert_eq!(entries[0].attrs.len(), 1);
}

#[test]
fn a_missing_comma_between_entries_is_one_error() {
    let (entries, errors) = parse(r#""a" => A { initial: X } "b" => B { initial: X }"#);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(entries.len(), 2);
}

#[test]
fn a_repeated_key_keeps_the_first_value() {
    let (entries, errors) = parse(r#""a" => A { initial: X, initial: Y }"#);
    assert_eq!(errors, ["duplicate key `initial` in this entry"]);
    assert_eq!(tokens(&entries[0].initial), "X");
}

#[test]
fn garbage_does_not_loop() {
    let (entries, errors) = parse(r#"struct X; "a" => A { : : , ,, initial X }"#);
    assert!(!errors.is_empty());
    assert_eq!(entries.len(), 1);
}
