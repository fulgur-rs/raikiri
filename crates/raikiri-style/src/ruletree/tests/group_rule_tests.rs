use super::*;
use cssparser::ToCss;

fn selector_text(rule: &StyleRule) -> String {
    rule.selectors.to_css_string()
}

#[test]
fn nested_supports_execute_at_their_source_position() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p {color:red} @supports (display:block) { \
         @supports (color:red) {div {color:blue}} span {color:green} \
         @supports (display:invalid) {aside {color:red}} } em {color:blue}",
        Origin::User,
    );
    assert_eq!(
        tree.style_rules()
            .iter()
            .map(selector_text)
            .collect::<Vec<_>>(),
        ["p", "div", "span", "em"]
    );
    for (order, rule) in tree.style_rules().iter().enumerate() {
        assert_eq!(rule.source_order, order as u32);
        assert_eq!(rule.origin, Origin::User);
    }
}

#[test]
fn media_and_supports_nest_in_either_order() {
    for source in [
        "@media print {@supports (color:red) {p {color:red}}}",
        "@supports (color:red) {@media print {p {color:red}}}",
        "@supports (color:red) {@media print {@supports (display:block) {p {color:red}}}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::UserAgent);
        assert!(tree.style_rules().is_empty(), "{source}");
        assert_eq!(tree.media_rules.len(), 1, "{source}");
        let rule = &tree.media_rules[0];
        assert!(rule.condition.matches(&MediaContext::print()));
        assert!(!rule.condition.matches(&MediaContext::screen()));
        assert_eq!(rule.rule.origin, Origin::UserAgent);
    }
}

#[test]
fn supports_inside_flattened_layers_execute_as_groups() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@layer base {@supports (color:red) {p {color:red} \
         @media print {div {color:blue}}}}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(selector_text(&tree.style_rules()[0]), "p");
    assert_eq!(selector_text(&tree.media_rules[0].rule), "div");
}

#[test]
fn existing_layers_inside_supports_keep_styles_pages_and_precedence() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "p {color:blue} @supports (color:red) {div {color:blue} \
         @layer a {p {color:red} @page {margin:1in}}}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 3);
    assert_eq!(tree.page_rules.len(), 1);
    assert_eq!(tree.page_rules[0].layer_order, 0);
    assert_eq!(
        tree.style_rules()
            .iter()
            .map(selector_text)
            .collect::<Vec<_>>(),
        ["p", "p", "div"]
    );
    assert!(matches!(
        tree.style_rules()[0].declarations[0].value(),
        PropertyValue::Color(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        })
    ));
}

#[test]
fn supported_layers_keep_existing_name_order_and_wrapper_isolation() {
    for (source, selectors) in [
        (
            "@supports (color:red) {u {color:blue} @layer a {la {color:red}}} \
             @layer b {lb {color:green}}",
            vec!["la", "lb", "u"],
        ),
        (
            "@layer b {lb {color:green}} \
             @supports (color:red) {u {color:blue} @layer a {la {color:red}}}",
            vec!["lb", "la", "u"],
        ),
        (
            "@layer b,a; @supports (color:red) {u {color:blue} @layer a {la {color:red}}} \
             @layer b {lb {color:green}}",
            vec!["lb", "la", "u"],
        ),
        (
            "@layer a {la {color:red}} \
             @supports (color:red) {u {color:blue} @layer a {lb {color:green}}}",
            vec!["la", "lb", "u"],
        ),
        (
            "@supports (display:invalid) {u {color:blue} @layer a {hidden {color:red}}} \
             @layer b {lb {color:green}}",
            vec!["lb"],
        ),
        (
            "@supports (color:red) {u {color:blue} \
             @future {@layer a {hidden {color:red}}} \
             @media print {@layer a {hidden {color:red}}}}",
            vec!["u"],
        ),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(
            tree.style_rules()
                .iter()
                .map(selector_text)
                .collect::<Vec<_>>(),
            selectors,
            "{source}"
        );
        assert!(tree.media_rules.is_empty(), "{source}");
    }
}

#[test]
fn supported_layer_statements_order_later_blocks_only_when_true() {
    for (condition, selectors) in [
        ("(color:red)", ["lb", "la"]),
        ("(display:invalid)", ["la", "lb"]),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!(
                "@supports {condition} {{@layer b,a;}} \
                @layer a {{la {{color:red}}}} @layer b {{lb {{color:blue}}}}"
            ),
            Origin::Author,
        );
        assert_eq!(
            tree.style_rules()
                .iter()
                .map(selector_text)
                .collect::<Vec<_>>(),
            selectors
        );
    }
}

#[test]
fn supported_layer_extraction_keeps_declaration_recovery_and_depth_limits() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color:red) {u {color:blue} \
         @layer a {p {color:red;content:\"oops\n;} div {color:blue}}}",
        Origin::Author,
    );
    assert_eq!(
        tree.style_rules()
            .iter()
            .map(selector_text)
            .collect::<Vec<_>>(),
        ["p", "div", "u"]
    );
    assert!(
        tree.style_rules()
            .iter()
            .all(|rule| rule.declarations.len() == 1)
    );

    let depth = MAX_OPAQUE_RULE_NESTING_DEPTH + 2;
    let source = format!(
        "{}@layer a {{hidden {{color:red}}}}{} div {{color:blue}}",
        "@supports (color:red) {".repeat(depth),
        "}".repeat(depth)
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(
        tree.style_rules()
            .iter()
            .map(selector_text)
            .collect::<Vec<_>>(),
        ["div"]
    );
}

#[test]
fn mixed_groups_intersect_local_and_stylesheet_media() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media(
        "@supports (color:red) {@media (min-width:500px) { \
         @supports (display:block) {@media (max-width:800px) {p {color:red}}}}}",
        Origin::Author,
        Some("print"),
    );
    assert_eq!(tree.media_rules.len(), 1);
    let condition = &tree.media_rules[0].condition;
    for (media_type, width, matches) in [
        (crate::media::MediaType::Print, 600, true),
        (crate::media::MediaType::Print, 400, false),
        (crate::media::MediaType::Print, 900, false),
        (crate::media::MediaType::Screen, 600, false),
    ] {
        assert_eq!(
            condition.matches(&MediaContext::with_viewport(media_type, width, 100)),
            matches
        );
    }
}

#[test]
fn false_or_invalid_groups_suppress_every_descendant() {
    for source in [
        "@supports (display:invalid) {@supports (color:red) {p {color:red}}}",
        "@supports (display:invalid) {@media print {p {color:red}}}",
        "@media not all {@supports (color:red) {p {color:red}}}",
        "@supports (color:red) {@media not all {p {color:red}}}",
        "@supports (color:red) or (color:blue) and (display:block) {p {color:red}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert!(tree.style_rules().is_empty(), "{source}");
        assert!(tree.media_rules.is_empty(), "{source}");
        assert_eq!(tree.opaque_at_rules().len(), 1, "{source}");
    }
}

#[test]
fn statement_groups_do_not_hide_following_blocks() {
    for source in [
        "@supports (color:red); @supports (color:red) {p {color:red}}",
        "@supports (color:red) {@supports (color:red); p {color:red}}",
        "@media print {@supports (color:red); @supports (color:red) {p {color:red}}}",
        "@supports (color:red); @supports (color:red) {@layer a {p {color:red}}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(
            tree.style_rules().len() + tree.media_rules.len(),
            1,
            "{source}"
        );
    }
}

#[test]
fn unknown_wrappers_do_not_execute_their_conditional_children() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color:red) {@future {@supports (color:red) {p {color:red}}} \
         div {color:blue}} @future {@media print {span {color:red}}}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(selector_text(&tree.style_rules()[0]), "div");
    assert!(tree.media_rules.is_empty());
}

#[test]
fn groups_preserve_raw_inspection_content_and_origin() {
    let body = " /* body */ @supports (display:block) { p {color:red} } ";
    let source = format!("@supports /* prelude */ (color:red) {{{body}}}");
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::User);
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.opaque_at_rules().len(), 1);
    let record = &tree.opaque_at_rules()[0];
    assert_eq!(record.prelude, " /* prelude */ (color:red) ");
    assert_eq!(record.body.as_block(), Some(body));
    assert_eq!(record.to_css(), source);
    assert_eq!(record.origin, Origin::User);
    let RuleNode::AtRule(inner) = &record.children()[0] else {
        panic!("nested supports record missing");
    };
    assert_eq!(inner.name, "supports");
    assert_eq!(inner.origin, Origin::User);
    assert_eq!(inner.children().len(), 1);
}

#[test]
fn leading_comments_do_not_turn_group_rules_into_selectors() {
    for source in [
        "/* lead */ @supports (color:red) {@supports (display:block) {p {color:red}}}",
        "@supports (color:red) {/* lead */ @media print {p {color:red}}}",
        "/* lead */ @media print {/* nested */ @supports (color:red) {p {color:red}}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(
            tree.style_rules().len() + tree.media_rules.len(),
            1,
            "{source}"
        );
        assert_eq!(tree.opaque_at_rules().len(), 1, "{source}");
    }
}

#[test]
fn conditional_styles_keep_one_order_across_views_and_calls() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color:red) {@media print {p {color:red}} div {color:blue}} \
         span {color:red} @media screen {@supports (color:red) {em {color:red}}}",
        Origin::Author,
    );
    tree.add_stylesheet("strong {color:red}", Origin::User);
    assert_eq!(
        tree.media_rules
            .iter()
            .map(|m| m.rule.source_order)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    assert_eq!(
        tree.style_rules()
            .iter()
            .map(|r| r.source_order)
            .collect::<Vec<_>>(),
        [1, 2, 4]
    );
}

#[test]
fn grouped_page_rules_keep_origin_and_conditions() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color:red) {@page {margin:1in} \
         @media print {@supports (color:red) {@page :first {margin:2in}}}}",
        Origin::User,
    );
    assert_eq!(tree.page_rules.len(), 2);
    assert_eq!(tree.page_rules[0].source_order, 0);
    assert_eq!(tree.page_rules[1].source_order, 1);
    assert!(tree.page_rules[0].media_condition.is_none());
    let condition = tree.page_rules[1].media_condition.as_ref().unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.matches(&MediaContext::screen()));
    assert!(tree.page_rules.iter().all(|r| r.origin == Origin::User));
}

#[test]
fn group_queries_and_selectors_share_stylesheet_namespaces() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@namespace svg 'urn:svg'; @supports selector(svg|a) { \
         @supports selector(svg|a) {svg|a {color:red}} \
         @media print {svg|a {color:blue}}}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(selector_text(&tree.style_rules()[0]), "svg|a");
    assert_eq!(selector_text(&tree.media_rules[0].rule), "svg|a");
}

#[test]
fn group_queries_and_declarations_share_consumer_properties() {
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut tree = RuleTree::empty_with_consumer_properties(&registrations);
    tree.add_stylesheet(
        "@supports (bookmark-level:1) {@supports (bookmark-level:2) { \
         p {bookmark-level:3}} @media print {@supports (bookmark-level:4) { \
         div {bookmark-level:5}}}}",
        Origin::Author,
    );
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.style_rules()[0].declarations.len(), 1);
    assert_eq!(tree.media_rules.len(), 1);
    assert_eq!(tree.media_rules[0].rule.declarations.len(), 1);
}

#[test]
fn supported_highlights_execute_without_a_leading_style_rule() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color:red) {@supports selector(::highlight(open)) { \
         ::highlight(open) {background-color:red}}} \
         @supports (display:invalid) {::highlight(hidden) {background-color:blue}}",
        Origin::Author,
    );
    assert_eq!(tree.custom_highlight_styles().len(), 1);
    assert!(tree.custom_highlight_styles().contains_key("open"));
}

#[test]
fn legacy_descriptor_registration_keeps_supported_rules_and_sheet_media() {
    let source = "@supports (color:red) {p {color:red} \
         @font-face {font-family:demo;src:url(font.woff)} \
         @counter-style marks {system:cyclic;symbols:\"*\"}} \
         @supports (display:invalid) {p {color:red} \
         @font-face {font-family:hidden;src:url(hidden.woff)} \
         @counter-style hidden {system:cyclic;symbols:\"-\"}}";
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media(source, Origin::User, Some("print"));
    assert_eq!(tree.media_rules.len(), 1);
    let print = MediaContext::print();
    let screen = MediaContext::screen();
    assert!(tree.font_faces_for(&print).get("demo").is_some());
    assert!(tree.font_faces_for(&screen).get("demo").is_none());
    assert!(tree.font_faces_for(&print).get("hidden").is_none());
    assert!(tree.counter_styles_for(&print).get("marks").is_some());
    assert!(tree.counter_styles_for(&screen).get("marks").is_none());
    assert!(tree.counter_styles_for(&print).get("hidden").is_none());
}

#[test]
fn tokenizer_errors_in_groups_drop_only_bad_declarations() {
    for wrapper in ["@supports (color:red)", "@media print"] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("{wrapper} {{p {{color:red;content:\"oops\n;}} div {{color:blue}}}}"),
            Origin::Author,
        );
        let rules: Vec<_> = tree
            .style_rules()
            .iter()
            .chain(tree.media_rules.iter().map(|r| &r.rule))
            .collect();
        assert_eq!(rules.len(), 2, "{wrapper}");
        assert!(rules.iter().all(|r| r.declarations.len() == 1));
        assert_eq!(tree.opaque_at_rules().len(), 1, "{wrapper}");
    }
}

#[test]
fn group_depth_and_unclosed_blocks_do_not_execute_partial_content() {
    let depth = MAX_OPAQUE_RULE_NESTING_DEPTH + 2;
    let source = format!(
        "{}p {{color:red}}{} div {{color:blue}}",
        "@supports (color:red) {".repeat(depth),
        "}".repeat(depth)
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&source, Origin::Author);
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(selector_text(&tree.style_rules()[0]), "div");
    for source in [
        "@supports (color:red) {p {color:red}",
        "@supports (color:red) {p {color:red",
        "@supports (color:red) {u {color:blue} @layer a {p {color:red}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert!(tree.style_rules().is_empty());
        assert!(tree.opaque_at_rules().is_empty());
    }
}

#[test]
fn escaped_group_names_use_css_identifier_parsing() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        r"@\73 upports (color:red) {@\6d edia print {p {color:red}}}",
        Origin::Author,
    );
    assert_eq!(tree.media_rules.len(), 1);
}

#[test]
fn layer_first_mentions_do_not_move_when_a_later_statement_names_them() {
    for source in [
        "@layer a {la {color:red}} @layer b,a; @layer b {lb {color:blue}}",
        "@supports (color:red) {@layer a {la {color:red}}} @layer b,a; @layer b {lb {color:blue}}",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        assert_eq!(
            tree.style_rules()
                .iter()
                .map(selector_text)
                .collect::<Vec<_>>(),
            ["la", "lb"],
            "{source}"
        );
    }
}

#[test]
fn stylesheet_media_does_not_register_highlights_in_the_unconditional_map() {
    for source in [
        "::highlight(open) {background-color:red}",
        "@supports (color:red) {::highlight(open) {background-color:red}}",
        "@supports (color:red) {@supports selector(::highlight(open)) {::highlight(open) {background-color:red}}}",
    ] {
        for media in [None, Some("print"), Some("screen"), Some("all")] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet_with_media(source, Origin::Author, media);
            assert_eq!(
                tree.custom_highlight_styles().contains_key("open"),
                media.is_none(),
                "{source}, {media:?}"
            );
        }
    }
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media all {@supports (color:red) {::highlight(open) {background-color:red}}}",
        Origin::Author,
    );
    assert!(tree.custom_highlight_styles().is_empty());
}
