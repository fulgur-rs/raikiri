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
        let condition = parse_media_prelude(source);
        assert_eq!(
            condition
                .as_ref()
                .is_some_and(|c| c.matches(&MediaContext::print())),
            print,
            "{source}"
        );
        assert_eq!(
            condition
                .as_ref()
                .is_some_and(|c| c.matches(&MediaContext::screen())),
            screen,
            "{source}"
        );
    }
}

#[test]
fn malformed_or_unknown_queries_do_not_match() {
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
        "not all and (unknown-feature)",
        "not screen,",
        "only print, url(\"bad\n\")",
    ] {
        assert_eq!(parse_media_prelude(source), None, "{source}");
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
fn default_print_dimensions_match_wpt_page_box() {
    let context = MediaContext::print();
    assert_eq!(context.viewport_width(), 480);
    assert_eq!(context.viewport_height(), 288);
}

#[test]
fn parses_viewport_features() {
    let condition = parse_media_prelude(
        "(min-width: 4in) and (max-width: 5in) and (min-height: 2in) and (max-height: 3in)",
    )
    .unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.matches(&MediaContext::with_viewport(MediaType::Screen, 800, 600,)));
}

#[test]
fn parses_exact_viewport_features() {
    let condition = parse_media_prelude("(width: 100px) and (height: 100px)").unwrap();
    assert!(condition.matches(&MediaContext::with_viewport(MediaType::Print, 100, 100,)));
    assert!(!condition.matches(&MediaContext::print()));
}

#[test]
fn parses_media_type_with_viewport_features() {
    let condition = parse_media_prelude("print and (min-width: 300px)").unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.matches(&MediaContext::screen()));
}

#[test]
fn rejects_unsupported_viewport_features() {
    assert!(
        !parse_media_prelude("(orientation: landscape)")
            .unwrap()
            .matches(&MediaContext::print())
    );
    assert_eq!(parse_media_prelude("screen and (color)"), None);
}

#[test]
fn parses_supported_media_alternatives() {
    let condition = parse_media_prelude("print, projection, screen").unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(condition.matches(&MediaContext::screen()));
    assert_eq!(parse_media_prelude("projection"), None);
}

#[test]
fn rejects_features_and_extra_tokens() {
    assert_eq!(parse_media_prelude("screen and (color)"), None);
    assert_eq!(parse_media_prelude("print screen"), None);
    assert!(parse_media_prelude("screen, (min-width: 1px), print").is_some());
}

#[test]
fn nested_commas_and_tokenizer_errors_do_not_leak_supported_names() {
    assert_eq!(parse_media_prelude("projection(foo, screen"), None);
    assert_eq!(parse_media_prelude("print, url(\"bad\n\")"), None);
    let condition = parse_media_prelude("print, (min-width: 1px, 2px)").unwrap();
    assert!(condition.matches(&MediaContext::print()));
}

#[test]
fn rejects_empty_media_alternatives() {
    assert_eq!(parse_media_prelude("print,"), None);
    assert_eq!(parse_media_prelude(", print"), None);
    assert_eq!(parse_media_prelude("print,,screen"), None);
    assert_eq!(parse_media_prelude("print, /* comment */"), None);
}

#[test]
fn comments_and_escaped_names_are_tokenized() {
    let condition = parse_media_prelude(" /* before */ \\70 rint /*,*/ ").unwrap();
    assert!(condition.matches(&MediaContext::print()));
}

fn matches_in(source: &str, media_type: MediaType, width: u32, height: u32) -> bool {
    parse_media_prelude(source)
        .is_some_and(|c| c.matches(&MediaContext::with_viewport(media_type, width, height)))
}

#[test]
fn modifiers_apply_to_the_whole_query_with_features() {
    // `not` negates `screen and (min-width: 300px)` as a whole.
    let source = "not screen and (min-width: 300px)";
    assert!(matches_in(source, MediaType::Print, 384, 192));
    assert!(matches_in(source, MediaType::Screen, 200, 600));
    assert!(!matches_in(source, MediaType::Screen, 800, 600));

    let source = "only print and (min-width: 300px)";
    assert!(matches_in(source, MediaType::Print, 384, 192));
    assert!(!matches_in(source, MediaType::Print, 200, 192));
}

#[test]
fn feature_queries_in_a_list_are_evaluated_independently() {
    let source = "screen, print and (min-width: 300px)";
    assert!(matches_in(source, MediaType::Print, 384, 192));
    assert!(!matches_in(source, MediaType::Print, 200, 192));
    assert!(matches_in(source, MediaType::Screen, 200, 600));
}

#[test]
fn keywords_are_case_insensitive_and_comments_are_skipped() {
    let source = "PRINT AND (MIN-WIDTH: 300PX) aNd /* x */ (max-width:/**/400px)";
    assert!(matches_in(source, MediaType::Print, 384, 192));
    assert!(!matches_in(source, MediaType::Print, 401, 192));
}

#[test]
fn and_inside_identifiers_is_not_a_separator() {
    // A name containing "and" must not be split into separate terms.
    assert!(
        !parse_media_prelude("(orientation: landscape)")
            .unwrap()
            .matches(&MediaContext::print())
    );
    assert_eq!(parse_media_prelude("handheld and (min-width: 1px)"), None);
}

#[test]
fn range_syntax() {
    for (source, width, expected) in [
        ("(width >= 300px)", 300, true),
        ("(width > 300px)", 300, false),
        ("(300px < width)", 301, true),
        ("(300px <= width <= 400px)", 400, true),
        ("(300px < width < 400px)", 400, false),
        ("(400px > width > 300px)", 350, true),
        ("(width = 384px)", 384, true),
    ] {
        assert_eq!(
            matches_in(source, MediaType::Print, width, 192),
            expected,
            "{source}"
        );
    }
    // Mixed directions, `min-` names, and a space inside `<=` are invalid.
    for source in [
        "(300px < width > 400px)",
        "(min-width >= 1px)",
        "(width < = 1px)",
    ] {
        assert_eq!(parse_media_prelude(source), None, "{source}");
    }
}

#[test]
fn boolean_or_and_not_conditions() {
    assert!(matches_in("(width)", MediaType::Print, 1, 1));
    assert!(!matches_in("(width)", MediaType::Print, 0, 1));
    assert!(matches_in(
        "(max-width: 100px) or (min-height: 100px)",
        MediaType::Print,
        384,
        192
    ));
    assert!(!matches_in(
        "(max-width: 100px) or (max-height: 100px)",
        MediaType::Print,
        384,
        192
    ));
    assert!(matches_in(
        "not (max-width: 100px)",
        MediaType::Print,
        384,
        192
    ));
    assert!(matches_in(
        "((min-width: 1px) and (min-height: 1px))",
        MediaType::Print,
        384,
        192
    ));
    // `and` and `or` cannot be mixed at one level without parentheses.
    assert_eq!(parse_media_prelude("(width) and (height) or (width)"), None);
}

#[test]
fn unknown_terms_use_three_valued_logic() {
    // true or unknown is true; false and unknown is false.
    assert!(matches_in(
        "(min-width: 1px) or (color)",
        MediaType::Print,
        384,
        192
    ));
    assert!(!matches_in(
        "(max-width: 1px) and (color)",
        MediaType::Print,
        384,
        192
    ));
    // not unknown stays unknown, which is `not all`.
    assert_eq!(parse_media_prelude("not (color)"), None);
    assert_eq!(parse_media_prelude("not all and (unknown-feature)"), None);
    assert_eq!(parse_media_prelude("foo(bar)"), None);
}

#[test]
fn relative_lengths_use_the_initial_font_size() {
    assert!(matches_in("(min-width: 24em)", MediaType::Print, 384, 192));
    assert!(!matches_in(
        "(min-width: 24.1rem)",
        MediaType::Print,
        384,
        192
    ));
    // `lh` has no initial length, so the feature is unknown.
    assert!(
        !parse_media_prelude("(min-width: 1lh)")
            .unwrap()
            .matches(&MediaContext::print())
    );
}

#[test]
fn nested_media_lists_intersect() {
    let outer = parse_media_prelude("print").unwrap();
    let inner = parse_media_prelude("(min-width: 300px)").unwrap();
    let both = outer.intersect(&inner);
    assert!(both.matches(&MediaContext::print()));
    assert!(!both.matches(&MediaContext::screen()));
    assert!(!both.matches(&MediaContext::with_viewport(MediaType::Print, 200, 192)));
    // The intersection links to the outer condition and shares the inner
    // queries instead of copying either, so nesting costs the same at any
    // depth.
    assert!(Arc::ptr_eq(
        both.lists.outer.as_ref().unwrap(),
        &outer.lists
    ));
    assert!(Arc::ptr_eq(&both.lists.queries, &inner.lists.queries));
    let deeper = both.intersect(&outer);
    assert!(Arc::ptr_eq(
        deeper.lists.outer.as_ref().unwrap(),
        &both.lists
    ));
    assert_eq!(deeper.lists().count(), 3);
    // A condition of several lists is linked on list by list, outermost first.
    let chained = outer.intersect(&both);
    let innermost_first: Vec<_> = chained
        .lists()
        .map(|lists| Arc::as_ptr(&lists.queries))
        .collect();
    assert_eq!(
        innermost_first,
        [
            Arc::as_ptr(&inner.lists.queries),
            Arc::as_ptr(&outer.lists.queries),
            Arc::as_ptr(&outer.lists.queries),
        ]
    );
    assert!(chained.matches(&MediaContext::with_viewport(MediaType::Print, 400, 400)));
    assert!(!chained.matches(&MediaContext::with_viewport(MediaType::Print, 200, 192)));
}

#[test]
fn long_chains_compare_format_and_drop_without_deep_recursion() {
    // Far deeper than a test thread's stack allows one frame per list.
    let print = parse_media_prelude("print").unwrap();
    let mut chain = print.clone();
    for _ in 0..200_000 {
        chain = chain.intersect(&print);
    }
    let same = chain.clone();
    assert_eq!(chain, same);
    assert_ne!(chain, chain.intersect(&print));
    assert_ne!(chain, print);
    drop(same);
    drop(chain);
    // Through the public entry point, as nested import conditions build it.
    let media = vec!["print"; 100_000];
    let mut tree = crate::ruletree::RuleTree::empty();
    tree.add_stylesheet_with_media_conditions(
        "a { color: red }",
        crate::ruletree::Origin::Author,
        &media,
    );
    drop(tree);
    // Formatting lists the outermost list first, as the conditions nest.
    let width = parse_media_prelude("(min-width: 300px)").unwrap();
    let text = format!("{:?}", print.intersect(&width));
    assert!(
        text.find("Typed").unwrap() < text.find("Condition(").unwrap(),
        "{text}"
    );
}

#[test]
fn negative_lengths_are_valid_and_false_in_the_negative_range() {
    // MQ4 §2.4.3: negative values parse, and `=`, `<`, `<=` against them are
    // false because the viewport is never negative.
    assert!(matches_in("(min-width: -1px)", MediaType::Print, 0, 0));
    assert!(!matches_in("(max-width: -1px)", MediaType::Print, 0, 0));
    assert!(!matches_in("(width = -1px)", MediaType::Print, 0, 0));
    assert!(!matches_in("(width < -1px)", MediaType::Print, 0, 0));
}

#[test]
fn error_tokens_anywhere_invalidate_the_list() {
    // An error token invalidates the whole list wherever it appears: at the
    // top level, inside a feature, or nested inside `<general-enclosed>`.
    for source in [
        "print, url(\"bad\n\")",
        "print, (min-width: \"bad\n)",
        "print, (foo [\"bad\n])",
        "print, (min-width: 1px) or (foo (bar \"bad\n))",
        "print, screen )",
    ] {
        assert_eq!(parse_media_prelude(source), None, "{source:?}");
    }
    // The same lists without the error token keep `print`.
    for source in [
        "print, (foo [bar])",
        "print, (min-width: 1px) or (foo (bar baz))",
    ] {
        let condition = parse_media_prelude(source).unwrap();
        assert!(condition.matches(&MediaContext::print()), "{source:?}");
    }
}

#[test]
fn paper_dimension_provenance_uses_valid_retained_features() {
    for (source, dependent) in [
        ("print", false),
        ("print, (orientation: landscape)", true),
        ("print, (min-device-aspect-ratio: 1/2)", true),
        ("print, (200px < width <= 300px)", true),
        ("print, (calc(100px) < width)", true),
        ("print, (width:100vw)", true),
        ("print, (width:1lh)", true),
        ("print, (device-width:0)", true),
        ("print, (aspect-ratio:0)", true),
        ("print, (aspect-ratio:calc(2))", true),
        ("print, (aspect-ratio:1 / calc(2))", true),
        ("print, (calc(1) / calc(2) < aspect-ratio)", true),
        ("print, (orientation)", true),
        ("print, (device-height >= 1px)", true),
        ("print, (2 > aspect-ratio >= 1)", true),
        ("print, (width:1cap)", true),
        ("print, (width:calc(1rex + 1rch + 1ric + 1rcap))", true),
        ("print, (aspect-ratio > 1/2)", true),
        ("print, (1/2 < aspect-ratio < 3/2)", true),
        ("print, (width:calc(100px + 1em))", true),
        ("print, (width:calc(1px/2))", true),
        ("print, (device-width:calc(1px/2))", true),
        ("print, (width:clamp(1px, 2em, 3px))", true),
        ("print, (width:min(1px))", true),
        ("print, (width:max(1px, 2em))", true),
        ("print, (100% < width)", false),
        ("print, (1/2 < aspect-ratio)", true),
        ("print, (unknown-feature < width)", false),
        ("print, (\"width\")", false),
        ("print, ((hover:width) or (width:261px))", true),
        ("print, (not (width:200px))", true),
        (r"print, (\77 idth: 200px)", true),
        ("print, (hover: width)", false),
        ("print, (unknown-feature: (width:200px))", false),
        ("print, (unknown-feature: 0 < width)", false),
        ("print, (unknown-feature: 'width')", false),
        ("print, (unknown-feature: calc(width))", false),
        ("print, unknown(width)", false),
        ("print /* (width: 200px) */", false),
        ("print, only (width:200px)", false),
        ("print, print and (width:200px) unexpected", false),
    ] {
        assert_eq!(
            parse_media_prelude(source).unwrap().depends_on_paper_size(),
            dependent,
            "{source}"
        );
    }
}

#[test]
fn invalid_paper_feature_values_do_not_qualify_matching_query_arms() {
    for source in [
        "print, (width: red)",
        "print, (width: red blue)",
        "print, (width: [1px])",
        "print, (orientation > landscape)",
        "print, (min-device-width > 1px)",
        "print, (1px < min-device-width)",
        "print, (1px < orientation)",
        "print, (1 < aspect-ratio > 2)",
        "print, (1 = aspect-ratio < 2)",
        "print, (1 < aspect-ratio < red)",
        "print, (aspect-ratio: -1)",
        "print, (aspect-ratio: 1/red)",
        "print, (aspect-ratio: calc(1px))",
        "print, (aspect-ratio: calc(1 + var(--x)))",
        "print, (min-device-height)",
        "print, (width: 100%)",
        "print, (width < red)",
        "print, (100% < width)",
        "print, (orientation: sideways)",
        "print, (min-orientation: landscape)",
        "print, (aspect-ratio: red)",
        "print, (aspect-ratio: pi)",
        "print, (aspect-ratio: 1/-2)",
        "print, (width: calc(red))",
        "print, (width: clamp(1px))",
        "print, (width: 1px/2)",
        "print, (device-width: 1px/2)",
        "print, (width: clamp(1px, 2px))",
        "print, (width: clamp(1px, 2px, 3px, 4px))",
        "print, (width: min())",
        "print, (width: calc(1px, 2px))",
        "print, (width: mod(1px))",
        "print, (width: round(sideways, 1px, 1px))",
        "print, (width: calc(1deg))",
        "print, (width: calc(1px * 1px))",
        "print, (width: calc(1px + 1%))",
        "print, (width: calc(0))",
        "print, (width: var(--x))",
        "print, (width: calc(1px + var(--x)))",
    ] {
        let condition = parse_media_prelude(source).unwrap();
        assert!(condition.matches(&MediaContext::print()));
        assert!(!condition.depends_on_paper_size(), "{source}");
    }
}

#[test]
fn never_matching_media_types_do_not_qualify_matching_query_arms() {
    for source in [
        "print, projection and (width: 1px)",
        "print, only projection and (orientation: landscape)",
    ] {
        let condition = parse_media_prelude(source).unwrap();
        assert!(condition.matches(&MediaContext::print()));
        assert!(!condition.depends_on_paper_size(), "{source}");
    }
}

#[test]
fn excessive_paper_math_nesting_remains_general_enclosed() {
    let value = format!("{}1px{}", "calc(".repeat(130), ")".repeat(130));
    let condition = parse_media_prelude(&format!("print, (width:{value})")).unwrap();
    assert!(condition.matches(&MediaContext::print()));
    assert!(!condition.depends_on_paper_size());
}
