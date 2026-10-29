use super::*;

#[test]
fn exhausted_budget_suspends_before_later_tokens_or_arena_growth() {
    let sink = RaikiriTreeSink::with_limits(None, Some(4));
    let name = QualName::new(None, markup5ever::ns!(html), "div".into());
    let mut parser = BudgetedFragmentParser::new(sink, name, true);
    parser.process(StrTendril::from_slice(&"<b>x</b>".repeat(100_000)));
    assert_eq!(
        parser.tokenizer.sink.0.sink.dom_node_limit_error(),
        Some((4, 5))
    );
    assert!(!parser.input.is_empty());
    let document = parser.tokenizer.sink.0.sink.finish();
    assert_eq!(document.dom.node_count(), 4);
}

#[test]
fn exhausted_character_budget_skips_comments_and_text_until_the_next_tag() {
    let sink = RaikiriTreeSink::with_limits(None, Some(4));
    let name = QualName::new(None, markup5ever::ns!(html), "div".into());
    let mut parser = BudgetedFragmentParser::new(sink, name, true);
    parser.process(StrTendril::from_slice(
        "<b>x<!-- ignored -->tail</b><p>later</p>",
    ));
    assert_eq!(
        parser.tokenizer.sink.0.sink.dom_node_limit_error(),
        Some((4, 5))
    );
    assert!(!parser.input.is_empty());
    assert_eq!(parser.finish().dom.node_count(), 4);
}

#[test]
fn foreign_context_preserves_cdata_as_text() {
    let sink = RaikiriTreeSink::with_limits(None, None);
    let name = QualName::new(None, markup5ever::ns!(svg), "svg".into());
    let mut parser = BudgetedFragmentParser::new(sink, name, true);
    parser.process(StrTendril::from_slice("<![CDATA[<not-an-element>]]>"));
    let document = parser.finish().dom;
    let fragment = document.get_node(document.root_index()).unwrap().children[0];
    assert_eq!(
        document.element_text_content(fragment).as_deref(),
        Some("<not-an-element>")
    );
}

#[test]
fn upstream_sink_errors_remain_parse_warnings() {
    let sink = RaikiriTreeSink::with_limits(None, None);
    let name = QualName::new(None, markup5ever::ns!(html), "div".into());
    let mut parser = BudgetedFragmentParser::new(sink, name, true);
    parser.error(Cow::Borrowed("fragment decoder diagnostic"));
    let document = parser.finish();
    assert!(document.warnings.iter().any(|warning| {
        matches!(&warning.kind, raikiri_traits::WarningKind::HtmlParseError { message } if message == "fragment decoder diagnostic")
    }));
}
