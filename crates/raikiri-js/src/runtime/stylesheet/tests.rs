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
