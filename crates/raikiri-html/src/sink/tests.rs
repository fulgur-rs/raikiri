use super::*;

mod collect_external_stylesheet_hrefs_tests;
mod find_document_base_href_tests;
mod inline_stylesheet_text_tests;
mod preloaded_stylesheet_tests;
mod stylesheet_link_tests;

#[test]
fn streaming_warnings_do_not_use_up_the_parse_error_cap() {
    let sink = RaikiriTreeSink::new(Some(2));
    sink.freeze_root_attributes();
    let attribute = Attribute {
        name: QualName::new(None, ns!(), "class".into()),
        value: "late".into(),
    };
    sink.add_attrs_if_missing(&1, vec![attribute]);
    sink.parse_error("first".into());
    sink.parse_error("second".into());
    let warnings = sink.warnings.borrow();
    let messages: Vec<&str> = warnings
        .iter()
        .filter_map(|warning| match &warning.kind {
            WarningKind::HtmlParseError { message } => Some(message.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 2, "{warnings:?}");
    assert_eq!(messages[0], "first");
    assert!(messages[1].contains("suppressed"), "{messages:?}");
    assert_eq!(warnings.len(), 3);
}
