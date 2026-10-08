use super::{
    INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR, InitialParseSelectorBudget,
    MAX_INITIAL_PARSE_DECLARATION_STORAGE_BYTES, MAX_INITIAL_PARSE_XML_DEPTH, ParsedCssDeclaration,
    SelectorFreezeBudget, SourceRootStyle, SvgDocument, SvgError, SvgRootStyle, SvgViewport,
    append_inline_declarations, apply_css_rewrite, apply_selector_edits,
    charge_initial_parse_node_count, estimate_initial_stylesheet_resources,
    freeze_svg_stylesheet_selectors, is_simplecss_name_start, normalize_svg_opacity_cascade,
    preflight_initial_svg_selectors, scope_stylesheet_properties, scoped_property_rule_len,
    simplecss_comment_end, simplecss_function_end, simplecss_recovery_declaration_tokens,
    simplecss_recovery_delimiter_counts, simplecss_recovery_rule_body_end,
    simplecss_recovery_rule_body_requires_raw_recovery,
    simplecss_recovery_selector_header_requires_raw_recovery,
    simplecss_recovery_selector_segment_is_valid, simplecss_recovery_string_end,
    simplecss_string_end, skip_simplecss_at_rule, skip_simplecss_spaces_and_comments,
    strip_inline_style_properties, strip_stylesheet_properties, unique_attribute_name,
    with_root_style_overrides, xml_attribute_escape_allocation_bytes,
};

const HALF_RED_RECT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

#[test]
fn styled_svg_source_preserves_inherited_opacity_without_rasterizing() {
    let svg = SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="currentColor" opacity="inherit"/></svg>"#).unwrap();
    let viewport = SvgViewport {
        width: 2.0,
        height: 1.0,
    };
    let source = svg
        .styled_source(
            viewport,
            SvgRootStyle {
                inherited_color: [0, 0, 255, 255],
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
        )
        .unwrap();
    let prepared = SvgDocument::parse(source.as_bytes()).unwrap();
    let image = prepared
        .rasterize(viewport, SvgRootStyle::default(), None)
        .unwrap();
    assert_eq!(&image.rgba[..4], &[0, 0, 255, 64]);
}

fn preflight_initial_svg(source: &str) -> Result<(), SvgError> {
    let xml = roxmltree::Document::parse(source).expect("test SVG is valid XML");
    preflight_initial_svg_selectors(&xml)
}

fn selector_freezing_svg(css: &str, elements: usize) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><style>{css}</style>{}</svg>",
        "<rect width=\"1\" height=\"1\"/>".repeat(elements)
    )
}

fn assert_selector_freezing_rejected(source: &str) {
    let svg = SvgDocument::parse(source.as_bytes()).expect("supported original SVG");
    let viewport = SvgViewport {
        width: 1.0,
        height: 1.0,
    };
    for css_pixel_scale in [false, true] {
        let result = if css_pixel_scale {
            svg.rasterize_at_css_pixel_scale(viewport, SvgRootStyle::default(), Some(4))
        } else {
            svg.rasterize(viewport, SvgRootStyle::default(), Some(4))
        };
        assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
            if message.contains("selector freezing resource limit")));
    }
}

#[test]
fn initial_stylesheet_estimator_skips_at_rules_and_unfinished_headers() {
    let source = "@import url('theme.css'); @media screen { @supports (display:grid) { rect { fill:red } } } rect { fill:blue }";
    let estimate = estimate_initial_stylesheet_resources(source).unwrap();
    assert_eq!(estimate.selector_count, 1);

    let unfinished = estimate_initial_stylesheet_resources("rect").unwrap();
    assert_eq!(unfinished.selector_count, 1);

    let mut budget = InitialParseSelectorBudget::new();
    budget.charge_stylesheet_sort_work(0).unwrap();
    budget.charge_stylesheet_sort_work(1).unwrap();

    assert!(is_simplecss_name_start(Some(b'A')));
    assert!(is_simplecss_name_start(Some(b'_')));
    assert!(is_simplecss_name_start(Some(0xF0)));
    assert!(!is_simplecss_name_start(Some(b'1')));
    assert!(!is_simplecss_name_start(None));

    let semicolon_rule = b"@import url(x);tail";
    assert_eq!(
        skip_simplecss_at_rule(semicolon_rule, 0),
        semicolon_rule
            .iter()
            .position(|byte| *byte == b';')
            .unwrap()
            + 1
    );
    let block_rule = b"@media { nested { } }tail";
    assert_eq!(
        skip_simplecss_at_rule(block_rule, 0),
        block_rule
            .windows(4)
            .position(|bytes| bytes == b"tail")
            .unwrap()
    );
    let unclosed_block_rule = b"@media { nested {";
    assert_eq!(
        skip_simplecss_at_rule(unclosed_block_rule, 0),
        unclosed_block_rule.len()
    );
    assert_eq!(skip_simplecss_at_rule(b"@unfinished", 0), 11);
}

#[test]
fn initial_stylesheet_estimator_accounts_for_comments_strings_functions_and_blocks() {
    for stylesheet in [
        "rect /* comment */ { fill:red /* declaration comment */; color:rgb(1,2,3) }",
        "rect /* { */ { fill:red }",
        "rect:is(g) { fill:red }",
        "rect[data-label='text'] { content:'quoted'; color:rgb(1,2,3) }",
        "rect[data-label='}'] { fill:red }",
        "rect:is(g}) { fill:red }",
        "rect { content:'}'; fill:red }",
        "rect { fill:fn(}); color:red }",
        "rect { nested:{ value:red; }; fill:blue }",
        "rect /* { unterminated",
        "rect /* unterminated",
        "rect { fill:red /* } */; color:blue }",
        "rect { fill:red /* }",
        "rect { fill:red /* unterminated",
        "rect { content:'unterminated",
        "rect { fill:fn(unterminated",
    ] {
        estimate_initial_stylesheet_resources(stylesheet)
            .unwrap_or_else(|error| panic!("estimation rejected {stylesheet:?}: {error}"));
    }
}

#[test]
fn initial_stylesheet_recovery_suffix_keeps_selector_and_declaration_prefixes() {
    let aliases = std::iter::repeat_n("g", 260).collect::<Vec<_>>().join(",");
    let selector_suffix = format!(r#"{aliases},[id="}}"] {{ fill:red; }}"#);
    let selector_estimate = estimate_initial_stylesheet_resources(&selector_suffix).unwrap();
    assert!(selector_estimate.selector_count >= 261);
    assert!(
        selector_estimate.declaration_storage_bytes
            >= 261
                * std::mem::size_of::<simplecss::Declaration<'static>>()
                * INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR
    );

    let declarations = format!("{}content:'}}'", "fill:red;".repeat(3_000));
    let declaration_suffix = format!("{aliases} {{{declarations}}}");
    let declaration_estimate = estimate_initial_stylesheet_resources(&declaration_suffix).unwrap();
    assert!(
        declaration_estimate.declaration_storage_bytes
            > MAX_INITIAL_PARSE_DECLARATION_STORAGE_BYTES
    );

    let single_alias_declarations = format!("{}content:'}}'", "fill:red;".repeat(100));
    let single_alias_suffix = format!("g {{{single_alias_declarations}}}");
    let single_alias_estimate =
        estimate_initial_stylesheet_resources(&single_alias_suffix).unwrap();
    let prefix_storage = 2
        * 101
        * std::mem::size_of::<simplecss::Declaration<'static>>()
        * INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR;
    assert!(single_alias_estimate.declaration_storage_bytes >= prefix_storage);
}

#[test]
fn simplecss_recovery_scanners_cover_fallback_tokens_and_malformed_inputs() {
    let valid_suffix = b"\"}\"],g { fill:red; } @media screen { rect { fill:blue } }";
    assert!(simplecss_recovery_delimiter_counts(valid_suffix, true).is_some());

    assert!(simplecss_recovery_delimiter_counts(b"\"}\"]{fill:@;}", true).is_none());
    assert!(simplecss_recovery_delimiter_counts(b"g { fill:fn(", true).is_none());
    assert_eq!(
        simplecss_recovery_selector_header_requires_raw_recovery(b"g", 0),
        None
    );
    assert!(simplecss_recovery_selector_segment_is_valid(
        "g:first-child"
    ));
    assert!(!simplecss_recovery_selector_segment_is_valid(
        ":unsupported"
    ));

    assert_eq!(simplecss_recovery_rule_body_end(b"fill:{x};}", 0), Some(9));
    assert_eq!(simplecss_recovery_rule_body_end(b"fill:red", 0), None);
    assert_eq!(simplecss_recovery_string_end(b"'unfinished", 0), None);
    assert_eq!(
        simplecss_recovery_rule_body_requires_raw_recovery(b"fill:@;}", 0),
        Some(true)
    );
    assert_eq!(
        simplecss_recovery_rule_body_requires_raw_recovery(b"fill:red@ /* },g{fill:blue} */ }", 0),
        Some(true)
    );
    assert_eq!(
        simplecss_recovery_declaration_tokens("fill:red ! important /* note */ ;; color:blue;")
            .unwrap(),
        (2, true)
    );
    assert_eq!(
        simplecss_recovery_declaration_tokens("fill:red !unknown;").unwrap(),
        (0, false)
    );
    assert_eq!(
        simplecss_recovery_declaration_tokens("/* unfinished").unwrap(),
        (0, false)
    );
}

#[test]
fn simplecss_scanner_helpers_handle_terminated_and_unterminated_tokens() {
    assert_eq!(simplecss_comment_end(b"/*x*/", 0), Some(5));
    assert_eq!(simplecss_comment_end(b"/*x", 0), None);

    let mut cursor = 0;
    assert!(skip_simplecss_spaces_and_comments(
        b" \t/*x*/ ",
        &mut cursor
    ));
    assert_eq!(cursor, 8);
    let mut unterminated_comment = 0;
    assert!(!skip_simplecss_spaces_and_comments(
        b"/*x",
        &mut unterminated_comment
    ));
    assert_eq!(unterminated_comment, 3);

    assert_eq!(simplecss_string_end(b"'a\\'b'", 0), 6);
    assert_eq!(simplecss_string_end(b"'unterminated", 0), 13);
    assert_eq!(simplecss_function_end(b"fn(x)", 1), 5);
    assert_eq!(simplecss_function_end(b"fn(x", 1), 4);
}

#[test]
fn initial_svg_preflight_handles_empty_styles_foreign_nodes_and_use_reference_forms() {
    let source = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:f=\"urn:foreign\" id=\"root\"><style/><style type=\"text/plain\">not CSS</style><style>g {{ fill:red }}</style><f:rect/><g id=\"target\"/><g id=\"target\"/><use href=\"#target\"/><use xlink:href=\"#target\"/><use/><use href=\"#missing\"/><use id=\"self\" href=\"#self\"/><use href=\"#root\"/></svg>"
    );

    preflight_initial_svg(&source).unwrap();
}

#[test]
fn initial_svg_preflight_stops_recursive_use_expansion_and_excessive_depth() {
    let recursive = format!(
        "<svg xmlns=\"{SVG_NAMESPACE}\"><style>g {{ fill:red }}</style><g id=\"template\"><use href=\"#template\"/></g><use href=\"#template\"/></svg>"
    );
    preflight_initial_svg(&recursive).unwrap();

    let mut nested = format!("<svg xmlns=\"{SVG_NAMESPACE}\"><style>g {{ fill:red }}</style>");
    nested.push_str(&"<g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH));
    nested.push_str(&"</g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH));
    nested.push_str("</svg>");
    let error = preflight_initial_svg(&nested).unwrap_err();
    assert!(matches!(error, SvgError::InvalidDocument(ref message)
        if message.contains("selector matching resource limit")));
}

#[test]
fn initial_xml_depth_scanner_skips_non_element_markup_and_enforces_limit() {
    super::ensure_initial_svg_xml_depth_bounded(
        "<svg><!-- <g> --><![CDATA[<rect/>]]><?probe <use/> ?><g title=\">\"/></svg>",
    )
    .unwrap();
    super::ensure_initial_svg_xml_depth_bounded("<svg/> ").unwrap();
    super::ensure_initial_svg_xml_depth_bounded("<!DOCTYPE svg><svg/>").unwrap();
    super::ensure_initial_svg_xml_depth_bounded("<svg").unwrap();
    super::ensure_initial_svg_xml_depth_bounded("<svg><!-- unfinished").unwrap();
    super::ensure_initial_svg_xml_depth_bounded("</orphan><svg/>").unwrap();

    let mut nested = "<svg>".to_owned();
    nested.push_str(&"<g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH - 1));
    nested.push_str(&"</g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH - 1));
    nested.push_str("</svg>");
    assert!(super::ensure_initial_svg_xml_depth_bounded(&nested).is_ok());

    let mut excessive = "<svg>".to_owned();
    excessive.push_str(&"<g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH));
    excessive.push_str(&"</g>".repeat(MAX_INITIAL_PARSE_XML_DEPTH));
    excessive.push_str("</svg>");
    assert!(super::ensure_initial_svg_xml_depth_bounded(&excessive).is_err());
}

#[test]
fn initial_parse_node_counter_checks_limits_and_overflow() {
    let mut count = 0;
    charge_initial_parse_node_count(&mut count, 1).unwrap();
    assert_eq!(count, 1);
    assert!(charge_initial_parse_node_count(&mut count, 1).is_err());

    let mut overflow = usize::MAX;
    assert!(charge_initial_parse_node_count(&mut overflow, usize::MAX).is_err());
}

#[test]
fn selector_freezing_rejects_excessive_matches_with_a_one_pixel_output() {
    let source = selector_freezing_svg(&"rect { fill:red }".repeat(257), 256);
    assert_selector_freezing_rejected(&source);
}

#[test]
fn selector_freezing_rejects_amplified_declarations_from_selector_aliases() {
    let selectors = vec!["rect"; 128].join(",");
    let css = format!("{selectors} {{ fill:red; unused:{} }}", "a".repeat(70_000));
    assert_selector_freezing_rejected(&selector_freezing_svg(&css, 1));
}

#[test]
fn selector_freezing_keeps_ordinary_aliases_and_cascade_pixels() {
    let source = selector_freezing_svg(
        "rect, svg rect { fill:blue } rect { fill:red!important }",
        2,
    );
    let svg = SvgDocument::parse(source.as_bytes()).expect("ordinary SVG");
    let viewport = SvgViewport {
        width: 1.0,
        height: 1.0,
    };
    for image in [
        svg.rasterize(viewport, SvgRootStyle::default(), Some(4)),
        svg.rasterize_at_css_pixel_scale(viewport, SvgRootStyle::default(), Some(4)),
    ] {
        assert_eq!(
            image.expect("ordinary freezing succeeds").rgba,
            [255, 0, 0, 255]
        );
    }
}

#[test]
fn selector_freezing_shares_limits_across_style_elements() {
    let source = selector_freezing_svg(&"rect { fill:red }".repeat(128), 256).replace(
        "</style>",
        &format!("</style><style>{}</style>", "rect { fill:red }".repeat(129)),
    );
    assert_selector_freezing_rejected(&source);
}

#[test]
fn selector_freezing_charges_xml_escape_expansion_before_allocation() {
    let selectors = vec!["rect"; 128].join(",");
    let css = format!(
        "<![CDATA[{selectors} {{ fill:red; unused:'{}' }}]]>",
        "&".repeat(20_000)
    );
    assert_selector_freezing_rejected(&selector_freezing_svg(&css, 1));
}

#[test]
fn selector_freezing_limits_nonmatching_selector_work() {
    assert_selector_freezing_rejected(&selector_freezing_svg(
        &"missing { fill:red }".repeat(1025),
        1024,
    ));
}

#[test]
fn selector_freezing_rejects_excessive_descendant_matcher_work() {
    let selectors = [
        format!("z {}", ["*"; 12].join(" ")),
        format!("[data-never] {}", ["*"; 12].join(" ")),
    ];
    for selector in selectors {
        let mut source = "<svg>".to_owned();
        for _ in 0..12 {
            source.push_str("<g>");
        }
        for _ in 0..12 {
            source.push_str("</g>");
        }
        source.push_str(&format!("<style>{selector} {{ fill:red }}</style></svg>"));
        let mut budget = SelectorFreezeBudget {
            bytes: usize::MAX,
            matches: usize::MAX,
            checks: 100,
        };

        let result = freeze_svg_stylesheet_selectors(&source, &mut budget);

        assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
            if message.contains("selector freezing resource limit")));
    }
}

#[test]
fn selector_freezing_limits_deep_child_matcher_recursion() {
    let selector = std::iter::once("z")
        .chain(std::iter::repeat_n("g", 140))
        .collect::<Vec<_>>()
        .join(" > ");
    let mut source = "<svg>".to_owned();
    for _ in 0..140 {
        source.push_str("<g>");
    }
    for _ in 0..140 {
        source.push_str("</g>");
    }
    source.push_str(&format!("<style>{selector} {{ fill:red }}</style></svg>"));
    let mut budget = SelectorFreezeBudget {
        bytes: usize::MAX,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = freeze_svg_stylesheet_selectors(&source, &mut budget);

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn selector_freezing_budget_survives_opacity_normalization() {
    let css = format!("rect {{ opacity:0.5{} }}", "0".repeat(60_000));
    let source = selector_freezing_svg(&css, 600);
    let svg = SvgDocument::parse(source.as_bytes()).expect("supported long opacity token");
    let viewport = SvgViewport {
        width: 1.0,
        height: 1.0,
    };
    let style = SvgRootStyle {
        neutralize_root_opacity: true,
        ..SvgRootStyle::default()
    };
    for result in [
        svg.rasterize(viewport, style, Some(4)),
        svg.rasterize_at_css_pixel_scale(viewport, style, Some(4)),
    ] {
        assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
            if message.contains("selector freezing resource limit")));
    }
}

#[test]
fn selector_freezing_budget_covers_xml_escape_and_opacity_stylesheet_copy() {
    let payload = "<>&".repeat(400_000);
    let css = format!("<![CDATA[rect {{ fill:'{payload}'; opacity:0.5 }}]]>");
    let source = selector_freezing_svg(&css, 1);
    let svg = SvgDocument::parse(source.as_bytes()).expect("supported large SVG stylesheet");
    let result = svg.rasterize(
        SvgViewport {
            width: 1.0,
            height: 1.0,
        },
        SvgRootStyle {
            neutralize_root_opacity: true,
            ..SvgRootStyle::default()
        },
        Some(4),
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn selector_edit_application_rejects_overlapping_ranges() {
    let result = apply_selector_edits(
        "abc",
        vec![(0..2, "x".to_owned()), (1..3, "y".to_owned())],
        &mut SelectorFreezeBudget::new(),
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message == "overlapping SVG selector edits"));
}

#[test]
fn root_style_rewrite_obeys_the_shared_selector_match_budget() {
    let source = "<svg><style>svg { background:red }</style><rect/><rect/></svg>";
    let mut budget = SelectorFreezeBudget {
        bytes: usize::MAX,
        matches: 1,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_charges_scoped_css_expansion_before_allocation() {
    let selectors = ["rect"; 12].join(",");
    let source = format!("<svg><style>{selectors} {{ background:red }}</style><rect/></svg>");
    let mut budget = SelectorFreezeBudget {
        bytes: source.len() * 8,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        &source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_rejects_budget_exhaustion_before_removing_root_attributes() {
    let source = "<svg background=\"red\"/>";
    let mut budget = SelectorFreezeBudget {
        bytes: source.len(),
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_rejects_budget_exhaustion_before_copying_existing_style() {
    let source = "<svg style=\"color:red\"/>";
    let mut budget = SelectorFreezeBudget {
        bytes: source.len(),
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        true,
        false,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_rejects_budget_exhaustion_before_replacing_existing_style() {
    let source = "<svg style=\"color:red\"/>";
    let original_style = "color:red";
    let retained = strip_inline_style_properties(original_style, &["opacity"]);
    let rewritten_style = append_inline_declarations(&retained, "opacity:1");
    let budget_bytes = source.len()
        + original_style.len() * 8
        + xml_attribute_escape_allocation_bytes(&rewritten_style).unwrap();
    let mut budget = SelectorFreezeBudget {
        bytes: budget_bytes,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        true,
        false,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_rejects_budget_exhaustion_before_copying_stylesheet_text() {
    let source = "<svg><style>rect { background:red }</style></svg>";
    let scope_attribute = "data-raikiri-root-opacity-scope";
    let mut budget = SelectorFreezeBudget {
        bytes: source.len() + scope_attribute.len(),
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn root_style_rewrite_rejects_budget_exhaustion_before_copying_scope_attributes() {
    let source = "<svg><style>rect { background:red }</style><rect/></svg>";
    let mut full_budget = SelectorFreezeBudget {
        bytes: usize::MAX,
        matches: usize::MAX,
        checks: usize::MAX,
    };
    let rewritten = with_root_style_overrides(
        source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut full_budget,
    )
    .unwrap();
    let bytes_through_scope_rewrite = usize::MAX - full_budget.bytes - rewritten.len() - 1;
    let mut limited_budget = SelectorFreezeBudget {
        bytes: bytes_through_scope_rewrite,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = with_root_style_overrides(
        source,
        1.0,
        false,
        true,
        SourceRootStyle::default(),
        &mut limited_budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn opacity_normalizer_skips_non_css_and_empty_style_elements() {
    let source = "<svg><style type=\"text/plain\">opacity:.5</style><style><!-- empty --></style><style><![CDATA[]]></style></svg>";
    let rewritten =
        normalize_svg_opacity_cascade(source, &mut SelectorFreezeBudget::new()).unwrap();

    assert_eq!(rewritten, source);
}

#[test]
fn stylesheet_rewrite_charges_each_removed_range_as_work() {
    let mut budget = SelectorFreezeBudget {
        bytes: usize::MAX,
        matches: usize::MAX,
        checks: 1,
    };

    let result = strip_stylesheet_properties(
        "rect { opacity:.25; color:red; opacity:.5 }",
        &["opacity"],
        &mut budget,
    );

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn stylesheet_rewrite_stops_when_declaration_storage_exceeds_budget() {
    let source = "rect{background:red}";
    let prelude_storage = "rect".len() + 2 * std::mem::size_of::<String>();
    let mut budget = SelectorFreezeBudget {
        bytes: prelude_storage,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = scope_stylesheet_properties(source, "scope", &["background"], &mut budget);

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
}

#[test]
fn stylesheet_rewrite_stops_before_allocating_scoped_declaration_references() {
    let source = "rect{background:red}";
    let prelude_storage = "rect".len() + 2 * std::mem::size_of::<String>();
    let declaration_storage =
        "background".len() + "red".len() + 2 * std::mem::size_of::<ParsedCssDeclaration>();
    let mut budget = SelectorFreezeBudget {
        bytes: prelude_storage
            + declaration_storage
            + 2 * std::mem::size_of::<&ParsedCssDeclaration>()
            - 1,
        matches: usize::MAX,
        checks: usize::MAX,
    };

    let result = scope_stylesheet_properties(source, "scope", &["background"], &mut budget);

    assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
        if message.contains("selector freezing resource limit")));
    assert!(budget.bytes < 2 * std::mem::size_of::<&ParsedCssDeclaration>());
}

#[test]
fn stylesheet_rewrite_rejects_invalid_removal_ranges() {
    let invalid_ranges = [vec![0..1, 0..1], std::iter::once(2..4).collect::<Vec<_>>()];
    for removals in invalid_ranges {
        let result = apply_css_rewrite(
            "abc",
            removals,
            Vec::new(),
            &mut SelectorFreezeBudget::new(),
        );

        assert!(matches!(result, Err(SvgError::InvalidDocument(ref message))
            if message == "overlapping SVG CSS rewrite ranges"));
    }
}

#[test]
fn scoped_property_rule_omits_selectors_without_a_scopeable_component() {
    assert_eq!(
        scoped_property_rule_len(
            &["/* only a comment */"],
            "scope",
            &["background:red".into()]
        ),
        None
    );
}

#[test]
fn marker_prefix_collision_search_skips_all_existing_numeric_suffixes() {
    let base = "data-raikiri-root-opacity-scope";
    let source = format!("{base} {base}-2");
    let mut budget = SelectorFreezeBudget::new();

    assert_eq!(
        unique_attribute_name(&source, base, &mut budget).unwrap(),
        format!("{base}-3")
    );
}

#[test]
fn marker_prefix_collision_search_skips_suffixes_without_digits() {
    let base = "data-raikiri-root-opacity-scope";
    let source = format!("{base}- {base}-text {base}-2");
    let mut budget = SelectorFreezeBudget::new();

    assert_eq!(
        unique_attribute_name(&source, base, &mut budget).unwrap(),
        format!("{base}-3")
    );
}

#[test]
fn selector_freezing_accepts_the_match_limit_and_preserves_skip_paths() {
    let source = selector_freezing_svg(&"rect { fill:red }".repeat(256), 256);
    let svg = SvgDocument::parse(source.as_bytes()).expect("exact-limit SVG");
    let viewport = SvgViewport {
        width: 1.0,
        height: 1.0,
    };
    assert_eq!(
        svg.rasterize(viewport, SvgRootStyle::default(), Some(4))
            .unwrap()
            .rgba,
        [255, 0, 0, 255]
    );

    let excessive = selector_freezing_svg(&"rect { fill:red }".repeat(257), 256);
    let svg = SvgDocument::parse(excessive.as_bytes()).unwrap();
    for style in [
        SvgRootStyle {
            visible: false,
            ..SvgRootStyle::default()
        },
        SvgRootStyle {
            opacity: 0.0,
            ..SvgRootStyle::default()
        },
    ] {
        assert_eq!(
            svg.rasterize(viewport, style, Some(4)).unwrap().rgba,
            [0; 4]
        );
    }
    let unchanged = excessive.replacen("<svg ", "<svg color=\"red\" ", 1);
    let svg = SvgDocument::parse(unchanged.as_bytes()).unwrap();
    assert_eq!(
        svg.rasterize(viewport, SvgRootStyle::default(), Some(4))
            .unwrap()
            .rgba,
        [255, 0, 0, 255]
    );
}

#[test]
fn rasterizes_at_requested_size_and_returns_straight_alpha_rgba() {
    let svg = SvgDocument::parse(HALF_RED_RECT).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 4.0,
                height: 2.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("bounded rasterization succeeds");

    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 128]);
    assert_eq!(&image.rgba[8..12], &[0, 0, 0, 0]);
}

#[test]
fn root_preserve_aspect_ratio_uses_the_requested_viewport() {
    let meet = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let stretched = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" preserveAspectRatio="none"><circle cx="50" cy="50" r="40" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let viewport = SvgViewport {
        width: 200.0,
        height: 100.0,
    };

    let meet = meet
        .rasterize(viewport, SvgRootStyle::default(), None)
        .expect("meet rasterization succeeds");
    let stretched = stretched
        .rasterize(viewport, SvgRootStyle::default(), None)
        .expect("none rasterization succeeds");
    let pixel = |image: &raikiri_traits::DecodedImage, x: usize, y: usize| {
        let index = (y * image.width as usize + x) * 4;
        [
            image.rgba[index],
            image.rgba[index + 1],
            image.rgba[index + 2],
            image.rgba[index + 3],
        ]
    };

    assert_eq!((meet.width, meet.height), (200, 100));
    assert_eq!(pixel(&meet, 140, 50), [0, 0, 0, 0]);
    assert_eq!(pixel(&stretched, 140, 50), [255, 0, 0, 255]);
}

#[test]
fn viewport_override_replaces_important_inline_dimensions() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" style="color:green!important;width:100px!important;height:100px!important"><style>svg { width:100px!important;height:100px!important }</style><rect width="100" height="100" fill="#ff0000"/></svg>"##;
    let resized = super::with_root_viewport_size(
        std::str::from_utf8(source).expect("SVG is UTF-8"),
        200.0,
        100.0,
    )
    .expect("viewport override succeeds");
    let tree = super::parse_tree(&resized).expect("adjusted SVG parses");

    assert!(resized.contains("color:green!important"));
    assert_eq!(tree.size().width(), 200.0);
    assert_eq!(tree.size().height(), 100.0);
}

#[test]
fn equal_ratio_viewport_resize_preserves_absolute_geometry_without_view_box() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 200.0,
                height: 200.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("rasterization succeeds");

    let alpha_at = |x: usize, y: usize| image.rgba[(y * image.width as usize + x) * 4 + 3];
    assert_eq!(alpha_at(5, 5), 255);
    assert_eq!(alpha_at(15, 5), 0);
}

#[test]
fn intrinsic_dimensions_are_independent_of_raster_size() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in" viewBox="0 0 2 1"/>"#,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 7.0,
                height: 3.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(svg.intrinsic_size().width, Some(192.0));
    assert_eq!(svg.intrinsic_size().height, Some(96.0));
    assert_eq!(svg.intrinsic_size().aspect_ratio, Some(2.0));
    assert_eq!((image.width, image.height), (7, 3));
}

#[test]
fn view_box_ratio_is_preserved_when_natural_dimensions_are_absent() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="4" height="2"/></svg>"#,
    )
    .expect("valid SVG");

    assert_eq!(svg.intrinsic_size().width, None);
    assert_eq!(svg.intrinsic_size().height, None);
    assert_eq!(svg.intrinsic_size().aspect_ratio, Some(2.0));
}

#[test]
fn inherited_current_color_is_applied_before_rasterization() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="currentColor"/></svg>"#,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                inherited_color: [0, 128, 0, 255],
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[0, 128, 0, 255]);
}

#[test]
fn host_opacity_and_visibility_apply_to_the_completed_svg() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let half_opacity = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");
    let hidden = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                visible: false,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("hidden SVG still has a transparent raster");

    assert_eq!(&half_opacity.rgba, &[255, 0, 0, 128]);
    assert_eq!(&hidden.rgba, &[0, 0, 0, 0]);
}

#[test]
fn root_opacity_can_be_neutralized_for_outer_group_compositing() {
    let sources: [&[u8]; 5] = [
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.25"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" style="opacity:0.25"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg { opacity:0.25 }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" style="opacity:0.25!important"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg { opacity:0.25!important }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    ];

    for source in sources {
        let svg = SvgDocument::parse(source).expect("valid SVG");
        let image = svg
            .rasterize(
                SvgViewport {
                    width: 1.0,
                    height: 1.0,
                },
                SvgRootStyle {
                    neutralize_root_opacity: true,
                    ..SvgRootStyle::default()
                },
                None,
            )
            .expect("rasterization succeeds");
        assert_eq!(
            &image.rgba,
            &[255, 0, 0, 255],
            "failed to neutralize source opacity in {}",
            String::from_utf8_lossy(source)
        );
    }
}

#[test]
fn root_opacity_neutralization_preserves_descendant_stylesheet_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg, rect { opacity:0.25!important;font-family:&quot;A&amp;B&quot; }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 64]);
}

#[test]
fn root_opacity_neutralization_preserves_inherited_opacity_on_children() {
    let sources: [&[u8]; 3] = [
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" opacity="inherit" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" style="opacity:inherit" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" style="opacity:inherit!important" fill="#ff0000"/></svg>"##,
    ];

    for source in sources {
        let svg = SvgDocument::parse(source).expect("valid SVG");
        let image = svg
            .rasterize(
                SvgViewport {
                    width: 1.0,
                    height: 1.0,
                },
                SvgRootStyle {
                    opacity: 0.5,
                    neutralize_root_opacity: true,
                    ..SvgRootStyle::default()
                },
                None,
            )
            .expect("rasterization succeeds");

        assert_eq!(
            &image.rgba,
            &[255, 0, 0, 128],
            "failed to preserve inherited child opacity in {}",
            String::from_utf8_lossy(source)
        );
    }
}

#[test]
fn root_opacity_neutralization_keeps_nested_inherited_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><g opacity="0.25"><rect width="1" height="1" style="opacity:inherit" fill="#ff0000"/></g></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 16]);
}

#[test]
fn root_opacity_neutralization_preserves_quoted_attribute_selectors() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>[data-name="a'b"] { fill:red } rect /* note */ { opacity:.25 }</style><rect data-name="a'b" width="1" height="1"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 64]);
}

#[test]
fn root_opacity_neutralization_preserves_stylesheet_inherited_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><style>rect { opacity:inherit }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 128]);
}

#[test]
fn root_opacity_neutralization_does_not_promote_inherited_opacity_importance() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><style>rect { opacity:inherit } .opaque { opacity:1 }</style><rect class="opaque" width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 255]);
}

#[test]
fn root_opacity_neutralization_preserves_use_instance_inheritance() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1" opacity="0.5"><rect id="source" width="1" height="1" opacity="inherit" fill="#ff0000"/><use href="#source" x="1" opacity="0.25"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 2.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba[..4], &[255, 0, 0, 128]);
    assert_eq!(&image.rgba[4..8], &[255, 0, 0, 16]);
}

#[test]
fn root_opacity_neutralization_preserves_important_inherited_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1" opacity="0.5"><style>rect { opacity:inherit!important } .opaque { opacity:1 }</style><rect id="source" class="opaque" width="1" height="1" fill="#ff0000"/><use href="#source" x="1" opacity="0.25"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 2.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba[..4], &[255, 0, 0, 128]);
    assert_eq!(&image.rgba[4..8], &[255, 0, 0, 16]);
}

#[test]
fn host_painted_root_background_is_omitted_from_svg_raster() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" style="opacity:.5;background-color:rgba(0,0,255,.5)"><rect width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 2.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                host_controls_root_background: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.rgba[4..8], &[0, 0, 0, 0]);
}

#[test]
fn root_opacity_neutralization_rewrites_cdata_stylesheet_content() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>
<![CDATA[svg { opacity:.25!important }]]>
</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 255]);
}

#[test]
fn svg11_public_doctype_is_inert_and_preserves_rasterization() {
    let body = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="green"/></svg>"#;
    let declaration = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#;
    let plain = SvgDocument::parse(body.as_bytes()).unwrap();
    let declared = SvgDocument::parse(format!("{declaration}{body}").as_bytes()).unwrap();
    assert_eq!(declared.intrinsic_size(), plain.intrinsic_size());
    for (viewport, root_style) in [
        (
            SvgViewport {
                width: 2.0,
                height: 1.0,
            },
            SvgRootStyle::default(),
        ),
        (
            SvgViewport {
                width: 4.0,
                height: 3.0,
            },
            SvgRootStyle::default(),
        ),
        (
            SvgViewport {
                width: 4.0,
                height: 3.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                host_controls_root_background: true,
                ..SvgRootStyle::default()
            },
        ),
    ] {
        assert_eq!(
            declared.rasterize(viewport, root_style, None).unwrap().rgba,
            plain.rasterize(viewport, root_style, None).unwrap().rgba
        );
    }
}

#[test]
fn public_doctype_does_not_enable_internal_entities_or_other_dtds() {
    let declarations = [
        r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [<!ENTITY x "y">]>"#,
        r#"<!DOCTYPE svg SYSTEM "file:///etc/passwd">"#,
        r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "https://example.test/other.dtd">"#,
    ];
    for declaration in declarations {
        let source = format!(
            "{declaration}<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>"
        );
        assert!(matches!(
            SvgDocument::parse(source.as_bytes()),
            Err(SvgError::UnsupportedDoctype)
        ));
    }
    let declaration = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">"#;
    let source = format!("{declaration}{declaration}<svg width=\"1\" height=\"1\"/>");
    assert!(SvgDocument::parse(source.as_bytes()).is_err());
    let source = format!("<svg width=\"1\" height=\"1\">{declaration}</svg>");
    assert!(SvgDocument::parse(source.as_bytes()).is_err());
    let source = format!("{declaration}<svg width=\"1\" height=\"1\"><text>&x;</text></svg>");
    assert!(SvgDocument::parse(source.as_bytes()).is_err());
    for href in [
        "https://example.test/image.png",
        "file:///etc/passwd",
        "data:image/png;base64,AA==",
    ] {
        let source =
            format!("{declaration}<svg width=\"1\" height=\"1\"><image href=\"{href}\"/></svg>");
        assert!(matches!(
            SvgDocument::parse(source.as_bytes()),
            Err(SvgError::ExternalReference)
        ));
    }
    for prefix in [
        format!("<!--{declaration}-->"),
        format!("<?probe {declaration}?>"),
    ] {
        let source = format!("{prefix}{declaration}<svg width=\"1\" height=\"1\"/>");
        assert!(SvgDocument::parse(source.as_bytes()).is_ok());
        let source =
            format!("{prefix}<!DOCTYPE svg [<!ENTITY x \"y\">]><svg width=\"1\" height=\"1\"/>");
        assert!(matches!(
            SvgDocument::parse(source.as_bytes()),
            Err(SvgError::UnsupportedDoctype)
        ));
    }
}

#[test]
fn rejects_doctypes_and_non_fragment_image_references() {
    let with_doctype = br#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    assert!(matches!(
        SvgDocument::parse(with_doctype),
        Err(SvgError::UnsupportedDoctype)
    ));

    for href in [
        "https://example.test/image.png",
        "file:///etc/passwd",
        "data:image/png;base64,AA==",
    ] {
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><image href=\"{href}\"/></svg>"
        );
        assert!(matches!(
            SvgDocument::parse(source.as_bytes()),
            Err(SvgError::ExternalReference)
        ));
    }
}

#[test]
fn rejects_filter_effects_before_rasterization() {
    let sources: &[&[u8]] = &[
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" filter="url(#f)"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" style="filter: drop-shadow(0 0 1px red)"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>rect { filter: url(#f) }</style><rect width="1" height="1"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><x:style xmlns:x="urn:foreign">rect { filter: url(#f) }</x:style><rect width="1" height="1"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" xlink:filter="url(#f)"/></svg>"##,
    ];

    for source in sources {
        assert!(
            SvgDocument::parse(source).is_err(),
            "filter-bearing SVG must be rejected before resvg can execute effects"
        );
    }

    let filter_none = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" style="filter:none"/></svg>"#,
    );
    assert!(filter_none.is_ok(), "filter:none has no pixel effect");

    let non_css_style = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style type="application/json">rect { filter:url(#f) }</style><rect width="1" height="1"/></svg>"#,
    );
    assert!(non_css_style.is_ok(), "non-CSS style data is not active");
}

#[test]
fn allows_unused_filter_definitions() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="unused"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" fill="blue"/></svg>"##,
    );

    assert!(
        svg.is_ok(),
        "an unreferenced filter definition has no effect"
    );
}

#[test]
fn parse_tree_rejects_active_filter_effects_for_every_parse_path() {
    let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" filter="url(#f)"/></svg>"##;

    assert!(matches!(
        super::parse_tree(source),
        Err(SvgError::UnsupportedFilterEffects)
    ));
}

#[test]
fn rejects_output_above_the_caller_byte_limit() {
    let svg = SvgDocument::parse(HALF_RED_RECT).expect("valid SVG");
    let error = svg
        .rasterize(
            SvgViewport {
                width: 4.0,
                height: 2.0,
            },
            SvgRootStyle::default(),
            Some(31),
        )
        .expect_err("32 output bytes exceed the 31-byte caller limit");

    assert!(matches!(
        error,
        SvgError::OutputLimitExceeded {
            bytes: 32,
            limit: 31,
        }
    ));
}

#[test]
fn svg_error_display_pins_each_variant() {
    assert!(format!("{}", SvgError::InvalidDocument("bad".to_string())).contains("bad"));
    assert!(!format!("{}", SvgError::UnsupportedDoctype).is_empty());
    assert!(!format!("{}", SvgError::ExternalReference).is_empty());
    assert!(!format!("{}", SvgError::UnsupportedFilterEffects).is_empty());
    assert!(!format!("{}", SvgError::InvalidViewport).is_empty());
    assert!(!format!("{}", SvgError::InvalidOpacity).is_empty());
    let msg = format!(
        "{}",
        SvgError::OutputLimitExceeded {
            bytes: 10,
            limit: 5
        }
    );
    assert!(msg.contains("10") && msg.contains('5'));
    assert!(!format!("{}", SvgError::AllocationFailed).is_empty());
}

#[test]
fn svg_error_is_std_error() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<SvgError>();
    let err: &dyn std::error::Error = &SvgError::InvalidViewport;
    assert!(err.source().is_none());
}

#[test]
fn fractional_viewport_keeps_percentage_geometry_on_the_css_pixel_grid() {
    let svg=SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="50%" width="1" height="10" fill="red"/></svg>"#).unwrap();
    let image = svg
        .rasterize_at_css_pixel_scale(
            SvgViewport {
                width: 5.2,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!((image.width, image.height), (6, 10));
    // x=2.6..3.6 crosses pixels 2 and 3. Allocating ceil(5.2) pixels must
    // not change the percentage basis to 6 or stretch the drawing to it.
    let alpha = |x| image.rgba[(5 * image.width as usize + x) * 4 + 3];
    assert!(alpha(2) > 0, "percentage geometry must start at x=2.6");
    assert!(alpha(3) > 0);
    assert_eq!(alpha(4), 0);
}

#[test]
fn fractional_image_rasterization_keeps_the_full_buffer_mapping() {
    let svg=SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="50%" width="1" height="100%" fill="green"/></svg>"#).unwrap();
    let fractional = svg
        .rasterize(
            SvgViewport {
                width: 5.2,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    let integer = svg
        .rasterize(
            SvgViewport {
                width: 6.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(fractional.rgba, integer.rgba);
    assert_eq!((fractional.width, fractional.height), (6, 10));
    let css = svg
        .rasterize_at_css_pixel_scale(
            SvgViewport {
                width: 5.2,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_ne!(css.rgba, fractional.rgba);
}

#[test]
fn styled_source_validates_inputs_without_allocating_a_raster() {
    let svg = SvgDocument::parse(HALF_RED_RECT).unwrap();
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            svg.styled_source(SvgViewport { width, height: 1.0 }, SvgRootStyle::default()),
            Err(SvgError::InvalidViewport)
        ));
        assert!(matches!(
            svg.styled_source(
                SvgViewport {
                    width: 1.0,
                    height: width
                },
                SvgRootStyle::default()
            ),
            Err(SvgError::InvalidViewport)
        ));
    }
    let viewport = SvgViewport {
        width: 100_000.0,
        height: 50_000.0,
    };
    for opacity in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            svg.styled_source(
                viewport,
                SvgRootStyle {
                    opacity,
                    ..SvgRootStyle::default()
                }
            ),
            Err(SvgError::InvalidOpacity)
        ));
    }
    assert!(svg.styled_source(viewport, SvgRootStyle::default()).is_ok());
    assert!(matches!(
        svg.rasterize(viewport, SvgRootStyle::default(), None),
        Err(SvgError::OutputLimitExceeded { .. })
    ));
}

#[test]
fn styled_source_resolves_absolute_viewport_even_with_matching_view_box_ratio() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect width="100%" height="100%"/></svg>"#,
    )
    .unwrap();
    let source = svg
        .styled_source(
            SvgViewport {
                width: 200.0,
                height: 100.0,
            },
            SvgRootStyle::default(),
        )
        .unwrap();
    let reparsed = SvgDocument::parse(source.as_bytes()).unwrap();
    assert_eq!(reparsed.tree.size().width(), 200.0);
    assert_eq!(reparsed.tree.size().height(), 100.0);
}

#[test]
fn styled_source_rejects_external_resources_before_exporting_xml() {
    for content in [
        "<use href='file:///outside.svg#shape'/>",
        "<use xmlns:xlink='http://www.w3.org/1999/xlink' xlink:href='https://example.com/a.svg#shape'/>",
        "<style>@import 'https://example.com/style.css'; rect{fill:red}</style>",
        "<style>rect{fill:url(https://example.com/paint.svg#p)}</style>",
        "<style>rect{fill:u\\72l('file:///paint.svg#p')}</style>",
        "<rect fill='url(https://example.com/paint.svg#p)'/>",
        "<rect style='fill:url(&quot;file:///paint.svg#p&quot;)'/>",
        "<g xml:base='https://example.com/'><use href='#shape'/></g>",
        "<foreignObject><img xmlns='http://www.w3.org/1999/xhtml' src='https://example.com/image.png'/></foreignObject>",
        "<?xml-stylesheet type='text/css' href='https://example.com/style.css'?>",
    ] {
        let source = format!("<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'>{content}</svg>");
        let svg = SvgDocument::parse(source.as_bytes()).unwrap();
        assert!(
            matches!(
                svg.styled_source(
                    SvgViewport {
                        width: 10.0,
                        height: 10.0
                    },
                    SvgRootStyle::default()
                ),
                Err(SvgError::ExternalReference)
            ),
            "external resource retained: {content}"
        );
    }
}

#[test]
fn exported_css_reference_validation_is_bounded_and_preserves_literal_strings() {
    use super::{MAX_FILTER_CSS_NESTING, reject_external_css_references};
    reject_external_css_references(
        "rect{fill:url(#paint);font-family:'url(https://example.com/)'}",
    )
    .unwrap();
    assert!(matches!(
        reject_external_css_references("rect{fill:url(\"#paint\" trailing)}"),
        Err(SvgError::InvalidDocument(_))
    ));
    let nested = format!(
        "{}url(#paint){}",
        "fn(".repeat(MAX_FILTER_CSS_NESTING),
        ")".repeat(MAX_FILTER_CSS_NESTING)
    );
    assert!(matches!(
        reject_external_css_references(&nested),
        Err(SvgError::InvalidDocument(_))
    ));
}

#[test]
fn styled_source_css_mime_variants_keep_original_attribute_selector_matches() {
    for mime in ["TEXT/CSS", " text/css ", "text/css; charset=utf-8"] {
        let original = format!(
            "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><style type='{mime}'>style[type=\"{mime}\"] + rect{{fill:red}}</style><rect width='10' height='10' fill='blue'/></svg>"
        );
        let svg = SvgDocument::parse(original.as_bytes()).unwrap();
        let source = svg
            .styled_source(
                SvgViewport {
                    width: 10.0,
                    height: 10.0,
                },
                SvgRootStyle::default(),
            )
            .unwrap();
        let prepared = SvgDocument::parse(source.as_bytes()).unwrap();
        let image = prepared
            .rasterize(
                SvgViewport {
                    width: 10.0,
                    height: 10.0,
                },
                SvgRootStyle::default(),
                None,
            )
            .unwrap();
        assert_eq!(&image.rgba[..4], &[255, 0, 0, 255], "CSS MIME: {mime}");
    }
}

#[test]
fn styled_source_keeps_local_resources_and_navigation_links() {
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><defs><linearGradient id='paint'><stop stop-color='red'/></linearGradient><rect id='shape' width='10' height='10'/></defs><style>use{{fill:url('#paint')}}</style><use href='#shape'/><a href='https://example.com/'><rect width='1' height='1'/></a></svg>"
    );
    let svg = SvgDocument::parse(source.as_bytes()).unwrap();
    let prepared = svg
        .styled_source(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
        )
        .unwrap();
    assert!(prepared.contains("#paint"));
    assert!(prepared.contains("#shape"));
    assert!(prepared.contains("https://example.com/"));
}

#[test]
fn styled_source_freezes_attribute_selectors_before_viewport_rewrites() {
    let svg = SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><style>svg[width="2"] rect { fill:red }</style><rect width="2" height="1"/></svg>"#).unwrap();
    let viewport = SvgViewport {
        width: 4.0,
        height: 2.0,
    };
    let source = svg
        .styled_source(viewport, SvgRootStyle::default())
        .unwrap();
    let prepared = SvgDocument::parse(source.as_bytes()).unwrap();
    assert_eq!(prepared.intrinsic_size().width, Some(4.0));
    let image = prepared
        .rasterize(viewport, SvgRootStyle::default(), None)
        .unwrap();
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn styled_source_keeps_text_for_a_consumers_font_database() {
    let svg = SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="30"><text x="0" y="20" font-family="Consumer Bundled Font">PDF text</text></svg>"#).unwrap();
    let source = svg
        .styled_source(
            SvgViewport {
                width: 100.0,
                height: 30.0,
            },
            SvgRootStyle::default(),
        )
        .unwrap();
    assert!(source.contains("<text"));
    assert!(source.contains("Consumer Bundled Font"));
    assert!(source.contains("PDF text"));
}

#[test]
fn styled_source_root_family_is_a_css_list_without_extra_declarations() {
    let svg = SvgDocument::parse(HALF_RED_RECT).unwrap();
    for family in [
        "serif;fill:url(https://example.com/paint.svg)",
        "serif;opacity:0",
        "serif,",
        "serif,,sans-serif",
    ] {
        assert!(
            matches!(
                svg.styled_source_with_root_color_and_font(
                    SvgViewport {
                        width: 2.0,
                        height: 1.0
                    },
                    SvgRootStyle::default(),
                    [0, 0, 0, 255],
                    12.0,
                    family
                ),
                Err(SvgError::InvalidDocument(_))
            ),
            "invalid family list: {family}"
        );
    }
}

#[test]
fn resolved_root_style_keeps_font_faces_and_original_visibility_selectors() {
    use super::SvgRootFont;
    let svg = SvgDocument::parse(br#"<svg width="50" height="30" visibility="visible" font-weight="400" style="font:normal 12px Ahem"><style>svg[visibility=visible][font-weight="400"] text{fill:red}</style><text y="20">Font</text></svg>"#).unwrap();
    let source = svg
        .styled_source_with_resolved_root_style(
            SvgViewport {
                width: 50.0,
                height: 30.0,
            },
            SvgRootStyle {
                visible: false,
                ..SvgRootStyle::default()
            },
            [0, 0, 0, 255],
            SvgRootFont {
                size: 12.0,
                family: "Ahem",
                weight: 700.0,
                style: "italic",
            },
        )
        .unwrap();
    let xml = roxmltree::Document::parse(&source).unwrap();
    let root = xml.root_element();
    assert!(root.attribute("font-weight").is_none());
    assert!(root.attribute("visibility").is_none());
    assert!(
        root.attribute("style")
            .unwrap()
            .contains("visibility:hidden")
    );
    let style = root.attribute("style").unwrap();
    assert!(style.contains("font-weight:700"));
    assert!(style.contains("font-style:italic"));
    assert!(source.contains("fill:red"));
}

#[test]
fn resolved_root_style_rejects_invalid_font_faces() {
    use super::SvgRootFont;
    let svg = SvgDocument::parse(HALF_RED_RECT).unwrap();
    for (weight, style) in [
        (0.0, "normal"),
        (1001.0, "normal"),
        (f32::NAN, "normal"),
        (f32::INFINITY, "normal"),
        (400.0, "unknown"),
        (400.0, "italic;fill:red"),
    ] {
        assert!(matches!(
            svg.styled_source_with_resolved_root_style(
                SvgViewport {
                    width: 2.0,
                    height: 1.0
                },
                SvgRootStyle::default(),
                [0, 0, 0, 255],
                SvgRootFont {
                    size: 12.0,
                    family: "serif",
                    weight,
                    style
                }
            ),
            Err(SvgError::InvalidDocument(_))
        ));
    }
}

#[test]
fn styled_source_root_fonts_validate_host_sizes_and_family_lists() {
    let svg =
        SvgDocument::parse(b"<svg width='10' height='10'><rect width='10' height='10'/></svg>")
            .unwrap();
    let viewport = SvgViewport {
        width: 10.0,
        height: 10.0,
    };
    for size in [-1.0, f32::INFINITY, f32::NAN] {
        assert!(
            svg.styled_source_with_root_color_and_font(
                viewport,
                SvgRootStyle::default(),
                [0, 0, 0, 255],
                size,
                "serif"
            )
            .is_err()
        );
    }
    assert!(
        svg.styled_source_with_root_color_and_font(
            viewport,
            SvgRootStyle::default(),
            [0, 0, 0, 255],
            12.0,
            " "
        )
        .is_err()
    );
    let family = "X".repeat(crate::MAX_SELECTOR_FREEZE_BYTES);
    assert!(
        svg.styled_source_with_root_color_and_font(
            viewport,
            SvgRootStyle::default(),
            [0, 0, 0, 255],
            12.0,
            &family
        )
        .is_err()
    );
    assert!(
        svg.styled_source_with_root_color_and_font(
            viewport,
            SvgRootStyle::default(),
            [0, 0, 0, 255],
            0.0,
            "serif"
        )
        .is_ok()
    );
}

#[test]
fn styled_source_root_fonts_preserve_font_shorthand_components_and_selectors() {
    let source = br#"<svg width="100" height="60" font-size="2em" font-family="Old" style="font:italic bold 2em serif !important"><style>svg[font-size="2em"],text {font:italic bold 2em serif !important;fill:red}</style><text y="40" style="font:invalid">text</text></svg>"#;
    let svg = SvgDocument::parse(source).unwrap();
    let source = svg
        .styled_source_with_root_color_and_font(
            SvgViewport {
                width: 100.0,
                height: 60.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
            24.0,
            "\"A & B\",serif",
        )
        .unwrap();
    let xml = roxmltree::Document::parse(&source).unwrap();
    let root = xml.root_element();
    assert!(root.attribute("font-size").is_none());
    assert!(root.attribute("font-family").is_none());
    let style = root.attribute("style").unwrap();
    assert!(style.contains("font-size:24px"));
    assert!(style.contains("font-family:\"A & B\",serif"));
    assert!(style.contains("font-weight:bold !important"));
    assert!(style.contains("font-style:italic !important"));
    assert!(source.contains("fill:red"));
    assert!(source.contains("font:invalid"));
    SvgDocument::parse(source.as_bytes()).unwrap();
}

#[test]
fn source_export_retains_skipped_at_rules_alongside_frozen_rules() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>@media all { rect { fill:red } } rect { opacity:.5 }</style><style>@font-face { font-family:local; font-weight:700 } circle { fill:blue }</style><rect width="10" height="10"/></svg>"#;
    let svg = SvgDocument::parse(source).unwrap();
    let exported = svg
        .styled_source_with_root_color(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
        )
        .unwrap();
    assert!(exported.contains("@media all { rect { fill:red } }"));
    assert!(exported.contains("@font-face { font-family:local; font-weight:700 }"));
    let reparsed = SvgDocument::parse(exported.as_bytes()).unwrap();
    let image = reparsed
        .rasterize(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert!(
        image
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[3] > 100 && pixel[3] < 150)
    );
}

#[test]
fn color_only_source_export_preserves_author_font_faces() {
    let svg = SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" font-weight="700" font-style="italic"><rect width="10" height="10" fill="currentColor"/></svg>"#).unwrap();
    let exported = svg
        .styled_source_with_root_color(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [255, 0, 0, 255],
        )
        .unwrap();
    assert!(exported.contains("font-weight=\"700\""));
    assert!(exported.contains("font-style=\"italic\""));
    let image = SvgDocument::parse(exported.as_bytes())
        .unwrap()
        .rasterize(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 255]);
}

#[test]
fn at_rule_retention_respects_tokens_and_original_style_elements() {
    let first = "@charset \"UTF-8\"; @media all { rect { content:\"};@fake{\" } @supports (display:block) { rect { opacity:.5 } } }";
    let second = "/* @discard { opacity:1 } */ rect { fill:red } @unknown \"a;b{c\"; @unfinished";
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><style>{first} rect {{ opacity:.5 }}</style><style>{second}</style><rect width='10' height='10'/></svg>"
    );
    let svg = SvgDocument::parse(source.as_bytes()).unwrap();
    let exported = svg
        .styled_source_with_root_color(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
        )
        .unwrap();
    let xml = roxmltree::Document::parse(&exported).unwrap();
    let styles: Vec<_> = xml
        .descendants()
        .filter(|node| node.has_tag_name("style"))
        .map(|node| node.text().unwrap_or_default())
        .collect();
    assert_eq!(styles.len(), 2);
    assert!(styles[0].contains("@charset \"UTF-8\";"));
    assert!(styles[0].contains(&first[first.find("@media").unwrap()..]));
    // SimpleCSS ends `@unknown` at the `;` inside its string and never ends
    // `@unfinished`, so retaining either would change what it parses next.
    assert!(!styles[1].contains("@unknown"), "{}", styles[1]);
    assert!(!styles[1].contains("@unfinished"), "{}", styles[1]);
    assert!(!styles[1].contains("@discard"));
}

#[test]
fn at_rule_retention_charges_the_shared_rewrite_budget() {
    let mut budget = SelectorFreezeBudget::new();
    budget.bytes = 1;
    assert!(matches!(
        super::retain_unfrozen_svg_css("@media all { rect { fill:red } }", &mut budget),
        Err(SvgError::InvalidDocument(_))
    ));
    let mut budget = SelectorFreezeBudget::new();
    for checks in [0, 1] {
        budget.checks = checks;
        assert!(matches!(
            super::retain_unfrozen_svg_css("@custom;", &mut budget),
            Err(SvgError::InvalidDocument(_))
        ));
    }
}

#[test]
fn source_export_without_frozen_rules_keeps_unmatched_selectors_inert() {
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><style>rect:is(.hot), rect {{ color:red }}</style><rect class='hot' width='10' height='10' fill='currentColor'/></svg>"
    );
    let exported = SvgDocument::parse(source.as_bytes())
        .unwrap()
        .styled_source_with_root_color(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
        )
        .unwrap();
    // The scoped copy keeps the list together, so SimpleCSS still drops its
    // supported `rect` entry after the unsupported one.
    assert!(exported.contains("rect:is(.hot), rect {"), "{exported}");
    assert!(
        exported.contains(
            "rect:is(.hot)[data-raikiri-root-opacity-scope], rect[data-raikiri-root-opacity-scope] {"
        ),
        "{exported}"
    );
    let image = SvgDocument::parse(exported.as_bytes())
        .unwrap()
        .rasterize(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(&image.rgba[..4], &[0, 0, 0, 255]);
}

#[test]
fn style_rule_retention_charges_the_shared_rewrite_budget() {
    let rule = "a:is(.b), c { fill:red }";
    let mut unlimited = SelectorFreezeBudget::new();
    assert_eq!(
        super::retain_unfrozen_svg_css(rule, &mut unlimited).unwrap(),
        format!("@media all {{ {rule} }}\n")
    );
    let initial = SelectorFreezeBudget::new();
    for checks in 0..initial.checks - unlimited.checks {
        let mut budget = SelectorFreezeBudget::new();
        budget.checks = checks;
        assert!(matches!(
            super::retain_unfrozen_svg_css(rule, &mut budget),
            Err(SvgError::InvalidDocument(_))
        ));
    }
    let used_bytes = initial.bytes - unlimited.bytes;
    for bytes in [0, used_bytes - 1] {
        let mut budget = SelectorFreezeBudget::new();
        budget.bytes = bytes;
        assert!(matches!(
            super::retain_unfrozen_svg_css(rule, &mut budget),
            Err(SvgError::InvalidDocument(_))
        ));
    }
}

#[test]
fn style_rule_retention_reads_the_whole_prelude_before_a_block() {
    // A block token directly before `{` must not cut the prelude short, or a
    // rule SimpleCSS applies would also be retained.
    let mut budget = SelectorFreezeBudget::new();
    assert_eq!(
        super::retain_unfrozen_svg_css(
            "rect[width]{fill:red} rect:lang(en){fill:blue}",
            &mut budget
        )
        .unwrap(),
        ""
    );
}

#[test]
fn css_retention_skips_text_simplecss_would_not_skip_exactly() {
    let mut budget = SelectorFreezeBudget::new();
    for stylesheet in [
        "rect:is(.a) { /* } */ fill:red }",
        "rect:is(.a) { font-family:\"{\" }",
        "rect:is(.a) { fill:blue",
        "@media all { rect { content:\"}\" } }",
        "@unknown \"a;b{c\";",
        "@unfinished",
        "@--custom { }",
    ] {
        assert_eq!(
            super::retain_unfrozen_svg_css(stylesheet, &mut budget).unwrap(),
            "",
            "{stylesheet}"
        );
    }
    assert_eq!(
        super::retain_unfrozen_svg_css(
            "@-custom { a { } } rect:is(.a) { content:\"{}\" }",
            &mut budget
        )
        .unwrap(),
        "@-custom { a { } }\n@media all { rect:is(.a) { content:\"{}\" } }\n"
    );
}

#[test]
fn source_export_keeps_simplecss_rules_beside_retained_css_with_braces() {
    // Unsafe retained text would expose `* { fill:red }` or swallow the scoped
    // `color:blue` rule appended after it.
    for unsafe_rule in ["g:not(.x) /* } */ { fill:red }", "g:not(.x) { fill:red"] {
        let source = format!(
            "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><style>rect {{ color:blue }} {unsafe_rule}</style><rect width='10' height='10' fill='currentColor'/></svg>"
        );
        let exported = SvgDocument::parse(source.as_bytes())
            .unwrap()
            .styled_source_with_root_color(
                SvgViewport {
                    width: 10.0,
                    height: 10.0,
                },
                SvgRootStyle::default(),
                [0, 0, 0, 255],
            )
            .unwrap();
        let image = SvgDocument::parse(exported.as_bytes())
            .unwrap()
            .rasterize(
                SvgViewport {
                    width: 10.0,
                    height: 10.0,
                },
                SvgRootStyle::default(),
                None,
            )
            .unwrap();
        assert_eq!(&image.rgba[..4], &[0, 0, 255, 255], "{exported}");
    }
}

#[test]
fn style_rule_retention_skips_rules_that_cannot_be_wrapped() {
    let mut budget = SelectorFreezeBudget::new();
    for stylesheet in [
        "{ fill:red }",
        "} a:is(.b) { fill:red }",
        "a:is(.b) } c { fill:red }",
        "; a:is(.b) { fill:red }",
        "a:is(.b); c { fill:red }",
        "a:is(.b)",
        "<!-- a, c { fill:red } -->",
    ] {
        assert_eq!(
            super::retain_unfrozen_svg_css(stylesheet, &mut budget).unwrap(),
            "",
            "{stylesheet}"
        );
    }
}

#[test]
fn source_export_retains_style_rules_with_unmatched_selectors() {
    let stylesheet = "rect { opacity:.5 } rect:is(.hot) { fill:red } rect:is(.hot), rect { color:red } } rect:is(.hot) { fill:blue }";
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='10' height='10'><style>{stylesheet}</style><rect class='hot' width='10' height='10' fill='currentColor'/></svg>"
    );
    let exported = SvgDocument::parse(source.as_bytes())
        .unwrap()
        .styled_source_with_root_color(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            [0, 0, 0, 255],
        )
        .unwrap();
    let xml = roxmltree::Document::parse(&exported).unwrap();
    let style = xml
        .descendants()
        .find(|node| node.has_tag_name("style"))
        .and_then(|node| node.text())
        .unwrap();
    assert!(
        style.contains("@media all { rect:is(.hot) { fill:red } }\n@media all { rect:is(.hot), rect { color:red } }"),
        "{style}"
    );
    assert!(!style.contains("rect { opacity:.5 }"), "{style}");
    assert!(!style.contains("fill:blue"), "{style}");
    // Retained rules stay invisible to the rasterizer, including the rule
    // whose selector list contains a supported entry.
    let image = SvgDocument::parse(exported.as_bytes())
        .unwrap()
        .rasterize(
            SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(&image.rgba[..3], &[0, 0, 0]);
    assert!((100..150).contains(&image.rgba[3]));
}

fn source_with_descendant_styles(
    source: &[u8],
    styles: &[super::SvgElementStyle<'_>],
) -> Result<String, SvgError> {
    SvgDocument::parse(source)?.styled_source_with_resolved_styles(
        SvgViewport {
            width: 20.0,
            height: 10.0,
        },
        SvgRootStyle::default(),
        [0, 0, 0, 255],
        super::SvgRootFont {
            size: 12.0,
            family: "sans-serif",
            weight: 400.0,
            style: "normal",
        },
        styles,
    )
}

#[test]
fn descendant_overrides_preserve_original_selector_matches_and_other_properties() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><style>rect[opacity='0.75']{opacity:0.75!important;fill:red}rect{color:blue!important}</style><rect opacity="0.75" width="10" height="10"/><rect x="10" width="10" height="10" fill="currentColor"/></svg>"#;
    let prepared = source_with_descendant_styles(
        source,
        &[
            super::SvgElementStyle {
                element_index: 2,
                declarations: "opacity:.25!important",
            },
            super::SvgElementStyle {
                element_index: 3,
                declarations: "color:green!important",
            },
        ],
    )
    .unwrap();
    let rgba = SvgDocument::parse(prepared.as_bytes())
        .unwrap()
        .rasterize(
            SvgViewport {
                width: 20.0,
                height: 10.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap()
        .rgba;
    assert_eq!(
        &rgba[(5 * 20 + 5) * 4..(5 * 20 + 5) * 4 + 4],
        &[255, 0, 0, 64]
    );
    assert_eq!(
        &rgba[(5 * 20 + 15) * 4..(5 * 20 + 15) * 4 + 4],
        &[0, 128, 0, 255]
    );
}

#[test]
fn descendant_overrides_replace_inline_and_presentation_properties_only() {
    let prepared = source_with_descendant_styles(
        br#"<svg xmlns="http://www.w3.org/2000/svg"><rect opacity=".9" style="opacity:.8!important;fill:red" width="20" height="10"/></svg>"#,
        &[super::SvgElementStyle { element_index: 1, declarations: "opacity:.25" }],
    ).unwrap();
    let xml = roxmltree::Document::parse(&prepared).unwrap();
    let rect = xml
        .descendants()
        .find(|node| node.has_tag_name("rect"))
        .unwrap();
    assert_eq!(rect.attribute("opacity"), None);
    let style = rect.attribute("style").unwrap();
    assert!(style.contains("fill:red"));
    assert!(style.contains("opacity:.25"));
    assert!(!style.contains("opacity:.8"));
}

#[test]
fn descendant_overrides_reject_invalid_indices_properties_and_resources() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#;
    for styles in [
        vec![super::SvgElementStyle {
            element_index: 0,
            declarations: "opacity:0",
        }],
        vec![super::SvgElementStyle {
            element_index: 2,
            declarations: "opacity:0",
        }],
        vec![
            super::SvgElementStyle {
                element_index: 1,
                declarations: "opacity:0"
            };
            2
        ],
        vec![super::SvgElementStyle {
            element_index: 1,
            declarations: "fill:red",
        }],
        vec![super::SvgElementStyle {
            element_index: 1,
            declarations: "opacity",
        }],
    ] {
        assert!(matches!(
            source_with_descendant_styles(source, &styles),
            Err(SvgError::InvalidDocument(_))
        ));
    }
    assert!(matches!(
        source_with_descendant_styles(
            source,
            &[super::SvgElementStyle {
                element_index: 1,
                declarations: "font-family:url(https://example.com/font)"
            },]
        ),
        Err(SvgError::ExternalReference)
    ));
}

#[test]
fn descendant_overrides_share_the_selector_rewrite_budget() {
    let source = "<svg xmlns='http://www.w3.org/2000/svg'><rect/></svg>";
    let styles = [super::SvgElementStyle {
        element_index: 1,
        declarations: "opacity:0",
    }];
    let mut budget = SelectorFreezeBudget::new();
    budget.bytes = 1;
    assert!(super::with_element_style_overrides(source, &styles, &mut budget).is_err());
    let mut budget = SelectorFreezeBudget::new();
    budget.checks = 1;
    assert!(super::with_element_style_overrides(source, &styles, &mut budget).is_err());
}

#[test]
fn descendant_rewrites_fail_cleanly_at_intermediate_resource_limits() {
    let source = "<svg xmlns='http://www.w3.org/2000/svg'><style>rect{color:red;opacity:.8}</style><rect style='opacity:.5;fill:currentColor'/></svg>";
    let styles = [super::SvgElementStyle {
        element_index: 2,
        declarations: "color:blue;opacity:.25",
    }];
    let expected =
        super::with_element_style_overrides(source, &styles, &mut SelectorFreezeBudget::new())
            .unwrap();
    for bytes in (0..10_000).step_by(17) {
        let mut budget = SelectorFreezeBudget::new();
        budget.bytes = bytes;
        match super::with_element_style_overrides(source, &styles, &mut budget) {
            Ok(prepared) => assert_eq!(prepared, expected),
            Err(SvgError::InvalidDocument(message)) => {
                assert!(
                    message.contains("selector freezing resource limit"),
                    "{message}"
                );
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
    for checks in 0..30 {
        let mut budget = SelectorFreezeBudget::new();
        budget.checks = checks;
        match super::with_element_style_overrides(source, &styles, &mut budget) {
            Ok(prepared) => assert_eq!(prepared, expected),
            Err(SvgError::InvalidDocument(message)) => {
                assert!(
                    message.contains("selector freezing resource limit"),
                    "{message}"
                );
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}

#[test]
fn descendant_scope_names_reject_exhausted_collision_suffixes() {
    let source = format!(
        "<svg data-raikiri-svg-color-scope-{}=''><rect/></svg>",
        usize::MAX,
    );
    let styles = [super::SvgElementStyle {
        element_index: 1,
        declarations: "color:blue",
    }];
    assert!(matches!(
        super::with_element_style_overrides(&source, &styles, &mut SelectorFreezeBudget::new()),
        Err(SvgError::InvalidDocument(message))
            if message.contains("selector freezing resource limit")
    ));
}

#[test]
fn font_shorthand_expansion_fails_cleanly_at_intermediate_byte_limits() {
    for (source, stylesheet) in [
        ("font:italic bold 12px serif!important", false),
        ("text{font:italic bold 12px serif!important}", true),
    ] {
        let expected =
            super::expand_font_shorthands(source, stylesheet, &mut SelectorFreezeBudget::new())
                .unwrap();
        assert!(expected.as_ref().unwrap().contains("font-size:12px"));
        for bytes in (0..5_000).step_by(17) {
            let mut budget = SelectorFreezeBudget::new();
            budget.bytes = bytes;
            match super::expand_font_shorthands(source, stylesheet, &mut budget) {
                Ok(prepared) => assert_eq!(prepared, expected),
                Err(SvgError::InvalidDocument(message)) => {
                    assert!(
                        message.contains("selector freezing resource limit"),
                        "{message}"
                    );
                }
                other => panic!("unexpected result: {other:?}"),
            }
        }
    }
}
