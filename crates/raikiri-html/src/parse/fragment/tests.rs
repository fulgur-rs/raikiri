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
