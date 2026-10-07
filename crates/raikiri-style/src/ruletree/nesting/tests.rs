use super::*;

#[test]
fn relative_selector_budget_includes_implicit_parent_work() {
    let namespaces = NamespaceMap::new();
    let mut input = ParserInput::new(".leaf");
    let mut parser = Parser::new(&mut input);
    let selectors = SelectorList::parse(
        &NamespacedSelectorParser::new(&namespaces),
        &mut parser,
        ParseRelative::No,
    )
    .unwrap();
    assert!(selector_list_cost(&selectors, Some(MAX_SELECTOR_WORK / 2)).is_some());
    assert!(selector_list_cost(&selectors, Some(MAX_SELECTOR_WORK)).is_none());
}
