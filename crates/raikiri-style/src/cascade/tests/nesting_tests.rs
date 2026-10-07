use super::*;
use crate::PropertyValue;

#[test]
fn is_parent_lists_match_through_the_cascade() {
    for selector in [
        ":is(.outer,.other) .leaf",
        ":is(.outer,.other) > .leaf",
        ":is(:is(.outer,.other),#absent) .leaf",
        ":is(.outer,.other).outer > .leaf",
    ] {
        let mut doc = TestDoc::new();
        let parent = doc.push_element_with_attrs(0, "div", None, &[("class", "outer")]);
        let target = doc.push_element_with_attrs(parent, "p", None, &[("class", "leaf")]);
        let other = doc.push_element_with_attrs(0, "div", None, &[("class", "unrelated")]);
        let miss = doc.push_element_with_attrs(other, "p", None, &[("class", "leaf")]);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!(".leaf{{color:red}} {selector}{{color:blue}}"),
            Origin::Author,
        );
        let values = cascade(&doc, &tree).unwrap();
        assert_eq!(values.computed[target].color, BLUE);
        assert_eq!(values.computed[miss].color, RED);
    }
}

#[test]
fn is_parent_lists_use_the_maximum_branch_specificity() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element_with_attrs(0, "div", None, &[("class", "outer")]);
    let target = doc.push_element_with_attrs(parent, "p", None, &[("class", "leaf")]);
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        ":is(#absent,.outer) > .leaf{color:blue} .outer > .leaf{color:red}",
        Origin::Author,
    );
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[target].color, BLUE);
}

fn nesting_tree(source: &str) -> RuleTree {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(source, Origin::Author);
    tree
}

fn nesting_doc() -> (TestDoc, usize, usize, usize) {
    let mut doc = TestDoc::new();
    let parent = doc.push_element_with_attrs(0, "div", None, &[("class", "outer")]);
    let child = doc.push_element_with_attrs(parent, "p", None, &[("class", "leaf")]);
    let other = doc.push_element_with_attrs(0, "p", None, &[("class", "leaf")]);
    (doc, parent, child, other)
}

#[test]
fn nested_media_declarations_apply_only_in_their_context() {
    let tree = nesting_tree(".outer {color:red; @media print {color:blue}} ");
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(
        cascade_with_media_context(&doc, &tree, &MediaContext::print())
            .unwrap()
            .computed[parent]
            .color,
        BLUE
    );
    assert_eq!(
        cascade_with_media_context(&doc, &tree, &MediaContext::screen())
            .unwrap()
            .computed[parent]
            .color,
        RED
    );
}

#[test]
fn nested_media_selectors_keep_the_nearest_style_parent() {
    let tree = nesting_tree(
        ".leaf{color:red} .outer {@media print {@supports (color:red) { > .leaf{color:blue}}}}",
    );
    let (doc, _, child, other) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[child].color, BLUE);
    assert_eq!(values.computed[other].color, RED);
}

#[test]
fn nested_media_intersections_do_not_leak_to_siblings() {
    let tree = nesting_tree(
        ".outer {color:red; @media print {@media screen {color:blue} color:green;} @media screen {color:blue}}",
    );
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(
        cascade_with_media_context(&doc, &tree, &MediaContext::print())
            .unwrap()
            .computed[parent]
            .color,
        CssColor {
            r: 0,
            g: 128,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        cascade_with_media_context(&doc, &tree, &MediaContext::screen())
            .unwrap()
            .computed[parent]
            .color,
        BLUE
    );
}

#[test]
fn nested_supports_keep_declarations_and_reject_false_groups() {
    let tree = nesting_tree(
        ".outer{color:red; @supports (display:block){color:blue} @supports (display:invalid){color:red}}",
    );
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn nested_layers_keep_normal_and_important_precedence() {
    for (suffix, expected) in [("", BLUE), ("!important", RED)] {
        let tree = nesting_tree(&format!(
            "@layer a,b; .outer {{ @layer a {{color:red {suffix}}} @layer b {{color:blue {suffix}}} }}"
        ));
        let (doc, parent, _, _) = nesting_doc();
        assert_eq!(
            cascade(&doc, &tree).unwrap().computed[parent].color,
            expected
        );
    }
}

#[test]
fn nested_layer_paths_reopen_across_stylesheets() {
    let mut tree =
        nesting_tree("@layer base.a,base.b; @layer base {.outer {@layer b {color:blue}}}");
    tree.add_stylesheet("@layer base.a { .outer{color:red} }", Origin::Author);
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn anonymous_nested_layers_are_distinct() {
    let tree = nesting_tree(".outer {@layer {color:red} @layer {color:blue}}");
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn nested_selector_lists_use_maximum_parent_specificity() {
    for child_selector in ["& > .leaf", "> .leaf", ".leaf", ":is(&) > .leaf"] {
        let tree = nesting_tree(&format!(
            "#absent,.outer {{{child_selector}{{color:blue}}}} .outer .leaf{{color:red}}"
        ));
        let (doc, _, child, _) = nesting_doc();
        assert_eq!(cascade(&doc, &tree).unwrap().computed[child].color, BLUE);
    }
}

#[test]
fn direct_group_declarations_keep_parent_branch_specificity() {
    let tree = nesting_tree("#absent,.outer {@media print {color:blue}} .outer{color:red}");
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, RED);
}

#[test]
fn direct_group_declarations_keep_parent_pseudo_elements() {
    let tree = nesting_tree(
        ".outer,.outer::before,.outer::after {content:'x'; color:red; @media print {color:blue}}",
    );
    let (doc, parent, _, _) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    for pseudo in [PseudoElem::Before, PseudoElem::After] {
        assert_eq!(
            values.pseudo[&(StyleNodeId(parent as u64), pseudo)].color,
            BLUE
        );
    }
}

#[test]
fn explicit_parent_selectors_do_not_target_parent_pseudo_elements() {
    let tree = nesting_tree(".outer,.outer::before {content:'x';color:red; & {color:blue}}");
    let (doc, parent, _, _) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    assert_eq!(
        values.pseudo[&(StyleNodeId(parent as u64), PseudoElem::Before)].color,
        RED
    );
}

#[test]
fn mixed_declarations_keep_source_order_and_pseudo_targets() {
    let tree = nesting_tree(
        ".outer,.outer::before {content:'x'; color:red; & {color:blue} color:green; @media print {color:blue} color:red;}",
    );
    let (doc, parent, _, _) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, RED);
    assert_eq!(
        values.pseudo[&(StyleNodeId(parent as u64), PseudoElem::Before)].color,
        RED
    );
}

#[test]
fn nested_declarations_resolve_variables_and_expand_shorthands() {
    let tree = nesting_tree(
        ".outer { @supports (color:red) {--paint:blue; color:var(--paint); margin:1px 2px 3px 4px; color:red!important} }",
    );
    let (doc, parent, _, _) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, RED);
    assert_eq!(
        values.computed[parent].margin.left,
        crate::resolve::ComputedLengthPercentageOrAuto::Px(4.0)
    );
}

#[test]
fn nested_selector_functions_do_not_confuse_ampersands_in_strings() {
    let mut doc = TestDoc::new();
    let parent =
        doc.push_element_with_attrs(0, "div", None, &[("class", "outer"), ("title", "x&y")]);
    let tree = nesting_tree(
        r#".outer { &[title="x&y"] {color:blue} :is(&, .other) {background-color:red} }"#,
    );
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    assert_eq!(values.computed[parent].background_color, RED);
}

#[test]
fn invalid_inner_at_rules_never_execute_their_descendants() {
    let tree = nesting_tree(
        ".outer{color:blue; @future {color:red; &{color:red}} @page{color:red} @font-face{font-family:x;src:url(x)}}",
    );
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
    assert!(tree.page_rules.is_empty());
}

#[test]
fn invalid_nested_rules_recover_to_later_declarations_and_siblings() {
    let tree = nesting_tree(
        ".outer{color:red; @media print {color:invalid; &???{color:red} color:blue;} &??? {color:red} } .leaf{color:red}",
    );
    let (doc, parent, _, other) = nesting_doc();
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    assert_eq!(values.computed[other].color, RED);
}

#[test]
fn repeated_parent_references_do_not_discard_ancestor_or_sibling_rules() {
    let (doc, parent, _, other) = nesting_doc();
    let source = format!(
        ".outer {{color:blue; {} color:red; {} }} .leaf{{color:red}}",
        "&&{".repeat(18),
        "}".repeat(18)
    );
    let tree = nesting_tree(&source);
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    assert_eq!(values.computed[other].color, RED);
}

#[test]
fn bounded_repeated_parent_references_still_match() {
    let tree = nesting_tree(".outer{color:red; &&{ &&{color:blue}}}");
    let (doc, parent, _, _) = nesting_doc();
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn nested_parent_references_work_inside_has_and_nth_filters() {
    let (doc, parent, child, _) = nesting_doc();
    for source in [
        ".outer { .leaf:nth-child(1 of :is(& > .leaf)) {color:blue} }",
        ".leaf { .outer:has(> &) {color:blue} }",
    ] {
        let tree = nesting_tree(source);
        let values = cascade(&doc, &tree).unwrap();
        let target = if source.starts_with(".leaf") {
            parent
        } else {
            child
        };
        assert_eq!(values.computed[target].color, BLUE);
    }
}

#[test]
fn ordinary_forgiving_selectors_keep_valid_branches_with_invalid_parent_references() {
    let (doc, _, _, other) = nesting_doc();
    for selector in [".leaf:is(&,.leaf)", ".leaf:where(&,.leaf)"] {
        let tree = nesting_tree(&format!(".leaf{{color:red}} {selector}{{color:blue}}"));
        assert_eq!(cascade(&doc, &tree).unwrap().computed[other].color, BLUE);
    }
}

#[test]
fn parent_pseudo_branches_do_not_increase_nested_selector_specificity() {
    let (doc, _, child, other) = nesting_doc();
    for source in [
        "p{color:blue} *,::before {& *{color:red}}",
        "p{color:blue} *,#absent::before { *{color:red}}",
    ] {
        let values = cascade(&doc, &nesting_tree(source)).unwrap();
        assert_eq!(values.computed[child].color, BLUE);
        assert_eq!(values.computed[other].color, BLUE);
    }
}

#[test]
fn all_pseudo_parent_branches_match_nothing_but_keep_other_forgiving_branches() {
    let (doc, parent, _, _) = nesting_doc();
    let values = cascade(
        &doc,
        &nesting_tree(".outer{color:red} .outer::before { &{color:red} :is(&,.outer){color:blue}}"),
    )
    .unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
}

#[test]
fn nested_layer_statements_establish_order_and_recover_invalid_forms() {
    let (doc, parent, _, _) = nesting_doc();
    let tree = nesting_tree(".outer{@layer b,a;@layer a{color:red}@layer b{color:blue}}");
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, RED);
    let tree = nesting_tree(
        ".outer{color:blue;@layer;@media print;@supports (display:block);@layer a,b{color:red}}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn broad_nested_selector_lists_preserve_ancestor_and_sibling_declarations() {
    let (doc, parent, _, other) = nesting_doc();
    let source = format!(
        ".outer{{color:blue;{}{{color:red}}}}.leaf{{color:red}}",
        vec!["&"; 12000].join(",")
    );
    let values = cascade(&doc, &nesting_tree(&source)).unwrap();
    assert_eq!(values.computed[parent].color, BLUE);
    assert_eq!(values.computed[other].color, RED);
}

#[test]
fn deep_nested_selector_functions_preserve_ancestor_declarations() {
    let (doc, parent, _, _) = nesting_doc();
    for function in ["", ":has("] {
        let selector = format!(
            "{}{}&{}{}",
            function,
            ":is(".repeat(132),
            ")".repeat(132),
            if function.is_empty() { "" } else { ")" }
        );
        let source = format!(".outer{{color:blue;{selector}{{color:red}}}}");
        let tree = nesting_tree(&source);
        assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
        assert!(tree.style_rules().iter().all(|r| {
            r.declarations()
                .iter()
                .all(|d| !matches!(d.value(),PropertyValue::Color(c) if *c==RED))
        }));
    }
}

#[test]
fn implicit_descendant_nesting_matches_only_children_of_the_parent() {
    let (doc, _, child, other) = nesting_doc();
    let tree = nesting_tree(".leaf{color:red}.outer{.leaf,.other-leaf{color:blue}}");
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[child].color, BLUE);
    assert_eq!(values.computed[other].color, RED);
}

#[test]
fn bounded_nested_selector_functions_still_match() {
    let (doc, parent, _, _) = nesting_doc();
    let selector = format!("{}&{}", ":is(".repeat(32), ")".repeat(32));
    let tree = nesting_tree(&format!(".outer{{color:red;{selector}{{color:blue}}}}"));
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn accumulated_selector_graph_depth_preserves_declarations() {
    let (doc, parent, _, _) = nesting_doc();
    for function in ["", ":has("] {
        let wrappers = if function.is_empty() { 32 } else { 31 };
        let selector = format!(
            "{}{}&{}{}",
            function,
            ":is(".repeat(wrappers),
            ")".repeat(wrappers),
            if function.is_empty() { "" } else { ")" }
        );
        let source = format!(
            ".outer{{color:blue;{}{selector}{{ &{{color:red}} }}{}}}",
            "&{".repeat(110),
            "}".repeat(110)
        );
        let tree = nesting_tree(&source);
        assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
        assert!(tree.style_rules().iter().all(|r| {
            r.declarations()
                .iter()
                .all(|d| !matches!(d.value(),PropertyValue::Color(c) if *c==RED))
        }));
    }
}

#[test]
fn has_parent_negation_is_contextually_invalid() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let tree =
        nesting_tree(".ancestor{color:red}.outer:not(:has(.leaf)){.ancestor:has(> &){color:blue}}");
    let values = cascade(&doc, &tree).unwrap();
    assert_eq!(values.computed[ancestor].color, RED);
}

#[test]
fn has_parent_invalid_branch_has_no_specificity() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let tree = nesting_tree(
        ".outer,#absent:has(.leaf){.ancestor:has(> &){color:blue}}.ancestor:has(>.outer){color:red}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[ancestor].color, RED);
}

#[test]
fn highlight_parent_keeps_valid_forgiving_branch() {
    let (doc, parent, _, _) = nesting_doc();
    let tree = nesting_tree(".outer{color:red}::highlight(note){:is(&,.outer){color:blue}}");
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn has_parent_keeps_valid_branches_and_outer_has() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    for source in [
        ".ancestor{color:red}.outer:not(:has(.leaf)),.outer{.ancestor:has(> &){color:blue}}",
        ".ancestor{color:red}.outer:is(:not(:has(.leaf)),.outer){.ancestor:has(> &){color:blue}}",
    ] {
        assert_eq!(
            cascade(&doc, &nesting_tree(source)).unwrap().computed[ancestor].color,
            BLUE
        );
    }
    let tree = nesting_tree(".outer{color:red}.outer:has(.leaf){&{color:blue}}");
    assert_eq!(cascade(&doc, &tree).unwrap().computed[outer].color, BLUE);
}

#[test]
fn has_parent_nth_context_is_invalid() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let tree = nesting_tree(
        ".ancestor{color:red}.outer:nth-child(1 of :has(.leaf)){.ancestor:has(> &){color:blue}}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[ancestor].color, RED);
}

#[test]
fn explicit_is_context_controls() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    for source in [
        ".ancestor{color:red}.ancestor:has(>:is(.outer:not(:has(.leaf)))){color:blue}",
        ".ancestor:has(>:is(.outer,#absent:has(.leaf))){color:blue}.ancestor:has(>.outer){color:red}",
        ".ancestor{color:red}.ancestor:has(>:is(.outer:nth-child(1 of :has(.leaf)))){color:blue}",
    ] {
        assert_eq!(
            cascade(&doc, &nesting_tree(source)).unwrap().computed[ancestor].color,
            RED
        );
    }
}

#[test]
fn has_parent_context_preserves_references_outside_the_has() {
    let mut doc = TestDoc::new();
    let root = doc.push_element_with_attrs(0, "div", None, &[("class", "outer")]);
    let ancestor = doc.push_element_with_attrs(root, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let tree = nesting_tree(
        ".outer,#absent:has(.leaf){& .ancestor:has(> &){color:blue}}.outer .ancestor:has(>.outer){color:red}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[ancestor].color, BLUE);
}

#[test]
fn deep_contextual_has_recovery_keeps_sibling_declarations() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    for depth in [25, 29] {
        let parent = format!(
            "{}.outer:not(:has(.leaf)){}",
            ":is(".repeat(depth),
            ")".repeat(depth)
        );
        let tree = nesting_tree(&format!(
            ".ancestor{{color:red}}{parent}{{.ancestor:has(>&){{color:blue}}}}.outer{{color:blue}}"
        ));
        let values = cascade(&doc, &tree).unwrap();
        assert_eq!(values.computed[ancestor].color, RED);
        assert_eq!(values.computed[outer].color, BLUE);
    }
}

#[test]
fn where_parent_keeps_valid_branches_inside_has() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let outer = doc.push_element_with_attrs(ancestor, "div", None, &[("class", "outer")]);
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let tree = nesting_tree(
        ".ancestor{color:red}:where(.outer:not(:has(.leaf)),.outer){.ancestor:has(>&){color:blue}}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[ancestor].color, BLUE);
}

#[test]
fn repeated_parent_has_references_keep_their_valid_matches() {
    let (doc, parent, _, _) = nesting_doc();
    let tree = nesting_tree(".outer{color:red}.outer:has(.leaf){&&{color:blue}}");
    assert_eq!(cascade(&doc, &tree).unwrap().computed[parent].color, BLUE);
}

#[test]
fn has_context_repair_preserves_explicit_any_namespace_parent() {
    let mut doc = TestDoc::new();
    let outer = doc.push_element_with_namespace(0, "div", "urn:foreign", &[("class", "outer")]);
    doc.push_element_with_namespace(outer, "div", "urn:foreign", &[("class", "outer")]);
    let tree = nesting_tree(
        "@namespace 'urn:default'; *|*.outer{color:red} *|*.outer,#absent:has(.leaf){&:has(>&){color:blue}}",
    );
    assert_eq!(cascade(&doc, &tree).unwrap().computed[outer].color, BLUE);
}

#[test]
fn contextual_parent_serialization_budget_preserves_other_rules() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let large = "a".repeat(32 * 1024);
    let outer = doc.push_element_with_attrs(
        ancestor,
        "div",
        None,
        &[("class", "outer"), ("data-big", &large)],
    );
    doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let sibling = doc.push_element_with_attrs(0, "div", None, &[("class", "sibling")]);
    for (count, expected) in [(4, BLUE), (2050, RED)] {
        let selector = format!(".ancestor:has(>{})", ":is(&,.outer)".repeat(count));
        let source = format!(
            ".ancestor{{color:red}}.outer[data-big=\"{large}\"]:has(.leaf){{color:blue;{selector}{{color:blue}}}}.sibling{{color:blue}}"
        );
        let values = cascade(&doc, &nesting_tree(&source)).unwrap();
        assert_eq!(values.computed[ancestor].color, expected);
        assert_eq!(values.computed[outer].color, BLUE);
        assert_eq!(values.computed[sibling].color, BLUE);
    }
}

#[test]
fn failed_contextual_repairs_spend_the_budget_but_keep_ordinary_nesting() {
    let mut doc = TestDoc::new();
    let ancestor = doc.push_element_with_attrs(0, "section", None, &[("class", "ancestor")]);
    let large = "a".repeat(32 * 1024);
    let outer = doc.push_element_with_attrs(
        ancestor,
        "div",
        None,
        &[("class", "outer"), ("data-big", &large)],
    );
    let leaf = doc.push_element_with_attrs(outer, "p", None, &[("class", "leaf")]);
    let sibling = doc.push_element_with_attrs(0, "div", None, &[("class", "sibling")]);
    let oversized = format!(".ancestor:has(>{})", ":is(&,.outer)".repeat(2050));
    let small = format!(".ancestor:has(>{})", ":is(&,.outer)".repeat(4));
    let source = format!(
        ".ancestor,.leaf{{color:red}}.outer[data-big=\"{large}\"]:has(.leaf){{color:blue;{oversized}{{color:blue}}{small}{{color:blue}}.leaf{{color:blue}}}}.sibling{{color:blue}}"
    );
    let values = cascade(&doc, &nesting_tree(&source)).unwrap();
    assert_eq!(values.computed[ancestor].color, RED);
    assert_eq!(values.computed[outer].color, BLUE);
    assert_eq!(values.computed[leaf].color, BLUE);
    assert_eq!(values.computed[sibling].color, BLUE);
}

#[test]
fn supports_nesting_selector_guards_working_nested_rules() {
    let (doc, parent, _, _) = nesting_doc();
    for query in [
        "selector(&)",
        "selector(:is(&,.outer))",
        "selector(:has(>&))",
        "selector(&:nth-child(1))",
    ] {
        let source = format!(".outer{{color:red;@supports {query}{{&{{color:blue}}}}}}");
        assert_eq!(
            cascade(&doc, &nesting_tree(&source)).unwrap().computed[parent].color,
            BLUE,
            "{query}"
        );
    }
    for query in [
        "selector(:is(&,:unknown))",
        "selector(&,.outer)",
        "selector(>&)",
        "selector(& || .outer)",
    ] {
        let source = format!(".outer{{color:red;@supports {query}{{&{{color:blue}}}}}}");
        assert_eq!(
            cascade(&doc, &nesting_tree(&source)).unwrap().computed[parent].color,
            RED,
            "{query}"
        );
    }
    let (doc, _, leaf, _) = nesting_doc();
    let tree = nesting_tree("@supports selector(&){.leaf{color:red;&{color:inherit}}}");
    assert_eq!(
        cascade(&doc, &tree).unwrap().computed[leaf].color,
        CssColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255
        }
    );
}
