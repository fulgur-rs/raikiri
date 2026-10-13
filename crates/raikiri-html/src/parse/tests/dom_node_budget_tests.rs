use super::*;

fn parse_fragment_with_limits(
    input: &[u8],
    limits: &RenderLimits,
) -> Result<UncascadedDocument, ParseError> {
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let context_name = QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from("div"),
    );
    let sink = RaikiriTreeSink::with_limits(limits.max_parse_warnings, limits.max_dom_nodes);
    let node_limit_signal = sink.dom_node_limit_signal();
    let parser = parse_html_fragment(sink, ParseOpts::default(), context_name, Vec::new(), true);
    parse_with_budgets_and_parser(
        input,
        parser,
        &options,
        limits.max_input_bytes,
        limits.max_dom_nodes,
        node_limit_signal,
    )
}

#[test]
fn fragment_node_limit_accepts_the_boundary_and_rejects_the_next_node() {
    let source = b"<template><b>x</b></template><!-- tail -->";
    let mut unlimited = RenderLimits::default();
    unlimited.max_dom_nodes = None;
    let parsed = parse_fragment_with_limits(source, &unlimited).expect("unlimited fragment parses");
    let node_count = parsed.dom.node_count() as u64;
    assert!(node_count > 1);

    let mut at_limit = RenderLimits::default();
    at_limit.max_dom_nodes = Some(node_count);
    let parsed = parse_fragment_with_limits(source, &at_limit)
        .expect("fragment at the node cap is accepted");
    assert_eq!(parsed.dom.node_count() as u64, node_count);

    let mut below_limit = RenderLimits::default();
    below_limit.max_dom_nodes = Some(node_count - 1);
    assert!(matches!(
        parse_fragment_with_limits(source, &below_limit),
        Err(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit,
            actual,
        }) if limit == node_count - 1 && actual == node_count
    ));
}
