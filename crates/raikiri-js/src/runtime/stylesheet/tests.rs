use super::parse_css_rules;

#[test]
fn top_level_rule_ranges_preserve_preludes_and_declaration_bodies() {
    let source =
        "@import url('theme.css');\nspan::target-text { color: cyan; }\nspan { color: magenta; }";
    let rules = parse_css_rules(source);

    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].prelude, "@import url('theme.css')");
    assert_eq!(
        &source[rules[0].source.clone()],
        "@import url('theme.css');"
    );
    assert_eq!(rules[0].body, None);
    assert_eq!(rules[1].prelude, "span::target-text");
    assert_eq!(&source[rules[1].body.clone().unwrap()], " color: cyan; ");
    assert_eq!(rules[2].prelude, "span");
    assert_eq!(&source[rules[2].body.clone().unwrap()], " color: magenta; ");
}

#[test]
fn malformed_blocks_stop_rule_parsing_without_panicking() {
    assert!(parse_css_rules("}").is_empty());
}

#[test]
fn rule_boundaries_follow_the_css_syntax_top_level_algorithm() {
    // A `;` does not end a qualified rule's prelude.
    let rules = parse_css_rules("a;b{} c{}");
    assert_eq!(
        rules.iter().map(|r| r.prelude.as_str()).collect::<Vec<_>>(),
        ["a;b", "c"]
    );

    // An at-rule without a block may end at EOF.
    let source = "p{} @import \"x\"";
    let rules = parse_css_rules(source);
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[1].prelude, "@import \"x\"");
    assert_eq!(&source[rules[1].source.clone()], "@import \"x\"");
    assert_eq!(rules[1].body, None);

    // Top-level `<!--` and `-->` are skipped, not part of a selector.
    let rules = parse_css_rules("<!-- p { color: red } -->");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].prelude, "p");
}
