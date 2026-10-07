use super::*;

#[test]
fn common_report_pseudo_selectors_parse_without_forgiving_invalid_lists() {
    for source in [
        "a:link",
        "area:any-link",
        "::backdrop",
        "input::file-selector-button",
        "*, ::after, ::before, ::backdrop, ::file-selector-button",
    ] {
        let list = parse_selector_list(source).expect("valid selector");
        let mut serialized = String::new();
        list.to_css(&mut serialized).unwrap();
        assert_eq!(serialized, source);
    }
    for source in ["p, ::unknown-pseudo", "p, :unknown-state", "p, input["] {
        assert!(parse_selector_list(source).is_err());
    }
}

#[test]
fn atom_precomputed_hash_is_deterministic() {
    let a1 = Atom::from("btn");
    let a2 = Atom::from("btn");
    let b = Atom::from("card");
    assert_eq!(a1.precomputed_hash(), a2.precomputed_hash());
    assert_ne!(a1.precomputed_hash(), b.precomputed_hash());
}

#[test]
fn atom_tocss_roundtrip() {
    let a = Atom::from("btn");
    let mut out = String::new();
    a.to_css(&mut out).expect("serialize");
    assert_eq!(out, "btn");
}

#[test]
fn parse_dot_btn_hover_roundtrip() {
    let list = parse_selector_list(".btn:hover").expect("parse .btn:hover");
    let mut out = String::new();
    list.to_css(&mut out).expect("serialize selector list");
    assert_eq!(out, ".btn:hover");
}

#[test]
fn parse_multi_selector_list() {
    let list = parse_selector_list("a:hover, .btn:active").expect("parse list");
    let mut out = String::new();
    list.to_css(&mut out).expect("serialize");
    assert!(out.contains(":hover"));
    assert!(out.contains(":active"));
    assert!(out.contains(','));
}

#[test]
fn parse_logical_and_relational_pseudo_classes() {
    use selectors::parser::Component;

    for (source, expected) in [
        ("div:is(.featured, .selected)", "is"),
        ("div:where(.featured, .selected)", "where"),
        ("div:has(> .featured)", "has"),
    ] {
        let list =
            parse_selector_list(source).unwrap_or_else(|error| panic!("parse {source:?}: {error}"));
        let mut components = list.slice()[0].iter_raw_match_order();
        let found = components.any(|component| {
            matches!(
                (expected, component),
                ("is", Component::Is(_))
                    | ("where", Component::Where(_))
                    | ("has", Component::Has(_))
            )
        });
        assert!(found, "{source:?} must contain the {expected} component");
    }

    for source in [
        ":has(a)",
        ":has(#a)",
        ":has(.a)",
        ":has([a])",
        ":has([a=\"b\"])",
        ":has([a|=\"b\"])",
        ":has(:hover)",
        "*:has(.a)",
        ".a:has(.b)",
        ".a:has(> .b)",
        ".a:has(~ .b)",
        ".a:has(+ .b)",
        ".a:has(.b) .c",
        ".a .b:has(.c)",
        ".a .b:has(.c .d)",
        ".a .b:has(.c .d) .e",
        ".a:has(.b:is(.c .d))",
        ".a:is(.b:has(.c) .d)",
        ".a:not(:has(.b))",
        ".a:has(:not(.b))",
        ".a:has(.b):has(.c)",
        "*|*:has(*)",
        ":has(*|*)",
    ] {
        assert!(parse_selector_list(source).is_ok());
    }

    for source in [
        ":has",
        ".a:has",
        ".a:has b",
        ":has()",
        ":has(123)",
        ":has(.a, 123)",
        ".a:has(.b:has(.c))",
    ] {
        assert!(parse_selector_list(source).is_err());
    }

    for source in [
        ":has(:is(:has(*)))",
        ":has(:where(:has(*)))",
        ":has(:is(.a, 123))",
    ] {
        assert!(parse_selector_list(source).is_ok());
    }
}

#[test]
fn parse_wpt_logical_selector_forms_roundtrip() {
    for pseudo in ["is", "where"] {
        for (source, expected) in [
            (
                format!(":{pseudo}(ul,ol,.list) > [hidden]"),
                format!(":{pseudo}(ul, ol, .list) > [hidden]"),
            ),
            (
                format!(":{pseudo}(:hover,:focus)"),
                format!(":{pseudo}(:hover, :focus)"),
            ),
            (
                format!("a:{pseudo}(:not(:hover))"),
                format!("a:{pseudo}(:not(:hover))"),
            ),
            (format!(":{pseudo}(#a)"), format!(":{pseudo}(#a)")),
            (
                format!(".a.b ~ :{pseudo}(.c.d ~ .e.f)"),
                format!(".a.b ~ :{pseudo}(.c.d ~ .e.f)"),
            ),
            (
                format!(".a.b ~ .c.d:{pseudo}(span.e + .f, .g.h > .i.j .k)"),
                format!(".a.b ~ .c.d:{pseudo}(span.e + .f, .g.h > .i.j .k)"),
            ),
        ] {
            let list = parse_selector_list(&source)
                .unwrap_or_else(|error| panic!("parse {source:?}: {error}"));
            let mut serialized = String::new();
            list.to_css(&mut serialized)
                .expect("serialize selector list");
            assert_eq!(serialized, expected);
        }
    }
}

#[test]
fn parse_lang_single_range_roundtrip() {
    let list = parse_selector_list(":lang(ja)").expect("parse :lang(ja)");
    let mut out = String::new();
    list.to_css(&mut out).expect("serialize");
    assert_eq!(out, ":lang(ja)");
}

#[test]
fn parse_lang_multi_range_comma_separated() {
    let list = parse_selector_list(":lang(en, fr-CA, \"*-Hant\")").expect("parse :lang(...)");
    let selector = &list.slice()[0];
    let mut iter = selector.iter();
    let component = iter.next().expect("one component");
    match component {
        selectors::parser::Component::NonTSPseudoClass(PseudoClass::Lang(ranges)) => {
            assert_eq!(ranges, &["en", "fr-CA", "*-Hant"]);
        }
        other => panic!("expected PseudoClass::Lang, got {other:?}"),
    }
}

#[test]
fn parse_dir_ltr_and_rtl_roundtrip() {
    let list = parse_selector_list(":dir(ltr)").expect("parse :dir(ltr)");
    let mut out = String::new();
    list.to_css(&mut out).expect("serialize");
    assert_eq!(out, ":dir(ltr)");

    let list = parse_selector_list(":dir(rtl)").expect("parse :dir(rtl)");
    let mut out = String::new();
    list.to_css(&mut out).expect("serialize");
    assert_eq!(out, ":dir(rtl)");
}

#[test]
fn parse_dir_rejects_non_ltr_rtl_identifier() {
    // `Direction` doc's documented scope cut: unlike the real spec
    // (valid selector, matches nothing), this parser treats any
    // identifier other than ltr/rtl as a parse error.
    assert!(parse_selector_list(":dir(sideways)").is_err());
}

#[test]
fn parse_dir_rejects_non_ltr_rtl_identifier_drops_whole_comma_separated_list() {
    // `Direction` doc's "blast radius" note: `SelectorList::parse` is
    // non-forgiving, so one bad selector in a comma-separated list
    // fails the *whole* list — `p` here is otherwise perfectly valid on
    // its own.
    assert!(parse_selector_list("p, :dir(sideways)").is_err());
}

#[test]
fn parse_nth_child_of_selector_list_uses_nth_of_component() {
    use selectors::parser::{Component, NthType};

    for (source, expected_type) in [
        (
            "p:nth-child(2 of .featured, [data-kind=\"selected\"])",
            NthType::Child,
        ),
        (
            "p:nth-last-child(2 of .featured, [data-kind=\"selected\"])",
            NthType::LastChild,
        ),
    ] {
        let list = parse_selector_list(source).expect("parse selector-list argument");
        let selector = &list.slice()[0];
        let component = selector
            .iter()
            .find(|component| matches!(component, Component::NthOf(_)))
            .expect("selector must contain Component::NthOf");
        match component {
            Component::NthOf(data) => {
                assert!(data.nth_data().ty == expected_type);
                assert_eq!(data.nth_data().an_plus_b.0, 0);
                assert_eq!(data.nth_data().an_plus_b.1, 2);
                assert_eq!(data.selectors().len(), 2);
            }
            // cov:ignore: `find` above only yields `Component::NthOf`;
            // this arm is unreachable by construction.
            _ => unreachable!("find above guarantees Component::NthOf"),
        }

        let mut serialized = String::new();
        list.to_css(&mut serialized)
            .expect("serialize selector list");
        assert_eq!(serialized, source);
    }
}

#[test]
fn parse_nth_child_of_rejects_invalid_selector_list_forms() {
    assert!(parse_selector_list("p:nth-child(2 of .featured,)").is_err());
    assert!(parse_selector_list("p:nth-of-type(2 of .featured)").is_err());
}

// ---- tree-abiding pseudo-element selector parsing ----
//
// CSS Pseudo-Elements Module Level 4 §4.1
// <https://drafts.csswg.org/css-pseudo-4/#generated-content> and CSS Lists
// 3 §3.7, Selectors Level 4 (pseudo-element grammar). `cascade.rs`'s test module covers
// matching/cascade behavior once parsed; these tests cover parsing
// (accept/reject shape) only.

#[test]
fn parse_before_and_after_pseudo_element_roundtrip() {
    for (src, expected) in [
        (".foo::before", PseudoElem::Before),
        ("p::after", PseudoElem::After),
        ("li::marker", PseudoElem::Marker),
        ("p::first-line", PseudoElem::FirstLine),
    ] {
        let list = parse_selector_list(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
        let selector = &list.slice()[0];
        assert_eq!(selector.pseudo_element(), Some(&expected));
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize selector list");
        assert_eq!(out, src);
    }
}

#[test]
fn parse_bare_pseudo_element_implies_universal_originating_selector() {
    // `::before` alone parses like `*::before` — no explicit type/class
    // required on the originating-element side.
    let list = parse_selector_list("::before").expect("parse ::before");
    let selector = &list.slice()[0];
    assert_eq!(selector.pseudo_element(), Some(&PseudoElem::Before));
}

#[test]
fn parse_pseudo_element_rejects_unknown_name() {
    // `::marker` is the one additional generated-content pseudo-element
    // supported by the list-item pipeline. Other unsupported names remain
    // fail-closed (same posture `parse_non_ts_pseudo_class` already has for
    // unrecognized pseudo-classes).
    assert!(parse_selector_list("::marker").is_ok());
    assert!(parse_selector_list("::details-content").is_err());
    assert!(parse_selector_list("::bogus").is_err());
}

#[test]
fn parse_pseudo_element_must_be_selector_tail() {
    // A pseudo-element must be the rightmost component — nothing may
    // follow it in the same selector.
    assert!(parse_selector_list("a::before b").is_err());
}

#[test]
fn parse_pseudo_element_rejects_chaining_after_before_or_after() {
    // Fail-closed posture (see `PseudoElem`/`RaikiriSelectorParser` doc):
    // no pseudo-class, and no other pseudo-element, may follow
    // `::before`/`::after` in this crate — `PseudoElem` overrides none
    // of `is_before_or_after`/`accepts_state_pseudo_classes`/
    // `parses_as_element_backed`'s `false` defaults, so the `selectors`
    // crate's own parser state machine rejects all three forms below.
    assert!(parse_selector_list("::before::after").is_err());
    assert!(parse_selector_list("::before:hover").is_err());
    assert!(parse_selector_list(".foo::before.bar").is_err());
    // `PseudoElem::FirstLine` overrides none of the same defaults, so it
    // is rejected by the same state machine.
    assert!(parse_selector_list("::first-line:hover").is_err());
}

#[test]
fn parse_pseudo_element_rejected_inside_nth_child_of_selector_list() {
    // CSS Selectors Level 4 forbids a pseudo-element inside
    // `:nth-child(An+B of S)`'s `S` — the `selectors` crate itself
    // enforces this at parse time (not something this crate's own
    // `is_supported_selector` needs to reject after the fact, though it
    // does so too as defense-in-depth — see `ruletree.rs`
    // `is_supported_selector`'s doc).
    assert!(parse_selector_list("p:nth-child(2 of .x::before)").is_err());
}

#[test]
fn parse_legacy_single_colon_before_and_after_syntax() {
    // CSS Pseudo-Elements Module Level 4 §8 "Compatibility Syntax"
    // <https://drafts.csswg.org/css-pseudo-4/#css2-compat>, verbatim:
    // "For compatibility with existing style sheets written against CSS
    // Level 2 `[...]`, user agents must also accept the previous
    // one-colon notation (:before, :after, :first-letter, :first-line)
    // for the ::before, ::after, ::first-letter, and ::first-line
    // pseudo-elements." The `selectors` crate's own
    // `is_css2_pseudo_element` already special-cases exactly these four
    // names into the same pseudo-element parse path the double-colon
    // syntax uses — so this MUST-level requirement already works for
    // `:before`/`:after`/`:first-line` without any extra code in this
    // crate beyond `parse_pseudo_element` accepting the double-colon
    // name, but was previously untested. `:first-letter` still parses to
    // a rejected name (this crate's `parse_pseudo_element` has no
    // `PseudoElem::FirstLetter` arm), so it is not included below.
    //
    // Deliberately does NOT assert a source round-trip via `to_css`
    // (unlike `parse_before_and_after_pseudo_element_roundtrip` above):
    // `:before`/`:after` and `::before`/`::after` denote the same
    // pseudo-element (Selectors Level 4 §3.10 "Syntax"
    // <https://www.w3.org/TR/selectors-4/#pseudo-element-syntax>, same
    // one-colon compatibility rule), and this crate's `PseudoElem` has
    // no field to remember which spelling the author used, so
    // `ToCss for PseudoElem` always serializes the double-colon form
    // regardless of which one was parsed. A naive `assert_eq!(out,
    // src)` against `:before` input would therefore fail spuriously;
    // the correct assertion is on the parsed `PseudoElem` value only.
    for (src, expected) in [
        (":before", PseudoElem::Before),
        (":after", PseudoElem::After),
        (":first-line", PseudoElem::FirstLine),
    ] {
        let list = parse_selector_list(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
        let selector = &list.slice()[0];
        assert_eq!(selector.pseudo_element(), Some(&expected));
    }
}

mod selector_depth_tests;
