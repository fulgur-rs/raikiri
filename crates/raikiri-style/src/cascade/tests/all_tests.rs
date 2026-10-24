//! The `all` shorthand with CSS-wide keywords (CSS Cascading 4 section 3.1).

use super::keyword_default_tests::{CASES, NO_ROLLBACK, SHORTHAND_CASES, child, same};
use super::*;
use crate::property::{Direction, TextWrapMode, UnicodeBidi, WhiteSpace, WhiteSpaceCollapse};

/// Properties `all` does not reset (CSS Cascade 4 section 3.1), and the
/// white space longhands, which the keyword on `all` also settles on the
/// `white-space` shorthand they combine into; a test of their own follows.
const NOT_COMPARED: &[&str] = &[
    "direction",
    "unicode-bidi",
    "white-space-collapse",
    "text-wrap-mode",
    "text-wrap",
];

/// The failures of `all: <keyword>` on the property `name`, compared with the
/// keyword on the property alone. `values` are those of a [`CASES`] row: the
/// first is the parent's, the second the user agent's.
fn all_failures(name: &str, values: &[&str]) -> Vec<String> {
    let mut failures = Vec::new();
    if NOT_COMPARED.contains(&name) {
        return failures;
    }
    let parent = format!("{name}: {}", values[0]);
    let ua_sheet = format!("p {{ {name}: {} }}", values[1]);
    let user_agent = [(ua_sheet.as_str(), Origin::UserAgent)];
    for before in values {
        for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
            let rolls_back = keyword.starts_with("revert");
            if rolls_back && NO_ROLLBACK.contains(&name) {
                continue;
            }
            let sheets: &[(&str, Origin)] = if rolls_back { &user_agent } else { &[] };
            let run = |css: String| child(&parent, sheets, &css);
            // A keyword after the property's own value takes the value away.
            let via_all = run(format!("#target {{ {name}: {before}; all: {keyword} }}"));
            let via_property = run(format!("#target {{ {name}: {before}; {name}: {keyword} }}"));
            if !same(&via_all, &via_property) {
                failures.push(format!("{name}: all: {keyword} after `{before}`"));
            }
            // The property's own value after the keyword wins, unless the
            // value is no valid one for the property.
            let after = run(format!("#target {{ all: {keyword}; {name}: {before} }}"));
            let alone = run(format!("#target {{ {name}: {before} }}"));
            if !same(&alone, &run(String::new())) && !same(&after, &alone) {
                failures.push(format!("{name}: `{before}` after all: {keyword}"));
            }
        }
    }
    failures
}

#[test]
fn all_gives_every_row_of_the_keyword_table_what_the_keyword_gives_the_property() {
    let failures: Vec<String> = CASES
        .iter()
        .flat_map(|&(name, _, values)| all_failures(name, values))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn all_gives_the_shorthands_what_the_keyword_gives_them() {
    let failures: Vec<String> = SHORTHAND_CASES
        .iter()
        .flat_map(|&(name, _, values)| all_failures(name, values))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Properties outside the keyword table that take the keywords, with a value
/// for the parent, one for the user agent and another.
const OTHER_CASES: &[(&str, &[&str])] = &[
    ("color", &["red", "green", "blue"]),
    ("display", &["flex", "block", "none"]),
    ("opacity", &["0.5", "0.25", "0.75"]),
    ("visibility", &["hidden", "collapse", "hidden"]),
    ("font-size", &["20px", "30px", "12px"]),
    ("font-weight", &["bold", "300", "900"]),
    ("font-style", &["italic", "oblique", "italic"]),
    ("font-family", &["serif", "monospace", "cursive"]),
    ("background-color", &["red", "blue", "green"]),
    ("vertical-align", &["top", "middle", "super"]),
    ("line-height", &["2", "3", "1.5"]),
    ("margin-top", &["5px", "7px", "9px"]),
    ("padding-left", &["5px", "7px", "9px"]),
    ("top", &["5px", "7px", "9px"]),
    ("width", &["50px", "60px", "70px"]),
    ("max-height", &["50px", "60px", "70px"]),
    ("border-top-style", &["solid", "dashed", "dotted"]),
    ("border-left-color", &["red", "blue", "green"]),
    ("border-top-left-radius", &["5px", "7px", "9px"]),
    ("text-align", &["center", "right", "justify"]),
    (
        "writing-mode",
        &["vertical-rl", "vertical-lr", "sideways-rl"],
    ),
];

#[test]
fn all_gives_the_other_properties_what_the_keyword_gives_them() {
    let failures: Vec<String> = OTHER_CASES
        .iter()
        .flat_map(|&(name, values)| all_failures(name, values))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn all_does_not_reset_direction_and_unicode_bidi() {
    for keyword in ["initial", "inherit", "unset"] {
        let values = child(
            "direction: ltr",
            &[],
            &format!(
                "#target {{ direction: rtl; unicode-bidi: embed; writing-mode: vertical-rl; \
                 all: {keyword} }}"
            ),
        );
        assert_eq!(values.direction, Direction::Rtl, "{keyword}");
        assert_eq!(values.unicode_bidi, UnicodeBidi::Embed, "{keyword}");
        assert_eq!(
            values.writing_mode,
            ComputedValues::initial().writing_mode,
            "{keyword}"
        );
    }
}

#[test]
fn an_all_keyword_leaves_the_custom_properties_alone() {
    let values = child("--a: 1", &[], "#target { --b: 2; all: initial }");
    assert_eq!(values.custom_properties.get("--a").as_deref(), Some("1"));
    assert_eq!(values.custom_properties.get("--b").as_deref(), Some("2"));
}

fn color_of(parent: &str, sheets: &[(&str, Origin)], css: &str) -> crate::property::CssColor {
    child(parent, sheets, css).color
}

#[test]
fn the_order_of_all_and_a_property_decides() {
    let initial = ComputedValues::initial().color;
    assert_eq!(
        color_of("", &[], "#target { color: red; all: initial }"),
        initial
    );
    assert_eq!(
        color_of("", &[], "#target { all: initial; color: red }"),
        RED
    );
    // Across rules of equal specificity the later one wins.
    assert_eq!(
        color_of("", &[], "p { color: red } p { all: initial }"),
        initial
    );
    assert_eq!(
        color_of("", &[], "p { all: initial } p { color: red }"),
        RED
    );
    // The later of two keywords wins.
    assert_eq!(
        color_of("color: blue", &[], "p { all: inherit } p { all: initial }"),
        initial
    );
    assert_eq!(
        color_of("color: blue", &[], "p { all: initial } p { all: inherit }"),
        BLUE
    );
}

#[test]
fn specificity_decides_between_all_and_a_property() {
    let initial = ComputedValues::initial().color;
    assert_eq!(
        color_of("", &[], "#target { all: initial } p { color: red }"),
        initial
    );
    assert_eq!(
        color_of("", &[], "p { all: initial } #target { color: red }"),
        RED
    );
}

#[test]
fn importance_decides_between_all_and_a_property() {
    let initial = ComputedValues::initial().color;
    assert_eq!(
        color_of(
            "",
            &[],
            "p { color: red !important } #target { all: initial }"
        ),
        RED
    );
    assert_eq!(
        color_of(
            "",
            &[],
            "#target { color: red } p { all: initial !important }"
        ),
        initial
    );
    assert_eq!(
        color_of("", &[], "#target { color: red !important; all: initial }"),
        RED
    );
}

#[test]
fn layers_decide_between_all_and_a_property() {
    let initial = ComputedValues::initial().color;
    for (css, expected) in [
        (
            "@layer a { #target { all: initial } } @layer b { #target { color: red } }",
            RED,
        ),
        (
            "@layer a { #target { color: red } } @layer b { #target { all: initial } }",
            initial,
        ),
        // Declarations outside a layer outrank those in one.
        (
            "@layer a { #target { all: initial } } #target { color: red }",
            RED,
        ),
        (
            "@layer a { #target { color: red } } #target { all: initial }",
            initial,
        ),
    ] {
        assert_eq!(color_of("", &[], css), expected, "{css}");
    }
}

#[test]
fn revert_layer_after_all_returns_to_the_lower_layer() {
    // The rollback takes the layer of the keyword away with it.
    let css = "@layer a { #target { color: red } } \
               @layer b { #target { all: initial } #target { all: revert-layer } }";
    assert_eq!(color_of("", &[], css), RED);
}

#[test]
fn all_revert_returns_to_the_user_agent_style() {
    let ua = "p { color: green; display: block }";
    let user_agent = [(ua, Origin::UserAgent)];
    let reverted = child(
        "",
        &user_agent,
        "#target { color: red; display: flex; all: revert }",
    );
    assert!(same(&reverted, &child("", &user_agent, "")));
    // A declaration after it still wins.
    assert_eq!(
        color_of("", &user_agent, "#target { all: revert; color: red }"),
        RED
    );
}

#[test]
fn a_style_attribute_takes_all_like_a_rule() {
    // The parent's style attribute is the one given.
    let initial = ComputedValues::initial().color;
    assert_eq!(color_of("color: red; all: initial", &[], ""), initial);
    assert_eq!(color_of("all: initial; color: red", &[], ""), RED);
}

#[test]
fn all_reaches_the_pseudo_elements() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("color: red"));
    let child = doc.push_element(parent, "p", None);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p::before { content: 'x'; all: initial } p::after { content: 'y' }",
        Origin::Author,
    );
    let result = cascade(&doc, &tree).unwrap();
    let id = StyleNodeId(child as u64);
    assert_eq!(
        result.pseudo[&(id, PseudoElem::Before)].color,
        ComputedValues::initial().color
    );
    assert_eq!(result.pseudo[&(id, PseudoElem::After)].color, RED);
}

#[test]
fn all_on_first_line_resets_only_the_properties_that_apply_to_it() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("color: red; text-align: center"));
    let child = doc.push_element(parent, "p", None);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p::first-line { all: initial }", Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    let first_line = &result.pseudo[&(StyleNodeId(child as u64), PseudoElem::FirstLine)];
    let initial = ComputedValues::initial();
    assert_eq!(first_line.color, initial.color);
    // `text-align` does not apply to the first line, so it stays inherited.
    assert_eq!(first_line.text_align, result.computed[child].text_align);
    assert_ne!(first_line.text_align, initial.text_align);
}

#[test]
fn all_revert_on_first_line_rolls_the_first_line_back() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("color: blue"));
    let child = doc.push_element(parent, "p", None);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p::first-line { color: red; all: revert }", Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    let first_line = &result.pseudo[&(StyleNodeId(child as u64), PseudoElem::FirstLine)];
    assert_eq!(first_line.color, BLUE);
}

#[test]
fn a_rollback_reaches_the_all_of_the_layer_below_however_many_the_layer_above_has() {
    // Twenty `all` of one layer stand for the strongest of them, so the one in
    // the layer below, which the rollback falls to, is not given up.
    let above = "p { all: inherit } ".repeat(20);
    let css = format!(
        "@layer a, b; @layer a {{ p {{ all: initial }} }} \
         @layer b {{ {above} #target {{ color: revert-layer }} }}"
    );
    assert_eq!(
        color_of("color: blue", &[], &css),
        ComputedValues::initial().color
    );
}

#[test]
fn all_counts_as_a_declaration_of_opacity_and_background_color() {
    let mut doc = TestDoc::new();
    let reset = doc.push_element(0, "p", None);
    let plain = doc.push_element(0, "div", None);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p { all: initial }", Origin::Author);
    let result = cascade(&doc, &tree).unwrap();
    assert!(result.opacity_specified[reset]);
    assert!(result.background_color_specified[reset]);
    assert!(!result.opacity_specified[plain]);
    assert!(!result.background_color_specified[plain]);
}

#[test]
fn all_inherit_and_initial_settle_the_white_space_longhands_together() {
    let parent = "white-space: pre-wrap";
    let inherited = child(parent, &[], "#target { white-space: nowrap; all: inherit }");
    let parent_values = child(parent, &[], "#target { white-space: inherit }");
    assert!(same(&inherited, &parent_values));
    assert_eq!(inherited.white_space, WhiteSpace::PreWrap);
    let initial = child(parent, &[], "#target { white-space: nowrap; all: initial }");
    assert!(same(
        &initial,
        &child(parent, &[], "#target { white-space: initial }")
    ));
    // A longhand declared after the keyword still wins.
    let mode = child(
        parent,
        &[],
        "#target { all: initial; text-wrap-mode: nowrap }",
    );
    assert_eq!(mode.effective_text_wrap_mode, TextWrapMode::Nowrap);
    assert_eq!(
        mode.effective_white_space_collapse,
        WhiteSpaceCollapse::Collapse
    );
}
