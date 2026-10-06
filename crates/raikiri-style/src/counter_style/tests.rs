use super::*;

// ── Parsing: rule name / prelude ──────────────────────────────────

#[test]
fn minimal_cyclic_rule_parses() {
    let rules =
        parse_counter_style_rules(r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name.as_str(), "thumbs");
    assert_eq!(rules[0].system, CounterStyleSystem::Cyclic);
    assert_eq!(rules[0].symbols, vec![CounterSymbol(SmolStr::new("*"))]);
}

#[test]
fn defaulted_system_is_symbolic() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].system, CounterStyleSystem::Symbolic);
}

#[test]
fn reserved_rule_names_are_dropped() {
    for name in [
        "decimal",
        "disc",
        "circle",
        "square",
        "disclosure-open",
        "disclosure-closed",
        "none",
        "DECIMAL",
    ] {
        let src = format!(r#"@counter-style {name} {{ system: cyclic; symbols: "*"; }}"#);
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes when the assertion fails; every iteration here
        // passes, so llvm-cov reports the macro's condition-false region
        // as an uncovered added line (attributed to the `assert!(`
        // line) even though the assertion itself runs, and does its
        // job, on every iteration.
        assert!(
            parse_counter_style_rules(&src).is_empty(),
            "expected {name:?} to be rejected as a rule name"
        );
    }
}

#[test]
fn reserved_names_are_still_valid_as_fallback_references() {
    // §3 dfn: those keywords "are valid <counter-style-name>s, but are
    // invalid when used here to name a counter style rule" — i.e. valid
    // as a *reference*.
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: cyclic; symbols: "*"; fallback: disc; }"#,
    );
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].fallback.as_str(), "disc");
}

#[test]
fn non_counter_style_at_rules_and_qualified_rules_are_ignored() {
    let rules = parse_counter_style_rules(
        r#"
            p { color: red }
            @media print { p { color: blue } }
            @counter-style thumbs { system: cyclic; symbols: "*"; }
            "#,
    );
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name.as_str(), "thumbs");
}

#[test]
fn empty_source_produces_no_rules() {
    assert!(parse_counter_style_rules("").is_empty());
}

// ── Parsing: individual descriptors ───────────────────────────────

#[test]
fn system_extends_stores_the_extended_name() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { system: extends decimal; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0].system,
        CounterStyleSystem::Extends(SmolStr::new("decimal"))
    );
}

#[test]
fn system_fixed_default_first_value_is_one() {
    let rules =
        parse_counter_style_rules(r#"@counter-style foo { system: fixed; symbols: "a" "b"; }"#);
    assert_eq!(
        rules[0].system,
        CounterStyleSystem::Fixed {
            first_symbol_value: 1
        }
    );
}

#[test]
fn system_fixed_explicit_first_value() {
    let rules =
        parse_counter_style_rules(r#"@counter-style foo { system: fixed 5; symbols: "a"; }"#);
    assert_eq!(
        rules[0].system,
        CounterStyleSystem::Fixed {
            first_symbol_value: 5
        }
    );
}

#[test]
fn system_unknown_keyword_drops_declaration_system_stays_default() {
    // `bogus` matches none of the 7 `system` keywords -> `parse_system`
    // returns `None` -> the whole `system` declaration is dropped (this
    // crate's usual invalid-declaration handling), leaving `system` at
    // its spec-initial value `symbolic`. `symbols: "*"` (1 entry) is
    // enough for `symbolic`'s own validity minimum, so the rest of the
    // rule still parses.
    let rules = parse_counter_style_rules(r#"@counter-style foo { system: bogus; symbols: "*"; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].system, CounterStyleSystem::Symbolic);
}

#[test]
fn negative_descriptor_prefix_only() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: numeric; symbols: "0" "1"; negative: "neg-"; }"#,
    );
    assert_eq!(
        rules[0].negative,
        NegativeDescriptor {
            prefix: CounterSymbol(SmolStr::new("neg-")),
            suffix: None,
        }
    );
}

#[test]
fn negative_descriptor_prefix_and_suffix() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: numeric; symbols: "0" "1"; negative: "(" ")"; }"#,
    );
    assert_eq!(
        rules[0].negative,
        NegativeDescriptor {
            prefix: CounterSymbol(SmolStr::new("(")),
            suffix: Some(CounterSymbol(SmolStr::new(")"))),
        }
    );
}

#[test]
fn negative_descriptor_default_is_hyphen_minus_only() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
    assert_eq!(rules[0].negative, NegativeDescriptor::default());
    assert_eq!(rules[0].negative.prefix.as_str(), "-");
}

#[test]
fn prefix_and_suffix_descriptors() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { symbols: "*"; prefix: "["; suffix: "]"; }"#,
    );
    assert_eq!(rules[0].prefix, CounterSymbol(SmolStr::new("[")));
    assert_eq!(rules[0].suffix, CounterSymbol(SmolStr::new("]")));
}

#[test]
fn default_prefix_suffix_match_spec_initial_values() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
    assert_eq!(rules[0].prefix.as_str(), "");
    assert_eq!(rules[0].suffix.as_str(), ". ");
}

#[test]
fn pad_integer_then_symbol_order() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: 3 "0"; }"#);
    assert_eq!(
        rules[0].pad,
        PadDescriptor {
            min_length: 3,
            symbol: CounterSymbol(SmolStr::new("0")),
        }
    );
}

#[test]
fn pad_symbol_then_integer_order() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: "0" 3; }"#);
    assert_eq!(
        rules[0].pad,
        PadDescriptor {
            min_length: 3,
            symbol: CounterSymbol(SmolStr::new("0")),
        }
    );
}

#[test]
fn pad_negative_integer_is_rejected() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: -1 "0"; }"#);
    // Whole `pad` declaration dropped, rest of the rule survives with
    // pad at its default.
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pad, PadDescriptor::default());
}

#[test]
fn pad_negative_integer_is_rejected_in_symbol_first_order_too() {
    // Sibling of `pad_negative_integer_is_rejected` above, but for the
    // *other* `&&` order (`<symbol> <integer>` instead of `<integer>
    // <symbol>`) — `parse_nonneg_int_and_symbol`'s second branch.
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: "0" -1; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pad, PadDescriptor::default());
}

#[test]
fn range_single_pair() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: 1 5; }"#);
    assert_eq!(
        rules[0].range,
        CounterRange::List(vec![RangeEntry {
            lower: RangeLimit::Finite(1),
            upper: RangeLimit::Finite(5),
        }])
    );
}

#[test]
fn range_comma_separated_multiple_pairs() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { symbols: "*"; range: 1 5, 10 infinite; }"#,
    );
    assert_eq!(
        rules[0].range,
        CounterRange::List(vec![
            RangeEntry {
                lower: RangeLimit::Finite(1),
                upper: RangeLimit::Finite(5),
            },
            RangeEntry {
                lower: RangeLimit::Finite(10),
                upper: RangeLimit::Infinite,
            },
        ])
    );
}

#[test]
fn range_lower_greater_than_upper_reverts_to_auto() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: 5 1; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].range, CounterRange::Auto);
}

#[test]
fn range_auto_keyword_is_explicit_default() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; range: auto; }"#);
    assert_eq!(rules[0].range, CounterRange::Auto);
}

#[test]
fn fallback_default_is_decimal() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; }"#);
    assert_eq!(rules[0].fallback.as_str(), "decimal");
}

#[test]
fn fallback_custom_name() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { symbols: "*"; fallback: my-other-style; }"#,
    );
    assert_eq!(rules[0].fallback.as_str(), "my-other-style");
}

#[test]
fn fallback_none_is_rejected_declaration_dropped() {
    // `<counter-style-name>` (`parse_counter_style_name_ref`) excludes
    // `none` even in *reference* position (unlike the 6 predefined-style
    // keywords, which are valid references — see
    // `reserved_names_are_still_valid_as_fallback_references` above).
    // The whole `fallback` declaration is dropped, leaving `fallback` at
    // its spec-initial value `decimal`.
    let rules =
        parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; fallback: none; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].fallback.as_str(), "decimal");
}

#[test]
fn additive_symbols_descending_weight_parses() {
    let rules = parse_counter_style_rules(
        r#"@counter-style roman-ish { system: additive; additive-symbols: 10 "X", 5 "V", 1 "I"; }"#,
    );
    assert_eq!(
        rules[0].additive_symbols,
        vec![
            (10, CounterSymbol(SmolStr::new("X"))),
            (5, CounterSymbol(SmolStr::new("V"))),
            (1, CounterSymbol(SmolStr::new("I"))),
        ]
    );
}

#[test]
fn additive_symbols_non_descending_order_invalidates_whole_rule() {
    // system: additive requires additive_symbols non-empty (`is_valid`);
    // a non-descending order drops the *declaration* (reverts to empty),
    // which then fails whole-rule validity for `additive`.
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: additive; additive-symbols: 1 "I", 5 "V"; }"#,
    );
    assert!(rules.is_empty());
}

#[test]
fn additive_without_additive_symbols_is_whole_rule_invalid() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { system: additive; }"#);
    assert!(rules.is_empty());
}

#[test]
fn extends_forbids_symbols_whole_rule_invalid() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: extends decimal; symbols: "*"; }"#,
    );
    assert!(rules.is_empty());
}

#[test]
fn extends_forbids_additive_symbols_whole_rule_invalid() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { system: extends decimal; additive-symbols: 1 "I"; }"#,
    );
    assert!(rules.is_empty());
}

#[test]
fn extends_alone_is_valid() {
    let rules = parse_counter_style_rules(r#"@counter-style foo { system: extends decimal; }"#);
    assert_eq!(rules.len(), 1);
}

#[test]
fn cyclic_requires_at_least_one_symbol() {
    assert!(parse_counter_style_rules(r#"@counter-style foo { system: cyclic; }"#).is_empty());
}

#[test]
fn numeric_requires_at_least_two_symbols() {
    assert!(
        parse_counter_style_rules(r#"@counter-style foo { system: numeric; symbols: "0"; }"#)
            .is_empty()
    );
    assert_eq!(
        parse_counter_style_rules(r#"@counter-style foo { system: numeric; symbols: "0" "1"; }"#)
            .len(),
        1
    );
}

#[test]
fn alphabetic_requires_at_least_two_symbols() {
    assert!(
        parse_counter_style_rules(r#"@counter-style foo { system: alphabetic; symbols: "a"; }"#)
            .is_empty()
    );
}

#[test]
fn unsupported_descriptor_speak_as_is_silently_dropped() {
    let rules =
        parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; speak-as: numbers; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].symbols, vec![CounterSymbol(SmolStr::new("*"))]);
}

#[test]
fn later_declaration_of_same_descriptor_wins() {
    let rules = parse_counter_style_rules(
        r#"@counter-style foo { prefix: "a"; prefix: "b"; symbols: "*"; }"#,
    );
    assert_eq!(rules[0].prefix, CounterSymbol(SmolStr::new("b")));
}

#[test]
fn trailing_garbage_after_descriptor_value_drops_declaration() {
    // `pad`'s grammar is exactly `<integer> && <symbol>` (no
    // repetition), so a 3rd token after both components is genuine
    // leftover — `CounterStyleDeclParser::parse_value`'s
    // `expect_exhausted()` call rejects the whole declaration rather
    // than silently accepting the `3 "0"` prefix. `pad` stays at its
    // default; the rest of the rule survives.
    let rules =
        parse_counter_style_rules(r#"@counter-style foo { symbols: "*"; pad: 3 "0" garbage; }"#);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pad, PadDescriptor::default());
}

// ── Registry ───────────────────────────────────────────────────────

#[test]
fn registry_from_source_and_get() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
    );
    assert_eq!(registry.len(), 1);
    assert!(registry.get("thumbs").is_some());
    assert!(registry.get("unknown").is_none());
}

#[test]
fn registry_empty_is_empty() {
    let registry = CounterStyleRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn later_same_name_rule_replaces_entirely() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style foo { system: cyclic; symbols: "a"; prefix: "["; }
            @counter-style foo { system: numeric; symbols: "0" "1"; }
            "#,
    );
    assert_eq!(registry.len(), 1);
    let rule = registry.get("foo").unwrap();
    assert_eq!(rule.system, CounterStyleSystem::Numeric);
    // The first rule's `prefix: "["` did NOT survive — atomic replace,
    // not a per-descriptor merge.
    assert_eq!(rule.prefix.as_str(), "");
}

#[test]
fn insert_rejects_invalid_rule_constructed_directly() {
    let mut registry = CounterStyleRegistry::new();
    let mut invalid = CounterStyleRule::new(SmolStr::new("foo"));
    invalid.system = CounterStyleSystem::Additive; // additive_symbols left empty -> invalid
    registry.insert(invalid);
    assert!(registry.is_empty());
}

#[test]
fn insert_with_origin_rejects_invalid_rule_constructed_directly() {
    // insert_with_origin has its own is_valid
    // gate, same enforcement point as insert() above — but every
    // production caller (RuleTree::add_stylesheet) only ever feeds it
    // rules that already passed parse_counter_style_rules's own
    // is_valid filter, so that gate is otherwise unreachable through
    // add_stylesheet. Exercise it directly, the same way
    // insert_rejects_invalid_rule_constructed_directly does for
    // insert(). Origin::UserAgent here is an arbitrary choice — the
    // is_valid check is origin-independent, it runs before the
    // origin-precedence resolution.
    let mut registry = CounterStyleRegistry::new();
    let mut invalid = CounterStyleRule::new(SmolStr::new("foo"));
    invalid.system = CounterStyleSystem::Additive; // additive_symbols left empty -> invalid
    registry.insert_with_origin(invalid, Origin::UserAgent);
    assert!(registry.is_empty());
}

// ── resolve_custom_counter: cyclic ────────────────────────────────

#[test]
fn resolve_cyclic_wraps_through_symbols() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", 1).as_deref(),
        Some("A")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", 2).as_deref(),
        Some("B")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", 3).as_deref(),
        Some("C")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", 4).as_deref(),
        Some("A")
    );
}

#[test]
fn resolve_cyclic_handles_zero_and_negative_without_negative_sign() {
    // Cyclic never uses a negative sign — negative values just keep
    // cycling per `(value-1) mod N`, no `-` wrapping.
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", 0).as_deref(),
        Some("C")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", -1).as_deref(),
        Some("B")
    );
}

// ── resolve_custom_counter: numeric ───────────────────────────────

#[test]
fn resolve_numeric_base_three() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "base3", 0).as_deref(),
        Some("0")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "base3", 5).as_deref(),
        Some("12")
    );
}

#[test]
fn resolve_numeric_negative_wraps_with_default_negative_sign() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "base3", -5).as_deref(),
        Some("-12")
    );
}

#[test]
fn resolve_numeric_negative_with_custom_negative_descriptor() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style base3 { system: numeric; symbols: "0" "1" "2"; negative: "(" ")"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "base3", -5).as_deref(),
        Some("(12)")
    );
}

// ── resolve_custom_counter: alphabetic ────────────────────────────

#[test]
fn resolve_alphabetic_bijective_base26() {
    let symbols = ('a'..='z')
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!(r#"@counter-style latin {{ system: alphabetic; symbols: {symbols}; }}"#);
    let registry = CounterStyleRegistry::from_source(&src);
    assert_eq!(
        resolve_custom_counter(&registry, "latin", 1).as_deref(),
        Some("a")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "latin", 26).as_deref(),
        Some("z")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "latin", 27).as_deref(),
        Some("aa")
    );
}

// ── resolve_custom_counter: symbolic ──────────────────────────────

#[test]
fn resolve_symbolic_doubles_symbol_on_successive_passes() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style stars { system: symbolic; symbols: "*"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "stars", 1).as_deref(),
        Some("*")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "stars", 2).as_deref(),
        Some("**")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "stars", 3).as_deref(),
        Some("***")
    );
}

#[test]
fn symbolic_repr_byte_budget_boundary_is_not_rejected() {
    let symbols = vec![CounterSymbol(SmolStr::new("*"))];
    // 1-byte symbol, so `reps == value == byte count`:
    // `value == MAX_REPR_BYTES` sits exactly on the cap's own
    // boundary — asserting it still produces the un-truncated,
    // full-length representation guards against an off-by-one that
    // rejects the boundary value itself. (Ordinary small-`reps`
    // behavior — no cap involvement at all — is already covered by
    // `resolve_symbolic_doubles_symbol_on_successive_passes` above,
    // with values 1/2/3.)
    let value = MAX_REPR_BYTES as i64;
    assert_eq!(
        symbolic_repr(&symbols, value).as_deref(),
        Some("*".repeat(value as usize).as_str())
    );
}

#[test]
fn symbolic_repr_caps_reps_to_bound_allocation() {
    let symbols = vec![CounterSymbol(SmolStr::new("*"))];
    // 1-byte symbol, so `reps * symbol.len() == value`:
    // `MAX_REPR_BYTES + 1` is the first value whose representation
    // byte size exceeds the cap.
    assert_eq!(symbolic_repr(&symbols, MAX_REPR_BYTES as i64 + 1), None);
    // A pathologically large counter value must not drive an
    // unbounded allocation — it hits the same cap, not `i32::MAX` reps.
    assert_eq!(symbolic_repr(&symbols, i64::from(i32::MAX)), None);
}

/// Regression: a long (not 1-codepoint) `<symbol>` `<string>` at a
/// repetition count comfortably under a hypothetical
/// repetition-count-only cap (4096) would still exceed the byte budget
/// once symbol length is accounted for. This is exactly the bypass a
/// repetition-count-only cap misses — `reps <= 4096` alone says nothing
/// about the symbol's own length, and a `<symbol>` `<string>` has no
/// parser-enforced length limit (`parse_symbol`).
#[test]
fn symbolic_repr_long_symbol_bypasses_repetition_only_cap() {
    let long_symbol = "x".repeat(100); // 100 bytes, not 1.
    let symbols = vec![CounterSymbol(SmolStr::new(long_symbol))];
    // 100 bytes * 4096 reps = 409,600 bytes, far over `MAX_REPR_BYTES`
    // (64 KiB) even though 4096 reps alone was previously accepted.
    assert_eq!(symbolic_repr(&symbols, 4096), None);
}

/// The flip side of the regression above: a long symbol used only a
/// handful of times is legitimate content well within the byte budget,
/// and must not be wrongly rejected just because it is "long" — the cap
/// is on total bytes, not on symbol length in isolation.
#[test]
fn symbolic_repr_long_symbol_with_few_reps_is_not_wrongly_rejected() {
    let long_symbol = "x".repeat(1000);
    let symbols = vec![CounterSymbol(SmolStr::new(long_symbol.clone()))];
    assert_eq!(
        symbolic_repr(&symbols, 1).as_deref(),
        Some(long_symbol.as_str())
    );
}

/// Regression: an empty (`""`) symbol's true representation
/// is always the empty string, at any repetition count, since
/// `reps * 0 == 0` bytes never exceeds the budget — unlike the earlier
/// repetition-count-only cap, which rejected large-`value` empty-symbol
/// representations even though their true rendered length is 0
/// codepoints and could never itself drive a large allocation.
#[test]
fn symbolic_repr_empty_symbol_never_capped_regardless_of_value() {
    let symbols = vec![CounterSymbol(SmolStr::new(""))];
    assert_eq!(symbolic_repr(&symbols, 5000).as_deref(), Some(""));
    assert_eq!(
        symbolic_repr(&symbols, i64::from(i32::MAX)).as_deref(),
        Some("")
    );
}

/// `reps * symbol.len()` can overflow `u64` for a sufficiently large
/// `value` paired with a long symbol — `checked_repr_bytes` must treat
/// that as "exceeds budget" rather than panicking or wrapping around to
/// a small, wrongly-accepted byte count.
#[test]
fn symbolic_repr_extreme_value_and_long_symbol_does_not_overflow() {
    let long_symbol = "x".repeat(2000);
    let symbols = vec![CounterSymbol(SmolStr::new(long_symbol))];
    assert_eq!(symbolic_repr(&symbols, i64::MAX), None);
}

#[test]
fn resolve_symbolic_extreme_value_falls_back_to_custom_style_in_registry() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style stars { system: symbolic; symbols: "*"; fallback: overflow; }
            @counter-style overflow { system: cyclic; symbols: "%"; }
            "#,
    );
    // `i32::MAX` is within symbolic's auto range (1..∞, §3.5), so step 2
    // of `generate_counter` does not intercept it — this genuinely
    // reaches `symbolic_repr`'s cap and exercises the `None` ->
    // fallback routing end to end, not just the direct cap check above.
    assert_eq!(
        resolve_custom_counter(&registry, "stars", i32::MAX).as_deref(),
        Some("%")
    );
}

// ── resolve_custom_counter: fixed ─────────────────────────────────

#[test]
fn resolve_fixed_within_window() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style small { system: fixed 1; symbols: "one" "two" "three"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "small", 1).as_deref(),
        Some("one")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "small", 3).as_deref(),
        Some("three")
    );
}

#[test]
fn resolve_fixed_exhausted_falls_back_to_unresolvable_decimal_default() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style small { system: fixed 1; symbols: "one" "two" "three"; }"#,
    );
    // fallback defaults to "decimal", which is not itself in this
    // registry — see `resolve_custom_counter`'s doc on why that's `None`
    // rather than this crate formatting "4" itself.
    assert_eq!(resolve_custom_counter(&registry, "small", 4), None);
}

#[test]
fn resolve_fixed_exhausted_falls_back_to_custom_style_in_registry() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style small { system: fixed 1; symbols: "one" "two"; fallback: overflow; }
            @counter-style overflow { system: cyclic; symbols: "%"; }
            "#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "small", 5).as_deref(),
        Some("%")
    );
}

// ── resolve_custom_counter: additive ──────────────────────────────

#[test]
fn resolve_additive_roman_like() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style toy-roman { system: additive; additive-symbols: 10 "X", 5 "V", 1 "I"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "toy-roman", 7).as_deref(),
        Some("VII")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "toy-roman", 11).as_deref(),
        Some("XI")
    );
}

#[test]
fn additive_repr_byte_budget_boundary_is_not_rejected() {
    let tuples = vec![(1, CounterSymbol(SmolStr::new("I")))];
    // Weight-1 tuple with a 1-byte symbol, so `reps == value == byte
    // count`: `value == MAX_REPR_BYTES` sits exactly on the cap's own
    // boundary — still fully represented, not truncated. (Ordinary
    // small-`reps` behavior — no cap involvement at all — is already
    // covered by `resolve_additive_roman_like` above, with values
    // 7/11.)
    let value = MAX_REPR_BYTES as i64;
    assert_eq!(
        additive_repr(&tuples, value).as_deref(),
        Some("I".repeat(value as usize).as_str())
    );
}

#[test]
fn additive_repr_caps_reps_to_bound_allocation() {
    let tuples = vec![(1, CounterSymbol(SmolStr::new("I")))];
    // First value whose single-tuple `reps * symbol.len()` exceeds the
    // byte budget.
    assert_eq!(additive_repr(&tuples, MAX_REPR_BYTES as i64 + 1), None);
    // A pathologically large counter value must not drive an
    // unbounded allocation.
    assert_eq!(additive_repr(&tuples, i64::from(i32::MAX)), None);
}

#[test]
fn additive_repr_caps_cumulative_bytes_across_tuples() {
    // No single tuple's byte contribution exceeds the budget on its
    // own, but the running total across tuples does — the cap must
    // catch this case too, not just a single oversized tuple.
    let tuples = vec![
        (2, CounterSymbol(SmolStr::new("A"))),
        (1, CounterSymbol(SmolStr::new("B"))),
    ];
    // `value` chosen so the first tuple alone contributes exactly
    // `MAX_REPR_BYTES` bytes (1-byte symbol, value / 2 == MAX_REPR_BYTES,
    // remainder 1), leaving a nonzero remainder that would need a
    // second tuple's bytes on top — pushing the cumulative total over
    // the budget.
    let value = MAX_REPR_BYTES as i64 * 2 + 1;
    assert_eq!(additive_repr(&tuples, value), None);
}

/// Regression, the additive-system sibling of
/// `symbolic_repr_long_symbol_bypasses_repetition_only_cap` — a long
/// tuple symbol at a repetition count comfortably under a hypothetical
/// repetition-count-only cap would still exceed the byte budget.
#[test]
fn additive_repr_long_symbol_bypasses_repetition_only_cap() {
    let long_symbol = "x".repeat(100); // 100 bytes, not 1.
    let tuples = vec![(1, CounterSymbol(SmolStr::new(long_symbol)))];
    // Weight-1 tuple, so `reps == value == 4096`; 100 bytes * 4096 reps
    // = 409,600 bytes, far over `MAX_REPR_BYTES` (64 KiB) even though
    // 4096 reps alone was previously accepted.
    assert_eq!(additive_repr(&tuples, 4096), None);
}

/// Regression, the additive-system sibling of
/// `symbolic_repr_empty_symbol_never_capped_regardless_of_value`: a
/// zero-weight tuple with an empty symbol never trips the cap, at any
/// value, since it contributes 0 bytes regardless of how many times a
/// *different* tuple in the same list repeats.
#[test]
fn additive_repr_empty_symbol_tuple_never_capped_regardless_of_value() {
    let tuples = vec![(1, CounterSymbol(SmolStr::new("")))];
    let value = i64::from(i32::MAX);
    assert_eq!(additive_repr(&tuples, value).as_deref(), Some(""));
}

/// `reps * symbol.len()` can overflow `u64` for a sufficiently large
/// `value` paired with a long tuple symbol — must be treated as
/// "exceeds budget", not panic or wrap around.
#[test]
fn additive_repr_extreme_value_and_long_symbol_does_not_overflow() {
    let long_symbol = "x".repeat(2000);
    let tuples = vec![(1, CounterSymbol(SmolStr::new(long_symbol)))];
    assert_eq!(additive_repr(&tuples, i64::MAX), None);
}

#[test]
fn additive_repr_zero_value_with_zero_weight_tuple_returns_symbol() {
    let tuples = vec![(0, CounterSymbol(SmolStr::new("Z")))];
    assert_eq!(additive_repr(&tuples, 0).as_deref(), Some("Z"));
}

#[test]
fn additive_repr_zero_value_without_zero_weight_tuple_returns_none() {
    let tuples = vec![(1, CounterSymbol(SmolStr::new("I")))];
    assert_eq!(additive_repr(&tuples, 0), None);
}

/// The `value == 0` branch built its matching tuple's symbol
/// string directly, bypassing the byte-budget check applied everywhere
/// else in this function. A weight-0 tuple is parser-valid
/// (`additive-symbols` accepts a weight of 0, and the additive system's
/// auto range — `auto_range` — includes 0), and `<symbol>`'s own length
/// has no parser-enforced limit, so a single such tuple with an
/// arbitrarily long symbol string still allocated unboundedly at
/// `value == 0` regardless of this function's byte budget.
#[test]
fn additive_repr_zero_value_long_symbol_bypasses_cap() {
    let long_symbol = "x".repeat(MAX_REPR_BYTES as usize + 1);
    let tuples = vec![(0, CounterSymbol(SmolStr::new(long_symbol)))];
    assert_eq!(additive_repr(&tuples, 0), None);
}

#[test]
fn resolve_additive_extreme_value_falls_back_to_custom_style_in_registry() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style toy-roman { system: additive; additive-symbols: 1 "I"; fallback: overflow; }
            @counter-style overflow { system: cyclic; symbols: "%"; }
            "#,
    );
    // `i32::MAX` is within additive's auto range (0..∞, §3.5), so step 2
    // of `generate_counter` does not intercept it — this genuinely
    // reaches `additive_repr`'s cap and exercises the `None` ->
    // fallback routing end to end, not just the direct cap check above.
    assert_eq!(
        resolve_custom_counter(&registry, "toy-roman", i32::MAX).as_deref(),
        Some("%")
    );
}

#[test]
fn resolve_additive_unrepresentable_value_falls_back() {
    // No tuple/combination reaches exactly 0 for value 2 given only a
    // weight-1 "I" as the smallest tuple would actually make 2
    // representable ("II") — use a registry where the smallest weight
    // is too large to reach 0 for the probed value.
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style big-only { system: additive; additive-symbols: 100 "C"; }"#,
    );
    assert_eq!(resolve_custom_counter(&registry, "big-only", 50), None);
}

#[test]
fn resolve_additive_zero_weight_tuple_handles_value_zero() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style with-zero { system: additive; additive-symbols: 1 "I", 0 "Z"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "with-zero", 0).as_deref(),
        Some("Z")
    );
}

// ── resolve_custom_counter: range / pad ───────────────────────────

#[test]
fn resolve_custom_range_out_of_bounds_falls_back() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style limited { system: cyclic; symbols: "*"; range: 1 3; }"#,
    );
    assert_eq!(resolve_custom_counter(&registry, "limited", 4), None);
    assert_eq!(
        resolve_custom_counter(&registry, "limited", 2).as_deref(),
        Some("*")
    );
}

#[test]
fn resolve_pad_prepends_symbol_to_minimum_length() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", 5).as_deref(),
        Some("005")
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", 500).as_deref(),
        Some("500")
    );
}

/// §3.6's own worked example (verbatim quote on `apply_pad`): the pad
/// `difference` is reduced by the negative descriptor's own length
/// *before* padding, so a negative value's sign counts toward
/// `pad.min_length` — `pad: 3 "0"` + value `-5` (default `negative: "-"`,
/// length 1) must be `"-05"`, not `"-005"`.
#[test]
fn resolve_pad_reduces_difference_by_default_negative_sign_length() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", -5).as_deref(),
        Some("-05")
    );
}

/// Same rule as the sibling test above, but with a 2-symbol `negative`
/// descriptor (`"(" ")"`, combined length 2) — `pad: 3 "0"` + value `-5`
/// has `difference = 3 - 1 - 2 = 0`, so no padding is prepended at all
/// and the result is `"(5)"`, not `"(005)"`.
#[test]
fn resolve_pad_reduces_difference_by_two_symbol_negative_descriptor_length() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; negative: "(" ")"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", -5).as_deref(),
        Some("(5)")
    );
}

/// Padding for a *positive* value must not be affected by the negative
/// descriptor's length — `negative_reserved` is `0` whenever the value
/// isn't negative, per §3.6's own gating clause.
#[test]
fn resolve_pad_unaffected_by_negative_descriptor_for_positive_value() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style padded { system: numeric; symbols: "0" "1" "2" "3" "4" "5" "6" "7" "8" "9"; pad: 3 "0"; negative: "(" ")"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", 5).as_deref(),
        Some("005")
    );
}

/// The `generate_counter` step-4 gate (`uses_negative && value_i64 < 0`,
/// see its doc) is also false when the *value* is negative but the
/// *system* doesn't use a negative sign (`cyclic` here) — not just when
/// the value is non-negative (the sibling test above). `negative_reserved`
/// must still be `0` in that case, so the pad `difference` is computed
/// from `repr`'s length alone: `cyclic` with 3 symbols and value `-5`
/// produces the single symbol `"A"` (`(-5-1).rem_euclid(3) == 0`), and
/// `pad: 3 "0"` pads it to `"00A"` — the 2-symbol `negative: "(" ")"`
/// descriptor (length 2) must NOT be subtracted from the difference, and
/// step 5's negative-sign wrapping must not apply either.
#[test]
fn resolve_pad_unaffected_by_negative_descriptor_for_negative_value_without_negative_sign() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style thumbs { system: cyclic; symbols: "A" "B" "C"; pad: 3 "0"; negative: "(" ")"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "thumbs", -5).as_deref(),
        Some("00A")
    );
}

// ── apply_pad: byte-budget cap (same DoS class as symbolic/additive) ──

#[test]
fn apply_pad_within_byte_budget_pads_normally() {
    let pad = PadDescriptor {
        min_length: 10,
        symbol: CounterSymbol(SmolStr::new("0")),
    };
    assert_eq!(
        apply_pad(&pad, "5".to_string(), 0).as_deref(),
        Some("0000000005")
    );
}

/// `pad.min_length` is an unbounded
/// `<integer [0,∞]>` (`parse_pad`) used directly as a `String::repeat`
/// count with no upper bound — the same unbounded-allocation shape as
/// `symbolic_repr`/`additive_repr`, reachable unconditionally from
/// `generate_counter` for every counter system, not just
/// symbolic/additive.
#[test]
fn apply_pad_huge_min_length_caps_to_bound_allocation() {
    let pad = PadDescriptor {
        min_length: u32::MAX,
        symbol: CounterSymbol(SmolStr::new("0")),
    };
    // `diff` is on the order of `u32::MAX` (~4.3 billion) even after
    // subtracting `repr`'s own length — 1 byte/rep alone is far over
    // `MAX_REPR_BYTES` (64 KiB).
    assert_eq!(apply_pad(&pad, "5".to_string(), 0), None);
}

/// Same vulnerability class as `apply_pad_huge_min_length_caps_to_bound_allocation`,
/// but via a long `pad` *symbol* at a modest `min_length` instead of an
/// extreme `min_length` — mirrors the `symbolic_repr`/`additive_repr`
/// long-symbol bypass of a would-be count-only cap: a `min_length` far
/// too small to look suspicious on its own can still drive a large
/// allocation once the pad symbol's own byte length is a free variable.
#[test]
fn apply_pad_long_symbol_at_modest_min_length_caps_to_bound_allocation() {
    let long_symbol = "x".repeat(2000);
    let pad = PadDescriptor {
        min_length: 100,
        symbol: CounterSymbol(SmolStr::new(long_symbol)),
    };
    // diff = 100 - 0 = 100; 100 * 2000 bytes = 200,000 bytes, over
    // `MAX_REPR_BYTES`.
    assert_eq!(apply_pad(&pad, String::new(), 0), None);
}

/// This function checked only the newly-added pad portion's
/// byte count against `MAX_REPR_BYTES`, not the total of the existing
/// `repr`'s own bytes plus the pad addition. `repr` here is exactly
/// `MAX_REPR_BYTES` bytes — itself within budget, the same boundary
/// `symbolic_repr`/`additive_repr` accept on their own (see
/// `symbolic_repr_byte_budget_boundary_is_not_rejected`) — and the pad
/// addition is a single extra byte, comfortably within budget in
/// isolation. Only measuring the combined total catches that the two
/// together exceed the cap by one byte.
#[test]
fn apply_pad_caps_combined_repr_and_pad_bytes_not_just_pad_portion() {
    let repr = "x".repeat(MAX_REPR_BYTES as usize);
    let pad = PadDescriptor {
        min_length: MAX_REPR_BYTES as u32 + 1,
        symbol: CounterSymbol(SmolStr::new("0")),
    };
    // diff = (MAX_REPR_BYTES + 1) - MAX_REPR_BYTES = 1; pad_bytes = 1,
    // trivially within budget alone. Combined with `repr`'s own
    // `MAX_REPR_BYTES` bytes, the total is `MAX_REPR_BYTES + 1`.
    assert_eq!(apply_pad(&pad, repr, 0), None);
}

/// Regression, the `apply_pad` sibling of
/// `symbolic_repr_empty_symbol_never_capped_regardless_of_value`: an
/// empty pad symbol contributes 0 bytes regardless of `min_length`, so
/// it never trips the cap even at `u32::MAX`.
#[test]
fn apply_pad_empty_symbol_never_capped_even_with_huge_min_length() {
    let pad = PadDescriptor {
        min_length: u32::MAX,
        symbol: CounterSymbol(SmolStr::new("")),
    };
    // An empty pad symbol can never actually lengthen the
    // representation (repeating "" contributes nothing), so `repr` is
    // returned unchanged rather than falling back.
    assert_eq!(apply_pad(&pad, "5".to_string(), 0).as_deref(), Some("5"));
}

/// End-to-end: an oversized `pad` descriptor routes through
/// `generate_counter`'s fallback hop exactly like a `symbolic_repr`/
/// `additive_repr` cap trip does, not just at the direct `apply_pad`
/// level above.
#[test]
fn resolve_pad_oversized_falls_back_to_custom_style_in_registry() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style padded { system: cyclic; symbols: "*"; pad: 100000 "0"; fallback: overflow; }
            @counter-style overflow { system: cyclic; symbols: "%"; }
            "#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "padded", 1).as_deref(),
        Some("%")
    );
}

// ── resolve_custom_counter: fixed auto-range is unbounded (§3.5) ──

/// §3.5 verbatim: "For cyclic, numeric, and fixed systems, the range is
/// negative infinity to positive infinity." — `fixed`'s actual
/// window-exhaustion behavior is enforced independently by `fixed_repr`
/// (see `auto_range`'s doc), so a `first_symbol_value` near `i32::MAX`
/// must not make `auto_range` compute an inverted (`lower > upper`)
/// finite window that silently rejects every value via the range check
/// at step 2 before `fixed_repr` ever runs.
#[test]
fn resolve_fixed_near_i32_max_does_not_invert_range() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style edge { system: fixed 2147483647; symbols: "Z"; }"#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "edge", i32::MAX).as_deref(),
        Some("Z")
    );
}

// ── resolve_custom_counter: extends scope-limited ─────────────────────

#[test]
fn resolve_extends_is_unimplemented_and_falls_through() {
    let registry = CounterStyleRegistry::from_source(
        r#"@counter-style my-decimal-ish { system: extends decimal; }"#,
    );
    // fallback defaults to "decimal", not in this registry -> None.
    assert_eq!(resolve_custom_counter(&registry, "my-decimal-ish", 5), None);
}

#[test]
fn resolve_extends_falls_back_to_custom_style_when_declared() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style my-decimal-ish { system: extends decimal; fallback: real; }
            @counter-style real { system: cyclic; symbols: "R"; }
            "#,
    );
    assert_eq!(
        resolve_custom_counter(&registry, "my-decimal-ish", 5).as_deref(),
        Some("R")
    );
}

// ── resolve_custom_counter: fallback chains / unknown / cycles ────

#[test]
fn resolve_unknown_name_is_none() {
    let registry = CounterStyleRegistry::new();
    assert_eq!(resolve_custom_counter(&registry, "nonexistent", 1), None);
}

#[test]
fn resolve_chains_through_two_custom_fallbacks() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style a { system: cyclic; symbols: "*"; range: 1 1; fallback: b; }
            @counter-style b { system: cyclic; symbols: "%"; range: 1 1; fallback: c; }
            @counter-style c { system: cyclic; symbols: "@"; }
            "#,
    );
    // value 2 is out of `a`'s range -> falls to b -> also out of range -> falls to c.
    assert_eq!(
        resolve_custom_counter(&registry, "a", 2).as_deref(),
        Some("@")
    );
}

#[test]
fn resolve_fallback_cycle_terminates_with_none() {
    let registry = CounterStyleRegistry::from_source(
        r#"
            @counter-style a { system: cyclic; symbols: "*"; range: 1 1; fallback: b; }
            @counter-style b { system: cyclic; symbols: "%"; range: 1 1; fallback: a; }
            "#,
    );
    // Neither `a` nor `b` accepts value 5 (both ranged to exactly 1);
    // the fallback chain cycles a -> b -> a -> ... and must terminate.
    assert_eq!(resolve_custom_counter(&registry, "a", 5), None);
}

#[test]
fn resolve_long_finite_fallback_chain_terminates_at_default_fallback() {
    const CHAIN_LENGTH: usize = 10_000;
    let mut registry = CounterStyleRegistry::new();
    for index in 0..CHAIN_LENGTH {
        let mut rule = CounterStyleRule::new(SmolStr::new(format!("chain-{index}")));
        rule.system = CounterStyleSystem::Cyclic;
        rule.range = CounterRange::List(vec![RangeEntry {
            lower: RangeLimit::Finite(1),
            upper: RangeLimit::Finite(1),
        }]);
        rule.symbols = vec![CounterSymbol(SmolStr::new("*"))];
        if index + 1 < CHAIN_LENGTH {
            rule.fallback = SmolStr::new(format!("chain-{}", index + 1));
        }
        registry.insert(rule);
    }

    let terminal_name = format!("chain-{}", CHAIN_LENGTH - 1);
    assert_eq!(
        registry
            .get(&terminal_name)
            .map(|rule| rule.fallback.as_str()),
        Some("decimal")
    );

    // Every custom style rejects 2, so the finite chain must reach the
    // final rule's default `decimal` fallback and return this crate's
    // `None` boundary without exhausting the process stack.
    assert_eq!(resolve_custom_counter(&registry, "chain-0", 2), None);
}
