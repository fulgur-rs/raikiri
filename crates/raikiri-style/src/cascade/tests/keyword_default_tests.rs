//! CSS-wide keywords on the properties of the `keyword_defaults` table (CSS
//! Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::cascade::keyword_defaults::KEYS;
use crate::property::{BoxSizing, FloatValue, TextTransform, WordBreak, property_key_for_name};

/// A property, whether it is inherited, and values that compute apart: the
/// first is the parent's, the second the user agent's, and a box `before`
/// each keyword declaration may start from takes any of them.
const CASES: &[(&str, bool, &[&str])] = &[
    ("position", false, &["relative", "absolute", "fixed"]),
    (
        "box-sizing",
        false,
        &["border-box", "border-box", "content-box"],
    ),
    (
        "text-decoration-line",
        false,
        &["underline", "overline", "line-through"],
    ),
    (
        "text-decoration-style",
        false,
        &["dashed", "dotted", "double"],
    ),
    ("z-index", false, &["3", "5", "-2"]),
    ("break-before", false, &["page", "column", "avoid"]),
    ("break-after", false, &["page", "column", "avoid"]),
    (
        "break-inside",
        false,
        &["avoid", "avoid-page", "avoid-column"],
    ),
    ("float", false, &["left", "right"]),
    ("clear", false, &["both", "left", "right"]),
    (
        "flex-direction",
        false,
        &["column", "row-reverse", "column-reverse"],
    ),
    ("flex-wrap", false, &["wrap", "wrap-reverse"]),
    ("flex-grow", false, &["2", "3", "0.5"]),
    ("flex-shrink", false, &["2", "0", "3"]),
    ("order", false, &["2", "3", "-1"]),
    (
        "justify-content",
        false,
        &["center", "flex-end", "space-between"],
    ),
    (
        "align-content",
        false,
        &["center", "flex-end", "space-between"],
    ),
    ("align-items", false, &["center", "flex-end", "baseline"]),
    ("align-self", false, &["center", "flex-end", "stretch"]),
    ("justify-items", false, &["center", "end", "start"]),
    ("justify-self", false, &["center", "end", "start"]),
    (
        "unicode-bidi",
        false,
        &["isolate", "embed", "bidi-override"],
    ),
    (
        "grid-auto-flow",
        false,
        &["column", "row dense", "column dense"],
    ),
    ("grid-template-areas", false, &["\"a b\"", "\"c d\""]),
    ("grid-row-start", false, &["2", "3", "span 2"]),
    ("grid-row-end", false, &["2", "3", "span 2"]),
    ("grid-column-start", false, &["2", "3", "span 2"]),
    ("grid-column-end", false, &["2", "3", "span 2"]),
    ("object-fit", false, &["cover", "contain", "none"]),
    ("isolation", false, &["isolate", "isolate", "auto"]),
    ("contain", false, &["layout", "paint", "strict"]),
    (
        "will-change",
        false,
        &["transform", "opacity", "scroll-position"],
    ),
    ("mix-blend-mode", false, &["multiply", "screen", "darken"]),
    (
        "mask-image",
        false,
        &["linear-gradient(red, blue)", "linear-gradient(blue, red)"],
    ),
    (
        "clip-path",
        false,
        &["circle(10px)", "inset(2px)", "polygon(0 0, 10px 0, 0 10px)"],
    ),
    ("filter", false, &["blur(2px)", "grayscale(1)", "sepia(1)"]),
    ("table-layout", false, &["fixed", "fixed", "auto"]),
    ("text-overflow", false, &["ellipsis", "ellipsis", "clip"]),
    ("column-count", false, &["2", "3", "4"]),
    ("column-fill", false, &["auto", "auto", "balance"]),
    (
        "border-collapse",
        true,
        &["collapse", "collapse", "separate"],
    ),
    ("caption-side", true, &["bottom", "bottom", "top"]),
    ("empty-cells", true, &["hide", "hide", "show"]),
    ("word-break", true, &["break-all", "keep-all", "normal"]),
    ("overflow-wrap", true, &["anywhere", "break-word", "normal"]),
    ("line-break", true, &["strict", "loose", "anywhere"]),
    ("hyphens", true, &["auto", "none", "manual"]),
    (
        "text-transform",
        true,
        &["uppercase", "lowercase", "capitalize"],
    ),
    (
        "text-justify",
        true,
        &["inter-character", "inter-word", "none"],
    ),
    (
        "hanging-punctuation",
        true,
        &["first", "last", "first last"],
    ),
    (
        "text-autospace",
        true,
        &["no-autospace", "ideograph-alpha", "ideograph-numeric"],
    ),
    (
        "text-spacing-trim",
        true,
        &["space-all", "trim-start", "space-first"],
    ),
    (
        "word-space-transform",
        true,
        &["space", "ideographic-space"],
    ),
    (
        "text-combine-upright",
        true,
        &["all", "digits 2", "digits 3"],
    ),
    ("text-wrap-style", true, &["balance", "pretty", "stable"]),
    (
        "text-emphasis-position",
        true,
        &["under left", "over right", "under right"],
    ),
    ("text-emphasis-style", true, &["dot", "circle", "sesame"]),
    ("text-underline-position", true, &["under", "left", "right"]),
    ("text-decoration-skip-ink", true, &["none", "all", "auto"]),
    (
        "text-decoration-skip-spaces",
        true,
        &["none", "all", "start"],
    ),
    ("font-kerning", true, &["none", "normal", "auto"]),
    ("font-optical-sizing", true, &["none", "none", "auto"]),
    ("font-variant-emoji", true, &["text", "emoji", "unicode"]),
    (
        "font-variant-ligatures",
        true,
        &["none", "no-common-ligatures", "discretionary-ligatures"],
    ),
    ("font-synthesis", true, &["none", "weight", "style"]),
    ("font-variant-position", true, &["sub", "super", "normal"]),
    ("font-palette", true, &["dark", "light", "normal"]),
    (
        "font-variant-numeric",
        true,
        &["oldstyle-nums", "tabular-nums", "slashed-zero"],
    ),
    (
        "font-variant-east-asian",
        true,
        &["jis78", "jis83", "full-width"],
    ),
    (
        "font-variation-settings",
        true,
        &["\"wght\" 700", "\"wght\" 300", "normal"],
    ),
    (
        "font-feature-settings",
        true,
        &["\"liga\" 0", "\"kern\" 0", "normal"],
    ),
    (
        "font-language-override",
        true,
        &["\"ENG\"", "\"JAN\"", "normal"],
    ),
    (
        "font-variant-caps",
        true,
        &["small-caps", "all-small-caps", "petite-caps"],
    ),
    ("hyphenate-character", true, &["'-'", "'~'", "auto"]),
    ("hyphenate-limit-chars", true, &["6 3 3", "7 2 2", "auto"]),
    ("orphans", true, &["3", "4", "5"]),
    ("widows", true, &["3", "4", "5"]),
    (
        "ruby-position",
        true,
        &["under", "alternate", "inter-character"],
    ),
    ("line-height", true, &["2", "3", "40px"]),
    ("letter-spacing", true, &["3px", "5px", "1em"]),
    ("word-spacing", true, &["3px", "5px", "1em"]),
    ("tab-size", true, &["3", "5", "20px"]),
    ("border-spacing", true, &["3px", "5px 6px", "7px"]),
];

/// The computed values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet.
fn child(parent: &str, sheets: &[(&str, Origin)], css: &str) -> ComputedValues {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    for &(sheet, origin) in sheets {
        tree.add_stylesheet(sheet, origin);
    }
    tree.add_stylesheet(css, Origin::Author);
    cascade(&doc, &tree).unwrap().computed[child].clone()
}

fn same(left: &ComputedValues, right: &ComputedValues) -> bool {
    format!("{left:?}") == format!("{right:?}")
}

#[test]
fn every_table_row_has_a_case() {
    for key in KEYS {
        assert!(
            CASES
                .iter()
                .any(|(name, ..)| property_key_for_name(name) == Some(*key)),
            "no case for {key:?}"
        );
    }
    for (name, ..) in CASES {
        let key = property_key_for_name(name).unwrap_or_else(|| panic!("{name} has no key"));
        assert!(KEYS.contains(&key), "{name} is not in the table");
    }
}

#[test]
fn the_keywords_resolve_for_every_row() {
    let mut failures = Vec::new();
    for &(name, inherited, values) in CASES {
        let (parent_value, ua_value) = (values[0], values[1]);
        let parent = format!("{name}: {parent_value}");
        let ua_sheet = format!("p {{ {name}: {ua_value} }}");
        let user_agent = [(ua_sheet.as_str(), Origin::UserAgent)];
        let from_parent = child(
            &parent,
            &[],
            &format!("#target {{ {name}: {parent_value} }}"),
        );
        // A non-inherited property is initial on a child that does not declare
        // it, whatever the parent has; keeping the parent also keeps what a
        // parent's value does to its children (a multicol parent, say).
        let initial = if inherited {
            child("", &[], "")
        } else {
            child(&parent, &[], "")
        };
        let ua = child(&parent, &user_agent, "");
        if same(&from_parent, &initial) {
            failures.push(format!("{name}: the parent's value is the initial one"));
            continue;
        }
        for before in values {
            let start = format!("#target {{ {name}: {before}; {name}: ");
            let run = |sheets: &[(&str, Origin)], keyword: &str| {
                child(&parent, sheets, &format!("{start}{keyword} }}"))
            };
            let unset = if inherited { &from_parent } else { &initial };
            let checks = [
                ("inherit", run(&[], "inherit"), &from_parent),
                ("initial", run(&[], "initial"), &initial),
                ("unset", run(&[], "unset"), unset),
                ("revert", run(&user_agent, "revert"), &ua),
                ("revert-layer", run(&user_agent, "revert-layer"), &ua),
            ];
            for (keyword, actual, expected) in checks {
                if !same(&actual, expected) {
                    failures.push(format!("{name}: {keyword} after `{before}`"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_important_keyword_beats_a_later_normal_declaration() {
    let css = "#target { box-sizing: inherit !important } p { box-sizing: content-box }";
    let inherited = child("box-sizing: border-box", &[], css);
    assert_eq!(inherited.box_sizing, BoxSizing::BorderBox);
}

#[test]
fn a_keyword_substituted_from_var_resolves_like_a_literal() {
    let parent = "box-sizing: border-box; text-transform: uppercase";
    let values = child(
        parent,
        &[],
        "#target { --kw: inherit; box-sizing: var(--kw); text-transform: var(--kw); \
         word-break: var(--missing, initial) }",
    );
    assert_eq!(values.box_sizing, BoxSizing::BorderBox);
    assert_eq!(values.text_transform, TextTransform::Uppercase);
    assert_eq!(values.word_break, WordBreak::Normal);
}

#[test]
fn float_takes_the_computed_none_of_an_absolutely_positioned_parent() {
    // CSS 2.1 §9.7: `float` computes to `none` on an absolutely positioned
    // box, and `inherit` takes the computed value.
    for (parent, expected) in [
        ("position: absolute; float: left", FloatValue::None),
        ("position: fixed; float: right", FloatValue::None),
        ("position: relative; float: left", FloatValue::Left),
        ("float: right", FloatValue::Right),
    ] {
        let values = child(parent, &[], "#target { float: inherit }");
        assert_eq!(values.float, expected, "{parent}");
    }
}
