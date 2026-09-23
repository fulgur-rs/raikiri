use super::*;
use crate::cascade::cascade;
use crate::cascade::test_support::*;
use crate::computed::ComputedValues;
use crate::property::BorderStyle;
use crate::ruletree::{Origin, RuleTree, build_rule_tree};
use crate::test_dom::TestDoc;

#[test]
fn type_selector_applies_color() {
    let cv = cascade_doc("p { color: red }", "p", None);
    assert_eq!(cv.color, RED);
}

#[test]
fn class_selector_applies_declaration() {
    // Acceptance: `.chapter-title { font-weight:
    // bold }` applied to `<p class="chapter-title">` — `font-weight: bold`
    // computes to 700.0 (property.rs `parse_font_weight`).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".chapter-title { font-weight: bold }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "chapter-title");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_weight, 700.0);
}

#[test]
fn class_selector_does_not_match_element_without_the_class() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".chapter-title { font-weight: bold }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "intro"); // different token

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].font_weight, 400.0,
        "initial, rule must not apply"
    );
}

#[test]
fn class_selector_matches_one_token_among_several() {
    // `class="a b c"` — HTML-spec ASCII whitespace split
    // (`StyleElement::has_class` doc), `.b` must match the middle token.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".b { font-weight: bold }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "a b c");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_weight, 700.0);
}

#[test]
fn class_selector_is_case_sensitive() {
    // CSS Selectors L4 class-html: HTML class matching in standards mode
    // is case-sensitive (`StyleElement::has_class` default impl does an
    // exact token compare, no ASCII-case-folding).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".Foo { font-weight: bold }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "foo"); // different case

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].font_weight, 400.0,
        "initial, case must not fold"
    );
}

#[test]
fn class_selector_case_sensitivity_across_quirks_modes() {
    struct Case {
        mode: StyleQuirksMode,
        element_class: &'static str,
        expect_match: bool,
    }
    let cases = [
        // NoQuirks (standards mode): always case-sensitive.
        Case {
            mode: StyleQuirksMode::NoQuirks,
            element_class: "foo",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::NoQuirks,
            element_class: "FOO",
            expect_match: false,
        },
        // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
        // from "quirks mode" — must NOT fold, same as NoQuirks.
        Case {
            mode: StyleQuirksMode::LimitedQuirks,
            element_class: "foo",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::LimitedQuirks,
            element_class: "FOO",
            expect_match: false,
        },
        // Quirks (full quirks mode): ASCII case-insensitive fold.
        Case {
            mode: StyleQuirksMode::Quirks,
            element_class: "foo",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::Quirks,
            element_class: "FOO",
            expect_match: true,
        },
    ];

    for case in cases {
        let mut doc = TestDoc::new();
        doc.quirks_mode = case.mode;
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".foo { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", case.element_class);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let matched = r.computed[p].font_weight == 700.0;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            matched, case.expect_match,
            "mode={:?} element_class={:?}: expected match={}",
            case.mode, case.element_class, case.expect_match
        );
    }
}

#[test]
fn has_class_ascii_case_insensitive_empty_query_is_false() {
    let mut doc = TestDoc::new();
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "foo");
    let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
    let elem = node.as_element().unwrap();
    assert!(!elem.has_class_ascii_case_insensitive(""));
}

#[test]
fn id_selector_applies_declaration() {
    // Acceptance: `#header { ... }` applied to
    // `<div id="header">`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "#header { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "id", "header");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, RED);
}

#[test]
fn id_selector_does_not_match_different_id() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "#header { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "id", "footer");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

#[test]
fn id_selector_case_sensitivity_across_quirks_modes() {
    struct Case {
        mode: StyleQuirksMode,
        element_id: &'static str,
        expect_match: bool,
    }
    let cases = [
        // NoQuirks (standards mode): always case-sensitive.
        Case {
            mode: StyleQuirksMode::NoQuirks,
            element_id: "header",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::NoQuirks,
            element_id: "HEADER",
            expect_match: false,
        },
        // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
        // from "quirks mode" — must NOT fold, same as NoQuirks.
        Case {
            mode: StyleQuirksMode::LimitedQuirks,
            element_id: "header",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::LimitedQuirks,
            element_id: "HEADER",
            expect_match: false,
        },
        // Quirks (full quirks mode): ASCII case-insensitive fold.
        Case {
            mode: StyleQuirksMode::Quirks,
            element_id: "header",
            expect_match: true,
        },
        Case {
            mode: StyleQuirksMode::Quirks,
            element_id: "HEADER",
            expect_match: true,
        },
    ];

    for case in cases {
        let mut doc = TestDoc::new();
        doc.quirks_mode = case.mode;
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#header { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "id", case.element_id);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let matched = r.computed[div].color == RED;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            matched, case.expect_match,
            "mode={:?} element_id={:?}: expected match={}",
            case.mode, case.element_id, case.expect_match
        );
    }
}

#[test]
fn attribute_exists_selector_applies_declaration() {
    // Acceptance: `[data-foo]` matches any
    // element carrying that attribute, regardless of its value.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "anything");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, RED);
}

#[test]
fn attribute_exists_selector_does_not_match_when_attr_absent() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo] { color: red }");
    let div = doc.push_element(0, "div", None); // no data-foo at all

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

#[test]
fn attribute_exists_selector_does_not_match_empty_value_attr() {
    // Pins `TestDoc`'s own `StyleElement::attr` override
    // (`test_dom.rs`), which deliberately keeps the older, stricter
    // "empty value is normalised to `None`" behavior as a
    // simplification local to this mock. Against `TestDoc`,
    // `data-foo=""` reads back as attribute-absent, so `[data-foo]`
    // does not match here.
    //
    // **This is `TestDoc`-only, not the real DOM's behavior.**
    // `raikiri-dom::dom_impl::ElementRef::attr` (the real DOM impl)
    // tracks attribute presence independent of value, so
    // `data-foo=""` does match `[data-foo]` there — this test's name
    // and outcome describe the mock's narrower contract, not a general
    // engine-level accepted-baseline divergence from CSS Selectors L4.
    // See `StyleElement::attr`'s trait doc (style_dom.rs) for the full
    // contract and this divergence's rationale.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

#[test]
fn attribute_exact_match_selector_does_not_match_empty_value_attr() {
    // Same root cause as
    // `attribute_exists_selector_does_not_match_empty_value_attr` above,
    // pinned separately because it goes through a different
    // `compound_matches` arm (`Component::AttributeInNoNamespace`, not
    // `..Exists`): `TestDoc`'s own `StyleElement::attr` override
    // (`test_dom.rs`) collapses `foo=""` into `None` before the
    // with-value arm's `match elem.attr(...) { Some(..) => ..,
    // None => false }` ever runs, so it takes the `None => false`
    // branch regardless of the selector's own value operand.
    //
    // **This is `TestDoc`-only, not the real DOM's behavior.** Per CSS
    // Selectors L4 (<https://www.w3.org/TR/selectors-4/#attribute-selectors>),
    // `[data-foo=""]` should match an element whose `data-foo` value is
    // exactly the empty string, and `raikiri-dom::dom_impl::ElementRef::attr`
    // (the real DOM impl) does support that — it tracks presence
    // independent of value. Only `TestDoc`'s deliberately-simplified
    // mock still collapses `foo=""` to absent; this test's name and
    // outcome describe that mock, not a general engine-level
    // accepted-baseline divergence. See `StyleElement::attr`'s trait
    // doc (style_dom.rs) for the full contract and this divergence's
    // rationale.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo=\"\"] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

#[test]
fn attribute_exists_selector_mixed_case_matches_html_element_via_lowercased_key() {
    // HTML LS "case-sensitivity of selectors"
    // (https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors):
    // attribute names on HTML elements in HTML documents are
    // ASCII-lowercased — html5ever already lower-cases them at parse
    // time (`raikiri-html::sink::wire_side_tables` stores whatever case
    // html5ever produced, unmodified). So a selector written with mixed
    // case, `[Data-Foo]`, must still match an HTML (default-namespace)
    // element whose stored attribute name is already-lowercased
    // `data-foo`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[Data-Foo] { color: red }");
    let div = doc.push_element(0, "div", None); // default namespace = HTML
    doc.set_attr(div, "data-foo", "anything");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, RED);
}

#[test]
fn attribute_exists_selector_mixed_case_uses_original_case_for_foreign_namespace_element() {
    // Foreign-namespace (SVG/MathML) elements are NOT covered by HTML
    // LS's "attributes on HTML elements in HTML documents" lowercasing
    // scope — html5ever's "adjust foreign attributes" step can restore
    // specific attributes to their original mixed case (e.g. `viewBox`),
    // and `wire_side_tables` stores whatever case html5ever produced,
    // unmodified. A selector written `[Data-Foo]` against such an
    // element must use the *original-case* lookup key, not the
    // lowercased one.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[Data-Foo] { color: red }");
    let svg_el = doc.push_element(0, "rect", None);
    doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
    doc.set_attr(svg_el, "Data-Foo", "anything"); // original mixed case

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[svg_el].color, RED);
}

#[test]
fn attribute_exists_selector_mixed_case_does_not_fall_back_to_lowercase_for_foreign_namespace_element()
 {
    // Same shape as the sibling test above, but the foreign-namespace
    // element carries only the *lowercased* attribute name — proving
    // the lookup is genuinely gated on the original-case key for
    // foreign elements, not silently trying both keys.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[Data-Foo] { color: red }");
    let svg_el = doc.push_element(0, "rect", None);
    doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
    doc.set_attr(svg_el, "data-foo", "anything"); // lowercased — wrong key for this element

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[svg_el].color, ComputedValues::initial().color);
}

#[test]
fn attribute_value_exact_match_selector_applies_declaration() {
    // Acceptance: `[data-foo="bar"]` exact-match
    // variant.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "bar");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, RED);
}

#[test]
fn attribute_value_exact_match_selector_does_not_match_different_value() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "baz");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

fn assert_attribute_operator_match(selector: &str, value: &str, expected_match: bool) {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, selector);
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", value);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color == RED, expected_match);
}

#[test]
fn attribute_prefix_match_selector_applies_declaration() {
    assert_attribute_operator_match(r#"[data-foo^="pre"] { color: red }"#, "prefix", true);
    assert_attribute_operator_match(r#"[data-foo^="pre"] { color: red }"#, "xprefix", false);
}

#[test]
fn attribute_suffix_match_selector_applies_declaration() {
    assert_attribute_operator_match(r#"[data-foo$="fix"] { color: red }"#, "prefix", true);
    assert_attribute_operator_match(r#"[data-foo$="fix"] { color: red }"#, "fixed", false);
}

#[test]
fn attribute_substring_match_selector_applies_declaration() {
    assert_attribute_operator_match(r#"[data-foo*="ref"] { color: red }"#, "prefix", true);
    assert_attribute_operator_match(r#"[data-foo*="ref"] { color: red }"#, "pfix", false);
}

#[test]
fn attribute_whitespace_token_match_selector_applies_declaration() {
    assert_attribute_operator_match(
        r#"[data-foo~="beta"] { color: red }"#,
        "alpha beta gamma",
        true,
    );
    assert_attribute_operator_match(
        r#"[data-foo~="beta"] { color: red }"#,
        "alphabetagamma",
        false,
    );
}

#[test]
fn attribute_hyphen_prefix_match_selector_applies_declaration() {
    assert_attribute_operator_match(r#"[data-foo|="en"] { color: red }"#, "en-US", true);
    assert_attribute_operator_match(r#"[data-foo|="en"] { color: red }"#, "english", false);
}

#[test]
fn attribute_value_exact_match_is_case_sensitive_for_data_attr() {
    // CSS Selectors L4 attribute-selectors: default case-sensitivity
    // (no `i`/`s` flag) depends on the document language; `data-*` is not
    // in HTML's ASCII-case-insensitive attribute list, so it resolves to
    // `ParsedCaseSensitivity::CaseSensitive` at parse time (selectors
    // crate `AttributeFlags::to_case_sensitivity`) — no
    // `resolve_case_sensitivity` branching is even reached for this case.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "BAR"); // different case

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, ComputedValues::initial().color);
}

#[test]
fn attribute_value_case_insensitive_flag_i_matches_regardless_of_case() {
    // `[foo="bar" i]` — explicit `i` flag forces ASCII-case-insensitive
    // matching regardless of the attribute's document-language default
    // (CSS Selectors L4 attribute-selectors, `AttributeFlags::AsciiCaseInsensitive`).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[data-foo=\"bar\" i] { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "data-foo", "BAR");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[div].color, RED);
}

#[test]
fn attribute_selector_style_local_name_matches_element_with_inline_style() {
    // `StyleElement::attr`'s doc contract requires overrides to keep
    // handling `local == "style"` by delegating to
    // `inline_style_source()`; `TestElementRef::attr`
    // does this, so `[style]` — an ordinary
    // existence attribute selector whose local name happens to be
    // `style` — must match any element carrying an inline `style="…"`.
    // `font-weight` (not touched by the inline `color: blue`) is the
    // observable, since inline style otherwise always outranks any
    // stylesheet rule (`INLINE_SPECIFICITY`) regardless of whether
    // `[style]` itself matched.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[style] { font-weight: bold }");
    let p = doc.push_element(0, "p", Some("color: blue"));

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_weight, 700.0);
}

#[test]
fn attribute_selector_style_does_not_match_element_without_inline_style() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[style] { font-weight: bold }");
    let p = doc.push_element(0, "p", None); // no inline style

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_weight, 400.0);
}

#[test]
fn compound_type_and_class_selector_requires_both() {
    // `p.chapter-title` — compound selector, AND semantics: both the type
    // and class component must match the same element.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p.chapter-title { font-weight: bold }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "chapter-title");
    let div = doc.push_element(0, "div", None); // wrong tag, same class
    doc.set_attr(div, "class", "chapter-title");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].font_weight, 700.0,
        "p.chapter-title must match <p class=chapter-title>"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[div].font_weight, 400.0,
        "div.chapter-title selector must not match <div class=chapter-title> (wrong tag)"
    );
}

#[test]
fn descendant_combinator_applies_declaration_to_direct_child() {
    // Acceptance: `.chapter h2` applied to
    // `<div class="chapter"><h2>...</h2></div>`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".chapter h2 { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "class", "chapter");
    let h2 = doc.push_element(div, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[h2].color, RED);
}

#[test]
fn descendant_combinator_applies_to_arbitrary_depth_descendant() {
    // "arbitrary descendant" (spec verbatim above) — must match even
    // through an intermediate <section> that itself matches neither
    // side of the selector.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".chapter h2 { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.set_attr(div, "class", "chapter");
    let section = doc.push_element(div, "section", None);
    let h2 = doc.push_element(section, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[h2].color, RED,
        "descendant combinator must match through an intermediate non-matching ancestor"
    );
}

#[test]
fn descendant_combinator_does_not_match_outside_the_subtree() {
    // Negative case: an <h2> that is not a descendant of any
    // `.chapter` must not pick up the declaration.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".chapter h2 { color: red }");
    let div = doc.push_element(0, "div", None); // no class="chapter"
    let h2 = doc.push_element(div, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[h2].color, ComputedValues::initial().color);
}

#[test]
fn child_combinator_applies_declaration_to_direct_child_only() {
    // Acceptance: `ol > li` applies to a
    // direct `<li>` child of `<ol>`, but NOT to a grandchild `<li>`
    // reached through an intervening `<ul>` (`<ol><li><ul><li>...`).
    //
    // Uses `background-color`, not `color`: `color` is an inherited
    // property (CSS Cascading L4 §5.2 inheritance) — using it here would
    // let the *direct* `<li>` match's computed value leak onto the
    // grandchild via ordinary inheritance (through the intervening
    // `<ul>`), producing a false pass regardless of whether the child
    // combinator itself correctly rejects the grandchild.
    // `background-color` is not inherited (CSS Backgrounds 3 §2.2), so a
    // red grandchild here can only mean the combinator matched it
    // directly.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "ol > li { background-color: red }");
    let ol = doc.push_element(0, "ol", None);
    let direct_li = doc.push_element(ol, "li", None);
    let ul = doc.push_element(direct_li, "ul", None);
    let grandchild_li = doc.push_element(ul, "li", None);
    // Root-level `<li>` with no `<ol>` ancestor at all (its
    // `ancestor_path` is empty, since the document root itself is not
    // an `Element`) — exercises `match_combinator_chain`'s
    // `Combinator::Child => ancestors.split_last() => None => false`
    // arm, distinct from the "wrong parent" case covered by
    // `grandchild_li` above (there `ancestors.split_last()` succeeds
    // but the resolved parent fails `compound_matches`).
    let orphan_li = doc.push_element(0, "li", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[direct_li].background_color, RED,
        "ol > li must match the direct <li> child of <ol>"
    );
    assert_eq!(
        r.computed[grandchild_li].background_color,
        ComputedValues::initial().background_color,
        "ol > li must NOT match a grandchild <li> reached through an intervening <ul>"
    );
    assert_eq!(
        r.computed[orphan_li].background_color,
        ComputedValues::initial().background_color,
        "ol > li must NOT match an <li> with no ancestor at all"
    );
}

#[test]
fn next_sibling_combinator_does_not_match_parent_child_relationship() {
    // `div + p` requires `div`/`p` to be
    // *siblings* (CSS Selectors L4 adjacent-sibling-combinators,
    // "share the same parent"). Here `p` is instead a *child* of
    // `div` — the ancestor relationship must NOT satisfy the sibling
    // combinator, even though `div` is literally `ancestors.last()`.
    // Directly exercises `match_combinator_chain`'s `NextSibling` arm
    // (this test predates sibling-combinator support, when it was named
    // `match_combinator_chain_rejects_unsupported_combinator_via_safety_net`,
    // and `+` fell through the `_ => false` safety net for a different
    // reason — repurposed now that `+` is supported).
    let list = crate::parse_selector_list("div + p").expect("selector parses");
    let mut doc = TestDoc::new();
    let div = doc.push_element(0, "div", None);
    let p = doc.push_element(div, "p", None);
    let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
    let elem = node.as_element().unwrap();
    assert_eq!(
        match_complex_selector_list(
            &list,
            &doc,
            &elem,
            StyleNodeId::new(p as u64),
            &[StyleNodeId::new(div as u64)],
            StyleQuirksMode::NoQuirks,
        ),
        None,
        "div + p must not match a p that is div's child, not its sibling"
    );
}

#[test]
fn descendant_and_child_combinator_are_distinguished_on_the_same_grandchild() {
    // Same grandchild `<li>` as above, matched instead by a descendant
    // (space) combinator on `ol` — must match, unlike the child (`>`)
    // combinator case, directly exercising "descendant と child の区別"
    // called out in the acceptance criteria. `background-color` again
    // (see the sibling test above) so a match is provably direct, not
    // inherited from the also-matching direct `<li>`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "ol li { background-color: red }");
    let ol = doc.push_element(0, "ol", None);
    let direct_li = doc.push_element(ol, "li", None);
    let ul = doc.push_element(direct_li, "ul", None);
    let grandchild_li = doc.push_element(ul, "li", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[direct_li].background_color, RED);
    assert_eq!(
        r.computed[grandchild_li].background_color, RED,
        "ol li (descendant combinator) must match the grandchild <li> too"
    );
}

#[test]
fn chained_descendant_and_child_combinator_matches_spec_example() {
    // CSS Selectors L4 child-combinators
    // (<https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim
    // example — see `match_combinator_chain`'s "Spec provenance note"
    // for how this text was confirmed): `div ol>li p` "represents a p
    // element that is a descendant of an li element; the li element
    // must be the child of an ol element; the ol element must be a
    // descendant of a div".
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "div ol>li p { color: red }");
    let div = doc.push_element(0, "div", None);
    // `ol` is a descendant of `div`, not a direct child — exercises the
    // "arbitrary descendant" half of the chained selector too.
    let wrapper = doc.push_element(div, "section", None);
    let ol = doc.push_element(wrapper, "ol", None);
    let li = doc.push_element(ol, "li", None);
    // `p` is a descendant of `li`, not a direct child.
    let span = doc.push_element(li, "span", None);
    let p = doc.push_element(span, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].color, RED);
}

#[test]
fn chained_descendant_and_child_combinator_rejects_wrong_child_parent() {
    // Same shape as the spec example above, but `li`'s parent is `ul`
    // instead of `ol` — the `>` (child) constraint must reject this
    // even though every other part of the chain still lines up.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "div ol>li p { color: red }");
    let div = doc.push_element(0, "div", None);
    let ul = doc.push_element(div, "ul", None); // not `ol`
    let li = doc.push_element(ul, "li", None);
    let p = doc.push_element(li, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].color, ComputedValues::initial().color);
}

#[test]
fn adjacent_sibling_combinator_applies_only_to_immediately_following_sibling() {
    // Acceptance: `h2 + p` applies to the `<p>`
    // immediately following an `<h2>`, but NOT to a second/third `<p>`
    // further along — CSS Selectors L4 next-sibling combinator
    // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
    // §14.3, "match_combinator_chain" doc's verbatim quote). Elements
    // are pushed at the **document root** (parent id `0`, no wrapping
    // `<div>`) deliberately — `ancestor_path` only ever contains
    // Element-kind ids, so a root-level sibling pair exercises
    // `match_combinator_chain`'s `ancestors.last() == None →
    // dom.root_id()` fallback; a wrapping element would hide a bug in
    // that fallback entirely.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 + p { background-color: red }");
    let _h2 = doc.push_element(0, "h2", None);
    let p1 = doc.push_element(0, "p", None); // immediately follows h2
    let p2 = doc.push_element(0, "p", None); // follows p1, not h2
    let p3 = doc.push_element(0, "p", None); // follows p2, not h2

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p1].background_color, RED,
        "h2 + p must match the p immediately following h2"
    );
    assert_eq!(
        r.computed[p2].background_color,
        ComputedValues::initial().background_color,
        "h2 + p must NOT match the second p (not immediately after h2)"
    );
    assert_eq!(
        r.computed[p3].background_color,
        ComputedValues::initial().background_color,
        "h2 + p must NOT match the third p (not immediately after h2)"
    );
}

#[test]
fn general_sibling_combinator_applies_to_every_following_sibling() {
    // Acceptance: `h2 ~ p` applies to every
    // `<p>` that follows an `<h2>`, not just the immediate one — CSS
    // Selectors L4 general-sibling combinator
    // (<https://www.w3.org/TR/selectors-4/#general-sibling-combinators>
    // §14.4). Same root-level layout as the adjacent-sibling test above
    // (same rationale — exercises the `ancestors.last() == None`
    // fallback).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 ~ p { background-color: red }");
    let _h2 = doc.push_element(0, "h2", None);
    let p1 = doc.push_element(0, "p", None);
    let p2 = doc.push_element(0, "p", None);
    let p3 = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p1].background_color, RED,
        "h2 ~ p must match the 1st following p"
    );
    assert_eq!(
        r.computed[p2].background_color, RED,
        "h2 ~ p must match the 2nd following p"
    );
    assert_eq!(
        r.computed[p3].background_color, RED,
        "h2 ~ p must match the 3rd following p"
    );
}

#[test]
fn adjacent_and_general_sibling_combinator_are_distinguished_on_the_same_dom() {
    // Acceptance, literal form: both `h2 + p`
    // and `h2 ~ p` active on the same `<h2><p><p><p>` DOM, using two
    // independent non-inherited properties (`background-color`, CSS
    // Backgrounds 3 §2.2; `box-sizing`, CSS Box Sizing dfn "Inherited:
    // no") so each combinator's reach is independently observable on
    // the same elements.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "h2 + p { background-color: red } h2 ~ p { box-sizing: border-box }",
    );
    let _h2 = doc.push_element(0, "h2", None);
    let p1 = doc.push_element(0, "p", None);
    let p2 = doc.push_element(0, "p", None);
    let p3 = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // `+` (background-color): only p1.
    assert_eq!(r.computed[p1].background_color, RED);
    assert_eq!(
        r.computed[p2].background_color,
        ComputedValues::initial().background_color
    );
    assert_eq!(
        r.computed[p3].background_color,
        ComputedValues::initial().background_color
    );
    // `~` (box-sizing): all three.
    assert_eq!(
        r.computed[p1].box_sizing,
        crate::property::BoxSizing::BorderBox
    );
    assert_eq!(
        r.computed[p2].box_sizing,
        crate::property::BoxSizing::BorderBox
    );
    assert_eq!(
        r.computed[p3].box_sizing,
        crate::property::BoxSizing::BorderBox
    );
}

#[test]
fn sibling_combinator_ignores_non_element_nodes_between_siblings() {
    // CSS Selectors L4 next-sibling combinator, verbatim: "Non-element
    // nodes (e.g. text between elements) are ignored when considering
    // the adjacency of elements"
    // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>).
    // A text node is pushed to the *root* (same parent as `h2`/`p`)
    // between them — `TestDoc::push_text(parent, ..)` appends to
    // `parent`'s children list in call order, so pushing it between the
    // `h2` and `p` pushes below makes it a genuine root-level sibling
    // positioned between them, not a descendant of either. `h2 + p`
    // must still match `p` despite this — i.e.
    // `match_combinator_chain`'s `NextSibling` arm
    // (`immediate_preceding_sibling`) must skip the non-Element
    // `child_ids` entry rather than treating the text node as "the"
    // immediately preceding sibling (which would make `p` NOT
    // immediately follow `h2` from an all-nodes perspective).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 + p { background-color: red }");
    let _h2 = doc.push_element(0, "h2", None);
    doc.push_text(0, "root-level text node, sibling of h2 and p, between them");
    let p = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].background_color, RED,
        "h2 + p must match p despite the text node inside h2 (not a sibling at all) \
             and must not be confused by non-element nodes in general"
    );
}

#[test]
fn general_sibling_combinator_skips_a_leading_non_element_candidate() {
    // Same CSS Selectors L4 "non-element nodes... are ignored" rule as
    // `sibling_combinator_ignores_non_element_nodes_between_siblings`
    // above, but for `~` (general sibling) rather than `+` (adjacent
    // sibling) — these exercise different code paths:
    // `PendingCandidates::LaterSibling`'s new resumable scan loop vs
    // `PendingCandidates::NextSibling`'s single-shot
    // `immediate_preceding_sibling`, for the same kind()-based skip
    // (`TestDoc` always reports `is_in_document() == true`; it has no
    // inert/`<template>`-descendant node concept, so this doesn't
    // separately discriminate `is_in_document_element`'s
    // `is_in_document()` conjunct from its `kind() == Element` one). A
    // root-level text node is pushed *before* `h2` (not between `h2`
    // and `p` — `~`'s candidate scan walks the parent's children
    // forward from the start, so the non-element candidate must be
    // reached *before* the eventual matching candidate to exercise the
    // "skip, keep scanning" branch rather than the "reached
    // `current_id`, stop" one).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 ~ p { background-color: red }");
    doc.push_text(0, "root-level text node, sibling of h2 and p, before both");
    let _h2 = doc.push_element(0, "h2", None);
    let p = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].background_color, RED,
        "h2 ~ p must match p despite a non-element sibling preceding h2 in \
             the same child list"
    );
}

#[test]
fn later_sibling_choice_point_resumes_live_iterator_across_backtrack() {
    // Discriminator for `PendingCandidates::LaterSibling`'s *live,
    // resumable* `D::ChildIter` - not covered by any existing test:
    // doubling `~`
    // is required to exercise one `LaterSibling` choice point being
    // popped-into-and-resumed by a stack.pop() backtrack from a
    // *different, nested* `LaterSibling` choice point (a single `~`
    // combined with `>`/` ` can't discriminate this, since sibling
    // candidates all share the same ancestors, so a Child/Descendant
    // check after a sibling jump can't distinguish "resume mid-scan"
    // from "rescan from the top").
    //
    // Children of the shared parent, document order: b1(.b), a(.a),
    // b2(.b), t(.t). Selector `.a ~ .b ~ .t` requires some `.b` that
    // precedes `t`, itself preceded by some `.a`.
    //
    // b1 is the *first* `.b` candidate tried for `t`'s `~` frame - but
    // b1's own nested `~` scan for `.a` immediately hits its `stop_at`
    // (b1 is the very first child), so that inner frame is exhausted
    // with zero candidates and pops immediately. Correctness requires
    // the *outer* frame (scanning for `.b` before `t`) to resume its
    // live iterator at `a` next (not restart at b1 - that would loop
    // forever / re-fail identically - and not skip past `a` straight to
    // `b2`, which would only find the wrong, but still spec-correct-
    // looking, match via a different `.b` and hide a real skip bug).
    // The only `.b` with a valid `.a` before it is b2 (via `a`), so a
    // match requires both correct resume *and* correct non-skip.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".a ~ .b ~ .t { background-color: red }");
    let b1 = doc.push_element(0, "div", None);
    doc.set_attr(b1, "class", "b");
    let a = doc.push_element(0, "div", None);
    doc.set_attr(a, "class", "a");
    let b2 = doc.push_element(0, "div", None);
    doc.set_attr(b2, "class", "b");
    let t = doc.push_element(0, "div", None);
    doc.set_attr(t, "class", "t");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[t].background_color, RED,
        ".a ~ .b ~ .t must match t via b2 (preceded by a), after b1's own \
             nested .a search (immediately empty) is backtracked past - this \
             requires the LaterSibling choice point's live child iterator to \
             resume correctly rather than restart or skip"
    );
}

#[test]
fn sibling_combinator_does_not_match_preceding_element() {
    // Order matters: CSS Selectors L4 requires the left compound's
    // element to *precede* the right compound's element. A `<p>` placed
    // BEFORE the `<h2>` must not satisfy `h2 + p` / `h2 ~ p` when
    // matching is attempted from that earlier `<p>`'s perspective.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 + p { background-color: red } h2 ~ p { color: red }");
    let p_before = doc.push_element(0, "p", None);
    let _h2 = doc.push_element(0, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p_before].background_color,
        ComputedValues::initial().background_color
    );
    assert_eq!(r.computed[p_before].color, ComputedValues::initial().color);
}

#[test]
fn sibling_combinator_applies_under_a_non_root_parent() {
    // Same as the acceptance tests above but wrapped in a `<div>`
    // parent, so `ancestors` is non-empty when the sibling combinator
    // arms run — exercises the `ancestors.last() == Some(parent)` branch
    // (as opposed to the root-level tests' `None → root_id()` fallback
    // branch) of `match_combinator_chain`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "h2 + p { background-color: red }");
    let wrap = doc.push_element(0, "div", None);
    let _h2 = doc.push_element(wrap, "h2", None);
    let p = doc.push_element(wrap, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].background_color, RED);
}

#[test]
fn sibling_combinator_composes_with_child_combinator_further_left() {
    // Mixed chain, sibling-then-ancestor direction: `.x > .y ~ .z`.
    // `.z` and `.y` are siblings (share parent `.x`); `.y` must in turn
    // be a direct child of `.x`. Exercises the "sibling jump keeps
    // `ancestors` unchanged, so a further-left Child/Descendant combinator
    // composes for free" path documented on
    // `is_supported_selector_list`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".x > .y ~ .z { background-color: red }");
    let x = doc.push_element(0, "div", None);
    doc.set_attr(x, "class", "x");
    let y = doc.push_element(x, "div", None);
    doc.set_attr(y, "class", "y");
    let z = doc.push_element(x, "div", None); // sibling of y, child of x
    doc.set_attr(z, "class", "z");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[z].background_color, RED);
}

#[test]
fn child_combinator_composes_with_sibling_combinator_further_left() {
    // Mixed chain, ancestor-then-sibling direction: `.x ~ .y > .z`.
    // `.z`'s parent is `.y`; `.y` must in turn have a preceding sibling
    // `.x` (sharing `.y`'s own parent). Exercises the opposite
    // composition from the test above — after the `Child` jump to `.y`,
    // the `ancestors` slice `match_from_element` carries onward is
    // already `.y`'s own ancestor chain, so `.last()` correctly resolves
    // to `.y`'s parent for the `LaterSibling` step (see
    // `match_combinator_chain`'s "親の解決" doc note).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".x ~ .y > .z { background-color: red }");
    let container = doc.push_element(0, "div", None);
    let x = doc.push_element(container, "div", None);
    doc.set_attr(x, "class", "x");
    let y = doc.push_element(container, "div", None); // sibling of x
    doc.set_attr(y, "class", "y");
    let z = doc.push_element(y, "div", None); // child of y
    doc.set_attr(z, "class", "z");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[z].background_color, RED);
}

#[test]
fn descendant_retry_past_a_failed_child_combinator_candidate_is_required() {
    // A descendant retry is required when a later child combinator can
    // inspect a different immediate parent; see `match_combinator_chain`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".x > .y .target { background-color: red }");
    let g = doc.push_element(0, "div", None);
    doc.set_attr(g, "class", "x");
    let f = doc.push_element(g, "div", None);
    doc.set_attr(f, "class", "y");
    let m = doc.push_element(f, "div", None); // no class — the dead-end Child target for C
    let c = doc.push_element(m, "div", None);
    doc.set_attr(c, "class", "y"); // nearest .y candidate, but a dead end via Child
    let target = doc.push_element(c, "div", None);
    doc.set_attr(target, "class", "target");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[target].background_color, RED,
        ".x > .y .target must match via the farther .y candidate (F) after the \
             nearer one (C) fails Child's fixed-parent check — the retry is required"
    );
}

fn uniform_div_chain_and_selector(
    depth: usize,
    compounds: usize,
) -> (
    TestDoc,
    StyleNodeId,
    Vec<StyleNodeId>,
    SelectorList<RaikiriSelectorImpl>,
) {
    let mut doc = TestDoc::new();
    let mut parent = 0usize;
    let mut ids = Vec::with_capacity(depth);
    for _ in 0..depth {
        parent = doc.push_element(parent, "div", None);
        ids.push(parent);
    }
    let target_id = StyleNodeId::new(*ids.last().expect("depth > 0") as u64);
    let ancestors: Vec<StyleNodeId> = ids[..ids.len() - 1]
        .iter()
        .map(|&id| StyleNodeId::new(id as u64))
        .collect();
    let selector = vec!["div"; compounds].join(" ");
    let list = crate::parse_selector_list(&selector).expect("selector parses");
    (doc, target_id, ancestors, list)
}

#[test]
fn descendant_combinator_deep_unsatisfiable_chain_does_not_explode() {
    let depth = 200;
    let (doc, target_id, ancestors, list) = uniform_div_chain_and_selector(depth, depth + 1);
    let node = doc.node(target_id).unwrap();
    let elem = node.as_element().unwrap();

    let start = std::time::Instant::now();
    let result = match_complex_selector_list(
        &list,
        &doc,
        &elem,
        target_id,
        &ancestors,
        StyleQuirksMode::NoQuirks,
    );
    let elapsed = start.elapsed();

    assert_eq!(
        result,
        None,
        "a {}-compound div-only selector against a {depth}-deep div chain is \
             genuinely unsatisfiable (one compound more than there are ancestor \
             slots) — must resolve to no match, not merely resolve fast",
        depth + 1
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "match_combinator_chain's Descendant backtracking must be memoized \
             to polynomial time — took {elapsed:?} for a {depth}-deep unsatisfiable \
             chain, which the pre-fix exponential backtracking could never \
             realistically finish at all"
    );
}

#[test]
fn later_sibling_combinator_deep_unsatisfiable_run_does_not_explode() {
    let sibling_count = 200;
    let mut doc = TestDoc::new();
    for _ in 0..sibling_count {
        doc.push_element(0, "div", None);
    }
    let target = doc.push_element(0, "div", None);
    let target_id = StyleNodeId::new(target as u64);
    // `target` has `sibling_count` preceding `<div>` siblings and no
    // ancestor element (root-level, same layout the existing sibling
    // acceptance tests above use to exercise the `ancestors.last() ==
    // None -> dom.root_id()` fallback).
    let selector = vec!["div"; sibling_count + 2].join(" ~ ");
    let list = crate::parse_selector_list(&selector).expect("selector parses");
    let node = doc.node(target_id).unwrap();
    let elem = node.as_element().unwrap();

    let start = std::time::Instant::now();
    let result = match_complex_selector_list(
        &list,
        &doc,
        &elem,
        target_id,
        &[],
        StyleQuirksMode::NoQuirks,
    );
    let elapsed = start.elapsed();

    assert_eq!(
        result,
        None,
        "a {}-compound div-only general-sibling selector against {sibling_count} \
             preceding siblings is genuinely unsatisfiable (one compound more than \
             there are preceding-sibling slots) — must resolve to no match",
        sibling_count + 2
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "LaterSibling backtracking must be bounded by the same memo as \
             Descendant — took {elapsed:?} for {sibling_count} unsatisfiable \
             siblings"
    );
}

#[test]
fn match_complex_selector_list_rejects_unsupported_component_via_safety_net() {
    // pseudo-class components never reach `match_complex_selector_list`
    // in the real pipeline — `ruletree.rs`'s `is_supported_selector_list`
    // drops any rule containing one at `add_stylesheet` time (pinned by
    // `ruletree::tests::pseudo_class_selector_still_dropped`). This test
    // calls `match_complex_selector_list` directly — both it and
    // `parse_selector_list` are reachable from this `#[cfg(test)] mod
    // tests` (`use super::*` / `crate::parse_selector_list`) — to
    // exercise `compound_matches`'s `_ => false` safety-net arm
    // defensively, per its own doc comment. `p:hover` has no combinator,
    // so `ancestors`/`elem_id` are irrelevant here — `&[]` / `p`'s own id
    // (this test was renamed alongside the function when `dom`/
    // `ancestors` args were added; the `elem_id` arg was added later,
    // matching the current signature).
    let list = crate::parse_selector_list("p:hover").expect("selector parses");
    let mut doc = TestDoc::new();
    let p = doc.push_element(0, "p", None);
    let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
    let elem = node.as_element().unwrap();
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        match_complex_selector_list(
            &list,
            &doc,
            &elem,
            StyleNodeId::new(p as u64),
            &[],
            StyleQuirksMode::NoQuirks,
        ),
        None,
        "NonTSPseudoClass component must fall through the safety net"
    );
}

#[test]
fn root_pseudo_class_matches_the_document_root_element_only() {
    // `background-color`, not `color`: `color` is inherited (CSS
    // Cascading L4 §5.2) — even a correct implementation that matched
    // `:root` on `<html>` alone would show a red `body.color` through
    // ordinary inheritance, making that a false-negative test for the
    // "must not also match a descendant" half (same pitfall
    // `child_combinator_applies_declaration_to_direct_child_only`'s own
    // comment documents). `background-color` is not inherited (CSS
    // Backgrounds 3 §2.2), so a red `body` here can only mean `:root`
    // itself wrongly matched it.
    //
    // `RuleTree::empty()` + `add_stylesheet`, not the usual
    // `push_element(0, "style", None)` + `build_rule_tree` convention
    // (quality-lens finding): that convention parks `<style>` itself as
    // a direct child of the Document node — i.e. an element sibling of
    // `<html>` that *also* has `parent_id.is_none()` and would *also*
    // match `:root`. Since this test never asserted anything about
    // `<style>`'s own computed value, that convention only proved "an
    // element with no element parent matches" (true of `html` here by
    // coincidence of push order), not "`:root` matches the root
    // element and no other top-level node" — the actual claim this
    // test's name makes. `RuleTree::empty()` avoids adding any such
    // ambiguous second candidate.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let body = doc.push_element(html, "body", None);

    let mut tree = RuleTree::empty();
    tree.add_stylesheet(":root { background-color: red }", Origin::Author);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[html].background_color, RED,
        ":root must match the root element"
    );
    assert_eq!(
        r.computed[body].background_color,
        ComputedValues::initial().background_color,
        ":root must not match a non-root descendant"
    );
}

#[test]
fn empty_pseudo_class_matches_element_with_no_children() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color, RED,
        ":empty must match a childless element"
    );
}

#[test]
fn empty_pseudo_class_matches_element_with_zero_length_text_child() {
    // Selectors Level 4 ignores zero-length text nodes when determining
    // whether `:empty` matches.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_text(p, "");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color, RED,
        ":empty must match an element with a zero-length text child"
    );
}

#[test]
fn empty_pseudo_class_does_not_match_element_with_an_element_child() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_element(p, "span", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color,
        ComputedValues::initial().color,
        ":empty must not match an element with an element child"
    );
}

#[test]
fn empty_pseudo_class_does_not_match_element_with_a_text_child() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_text(p, "hello");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color,
        ComputedValues::initial().color,
        ":empty must not match an element with a non-empty text child"
    );
}

#[test]
fn empty_pseudo_class_matches_whitespace_only_text_child() {
    // Acceptance-pinning test for the correction documented in
    // `matches_empty` doc's "Spec provenance and correction" note:
    // CSS Selectors L4 *deliberately changed* `:empty` from L3 so that
    // whitespace-only content — "given white space is largely
    // collapsible in HTML and is therefore used for source code
    // formatting" (L4 changelog note, verbatim) — no longer
    // disqualifies. The L4 spec's own worked example lists `<p> </p>`
    // among what `p:empty` matches, verbatim. This test used to assert
    // the opposite (the pre-correction L3-only reading); inverted, not
    // just renamed, when the bug was fixed.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_text(p, " ");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color, RED,
        ":empty must match an element with a document-white-space-only text child (CSS Selectors L4)"
    );
}

#[test]
fn empty_pseudo_class_does_not_match_nbsp_only_text_child() {
    // No-break space (U+00A0) is explicitly NOT a "document white
    // space character" (CSS Text 4, `is_document_white_space` doc) —
    // the L4 spec's own worked example lists `<div>&nbsp;</div>`
    // among what `div:empty` does *not* match, verbatim. Distinguishes
    // this from the (now-passing) plain-space case above: `:empty`'s
    // L4 whitespace carve-out is narrower than "any Unicode
    // whitespace".
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_text(p, "\u{00A0}");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color,
        ComputedValues::initial().color,
        ":empty must not match an element with an NBSP-only text child"
    );
}

#[test]
fn empty_pseudo_class_matches_element_with_only_a_comment_child() {
    // "comments... must not affect whether an element is considered
    // empty" (CSS Selectors L4 §13.2 `#the-empty-pseudo`, verbatim,
    // unchanged from L3) — a comment-only element still matches
    // `:empty`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:empty { color: red }");
    let wrap = doc.push_element(0, "div", None);
    let p = doc.push_element(wrap, "p", None);
    doc.push_comment(p, " note ");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].color, RED,
        ":empty must match an element whose only child is a comment"
    );
}

#[test]
fn first_child_last_child_only_child_ignore_text_node_siblings() {
    // CSS Selectors L3 §6.6 preamble (verbatim, `sibling_position`
    // doc): "Standalone text and other non-element nodes are not
    // counted when calculating the position of an element in its list
    // of siblings" — a text node between two <li> must not shift
    // indices.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "li:first-child { color: red } \
             li:last-child { background-color: red }",
    );
    let ul = doc.push_element(0, "ul", None);
    let first = doc.push_element(ul, "li", None);
    doc.push_text(ul, "\n  "); // whitespace between <li> siblings
    let last = doc.push_element(ul, "li", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[first].color, RED,
        ":first-child must match despite an intervening text node"
    );
    assert_eq!(
        r.computed[last].background_color, RED,
        ":last-child must match despite an intervening text node"
    );
    assert_eq!(
        r.computed[first].background_color,
        ComputedValues::initial().background_color,
        "the first <li> must not also match :last-child"
    );
}

#[test]
fn only_child_matches_the_sole_element_child_and_nothing_else() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "li:only-child { color: red }");
    let solo_wrap = doc.push_element(0, "ul", None);
    let solo = doc.push_element(solo_wrap, "li", None);
    let pair_wrap = doc.push_element(0, "ul", None);
    let pair_a = doc.push_element(pair_wrap, "li", None);
    let pair_b = doc.push_element(pair_wrap, "li", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[solo].color, RED,
        ":only-child must match a sole <li>"
    );
    assert_eq!(
        r.computed[pair_a].color,
        ComputedValues::initial().color,
        ":only-child must not match when a sibling <li> exists"
    );
    assert_eq!(
        r.computed[pair_b].color,
        ComputedValues::initial().color,
        ":only-child must not match when a sibling <li> exists"
    );
}

#[test]
fn nth_child_zebra_striping_acceptance() {
    // Acceptance: `:nth-child(2n+1)` zebra
    // striping. Rows 1/3/5 (1-based) get the declaration, 2/4 don't.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "tr:nth-child(2n+1) { background-color: red }");
    let table = doc.push_element(0, "table", None);
    let rows: Vec<usize> = (0..5)
        .map(|_| doc.push_element(table, "tr", None))
        .collect();

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    for (i, &row) in rows.iter().enumerate() {
        let one_based = i + 1;
        let expect_red = one_based % 2 == 1;
        assert_eq!(
            r.computed[row].background_color,
            if expect_red {
                RED
            } else {
                ComputedValues::initial().background_color
            },
            "row {one_based} (0-based index {i}): nth-child(2n+1) zebra stripe mismatch"
        );
    }
}

#[test]
fn nth_child_negative_b_and_explicit_index_forms() {
    // `:nth-child(3)` (a=0) and `:nth-child(-n+2)` (first two only) —
    // exercises `AnPlusB` beyond the simple odd/even case.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "li:nth-child(3) { color: red } li:nth-child(-n+2) { background-color: red }",
    );
    let ul = doc.push_element(0, "ul", None);
    let items: Vec<usize> = (0..4).map(|_| doc.push_element(ul, "li", None)).collect();

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[items[0]].background_color, RED,
        "index 1 in -n+2"
    );
    assert_eq!(
        r.computed[items[1]].background_color, RED,
        "index 2 in -n+2"
    );
    assert_eq!(
        r.computed[items[2]].background_color,
        ComputedValues::initial().background_color,
        "index 3 not in -n+2"
    );
    assert_eq!(
        r.computed[items[2]].color, RED,
        "index 3 matches nth-child(3)"
    );
    assert_eq!(
        r.computed[items[0]].color,
        ComputedValues::initial().color,
        "index 1 does not match nth-child(3)"
    );
}

#[test]
fn nth_last_child_counts_from_the_end() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "li:nth-last-child(1) { color: red }");
    let ul = doc.push_element(0, "ul", None);
    let items: Vec<usize> = (0..3).map(|_| doc.push_element(ul, "li", None)).collect();

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[items[2]].color, RED,
        ":nth-last-child(1) must match the last element sibling"
    );
    assert_eq!(
        r.computed[items[0]].color,
        ComputedValues::initial().color,
        ":nth-last-child(1) must not match the first element sibling"
    );
}

#[test]
fn first_of_type_last_of_type_only_of_type_are_restricted_to_matching_tag() {
    // Mixed-tag sibling list: <h2><p><p><h2> — the -of-type family must
    // count only same-tag siblings (CSS Selectors L3 §6.6, "an+b-1
    // siblings with the same expanded element name"), unlike plain
    // :first-child/:last-child/:only-child.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "h2:first-of-type { color: red } \
             h2:last-of-type { background-color: red } \
             p:only-of-type { border-top-style: solid }",
    );
    let section = doc.push_element(0, "section", None);
    let h2_first = doc.push_element(section, "h2", None);
    let p = doc.push_element(section, "p", None);
    let h2_last = doc.push_element(section, "h2", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[h2_first].color, RED,
        "h2:first-of-type must match the first <h2> even though a <p> is its actual first-child"
    );
    assert_eq!(
        r.computed[h2_last].background_color, RED,
        "h2:last-of-type must match the second <h2>"
    );
    assert_eq!(
        r.computed[h2_first].background_color,
        ComputedValues::initial().background_color,
        "the first <h2> must not also match :last-of-type"
    );
    assert_eq!(
        r.computed[p].border.top.style,
        BorderStyle::Solid,
        "p:only-of-type must match the sole <p> despite <h2> siblings"
    );
}

#[test]
fn nth_of_type_counts_only_same_tag_siblings() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p:nth-of-type(2) { color: red }");
    let div = doc.push_element(0, "div", None);
    doc.push_element(div, "h2", None);
    let p1 = doc.push_element(div, "p", None);
    doc.push_element(div, "h2", None);
    let p2 = doc.push_element(div, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p2].color, RED,
        "p:nth-of-type(2) must match the 2nd <p>, ignoring interleaved <h2> siblings"
    );
    assert_eq!(
        r.computed[p1].color,
        ComputedValues::initial().color,
        "p:nth-of-type(2) must not match the 1st <p>"
    );
}

#[test]
fn root_element_matches_first_child_last_child_only_child_and_nth_child_1() {
    // The root element has no *element* parent, but per CSS Selectors
    // L3's "an+b-1 siblings before/after it" framing (`matches_nth`
    // doc) it still has a (trivial, size-1) sibling list — itself
    // alone under the Document node.
    //
    // Deliberately does NOT use the usual `push_element(0, "style",
    // None)` + `build_rule_tree` convention: that convention parks the
    // `<style>` element itself as a direct child of the Document node
    // (id 0) — i.e. as an *element sibling of the root element being
    // tested here*, which would make `<style>` the real first element
    // child and `<html>` the second, defeating the point of this test.
    // `RuleTree::empty()` + `add_stylesheet` supplies the CSS without
    // adding any DOM node at all.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);

    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "html:first-child { color: red } \
             html:last-child { background-color: red } \
             html:only-child { border-top-style: solid } \
             html:nth-child(1) { border-bottom-style: solid }",
        Origin::Author,
    );
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[html].color, RED,
        "root element must match :first-child"
    );
    assert_eq!(
        r.computed[html].background_color, RED,
        "root element must match :last-child"
    );
    assert_eq!(
        r.computed[html].border.top.style,
        BorderStyle::Solid,
        "root element must match :only-child"
    );
    assert_eq!(
        r.computed[html].border.bottom.style,
        BorderStyle::Solid,
        "root element must match :nth-child(1)"
    );
}

#[test]
fn root_element_does_not_match_nth_child_2() {
    // Negative half of the previous test (WPT reference:
    // `css/selectors/child-indexed-no-parent.html`, per CSS Selectors
    // L3's "an+b-1 siblings before it" framing this crate follows):
    // the root element's sibling list under `dom.root_id()` has size
    // 1 (itself alone), so no `:nth-child(N)`/`:nth-last-child(N)` for
    // `N >= 2` can ever match it. `:root:nth-last-child(2)` is the
    // canonical form of this check.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);

    let mut tree = RuleTree::empty();
    tree.add_stylesheet(":root:nth-last-child(2) { color: red }", Origin::Author);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[html].color,
        ComputedValues::initial().color,
        ":root:nth-last-child(2) must not match — the root element has no siblings at all"
    );
}

#[test]
fn section_gt_p_first_child_acceptance() {
    // Acceptance: `.section > p:first-child { font-weight: bold }`.
    // Combines the child combinator with a
    // structural pseudo-class on the *rightmost* compound.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".section > p:first-child { font-weight: bold }");
    let section = doc.push_element(0, "div", None);
    doc.set_attr(section, "class", "section");
    let first_p = doc.push_element(section, "p", None);
    let second_p = doc.push_element(section, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[first_p].font_weight, 700.0,
        ".section > p:first-child must match the first <p>"
    );
    assert_eq!(
        r.computed[second_p].font_weight,
        ComputedValues::initial().font_weight,
        ".section > p:first-child must not match the second <p>"
    );
}

#[test]
fn is_pseudo_class_matches_any_inner_selector() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "div:is(.featured, [data-kind=selected]) { font-weight: bold }",
    );

    let by_class = doc.push_element(0, "div", None);
    doc.set_attr(by_class, "class", "featured");
    let by_attribute = doc.push_element(0, "div", None);
    doc.set_attr(by_attribute, "data-kind", "selected");
    let no_match = doc.push_element(0, "div", None);

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[by_class].font_weight, 700.0);
    assert_eq!(result.computed[by_attribute].font_weight, 700.0);
    assert_eq!(result.computed[no_match].font_weight, 400.0);
}

#[test]
fn is_and_where_use_selectors_specificity_rules() {
    let mut is_doc = TestDoc::new();
    let is_style = is_doc.push_element(0, "style", None);
    is_doc.push_text(
        is_style,
        ".featured { font-weight: bold } \
             div:is(#unused, .featured) { font-weight: 300 }",
    );
    let is_element = is_doc.push_element(0, "div", None);
    is_doc.set_attr(is_element, "class", "featured");
    let is_tree = build_rule_tree(&is_doc);
    let is_result = cascade(&is_doc, &is_tree).expect("cascade Ok");
    assert_eq!(is_result.computed[is_element].font_weight, 300.0);

    let mut where_doc = TestDoc::new();
    let where_style = where_doc.push_element(0, "style", None);
    where_doc.push_text(
        where_style,
        ".featured { font-weight: bold } \
             div:where(#unused, .featured) { font-weight: 300 }",
    );
    let where_element = where_doc.push_element(0, "div", None);
    where_doc.set_attr(where_element, "class", "featured");
    let where_tree = build_rule_tree(&where_doc);
    let where_result = cascade(&where_doc, &where_tree).expect("cascade Ok");
    assert_eq!(where_result.computed[where_element].font_weight, 700.0);
}

#[test]
fn forgiving_logical_selector_ignores_invalid_branches() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "div:is(.featured, :unknown-pseudo) { font-weight: 300 } \
             div:where(.selected, 123) { font-weight: 500 }",
    );
    let is_element = doc.push_element(0, "div", None);
    doc.set_attr(is_element, "class", "featured");
    let where_element = doc.push_element(0, "div", None);
    doc.set_attr(where_element, "class", "selected");

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[is_element].font_weight, 300.0);
    assert_eq!(result.computed[where_element].font_weight, 500.0);
}

#[test]
fn has_matches_descendants_and_relative_siblings() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "section:has(> .direct:is(.featured, 123)) { font-weight: 700 } \
             article:has(.deep) { font-weight: 600 } \
             div:has(+ p.adjacent) { font-weight: 500 } \
             div:has(~ p.later) { font-weight: 300 }",
    );

    let direct_section = doc.push_element(0, "section", None);
    let direct_child = doc.push_element(direct_section, "div", None);
    doc.set_attr(direct_child, "class", "direct featured");
    let indirect_section = doc.push_element(0, "section", None);
    let wrapper = doc.push_element(indirect_section, "div", None);
    let indirect_child = doc.push_element(wrapper, "div", None);
    doc.set_attr(indirect_child, "class", "direct featured");

    let deep_article = doc.push_element(0, "article", None);
    let deep_wrapper = doc.push_element(deep_article, "div", None);
    doc.push_element(deep_wrapper, "span", None);
    let deep_target = doc.push_element(deep_wrapper, "span", None);
    doc.set_attr(deep_target, "class", "deep");
    let empty_article = doc.push_element(0, "article", None);

    let adjacent_parent = doc.push_element(0, "main", None);
    let adjacent_anchor = doc.push_element(adjacent_parent, "div", None);
    let adjacent = doc.push_element(adjacent_parent, "p", None);
    doc.set_attr(adjacent, "class", "adjacent");
    let no_adjacent_parent = doc.push_element(0, "main", None);
    let no_adjacent = doc.push_element(no_adjacent_parent, "div", None);
    doc.push_element(no_adjacent_parent, "span", None);

    let later_parent = doc.push_element(0, "main", None);
    let later_anchor = doc.push_element(later_parent, "div", None);
    doc.push_element(later_parent, "span", None);
    let later = doc.push_element(later_parent, "p", None);
    doc.set_attr(later, "class", "later");
    let no_later_parent = doc.push_element(0, "main", None);
    let no_later = doc.push_element(no_later_parent, "div", None);
    doc.push_element(no_later_parent, "span", None);

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[direct_section].font_weight, 700.0);
    assert_eq!(result.computed[indirect_section].font_weight, 400.0);
    assert_eq!(result.computed[deep_article].font_weight, 600.0);
    assert_eq!(result.computed[empty_article].font_weight, 400.0);
    assert_eq!(result.computed[adjacent_anchor].font_weight, 500.0);
    assert_eq!(result.computed[no_adjacent].font_weight, 400.0);
    assert_eq!(result.computed[later_anchor].font_weight, 300.0);
    assert_eq!(result.computed[no_later].font_weight, 400.0);
}

#[test]
fn has_matches_mixed_relative_combinator_chains() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "#child-desc:has(> .first .target) { font-weight: 701 } \
             #adj-desc:has(+ .first .target) { font-weight: 702 } \
             #child-sib:has(> .first + .target) { font-weight: 703 } \
             #adj-sib:has(+ .first + .target) { font-weight: 704 } \
             #general-child:has(~ .first > .target) { font-weight: 705 } \
             #general-sib:has(~ .first + .target) { font-weight: 706 }",
    );

    let child_desc = doc.push_element(0, "section", None);
    doc.set_attr(child_desc, "id", "child-desc");
    let first = doc.push_element(child_desc, "div", None);
    doc.set_attr(first, "class", "first");
    let target = doc.push_element(first, "span", None);
    doc.set_attr(target, "class", "target");

    let adj_desc_parent = doc.push_element(0, "main", None);
    let adj_desc = doc.push_element(adj_desc_parent, "section", None);
    doc.set_attr(adj_desc, "id", "adj-desc");
    doc.push_text(adj_desc_parent, "ignored text");
    let first = doc.push_element(adj_desc_parent, "div", None);
    doc.set_attr(first, "class", "first");
    let target = doc.push_element(first, "span", None);
    doc.set_attr(target, "class", "target");

    let child_sib = doc.push_element(0, "section", None);
    doc.set_attr(child_sib, "id", "child-sib");
    let first = doc.push_element(child_sib, "div", None);
    doc.set_attr(first, "class", "first");
    doc.push_comment(child_sib, "ignored comment");
    let target = doc.push_element(child_sib, "span", None);
    doc.set_attr(target, "class", "target");

    let adj_sib_parent = doc.push_element(0, "main", None);
    let adj_sib = doc.push_element(adj_sib_parent, "section", None);
    doc.set_attr(adj_sib, "id", "adj-sib");
    let first = doc.push_element(adj_sib_parent, "div", None);
    doc.set_attr(first, "class", "first");
    let target = doc.push_element(adj_sib_parent, "span", None);
    doc.set_attr(target, "class", "target");

    let general_child_parent = doc.push_element(0, "main", None);
    let general_child = doc.push_element(general_child_parent, "section", None);
    doc.set_attr(general_child, "id", "general-child");
    doc.push_element(general_child_parent, "i", None);
    let first = doc.push_element(general_child_parent, "div", None);
    doc.set_attr(first, "class", "first");
    let target = doc.push_element(first, "span", None);
    doc.set_attr(target, "class", "target");

    let general_sib_parent = doc.push_element(0, "main", None);
    let general_sib = doc.push_element(general_sib_parent, "section", None);
    doc.set_attr(general_sib, "id", "general-sib");
    doc.push_element(general_sib_parent, "i", None);
    let first = doc.push_element(general_sib_parent, "div", None);
    doc.set_attr(first, "class", "first");
    let target = doc.push_element(general_sib_parent, "span", None);
    doc.set_attr(target, "class", "target");

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    for (id, expected) in [
        (child_desc, 701.0),
        (adj_desc, 702.0),
        (child_sib, 703.0),
        (adj_sib, 704.0),
        (general_child, 705.0),
        (general_sib, 706.0),
    ] {
        assert_eq!(result.computed[id].font_weight, expected);
    }
}

#[test]
fn has_deep_branched_miss_uses_bounded_ancestor_storage() {
    const DEPTH: usize = 2_048;
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "section:has(.missing) { font-weight: 700 } section { color: red }",
    );

    let anchor = doc.push_element(0, "section", None);
    let mut current = anchor;
    for _ in 0..DEPTH {
        // The deep child keeps the walk going while the sibling remains
        // pending on the explicit stack at every level.
        let next = doc.push_element(current, "div", None);
        doc.push_element(current, "span", None);
        current = next;
    }

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[anchor].color, RED);
    assert_eq!(
        result.computed[anchor].font_weight,
        ComputedValues::initial().font_weight,
    );
}

#[test]
fn nested_has_in_forgiving_branches_is_ignored() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "#nested:has(:is(:has(*), .valid)) { font-weight: 701 } \
             #fallback:is(:has(:has(*)), .fallback) { font-weight: 702 }",
    );

    let nested = doc.push_element(0, "div", None);
    doc.set_attr(nested, "id", "nested");
    let valid = doc.push_element(nested, "span", None);
    doc.set_attr(valid, "class", "valid");

    let fallback = doc.push_element(0, "div", None);
    doc.set_attr(fallback, "id", "fallback");
    doc.set_attr(fallback, "class", "fallback");

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[nested].font_weight, 701.0);
    assert_eq!(result.computed[fallback].font_weight, 702.0);
}

#[test]
fn logical_selectors_compose_with_has_and_not() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "section:is(:has(> .item), .fallback) { font-weight: 700 } \
             section:not(:has(> .missing)) { color: red }",
    );

    let has_section = doc.push_element(0, "section", None);
    let item = doc.push_element(has_section, "div", None);
    doc.set_attr(item, "class", "item");
    let fallback_section = doc.push_element(0, "section", None);
    doc.set_attr(fallback_section, "class", "fallback");
    let missing_section = doc.push_element(0, "section", None);
    let missing = doc.push_element(missing_section, "div", None);
    doc.set_attr(missing, "class", "missing");

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(result.computed[has_section].font_weight, 700.0);
    assert_eq!(result.computed[fallback_section].font_weight, 700.0);
    assert_eq!(result.computed[missing_section].font_weight, 400.0);
    assert_eq!(result.computed[has_section].color, RED);
    assert_eq!(result.computed[fallback_section].color, RED);
    assert_eq!(
        result.computed[missing_section].color,
        ComputedValues::initial().color
    );
}

#[test]
fn negation_matches_when_inner_selector_does_not_match() {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "section > div:not(:first-child) { font-weight: bold }",
    );
    let section = doc.push_element(0, "section", None);
    let first = doc.push_element(section, "div", None);
    let second = doc.push_element(section, "div", None);

    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        result.computed[first].font_weight,
        ComputedValues::initial().font_weight
    );
    assert_eq!(result.computed[second].font_weight, 700.0);
}

#[test]
fn structural_pseudo_class_on_an_ancestor_compound_uses_that_ancestors_own_parent() {
    // `body > div:only-child p` — the structural pseudo-class sits on
    // the *ancestor* compound (`div:only-child`), reached by crossing
    // the child combinator via `match_from_ancestor`, not on the
    // rightmost compound. This is the one test that would catch a
    // wrong `parent_id` slice at that recursion site (using `elem`'s
    // parent instead of the ancestor-being-matched's own parent).
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "body > div:only-child p { color: red }");
    let body = doc.push_element(0, "body", None);
    let solo_div = doc.push_element(body, "div", None); // only element child of <body>
    let p_under_solo = doc.push_element(solo_div, "p", None);

    let other_body = doc.push_element(0, "body", None);
    let div_a = doc.push_element(other_body, "div", None);
    doc.push_element(other_body, "div", None); // makes div_a NOT an only-child
    let p_under_div_a = doc.push_element(div_a, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p_under_solo].color, RED,
        "body > div:only-child p must match when the <div> really is body's only child"
    );
    assert_eq!(
        r.computed[p_under_div_a].color,
        ComputedValues::initial().color,
        "body > div:only-child p must not match when the <div> has a sibling <div>"
    );
}

#[test]
fn nth_child_of_extended_syntax_is_accepted_as_selector_list_argument() {
    let list = crate::parse_selector_list("p:nth-child(2n+1 of .foo)")
        .expect("the `of S` selector-list syntax must parse");
    assert_eq!(list.slice().len(), 1);
}

#[test]
fn nth_child_of_selector_list_filters_siblings_for_both_directions() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        "li:nth-child(2 of .featured, [data-kind=selected]) { color: red } \
             li:nth-last-child(2 of .featured, [data-kind=selected]) { background-color: red }",
    );
    let ul = doc.push_element(0, "ul", None);

    let first_featured = doc.push_element(ul, "li", None);
    doc.set_attr(first_featured, "class", "featured");

    let unfiltered_before_second = doc.push_element(ul, "li", None);

    let second_filtered = doc.push_element(ul, "li", None);
    doc.set_attr(second_filtered, "data-kind", "selected");

    let third_filtered_non_li = doc.push_element(ul, "div", None);
    doc.set_attr(third_filtered_non_li, "class", "featured");

    let unfiltered = doc.push_element(ul, "li", None);

    let second_from_end = doc.push_element(ul, "li", None);
    doc.set_attr(second_from_end, "class", "featured");

    let unfiltered_before_last = doc.push_element(ul, "li", None);

    let last_filtered = doc.push_element(ul, "li", None);
    doc.set_attr(last_filtered, "data-kind", "selected");

    let tree = build_rule_tree(&doc);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        tree.style_rules.len(),
        2,
        "flat selector-list filters must remain captured"
    );
    let r = cascade(&doc, &tree).expect("cascade Ok");

    assert_eq!(r.computed[second_filtered].color, RED);
    assert_eq!(r.computed[second_from_end].background_color, RED);
    assert_eq!(
        r.computed[first_featured].color,
        ComputedValues::initial().color
    );
    assert_eq!(
        r.computed[third_filtered_non_li].color,
        ComputedValues::initial().color
    );
    assert_eq!(
        r.computed[unfiltered].color,
        ComputedValues::initial().color
    );
    assert_eq!(
        r.computed[unfiltered_before_second].color,
        ComputedValues::initial().color
    );
    assert_eq!(
        r.computed[unfiltered_before_last].background_color,
        ComputedValues::initial().background_color
    );
    assert_eq!(
        r.computed[last_filtered].background_color,
        ComputedValues::initial().background_color
    );
}

#[test]
fn nested_nth_child_filter_is_rejected_before_sibling_scan() {
    // Selectors L4 permits a complex-real-selector-list in `of S`, but this
    // implementation rejects nested structural nth components before the
    // outer filter can scan the sibling list. Keep enough siblings here to
    // make accidentally accepting the nested filter observable.
    let selector = "li:nth-child(1 of :nth-child(1))";
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        crate::parse_selector_list(selector).is_ok(),
        "the nested nth-child selector must parse before support filtering"
    );

    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "li:nth-child(1 of :nth-child(1)) { background-color: red }",
    );
    let list = doc.push_element(0, "ul", None);
    let sibling_count = 256;
    let items: Vec<_> = (0..sibling_count)
        .map(|_| doc.push_element(list, "li", None))
        .collect();

    let tree = build_rule_tree(&doc);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        tree.style_rules.is_empty(),
        "nested nth-child filters must be dropped before sibling scans"
    );
    let result = cascade(&doc, &tree).expect("cascade Ok");
    for item in items {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            result.computed[item].background_color,
            ComputedValues::initial().background_color,
            "nested nth-child filter must not style any of {sibling_count} siblings"
        );
    }
}

#[test]
fn nested_nth_last_child_filter_is_rejected_before_sibling_scan() {
    // The from-end form must share the same bounded support boundary as
    // the from-start form; otherwise it could retain a second expensive
    // nested sibling scan path.
    let selector = "li:nth-last-child(1 of :nth-last-child(1))";
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        crate::parse_selector_list(selector).is_ok(),
        "the nested nth-last-child selector must parse before support filtering"
    );

    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(
        style,
        "li:nth-last-child(1 of :nth-last-child(1)) { background-color: red }",
    );
    let list = doc.push_element(0, "ul", None);
    let sibling_count = 256;
    let items: Vec<_> = (0..sibling_count)
        .map(|_| doc.push_element(list, "li", None))
        .collect();

    let tree = build_rule_tree(&doc);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        tree.style_rules.is_empty(),
        "nested nth-last-child filters must be dropped before sibling scans"
    );
    let result = cascade(&doc, &tree).expect("cascade Ok");
    for item in items {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            result.computed[item].background_color,
            ComputedValues::initial().background_color,
            "nested nth-last-child filter must not style any of {sibling_count} siblings"
        );
    }
}

#[test]
fn nth_child_of_ignores_inert_element_siblings() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "li:nth-child(2 of .featured) { color: red }");
    let ul = doc.push_element(0, "ul", None);

    let first_featured = doc.push_element(ul, "li", None);
    doc.set_attr(first_featured, "class", "featured");

    let inert_featured = doc.push_element(ul, "li", None);
    doc.set_attr(inert_featured, "class", "featured");
    doc.set_in_document(inert_featured, false);

    let second_featured = doc.push_element(ul, "li", None);
    doc.set_attr(second_featured, "class", "featured");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");

    assert_eq!(r.computed[second_featured].color, RED);
    assert_eq!(
        r.computed[inert_featured].color,
        ComputedValues::initial().color
    );
}

#[test]
fn resolve_case_sensitivity_html_default_namespace_folds_case_for_html_case_insensitive_attr() {
    // `type` is on HTML's ASCII-case-insensitive attribute list (the
    // selectors crate's generated `ascii_case_insensitive_html_attributes`
    // set) — with no explicit `i`/`s` flag, `[type=...]` parses to
    // `ParsedCaseSensitivity::AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument`,
    // which `resolve_case_sensitivity` must fold to ASCII-case-insensitive
    // for an element in the default (HTML) namespace — `TestElementRef`
    // returns `None` from `namespace_uri()` unless overridden via
    // `TestDoc::set_namespace`.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[type=\"text\"] { color: red }");
    let input = doc.push_element(0, "input", None);
    doc.set_attr(input, "type", "TEXT"); // different case than the selector

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[input].color, RED,
        "[type=...] must ASCII-case-fold under the HTML default"
    );
}

#[test]
fn resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive() {
    // Same `[type=...]` shape as the sibling test above, but the element
    // carries an explicit non-HTML namespace (SVG) — `resolve_case_sensitivity`
    // must fall back to case-sensitive matching for it (own doc comment:
    // "SVG 等 non-HTML namespace の element は case-sensitive 側に倒す").
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "[type=\"text\"] { color: red }");
    let input = doc.push_element(0, "input", None);
    doc.set_attr(input, "type", "TEXT");
    doc.set_namespace(input, "http://www.w3.org/2000/svg");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[input].color,
        ComputedValues::initial().color,
        "non-HTML-namespace element must not case-fold [type=...]"
    );
}

fn deep_child_combinator_chain_doc(depth: usize) -> (TestDoc, usize) {
    let mut doc = TestDoc::new();
    let mut parent = 0usize;
    for _ in 0..depth {
        parent = doc.push_element(parent, "div", None);
    }
    let style = doc.push_element(parent, "style", None);
    let selector = vec!["div"; depth].join(" > ");
    doc.push_text(style, &format!("{selector} {{ color: red }}"));
    (doc, parent)
}

#[test]
fn deep_child_combinator_chain_small_stack_no_overflow() {
    let (doc, deepest) = deep_child_combinator_chain_doc(500);
    let tree = build_rule_tree(&doc);
    let color = std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn_scoped(scope, || {
                let result = cascade(&doc, &tree).expect("cascade Ok");
                result.computed[deepest].color
            })
            .expect("spawn thread");
        handle.join().expect(
            "thread must not stack-overflow matching a long, successively-matching \
                 child-combinator chain",
        )
    });
    assert_eq!(color, RED);
}

#[test]
fn no_pseudo_element_rule_leaves_pseudo_map_empty_for_that_element() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, "p { color: red }");
    doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert!(r.pseudo.is_empty(), "no ::before/::after rule anywhere");
}

#[test]
fn bare_before_pseudo_element_matches_every_element_like_universal() {
    // Bare `::before` (no preceding type/class) parses as an implicit
    // universal originating-element selector — `*::before`. Both `<p>`
    // and `<span>` must get an entry.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, r#"::before { content: "x" }"#);
    let p = doc.push_element(0, "p", None);
    let span = doc.push_element(0, "span", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert!(
        r.pseudo
            .contains_key(&(StyleNodeId(p as u64), PseudoElem::Before))
    );
    assert!(
        r.pseudo
            .contains_key(&(StyleNodeId(span as u64), PseudoElem::Before))
    );
}

#[test]
fn pseudo_element_originating_selector_can_use_a_combinator_chain() {
    // Every other pseudo-element test above uses a single-compound
    // originating-element selector (`.foo::before`, `p::before`, bare
    // `::before`) — this one exercises `selector_matches_pseudo_element`
    // when the part of the selector *before* the pseudo-element itself
    // spans a combinator (`div p::before`, a descendant combinator),
    // which routes through `match_combinator_chain` exactly like an
    // ordinary (non-pseudo) selector's own combinator chain does.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, r#"div p::before { content: "nested" }"#);
    let div = doc.push_element(0, "div", None);
    let p_inside = doc.push_element(div, "p", None);
    let p_outside = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        r.pseudo
            .contains_key(&(StyleNodeId(p_inside as u64), PseudoElem::Before)),
        "`div p::before` must match the `<p>` that is a descendant of `<div>`"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        !r.pseudo
            .contains_key(&(StyleNodeId(p_outside as u64), PseudoElem::Before)),
        "a `<p>` outside any `<div>` must not match `div p::before`"
    );
}

#[test]
fn selector_list_can_mix_real_element_and_pseudo_element_targets() {
    // `p, p::before { content: "x" }` — one selector targets the real
    // `<p>`, the other targets its `::before`. Both must apply
    // independently from the same rule.
    use crate::property::ContentComponent;
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, r#"p, p::before { content: "x" }"#);
    let p = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        *r.computed[p].content,
        vec![ContentComponent::Literal("x".into())],
        "the `p` branch of the selector list must still apply to the \
             real element"
    );
    let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
    assert_eq!(*before.content, vec![ContentComponent::Literal("x".into())]);
}

#[test]
fn pseudo_element_selector_never_matches_real_element_directly() {
    // Safety-net regression: `.foo::before` alone must not also apply
    // its declarations to the real `.foo` element (only to its
    // `::before`) — pins the `compound_matches` `_ => false` interaction
    // `selector_matches_pseudo_element`'s doc describes.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, r#".foo::before { color: blue }"#);
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "class", "foo");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[p].color,
        ComputedValues::initial().color,
        "must stay initial — the ::before rule must not leak onto the \
             real element"
    );
}
