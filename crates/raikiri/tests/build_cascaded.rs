//! Integration tests for the raikiri umbrella crate.
//!
//! Verify that a consumer can parse, call build_cascaded, and inspect
//! display values using only `use raikiri::…;`.

use raikiri::{
    DisplayValue, Dom, Element, MediaContext, Node, NodeId, NodeKind, PageBox, PageContextQuery,
    ParseOptions, build_cascaded, build_cascaded_for_page, build_cascaded_with_media_context,
    parse,
};

fn parse_html(source: &str) -> raikiri::UncascadedDocument {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    parse(source.as_bytes(), &opts).expect("parse")
}

/// Return the NodeId of the first element with the matching tag in a
/// depth-first traversal from the DOM root.
///
/// Use an explicit `Vec` stack to avoid stack overflow in deeply nested
/// documents (as in raikiri-style::ruletree::walk_and_collect). Because the
/// stack is LIFO, push children in reverse to preserve preorder and sibling
/// document order.
fn find_by_tag<D: Dom>(dom: &D, tag: &str) -> Option<NodeId> {
    let mut stack: Vec<NodeId> = vec![dom.root_id()];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            if node.kind() == NodeKind::Element
                && let Some(elem) = node.as_element()
                && elem.tag_name().eq_ignore_ascii_case(tag)
            {
                return Some(id);
            }
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
    None
}

#[test]
fn p_without_author_style_is_display_block_via_ua_css() {
    let doc = parse_html("<html><body><p>Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<p> should inherit display: block from bundled UA CSS via build_cascaded",
    );
}

#[test]
fn explicit_media_context_reaches_umbrella_cascade() {
    let html = "<html><head><style>@media screen { p { display: inline } }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");

    let print_result = build_cascaded(&doc);
    assert_eq!(
        print_result.computed[p_id.0 as usize].display,
        DisplayValue::Block
    );

    let screen_result = build_cascaded_with_media_context(&doc, &MediaContext::screen());
    assert_eq!(
        screen_result.computed[p_id.0 as usize].display,
        DisplayValue::Inline
    );
}

#[test]
fn page_size_cascade_reaches_umbrella_page_box_consumer() {
    let doc = parse_html(
        "<html><head><style>@page { size: 300px 50px }</style></head>         <body><p>Hi</p></body></html>",
    );
    let mut query = PageContextQuery::default();
    query.is_first = true;
    query.is_right = true;
    let cascade = build_cascaded_for_page(&doc, &query);
    let page_box = PageBox::from_page_size(cascade.page.size());
    assert_eq!(page_box.width, 300.0);
    assert_eq!(page_box.height, 50.0);
}

#[test]
fn page_first_selector_reaches_umbrella_page_cascade() {
    let doc = parse_html(
        "<html><head><style>         @page { size: 300px } @page :first { size: 400px 60px }         </style></head><body><p>Hi</p></body></html>",
    );
    let mut query = PageContextQuery::default();
    query.is_first = true;
    query.is_right = true;
    let cascade = build_cascaded_for_page(&doc, &query);
    let page_box = PageBox::from_page_size(cascade.page.size());
    assert_eq!(page_box.width, 400.0);
    assert_eq!(page_box.height, 60.0);
}

#[test]
fn sectioning_and_grouping_elements_are_display_block_via_ua_css() {
    // Acceptance: <article><h2>...</h2><p>...</p></article> must render
    // as a block box, not become inline and intermingle with its children.
    // Check all 11 elements covered by the same UA CSS addition, including
    // article, through real parse → build_cascaded execution. The
    // `minimal_ua_css_covers_required_display_block_selectors` test in
    // `crates/raikiri-html/src/lib.rs` only scans raw `MINIMAL_UA_CSS` text;
    // it cannot catch CSS parser failures (e.g., malformed comments).
    // Inspecting computed values after cascade makes this test non-vacuous.
    // Include hgroup: it belongs to the same §sections-and-headings (15.3.6)
    // selector group as article/aside/nav/section but was missed initially.
    for tag in [
        "article",
        "section",
        "nav",
        "aside",
        "hgroup",
        "header",
        "footer",
        "main",
        "figure",
        "figcaption",
        "blockquote",
    ] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "<{tag}> should be display: block from bundled UA CSS via build_cascaded",
        );
    }
}

#[test]
fn list_elements_use_list_item_display_via_ua_css() {
    // Acceptance: each list item in <ul><li>...</li><li>...</li></ul>
    // has `display: list-item`, while ol/ul themselves have display: block.
    // Checking computed values after cascade makes this non-vacuous.
    for tag in ["ol", "ul", "li"] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        let expected = if tag == "li" {
            DisplayValue::ListItem
        } else {
            DisplayValue::Block
        };
        assert_eq!(
            display, expected,
            "<{tag}> should resolve to its list UA display value via build_cascaded",
        );
    }
}

#[test]
fn li_list_item_display_stacks_siblings_as_boxes() {
    // Reproduce the literal acceptance fixture <ul><li>A</li><li>B</li></ul>.
    // Check both li children through ul.child_ids, not a tag scan: find_by_tag
    // would inspect only the first li and miss a regression in the second.
    let doc = parse_html("<html><body><ul><li>A</li><li>B</li></ul></body></html>");
    let result = build_cascaded(&doc);

    let ul_id = find_by_tag(&doc.dom, "ul").expect("<ul> exists");
    let li_ids: Vec<NodeId> = doc.dom.child_ids(ul_id).collect();
    assert_eq!(li_ids.len(), 2, "expected two <li> children under <ul>");
    for li_id in li_ids {
        let node = doc.dom.node(li_id).expect("child node exists");
        assert_eq!(
            node.kind(),
            NodeKind::Element,
            "expected an Element child under <ul>",
        );
        let tag = node
            .as_element()
            .expect("Element node has as_element()")
            .tag_name()
            .to_owned();
        assert_eq!(
            tag, "li",
            "expected <ul> children to be <li>, found <{tag}>"
        );

        let display = result.computed[li_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::ListItem,
            "each <li> under <ul> should be display: list-item from bundled UA CSS",
        );
    }
}

#[test]
fn author_inline_style_overrides_ua_display_block() {
    // NB: cascade drops class/id selectors, so use inline style.
    let doc = parse_html("<html><body><p style=\"display:inline\">Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "author inline style (Normal Author) should override UA (Normal UA) per cascade rank",
    );
}

#[test]
fn dom_style_element_author_rule_overrides_ua() {
    // Verify that an explicit author <style> rule overrides UA CSS.
    // Cascade supports only type selectors, so use p{...}.
    let html = "<html><head><style>p { display: inline }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "DOM <style> Author rule should override UA (both via umbrella integration)",
    );
}

#[test]
fn lang_pseudo_class_inherits_from_html_lang_attribute_through_real_parse_pipeline() {
    // Acceptance test, exercised through the *real*
    // html5ever parse -> raikiri-dom -> build_cascaded pipeline (not the
    // raikiri-style-internal `TestDoc` mock other coverage for this feature
    // uses):
    // `<html lang="ja">` with a `<p>` descendant that carries no `lang`
    // attribute of its own must still match `:lang(ja)`. This also pins
    // that the `lang` attribute integration
    // (`raikiri-html`'s sink -> `raikiri-dom::Node.attributes` ->
    // `ElementRef::attr`) actually surfaces `lang` where
    // `raikiri-style::StyleElement::attr("lang")` reads it end to end.
    let html = "<html lang=\"ja\"><head>\
                <style>:lang(ja) { font-family: serif-ja }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let font_family = &result.computed[p_id.0 as usize].font_family;
    assert_eq!(
        font_family[0].to_string(),
        "serif-ja",
        ":lang(ja) must match <p> via the ancestor <html lang=\"ja\">, \
         through the real parse pipeline",
    );
}

#[test]
fn extra_stylesheets_user_rule_overrides_ua_via_umbrella() {
    // Verify that CSS supplied in opts.extra_stylesheets reaches
    // build_cascaded with User origin. Parsing pushes it into
    // Document.stylesheets as User, without retagging it as Author.
    // Normal User (rank 1) outranks normal UserAgent (rank 0).
    let extra: &[&str] = &["p { display: inline }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(display, DisplayValue::Inline);
}

#[test]
fn style_inside_template_element_does_not_affect_cascade() {
    // <template> is inert by specification. Its <style> does not enter
    // cascade, so <p> gets only UA CSS (`display: block`). This checks the
    // invariant in raikiri-html/src/sink.rs:315-318 through the umbrella API.
    let html = "<html><head><template><style>p { display: inline }</style></template></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<template> 内の <style> は inert として無視され、<p> は UA CSS の display: block を得る",
    );
}

#[test]
fn user_important_beats_normal_ua_via_umbrella() {
    // CSS Cascading L4 §6.1 "Cascade Sorting Order" describes Origin and
    // Importance (<https://www.w3.org/TR/css-cascade-4/#cascade-sort>);
    // §6.3 reverses origins for `!important`
    // (<https://www.w3.org/TR/css-cascade-4/#importance>). The complete
    // raikiri-style cascade_rank order (four tiers, eight arms) is:
    //   Normal UA(0) < Normal User(1) < Normal Hint(2) < Normal Author(3) <
    //   Important Author(4) < Important Hint(5) < Important User(6) < Important UA(7).
    // Bundled UA CSS (minimal.css) has no !important rules, so this test
    // cannot directly verify the reversal against Important UA. Instead,
    // verify that Important User beats Normal UA end to end, including the
    // umbrella StylesheetKind → Origin map (extra_stylesheets → `Origin::User`).
    // Parsing inserts extra_stylesheets with User kind, not retagged Author.
    let extra: &[&str] = &["p { display: inline !important }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;

    // Bundled minimal.css has no !important rules at this stage. User
    // !important therefore wins (Normal UA 0 < ... < Important User 6), and
    // p becomes inline. This pins Important User > Normal UA origin ranking
    // through umbrella integration.
    assert_eq!(
        display,
        DisplayValue::Inline,
        "Important User (extra_stylesheets) should beat Normal UA via umbrella cascade integration",
    );
}

#[test]
fn umbrella_re_exports_cover_computed_value_types_and_parse_options_fields() {
    // Prove AC #6 (consumers need only use raikiri::…) at name-resolution
    // level. Actually use exported value types (CssColor / Length / Atom),
    // Document (UncascadedDocument.dom), NetworkProvider (ParseOptions.network),
    // and Url (ParseOptions.base_url). Build ParseOptions and bind typed
    // ComputedValues fields without direct sub-crate dependencies.
    use raikiri::{
        ComputedLength, CssColor, Document, FontFamilyKind, FontFamilyName, Length,
        NetworkProvider, ParseOptions, PropertyValue, Url, build_cascaded, parse,
    };

    // Compile-check that NetworkProvider can be named through dyn.
    let _network: Option<&dyn NetworkProvider> = None;
    // Parse a Url for use as base_url.
    let base = Url::parse("https://example.com/").expect("url parse");
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: _network,
        base_url: Some(base),
    };

    let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
    // Annotate UncascadedDocument.dom as Document to verify its re-export.
    let _dom: &Document = &doc.dom;

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let computed = &result.computed[p_id.0 as usize];

    // Bind a ComputedValues field with an explicit exported value type.
    let _color: CssColor = computed.color;
    // `font_size` uses the computed-layer `ComputedLength`.
    let _font_size: ComputedLength = computed.font_size;
    // The specified-layer `Length` remains nameable by consumers as a
    // `PropertyValue` payload; that motivates its umbrella re-export.
    let _specified_font_size: PropertyValue = PropertyValue::FontSize(Length::Px(12.0));
    // ComputedValues retains both each family's text and whether it is a
    // generic keyword or a named family.
    let font_family: &Vec<FontFamilyName> = &computed.font_family;
    // The initial font-family is the generic `serif` family.
    assert!(
        !font_family.is_empty(),
        "font_family should have at least initial serif family"
    );
    assert_eq!(font_family[0].1, FontFamilyKind::Generic);
}

#[test]
fn umbrella_re_exports_cover_sides_and_specified_payload_types() {
    // Compile-check typed umbrella access to Sides<T> payloads of
    // `PropertyValue` (Padding/Margin/Border) and to LineHeight. As with
    // `PropertyValue::FontSize(Length::Px(12.0))`, actually construct each
    // directly constructible variant (Padding/Margin/LineHeight), rather
    // than checking only its type.
    use raikiri::{
        Border, BorderColor, BorderStyle, CssColor, Length, LengthOrAuto, LineHeight,
        PropertyValue, Sides,
    };

    // `Padding(Sides<Length>)`: both `Sides<T>` and `Length` are re-exported
    // and can be constructed directly without a struct-literal restriction.
    let _specified_padding: PropertyValue = PropertyValue::Padding(Sides::all(Length::Px(4.0)));

    // `Margin(Sides<LengthOrAuto>)`: LengthOrAuto is also constructible directly.
    let _specified_margin: PropertyValue =
        PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(8.0))));

    // `LineHeight(LineHeight)`: the enum is `#[non_exhaustive]`, but its
    // existing variants can still be constructed externally, like `Length`.
    let _specified_line_height: PropertyValue = PropertyValue::LineHeight(LineHeight::Normal);

    // `Border(Sides<Border>)`: previously, Border was a `#[non_exhaustive]`
    // struct, so external construction via `Border { .. }` failed (E0639).
    // Only the type could be checked by coercing the tuple-variant constructor
    // to a function pointer; no value could be built. With `Border::new()`
    // (= `Self::default()`), test actual construction as for FontSize,
    // Padding, Margin, and LineHeight. Also mutate its public fields away
    // from initial values, and construct the umbrella-re-exported
    // `BorderStyle` and `BorderColor` types added at the same time.
    let mut border = Border::new();
    assert_eq!(
        border.width,
        Length::Px(3.0),
        "Border::new() は CSS 初期値 (medium=3px)"
    );
    assert_eq!(border.style, BorderStyle::None);
    assert_eq!(border.color, BorderColor::CurrentColor);
    border.width = Length::Px(2.0);
    border.style = BorderStyle::Solid;
    border.color = BorderColor::Resolved(CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    });
    let _specified_border: PropertyValue = PropertyValue::Border(Sides::all(border));
}

#[test]
fn umbrella_re_exports_cover_computed_sides_container_fields() {
    // The first five re-exported Computed* types were only leaf types.
    // `ComputedValues.padding`, `.margin`, and `.border` actually use
    // `Sides<Computed*>`, which could not be named until `Sides<T>` was
    // re-exported (see field definitions in crates/raikiri-style/src/
    // computed.rs). Verify typed access to those three fields using real
    // cascade output after the later `Sides` addition.
    use raikiri::{
        ComputedBorder, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ParseOptions,
        Sides, build_cascaded, parse,
    };

    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let computed = &result.computed[p_id.0 as usize];

    let _padding: Sides<ComputedLengthPercentage> = computed.padding;
    let _margin: Sides<ComputedLengthPercentageOrAuto> = computed.margin;
    let _border: Sides<ComputedBorder> = computed.border;
}

#[test]
fn concrete_network_provider_impl_via_raikiri_only_re_exports() {
    // Compile-check that a downstream NetworkProvider implementation needs
    // no direct sub-crate dependencies. Import all three fetch() signature
    // types (parameter, return, error), Bytes, Url, and supporting Method,
    // Body, HeaderMap, and ResourceKind through raikiri. No round trip is
    // required (fetch immediately returns NetworkError::Aborted); the point
    // is complete name resolution for the trait implementation.
    use raikiri::{
        Bytes, FetchOutcome, FetchedResource, NetworkError, NetworkProvider, Request, Url,
    };

    struct DummyProvider;

    impl NetworkProvider for DummyProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            // Read every Request field to compile-check access (even if unused).
            let _url: &Url = &request.url;
            let _kind = request.kind;

            // Construct FetchedResource using Bytes and Url.
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: Bytes::from_static(b""),
                content_type: None,
                final_url: Url::parse("about:blank").unwrap(),
                encoding: None,
            }))
        }
    }

    // Convert to a dyn trait object compatible with ParseOptions.network.
    let provider: &dyn NetworkProvider = &DummyProvider;
    let _network: Option<&dyn NetworkProvider> = Some(provider);
}

#[test]
fn link_rel_stylesheet_fetched_css_reaches_computed_style_through_real_cascade() {
    // raikiri-html unit tests cover detection and fetching of
    // <link rel="stylesheet" href="..."> via NetworkProvider::fetch and
    // ResourceKind::ExternalStylesheet, then conversion to CSS text. This
    // umbrella test checks the next step: real parse → build_cascaded must
    // take the fetched data in `UncascadedDocument.stylesheet_sources`, add
    // it as an Author stylesheet to RuleTree, and produce computed styles.
    // As with img_width_height_html_attributes_reach_computed_style_through_real_parse_path,
    // reading the code alone cannot prove that the stages are wired up.
    use raikiri::{
        Bytes, DisplayValue, FetchOutcome, FetchedResource, NetworkError, NetworkProvider,
        ParseOptions, Request, Url, build_cascaded, parse,
    };

    struct StylesheetProvider;

    impl NetworkProvider for StylesheetProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: Bytes::from_static(b"div { display: none }"),
                content_type: Some("text/css".to_string()),
                final_url: request.url,
                encoding: None,
            }))
        }
    }

    let provider = StylesheetProvider;
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&provider as &dyn NetworkProvider),
        base_url: Some(Url::parse("https://example.test/").expect("valid base url")),
    };
    let html = br#"<html><head><link rel="stylesheet" href="a.css"></head><body><div>Hi</div></body></html>"#;
    let doc = parse(&html[..], &opts).expect("parse");

    // Also inspect raikiri-html's stylesheet_sources contract to verify
    // that fetched CSS arrives as an Author stylesheet.
    assert_eq!(
        doc.stylesheet_sources,
        vec![String::from("div { display: none }")],
        "fetched external stylesheet CSS text must land in stylesheet_sources"
    );

    let result = build_cascaded(&doc);
    let div_id = find_by_tag(&doc.dom, "div").expect("<div> exists");
    let display = result.computed[div_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::None,
        "div {{ display: none }} fetched via <link rel=stylesheet> must beat the UA CSS \
         display:block default through the real build_cascaded pipeline"
    );
}

#[test]
fn imported_stylesheet_rules_reach_cascade_in_source_order() {
    use std::sync::Mutex;

    use raikiri::{
        Bytes, FetchOutcome, FetchedResource, NetworkError, NetworkProvider, ParseOptions, Request,
        ResourceKind, Url, build_cascaded, parse,
    };

    struct ImportProvider {
        calls: Mutex<Vec<(String, ResourceKind)>>,
    }

    impl NetworkProvider for ImportProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let requested = request.url.to_string();
            self.calls
                .lock()
                .unwrap()
                .push((requested.clone(), request.kind));
            let (css, final_url) = match requested.as_str() {
                "https://example.test/css/main.css" => (
                    r#"@import "nested.css"; p { display: inline }"#,
                    "https://example.test/css/main.css",
                ),
                "https://example.test/css/nested.css" => {
                    ("p { display: none }", "https://example.test/css/nested.css")
                }
                _ => return Err(NetworkError::Other("not found".to_owned())),
            };
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: Bytes::from(css),
                content_type: Some("text/css".to_owned()),
                final_url: Url::parse(final_url).unwrap(),
                encoding: None,
            }))
        }
    }

    let provider = ImportProvider {
        calls: Mutex::new(Vec::new()),
    };
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&provider as &dyn NetworkProvider),
        base_url: Some(Url::parse("https://example.test/").unwrap()),
    };
    let html = br#"<html><head><link rel="stylesheet" href="css/main.css"></head>
        <body><p>Hi</p></body></html>"#;
    let doc = parse(&html[..], &opts).expect("parse");
    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    assert_eq!(
        result.computed[p_id.0 as usize].display,
        DisplayValue::Inline,
        "parent rules must follow imported rules at the import's source position"
    );
    assert_eq!(
        *provider.calls.lock().unwrap(),
        vec![
            (
                "https://example.test/css/main.css".to_owned(),
                ResourceKind::ExternalStylesheet,
            ),
            (
                "https://example.test/css/nested.css".to_owned(),
                ResourceKind::StylesheetImport,
            ),
        ]
    );
}

#[test]
fn imported_media_condition_is_evaluated_by_the_selected_cascade_context() {
    use raikiri::{
        Bytes, FetchOutcome, FetchedResource, MediaContext, NetworkError, NetworkProvider,
        ParseOptions, Request, Url, build_cascaded, build_cascaded_with_media_context, parse,
    };

    struct ImportProvider;

    impl NetworkProvider for ImportProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            let (css, final_url) = match request.url.as_str() {
                "https://example.test/main.css" => (
                    r#"@import "nested.css" screen;"#,
                    "https://example.test/main.css",
                ),
                "https://example.test/nested.css" => {
                    ("p { display: inline }", "https://example.test/nested.css")
                }
                _ => return Err(NetworkError::Other("not found".to_owned())),
            };
            Ok(FetchOutcome::Body(FetchedResource {
                bytes: Bytes::from(css),
                content_type: Some("text/css".to_owned()),
                final_url: Url::parse(final_url).unwrap(),
                encoding: None,
            }))
        }
    }

    let provider = ImportProvider;
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&provider as &dyn NetworkProvider),
        base_url: Some(Url::parse("https://example.test/").unwrap()),
    };
    let html = br#"<html><head><link rel="stylesheet" href="main.css"></head>
        <body><p>Hi</p></body></html>"#;
    let doc = parse(&html[..], &opts).expect("parse");
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");

    let print_result = build_cascaded(&doc);
    assert_eq!(
        print_result.computed[p_id.0 as usize].display,
        DisplayValue::Block,
        "screen-only imported rules must not apply to the default print context"
    );
    let screen_result = build_cascaded_with_media_context(&doc, &MediaContext::screen());
    assert_eq!(
        screen_result.computed[p_id.0 as usize].display,
        DisplayValue::Inline,
        "screen-only imported rules must apply in the screen context"
    );
}

#[test]
fn body_style_element_reaches_umbrella_cascade() {
    // Body styles are collected as Author stylesheets just like head styles.
    // Place the rule after the target element to prove that the final cascade
    // still applies it across the document.
    let html = "<html><head></head>\
                <body><p>Hi</p><style>p { display: inline }</style></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "body <style> must reach the umbrella Author cascade",
    );
}
#[test]
fn img_width_height_html_attributes_reach_computed_style_through_real_parse_path() {
    // The initial scope reduction assumed that implementation in
    // raikiri-style (`crate::cascade::
    // push_img_dimension_hints`) sufficed because static code inspection
    // found generic null-namespace attribute wiring into `Node.attributes`
    // in `crates/raikiri-html/src/sink.rs::wire_side_tables`. This wiring
    // had not been executed. Unlike raikiri-style's `TestDoc` unit test
    // (where the mock injects its own attributes), this umbrella test
    // supplies runtime evidence for the real html5ever TreeSink
    // (`RaikiriTreeSink`) → `Document.set_element_attributes` →
    // `ElementRef::attr()` → `StyleElement::attr()` path, using only public
    // raikiri APIs as an external consumer would.
    let doc = parse_html(r#"<html><body><img src="x.png" width="100" height="50"></body></html>"#);
    let result = build_cascaded(&doc);

    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(100.0),
        "width attribute must reach computed style via the real sink → StyleElement::attr() path"
    );
    assert_eq!(
        computed.height,
        raikiri::ComputedLengthPercentageOrAuto::Px(50.0),
        "height attribute must reach computed style via the real sink → StyleElement::attr() path"
    );
}

#[test]
fn img_width_html_attribute_overridable_by_real_author_stylesheet_through_real_parse_path() {
    // Also prove through the real path that Author CSS can override the
    // hint. An Author-origin declaration from `<style>` must beat the hint
    // after passing through `raikiri::build_cascaded` source_order resolution,
    // including `stylesheet_kind_to_origin` wiring.
    let html = r#"<html><head><style>img { width: 30px }</style></head>
                  <body><img src="x.png" width="100"></body></html>"#;
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(30.0),
        "real <style> Author rule must outrank the author-origin presentational hint \
         (same origin, real rule's non-zero specificity wins) end-to-end"
    );
}

#[test]
fn img_width_presentational_hint_beats_extra_stylesheets_user_origin_via_umbrella() {
    // Consumer-visible behavior change: `extra_stylesheets`
    // is now tagged `StylesheetKind::User` (→ `Origin::User`, normal rank 1), which
    // CSS Cascading L5 §6.5 places *below* `Origin::AuthorPresentationalHint` (normal
    // rank 2) — so `<img width>`'s presentational hint now beats an
    // `extra_stylesheets` rule regardless of specificity. Previously, `extra_stylesheets`
    // was tagged `Author` (rank 3), which beat the hint — contrast with
    // `img_width_html_attribute_overridable_by_real_author_stylesheet_through_real_parse_path`
    // above, where a *real* Author-origin rule (in-document `<style>`) still beats the
    // hint today.
    //
    // The `extra_stylesheets` rule below sets both `width` (contested by the hint,
    // since the `<img>` has a `width` attribute) and `height` (uncontested — no
    // `height` attribute, so no height hint is pushed). Asserting both distinguishes
    // "the hint outranked the width declaration" from "the stylesheet never reached
    // the RuleTree at all" (which would leave *both* properties at their initial
    // value, not just width) — `extra_stylesheets` reachability on its own is already
    // covered by `extra_stylesheets_user_rule_overrides_ua_via_umbrella` above, but this
    // test is the one cited by name from crates/raikiri-style/src/cascade.rs's
    // `push_img_dimension_hints` doc as *the* end-to-end check for the flipped ranking, so it
    // should stand alone.
    let extra: &[&str] = &["img { width: 30px; height: 7px }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&br#"<img src="x.png" width="100">"#[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let img_id = find_by_tag(&doc.dom, "img").expect("<img> exists");
    let computed = &result.computed[img_id.0 as usize];
    assert_eq!(
        computed.width,
        raikiri::ComputedLengthPercentageOrAuto::Px(100.0),
        "img width presentational hint (Origin::AuthorPresentationalHint, rank 2) must \
         beat extra_stylesheets (Origin::User, rank 1) — this ranking flipped \
         (extra_stylesheets used to be tagged Author, rank 3)"
    );
    assert_eq!(
        computed.height,
        raikiri::ComputedLengthPercentageOrAuto::Px(7.0),
        "extra_stylesheets' uncontested height declaration must still land — proves the \
         stylesheet did reach the RuleTree and it's specifically the width property that \
         lost to the hint, not the whole stylesheet being absent"
    );
}

#[test]
fn hr_is_display_block_border_inset_and_margin_via_ua_css() {
    // Acceptance: <hr> renders as a horizontal rule. HTML Living Standard
    // §the-hr-element-2 (15.3.11) specifies `hr { color: gray;
    // border-style: inset; border-width: 1px;
    // margin-block: 0.5em; margin-inline: auto; overflow: hidden; }`.
    // `display: block` comes from the separate §flow-content-3 (15.3.3)
    // group. raikiri-style lacks independent multisided border-style and
    // border-width properties and logical margin-block/margin-inline
    // properties. It does support overflow; see the minimal.css comments.
    // This test checks the cascade output of the substitute rules actually
    // declared in minimal.css (border and margin shorthands plus color),
    // not the literal specification rule.
    //
    // NB: `computed.overflow` (raikiri-style `OverflowValue`/`OverflowXY`)
    // is deliberately **not** asserted here — this
    // module's own doc states its purpose is verifying `use raikiri::…;`
    // alone suffices, and `OverflowValue`/`OverflowXY` are not (yet)
    // re-exported at the umbrella crate root (`crates/raikiri/src/lib.rs`
    // re-exports `Border`/`BorderColor`/`BorderStyle`/`LineHeight` from
    // `raikiri_style::property` for the same "consumer needs the payload
    // type to match on this `PropertyValue` variant" reason those were
    // added; `OverflowValue`/`OverflowXY` would be
    // the same shape of follow-up, but deciding the umbrella's public
    // surface is out of scope here).
    // Like `sectioning_and_grouping_elements_are_display_block_via_ua_css`
    // and `list_elements_use_list_item_display_via_ua_css`, this test uses
    // real parse → build_cascaded, not a textual scan in raikiri-html::lib:
    // a scan cannot prove that cssparser accepts the rule.
    let doc = parse_html("<html><body><hr></body></html>");
    let result = build_cascaded(&doc);

    let hr_id = find_by_tag(&doc.dom, "hr").expect("<hr> exists");
    let computed = &result.computed[hr_id.0 as usize];

    assert_eq!(
        computed.display,
        DisplayValue::Block,
        "<hr> should be display: block from the flow-content UA CSS group"
    );

    // color: gray is necessary, not decorative. The `border` shorthand
    // below omits color and defaults to currentcolor. This declaration
    // makes the border gray as specified; without it, the hr might inherit
    // a different color for its border.
    assert_eq!(
        computed.color,
        raikiri::CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        },
        "<hr> color should resolve to CSS named color `gray`"
    );

    // Replace border-style: inset plus border-width: 1px on every side
    // with the `border: 1px inset` shorthand (raikiri-style lacks separate
    // border-style/border-width properties). Check all four sides, not just
    // top, to prove the `Sides::all` shorthand expansion happened.
    for (side_name, side) in [
        ("top", &computed.border.top),
        ("right", &computed.border.right),
        ("bottom", &computed.border.bottom),
        ("left", &computed.border.left),
    ] {
        assert_eq!(
            side.width(),
            raikiri::ComputedLength(1.0),
            "<hr> border-{side_name}-width should be 1px"
        );
        assert_eq!(
            side.style(),
            raikiri::BorderStyle::Inset,
            "<hr> border-{side_name}-style should be inset"
        );
        assert_eq!(
            side.color,
            raikiri::BorderColor::CurrentColor,
            "<hr> border-{side_name}-color should be the shorthand's omitted-color default \
             (currentcolor), not an explicit color"
        );
    }

    // Fallback for margin-block: 0.5em, expanded by the margin shorthand
    // to physical margin-top/margin-bottom longhands; raikiri-style has no
    // logical margin-block property. Resolve 0.5em against the inherited
    // UA-default 16px font size.
    assert_eq!(
        computed.margin.top,
        raikiri::ComputedLengthPercentageOrAuto::Px(8.0),
        "<hr> margin-top should be 0.5em (8px at default 16px font-size), the margin-block \
         fallback"
    );
    assert_eq!(
        computed.margin.bottom,
        raikiri::ComputedLengthPercentageOrAuto::Px(8.0),
        "<hr> margin-bottom should be 0.5em (8px at default 16px font-size), the margin-block \
         fallback"
    );

    // Fallback for margin-inline: auto, expanded by the margin shorthand
    // to physical margin-left/margin-right longhands; raikiri-style has no
    // logical margin-inline property.
    assert_eq!(
        computed.margin.left,
        raikiri::ComputedLengthPercentageOrAuto::Auto,
        "<hr> margin-left should be auto, the margin-inline fallback"
    );
    assert_eq!(
        computed.margin.right,
        raikiri::ComputedLengthPercentageOrAuto::Auto,
        "<hr> margin-right should be auto, the margin-inline fallback"
    );
}

#[test]
fn flow_content_3_residue_elements_are_display_block_via_ua_css() {
    // Acceptance: the 7 §flow-content-3 (15.3.3)
    // display:block selector members that were previously untracked
    // now resolve to display: block end-to-end.
    // center/listing/plaintext/xmp are HTML LS §16.2 "entirely obsolete"
    // elements, but that classification governs authoring conformance,
    // not UA rendering — §flow-content-3 itself still lists them in the
    // same display:block selector as address/search (full reasoning in
    // minimal.css's comment). dialog, the 7th residue element, is
    // deliberately NOT in this list — its display resolves conditionally
    // on the `open` attribute rather than unconditionally to
    // `display: block`, so it needs its own attribute-driven test rather
    // than fitting this unconditional loop (see
    // `dialog_display_reflects_open_attribute_via_ua_css` below). Same
    // non-vacuous real parse -> build_cascaded pattern as
    // `sectioning_and_grouping_elements_are_display_block_via_ua_css` /
    // `list_elements_use_list_item_display_via_ua_css` /
    // `hr_is_display_block_border_inset_and_margin_via_ua_css` (the
    // raikiri-html::lib textual scan only confirms the rule text exists,
    // not that cssparser actually accepts it end-to-end).
    for tag in [
        "address",
        "center",
        "listing",
        "plaintext",
        "pre",
        "search",
        "xmp",
    ] {
        let html = format!("<html><body><{tag}>Hi</{tag}></body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let el_id = find_by_tag(&doc.dom, tag).unwrap_or_else(|| panic!("<{tag}> exists"));
        let display = result.computed[el_id.0 as usize].display;
        assert_eq!(
            display,
            DisplayValue::Block,
            "<{tag}> should be display: block from the flow-content-3 UA CSS group"
        );
    }
}

#[test]
fn dialog_display_reflects_open_attribute_via_ua_css() {
    // HTML LS §flow-content-3 (15.3.3) specifies
    // `dialog:not([open]) { display: none; }` alongside dialog's
    // membership in the section's unconditional `display: block`
    // group selector. minimal.css reproduces that pair without `:not()`
    // support via specificity instead (`dialog { display: none; }`
    // overridden by the higher-specificity `dialog[open] { display:
    // block; }` — see that file's comment for the full rationale).
    //
    // This exercises the real parse -> build_cascaded path (not a
    // raikiri-style unit test with a mock attribute), so that
    // `raikiri_dom::ElementRef::attr()`'s presence-vs-value handling of
    // the bare HTML5 boolean-attribute form (`<dialog open>`, value `""`)
    // is proven end-to-end rather than merely assumed, alongside the
    // non-empty `open="open"` form.
    let cases = [
        ("<dialog>Hi</dialog>", DisplayValue::None),
        ("<dialog open>Hi</dialog>", DisplayValue::Block),
        ("<dialog open=\"open\">Hi</dialog>", DisplayValue::Block),
    ];
    for (fragment, expected) in cases {
        let html = format!("<html><body>{fragment}</body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);

        let dialog_id = find_by_tag(&doc.dom, "dialog").expect("<dialog> exists");
        let display = result.computed[dialog_id.0 as usize].display;
        assert_eq!(
            display, expected,
            "dialog display should reflect the open attribute per minimal.css's \
             dialog/dialog[open] specificity pair (fragment: {fragment:?})"
        );
    }
}

#[test]
fn div_direction_reflects_dir_attribute_via_ua_css() {
    // HTML LS Rendering §15.3.5 bidirectional rules carried in minimal.css:
    // `[dir]:dir(ltr) { direction: ltr; }` / `[dir]:dir(rtl) { direction: rtl; }`
    // (bdi/input-tel special cases and unicode-bidi:isolate excluded, see
    // that file's comment). Exercises the real parse -> build_cascaded path.
    // Author declarations override the UA rule (presentational-hint rank).
    use raikiri_style::property::Direction;
    let cases = [
        ("<div>Hi</div>", Direction::Ltr),
        ("<div dir=ltr>Hi</div>", Direction::Ltr),
        ("<div dir=rtl>Hi</div>", Direction::Rtl),
        (
            "<div dir=rtl style=\"direction: ltr\">Hi</div>",
            Direction::Ltr,
        ),
    ];
    for (fragment, expected) in cases {
        let html = format!("<html><body>{fragment}</body></html>");
        let doc = parse_html(&html);
        let result = build_cascaded(&doc);
        let div_id = find_by_tag(&doc.dom, "div").expect("<div> exists");
        let direction = result.computed[div_id.0 as usize].direction;
        assert_eq!(
            direction, expected,
            "div direction should reflect the dir attribute per minimal.css (fragment: {fragment:?})"
        );
    }
}

#[test]
fn style_media_attribute_guards_the_stylesheet() {
    let html = r#"<html><head>
        <style media="screen">p { display: inline }</style>
        <style media="">span { display: block }</style>
        </head><body><p>Hi</p><span>x</span>
        <style media="print">div { display: inline }</style><div>y</div>
        </body></html>"#;
    let doc = parse_html(html);
    assert_eq!(
        doc.stylesheet_media,
        [
            Some("screen".to_owned()),
            Some(String::new()),
            Some("print".to_owned())
        ]
    );
    let p = find_by_tag(&doc.dom, "p").expect("<p> exists").0 as usize;
    let span = find_by_tag(&doc.dom, "span").expect("<span> exists").0 as usize;
    let div = find_by_tag(&doc.dom, "div").expect("<div> exists").0 as usize;

    let print = build_cascaded(&doc);
    assert_eq!(print.computed[p].display, DisplayValue::Block);
    assert_eq!(print.computed[span].display, DisplayValue::Block);
    assert_eq!(print.computed[div].display, DisplayValue::Inline);

    let screen = build_cascaded_with_media_context(&doc, &MediaContext::screen());
    assert_eq!(screen.computed[p].display, DisplayValue::Inline);
    assert_eq!(screen.computed[span].display, DisplayValue::Block);
    assert_eq!(screen.computed[div].display, DisplayValue::Block);
}
