use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

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

fn marker_render_info<'a>(
    document: &'a Document,
    cascade: &'a CascadeResult,
    node_id: usize,
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    let snapshots = crate::counter_snapshots(document, cascade).expect("counter snapshots");
    marker_render_info_with_snapshots(document, cascade, node_id, &snapshots)
}
fn generated_pseudo_content_with_snapshots<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    node_id: usize,
    pseudo: raikiri_style::PseudoElem,
    snapshots: &[CounterSnapshot],
) -> Option<(&'a raikiri_style::ComputedValues, String)> {
    crate::generated_content::generated_text(document, cascade, node_id, pseudo, snapshots)
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
fn running_element_content_is_not_resolved_as_marker_text() {
    let marker = marker_content_text(
        &[ContentComponent::Element {
            name: "header".into(),
        }],
        &[] as &[(&str, &str)],
        false,
        1,
        &CounterSnapshot::default(),
        &CounterStyleRegistry::default(),
    );
    assert_eq!(marker, Some(String::new()));
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
    let snapshots = crate::counter_snapshots(&document, &cascade)
        .expect("test counter snapshots stay within budget");
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
