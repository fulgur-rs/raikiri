//! CSS-wide keywords on the properties of the `keyword_defaults` table (CSS
//! Cascading 4 §7.3 "Explicit Defaulting").

use super::*;
use crate::Atom;
use crate::cascade::keyword_defaults::KEYS;
use crate::computed::ComputedBackgroundLayer;
use crate::property::{
    BoxSizing, FloatValue, PageValue, TextAutospace, TextSpacingTrim, TextTransform, TextWrapMode,
    TextWrapStyle, WhiteSpaceCollapse, WordBreak, property_key_for_name,
};
use crate::resolve::{ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal};

/// A property, whether it is inherited, and values that compute apart: the
/// first is the parent's, the second the user agent's, and a box `before`
/// each keyword declaration may start from takes any of them.
pub(super) const CASES: &[(&str, bool, &[&str])] = &[
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
        "counter-reset",
        false,
        &["item 2 other", "note -1", "section", "reversed(rev) 3"],
    ),
    (
        "counter-increment",
        false,
        &["item 2 other", "note -1", "section"],
    ),
    (
        "counter-set",
        false,
        &["item 5 other 0", "note 3", "section -2"],
    ),
    (
        "quotes",
        true,
        &[
            r#""«" "»" "‹" "›""#,
            r#""“" "”""#,
            "none",
            "auto",
            r#""'" "'""#,
        ],
    ),
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
    (
        "grid-template-columns",
        false,
        &[
            "100px 1fr",
            "[a] 2fr [b] auto",
            "repeat(2, minmax(10px, 1fr)) fit-content(20%)",
            "calc(10% + 4px) minmax(min-content, 2em) repeat(auto-fill, 30px)",
        ],
    ),
    (
        "grid-template-rows",
        false,
        &[
            "50% auto",
            "max-content [x] 1fr",
            "repeat(3, 20px)",
            "minmax(5%, max-content) calc(2em + 10%)",
        ],
    ),
    ("grid-template-areas", false, &["\"a b\"", "\"c d\""]),
    (
        "grid-auto-columns",
        false,
        &[
            "100px",
            "minmax(10px, 1fr) auto",
            "fit-content(30%) 2fr",
            "calc(10% + 2px) min-content",
        ],
    ),
    (
        "grid-auto-rows",
        false,
        &[
            "50px 1fr",
            "min-content",
            "calc(10% + 2px) max-content",
            "minmax(auto, 3em)",
        ],
    ),
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
    (
        "background-image",
        false,
        &[
            "linear-gradient(red, blue)",
            "url(a.png)",
            "url(b.png), none",
        ],
    ),
    (
        "background-repeat",
        false,
        &["no-repeat", "repeat-x", "space round, repeat-y"],
    ),
    (
        "background-attachment",
        false,
        &["fixed", "local", "fixed, scroll"],
    ),
    (
        "background-clip",
        false,
        &["padding-box", "content-box", "text"],
    ),
    (
        "background-origin",
        false,
        &["content-box", "border-box", "border-box, padding-box"],
    ),
    (
        "background-position-x",
        false,
        &["10px", "30%", "right 5px, center"],
    ),
    (
        "background-position-y",
        false,
        &["20px", "40%", "bottom 6px, center"],
    ),
    (
        "background-size",
        false,
        &["cover", "contain", "20px 30%, auto 5px"],
    ),
    ("outline-style", false, &["dashed", "dotted", "double"]),
    ("outline-color", false, &["red", "blue", "green"]),
    ("outline-offset", false, &["3px", "5px", "-2px"]),
    ("text-decoration-color", false, &["red", "blue", "green"]),
    ("text-emphasis-color", true, &["red", "blue", "green"]),
    (
        "white-space-collapse",
        true,
        &["preserve", "preserve-breaks", "break-spaces"],
    ),
    ("text-wrap-mode", true, &["nowrap", "wrap", "nowrap"]),
];

/// The shorthands over properties of the table, with the same three kinds of
/// value as [`CASES`]. Each shorthand sets every longhand it expands to, so a
/// keyword on it leaves nothing of an earlier value behind.
pub(super) const SHORTHAND_CASES: &[(&str, bool, &[&str])] = &[
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
    (
        "grid-template",
        false,
        &[
            "100px 1fr / 50px 2fr",
            "\"a b\" 40px \"c d\" 1fr / auto 1fr",
            "20% auto / repeat(2, 10px)",
        ],
    ),
    (
        "grid",
        false,
        &[
            "100px 1fr / 50px 2fr",
            "\"a b\" 40px \"c d\" 1fr / auto 1fr",
            "auto-flow dense 30px / 1fr 2fr",
        ],
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
        "background-position",
        false,
        &["10px 20px", "30% 40%", "right 5px bottom 6px, center"],
    ),
    (
        "background",
        false,
        &[
            "url(a.png) no-repeat fixed 10px 20px / cover red",
            "linear-gradient(red, blue) padding-box content-box blue, url(b.png)",
            "green",
        ],
    ),
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
    (
        "text-wrap",
        true,
        &["nowrap balance", "wrap pretty", "stable"],
    ),
    (
        "text-spacing",
        true,
        &[
            "space-all no-autospace",
            "trim-start",
            "trim-all ideograph-alpha",
        ],
    ),
];

/// The computed values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under `sheets`, followed by `css` as an author sheet.
pub(super) fn child(parent: &str, sheets: &[(&str, Origin)], css: &str) -> ComputedValues {
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

pub(super) fn same(left: &ComputedValues, right: &ComputedValues) -> bool {
    format!("{left:?}") == format!("{right:?}")
}

/// Rows with a test of their own: the computed value depends on another
/// property, so a case that sets only this one cannot tell the keywords apart.
pub(super) const SEPARATE_TESTS: &[&str] = &["outline-width", "white-space"];

/// Properties that drop `revert` and `revert-layer`: the user agent sets
/// `white-space` whole, so these have no cascade slot to roll back to.
pub(super) const NO_ROLLBACK: &[&str] = &["white-space-collapse", "text-wrap-mode", "text-wrap"];

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
        let mut checks = vec![
            ("inherit", run(&[], "inherit"), &from_parent),
            ("initial", run(&[], "initial"), &initial),
            ("unset", run(&[], "unset"), unset),
        ];
        if !NO_ROLLBACK.contains(&name) {
            checks.push(("revert", run(&user_agent, "revert"), &ua));
            checks.push(("revert-layer", run(&user_agent, "revert-layer"), &ua));
        }
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
        let longhands: Vec<(PropertyKey, &str)> = match classify(key) {
            KeyClass::Shorthand { longhands } => longhands
                .iter()
                .map(|&longhand| {
                    // The projection of a shorthand's keyword onto a longhand
                    // renames the marker, so the longhand needs a name that
                    // maps back to it.
                    let longhand_name = keyword_longhand_name(longhand)
                        .unwrap_or_else(|| panic!("{name}: {longhand:?} has no name"));
                    (longhand, longhand_name)
                })
                .collect(),
            // A shorthand the cascade keeps whole expands a keyword into
            // longhands named by the expansion itself.
            KeyClass::Retained => crate::rule::retained_keyword_longhands(&DeferredValue {
                property: name.into(),
                value: "inherit".into(),
                key,
            })
            .unwrap_or_else(|| panic!("{name} has no longhands for a keyword"))
            .to_vec(),
            KeyClass::Longhand => panic!("{name} is not a shorthand"),
        };
        for (longhand, longhand_name) in longhands {
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
fn a_background_keyword_takes_one_item_per_layer_of_the_parent() {
    let parent = "background-image: url(a.png), url(b.png); \
                  background-repeat: no-repeat, repeat-x; \
                  background-position: 10px 20px, 30% 40%; \
                  background-size: cover, 5px auto; \
                  background-attachment: fixed, local; \
                  background-clip: padding-box, content-box; \
                  background-origin: content-box, border-box";
    let parent_style = child("", &[], &format!("#target {{ {parent} }}"));
    let inherited = child(
        parent,
        &[],
        "#target { background-image: url(c.png), url(d.png); background-repeat: inherit; \
         background-position: inherit; background-size: inherit; \
         background-attachment: inherit; background-clip: inherit; background-origin: inherit }",
    );
    let (parent_layers, layers) = (
        parent_style.background_layers(),
        inherited.background_layers(),
    );
    assert_eq!(layers.len(), 2);
    for (layer, parent_layer) in layers.iter().zip(parent_layers) {
        assert_eq!(layer.repeat, parent_layer.repeat);
        assert_eq!(layer.position, parent_layer.position);
        assert_eq!(layer.size, parent_layer.size);
        assert_eq!(layer.attachment, parent_layer.attachment);
        assert_eq!(layer.clip, parent_layer.clip);
        assert_eq!(layer.origin, parent_layer.origin);
    }
    // The child's own images stay, and `background: inherit` takes them too.
    assert_ne!(layers[0].image, parent_layers[0].image);
    let whole = child(parent, &[], "#target { background: inherit }");
    assert_eq!(whole.background_layers(), parent_style.background_layers());
}

#[test]
fn a_background_keyword_takes_the_whole_list_of_the_parent() {
    // The lists of the parent are not cut to its three images: the child's
    // five images repeat the parent's short lists from their start, and the
    // list that is longer than the parent's images keeps its last items.
    let lists = "background-repeat: no-repeat, repeat-x; \
                 background-attachment: fixed; \
                 background-position-x: 1px, 2px, 3px, 4px, 5px; \
                 background-position-y: 10%, 20%; \
                 background-size: 6px, 7px auto; \
                 background-clip: padding-box, content-box; \
                 background-origin: content-box, border-box, padding-box, content-box";
    let parent = format!("background-image: url(a.png), url(b.png), url(c.png); {lists}");
    let images = "background-image: url(d.png), url(e.png), url(f.png), url(g.png), url(h.png)";
    let inherited = child(
        &parent,
        &[],
        &format!(
            "#target {{ {images}; background-repeat: inherit; background-attachment: inherit; \
             background-position-x: inherit; background-position-y: inherit; \
             background-size: inherit; background-clip: inherit; background-origin: inherit }}"
        ),
    );
    let authored = child("", &[], &format!("#target {{ {images}; {lists} }}"));
    assert_eq!(inherited.background_layers().len(), 5);
    assert_eq!(inherited.background_layers(), authored.background_layers());
}

#[test]
fn a_background_list_survives_a_node_with_another_number_of_images() {
    // The middle node has three images and the lists of the outer one, which
    // are two items long; the node below it, with four images, repeats the
    // original two items rather than the three layers the middle node shows.
    let mut doc = TestDoc::new();
    let outer = doc.push_element(
        0,
        "div",
        Some("background-image: url(a.png), url(b.png); background-repeat: no-repeat, repeat-x"),
    );
    let middle = doc.push_element(
        outer,
        "div",
        Some(
            "background-image: url(c.png), url(d.png), url(e.png); \
             background-repeat: inherit",
        ),
    );
    let inner = doc.push_element(
        middle,
        "div",
        Some(
            "background-image: url(f.png), url(g.png), url(h.png), url(i.png); \
             background-repeat: inherit",
        ),
    );
    let computed = cascade(&doc, &RuleTree::empty()).unwrap().computed;
    let repeats = |id: usize| -> Vec<_> {
        computed[id]
            .background_layers()
            .iter()
            .map(|layer| layer.repeat)
            .collect()
    };
    let (no_repeat, repeat_x) = (repeats(outer)[0], repeats(outer)[1]);
    assert_ne!(no_repeat, repeat_x);
    assert_eq!(repeats(middle), [no_repeat, repeat_x, no_repeat]);
    assert_eq!(repeats(inner), [no_repeat, repeat_x, no_repeat, repeat_x]);
}

#[test]
fn a_background_list_is_kept_only_where_the_layers_do_not_say_it_all() {
    let images = |count: usize| {
        let urls: Vec<String> = (0..count)
            .map(|index| format!("url({index}.png)"))
            .collect();
        format!("background-image: {}", urls.join(", "))
    };
    let kept = |count: usize, repeat: &str| {
        let css = format!(
            "#target {{ {}; background-repeat: {repeat} }}",
            images(count)
        );
        child("", &[], &css).background_lists.is_some()
    };
    // One value throughout, one item per layer, and a list that repeats
    // evenly over the layers read the same from the layers as from the list.
    assert!(!kept(3, "no-repeat"));
    assert!(!kept(3, "no-repeat, repeat-x, repeat-y"));
    assert!(!kept(4, "no-repeat, repeat-x"));
    assert!(!kept(3, "no-repeat, no-repeat"));
    // A list that the layers cut short, or fill unevenly, is read back whole.
    assert!(kept(3, "no-repeat, repeat-x"));
    assert!(kept(2, "no-repeat, repeat-x, repeat-y"));
}

#[test]
fn an_initial_background_keyword_is_one_initial_layer() {
    let parent = "background-image: url(a.png), url(b.png); background-repeat: no-repeat";
    let values = child(
        parent,
        &[],
        "#target { background-image: url(c.png), url(d.png); background-repeat: initial }",
    );
    assert_eq!(values.background_layers().len(), 2);
    assert!(
        values
            .background_layers()
            .iter()
            .all(|layer| layer.repeat == ComputedBackgroundLayer::initial().repeat)
    );
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
fn quotes_inherit_keeps_the_auto_of_a_parent_that_never_set_it() {
    // `auto` is the initial value, held as an empty list with a flag. A
    // keyword that copied only the list would turn it into `none`, which
    // shows no quote marks at all.
    let plain = child("", &[], "");
    assert!(plain.quotes_auto);
    let inherited = child("", &[], r#"#target { quotes: "a" "b"; quotes: inherit }"#);
    assert!(same(&inherited, &plain));
    let unset = child("", &[], r#"#target { quotes: "a" "b"; quotes: unset }"#);
    assert!(same(&unset, &plain));
}

#[test]
fn quotes_none_and_auto_stay_apart_through_the_keywords() {
    let none = child("quotes: none", &[], "");
    assert!(none.quotes.is_empty() && !none.quotes_auto);
    let inherited = child(
        "quotes: none",
        &[],
        "#target { quotes: auto; quotes: inherit }",
    );
    assert!(inherited.quotes.is_empty() && !inherited.quotes_auto);
    let initial = child(
        "quotes: none",
        &[],
        "#target { quotes: none; quotes: initial }",
    );
    assert!(initial.quotes.is_empty() && initial.quotes_auto);
    let auto = child(r#"quotes: "a" "b""#, &[], "#target { quotes: auto }");
    assert!(auto.quotes.is_empty() && auto.quotes_auto);
}

#[test]
fn a_quotes_keyword_substituted_from_var_resolves_like_a_literal() {
    let parent = r#"quotes: "«" "»""#;
    let initial = child(parent, &[], "#target { --kw: initial; quotes: var(--kw) }");
    assert!(initial.quotes.is_empty() && initial.quotes_auto);
    let inherited = child(parent, &[], "#target { --kw: inherit; quotes: var(--kw) }");
    assert_eq!(inherited.quotes.len(), 1);
    assert!(!inherited.quotes_auto);
}

#[test]
fn quotes_auto_does_not_combine_with_a_list() {
    // The declaration is invalid, so the earlier one stays in force.
    let values = child(
        "",
        &[],
        r#"#target { quotes: "a" "b"; quotes: auto "c" "d" }"#,
    );
    assert_eq!(values.quotes.len(), 1);
    assert!(!values.quotes_auto);
}

#[test]
fn the_legacy_grid_gap_names_take_the_keywords_of_the_gap_properties() {
    let parent = "gap: 3px 4px";
    for (alias, canonical) in [
        ("grid-gap", "gap"),
        ("grid-row-gap", "row-gap"),
        ("grid-column-gap", "column-gap"),
    ] {
        for keyword in ["inherit", "initial", "unset"] {
            let through_alias = child(
                parent,
                &[],
                &format!("#target {{ {canonical}: 9px; {alias}: {keyword} }}"),
            );
            let literal = child(
                parent,
                &[],
                &format!("#target {{ {canonical}: 9px; {canonical}: {keyword} }}"),
            );
            assert!(same(&through_alias, &literal), "{alias}: {keyword}");
        }
    }
}

#[test]
fn the_legacy_grid_gap_names_take_var_and_math_values_like_the_gap_properties() {
    for (alias, canonical, value) in [
        ("grid-gap", "gap", "var(--g)"),
        ("grid-gap", "gap", "calc(1px + 2px) var(--g)"),
        ("grid-row-gap", "row-gap", "var(--g)"),
        ("grid-column-gap", "column-gap", "calc(1px + 2px)"),
    ] {
        let sheet = |name: &str| format!("#target {{ --g: 7px; {name}: {value} }}");
        let through_alias = child("", &[], &sheet(alias));
        let literal = child("", &[], &sheet(canonical));
        assert!(same(&through_alias, &literal), "{alias}: {value}");
        let normal = ComputedLengthPercentageOrNormal::Normal;
        assert!(
            through_alias.row_gap != normal || through_alias.column_gap != normal,
            "{alias}: {value} is applied"
        );
    }
}

/// The `page` values of a parent styled `parent` and of its child `p`
/// (`id="target"`) under the author sheet `css` and the user agent sheet `ua`.
fn page_pair(parent: &str, ua: &str, css: &str) -> (PageValue, PageValue) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some(parent));
    let child = doc.push_element_with_attrs(parent, "p", None, &[("id", "target")]);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(ua, Origin::UserAgent);
    tree.add_stylesheet(css, Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    (
        result.page_values[parent].clone(),
        result.page_values[child].clone(),
    )
}

fn named(name: &str) -> PageValue {
    PageValue::Named(Atom::from(name))
}

#[test]
fn page_inherit_takes_the_page_of_the_parent() {
    let (parent, child) = page_pair("page: wide", "", "#target { page: inherit }");
    assert_eq!((parent, child), (named("wide"), named("wide")));
    let (_, child) = page_pair("page: wide", "", "#target { page: other; page: inherit }");
    assert_eq!(child, named("wide"));
    // `page` is not inherited, so a parent without a declaration holds `auto`.
    let (parent, child) = page_pair("", "", "#target { page: other; page: inherit }");
    assert_eq!((parent, child), (PageValue::Auto, PageValue::Auto));
}

#[test]
fn page_initial_and_unset_are_auto() {
    for keyword in ["initial", "unset"] {
        let (_, child) = page_pair(
            "page: wide",
            "",
            &format!("#target {{ page: other; page: {keyword} }}"),
        );
        assert_eq!(child, PageValue::Auto, "{keyword}");
    }
}

#[test]
fn page_revert_returns_to_the_user_agent_value() {
    let ua = "p { page: from-ua }";
    for keyword in ["revert", "revert-layer"] {
        let (_, child) = page_pair(
            "",
            ua,
            &format!("#target {{ page: other; page: {keyword} }}"),
        );
        assert_eq!(child, named("from-ua"), "{keyword}");
        let (_, child) = page_pair(
            "",
            "",
            &format!("#target {{ page: other; page: {keyword} }}"),
        );
        assert_eq!(
            child,
            PageValue::Auto,
            "{keyword} with nothing to roll back to"
        );
    }
}

#[test]
fn a_page_keyword_substituted_from_var_resolves_like_a_literal() {
    let (_, child) = page_pair(
        "page: wide",
        "",
        "#target { --kw: inherit; page: var(--kw) }",
    );
    assert_eq!(child, named("wide"));
    let (_, child) = page_pair(
        "page: wide",
        "",
        "#target { page: other; --kw: initial; page: var(--kw) }",
    );
    assert_eq!(child, PageValue::Auto);
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

/// The effective `white-space-collapse` and `text-wrap-mode` of the child `p`
/// of a `div` styled `parent`, under `css` as an author sheet. `white-space`
/// and its longhands settle them by cascade order.
fn white_space_pair(
    parent: &str,
    sheets: &[(&str, Origin)],
    css: &str,
) -> (WhiteSpaceCollapse, TextWrapMode) {
    let values = child(parent, sheets, &format!("#target {{ {css} }}"));
    (
        values.effective_white_space_collapse,
        values.effective_text_wrap_mode,
    )
}

#[test]
fn white_space_keywords_take_both_halves_of_the_parent() {
    use TextWrapMode::{Nowrap, Wrap};
    use WhiteSpaceCollapse::{Collapse, Preserve};
    // `pre` is `preserve` and `nowrap`, so both halves differ from initial.
    let pair = |css: &str| white_space_pair("white-space: pre", &[], css);
    assert_eq!(
        pair("white-space: nowrap; white-space: inherit"),
        (Preserve, Nowrap)
    );
    assert_eq!(
        pair("white-space: pre-wrap; white-space: initial"),
        (Collapse, Wrap)
    );
    // `white-space` is inherited, so `unset` is `inherit`.
    assert_eq!(
        pair("white-space: normal; white-space: unset"),
        (Preserve, Nowrap)
    );
    // A keyword on the parent's own `white-space` longhands reads the same.
    assert_eq!(
        white_space_pair(
            "white-space-collapse: preserve-breaks; text-wrap-mode: nowrap",
            &[],
            "white-space: normal; white-space: inherit"
        ),
        (WhiteSpaceCollapse::PreserveBreaks, Nowrap)
    );
}

#[test]
fn a_white_space_keyword_competes_with_the_longhands_in_cascade_order() {
    use TextWrapMode::{Nowrap, Wrap};
    use WhiteSpaceCollapse::{Collapse, Preserve};
    // `pre-wrap` is `preserve` and `wrap`.
    let pair = |css: &str| white_space_pair("white-space: pre-wrap", &[], css);
    // The later declaration decides each half, whichever of the legacy
    // keyword, the longhands and the CSS-wide keywords it is.
    assert_eq!(
        pair("white-space: inherit; white-space: nowrap"),
        (Collapse, Nowrap)
    );
    assert_eq!(
        pair("white-space: nowrap; white-space-collapse: inherit"),
        (Preserve, Nowrap)
    );
    assert_eq!(
        pair("white-space: nowrap; text-wrap-mode: inherit"),
        (Collapse, Wrap)
    );
    assert_eq!(
        pair("white-space-collapse: inherit; white-space: normal"),
        (Collapse, Wrap)
    );
    assert_eq!(
        pair("text-wrap-mode: initial; white-space-collapse: initial; white-space: inherit"),
        (Preserve, Wrap)
    );
    // An important keyword beats a later normal `white-space`.
    assert_eq!(
        pair("white-space: inherit !important; white-space: nowrap"),
        (Preserve, Wrap)
    );
    // A keyword of the longhands alone leaves the other half to the legacy
    // keyword declared before it.
    assert_eq!(
        pair("white-space: pre; white-space-collapse: initial"),
        (Collapse, Nowrap)
    );
}

#[test]
fn a_white_space_rollback_returns_to_the_user_agent_keyword() {
    use TextWrapMode::Nowrap;
    use WhiteSpaceCollapse::Preserve;
    // The user agent sets `white-space` whole, so the rollback keeps the
    // shorthand in its own cascade slot instead of expanding it.
    let user_agent = [("p { white-space: pre }", Origin::UserAgent)];
    for keyword in ["revert", "revert-layer"] {
        assert_eq!(
            white_space_pair(
                "",
                &user_agent,
                &format!("white-space: normal; white-space: {keyword}")
            ),
            (Preserve, Nowrap),
            "{keyword}"
        );
    }
}

#[test]
fn text_wrap_keywords_take_the_mode_and_the_style_of_the_parent() {
    let parent = "text-wrap: nowrap balance";
    let values = |css: &str| child(parent, &[], &format!("#target {{ {css} }}"));
    let inherited = values("text-wrap: wrap pretty; text-wrap: inherit");
    assert_eq!(inherited.effective_text_wrap_mode, TextWrapMode::Nowrap);
    assert_eq!(inherited.text_wrap_style, TextWrapStyle::Balance);
    let initial = values("text-wrap: nowrap pretty; text-wrap: initial");
    assert_eq!(initial.effective_text_wrap_mode, TextWrapMode::Wrap);
    assert_eq!(initial.text_wrap_style, TextWrapStyle::Auto);
    // The style longhand and the shorthand compete in cascade order.
    let style_last = values("text-wrap: inherit; text-wrap-style: pretty");
    assert_eq!(style_last.effective_text_wrap_mode, TextWrapMode::Nowrap);
    assert_eq!(style_last.text_wrap_style, TextWrapStyle::Pretty);
    let keyword_last = values("text-wrap-style: pretty; text-wrap: inherit");
    assert_eq!(keyword_last.text_wrap_style, TextWrapStyle::Balance);
    // `text-wrap-mode` is only the mode.
    let mode_only = values("text-wrap-style: pretty; text-wrap-mode: inherit");
    assert_eq!(mode_only.effective_text_wrap_mode, TextWrapMode::Nowrap);
    assert_eq!(mode_only.text_wrap_style, TextWrapStyle::Pretty);
}

#[test]
fn text_spacing_keywords_take_the_trim_and_the_autospace_of_the_parent() {
    let parent = "text-spacing: space-all no-autospace";
    let values = |css: &str| child(parent, &[], &format!("#target {{ {css} }}"));
    let inherited = values("text-spacing: trim-start; text-spacing: inherit");
    assert_eq!(inherited.text_spacing_trim, TextSpacingTrim::SpaceAll);
    assert_eq!(inherited.text_autospace, TextAutospace::NoAutospace);
    let initial = values("text-spacing: trim-start no-autospace; text-spacing: initial");
    assert_eq!(initial.text_spacing_trim, TextSpacingTrim::Normal);
    assert_eq!(initial.text_autospace, TextAutospace::Normal);
    // A later longhand keeps its own value next to the shorthand keyword.
    let longhand_last = values("text-spacing: initial; text-spacing-trim: trim-all");
    assert_eq!(longhand_last.text_spacing_trim, TextSpacingTrim::TrimAll);
    assert_eq!(longhand_last.text_autospace, TextAutospace::Normal);
}

#[test]
fn a_keyword_substituted_from_var_resolves_white_space_through_its_legacy_keyword() {
    use TextWrapMode::{Nowrap, Wrap};
    use WhiteSpaceCollapse::{Collapse, Preserve};
    // A `white-space` value written with `var()` stays one declaration, so a
    // keyword it substitutes is the legacy keyword that stands for the
    // effective halves of the source.
    let pair = |keyword: &str| {
        white_space_pair(
            "white-space: pre",
            &[],
            &format!("--kw: {keyword}; white-space: nowrap; white-space: var(--kw)"),
        )
    };
    assert_eq!(pair("inherit"), (Preserve, Nowrap));
    assert_eq!(pair("initial"), (Collapse, Wrap));
    assert_eq!(pair("unset"), (Preserve, Nowrap));
}

#[test]
fn a_white_space_keyword_keeps_the_legacy_keyword_in_step() {
    use crate::property::WhiteSpace;
    let legacy =
        |parent: &str, css: &str| child(parent, &[], &format!("#target {{ {css} }}")).white_space;
    assert_eq!(
        legacy("white-space: pre", "white-space: inherit"),
        WhiteSpace::Pre
    );
    assert_eq!(
        legacy("white-space: pre", "white-space: initial"),
        WhiteSpace::Normal
    );
    // The longhands can combine in a way no keyword names; the keyword then
    // stays as the parent authored it.
    assert_eq!(
        legacy(
            "white-space: pre-wrap; white-space-collapse: discard",
            "white-space: inherit"
        ),
        WhiteSpace::PreWrap
    );
}

#[test]
fn a_keyword_substituted_from_var_names_both_longhands_of_text_wrap_and_text_spacing() {
    // Both shorthands stay one declaration when written with `var()`, so the
    // keyword they substitute has to reset the longhands the literal one does.
    let parent = "text-wrap: nowrap balance; text-spacing: space-all no-autospace";
    for keyword in ["inherit", "initial", "unset"] {
        let through_var = child(
            parent,
            &[],
            &format!(
                "#target {{ --kw: {keyword}; text-wrap: var(--kw); text-spacing: var(--kw) }}"
            ),
        );
        let literal = child(
            parent,
            &[],
            &format!(
                "#target {{ --kw: {keyword}; text-wrap: {keyword}; text-spacing: {keyword} }}"
            ),
        );
        assert!(same(&through_var, &literal), "{keyword}");
    }
}

#[test]
fn only_the_border_side_longhands_have_a_keyword_variant() {
    use crate::property::{CssWideKeyword, PropertyKey, PropertyValue};
    use crate::rule::border_side_css_wide;
    assert_eq!(
        border_side_css_wide(PropertyKey::BorderLeftWidth, CssWideKeyword::Initial),
        Some(PropertyValue::BorderLeftWidthCssWide(
            CssWideKeyword::Initial
        ))
    );
    assert_eq!(
        border_side_css_wide(PropertyKey::Width, CssWideKeyword::Inherit),
        None
    );
}
