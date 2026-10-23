//! CSS-wide keywords on the properties of the `keyword_defaults` table (CSS
//! Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::cascade::keyword_defaults::KEYS;
use crate::property::{BoxSizing, FloatValue, TextTransform, WordBreak, property_key_for_name};
use crate::resolve::{ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal};

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
    ("flex-basis", false, &["30px", "40%", "calc(10% + 4px)"]),
    ("row-gap", false, &["3px", "5%", "calc(10% + 4px)"]),
    ("column-gap", false, &["3px", "5%", "calc(10% + 4px)"]),
    ("column-width", false, &["30px", "50px", "auto"]),
    ("overflow-x", false, &["hidden", "scroll", "auto"]),
    ("overflow-y", false, &["hidden", "scroll", "auto"]),
    (
        "text-indent",
        true,
        &["10px", "5%", "2em hanging each-line"],
    ),
    (
        "text-underline-offset",
        true,
        &["3px", "20%", "calc(10% + 4px)"],
    ),
    (
        "text-decoration-thickness",
        false,
        &["3px", "from-font", "5px"],
    ),
    ("text-decoration-inset", false, &["3px 4px", "auto", "5px"]),
    ("content", false, &["\"a\"", "\"b\"", "\"c\""]),
    (
        "string-set",
        false,
        &["title \"a\"", "title \"b\"", "name \"c\""],
    ),
    (
        "box-shadow",
        false,
        &["1px 2px 3px 4px red", "inset 2px 2px blue", "5px 5px green"],
    ),
    (
        "object-position",
        false,
        &["10px 20px", "30% 40%", "right 5px bottom 6px"],
    ),
    (
        "transform",
        false,
        &[
            "translate(10px, 20%)",
            "rotate(45deg) scale(2)",
            "skewX(10deg)",
        ],
    ),
    (
        "transform-origin",
        false,
        &["10px 20px", "30% 40%", "right bottom 8px"],
    ),
    ("outline-style", false, &["dashed", "dotted", "double"]),
    ("outline-color", false, &["red", "blue", "green"]),
    ("outline-offset", false, &["3px", "5px", "-2px"]),
    ("text-decoration-color", false, &["red", "blue", "green"]),
    ("text-emphasis-color", true, &["red", "blue", "green"]),
];

/// The shorthands over properties of the table, with the same three kinds of
/// value as [`CASES`]. Each shorthand sets every longhand it expands to, so a
/// keyword on it leaves nothing of an earlier value behind.
const SHORTHAND_CASES: &[(&str, bool, &[&str])] = &[
    (
        "font",
        true,
        &[
            "italic small-caps bold 20px/2 serif",
            "oblique 14px/30px monospace",
            "12px sans-serif",
        ],
    ),
    ("flex", false, &["2 3 10px", "1 1 40%", "0 0 content"]),
    (
        "flex-flow",
        false,
        &["column wrap", "row-reverse wrap-reverse", "column-reverse"],
    ),
    (
        "gap",
        false,
        &["3px 4px", "5% 6%", "calc(10% + 4px) normal"],
    ),
    (
        "place-content",
        false,
        &["center end", "space-between start", "flex-end center"],
    ),
    (
        "place-items",
        false,
        &["center end", "start center", "flex-end baseline"],
    ),
    (
        "place-self",
        false,
        &["center end", "start center", "flex-end baseline"],
    ),
    ("grid-row", false, &["2 / 4", "span 2 / 5", "3 / span 2"]),
    ("grid-column", false, &["2 / 4", "span 2 / 5", "3 / span 2"]),
    (
        "overflow",
        false,
        &["hidden scroll", "scroll", "clip hidden"],
    ),
    ("columns", false, &["30px 3", "50px 2", "auto 4"]),
    (
        "outline",
        false,
        &["4px solid red", "2px dashed blue", "3px dotted green"],
    ),
    (
        "text-decoration",
        false,
        &[
            "underline wavy red",
            "overline dotted blue",
            "line-through double green",
        ],
    ),
    (
        "text-emphasis",
        true,
        &["dot red", "circle blue", "sesame green"],
    ),
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

/// Rows with a test of their own: the computed value depends on another
/// property, so a case that sets only this one cannot tell the keywords apart.
const SEPARATE_TESTS: &[&str] = &["outline-width"];

#[test]
fn every_table_row_has_a_case() {
    for key in KEYS {
        assert!(
            CASES
                .iter()
                .map(|(name, ..)| name)
                .chain(SEPARATE_TESTS)
                .any(|name| property_key_for_name(name) == Some(*key)),
            "no case for {key:?}"
        );
    }
    for (name, ..) in CASES {
        let key = property_key_for_name(name).unwrap_or_else(|| panic!("{name} has no key"));
        assert!(KEYS.contains(&key), "{name} is not in the table");
    }
}

/// The failures of the five keywords on the property `name` after each of
/// `values`, compared with the value that rule gives them (see [`CASES`]).
fn keyword_failures(name: &str, inherited: bool, values: &[&str]) -> Vec<String> {
    let mut failures = Vec::new();
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
        return failures;
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
    failures
}

#[test]
fn the_keywords_resolve_for_every_row() {
    let failures: Vec<String> = CASES
        .iter()
        .flat_map(|&(name, inherited, values)| keyword_failures(name, inherited, values))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_keywords_resolve_on_the_shorthands() {
    let failures: Vec<String> = SHORTHAND_CASES
        .iter()
        .flat_map(|&(name, inherited, values)| keyword_failures(name, inherited, values))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_longhand_of_a_shorthand_case_resolves_the_keyword() {
    use crate::cascade::custom_property::keyword_longhand_name;
    use crate::cascade::inherit::resolve_defaulting_value;
    use crate::property::{DeferredValue, PropertyKey, PropertyValue};
    use crate::rule::{KeyClass, classify};

    let initial = ComputedValues::initial();
    // A key outside the shorthands has no such name.
    assert_eq!(keyword_longhand_name(PropertyKey::Color), None);
    for &(name, ..) in SHORTHAND_CASES {
        let key = property_key_for_name(name).unwrap_or_else(|| panic!("{name} has no key"));
        let KeyClass::Shorthand { longhands } = classify(key) else {
            panic!("{name} is not expanded into longhands");
        };
        for &longhand in longhands {
            // The projection of a shorthand's keyword onto a longhand renames
            // the marker, so the longhand needs a name that maps back to it.
            let longhand_name = keyword_longhand_name(longhand)
                .unwrap_or_else(|| panic!("{name}: {longhand:?} has no name"));
            assert_eq!(property_key_for_name(longhand_name), Some(longhand));
            let marker = PropertyValue::Deferred(DeferredValue {
                property: longhand_name.into(),
                value: "inherit".into(),
                key: longhand,
            });
            assert!(
                !matches!(
                    resolve_defaulting_value(marker, &initial),
                    PropertyValue::Deferred(_)
                ),
                "{name}: {longhand:?} keeps its keyword marker"
            );
        }
    }
}

#[test]
fn outline_width_keywords_take_the_computed_width() {
    // A width computes to zero while the style is `none`, so the parent
    // needs a visible outline for the keyword to carry a width at all.
    let visible = "outline-width: 4px; outline-style: solid";
    let width = |parent: &str, sheets: &[(&str, Origin)], css: &str| {
        child(parent, sheets, css).outline.width().0
    };
    let with =
        |keyword: &str| format!("#target {{ outline-style: dashed; outline-width: {keyword} }}");
    assert_eq!(width(visible, &[], &with("inherit")), 4.0);
    // `medium`, the initial width, is 3px.
    assert_eq!(width(visible, &[], &with("initial")), 3.0);
    assert_eq!(width(visible, &[], &with("unset")), 3.0);
    // The parent's computed width is zero, and that is what `inherit` takes.
    assert_eq!(width("outline-width: 4px", &[], &with("inherit")), 0.0);
    // Without a style of its own, the initial width still computes to zero.
    assert_eq!(
        width(
            visible,
            &[],
            "#target { outline-width: 6px; outline-width: initial }"
        ),
        0.0
    );
    // `revert` and `revert-layer` roll back to the user agent's width.
    let user_agent = [("p { outline-width: 7px }", Origin::UserAgent)];
    for keyword in ["revert", "revert-layer"] {
        assert_eq!(
            width(visible, &user_agent, &with(keyword)),
            7.0,
            "{keyword}"
        );
    }
    // A keyword substituted from `var()` resolves like a literal one.
    for keyword in ["inherit", "initial", "unset"] {
        let through_var = format!(
            "#target {{ --kw: {keyword}; outline-style: dashed; outline-width: var(--kw) }}"
        );
        assert_eq!(
            width(visible, &[], &through_var),
            width(visible, &[], &with(keyword)),
            "{keyword}"
        );
    }
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
fn a_keyword_substituted_from_var_resolves_on_a_shorthand() {
    let parent = "gap: 3px 4px; flex: 2 3 10px; font: italic 20px/2 serif; overflow: hidden scroll; \
                  outline: 4px solid red; text-decoration: underline wavy red; \
                  text-emphasis: dot red";
    for keyword in ["inherit", "initial", "unset"] {
        let through_var = child(
            parent,
            &[],
            &format!(
                "#target {{ --kw: {keyword}; gap: var(--kw); flex: var(--kw); font: var(--kw); \
                 overflow: var(--kw); outline: var(--kw); text-decoration: var(--kw); \
                 text-emphasis: var(--kw) }}"
            ),
        );
        let literal = child(
            parent,
            &[],
            &format!(
                "#target {{ --kw: {keyword}; gap: {keyword}; flex: {keyword}; \
                 font: {keyword}; overflow: {keyword}; outline: {keyword}; \
                 text-decoration: {keyword}; text-emphasis: {keyword} }}"
            ),
        );
        assert!(same(&through_var, &literal), "{keyword}");
    }
}

#[test]
fn an_important_keyword_on_a_shorthand_beats_a_later_longhand() {
    let css = "#target { gap: inherit !important } p { row-gap: 9px }";
    let values = child("gap: 3px 4px", &[], css);
    assert_eq!(values.row_gap, ComputedLengthPercentageOrNormal::Px(3.0));
    assert_eq!(values.column_gap, ComputedLengthPercentageOrNormal::Px(4.0));
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

#[test]
fn a_min_block_size_keyword_competes_with_min_height_in_cascade_order() {
    let parent = "min-height: 30px";
    let physical_last = child(
        parent,
        &[],
        "#target { min-block-size: inherit; min-height: 10px }",
    );
    assert_eq!(
        physical_last.min_height,
        ComputedLengthPercentageOrAuto::Px(10.0)
    );
    assert_eq!(physical_last.min_block_size, None);
    let keyword_last = child(
        parent,
        &[],
        "#target { min-height: 10px; min-block-size: inherit }",
    );
    assert_eq!(
        keyword_last.min_height,
        ComputedLengthPercentageOrAuto::Px(30.0)
    );
}

#[test]
fn inherit_keeps_the_keyword_values_of_the_text_decoration_lengths() {
    use crate::resolve::{ComputedTextDecorationInset, ComputedTextDecorationThickness};
    // `from-font` and `auto` are not lengths: the lift back to the specified
    // value must not turn them into one.
    let values = child(
        "text-decoration-thickness: from-font; text-decoration-inset: auto",
        &[],
        "#target { text-decoration-thickness: inherit; text-decoration-inset: inherit }",
    );
    assert_eq!(
        values.text_decoration_thickness,
        ComputedTextDecorationThickness::FromFont
    );
    assert_eq!(
        values.text_decoration_inset,
        ComputedTextDecorationInset::Auto
    );
}
