use super::*;

fn parse_one(source: &str) -> Option<FontFaceRule> {
    let mut rules = parse_font_face_rules(source);
    assert!(
        rules.len() <= 1,
        "test helper expects ≤1 rule, got {}",
        rules.len()
    );
    rules.pop()
}

#[test]
fn ahem_shape_parses() {
    // The exact shape of target/wpt/fonts/ahem.css.
    let rule = parse_one("@font-face { font-family: 'Ahem'; src: url('/fonts/Ahem.ttf'); }")
        .expect("Ahem-shaped rule must parse");
    assert_eq!(rule.family.as_str(), "Ahem");
    assert_eq!(
        rule.src,
        vec![FontFaceSource::Url {
            url: SmolStr::new("/fonts/Ahem.ttf"),
            format: None,
        }]
    );
    assert_eq!(rule.style, FontFaceStyle::Normal);
    assert_eq!(rule.display, FontFaceDisplay::Auto);
    assert!(rule.unicode_range.is_empty());
}

#[test]
fn local_source_parses() {
    let rule = parse_one("@font-face { font-family: Foo; src: local(Times New Roman); }")
        .expect("local() rule must parse");
    assert_eq!(
        rule.src,
        vec![FontFaceSource::Local(SmolStr::new("Times New Roman"))]
    );
}

#[test]
fn multiple_sources_keep_order_with_format_hint() {
    let rule = parse_one(
            "@font-face { font-family: Foo; src: local(\"Foo\"), url(foo.woff2) format(\"woff2\"), url(foo.ttf) format(\"truetype\"); }",
        )
        .expect("multi-source rule must parse");
    assert_eq!(
        rule.src,
        vec![
            FontFaceSource::Local(SmolStr::new("Foo")),
            FontFaceSource::Url {
                url: SmolStr::new("foo.woff2"),
                format: Some(SmolStr::new("woff2")),
            },
            FontFaceSource::Url {
                url: SmolStr::new("foo.ttf"),
                format: Some(SmolStr::new("truetype")),
            },
        ]
    );
}

#[test]
fn tech_hint_is_consumed_not_stored() {
    let rule = parse_one("@font-face { font-family: Foo; src: url(foo.ttf) tech(color-COLRv1); }")
        .expect("tech() must not poison the component");
    assert_eq!(
        rule.src,
        vec![FontFaceSource::Url {
            url: SmolStr::new("foo.ttf"),
            format: None,
        }]
    );
}

#[test]
fn invalid_component_does_not_kill_siblings() {
    let rule = parse_one("@font-face { font-family: Foo; src: bogus-function(1), url(good.ttf); }")
        .expect("rule with one bad component must survive");
    assert_eq!(
        rule.src,
        vec![FontFaceSource::Url {
            url: SmolStr::new("good.ttf"),
            format: None,
        }]
    );
}

#[test]
fn missing_family_or_src_is_invalid() {
    assert!(
        parse_one("@font-face { src: url(a.ttf); }").is_none(),
        "missing font-family must drop the rule"
    );
    assert!(
        parse_one("@font-face { font-family: Foo; }").is_none(),
        "missing src must drop the rule"
    );
    assert!(
        parse_one("@font-face { font-family: ''; src: url(a.ttf); }").is_none(),
        "empty font-family must drop the rule"
    );
    assert!(
        parse_one("@font-face { font-family: Foo; src: bogus(1); }").is_none(),
        "all-invalid src must drop the rule"
    );
}

#[test]
fn statement_form_without_block_is_dropped() {
    assert!(
        parse_one("@font-face;").is_none(),
        "block-less @font-face defines no descriptors"
    );
}

#[test]
fn non_font_face_rules_are_ignored() {
    let rules = parse_font_face_rules(
        "p { color: red; } @media print { p { color: black; } } @page { size: A4; }",
    );
    assert!(rules.is_empty());
}

#[test]
fn prelude_content_invalidates_rule() {
    assert!(
        parse_one("@font-face Foo { font-family: Foo; src: url(a.ttf); }").is_none(),
        "@font-face takes no prelude"
    );
}

#[test]
fn later_descriptors_win() {
    let rule = parse_one(
            "@font-face { font-family: A; font-family: B; src: url(a.ttf); font-style: normal; font-style: italic; font-display: block; font-display: swap; }",
        )
        .expect("rule must parse");
    assert_eq!(rule.family.as_str(), "B");
    assert_eq!(rule.style, FontFaceStyle::Italic);
    assert_eq!(rule.display, FontFaceDisplay::Swap);
}

#[test]
fn weight_forms() {
    let number = parse_one("@font-face { font-family: F; src: url(a.ttf); font-weight: 700; }")
        .expect("number weight must parse");
    assert_eq!(number.weight, FontFaceWeight::Number(700.0));
    let range = parse_one("@font-face { font-family: F; src: url(a.ttf); font-weight: 100 900; }")
        .expect("range weight must parse");
    assert_eq!(range.weight, FontFaceWeight::Range(100.0, 900.0));
    let fallback = parse_one("@font-face { font-family: F; src: url(a.ttf); font-weight: 0; }")
        .expect("out-of-range weight drops the declaration, not the rule");
    assert_eq!(
        fallback.weight,
        FontFaceWeight::Normal,
        "dropped font-weight declaration leaves the default"
    );
}

#[test]
fn unicode_range_parses() {
    let rule = parse_one(
        "@font-face { font-family: F; src: url(a.ttf); unicode-range: U+0025-00FF, U+4??; }",
    )
    .expect("unicode-range must parse");
    assert_eq!(rule.unicode_range, vec![(0x25, 0xFF), (0x400, 0x4FF)]);
}

#[test]
fn registry_origin_precedence() {
    let mut registry = FontFaceRegistry::new();
    let author = FontFaceRule {
        family: SmolStr::new("F"),
        src: vec![FontFaceSource::Url {
            url: SmolStr::new("author.ttf"),
            format: None,
        }],
        style: FontFaceStyle::Normal,
        weight: FontFaceWeight::Normal,
        stretch: FontFaceStretch(SmolStr::new("normal")),
        unicode_range: Vec::new(),
        display: FontFaceDisplay::Auto,
    };
    let mut ua = author.clone();
    ua.src = vec![FontFaceSource::Url {
        url: SmolStr::new("ua.ttf"),
        format: None,
    }];
    registry.insert_with_origin(ua, Origin::UserAgent);
    registry.insert_with_origin(author, Origin::Author);
    assert_eq!(
        registry.get("F").expect("must exist").src,
        vec![FontFaceSource::Url {
            url: SmolStr::new("author.ttf"),
            format: None,
        }]
    );
    // A later UserAgent rule must not clobber the Author entry.
    let mut ua2 = FontFaceRule::new(SmolStr::new("F"));
    ua2.src = vec![FontFaceSource::Url {
        url: SmolStr::new("ua2.ttf"),
        format: None,
    }];
    registry.insert_with_origin(ua2, Origin::UserAgent);
    assert_eq!(
        registry.get("F").expect("must exist").src[0],
        FontFaceSource::Url {
            url: SmolStr::new("author.ttf"),
            format: None,
        }
    );
}

#[test]
fn registry_drops_invalid_on_insert() {
    let mut registry = FontFaceRegistry::new();
    registry.insert(FontFaceRule::new(SmolStr::new("Empty")));
    assert!(registry.is_empty());
}

#[test]
fn unknown_descriptors_are_dropped() {
    let rule = parse_one(
        "@font-face { font-family: F; src: url(a.ttf); size-adjust: 90%; ascent-override: 80%; }",
    )
    .expect("unknown descriptors must not kill the rule");
    assert_eq!(rule.family.as_str(), "F");
    assert_eq!(rule.src.len(), 1);
}
