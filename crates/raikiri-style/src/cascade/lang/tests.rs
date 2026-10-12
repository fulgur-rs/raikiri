use super::*;
use crate::cascade::cascade;
use crate::cascade::test_support::*;
use crate::computed::ComputedValues;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

#[test]
fn lang_matches_own_lang_attribute() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "lang", "ja");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_family[0].to_string(), "serif-ja");
}

#[test]
fn lang_matches_via_ancestor_inherited_language() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
    let html = doc.push_element(0, "html", None);
    doc.set_attr(html, "lang", "ja");
    let body = doc.push_element(html, "body", None);
    let p = doc.push_element(body, "p", None); // no lang of its own

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].font_family[0].to_string(),
        "serif-ja",
        ":lang(ja) must match an element with no lang attribute of its \
         own when an ancestor carries lang=\"ja\""
    );
}

#[test]
fn lang_ja_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
    let html = doc.push_element(0, "html", None); // no lang
    let body = doc.push_element(html, "body", None); // no lang
    let p = doc.push_element(body, "p", None); // no lang

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].font_family,
        ComputedValues::initial().font_family,
        ":lang(ja) must not match when no element in the chain has a lang attribute"
    );
}

#[test]
fn lang_prefers_nearest_ancestor_lang_over_farther_one() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(en) { font-family: serif-en }");
    let html = doc.push_element(0, "html", None);
    doc.set_attr(html, "lang", "ja");
    let section = doc.push_element(html, "section", None);
    doc.set_attr(section, "lang", "en");
    let p = doc.push_element(section, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
}

#[test]
fn lang_on_mathml_namespace_element_is_ignored_falls_through_to_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":lang(en) { font-family: en-font } :lang(ja) { font-family: ja-font }",
    );
    let html = doc.push_element(0, "html", None);
    doc.set_attr(html, "lang", "en");
    let math = doc.push_element_with_namespace(
        html,
        "math",
        "http://www.w3.org/1998/Math/MathML",
        &[("lang", "ja")],
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[math].font_family[0].to_string(),
        "en-font",
        "MathML element's own lang attribute must be ignored per HTML \
         LS §3.2.6.2 (HTML/SVG only), falling through to the <html \
         lang=\"en\"> ancestor — :lang(en) must match, :lang(ja) must not"
    );
}

#[test]
fn lang_pseudo_class_matches_on_non_rightmost_compound_via_descendant_combinator() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(ja) p { font-family: serif-ja }");
    let html = doc.push_element(0, "html", None);
    doc.set_attr(html, "lang", "ja");
    let body = doc.push_element(html, "body", None);
    let p = doc.push_element(body, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[p].font_family[0].to_string(),
        "serif-ja",
        ":lang(ja) p must match <p> whose ancestor <html> (not <p> \
         itself) satisfies :lang(ja), via the ancestor-compound path \
         (match_from_ancestor) rather than the rightmost-element path"
    );
}

#[test]
fn lang_range_matches_more_specific_tag_via_extended_filtering() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(en) { font-family: serif-en }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "lang", "en-US");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
}

#[test]
fn language_range_matches_rfc4647_de_de_examples() {
    // Matches (RFC 4647 §3.3.2, "matches all of the following tags"):
    for tag in [
        "de-DE",
        "de-de",
        "de-Latn-DE",
        "de-Latf-DE",
        "de-DE-x-goethe",
        "de-Latn-DE-1996",
        "de-Deva-DE",
    ] {
        assert!(
            language_range_matches("de-*-DE", tag),
            "de-*-DE must match {tag}"
        );
        assert!(
            language_range_matches("de-DE", tag),
            "de-DE (synonym) must match {tag}"
        );
    }
    // Does not match (RFC 4647 §3.3.2, "does not match any of the
    // following tags"):
    assert!(
        !language_range_matches("de-*-DE", "de"),
        "missing 'DE' subtag entirely"
    );
    assert!(
        !language_range_matches("de-*-DE", "de-x-DE"),
        "singleton 'x' occurs before 'DE', blocking the skip"
    );
    assert!(
        !language_range_matches("de-*-DE", "de-Deva"),
        "'Deva' present but 'DE' subtag never appears"
    );
}

#[test]
fn language_range_matches_is_ascii_case_insensitive() {
    assert!(language_range_matches("EN", "en-us"));
    assert!(language_range_matches("en", "EN-US"));
}

#[test]
fn language_range_matches_rejects_non_ascii_ill_formed_range() {
    for content_language in ["en", "en-US", "åå", ""] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !language_range_matches("åå", content_language),
            "ill-formed range :lang(åå) must never match content \
             language {content_language:?}"
        );
    }
}

#[test]
fn language_range_matches_well_formed_unregistered_subtag_can_match() {
    assert!(language_range_matches("qq", "qq"));
    assert!(language_range_matches("qq", "qq-Latn"));
}

#[test]
fn language_range_matches_rejects_overlong_subtag_on_either_side() {
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        !language_range_matches("en", "en-abcdefghi"),
        "9-character subtag exceeds RFC 5646's 8-character maximum"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        !language_range_matches("en-abcdefghi", "en-US"),
        "same overlong-subtag rejection, this time on the range side"
    );
}

#[test]
fn language_range_matches_rejects_empty_subtag_from_double_hyphen() {
    assert!(!language_range_matches("en", "en--US"));
}

#[test]
fn language_range_matches_canonicalizes_deprecated_primary_language_subtag() {
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        language_range_matches("he", "iw"),
        ":lang(he) must match the deprecated-but-still-in-the-wild lang=\"iw\""
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        language_range_matches("iw", "he"),
        ":lang(iw) must also match lang=\"he\" — the :lang() argument \
         itself is canonicalized too, per Selectors L4 §7.2"
    );
}

#[test]
fn language_range_matches_canonicalizes_remaining_deprecated_subtag_table_entries() {
    for (deprecated, preferred) in [
        ("bh", "bih"),
        ("in", "id"),
        ("ji", "yi"),
        ("jw", "jv"),
        ("mo", "ro"),
    ] {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches(preferred, deprecated),
            ":lang({preferred}) must match lang=\"{deprecated}\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches(deprecated, preferred),
            ":lang({deprecated}) must match lang=\"{preferred}\""
        );
    }
}

#[test]
fn language_range_matches_well_formed_non_canonicalized_tag_still_matches() {
    assert!(language_range_matches("zh-Hans", "zh-Hans-CN"));
    assert!(!language_range_matches("zh-Hant", "zh-Hans-CN"));
}

#[test]
fn language_range_matches_privateuse_tag_matches_itself() {
    assert!(language_range_matches("x-foo", "x-foo"));
    // The ABNF's `1*` requires at least one value subtag after the `x`
    // introducer — a bare `x` alone is ill-formed and must not match.
    assert!(!language_range_matches("x", "x"));
}

#[test]
fn language_range_matches_wildcard_matches_any_tagged_language() {
    // The CSS-spec-level "wildcard doesn't match untagged" rule
    // (`lang_pseudo_matches` doc) is enforced directly by this
    // function's own empty-`content_language` early return, which
    // requires exact string equality against `range` and so yields
    // `false` for a bare `*` range against an empty `content_language`
    // — this function itself just needs to accept any non-empty tag
    // for a bare `*` range (RFC 4647 §3.3.2 step 2's wildcard-subtag
    // clause), which is what the assertions below check.
    assert!(language_range_matches("*", "ja"));
    assert!(language_range_matches("*", "en-US"));
    assert!(language_range_matches("*", "und"));
}

#[test]
fn lang_empty_string_range_matches_when_no_lang_anywhere_in_ancestor_chain() {
    // `background-color`, not `font-family`: `font-family` is inherited
    // (CSS Fonts 4 §2), so a rule that only matched `html` or `body`
    // would still show up on `untagged` via ordinary inheritance,
    // making font-family unable to distinguish "matched `untagged`
    // itself" from "matched an ancestor and inherited down" (same
    // pitfall `root_pseudo_class_matches_the_document_root_element_only`'s
    // own comment documents). `background-color` is not inherited (CSS
    // Backgrounds 3 §2.2), so red on `untagged` can only mean
    // `:lang("")` matched `untagged` itself. `tagged` (an explicit
    // `lang="en"` sibling) is the negative control proving the rule
    // isn't matching unconditionally.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(\"\") { background-color: red }");
    let html = doc.push_element(0, "html", None); // no lang
    let body = doc.push_element(html, "body", None); // no lang
    let untagged = doc.push_element(body, "p", None); // no lang
    let tagged = doc.push_element(body, "p", None);
    doc.set_attr(tagged, "lang", "en");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[untagged].background_color, RED,
        ":lang(\"\") must match an element with no lang/xml:lang anywhere \
         in its ancestor chain, per Selectors L4 §7.2"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[tagged].background_color,
        ComputedValues::initial().background_color,
        ":lang(\"\") must not match an element with an explicit lang \
         attribute in its own chain"
    );
}

#[test]
fn lang_wildcard_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":lang(\"*\") { font-family: wildcard-font }");
    let html = doc.push_element(0, "html", None); // no lang
    let body = doc.push_element(html, "body", None); // no lang
    let untagged = doc.push_element(body, "p", None); // no lang
    let tagged = doc.push_element(body, "p", None);
    doc.set_attr(tagged, "lang", "en");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[untagged].font_family,
        ComputedValues::initial().font_family,
        ":lang(*) must not match an element with no lang/xml:lang \
         anywhere in its ancestor chain, per Selectors L4 §7.2"
    );
    // Positive control: without this, a silently-dropped/unparsed
    // `:lang(*)` rule would make the negative assertion above pass
    // vacuously.
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[tagged].font_family[0].to_string(),
        "wildcard-font",
        ":lang(*) must match an element with a real, non-empty lang \
         attribute"
    );
}
