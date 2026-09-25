use super::*;
use anyrender::Scene;
use anyrender::recording::RenderCommand;
use raikiri_dom::Document;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

#[test]
fn inline_svg_root_opacity_detection_requires_a_concrete_value() {
    assert!(inline_svg_root_has_opacity(Some("0.5"), None));
    assert!(inline_svg_root_has_opacity(
        None,
        Some("fill: red; opacity: 50% !important")
    ));
    assert!(!inline_svg_root_has_opacity(Some("var(--opacity)"), None));
    assert!(!inline_svg_root_has_opacity(
        None,
        Some("opacity: var(--opacity)")
    ));
    assert!(!inline_svg_root_has_opacity(
        None,
        Some("fill: red; opacity: invalid")
    ));
}

#[test]
fn vertical_align_length_uses_css_raise_lower_sign_in_y_down_space() {
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(96.0)),
            DisplayValue::Inline,
            16.0,
        ),
        -96.0
    );
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(-12.0)),
            DisplayValue::InlineBlock,
            16.0,
        ),
        12.0
    );
    assert_eq!(
        vertical_align_shift_px(
            VerticalAlign::Length(Length::Px(96.0)),
            DisplayValue::Block,
            16.0,
        ),
        0.0
    );
}

fn list_fixture(
    first_style: &str,
    second_style: &str,
    marker_stylesheet: Option<&str>,
) -> (Document, CascadeResult, usize, usize) {
    let mut document = Document::new();
    if let Some(stylesheet) = marker_stylesheet {
        let style = document.append_element(
            Some(document.root_index()),
            "style",
            Style::default(),
            None::<&str>,
        );
        document.append_text(style, stylesheet);
    }
    let first = document.append_element(
        Some(document.root_index()),
        "li",
        Style::default(),
        Some(first_style),
    );
    let second = document.append_element(
        Some(document.root_index()),
        "li",
        Style::default(),
        Some(second_style),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (document, cascade, first, second)
}

#[test]
fn flex_static_boxes_use_the_positioned_paint_bucket() {
    let mut document = Document::new();
    let flex = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    assert_eq!(paint_order_key(&cascade, flex), (2, 0));
}

#[test]
fn flex_grid_paint_order_uses_order_with_stable_source_order_ties() {
    fn parent_with_ordered_children(document: &mut Document, display: &str) -> (usize, [usize; 3]) {
        let parent = document.append_element(
            Some(document.root_index()),
            "div",
            Style::default(),
            Some(display),
        );
        let a = document.append_element(Some(parent), "div", Style::default(), Some("order:2"));
        let b = document.append_element(Some(parent), "div", Style::default(), Some("order:-1"));
        let c = document.append_element(Some(parent), "div", Style::default(), Some("order:2"));
        (parent, [a, b, c])
    }

    let mut document = Document::new();
    let (flex, flex_items) = parent_with_ordered_children(&mut document, "display:flex");
    let absolute_first = document.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:10;position:absolute"),
    );
    let absolute_second = document.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:5;position:absolute"),
    );
    let (grid, grid_items) = parent_with_ordered_children(&mut document, "display:grid");
    let (block, block_items) = parent_with_ordered_children(&mut document, "display:block");

    // Flex/grid items with different inner display values still share one
    // order-modified paint sequence.
    let mixed_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:grid"),
    );
    let nested_flex_item = document.append_element(
        Some(mixed_parent),
        "div",
        Style::default(),
        Some("display:flex;order:-1"),
    );
    let block_item = document.append_element(
        Some(mixed_parent),
        "div",
        Style::default(),
        Some("display:block;order:1"),
    );

    let z_index_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let z_one_late = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("z-index:1;order:1"),
    );
    let z_two_early = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("z-index:2;order:-1"),
    );
    let z_one_early = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("z-index:1;order:0"),
    );
    let positioned_z_two = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("position:relative;z-index:2;order:-2"),
    );

    // Out-of-flow flex children are treated as order 0 for painting. Their
    // authored order is ignored, with DOM order breaking the 0-value tie.
    let positioned_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let absolute_slot = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("position:absolute;order:100"),
    );
    let relative_late = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("position:relative;order:1"),
    );
    let relative_early = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("position:relative;order:-1"),
    );

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");

    let mut flex_children = document.get_node(flex).unwrap().children.clone();
    sort_paint_children(&mut flex_children, cascade.computed[flex].display, &cascade);
    assert_eq!(
        flex_children,
        [
            flex_items[1],
            flex_items[0],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );
    assert_eq!(
        document.get_node(flex).unwrap().children.as_slice(),
        &[
            flex_items[0],
            flex_items[1],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );

    let mut grid_children = document.get_node(grid).unwrap().children.clone();
    sort_paint_children(&mut grid_children, cascade.computed[grid].display, &cascade);
    assert_eq!(grid_children, [grid_items[1], grid_items[0], grid_items[2]]);
    assert_eq!(
        document.get_node(grid).unwrap().children.as_slice(),
        &grid_items
    );

    let mut mixed_children = document.get_node(mixed_parent).unwrap().children.clone();
    sort_paint_children(
        &mut mixed_children,
        cascade.computed[mixed_parent].display,
        &cascade,
    );
    assert_eq!(mixed_children, [nested_flex_item, block_item]);

    let mut z_index_children = document.get_node(z_index_parent).unwrap().children.clone();
    sort_paint_children(
        &mut z_index_children,
        cascade.computed[z_index_parent].display,
        &cascade,
    );
    assert_eq!(
        z_index_children,
        [z_one_early, z_one_late, positioned_z_two, z_two_early]
    );

    let mut positioned_children = document
        .get_node(positioned_parent)
        .unwrap()
        .children
        .clone();
    sort_paint_children(
        &mut positioned_children,
        cascade.computed[positioned_parent].display,
        &cascade,
    );
    assert_eq!(
        positioned_children,
        [relative_early, absolute_slot, relative_late]
    );

    let mut block_children = document.get_node(block).unwrap().children.clone();
    sort_paint_children(
        &mut block_children,
        cascade.computed[block].display,
        &cascade,
    );
    assert_eq!(block_children, block_items);
}

#[test]
fn list_marker_text_formats_ordinals_and_styles() {
    let (mut document, cascade, first, second) =
        list_fixture("display: list-item", "display: list-item", None);
    assert_eq!(
        list_marker_text(&document, &cascade, first, &ListStyleType::Disc),
        Some("• ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("decimal".into())
        ),
        Some("2. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("decimal-leading-zero".into())
        ),
        Some("02. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("lower-alpha".into())
        ),
        Some("b. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("upper-alpha".into())
        ),
        Some("B. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("lower-roman".into())
        ),
        Some("ii. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("upper-roman".into())
        ),
        Some("II. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("circle".into())
        ),
        Some("◦ ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("square".into())
        ),
        Some("▪ ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("custom-counter".into())
        ),
        Some("2. ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::String("§".into())
        ),
        Some("§".to_string())
    );
    assert_eq!(
        list_marker_text(&document, &cascade, second, &ListStyleType::None),
        None
    );
    // Defensive ordinal paths: the root has no parent, while a text child
    // has a parent but is not itself a list item.
    assert_eq!(
        list_item_ordinal(&document, &cascade, document.root_index()),
        1
    );
    let text = document.append_text(document.root_index(), "text");
    assert_eq!(list_item_ordinal(&document, &cascade, text), 1);
    assert_eq!(alpha_marker(0), "");
}

#[test]
fn custom_counter_styles_reach_default_and_explicit_markers() {
    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: thumbs",
        "display: list-item; list-style-type: thumbs",
        Some(
            r#"@counter-style thumbs {
                    system: cyclic;
                    symbols: "A" "B";
                    prefix: "[";
                    suffix: "] ";
                }"#,
        ),
    );
    assert_eq!(cascade.counter_styles.len(), 1);
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            first,
            &ListStyleType::Named("thumbs".into()),
        ),
        Some("[A] ".to_string())
    );
    assert_eq!(
        list_marker_text(
            &document,
            &cascade,
            second,
            &ListStyleType::Named("thumbs".into()),
        ),
        Some("[B] ".to_string())
    );

    let explicit = vec![ContentComponent::Counter {
        name: "list-item".into(),
        style: CounterStyle::Named("thumbs".into()),
    }];
    let marker = marker_content_text(
        &explicit,
        &[] as &[(&str, &str)],
        false,
        1,
        &CounterSnapshot::default(),
        &cascade.counter_styles,
    );
    assert_eq!(marker, Some("A".to_string()));

    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-type: disc",
        "display: list-item",
        Some(
            r#"@counter-style thumbs {
                    system: cyclic;
                    symbols: "A" "B";
                }
                li::marker { content: counter(list-item, thumbs); }"#,
        ),
    );
    let (_, explicit_content) =
        marker_render_info(&document, &cascade, first).expect("custom marker content");
    assert_eq!(explicit_content, "A");
}

#[test]
fn marker_render_info_resolves_named_counters_from_element_scopes() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(
            style,
            "section { counter-reset: step 4 list-item 4 } section::before { content: counter(step) } section::after { content: counters(step, \".\") } li { display: list-item; counter-increment: step list-item; list-style: none } li::marker { counter-reset: local 1; counter-increment: local 2 fresh 3; counter-set: local 9 setfresh 4; content: counter(step) \"/\" counter(list-item) \"/\" counter(local) \"/\" counter(fresh) \"/\" counter(setfresh) }",
        );
    let section = document.append_element(
        Some(document.root_index()),
        "section",
        Style::default(),
        None::<&str>,
    );
    let first = document.append_element(Some(section), "li", Style::default(), None::<&str>);
    let second = document.append_element(Some(section), "li", Style::default(), None::<&str>);
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let (_, first_content) = marker_render_info(&document, &cascade, first).expect("first marker");
    let (_, second_content) =
        marker_render_info(&document, &cascade, second).expect("second marker");
    assert_eq!(first_content, "5/5/9/3/4");
    assert_eq!(second_content, "6/6/9/3/4");
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade);
    let (_, before_content) = generated_pseudo_content_with_snapshots(
        &document,
        &cascade,
        section,
        raikiri_style::PseudoElem::Before,
        &snapshots,
    )
    .expect("generated before content");
    assert_eq!(before_content, "4");
    let (_, after_content) = generated_pseudo_content_with_snapshots(
        &document,
        &cascade,
        section,
        raikiri_style::PseudoElem::After,
        &snapshots,
    )
    .expect("generated after content");
    assert_eq!(after_content, "4");
}

#[test]
fn marker_content_text_resolves_literals_counters_and_quotes() {
    let registry = CounterStyleRegistry::new();
    let components = vec![
        ContentComponent::Quote(QuoteKeyword::OpenQuote),
        ContentComponent::Literal("(".into()),
        ContentComponent::Counter {
            name: "list-item".into(),
            style: CounterStyle::Decimal,
        },
        ContentComponent::Counters {
            name: "list-item".into(),
            separator: ".".to_string(),
            style: CounterStyle::Named("upper-roman".into()),
        },
        // Named counters are resolved from the supplied scope snapshot.
        ContentComponent::Counters {
            name: "chapter".into(),
            separator: ".".to_string(),
            style: CounterStyle::Decimal,
        },
        ContentComponent::Quote(QuoteKeyword::CloseQuote),
    ];
    let mut counters = CounterSnapshot::default();
    counters.insert(raikiri_traits::Symbol::new("chapter"), vec![1, 3]);
    assert_eq!(
        marker_content_text(&components, &[("<", ">")], false, 2, &counters, &registry,),
        Some("<(2II1.3>".to_string())
    );
    let mut list_item_counters = CounterSnapshot::default();
    list_item_counters.insert(raikiri_traits::Symbol::new("list-item"), vec![1, 2]);
    assert_eq!(
        marker_content_text(
            &components,
            &[] as &[(&str, &str)],
            false,
            2,
            &list_item_counters,
            &registry,
        ),
        Some("(2I.II".to_string())
    );
    assert_eq!(
        marker_content_text(
            &[],
            &[] as &[(&str, &str)],
            true,
            1,
            &CounterSnapshot::default(),
            &registry,
        ),
        None
    );
    assert_eq!(
        marker_content_text(
            &[
                ContentComponent::Quote(QuoteKeyword::OpenQuote),
                ContentComponent::Quote(QuoteKeyword::OpenQuote),
                ContentComponent::Quote(QuoteKeyword::CloseQuote),
                ContentComponent::Quote(QuoteKeyword::CloseQuote),
                ContentComponent::Quote(QuoteKeyword::NoOpenQuote),
                ContentComponent::Quote(QuoteKeyword::NoCloseQuote),
            ],
            &[] as &[(&str, &str)],
            true,
            1,
            &CounterSnapshot::default(),
            &registry,
        ),
        Some("“‘’”".to_string())
    );
}

#[test]
fn generated_content_resolves_dom_attributes_and_fallbacks() {
    let mut document = Document::new();
    let element = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        None::<&str>,
    );
    document.set_element_attributes(element, vec![("data-value".into(), "Actual".into())]);
    let components = vec![
        ContentComponent::AttrFallback {
            name: "missing".into(),
            fallback: Some("Fallback".into()),
        },
        ContentComponent::Literal(" ".into()),
        ContentComponent::Attr {
            name: "data-value".into(),
        },
        ContentComponent::Literal(" ".into()),
        ContentComponent::AttrFallback {
            name: "missing-invalid".into(),
            fallback: None,
        },
    ];
    let registry = CounterStyleRegistry::new();
    let rendered = content_components_to_text_with_quotes(
        &document,
        element,
        &components,
        &[] as &[(&str, &str)],
        false,
        &CounterSnapshot::default(),
        &registry,
    );
    assert_eq!(rendered.as_deref(), Some("Fallback Actual "));
}

#[test]
fn marker_render_info_uses_author_content_and_falls_back_to_list_style() {
    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: decimal",
        "display: list-item; list-style-type: none",
        Some(r##"li::marker { content: "#"; color: blue }"##),
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("marker info");
    assert_eq!(content, "#");
    let (_, content) = marker_render_info(&document, &cascade, second)
        .expect("marker rule supplies content even when list-style is none");
    assert_eq!(content, "#");

    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: decimal",
        "display: list-item; list-style-type: none",
        None,
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("fallback marker");
    assert_eq!(content, "1. ");
    assert!(marker_render_info(&document, &cascade, second).is_none());

    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-type: decimal; counter-reset: list-item 9",
        "display: list-item",
        None,
    );
    let (_, content) = marker_render_info(&document, &cascade, first)
        .expect("explicit list-item counter fallback marker");
    assert_eq!(content, "9. ");

    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-type: disc",
        "display: list-item",
        Some("li::marker { display: none }"),
    );
    assert!(marker_render_info(&document, &cascade, first).is_none());
}

#[test]
fn generated_pseudo_metrics_preserve_before_after_order() {
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::before { content: "A " } li::after { content: "B" }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade);
    assert!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            &snapshots
        ) > 0.0
    );
    assert!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::After,
            &snapshots
        ) > 0.0
    );

    let mut scene = Scene::new();
    let before_advance = paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::Before,
        0.0,
        0.0,
        200.0,
        30.0,
        &snapshots,
    );
    let after_start = scene.commands.len();
    assert!(before_advance > 0.0);
    let _ = paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::After,
        before_advance,
        0.0,
        200.0 - before_advance,
        30.0,
        &snapshots,
    );
    assert!(scene.commands.len() > after_start);

    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::before { display: none; content: "hidden" }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade);
    assert_eq!(
        generated_pseudo_text_height(
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            &snapshots
        ),
        0.0
    );
    let mut hidden_scene = Scene::new();
    assert_eq!(
        paint_generated_pseudo(
            &mut hidden_scene,
            &document,
            &cascade,
            first,
            raikiri_style::PseudoElem::Before,
            0.0,
            0.0,
            200.0,
            30.0,
            &snapshots,
        ),
        0.0
    );
}

#[test]
fn paint_document_skips_hidden_table_decoration() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let table = document.append_element(
            Some(body),
            "table",
            Style::default(),
            Some(
                "display: table; width: 100px; height: 100px; box-sizing: border-box; border: 20px solid red; border-collapse: collapse; visibility: hidden",
            ),
        );
    let row = document.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display: table-row"),
    );
    document.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display: table-cell"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(
        &mut document,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
    )
    .expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    let draws = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::Fill(_) | RenderCommand::Stroke(_)))
        .count();
    // The only draw is the page canvas; the hidden table's border is absent.
    assert_eq!(draws, 1);
}

#[test]
fn paint_document_orders_literal_pseudos_around_direct_text() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        r##"div::before { content: "BEFORE " } div::after { content: " AFTER" }"##,
    );
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = document.append_element(Some(body), "div", Style::default(), None::<&str>);
    document.append_text(div, "BODY");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(
        &mut document,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
    )
    .expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    let glyph_x: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(glyph_run) => Some(glyph_run.transform.as_coeffs()[4]),
            _ => None,
        })
        .collect();
    assert!(
        glyph_x.len() >= 3,
        // cov:ignore: assert! diagnostic is only evaluated on failure
        "expected before, text, and after glyph runs"
    );
    let last_three = &glyph_x[glyph_x.len() - 3..];
    assert!(
        last_three[0] < last_three[1] && last_three[1] < last_three[2],
        // cov:ignore: assert! diagnostic is only evaluated on failure
        "literal pseudo/text runs must advance in source order: {last_three:?}"
    );
}

#[test]
fn paint_document_offsets_generated_counter_inline_siblings() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
            style,
            r##"div { counter-reset: c } div span { counter-increment: c } div span::before { content: counter(c) } div span::after { display: none; content: "hidden" }"##,
        );
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let test = document.append_element(Some(body), "div", Style::default(), None::<&str>);
    document.append_text(test, "\n");
    document.append_element(Some(test), "span", Style::default(), None::<&str>);
    document.append_text(test, "\n");
    document.append_element(Some(test), "span", Style::default(), None::<&str>);

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(
        &mut document,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
    )
    .expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_document_expands_auto_height_for_empty_pseudo_box() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        r##"div { border: 2px solid black } div::before { content: "A" }"##,
    );
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    document.append_element(Some(body), "div", Style::default(), None::<&str>);
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    raikiri_dom::layout_single_page(
        &mut document,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
    )
    .expect("layout Ok");

    let mut scene = Scene::new();
    crate::paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_generated_pseudo_emits_counter_content() {
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; counter-reset: marker 3",
        "display: list-item",
        Some(r##"li::before { content: counter(marker) }"##),
    );
    let snapshots = raikiri_dom::counter_snapshots(&document, &cascade);
    let mut scene = Scene::new();
    paint_generated_pseudo(
        &mut scene,
        &document,
        &cascade,
        first,
        raikiri_style::PseudoElem::Before,
        0.0,
        0.0,
        200.0,
        30.0,
        &snapshots,
    );
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
}

#[test]
fn paint_list_marker_emits_text_and_honors_display_none() {
    let (document, cascade, first, second) = list_fixture(
        "display: list-item; list-style-type: disc; list-style-position: outside",
        "display: list-item; list-style-type: none",
        None,
    );
    let mut scene = Scene::new();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert!(
        scene
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::GlyphRun(_)))
    );
    let before = scene.commands.len();
    paint_list_marker(
        &mut scene, &document, &cascade, second, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(scene.commands.len(), before);

    // Inside markers use the DOM bridge's reserved gutter.
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; list-style-position: inside",
        "display: list-item",
        None,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );

    // Empty generated content and a zero-sized marker both fail closed
    // before a glyph command is emitted.
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::marker { content: none }"##),
    );
    let (_, content) = marker_render_info(&document, &cascade, first).expect("none marker");
    assert_eq!(content, "");
    let before = scene.commands.len();
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    assert_eq!(scene.commands.len(), before);
    let (document, cascade, first, _) = list_fixture(
        "display: list-item; font-size: 0",
        "display: list-item",
        None,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 0.0, 30.0, 0.0,
    );
    let (document, cascade, first, _) = list_fixture(
        "display: list-item",
        "display: list-item",
        Some(r##"li::marker { content: "\u{200b}" }"##),
    );
    paint_list_marker(
        &mut scene, &document, &cascade, first, 0.0, 0.0, 200.0, 30.0, 0.0,
    );
}

#[test]
fn border_radius_normalization_scales_adjacent_edges() {
    let normalized =
        normalize_border_radii(100.0, 100.0, RoundedRectRadii::new(80.0, 80.0, 80.0, 80.0));
    assert_eq!(normalized.top_left, 50.0);
    assert_eq!(normalized.top_right, 50.0);
    assert_eq!(normalized.bottom_right, 50.0);
    assert_eq!(normalized.bottom_left, 50.0);
    assert_eq!(
        used_border_radius(ComputedLengthPercentage::Percent(25.0), 200.0),
        50.0
    );
}

#[test]
fn border_radius_paint_keeps_lengths_but_defers_percentages() {
    let radius = ComputedBorderRadius::corners(
        ComputedLengthPercentage::Px(12.0),
        ComputedLengthPercentage::Percent(25.0),
        ComputedLengthPercentage::Px(4.0),
        ComputedLengthPercentage::Percent(50.0),
    );
    let used = paintable_border_radius(&radius, true);
    assert_eq!(used.top_left, ComputedLengthPercentage::Px(12.0));
    assert_eq!(used.top_right, ComputedLengthPercentage::Px(0.0));
    assert_eq!(used.bottom_right, ComputedLengthPercentage::Px(4.0));
    assert_eq!(used.bottom_left, ComputedLengthPercentage::Px(0.0));
}

#[test]
fn background_image_geometry_covers_supported_size_and_position_forms() {
    let decoded = raikiri_traits::DecodedImage {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
    };
    // Construct the non-exhaustive computed position values through the
    // real parser/cascade boundary rather than bypassing their visibility
    // contract with struct literals.
    let mut document = Document::new();
    let start_px_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("background-position: 3px 4px"),
    );
    let start_percent_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("background-position: 25% 50%"),
    );
    let end_px_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("background-position: right 3px bottom 4px"),
    );
    let end_percent_id = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("background-position: bottom 50% right 25%"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let repeat = cascade.computed[start_px_id].background_repeat;
    let start_px = cascade.computed[start_px_id].background_position;
    let start_percent = cascade.computed[start_percent_id].background_position;
    let end_px = cascade.computed[end_px_id].background_position;
    let end_percent = cascade.computed[end_percent_id].background_position;
    let mut scene = Scene::new();
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, 100.0, 50.0, &start_px, &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        0.0,
        0.0,
        100.0,
        50.0,
        100.0,
        50.0,
        &start_percent,
        &repeat,
    );
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, 20.0, 10.0, &end_px, &repeat,
    );
    paint_background_image(
        &mut scene,
        &decoded,
        0.0,
        0.0,
        100.0,
        50.0,
        50.0,
        12.5,
        &end_percent,
        &repeat,
    );
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, 12.0, 6.0, &start_px, &repeat,
    );
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, 22.0, 11.0, &start_px, &repeat,
    );
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, 2.0, 1.0, &start_px, &repeat,
    );
    assert!(!scene.commands.is_empty());

    let before = scene.commands.len();
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 0.0, 50.0, 100.0, 50.0, &start_px, &repeat,
    );
    let zero = raikiri_traits::DecodedImage {
        width: 0,
        height: 1,
        rgba: Vec::new(),
    };
    paint_background_image(
        &mut scene, &zero, 0.0, 0.0, 100.0, 50.0, 100.0, 50.0, &start_px, &repeat,
    );
    paint_background_image(
        &mut scene, &decoded, 0.0, 0.0, 100.0, 50.0, -1.0, 10.0, &start_px, &repeat,
    );
    assert_eq!(scene.commands.len(), before);
}

#[test]
fn background_image_dimensions_resolve_size_against_intrinsic_metadata() {
    let intrinsic = raikiri_traits::ImageIntrinsicSize {
        width: Some(2.0),
        height: Some(1.0),
        aspect_ratio: Some(2.0),
    };

    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Cover, 100.0, 50.0, intrinsic),
        Some((100.0, 50.0))
    );
    assert_eq!(
        background_image_dimensions(&ComputedBackgroundSize::Contain, 100.0, 50.0, intrinsic),
        Some((100.0, 50.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(20.0),
                height: ComputedLengthPercentageOrAuto::Px(10.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((20.0, 10.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Percent(50.0),
                height: ComputedLengthPercentageOrAuto::Percent(25.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((50.0, 12.5))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Calc(
                    raikiri_style::property::CalcLengthPercentage {
                        percent: 10.0,
                        px: 2.0,
                    },
                ),
                height: ComputedLengthPercentageOrAuto::Auto,
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((12.0, 6.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Auto,
                height: ComputedLengthPercentageOrAuto::Calc(
                    raikiri_style::property::CalcLengthPercentage {
                        percent: 20.0,
                        px: 1.0,
                    },
                ),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((22.0, 11.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Auto,
                height: ComputedLengthPercentageOrAuto::Auto,
            },
            100.0,
            50.0,
            intrinsic,
        ),
        Some((2.0, 1.0))
    );
    assert_eq!(
        background_image_dimensions(
            &ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(-1.0),
                height: ComputedLengthPercentageOrAuto::Px(10.0),
            },
            100.0,
            50.0,
            intrinsic,
        ),
        None
    );
}

#[test]
fn inherited_margin_box_font_uses_root_computed_family() {
    let mut document = Document::new();
    let style = document.append_element(
        Some(document.root_index()),
        "style",
        Style::default(),
        None::<&str>,
    );
    document.append_text(style, "@page { @top-left { content: 'x'; } }");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let rule = cascade
        .page
        .margin_boxes()
        .first()
        .expect("the @page fixture has a margin box");

    assert_eq!(
        inherited_margin_box_font(&document, &cascade, rule),
        (16.0, "serif".to_owned())
    );
}
