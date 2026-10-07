use super::*;
use cssparser::ToCss;

#[test]
fn native_nesting_preserves_origin_namespace_and_order() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@namespace svg 'urn:svg'; svg|svg { color:red; @media print { > svg|rect {color:blue} color:green; } color:blue; }", Origin::User);
    assert_eq!(tree.style_rules().len(), 2);
    assert_eq!(tree.media_rules.len(), 2);
    let nested = &tree.media_rules[0].rule;
    assert_eq!(nested.origin, Origin::User);
    assert!(nested.selectors.to_css_string().contains("svg|rect"));
    assert!(nested.source_order < tree.style_rules()[1].source_order);
}

#[test]
fn large_parent_products_do_not_expand_or_discard_sibling_rules() {
    let mut tree = RuleTree::empty();
    let nested = vec!["&"; 14].join(" + ");
    tree.add_stylesheet(
        &format!("before {{color:red}} .a,.b {{ {nested}{{color:blue}} }} after {{color:red}}"),
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 3);
    assert_eq!(tree.style_rules()[1].selectors.slice().len(), 1);
}

#[test]
fn large_stylesheets_no_longer_share_the_expansion_item_quota() {
    let mut tree = RuleTree::empty();
    let source = ".a,.b { & > .c {color:red} }".repeat(2200);
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(tree.style_rules().len(), 2200);
}

#[test]
fn unsupported_inner_registration_rules_do_not_escape_the_style_body() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("p {@counter-style x {system:cyclic;symbols:'x'} @page {margin:1px} @font-face {font-family:x;src:url(x)} color:red}", Origin::Author);
    assert_eq!(tree.style_rules().len(), 1);
    assert!(tree.page_rules.is_empty());
    assert!(tree.counter_styles().get("x").is_none());
    assert!(tree.font_faces().is_empty());
}

#[test]
fn nested_groups_keep_the_original_opaque_source() {
    let mut tree = RuleTree::empty();
    let body = " .a { @media print { color:blue; > .b {color:red} } } ";
    tree.add_stylesheet(&format!("@layer example {{{body}}}"), Origin::Author);
    assert_eq!(
        tree.opaque_at_rules()[0].body,
        AtRuleBody::Block(body.to_owned())
    );
    assert_eq!(tree.media_rules.len(), 2);
}

#[test]
fn excessive_native_style_depth_keeps_ancestor_and_sibling_declarations() {
    let mut tree = RuleTree::empty();
    let source = format!(
        "p{{color:red; {} color:blue; {} }} aside{{color:red}}",
        "&{".repeat(140),
        "}".repeat(140)
    );
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(
        tree.style_rules()
            .iter()
            .filter(|r| !r.declarations().is_empty())
            .count(),
        2
    );
    assert!(
        tree.style_rules()
            .iter()
            .all(|r| r.declarations().iter().all(|d| !matches!(
                d.value(),
                PropertyValue::Color(CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    ..
                })
            )))
    );
}

#[test]
fn mixed_style_group_depth_uses_one_limit() {
    let mut tree = RuleTree::empty();
    let source = format!(
        "p{{color:red; {} color:blue; {} }} aside{{color:red}}",
        "@supports (color:red) { & {".repeat(80),
        "}}".repeat(80)
    );
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(
        tree.style_rules()
            .iter()
            .filter(|r| !r.declarations().is_empty())
            .count(),
        2
    );
    assert!(
        tree.style_rules()
            .iter()
            .all(|r| r.declarations().iter().all(|d| !matches!(
                d.value(),
                PropertyValue::Color(CssColor {
                    r: 0,
                    g: 0,
                    b: 255,
                    ..
                })
            )))
    );
}

#[test]
fn custom_property_brace_blocks_are_values_not_nested_rules() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p{--tokens:{color:red; & {color:blue}}; color:blue}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.style_rules()[0].declarations().len(), 2);
}

#[test]
fn nested_groups_share_consumer_property_grammar_and_importance() {
    let mut tree =
        RuleTree::empty_with_consumer_properties(&[ConsumerPropertyRegistration::integer(
            "consumer-level",
        )]);
    tree.add_stylesheet("p {@supports (consumer-level:1) {@media print {consumer-level:2!important; consumer-level:invalid; color:blue}}}", Origin::Author);
    assert_eq!(tree.media_rules.len(), 1);
    let declarations = tree.media_rules[0].rule.declarations();
    assert_eq!(declarations.len(), 2);
    assert!(declarations[0].important);
    let PropertyValue::CustomProperty(custom) = declarations[0].value() else {
        panic!("expected registered consumer value");
    };
    assert_eq!(custom.name, "--consumer-level");
    assert_eq!(custom.value, "2");
}

#[test]
fn highlight_direct_declarations_inside_nested_supports_apply() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "::highlight(note){background-color:red;@supports (display:block){background-color:blue}}",
        Origin::Author,
    );
    assert_eq!(
        tree.custom_highlight_styles().get("note"),
        Some(&CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        })
    );
}

#[test]
fn highlight_nested_groups_preserve_layer_priority_and_static_media_policy() {
    let blue = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    for source in [
        "@layer a,b;::highlight(note){@layer a{background-color:red}@layer b{background-color:blue}}",
        "@layer a,b;::highlight(note){@layer a{background-color:blue!important}@layer b{background-color:red!important}}",
        "::highlight(note){@supports (display:invalid){background-color:red}background-color:blue;@media print{background-color:red}}",
        "@supports (display:block){::highlight(note){@supports (color:red){background-color:blue}}}",
        "::highlight(note){background-color:blue;&{background-color:red}.child{background-color:red}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(tree.custom_highlight_styles().get("note"), Some(&blue));
    }
}

#[test]
fn nested_highlight_registrations_share_large_names() {
    let name = "h".repeat(16 * 1024);
    let source = format!(
        "::highlight({name}){{background-color:red;{}background-color:blue}}",
        "@supports (display:block){background-color:red}".repeat(96)
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(tree.highlight_log.len(), 98);
    let first = tree.highlight_log[0].rule.0.as_ptr();
    assert!(
        tree.highlight_log
            .iter()
            .all(|entry| entry.rule.0.as_ptr() == first)
    );
    assert_eq!(
        tree.custom_highlight_styles().get(&name),
        Some(&CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        })
    );
}
