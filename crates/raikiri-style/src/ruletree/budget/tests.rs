use super::*;
use crate::ruletree::{Origin, RuleTree};
use crate::test_dom::TestDoc;

fn unlimited() -> RuleTreeLimits {
    RuleTreeLimits {
        max_rules: None,
        max_selectors: None,
        max_declarations: None,
    }
}

/// The rules, selectors and declarations a tree retains from `css`.
fn counts(css: &str) -> [u64; 3] {
    let mut tree = RuleTree::empty().with_limits(unlimited());
    tree.add_stylesheet(css, Origin::Author);
    assert!(tree.limit_exceeded().is_none());
    tree.budget.counts()
}

/// The kind, limit and count of a tree that passed a limit.
fn exceeded(tree: &RuleTree) -> (CascadeLimitKind, u64, u64) {
    match tree.limit_exceeded() {
        Some(CascadeError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => (kind, limit, actual),
        other => panic!("expected a passed limit, got {other:?}"),
    }
}

fn tree_within(css: &str, limits: RuleTreeLimits) -> RuleTree {
    let mut tree = RuleTree::empty().with_limits(limits);
    tree.add_stylesheet(css, Origin::Author);
    tree
}

#[test]
fn style_rules_count_their_selectors_and_expanded_declarations() {
    // `margin` expands to its four longhands; a rule without declarations
    // still counts as a rule.
    assert_eq!(counts("a, b { color: red; margin: 0 }"), [1, 2, 5]);
    assert_eq!(counts("a {}"), [1, 1, 0]);
    // A run of declarations on each side of a nested rule is a rule of its
    // own, with the parent's selectors.
    assert_eq!(
        counts("a, b { color: red; c { color: blue } color: green }"),
        [3, 5, 3]
    );
    assert_eq!(
        counts("::highlight(h) { background-color: red }"),
        [1, 0, 1]
    );
}

#[test]
fn at_rules_count_their_records_layers_and_page_boxes() {
    // The `@media` record, and its rule.
    assert_eq!(counts("@media print { a { color: red } }"), [2, 1, 1]);
    // The statement's record and each name it declares.
    assert_eq!(counts("@layer x, y;"), [3, 0, 0]);
    // The block's record and its layer, and the rule inside.
    assert_eq!(counts("@layer x { a {} }"), [3, 1, 0]);
    // Each segment of a dotted name declares a layer, and so does an
    // anonymous layer block. A nested `@layer` counts only its layers.
    assert_eq!(counts("@layer a.b.c;"), [4, 0, 0]);
    assert_eq!(counts("@layer { a {} }"), [3, 1, 0]);
    assert_eq!(counts("@media print { @layer a.b, c; }"), [4, 0, 0]);
    assert_eq!(counts("a { @layer { color: red } }"), [2, 1, 1]);
    // A group rule nested in another rule is a rule of its own.
    assert_eq!(counts("@media print { @media screen { a {} } }"), [3, 1, 0]);
    assert_eq!(counts("a { @media print { color: red } }"), [2, 1, 1]);
    // Descriptors count as declarations.
    assert_eq!(
        counts("@font-face { font-family: f; src: url(f.woff) }"),
        [1, 0, 2]
    );
    assert_eq!(
        counts("@media print { @counter-style c { system: cyclic; symbols: '*' } }"),
        [2, 0, 2]
    );
    assert_eq!(counts("@foo bar;"), [1, 0, 0]);
    assert_eq!(counts("@charset \"utf-8\";"), [1, 0, 0]);
    // A page rule and its margin box, with all their declarations.
    let [rules, selectors, declarations] =
        counts("@page { margin: 1in; @top-left { content: 'x' } }");
    assert_eq!((rules, selectors), (2, 0));
    assert!(declarations >= 2, "{declarations}");
}

const EVERY_KIND: &str = "
    @charset \"utf-8\";
    a { color: red; margin: 0 }
    b, c {}
    d { color: red; e { color: blue } color: green }
    @media print { f { color: blue } @media screen { g {} } }
    @supports (display: grid) { h { display: grid } }
    @page :first { margin: 1in; @top-left { content: 'x' } }
    @layer x, y.w;
    @layer z { i { color: red } }
    @layer { k {} }
    j { @media print { color: red } @layer v { color: blue } }
    @font-face { font-family: f; src: url(f.woff) }
    @counter-style c { system: cyclic; symbols: '*' }
    @foo bar;
    @foo { baz }
    ::highlight(h) { background-color: red }
";

#[test]
fn limits_at_what_the_stylesheet_needs_retain_everything() {
    let [rules, selectors, declarations] = counts(EVERY_KIND);
    let exact = RuleTreeLimits {
        max_rules: Some(rules),
        max_selectors: Some(selectors),
        max_declarations: Some(declarations),
    };
    let tree = tree_within(EVERY_KIND, exact);
    assert!(tree.limit_exceeded().is_none());
    assert_eq!(tree.budget.counts(), [rules, selectors, declarations]);
    let reference = tree_within(EVERY_KIND, unlimited());
    assert_eq!(tree.style_rules().len(), reference.style_rules().len());
    assert_eq!(tree.rules().len(), reference.rules().len());
}

#[test]
fn one_less_than_the_stylesheet_needs_passes_the_limit() {
    let [rules, selectors, declarations] = counts(EVERY_KIND);
    for (kind, needed, limits) in [
        (
            CascadeLimitKind::StyleRules,
            rules,
            RuleTreeLimits {
                max_rules: Some(rules - 1),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::StyleSelectors,
            selectors,
            RuleTreeLimits {
                max_selectors: Some(selectors - 1),
                ..unlimited()
            },
        ),
        (
            CascadeLimitKind::StyleDeclarations,
            declarations,
            RuleTreeLimits {
                max_declarations: Some(declarations - 1),
                ..unlimited()
            },
        ),
    ] {
        let tree = tree_within(EVERY_KIND, limits);
        let (failed, limit, actual) = exceeded(&tree);
        assert_eq!((failed, limit), (kind, needed - 1), "{limits:?}");
        assert!(
            limit < actual && actual <= needed,
            "{kind:?}: {actual} of {needed}"
        );
    }
}

#[test]
fn every_kind_of_record_counts_as_a_rule() {
    for css in [
        "a {} b {}",
        "a { color: red; b { color: blue } color: green }",
        "@media print { a {} } b {}",
        "@media print { a {} b {} }",
        "@page { margin: 1in } a {}",
        "@page { @top-left { content: 'x' } }",
        "@layer x, y;",
        "@layer x.y;",
        "@layer x { a {} }",
        "@layer { a {} }",
        "a { @layer x, y; }",
        "a { @layer x { color: red } }",
        "a { @layer { color: red } }",
        "a { @media print { color: red } }",
        "@media print { @layer x, y; }",
        "@media print { @layer x { a {} } }",
        "@media print { @layer { a {} } }",
        "@media print { @media screen { a {} } }",
        "@font-face { font-family: f; src: url(f.woff) } a {}",
        "@media print { @font-face { font-family: f; src: url(f.woff) } a {} }",
        "@foo bar; a {}",
        "@foo { baz } a {}",
        "@namespace svg url(http://www.w3.org/2000/svg); a {}",
        "@charset \"utf-8\"; a {}",
        "::highlight(h) { background-color: red } a {}",
    ] {
        let [rules, ..] = counts(css);
        let limits = RuleTreeLimits {
            max_rules: Some(rules - 1),
            ..unlimited()
        };
        let tree = tree_within(css, limits);
        assert_eq!(exceeded(&tree).0, CascadeLimitKind::StyleRules, "{css}");
    }
}

#[test]
fn every_counting_site_can_be_the_one_past_the_limit() {
    // Each stylesheet's last counted rule comes from a different site: the
    // leading `@charset`, a top-level block at-rule, a statement's record,
    // layer names, anonymous layer blocks at the top level, in a group and
    // in a style rule, a group, a descriptor and a page inside a group, a
    // group inside a style rule, and a margin box.
    for (css, max_rules) in [
        ("@charset \"utf-8\";", 0),
        ("a {} @foo { baz }", 1),
        ("@page {}", 0),
        ("@layer x.y;", 2),
        ("@layer x.y;", 1),
        ("@media print { a {} @layer x {} }", 2),
        ("b {} a { @layer x {} }", 1),
        ("@layer {}", 1),
        ("@media print { @layer {} }", 1),
        ("a { @layer {} }", 0),
        ("@media print { @media screen {} }", 1),
        (
            "@media print { a {} @font-face { font-family: f; src: url(f.woff) } }",
            2,
        ),
        ("@media print { @page {} }", 1),
        ("a { @media print {} }", 0),
        ("a { @media print { color: red } }", 1),
        ("a { color: red; b {} }", 1),
        ("@page { @top-left {} @top-right {} }", 2),
    ] {
        let limits = RuleTreeLimits {
            max_rules: Some(max_rules),
            ..unlimited()
        };
        assert_eq!(
            exceeded(&tree_within(css, limits)),
            (CascadeLimitKind::StyleRules, max_rules, max_rules + 1),
            "{css}"
        );
    }
}

#[test]
fn a_huge_rule_stops_at_the_first_declaration_past_the_limit() {
    let borders = "border: 0; ".repeat(1000);
    let limits = RuleTreeLimits {
        max_declarations: Some(30),
        ..unlimited()
    };
    // Each `border` expands to twelve longhands; parsing stops at the
    // third, as soon as the count passes the limit. Page bodies and their
    // margin boxes stop the same way.
    for (css, actual) in [
        (format!("a {{ {borders} }}"), 36),
        (format!("@page {{ {borders} }}"), 36),
        (format!("@media print {{ @page {{ {borders} }} }}"), 36),
        (format!("@page {{ @top-left {{ {borders} }} }}"), 36),
        // The four `margin` longhands come first.
        (
            format!("@page {{ margin: 0; @top-left {{ {borders} }} }}"),
            40,
        ),
    ] {
        assert_eq!(
            exceeded(&tree_within(&css, limits)),
            (CascadeLimitKind::StyleDeclarations, 30, actual),
            "{css}"
        );
    }
    // Descriptor blocks stop the same way.
    for css in [
        format!(
            "@font-face {{ font-family: f; {} }}",
            "font-style: normal; ".repeat(1000)
        ),
        format!(
            "@media print {{ @counter-style c {{ system: cyclic; {} }} }}",
            "symbols: '*'; ".repeat(1000)
        ),
    ] {
        assert_eq!(
            exceeded(&tree_within(&css, limits)),
            (CascadeLimitKind::StyleDeclarations, 30, 31),
            "{css:.40}"
        );
    }
    // Page descriptors count one each.
    let limits = RuleTreeLimits {
        max_declarations: Some(2),
        ..unlimited()
    };
    for css in [
        "@page { size: a4; marks: crop; bleed: 1in }",
        "@page { marks: crop; bleed: 1in; size: a4 }",
        "@page { bleed: 1in; size: a4; marks: crop }",
    ] {
        assert_eq!(
            exceeded(&tree_within(css, limits)),
            (CascadeLimitKind::StyleDeclarations, 2, 3),
            "{css}"
        );
    }
}

#[test]
fn a_selector_list_past_the_limit_is_not_parsed() {
    let limits = RuleTreeLimits {
        max_selectors: Some(2),
        ..unlimited()
    };
    // A list is counted from its tokens before it is parsed, so the count
    // reached is the whole list's, at the top level, in a group and nested
    // in a style rule.
    let huge = format!("{} {{}}", vec!["a"; 100_000].join(", "));
    for (css, actual) in [
        ("a, b, c {}", 3),
        ("@media print { a, b, c {} }", 3),
        ("a { b, c, d {} }", 3),
        (huge.as_str(), 100_000),
    ] {
        assert_eq!(
            exceeded(&tree_within(css, limits)),
            (CascadeLimitKind::StyleSelectors, 2, actual),
            "{css:.40}"
        );
    }
    // A custom highlight rule holds no selectors.
    let limits = RuleTreeLimits {
        max_selectors: Some(0),
        ..unlimited()
    };
    let tree = tree_within("::highlight(h) { background-color: red }", limits);
    assert!(tree.limit_exceeded().is_none());
}

#[test]
fn page_selector_entries_count_as_selectors() {
    assert_eq!(counts("@page :first, :left, named {}"), [1, 3, 0]);
    assert_eq!(counts("@page {}"), [1, 0, 0]);
    // A huge list stops at the first entry past the limit.
    let limits = RuleTreeLimits {
        max_selectors: Some(10),
        ..unlimited()
    };
    let entries = vec![":first"; 100_000].join(",");
    for css in [
        format!("@page {entries} {{}}"),
        format!("@media print {{ @page {entries} {{}} }}"),
    ] {
        assert_eq!(
            exceeded(&tree_within(&css, limits)),
            (CascadeLimitKind::StyleSelectors, 10, 11),
            "{css:.40}"
        );
    }
}

#[test]
fn text_that_opens_no_block_holds_no_selectors() {
    // A declaration that cssparser retries as a nested rule, and a prelude
    // at the end of a stylesheet, are not rules, so their commas count for
    // nothing.
    let list = vec!["a"; 2000].join(",");
    let limits = RuleTreeLimits {
        max_selectors: Some(1),
        ..unlimited()
    };
    for css in [
        format!("a {{ unknown-property: {list}; color: red }}"),
        format!("a {{}} {list}"),
    ] {
        let tree = tree_within(&css, limits);
        assert!(tree.limit_exceeded().is_none(), "{css:.40}");
        assert_eq!(tree.style_rules().len(), 1, "{css:.40}");
    }
}

#[test]
fn layer_names_stop_at_the_first_one_past_the_limit() {
    let names: Vec<String> = (0..100_000).map(|i| format!("l{i}")).collect();
    let names = names.join(", ");
    let limits = RuleTreeLimits {
        max_rules: Some(10),
        ..unlimited()
    };
    // The `@media` record takes the place of one name.
    for css in [
        format!("@layer {names};"),
        format!("@media print {{ @layer {names}; }}"),
        format!("a {{ @layer {names}; }}"),
    ] {
        assert_eq!(
            exceeded(&tree_within(&css, limits)),
            (CascadeLimitKind::StyleRules, 10, 11),
            "{css:.40}"
        );
    }
    // A dotted name counts all its segments at once.
    let dotted = format!("@layer {};", vec!["a"; 100].join("."));
    assert_eq!(
        exceeded(&tree_within(&dotted, limits)),
        (CascadeLimitKind::StyleRules, 10, 100)
    );
}

#[test]
fn a_tree_past_a_limit_retains_nothing_more() {
    let limits = RuleTreeLimits {
        max_rules: Some(2),
        ..unlimited()
    };
    let mut tree = tree_within("a {} b {} c {} d {}", limits);
    assert_eq!(exceeded(&tree), (CascadeLimitKind::StyleRules, 2, 3));
    // The rules before the first one past the limit stay.
    assert_eq!(tree.style_rules().len(), 2);
    tree.add_stylesheet("e {}", Origin::Author);
    tree.add_stylesheet_with_media("f {}", Origin::Author, Some("print"));
    assert_eq!(tree.style_rules().len(), 2);
    assert!(tree.media_rules.is_empty());
    assert_eq!(exceeded(&tree), (CascadeLimitKind::StyleRules, 2, 3));
}

#[test]
fn a_prelude_past_the_rule_limit_is_not_parsed() {
    // With no rule left, a rule's prelude is not parsed either, whatever the
    // other limits allow.
    let entries = vec![":first"; 100_000].join(",");
    let selectors = vec!["b"; 100_000].join(",");
    let limits = RuleTreeLimits {
        max_rules: Some(1),
        ..unlimited()
    };
    for css in [
        format!("a {{}} @page {entries} {{}}"),
        format!("@media print {{ @page {entries} {{}} }}"),
        format!("a {{}} {selectors} {{}}"),
    ] {
        let tree = tree_within(&css, limits);
        assert_eq!(
            exceeded(&tree),
            (CascadeLimitKind::StyleRules, 1, 2),
            "{css:.40}"
        );
        assert!(tree.budget.counts()[1] <= 1, "{css:.40}");
    }
}

#[test]
fn a_rule_past_the_rule_limit_is_not_parsed_on() {
    // Its run of declarations is counted as a rule before its first
    // declaration is parsed, whatever the other limits allow.
    let css = format!("a {{}} b {{ {} }}", "color: red; ".repeat(1000));
    let tree = tree_within(
        &css,
        RuleTreeLimits {
            max_rules: Some(1),
            ..unlimited()
        },
    );
    assert_eq!(exceeded(&tree), (CascadeLimitKind::StyleRules, 1, 2));
    assert_eq!(tree.budget.counts(), [1, 1, 0]);
}

#[test]
fn limits_set_later_count_what_the_tree_retained() {
    let mut tree = RuleTree::empty().with_limits(unlimited());
    tree.add_stylesheet("a {} b {} c {}", Origin::Author);
    let tree = tree.with_limits(RuleTreeLimits {
        max_rules: Some(2),
        ..unlimited()
    });
    assert_eq!(exceeded(&tree), (CascadeLimitKind::StyleRules, 2, 3));
}

#[test]
fn the_cascade_refuses_a_tree_past_a_limit() {
    let mut doc = TestDoc::new();
    doc.push_element(0, "p", None);
    let tree = tree_within(
        "p {} p {}",
        RuleTreeLimits {
            max_rules: Some(1),
            ..unlimited()
        },
    );
    let error = crate::cascade::cascade(&doc, &tree).expect_err("the tree passed a limit");
    assert_eq!(
        error.to_string(),
        "CSS cascade limit exceeded: StyleRules (limit=1, actual=2)"
    );
    let first_line = crate::cascade::cascade_with_first_line(
        &doc,
        &tree,
        &crate::media::MediaContext::default(),
        crate::style_dom::StyleNodeId(1),
    );
    assert!(matches!(
        first_line,
        Err(CascadeError::LimitExceeded {
            kind: CascadeLimitKind::StyleRules,
            ..
        })
    ));
}

#[test]
fn the_default_limits_are_finite_and_admit_ordinary_stylesheets() {
    let limits = RuleTreeLimits::default();
    assert!(limits.max_rules.is_some());
    assert!(limits.max_selectors.is_some());
    assert!(limits.max_declarations.is_some());
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(EVERY_KIND, Origin::Author);
    assert!(tree.limit_exceeded().is_none());
}
