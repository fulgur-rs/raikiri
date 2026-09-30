//! One snapshot per projection of [`super::items`] (and per per-entry
//! projection the expansion uses), over a fixed three-entry fixture: a
//! `keywords:` entry serialized by keyword, a `parse:` entry without a
//! serialization, and an inherited entry with a computed type, its hook, a
//! lift, a renamed field, a residue function and a serializer function.
//! The whole-module snapshots in the crate's expansion tests pin how the
//! projections combine.

use proc_macro2::TokenStream;
use quote::quote;

use super::*;
use crate::diag::Errors;
use crate::model::build;
use crate::parse::parse_block;
use crate::tests::pretty;

const FIXTURE: &str = r#"
    /// CSS Compositing 1 §3.4.2
    "isolation" => Isolation { keywords: [Auto, Isolate], initial: Auto, inherited: no, serialize: keyword },
    /// CSS Images 3 §5.1
    "object-fit" => ObjectFit { initial: Fill, inherited: no, parse: parse_object_fit, sample: Contain, residue: none },
    /// CSS Text 3 §7.2
    "word-spacing" => WordSpacing: Length {
        initial: Length::Px(0.0), inherited: yes, parse: parse_length,
        computed: ComputedLength, compute: absolutize_length, lift: lift_length,
        field: spacing, sample: Length::Em(2.0), residue: length_residue,
        serialize: serialize_length,
    },
"#;

/// The validated fixture entries; the fixture has no mistakes.
fn fixture() -> Vec<Entry> {
    let mut errors = Errors::default();
    let raw = parse_block(FIXTURE.parse().expect("fixture tokenizes"), &mut errors);
    let entries = build(raw, &[], &mut errors);
    let errors: Vec<String> = errors.into_vec().iter().map(|e| e.to_string()).collect();
    assert_eq!(errors, Vec::<String>::new());
    assert_eq!(entries.len(), 3);
    entries
}

/// `projection` of each fixture entry under a `--- css-name ---` header.
fn per_entry(projection: impl Fn(&Entry) -> TokenStream) -> String {
    fixture()
        .iter()
        .map(|entry| {
            format!(
                "--- {} ---\n{}",
                entry.name.value(),
                pretty(projection(entry))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `projection` over the whole fixture.
fn over_fixture(projection: impl Fn(&[Entry]) -> TokenStream) -> String {
    pretty(projection(&fixture()))
}

#[test]
fn projection_value_and_key_variants() {
    insta::assert_snapshot!(
        "value_and_key_variants",
        per_entry(|entry| {
            let value = value_variant(entry);
            let key = key_variant(entry);
            quote!(#value, #key,)
        })
    );
}

#[test]
fn projection_keyword_enum() {
    insta::assert_snapshot!("keyword_enum", per_entry(keyword_enum));
}

#[test]
fn projection_type_module() {
    insta::assert_snapshot!("type_module", per_entry(type_module));
}

#[test]
fn projection_longhand_impl() {
    insta::assert_snapshot!("longhand_impl", per_entry(longhand_impl));
}

/// The per-entry items are the three per-entry projections, interleaved
/// entry by entry.
#[test]
fn projection_per_entry_items_interleaves_the_entry_projections() {
    let entries = fixture();
    let expected: TokenStream = entries
        .iter()
        .flat_map(|e| [keyword_enum(e), type_module(e), longhand_impl(e)])
        .collect();
    assert_eq!(per_entry_items(&entries).to_string(), expected.to_string());
}

/// The two `Longhand` impl shapes the main fixture lacks: a computed type
/// lifted back through the default `Into::into` (no `lift:`), and a computed
/// type whose hook is missing, whose `compute` body is `unreachable!()` (the
/// missing hook is reported once, as checked here).
#[test]
fn projection_longhand_impl_default_lift_and_broken_compute() {
    let mut errors = Errors::default();
    let raw = parse_block(
        r#"
        /// Default lift.
        "tab-width" => TabWidth: Length { initial: Length::Px(8.0), inherited: yes, parse: parse_length,
            computed: Px, compute: absolutize_length, sample: Length::Em(1.0), residue: length_residue },
        /// Missing hook.
        "gap-width" => GapWidth: Length { initial: Length::Px(0.0), inherited: no, parse: parse_length,
            computed: Px, sample: Length::Em(1.0), residue: length_residue },
        "#
        .parse()
        .expect("fixture tokenizes"),
        &mut errors,
    );
    let entries = build(raw, &[], &mut errors);
    let errors: Vec<String> = errors.into_vec().iter().map(|e| e.to_string()).collect();
    assert_eq!(
        errors,
        [
            "`computed: Px` needs `compute: <fn>`, the `fn(Specified, &AbsolutizeCx) -> Px` hook that produces it"
        ]
    );
    let rendered = entries
        .iter()
        .map(|entry| {
            format!(
                "--- {} ---\n{}",
                entry.name.value(),
                pretty(longhand_impl(entry))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("longhand_impl_default_lift_and_broken_compute", rendered);
}

#[test]
fn projection_key_method() {
    let hand_written = quote!(PropertyValue::Color { .. } => PropertyKey::Color,);
    insta::assert_snapshot!(
        "key_method",
        pretty(key_method(&fixture(), &[hand_written]))
    );
}

#[test]
fn projection_table_structs() {
    insta::assert_snapshot!("table_structs", over_fixture(table_structs));
}

#[test]
fn projection_specified_table_impl() {
    insta::assert_snapshot!("specified_table_impl", over_fixture(specified_table_impl));
}

#[test]
fn projection_computed_table_impl() {
    insta::assert_snapshot!("computed_table_impl", over_fixture(computed_table_impl));
}

#[test]
fn projection_page_absolutize_fn() {
    insta::assert_snapshot!("page_absolutize_fn", over_fixture(page_absolutize_fn));
}

#[test]
fn projection_deref_impls() {
    insta::assert_snapshot!("deref_impls", pretty(deref_impls()));
}

#[test]
fn projection_longhand_names_const() {
    insta::assert_snapshot!("longhand_names_const", over_fixture(longhand_names_const));
}

#[test]
fn projection_value_pat_macro() {
    insta::assert_snapshot!("value_pat_macro", over_fixture(value_pat_macro));
    // No pattern matches nothing: without entries the macro is omitted.
    assert!(value_pat_macro(&[]).is_empty());
}

#[test]
fn projection_sample_fns() {
    insta::assert_snapshot!("sample_fns", over_fixture(sample_fns));
}

#[test]
fn projection_specified_residue_fn() {
    insta::assert_snapshot!("specified_residue_fn", over_fixture(specified_residue_fn));
}

#[test]
fn projection_registry_macros() {
    insta::assert_snapshot!("registry_macros", over_fixture(registry_macros));
}

#[test]
fn projection_name_lookup_fn() {
    insta::assert_snapshot!("name_lookup_fn", over_fixture(name_lookup_fn));
}

#[test]
fn projection_parse_dispatch_fn() {
    insta::assert_snapshot!("parse_dispatch_fn", over_fixture(parse_dispatch_fn));
}

#[test]
fn projection_serialize_fn() {
    insta::assert_snapshot!("serialize_fn", over_fixture(serialize_fn));
}

#[test]
fn projection_serialize_computed_fn() {
    insta::assert_snapshot!("serialize_computed_fn", over_fixture(serialize_computed_fn));
}

#[test]
fn projection_serializes_fn() {
    insta::assert_snapshot!("serializes_fn", over_fixture(serializes_fn));
}

/// The serializer shapes the main fixture lacks: `serialize: keyword` on a
/// `parse:` entry (reported once, its arm unreachable), and no entry with
/// `serialize:` at all (the predicate is `false`, and the computed
/// serializer only has its `None` arms).
#[test]
fn projection_serialize_fns_broken_and_without_serializers() {
    let mut errors = Errors::default();
    let raw = parse_block(
        r#"
        /// Keyword on a parsed value.
        "opacity" => Opacity: f32 { initial: 1.0, inherited: no, parse: parse_number,
            sample: 2.0, residue: none, serialize: keyword },
        "#
        .parse()
        .expect("fixture tokenizes"),
        &mut errors,
    );
    let broken = build(raw, &[], &mut errors);
    let errors: Vec<String> = errors.into_vec().iter().map(|e| e.to_string()).collect();
    assert_eq!(
        errors,
        [
            "`serialize: keyword` uses the enum a `keywords:` entry generates; a `parse:` entry names a `fn(&Specified) -> Option<String>`"
        ]
    );
    let without: Vec<Entry> = fixture()
        .into_iter()
        .filter(|e| matches!(e.serialize, crate::model::Serialize::None))
        .collect();
    assert_eq!(without.len(), 1);
    let rendered = format!(
        "--- broken ---\n{}--- broken, computed ---\n{}--- without serializers ---\n{}{}",
        pretty(serialize_fn(&broken)),
        pretty(serialize_computed_fn(&broken)),
        pretty(serialize_computed_fn(&without)),
        pretty(serializes_fn(&without)),
    );
    insta::assert_snapshot!("serialize_fns_broken_and_without_serializers", rendered);
}
