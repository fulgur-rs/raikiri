//! Tests for the generated-content property parsers in `parse/content.rs`.

use super::*;

// ── counter-* (CSS Lists 3 §4) ──

// Because these values are wrapped in `PropertyValue::Counter*(Arc<Vec<..>>)`,
// switch to a helper returning Arc<Vec<..>> for literal test comparisons
// (the same pattern as the content/string_set helpers).
fn counter_pairs(pairs: &[(&str, i32)]) -> Arc<Vec<(SmolStr, i32)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(name, value)| (SmolStr::new(name), *value))
            .collect(),
    )
}

#[test]
fn counter_reset_single_name_defaults_to_zero() {
    // Spec: reset defaults to 0.
    assert_eq!(
        parse("chapter", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", 0
        )])))
    );
}

#[test]
fn counter_reset_multiple_names_with_mixed_ints() {
    // Second name has an integer: first defaults to 0, second takes 3.
    assert_eq!(
        parse("chapter section 3", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[
            ("chapter", 0),
            ("section", 3)
        ])))
    );
}

#[test]
fn counter_reset_none_returns_empty_vec() {
    // Spec: `none` is equivalent to an empty list (top-level alternative).
    // Use the shared Arc slot (`empty_counter_entries`) for the empty case.
    assert_eq!(
        parse("none", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_rejects_number_first() {
    // A leading number cannot be peeled as an identifier: empty → None (drop).
    // Spec §4: `<counter-name> = <custom-ident>`; numbers are not counter names.
    assert_eq!(parse("123 abc", "counter-reset"), None);
}

#[test]
fn counter_increment_single_name_defaults_to_one() {
    // Spec: increment defaults to 1.
    assert_eq!(
        parse("chapter", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", 1
        )])))
    );
}

#[test]
fn counter_increment_mixed_int_and_default() {
    // `chapter 2 section` → chapter=2, section=default(1)
    assert_eq!(
        parse("chapter 2 section", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[
            ("chapter", 2),
            ("section", 1)
        ])))
    );
}

#[test]
fn counter_increment_accepts_negative_integer() {
    // Spec §4: <integer> includes negatives (to decrement a counter).
    assert_eq!(
        parse("chapter -1", "counter-increment"),
        Some(PropertyValue::CounterIncrement(counter_pairs(&[(
            "chapter", -1
        )])))
    );
}

#[test]
fn counter_increment_none_returns_empty_vec() {
    // Use the shared Arc slot for the empty case.
    assert_eq!(
        parse("none", "counter-increment"),
        Some(PropertyValue::CounterIncrement(empty_counter_entries()))
    );
}

#[test]
fn counter_set_defaults_to_zero() {
    // Spec: set defaults to 0.
    assert_eq!(
        parse("page 5 note", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[
            ("page", 5),
            ("note", 0)
        ])))
    );
}

#[test]
fn counter_set_none_returns_empty_vec() {
    // Use the shared Arc slot for the empty case.
    assert_eq!(
        parse("none", "counter-set"),
        Some(PropertyValue::CounterSet(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_is_case_insensitive_on_none() {
    // CSS spec: the `none` keyword is ASCII case-insensitive.
    // Use the shared Arc slot for the empty case.
    assert_eq!(
        parse("NONE", "counter-reset"),
        Some(PropertyValue::CounterReset(empty_counter_entries()))
    );
}

#[test]
fn counter_reset_accepts_inherit_marker_and_rejects_other_reserved_names() {
    // `counter-reset: inherit` is retained as a page-context marker;
    // the other CSS-wide keywords are not valid counter names.
    assert_eq!(
        parse("inherit", "counter-reset"),
        Some(PropertyValue::CounterResetInherit)
    );
    assert_eq!(parse("initial", "counter-reset"), None);
    assert_eq!(parse("unset", "counter-reset"), None);
    assert_eq!(parse("revert", "counter-reset"), None);
    assert_eq!(parse("default", "counter-reset"), None);
}

#[test]
fn counter_reset_accepts_negative_integer() {
    // CSS Values 3 §4.2 "Integers: the <integer> type"
    // (https://www.w3.org/TR/css-values-3/#integers): <integer> includes negatives.
    // Reset and set use the same grammar as increment.
    assert_eq!(
        parse("chapter -5", "counter-reset"),
        Some(PropertyValue::CounterReset(counter_pairs(&[(
            "chapter", -5
        )])))
    );
}

#[test]
fn counter_set_accepts_negative_integer() {
    // Same rule (parity with negative-integer coverage for reset/increment).
    assert_eq!(
        parse("page -3", "counter-set"),
        Some(PropertyValue::CounterSet(counter_pairs(&[("page", -3)])))
    );
}

// ── content property (CSS Content 3 §2) ──
//
// The task's verification items are derived from the spec. In the task,
// `raikiri_traits::ContentValueItem` is the downstream (raikiri-dom) mapping target.
// raikiri-style is a leaf crate without a dependency on raikiri-traits;
// following the counter-* precedent, it emits a local `ContentComponent`
// (principle 1: follow precedent). Replace Symbol with SmolStr and Url with String.

#[test]
fn content_parse_string_function() {
    // Verification 1: content: string(my_str)
    // → ContentComponent::String { name: "my_str", fetch: default (First) }
    let items = content_items("string(my_str)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::String {
            name: SmolStr::new("my_str"),
            fetch: StringFetchMode::First,
        }
    );
}

#[test]
fn content_parse_counter_function() {
    // Verification 2: content: counter(chapter)
    // → ContentComponent::Counter { name: "chapter", style: default (Decimal) }
    let items = content_items("counter(chapter)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_counters_function() {
    // Verification 3: content: counters(section, ".")
    // → ContentComponent::Counters { name, separator: ".", style: default }
    let items = content_items(r#"counters(section, ".")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counter_function() {
    // Verification 4: content: target-counter(url("#anchor"), page)
    // → ContentComponent::TargetCounter { url: "#anchor", name: "page", style: default }
    let items = content_items(r##"target-counter(url("#anchor"), page)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounter {
            url: String::from("#anchor"),
            name: SmolStr::new("page"),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_counters_function() {
    // Verification 5: content: target-counters(url("#anchor"), section, ".")
    // → ContentComponent::TargetCounters { url, name, separator, style: default }
    let items = content_items(r##"target-counters(url("#anchor"), section, ".")"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetCounters {
            url: String::from("#anchor"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::Decimal,
        }
    );
}

#[test]
fn content_parse_target_text_first_letter() {
    // Verification 6: content: target-text(url("#anchor"), first-letter)
    // → ContentComponent::TargetText { url, part: ContentPart::FirstLetter }
    //
    // Note: "content-first-letter" in the task description conflicts with the
    // spec (§2.6.3 `[ content | before | after | first-letter ]?`), so
    // use the spec-correct `first-letter` (correcting the task description
    // after discovering the mistake).
    let items = content_items(r##"target-text(url("#anchor"), first-letter)"##);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::TargetText {
            url: String::from("#anchor"),
            part: ContentPart::FirstLetter,
        }
    );
}

#[test]
fn content_parse_attr_function() {
    // Verification 7: content: attr(href) → ContentComponent::Attr { name: "href" }
    let items = content_items("attr(href)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Attr {
            name: SmolStr::new("href"),
        }
    );
}

#[test]
fn content_parse_attr_untyped_fallbacks() {
    let items = content_items(r#"attr(missing, "Fallback value") attr(missing, invalid)"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: Some(SmolStr::new("Fallback value")),
            },
            ContentComponent::AttrFallback {
                name: SmolStr::new("missing"),
                fallback: None,
            },
        ]
    );
}

#[test]
fn content_parse_literal_string() {
    // Verification 8: content: "hello" → ContentComponent::Literal("hello")
    let items = content_items(r#""hello""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn content_parse_mixed_sequence_preserves_order() {
    // Verification 9: content: "Chapter " counter(chapter) ": " string(chapter_title)
    // → 4-item Vec in order
    let items = content_items(r#""Chapter " counter(chapter) ": " string(chapter_title)"#);
    assert_eq!(items.len(), 4, "expected 4 items, got {items:?}");
    assert_eq!(
        items[0],
        ContentComponent::Literal(SmolStr::new("Chapter "))
    );
    assert_eq!(
        items[1],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(items[2], ContentComponent::Literal(SmolStr::new(": ")));
    assert_eq!(
        items[3],
        ContentComponent::String {
            name: SmolStr::new("chapter_title"),
            fetch: StringFetchMode::First,
        }
    );
}

// ── content property: image / contents / <quote> / leader() (CSS Content 3
// §2.2 / §2.3 / §2.4.2 / §2.5.1 — under-accept fix, CssContent3 mode arm) ──

#[test]
fn content_parse_image_url_quoted_form() {
    // The `<url>` alternative of `<image>`: quoted `url("...")` form.
    let items = content_items(r#"url("cat.png")"#);
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_parse_image_url_unquoted_form() {
    // The `<url>` alternative of `<image>`: unquoted `url(...)` url-token form.
    let items = content_items("url(cat.png)");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0],
        ContentComponent::Image {
            url: String::from("cat.png"),
        }
    );
}

#[test]
fn content_bare_string_is_still_literal_not_image() {
    // Regression check: `<image>` allows only `<url> | <gradient>`,
    // not bare `<string>` (unlike the target-* grammar `[<string>|<url>]`).
    // `expect_url` does not accept a quoted string by itself, so
    // `content: "cat.png"` stays Literal; do not incorrectly turn it into Image.
    let items = content_items(r#""cat.png""#);
    assert_eq!(
        items,
        vec![ContentComponent::Literal(SmolStr::new("cat.png"))]
    );
}

#[test]
fn content_parse_contents_keyword() {
    // CSS Content 3 §2.3 "Elemental Content: the contents keyword".
    let items = content_items("contents");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_contents_keyword_is_case_insensitive() {
    let items = content_items("CoNtEnTs");
    assert_eq!(items, vec![ContentComponent::Contents]);
}

#[test]
fn content_parse_quote_keywords() {
    // CSS Content 3 §2.4.2 `<quote> = open-quote | close-quote |
    // no-open-quote | no-close-quote`: check all four keywords.
    assert_eq!(
        content_items("open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
    assert_eq!(
        content_items("close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::CloseQuote)]
    );
    assert_eq!(
        content_items("no-open-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoOpenQuote)]
    );
    assert_eq!(
        content_items("no-close-quote"),
        vec![ContentComponent::Quote(QuoteKeyword::NoCloseQuote)]
    );
}

#[test]
fn content_quote_keyword_is_case_insensitive() {
    let items = content_items("OPEN-QUOTE");
    assert_eq!(
        items,
        vec![ContentComponent::Quote(QuoteKeyword::OpenQuote)]
    );
}

#[test]
fn content_parse_leader_dotted_solid_space_keywords() {
    // CSS Content 3 §2.5.1 `<leader-type> = dotted | solid | space | <string>`.
    assert_eq!(
        content_items("leader(dotted)"),
        vec![ContentComponent::Leader(LeaderType::Dotted)]
    );
    assert_eq!(
        content_items("leader(solid)"),
        vec![ContentComponent::Leader(LeaderType::Solid)]
    );
    assert_eq!(
        content_items("leader(space)"),
        vec![ContentComponent::Leader(LeaderType::Space)]
    );
}

#[test]
fn content_parse_leader_custom_string() {
    let items = content_items(r#"leader(".~.")"#);
    assert_eq!(
        items,
        vec![ContentComponent::Leader(LeaderType::String(SmolStr::new(
            ".~."
        )))]
    );
}

#[test]
fn content_leader_is_case_insensitive() {
    let items = content_items("LEADER(DOTTED)");
    assert_eq!(items, vec![ContentComponent::Leader(LeaderType::Dotted)]);
}

#[test]
fn content_leader_rejects_missing_argument() {
    // Spec production `leader( <leader-type> )` has no `?`, so its argument is required.
    // Bare `leader()` is invalid per spec → drop the declaration.
    assert_eq!(parse("leader()", "content"), None);
}

#[test]
fn content_leader_rejects_unknown_keyword() {
    assert_eq!(parse("leader(bogus)", "content"), None);
}

#[test]
fn content_rejects_unknown_bare_keyword() {
    // Of the five keywords in `parse_content_bare_keyword` (`contents` / four `<quote>`),
    // an identifier matching none falls through to `_ => None`;
    // with zero items (when it is the only token), drop the declaration.
    assert_eq!(parse("bogus", "content"), None);
}

#[test]
fn content_unknown_bare_keyword_mid_list_stops_items_and_leaves_leftover() {
    // If an unknown identifier follows a recognized item (`counter(chapter)`),
    // the items+ loop breaks on the unknown token (catch-all break path).
    // The `parse` helper does not use the caller's (rule.rs) equivalent of
    // `expect_exhausted`, so one item is observable through this helper;
    // the existing `content_rejects_unknown_function` and
    // `string_set_accepts_missing_comma_single_leftover_entry` tests cover
    // the same break-then-leftover pattern for dropping leftovers.
    let items = content_items("counter(chapter) bogus");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_parse_mixed_sequence_with_new_alternatives() {
    // Mix image / contents / quote / leader with existing alternatives,
    // then verify that order is preserved.
    let items =
        content_items(r#"open-quote "term" close-quote leader(dotted) url("icon.png") contents"#);
    assert_eq!(
        items,
        vec![
            ContentComponent::Quote(QuoteKeyword::OpenQuote),
            ContentComponent::Literal(SmolStr::new("term")),
            ContentComponent::Quote(QuoteKeyword::CloseQuote),
            ContentComponent::Leader(LeaderType::Dotted),
            ContentComponent::Image {
                url: String::from("icon.png"),
            },
            ContentComponent::Contents,
        ]
    );
}

// ── string-set narrow <content-list> gate: image / contents / quote /
// leader() (CSS GCPM 3 §1.1.1 L82) ──
//
// The GCPM 3 §1.1.1 narrow list contains none of `<image>` / `contents` /
// `<quote>` / `leader()` (same rationale as the existing string_set_rejects_*
// group). Follow sibling tests with one declaration per rejection
// check.

#[test]
fn string_set_rejects_image_url() {
    assert_eq!(parse(r#"title url("a.png")"#, "string-set"), None);
}

#[test]
fn string_set_rejects_contents_keyword() {
    assert_eq!(parse("title contents", "string-set"), None);
}

#[test]
fn string_set_rejects_quote_keyword() {
    assert_eq!(parse("title open-quote", "string-set"), None);
}

#[test]
fn string_set_rejects_leader_fn() {
    assert_eq!(parse("title leader(dotted)", "string-set"), None);
}

// ── content property edge cases (spec-derived, guard rails) ──

#[test]
fn content_normal_returns_empty_list() {
    // Spec §1: `normal` means "as if content was not explicitly specified"; store an empty list.
    // Decide pseudo-element generation downstream.
    assert_eq!(
        parse("normal", "content"),
        Some(PropertyValue::Content(empty_content_list()))
    );
}

#[test]
fn content_none_keeps_a_suppression_sentinel() {
    assert_eq!(
        parse("none", "content"),
        Some(PropertyValue::Content(Arc::new(vec![
            ContentComponent::None
        ])))
    );
}

#[test]
fn content_string_with_fetch_last_keyword() {
    // Smoke-test acceptance of every second-argument keyword of string() in spec §2.7.2.
    let items = content_items("string(head, last)");
    assert_eq!(
        items,
        vec![ContentComponent::String {
            name: SmolStr::new("head"),
            fetch: StringFetchMode::Last,
        }]
    );
}

#[test]
fn content_element_keeps_its_page_selection_keyword() {
    // CSS GCPM 3 §1.2.2: element(<custom-ident>, [first|start|last|first-except]?),
    // with `first` when the keyword is omitted.
    assert_eq!(
        content_items("element(header)"),
        vec![ContentComponent::Element {
            name: SmolStr::new("header"),
            fetch: StringFetchMode::First,
        }]
    );
    for (keyword, fetch) in [
        ("first", StringFetchMode::First),
        ("start", StringFetchMode::Start),
        ("last", StringFetchMode::Last),
        ("first-except", StringFetchMode::FirstExcept),
    ] {
        assert_eq!(
            content_items(&format!("element(header, {keyword})")),
            vec![ContentComponent::Element {
                name: SmolStr::new("header"),
                fetch,
            }]
        );
    }
}

#[test]
fn content_target_text_default_part_is_content() {
    // When target-text() omits its second argument, raikiri uses ContentPart::Content
    // as its fallback (not based on a spec declaration of a "default":
    // CSS Content 3 §2.6.3 does not specify the omitted argument's value).
    let items = content_items(r##"target-text(url("#a"))"##);
    assert_eq!(
        items,
        vec![ContentComponent::TargetText {
            url: String::from("#a"),
            part: ContentPart::Content,
        }]
    );
}

#[test]
fn content_counter_rejects_none_name() {
    // spec CSS Lists 3 §4 <https://www.w3.org/TR/css-lists-3/#typedef-counter-name>:
    // "A <counter-name> name cannot match the keyword `none`; such an identifier
    // is invalid as a <counter-name>". The first argument of §4.7 counter() is
    // a <counter-name>, so drop the `counter(none)` declaration.
    // Consistent with existing counter-reset/increment/set (property.rs) and Chrome/Firefox.
    assert_eq!(parse("counter(none)", "content"), None);
}

#[test]
fn content_counters_rejects_none_name() {
    // CSS Lists 3 §4 / §4.7: counters() also has <counter-name>
    // as its first argument, so `none` is invalid.
    assert_eq!(parse(r#"counters(none, ".")"#, "content"), None);
}

#[test]
fn content_counter_with_named_style_preserves_ident() {
    // CSS Lists 3 §4.7: the second argument `<counter-style>` also accepts
    // named styles other than decimal. Keep the raw identifier for raikiri-dom to interpret.
    let items = content_items("counter(chapter, upper-alpha)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Named(SmolStr::new("upper-alpha")),
        }]
    );
}

#[test]
fn content_rejects_unknown_function() {
    // An unknown function cannot start items: peeling the first token fails.
    // If the first token is an unknown function, items stay empty → None (drop).
    assert_eq!(parse("bogus(x)", "content"), None);
}

#[test]
fn content_case_insensitive_function_name() {
    // Spec: function names are ASCII case-insensitive.
    let items = content_items("COUNTER(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

#[test]
fn content_key_maps_to_content_property_key() {
    // PropertyValue::Content → PropertyKey::Content (discriminant integrity for
    // cascade winner selection, like the existing sibling counter-* pattern).
    let cv = PropertyValue::Content(empty_content_list());
    assert_eq!(cv.key(), PropertyKey::Content);
}

// ── parse_optional_counter_style trailing-comma strict reject ──
//
// CSS Lists 3 §4.7 `counter(<counter-name>, <counter-style>?)` /
// CSS Content 3 §2.6.1–2 `target-counter()` / `target-counters()` have
// `<counter-style>?`: after `,`, an identifier is required. A trailing comma
// (as in `counter(chapter,)`) is invalid per spec; drop the whole declaration.
// Siblings `parse_string_fetch` / `parse_content_part` already propagate `?`
// strictly. Only `parse_optional_counter_style` silently fell back to Decimal;
// check that regression.

#[test]
fn content_counter_rejects_trailing_comma() {
    // In `counter(chapter,)`, an identifier is missing after the comma;
    // invalid per spec → None, dropping the declaration (as in Chrome/Firefox).
    assert_eq!(parse("counter(chapter,)", "content"), None);
}

#[test]
fn content_counters_rejects_trailing_comma() {
    // `counters(chapter, ".",)`: trailing comma after the separator string.
    assert_eq!(parse(r#"counters(chapter, ".",)"#, "content"), None);
}

#[test]
fn content_target_counter_rejects_trailing_comma() {
    // `target-counter(url("#a"), page,)`: trailing comma after the name.
    // target-counter/target-counters call parse_optional_counter_style
    // (via parse_target_counter_fn / parse_target_counters_fn); drop with the same pattern.
    assert_eq!(
        parse(r##"target-counter(url("#a"), page,)"##, "content"),
        None
    );
}

#[test]
fn content_target_counters_rejects_trailing_comma() {
    // `target-counters(url("#a"), section, ".",)`: trailing comma after separator.
    assert_eq!(
        parse(r##"target-counters(url("#a"), section, ".",)"##, "content"),
        None
    );
}

#[test]
fn content_string_rejects_trailing_comma() {
    // Control case (keep the current strict behavior): `string(foo,)`
    // already returns None via `?` propagation in `parse_string_fetch`.
    assert_eq!(parse("string(foo,)", "content"), None);
}

#[test]
fn content_target_text_rejects_trailing_comma() {
    // Control case: `target-text(url("#a"),)` returns None already via
    // `?` propagation in `parse_content_part`.
    assert_eq!(parse(r##"target-text(url("#a"),)"##, "content"), None);
}

#[test]
fn content_counter_accepts_bare_default() {
    // `counter(chapter)` is valid without a trailing comma; Decimal defaults,
    // so it returns Some (strictness must not change the valid path).
    let items = content_items("counter(chapter)");
    assert_eq!(
        items,
        vec![ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }]
    );
}

// ── string-set (CSS GCPM 3 §1.1.1) ──
//
// Grammar: `none | [ <custom-ident> <content-list> ]#`; each entry is a
// (name, content-list) pair and reuses `ContentComponent` +
// `parse_content_list_items`. The task's "4-item Vec" mistakenly counted the entry name
// as content; there are actually three items (name is tuple element one).

fn string_set_entries(source: &str) -> Vec<(SmolStr, Vec<ContentComponent>)> {
    match parse(source, "string-set") {
        // PropertyValue::StringSet(Arc<Vec<..>>), following the content_items pattern.
        Some(PropertyValue::StringSet(v)) => (*v).clone(),
        other => panic!("expected PropertyValue::StringSet, got {other:?}"),
    }
}

#[test]
fn string_set_single_entry_with_literal() {
    // Verification 1: string-set: my_str "hello"
    // → `[(SmolStr("my_str"), [Literal("hello")])]`
    let entries = string_set_entries(r#"my_str "hello""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("my_str"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("hello"))]
    );
}

#[test]
fn string_set_mixed_content_list_preserves_order() {
    // Verification 2:
    // string-set: chapter_title counter(chapter) ": " attr(title)
    //
    // The initial `chapter_title` is the entry name (tuple element one). The content list
    // contains the remaining `counter(chapter) ": " attr(title)` = three items.
    //
    // Note: the original test ended with `string(chapter_title)`, but
    // GCPM 3 §1.1.1's narrow list excludes the `string()` function, so
    // replace it with `attr()` (one of five alternatives in the GCPM narrow list).
    // Keep the test's main purpose: preserve the order of a mixed content list.
    let entries = string_set_entries(r#"chapter_title counter(chapter) ": " attr(title)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("chapter_title"));
    assert_eq!(entries[0].1.len(), 3);
    assert_eq!(
        entries[0].1[0],
        ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::Decimal,
        }
    );
    assert_eq!(
        entries[0].1[1],
        ContentComponent::Literal(SmolStr::new(": "))
    );
    assert_eq!(
        entries[0].1[2],
        ContentComponent::Attr {
            name: SmolStr::new("title"),
        }
    );
}

#[test]
fn string_set_comma_separated_multi_entry() {
    // Verification 3: string-set: a "x", b "y" → 2 entries
    let entries = string_set_entries(r#"a "x", b "y""#);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
    assert_eq!(entries[1].0, SmolStr::new("b"));
    assert_eq!(
        entries[1].1,
        vec![ContentComponent::Literal(SmolStr::new("y"))]
    );
}

#[test]
fn string_set_none_returns_empty_vec() {
    // spec §1.1.1: top-level `none` = empty list
    assert_eq!(
        parse("none", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_rejects_reserved_css_wide_keyword_as_name() {
    // spec §1.1.1 + CSS Values 4 §4.2
    // <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // `<custom-ident>` excludes CSS-wide keywords.
    // If the first identifier is `inherit`, try_parse rewinds, leaving no entries → None.
    //
    // Note: if the first identifier is `none`, the top-level alternative runs first,
    // returning `Some(empty)`; downstream `expect_exhausted` drops the declaration
    // because of leftover tokens (at rule.rs level). This case is not checked by
    // parse_value alone.
    assert_eq!(parse("inherit \"x\"", "string-set"), None);
    assert_eq!(parse("initial \"x\"", "string-set"), None);
    assert_eq!(parse("unset \"x\"", "string-set"), None);
    assert_eq!(parse("revert \"x\"", "string-set"), None);
    assert_eq!(parse("default \"x\"", "string-set"), None);
}

#[test]
fn string_set_rejects_name_without_content_list() {
    // Spec §1.1.1 and Content 3 §2: `<content-list>` requires at least one item.
    // A name alone yields zero items → None, dropping the declaration.
    assert_eq!(parse("my_str", "string-set"), None);
}

#[test]
fn string_set_is_case_insensitive_on_none() {
    // CSS spec: the `none` keyword is ASCII case-insensitive.
    assert_eq!(
        parse("NONE", "string-set"),
        Some(PropertyValue::StringSet(empty_string_set_entries()))
    );
}

#[test]
fn string_set_key_maps_to_string_set_property_key() {
    // PropertyValue::StringSet → PropertyKey::StringSet (discriminant integrity for
    // cascade winner selection, as with sibling counter-* / content).
    let v = PropertyValue::StringSet(empty_string_set_entries());
    assert_eq!(v.key(), PropertyKey::StringSet);
}

// ── string-set trailing-comma strict reject ──
//
// `#` (comma-separated multiplier, CSS Values 4 §2.3
// <https://www.w3.org/TR/css-values-4/#mult-comma>) forbids trailing commas.
// GCPM 3 §1.1.1 `<string-set-value> = [ <custom-ident>
// <content-list> ]#` requires commas between entries and forbids a trailing comma.
//
// The original implementation's separator loop used `try_parse(expect_comma).is_err() {
// break }`; it silently accepted trailing commas (after consuming the comma,
// the next iteration failed to parse a name, broke, and returned existing entries
// as Some). Make it strict with the same `.ok()?` propagation principle.

#[test]
fn string_set_rejects_trailing_comma_single_entry() {
    // `string-set: a "x",` → trailing comma → declaration drop.
    // Before the fix it silently returned Some(`[(a, [Literal("x")])]`).
    assert_eq!(parse(r#"a "x","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_two_entries() {
    // `string-set: a "x", b "y",` → trailing comma → declaration drop.
    // An internal comma is a valid separator; only a final comma violates `#`.
    assert_eq!(parse(r#"a "x", b "y","#, "string-set"), None);
}

#[test]
fn string_set_rejects_trailing_comma_three_entries() {
    // Pin consistent strict rejection across three entries and a trailing comma.
    assert_eq!(parse(r#"a "x", b "y", c "z","#, "string-set"), None);
}

#[test]
fn string_set_rejects_missing_entry_after_comma() {
    // In `string-set: a "x", b`, the name after the comma parses, but `<content-list>`
    // has zero items (`parse_content_list_items` is empty): drop the declaration.
    // This follows an items-empty rejection path distinct from trailing comma tests.
    assert_eq!(parse(r#"a "x", b"#, "string-set"), None);
}

#[test]
fn string_set_accepts_missing_comma_single_leftover_entry() {
    // `string-set: a "x" b "y"` lacks a separating comma. After pushing
    // (a, `["x"]`) in iteration one, bottom expect_comma fails and breaks. The
    // leftover `b "y"` is not dropped by this helper (direct parse_value call,
    // without caller expect_exhausted), so it returns Some with one entry.
    // The real caller (rule.rs) drops the declaration via expect_exhausted.
    // This test checks that parse_string_set's break exit returns Some (not the
    // `.ok()?` path), separate from the trailing-comma fix.
    // non-regression coverage.
    let entries = string_set_entries(r#"a "x" b "y""#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("a"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Literal(SmolStr::new("x"))]
    );
}

// ── string-set narrow <content-list> gate (CSS GCPM 3 §1.1.1) ──
//
// GCPM 3 §1.1.1 L82 verbatim: <content-list> = [ <string> | <counter()> |
// <counters()> | <content()> | <attr()> ]+ narrows the broad CSS Content 3 §2 list
// for string-set. `string()` (a function, distinct from a bare literal)
// and `target-counter()` / `target-counters()` / `target-text()` are
// outside the spec grammar. Reject them in `ContentListMode::GcpmStringSet`
// mode (parse_content_list_items has zero items → parse_string_set →
// None → drop declaration). For example, the cascade-shadow case
// `p.hi { string-set: title target-counter(url("#x"), page); }` correctly
// drops the .hi rule; verify this at the parser layer.
//
// In contrast, the content property (CssContent3 mode) still accepts all these
// (verified against regressions by the content_parse_* tests below).

#[test]
fn string_set_rejects_string_fn() {
    // GCPM 3 §1.1.1 L82 excludes the `string()` function from the narrow list.
    // Reject the function, not the bare `<string>` literal (`"..."`).
    assert_eq!(parse("title string(x)", "string-set"), None);
}

#[test]
fn string_set_rejects_target_counter_fn() {
    // GCPM 3 §1.1.1 L82 excludes `target-counter()` from the narrow list.
    // This is the main cascade-shadow case: dropping this declaration lets an
    // earlier valid rule win in the cascade.
    assert_eq!(
        parse(r##"title target-counter(url("#a"), page)"##, "string-set"),
        None
    );
}

#[test]
fn string_set_rejects_target_counters_fn() {
    // GCPM 3 §1.1.1 L82 excludes `target-counters()` from the narrow list.
    assert_eq!(
        parse(
            r##"title target-counters(url("#a"), section, ".")"##,
            "string-set"
        ),
        None
    );
}

#[test]
fn string_set_rejects_target_text_fn() {
    // GCPM 3 §1.1.1 L82 excludes `target-text()` from the narrow list.
    assert_eq!(
        parse(r##"title target-text(url("#a"))"##, "string-set"),
        None
    );
}

// ── content() function (CSS GCPM 3 §1.1.1.1) ──
//
// Grammar (verbatim from line 758 of TR/css-gcpm-3/, in the string-set/GCPM3
// grammar):
//   content() = content(`[text | before | after | first-letter]`)
// Four keywords. If omitted, use `text` as a fallback (not based on
// a GCPM 3 spec statement of "default": the meaning of "default" remains
// an open WG issue, and the grammar does not have `?`). GCPM 3
// §1.1.1's narrow `<content-list>` and CSS Content 3 §2's broad
// `<content-list>` both accept this unconditionally, so accept it
// in content lists for both string-set and content properties
// (the `content()` arm accepts both modes unconditionally even after
// introducing `ContentListMode` dispatch).
//
// CSS Content 3 §2.7.3 separately defines content() with `?` and five keywords
// (including `marker`), conflicting with the GCPM 3 §1.1.1.1 set. This
// implementation deliberately follows GCPM 3 §1.1.1.1 for keyword choices;
// thus it rejects `marker` even in the content property (see the
// `parse_content_fn` doc comment for details and reasoning). CSS Content 3 §2.7.3's
// `marker` keyword remains a known feature gap.
//
// Before the fix, `string-set: title content(text)` was
// silently dropped (parse_content_function had no matching arm →
// parse_content_list_items break → 0 items → parse_string_set None →
// declaration drop). Check that the new arm returns Some instead.

#[test]
fn string_set_content_text_reproduces_pre_fix_drop() {
    // Primary reproduction case from the description:
    // before the fix it dropped the declaration (None); afterward it returns
    // Some containing (title, `[Content{keyword: Text}]`).
    let entries = string_set_entries("title content(text)");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("title"));
    assert_eq!(
        entries[0].1,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_explicit_text_keyword() {
    // §1.1.1.1: `content(text)` gets the element's string value (the same keyword
    // as the fallback for bare `content()`, but preserve the explicit keyword
    // to leave room for downstream branching).
    let items = content_items("content(text)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_default_keyword_on_empty_parens() {
    // Spec example in §1.1.1.1: `h2 { string-set: heading content() }` (string-set
    // in the GCPM3 context). Bare `content()` uses `text` as fallback,
    // not because GCPM 3 defines a spec "default".
    // For the content property's conflicting grammar, see parse_content_fn's doc above.
    let items = content_items("content()");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_before_keyword() {
    // The `before` keyword is used in the §1.1.1.1 spec example
    // `h1 { string-set: header content(before) ':' content(text); }`.
    let items = content_items("content(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_after_keyword() {
    let items = content_items("content(after)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::After,
        }]
    );
}

#[test]
fn content_content_fn_first_letter_keyword() {
    let items = content_items("content(first-letter)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::FirstLetter,
        }]
    );
}

#[test]
fn content_content_fn_rejects_unknown_keyword() {
    // GCPM 3 §1.1.1.1 grammar has only four alternatives,
    // `[text | before | after | first-letter]`, in the string-set/GCPM3 context.
    // For any other identifier, parse_content_text_keyword returns None;
    // that propagates up, parse_content_list_items breaks, and the declaration
    // drops as None. `marker` is absent from this GCPM3 grammar. Although
    // CSS Content 3 §2.7.3 separately defines five keywords including `marker`,
    // this implementation follows GCPM 3 §1.1.1.1 for its keyword set
    // (see parse_content_fn's doc comment); rejecting `marker` is deliberate,
    // not an unresolved defect.
    // This test checks the current GCPM3-scoped implementation behavior;
    // it does not claim that `marker` is absent
    // from every specification.
    assert_eq!(parse("content(marker)", "content"), None);
    assert_eq!(parse("content(bogus)", "content"), None);
}

#[test]
fn content_content_fn_rejects_target_text_keyword() {
    // §1.1.1.1 offers the `text` alternative (target-text() §2.6.3 uses `content`).
    // Due to this spelling difference, `content(content)` is invalid per spec: reject it.
    // Regression check to prevent confusion through reuse of sibling ContentPart.
    assert_eq!(parse("content(content)", "content"), None);
}

#[test]
fn content_content_fn_case_insensitive_keyword() {
    // By CSS convention, keywords are ASCII case-insensitive.
    let items = content_items("content(TEXT)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        }]
    );
}

#[test]
fn content_content_fn_case_insensitive_function_name() {
    // parse_content_function dispatches ASCII case-insensitively, like its existing arms.
    let items = content_items("CONTENT(before)");
    assert_eq!(
        items,
        vec![ContentComponent::Content {
            keyword: ContentTextKeyword::Before,
        }]
    );
}

#[test]
fn content_content_fn_rejects_extra_argument() {
    // The grammar takes one argument. Any extra tokens are
    // rejected by parse_entirely inside parse_nested_block, dropping the declaration.
    assert_eq!(parse("content(text, extra)", "content"), None);
    assert_eq!(parse("content(text before)", "content"), None);
}

#[test]
fn string_set_content_fn_mixed_with_other_items() {
    // Spec example in §1.1.1.1:
    //   h1 { string-set: header content(before) ':' content(text); }
    // → (header, `[Content{Before}, Literal(":"), Content{Text}]`) 3 items.
    let entries = string_set_entries(r#"header content(before) ":" content(text)"#);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, SmolStr::new("header"));
    assert_eq!(
        entries[0].1,
        vec![
            ContentComponent::Content {
                keyword: ContentTextKeyword::Before,
            },
            ContentComponent::Literal(SmolStr::new(":")),
            ContentComponent::Content {
                keyword: ContentTextKeyword::Text,
            },
        ]
    );
}

// ── page (CSS Paged Media 3 §8.1) ──

#[test]
fn page_value_parses_auto_and_custom_ident() {
    assert_eq!(
        parse("auto", "page"),
        Some(PropertyValue::Page(PageValue::Auto))
    );
    assert_eq!(
        parse("table", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("table"))))
    );
    assert_eq!(
        parse("xyzabc", "page"),
        Some(PropertyValue::Page(PageValue::Named(Atom::from("xyzabc"))))
    );
    // CSS-wide keywords and `default` are not custom idents.
    for kw in [
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ] {
        assert_eq!(parse_entire(kw, "page"), None, "{kw}");
    }
    // two idents / dimension are grammar-outside (caller drops).
    assert_eq!(parse_entire("not valid", "page"), None);
    assert_eq!(parse_entire("123px", "page"), None);
}

#[test]
fn page_value_key_maps_to_page_property_key() {
    let v = PropertyValue::Page(PageValue::Auto);
    assert_eq!(v.key(), PropertyKey::Page);
}

// ── quotes property (CSS Content Module Level 3 §2.4.1 / legacy CSS2
// §12.3.1 grammar subset `none | [ <string> <string> ]+`) ──

fn quotes_pairs(pairs: &[(&str, &str)]) -> Arc<Vec<(SmolStr, SmolStr)>> {
    Arc::new(
        pairs
            .iter()
            .map(|(open, close)| (SmolStr::new(open), SmolStr::new(close)))
            .collect(),
    )
}

#[test]
fn quotes_none_returns_empty_vec() {
    // Spec: `none` is equivalent to an empty list (top-level alternative).
    // Use the shared Arc slot (`empty_quotes_entries`) for the empty case.
    assert_eq!(
        parse("none", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_is_case_insensitive_on_none() {
    // CSS spec: the `none` keyword is ASCII case-insensitive.
    assert_eq!(
        parse("NONE", "quotes"),
        Some(PropertyValue::Quotes(empty_quotes_entries()))
    );
}

#[test]
fn quotes_single_pair() {
    assert_eq!(
        parse(r#""«" "»""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[("«", "»")])))
    );
}

#[test]
fn quotes_multiple_pairs_deeper_nesting_levels() {
    // The second pair is at a deeper nesting level than the first (see the
    // "levels of nesting" section of the `PropertyValue::Quotes` doc).
    assert_eq!(
        parse(r#""«" "»" "‹" "›""#, "quotes"),
        Some(PropertyValue::Quotes(quotes_pairs(&[
            ("«", "»"),
            ("‹", "›"),
        ])))
    );
}

#[test]
fn quotes_rejects_empty_declaration() {
    // The grammar is `[ <string> <string> ]+`: zero pairs (not `none`, just
    // an empty value) are invalid; drop the declaration.
    assert_eq!(parse("", "quotes"), None);
}

#[test]
fn quotes_rejects_odd_number_of_strings() {
    // A trailing unpaired <string> does not match `[ <string> <string> ]+`
    // (see the "malformed pair" section of the `parse_quotes_property` doc).
    assert_eq!(parse(r#""«" "»" "‹""#, "quotes"), None);
}

#[test]
fn quotes_rejects_non_string_token() {
    // A non-<string> token (bare identifier) violates the grammar.
    assert_eq!(parse("open close", "quotes"), None);
}

#[test]
fn quotes_key_maps_to_quotes_property_key() {
    assert_eq!(
        PropertyValue::Quotes(empty_quotes_entries()).key(),
        PropertyKey::Quotes
    );
}
