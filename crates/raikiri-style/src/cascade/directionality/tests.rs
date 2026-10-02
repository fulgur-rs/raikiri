use super::*;
use crate::cascade::cascade;
use crate::cascade::test_support::*;
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

#[test]
fn dir_matches_explicit_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
    let p = doc.push_element(0, "p", None);
    doc.set_attr(p, "dir", "ltr");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
}

#[test]
fn dir_matches_explicit_rtl_and_inherits_to_descendant_without_own_dir() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":dir(rtl) { font-family: rtl-font }");
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None); // no dir of its own

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[article].font_family[0].to_string(),
        "rtl-font",
        ":dir(rtl) must match the element with the explicit attribute"
    );
    assert_eq!(
        r.computed[span].font_family[0].to_string(),
        "rtl-font",
        ":dir(rtl) must match a descendant with no dir attribute of its \
         own, inheriting from its dir=\"rtl\" ancestor"
    );
}

#[test]
fn dir_matches_explicit_ltr_and_inherits_to_descendant_without_own_dir() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let section = doc.push_element(0, "section", None);
    doc.set_attr(section, "dir", "rtl"); // grandparent, see doc above
    let article = doc.push_element(section, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None); // no dir of its own

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        ":dir(ltr) must match a descendant with no dir attribute of its \
         own, inheriting from its dir=\"ltr\" ancestor rather than \
         continuing past it to the dir=\"rtl\" grandparent"
    );
}

#[test]
fn dir_defaults_to_ltr_when_no_dir_attribute_anywhere() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { font-family: rtl-font } :dir(ltr) { font-family: ltr-font }",
    );
    let p = doc.push_element(0, "p", None);

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
}

#[test]
fn dir_auto_scans_own_text_ltr_first_strong_overrides_rtl_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // "Hello" is wrapped in an ordinary (non-excluded) nested <b>, not a
    // direct text child of `span` — exercises the recursive descent
    // into an un-excluded element subtree in
    // The descendant scan, not just the direct-text-child case.
    let bold = doc.push_element(span, "b", None);
    doc.push_text(bold, "Hello"); // first strong character 'H' is type L

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[article].background_color, RED,
        ":dir(rtl) must still match the article's own explicit dir=\"rtl\""
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "dir=\"auto\" must resolve via the element's own text scan (first \
         strong character 'H' is type L → ltr), not by inheriting the \
         rtl ancestor's directionality"
    );
}

#[test]
fn dir_auto_with_arabic_text_resolves_rtl_despite_ltr_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    doc.push_text(span, "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}"); // "السلام"

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "first strong character is Arabic (type AL) → rtl"
    );
}

#[test]
fn dir_auto_with_hebrew_text_resolves_rtl_despite_ltr_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    doc.push_text(span, "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"); // "שלום"

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "first strong character is Hebrew (type R) → rtl"
    );
}

#[test]
fn dir_auto_with_hebrew_presentation_forms_text_resolves_rtl_despite_ltr_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    doc.push_text(span, "\u{FB1D}"); // HEBREW LETTER YOD WITH HIRIQ

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "Hebrew presentation forms character is type R → rtl"
    );
}

#[test]
fn dir_auto_with_leading_ltr_mark_before_arabic_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+200E LEFT-TO-RIGHT MARK, then "السلام" (Arabic, type AL).
    doc.push_text(
        span,
        "\u{200E}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "first strong character is U+200E (type L) → ltr, despite \
         unrelated Arabic text right after it"
    );
}

#[test]
fn dir_auto_with_leading_rtl_mark_before_latin_text_resolves_rtl() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+200F RIGHT-TO-LEFT MARK, then unrelated Latin (type L) text.
    doc.push_text(span, "\u{200F}Hello");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "first strong character is U+200F (type R) → rtl, despite \
         unrelated Latin text right after it"
    );
}

#[test]
fn dir_auto_skips_non_strong_alphabetic_combining_mark_before_hebrew_resolves_rtl() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+0941 DEVANAGARI VOWEL SIGN U, then "שלום" (Hebrew, type R).
    doc.push_text(span, "\u{0941}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "leading non-strong combining mark must be skipped (not \
         misclassified as strong L) so the scan reaches the Hebrew \
         text and resolves rtl"
    );
}

#[test]
fn dir_auto_skips_non_strong_alphabetic_modifier_letter_before_arabic_resolves_rtl() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "ltr");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT, then "السلام" (Arabic, type AL).
    doc.push_text(
        span,
        "\u{02C6}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, BLUE);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "leading non-strong alphabetic modifier letter must be skipped \
         so the scan reaches the Arabic text and resolves rtl"
    );
}

#[test]
fn dir_auto_skips_arabic_indic_digit_before_latin_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+0664 ARABIC-INDIC DIGIT FOUR, then Latin "H".
    doc.push_text(span, "\u{0664}H");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "leading Arabic-Indic digit (weak AN, not strong AL) must be \
         skipped so the scan reaches the Latin text and resolves ltr"
    );
}

#[test]
fn dir_auto_with_devanagari_digit_before_arabic_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+0966 DEVANAGARI DIGIT ZERO, then "السلام" (Arabic, type AL).
    doc.push_text(
        span,
        "\u{0966}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "leading Devanagari digit (strong L, despite not being \
         is_alphabetic()) must resolve ltr on its own, not fall through \
         to the later Arabic text"
    );
}

#[test]
fn dir_auto_with_armenian_punctuation_before_hebrew_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+055A ARMENIAN APOSTROPHE, then "שלום" (Hebrew, type R).
    doc.push_text(span, "\u{055A}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "leading Armenian punctuation (strong L, despite not being \
         is_alphabetic()) must resolve ltr on its own, not fall \
         through to the later Hebrew text"
    );
}

#[test]
fn dir_auto_with_balinese_virama_before_arabic_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+1B44 BALINESE ADEG ADEG, then "السلام" (Arabic, type AL).
    doc.push_text(
        span,
        "\u{1B44}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "leading Balinese virama (strong L, despite not being \
         is_alphabetic()) must resolve ltr on its own, not fall \
         through to the later Arabic text"
    );
}

#[test]
fn dir_auto_with_sharada_vowel_sign_ooe_before_arabic_text_resolves_ltr() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    // U+11B61 SHARADA VOWEL SIGN OOE, then "السلام" (Arabic, type AL).
    doc.push_text(
        span,
        "\u{11B61}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "leading Sharada vowel sign (strong L, despite not being \
         is_alphabetic() under this workspace's pinned rustc) must \
         resolve ltr on its own, not fall through to the later Arabic \
         text"
    );
}

#[test]
fn strong_bidi_range_tables_preserve_classification_invariants() {
    fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
        a.0 <= b.1 && b.0 <= a.1
    }

    fn assert_table_is_internally_disjoint(name: &str, table: &[(u32, u32)]) {
        for (index, &left) in table.iter().enumerate() {
            assert!(left.0 <= left.1, "{name}[{index}] has an inverted range");
            for (other_index, &right) in table.iter().enumerate().skip(index + 1) {
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    !overlaps(left, right),
                    "{name}[{index}] {left:#x?} overlaps {name}[{other_index}] {right:#x?}"
                );
            }
        }
    }

    fn assert_tables_are_disjoint(
        left_name: &str,
        left: &[(u32, u32)],
        right_name: &str,
        right: &[(u32, u32)],
    ) {
        for (left_index, &left_range) in left.iter().enumerate() {
            for (right_index, &right_range) in right.iter().enumerate() {
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    !overlaps(left_range, right_range),
                    "{left_name}[{left_index}] {left_range:#x?} overlaps \
                     {right_name}[{right_index}] {right_range:#x?}"
                );
            }
        }
    }

    let tables = [
        ("AL_RANGES", AL_RANGES),
        ("R_RANGES", R_RANGES),
        (
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
        ),
        ("NON_STRONG_ALPHABETIC_RANGES", NON_STRONG_ALPHABETIC_RANGES),
        (
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        ),
    ];

    for &(name, table) in &tables {
        assert_table_is_internally_disjoint(name, table);
    }
    assert_tables_are_disjoint("AL_RANGES", AL_RANGES, "R_RANGES", R_RANGES);
    assert_tables_are_disjoint(
        "AL_RANGES",
        AL_RANGES,
        "NON_STRONG_ALPHABETIC_RANGES",
        NON_STRONG_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "R_RANGES",
        R_RANGES,
        "NON_STRONG_ALPHABETIC_RANGES",
        NON_STRONG_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "AL_RANGES",
        AL_RANGES,
        "STRONG_L_NON_ALPHABETIC_RANGES",
        STRONG_L_NON_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "R_RANGES",
        R_RANGES,
        "STRONG_L_NON_ALPHABETIC_RANGES",
        STRONG_L_NON_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "NON_STRONG_WITHIN_AL_R_RANGES",
        NON_STRONG_WITHIN_AL_R_RANGES,
        "NON_STRONG_ALPHABETIC_RANGES",
        NON_STRONG_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "NON_STRONG_WITHIN_AL_R_RANGES",
        NON_STRONG_WITHIN_AL_R_RANGES,
        "STRONG_L_NON_ALPHABETIC_RANGES",
        STRONG_L_NON_ALPHABETIC_RANGES,
    );
    assert_tables_are_disjoint(
        "NON_STRONG_ALPHABETIC_RANGES",
        NON_STRONG_ALPHABETIC_RANGES,
        "STRONG_L_NON_ALPHABETIC_RANGES",
        STRONG_L_NON_ALPHABETIC_RANGES,
    );

    for &(lo, hi) in NON_STRONG_WITHIN_AL_R_RANGES {
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes for a deliberately broken generated const table;
        // constructing one here would test the fixture instead of the
        // invariant, while the checked-in table is immutable.
        assert!(
            AL_RANGES
                .iter()
                .chain(R_RANGES)
                .any(|&outer| outer.0 <= lo && hi <= outer.1),
            "non-strong exception {lo:#x}..={hi:#x} is outside AL/R defaults"
        );
    }

    for &(lo, hi) in NON_STRONG_ALPHABETIC_RANGES {
        for cp in lo..=hi {
            let character = char::from_u32(cp).expect("range is a Unicode scalar");
            let resolved = strong_bidi_type(character);
            if character.is_alphabetic() {
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    resolved, None,
                    "alphabetic NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must be excluded from L"
                );
            }
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert!(
                resolved != Some(StrongBidiType::L),
                "NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must never resolve as L"
            );
        }
    }
    for &(lo, hi) in STRONG_L_NON_ALPHABETIC_RANGES {
        for cp in lo..=hi {
            let character = char::from_u32(cp).expect("range is a Unicode scalar");
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert!(
                !character.is_alphabetic(),
                "STRONG_L_NON_ALPHABETIC_RANGES contains alphabetic U+{cp:04X}"
            );
            // cov:ignore: the failure-message branch of this `assert_eq!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert_eq!(
                strong_bidi_type(character),
                Some(StrongBidiType::L),
                "STRONG_L_NON_ALPHABETIC_RANGES entry U+{cp:04X} must resolve as L"
            );
        }
    }

    for &(lo, hi) in AL_RANGES {
        for cp in lo..=hi {
            let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                .iter()
                .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
            {
                None
            } else {
                Some(StrongBidiType::Al)
            };
            // cov:ignore: the failure-message branch of this `assert_eq!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert_eq!(
                strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                expected,
                "AL_RANGES classification changed for U+{cp:04X}"
            );
        }
    }
    for &(lo, hi) in R_RANGES {
        for cp in lo..=hi {
            let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                .iter()
                .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
            {
                None
            } else {
                Some(StrongBidiType::R)
            };
            // cov:ignore: the failure-message branch of this `assert_eq!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert_eq!(
                strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                expected,
                "R_RANGES classification changed for U+{cp:04X}"
            );
        }
    }
}

#[test]
fn dir_auto_with_no_strong_directional_text_falls_back_to_ltr_despite_rtl_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None);
    doc.set_attr(span, "dir", "auto");
    doc.push_text(span, "123 456!");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[article].background_color, RED);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, BLUE,
        "no strong L/AL/R character anywhere → 'ltr' fallback, not the \
         rtl ancestor's directionality"
    );
}

#[test]
fn dir_undefined_still_falls_through_to_ancestor_via_background_color() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let article = doc.push_element(0, "article", None);
    doc.set_attr(article, "dir", "rtl");
    let span = doc.push_element(article, "span", None); // no dir attribute at all

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].background_color, RED,
        "missing dir attribute must still fall through to the ancestor's \
         directionality"
    );
}

#[test]
fn auto_directionality_skips_descendant_with_own_dir_attribute() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let outer = doc.push_element(0, "div", None);
    doc.set_attr(outer, "dir", "auto");
    let inner = doc.push_element(outer, "span", None);
    doc.set_attr(inner, "dir", "rtl");
    doc.push_text(inner, "\u{0627}"); // Arabic alef — must be skipped
    doc.push_text(outer, "Hello"); // outer's own trailing text

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[outer].background_color, BLUE,
        "the dir=\"rtl\" descendant's text must be excluded from the \
         outer element's own auto-directionality scan"
    );
}

#[test]
fn auto_directionality_skips_bdi_descendant_text() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let outer = doc.push_element(0, "div", None);
    doc.set_attr(outer, "dir", "auto");
    let bdi = doc.push_element(outer, "bdi", None); // no dir attribute
    doc.push_text(bdi, "\u{0627}"); // Arabic alef — must be skipped
    doc.push_text(outer, "Hello");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[outer].background_color, BLUE,
        "a bdi descendant's text must be excluded from the outer \
         element's own auto-directionality scan, regardless of its own \
         dir state"
    );
}

#[test]
fn auto_directionality_skips_script_and_style_descendant_text() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let outer = doc.push_element(0, "div", None);
    doc.set_attr(outer, "dir", "auto");
    let nested_script = doc.push_element(outer, "script", None);
    doc.push_text(nested_script, "\u{0627}");
    let nested_style = doc.push_element(outer, "style", None);
    doc.push_text(nested_style, "\u{0627}");
    doc.push_text(outer, "Hello");

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[outer].background_color, BLUE,
        "script/style descendant text must be excluded from the outer \
         element's own auto-directionality scan"
    );
}

#[test]
fn auto_directionality_scans_into_foreign_namespace_descendant_text() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let outer = doc.push_element(0, "div", None);
    doc.set_attr(outer, "dir", "auto");
    let svg_text =
        doc.push_element_with_namespace(outer, "text", "http://www.w3.org/2000/svg", &[]);
    doc.push_text(svg_text, "\u{0627}"); // Arabic alef

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[outer].background_color, RED,
        "a foreign-namespace descendant's text must still be scanned, \
         not excluded"
    );
}

#[test]
fn auto_directionality_ignores_comment_node_text() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(
        s,
        ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
    );
    let outer = doc.push_element(0, "div", None);
    doc.set_attr(outer, "dir", "auto");
    doc.push_comment(outer, "\u{0627}"); // Arabic alef inside a comment

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[outer].background_color, BLUE,
        "a comment node's text must never be scanned; with no other \
         text present this must resolve via the 'ltr' no-strong-\
         character fallback"
    );
}

#[test]
fn dir_attribute_on_foreign_namespace_element_is_ignored_falls_through_to_ancestor() {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
    let html = doc.push_element(0, "html", None); // no dir -> default ltr
    let svg = doc.push_element_with_namespace(
        html,
        "svg",
        "http://www.w3.org/2000/svg",
        &[("dir", "rtl")],
    );

    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(
        r.computed[svg].font_family[0].to_string(),
        "ltr-font",
        "dir on a foreign-namespace element must be ignored, not treated \
         as an explicit directionality"
    );
}

#[test]
fn direction_wired_through_cascade_from_inline_style() {
    use crate::property::Direction;
    let cv = cascade_doc("", "p", Some("direction: rtl"));
    assert_eq!(cv.direction, Direction::Rtl);
}

#[test]
fn direction_inherits_from_parent_element() {
    // CSS Writing Modes 4 §2.1: direction is **inherited**.
    use crate::property::Direction;
    let mut doc = TestDoc::new();
    let p = doc.push_element(0, "p", Some("direction: rtl"));
    let span = doc.push_element(p, "span", None);
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].direction, Direction::Rtl);
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        r.computed[span].direction,
        Direction::Rtl,
        "child should inherit direction from parent (CSS Writing Modes 4 §2.1 Inherited: yes)"
    );
}

#[test]
fn direction_child_own_value_wins_over_inherited() {
    use crate::property::Direction;
    let mut doc = TestDoc::new();
    let p = doc.push_element(0, "p", Some("direction: rtl"));
    let span = doc.push_element(p, "span", Some("direction: ltr"));
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    assert_eq!(r.computed[p].direction, Direction::Rtl);
    assert_eq!(r.computed[span].direction, Direction::Ltr);
}

const DIRECTIONALITY_CHILD_TEST_ENV: &str = "RAIKIRI_DIRECTIONALITY_CHILD_TEST";

fn assert_isolated_test_succeeds(test_name: &str, mode: &str) {
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg(test_name)
        .env(DIRECTIONALITY_CHILD_TEST_ENV, mode)
        .output()
        .expect("spawn isolated test process");
    assert!(
        output.status.success(),
        "isolated {mode} test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn dir_auto_deep_tree_matches_rtl_without_native_recursion() {
    const TEST_NAME: &str =
        "cascade::directionality::tests::dir_auto_deep_tree_matches_rtl_without_native_recursion";
    if std::env::var(DIRECTIONALITY_CHILD_TEST_ENV).as_deref() == Ok("deep") {
        let handle = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut doc = TestDoc::new();
                let auto = doc.push_element(0, "div", None);
                doc.set_attr(auto, "dir", "auto");
                let mut parent = auto;
                for _ in 0..10_000 {
                    parent = doc.push_element(parent, "div", None);
                }
                doc.push_text(parent, "\u{05D0}");
                let query = crate::cascade::SelectorQuery::parse(":dir(rtl)")
                    .expect("valid directionality selector");
                assert!(query.matches(&doc, StyleNodeId::new(auto as u64), &[]));
            })
            .expect("spawn small-stack scan");
        handle
            .join()
            .expect("deep :dir(auto) scan must not overflow");
        return;
    }
    assert_isolated_test_succeeds(TEST_NAME, "deep");
}

#[test]
fn dir_auto_scan_cycle_terminates_within_budget() {
    const TEST_NAME: &str =
        "cascade::directionality::tests::dir_auto_scan_cycle_terminates_within_budget";
    if std::env::var(DIRECTIONALITY_CHILD_TEST_ENV).as_deref() == Ok("cycle") {
        let mut doc = TestDoc::new();
        let auto = doc.push_element(0, "div", None);
        doc.set_attr(auto, "dir", "auto");
        let cycle = doc.push_element(auto, "div", None);
        doc.nodes[cycle].children.push(cycle);
        assert_eq!(
            auto_directionality(&doc, StyleNodeId::new(auto as u64)),
            None,
        );
        return;
    }
    assert_isolated_test_succeeds(TEST_NAME, "cycle");
}

#[test]
fn dir_auto_scan_stops_before_reading_over_budget_text() {
    let mut doc = TestDoc::new();
    let auto = doc.push_element(0, "div", None);
    doc.set_attr(auto, "dir", "auto");
    let text = "a".repeat(9 * 1024 * 1024);
    doc.push_text(auto, &text);
    assert_eq!(
        auto_directionality(&doc, StyleNodeId::new(auto as u64)),
        None,
    );
}

#[test]
fn dir_auto_scan_continues_after_an_invalid_child_id() {
    let mut doc = TestDoc::new();
    let auto = doc.push_element(0, "div", None);
    doc.set_attr(auto, "dir", "auto");
    doc.nodes[auto].children.push(usize::MAX);
    doc.push_text(auto, "\u{05D0}");

    assert_eq!(
        auto_directionality(&doc, StyleNodeId::new(auto as u64)),
        Some(Direction::Rtl),
    );
}
