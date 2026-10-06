use super::*;

#[test]
fn media_type_modifiers_match_selected_contexts() {
    for (source, print, screen) in [
        ("not screen", true, false),
        ("not print", false, true),
        ("only print", true, false),
        ("only screen", false, true),
        ("only all", true, true),
        ("not all", false, false),
        ("not projection", true, true),
        ("not unknown-medium", true, true),
        ("only unknown-medium", false, false),
        ("NoT /* comment */ ScReEn", true, false),
        (r"\6e ot \73 creen", true, false),
        (r"\6f nly /*,*/ \70 rint", true, false),
        ("only screen, not screen", true, true),
        ("not all, only print", true, false),
        ("not only screen, only print", true, false),
    ] {
        let condition = parse_media_condition(source);
        assert_eq!(
            condition.is_some_and(|c| c.matches(&MediaContext::print())),
            print,
            "{source}"
        );
        assert_eq!(
            condition.is_some_and(|c| c.matches(&MediaContext::screen())),
            screen,
            "{source}"
        );
    }
}

#[test]
fn malformed_modifiers_and_unsupported_negation_do_not_match() {
    for source in [
        "not",
        "only",
        "not only screen",
        "only not print",
        "not not print",
        "not and",
        "not or",
        "not layer",
        "not (screen)",
        "not screen print",
        "not screen and (min-width: 300px)",
        "not all and (unknown-feature)",
        "only print and (min-width: 300px)",
        "not screen,",
        "only print, url(\"bad\n\")",
    ] {
        assert_eq!(parse_media_condition(source), None, "{source}");
    }
}

#[test]
fn context_constructors_and_default() {
    assert_eq!(MediaContext::default(), MediaContext::print());
    assert_eq!(MediaContext::print().media_type(), MediaType::Print);
    assert_eq!(MediaContext::screen().media_type(), MediaType::Screen);
    assert_eq!(MediaContext::new(MediaType::Screen), MediaContext::screen());
}

#[test]
fn default_print_viewport_matches_wpt_page_area() {
    let context = MediaContext::print();
    assert_eq!(context.viewport_width(), 384);
    assert_eq!(context.viewport_height(), 192);
}

#[test]
fn parses_viewport_features() {
    let condition = parse_media_condition(
        "(min-width: 4in) and (max-width: 5in) and (min-height: 2in) and (max-height: 3in)",
    )
    .unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.matches(&MediaContext::with_viewport(MediaType::Screen, 800, 600,)));
}

#[test]
fn parses_exact_viewport_features() {
    let condition = parse_media_condition("(width: 100px) and (height: 100px)").unwrap();
    assert!(condition.matches(&MediaContext::with_viewport(MediaType::Print, 100, 100,)));
    assert!(!condition.matches(&MediaContext::print()));
}

#[test]
fn parses_media_type_with_viewport_features() {
    let condition = parse_media_condition("print and (min-width: 300px)").unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.matches(&MediaContext::screen()));
}

#[test]
fn rejects_unsupported_viewport_features() {
    assert_eq!(parse_media_condition("(orientation: landscape)"), None);
    assert_eq!(parse_media_condition("screen and (color)"), None);
}

#[test]
fn parses_supported_media_alternatives() {
    let condition = parse_media_condition("print, projection, screen").unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(condition.matches(&MediaContext::screen()));
    assert_eq!(parse_media_condition("projection"), None);
}

#[test]
fn rejects_features_and_extra_tokens() {
    assert_eq!(parse_media_condition("screen and (color)"), None);
    assert_eq!(parse_media_condition("print screen"), None);
    assert!(parse_media_condition("screen, (min-width: 1px), print").is_some());
}

#[test]
fn nested_commas_and_tokenizer_errors_do_not_leak_supported_names() {
    assert_eq!(parse_media_condition("projection(foo, screen"), None);
    assert_eq!(parse_media_condition("print, url(\"bad\n\")"), None);
    let condition = parse_media_condition("print, (min-width: 1px, 2px)").unwrap();
    assert!(condition.matches(&MediaContext::print()));
}

#[test]
fn rejects_empty_media_alternatives() {
    assert_eq!(parse_media_condition("print,"), None);
    assert_eq!(parse_media_condition(", print"), None);
    assert_eq!(parse_media_condition("print,,screen"), None);
    assert_eq!(parse_media_condition("print, /* comment */"), None);
}

#[test]
fn comments_and_escaped_names_are_tokenized() {
    let condition = parse_media_condition(" /* before */ \\70 rint /*,*/ ").unwrap();
    assert!(condition.matches(&MediaContext::print()));
}
