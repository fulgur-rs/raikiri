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
    parse_with_budgets_and_parser(input, parser, &options, limits.max_input_bytes, || {
        dom_node_budget_error(limits.max_dom_nodes, &node_limit_signal)
    })
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

#[test]
fn nodes_the_parser_adds_at_the_end_of_input_count_against_the_limit() {
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    // Empty input reads no chunk: the html, head, and body elements all come
    // from finishing the parse.
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = Some(1);
    assert!(matches!(
        parse_with_budgets(&b""[..], &options, &limits),
        Err(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit: 1,
            actual: 2,
        })
    ));
}

#[test]
fn public_fragment_budget_counts_templates_but_excludes_parser_wrappers() {
    for tag in ["div", "template", "table"] {
        let options = ParseOptions::new();
        let mut limits = RenderLimits::default();
        limits.max_dom_nodes = None;
        let result = super::super::parse_fragment_with_limits(
            &b""[..],
            &options,
            tag,
            "http://www.w3.org/1999/xhtml",
            true,
            &limits,
            Some(0),
        );
        assert!(result.is_ok(), "{tag}: {result:?}");
    }
    for (source, count) in [("<b>x</b>", 2), ("<template><b>x</b></template>", 4)] {
        for limit in [count - 1, count, count + 1] {
            let mut limits = RenderLimits::default();
            limits.max_dom_nodes = None;
            let result = super::super::parse_fragment_with_limits(
                source.as_bytes(),
                &ParseOptions::new(),
                "div",
                "http://www.w3.org/1999/xhtml",
                true,
                &limits,
                Some(limit),
            );
            if limit < count {
                assert!(matches!(
                    result,
                    Err(ParseError::LimitExceeded {
                        kind: LimitKind::DomNodes,
                        ..
                    })
                ));
            } else {
                assert!(result.is_ok(), "{result:?}");
            }
        }
    }
}

#[test]
fn tiny_fragment_budget_stops_reading_large_input_early() {
    struct Counted<'a> {
        input: &'a [u8],
        read: &'a Cell<usize>,
    }
    impl Read for Counted<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let count = self.input.read(buffer)?;
            self.read.set(self.read.get() + count);
            Ok(count)
        }
    }
    let input = "<b>x</b>".repeat(1_000_000);
    let read = Cell::new(0);
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = None;
    let result = super::super::parse_fragment_with_limits(
        Counted {
            input: input.as_bytes(),
            read: &read,
        },
        &ParseOptions::new(),
        "div",
        "http://www.w3.org/1999/xhtml",
        true,
        &limits,
        Some(1),
    );
    assert!(matches!(
        result,
        Err(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            ..
        })
    ));
    assert!(read.get() <= 65536);
}

#[test]
fn fragment_budget_handles_malformed_formatting_tables_and_foreign_content() {
    for markup in [
        "<b><i><p>a</b>b</i>c</p>",
        "<table><b>x<tr><td><i>y</b>z</table>",
        "<template><template><b>x</b></template></template>",
        "<svg><foreignObject><b>x</b></foreignObject></svg>",
        "<b><i><u><em><strong><p>x</b>x</i>x</u>x</em>x</strong>",
    ] {
        for cap in 0..30 {
            let mut limits = RenderLimits::default();
            limits.max_dom_nodes = None;
            let result = super::super::parse_fragment_with_limits(
                markup.as_bytes(),
                &ParseOptions::new(),
                "div",
                "http://www.w3.org/1999/xhtml",
                true,
                &limits,
                Some(cap),
            );
            if let Err(error) = result {
                assert!(matches!(
                    error,
                    ParseError::LimitExceeded {
                        kind: LimitKind::DomNodes,
                        ..
                    }
                ));
            }
        }
    }
}

#[test]
fn fragment_byte_limit_remains_distinct_from_node_limit() {
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = Some(10);
    limits.max_input_bytes = Some(1);
    let result = super::super::parse_fragment_with_limits(
        &b"<b>x</b>"[..],
        &ParseOptions::new(),
        "div",
        "http://www.w3.org/1999/xhtml",
        true,
        &limits,
        None,
    );
    assert!(matches!(
        result,
        Err(ParseError::LimitExceeded {
            kind: LimitKind::InputBytes,
            ..
        })
    ));
}

#[test]
fn fragment_total_arena_limit_preserves_wrapper_accounting() {
    for (context, overhead) in [("div", 3), ("template", 4)] {
        for cap in 0..=overhead {
            let mut limits = RenderLimits::default();
            limits.max_dom_nodes = Some(cap);
            let result = super::super::parse_fragment_with_limits(
                &b""[..],
                &ParseOptions::new(),
                context,
                "http://www.w3.org/1999/xhtml",
                true,
                &limits,
                Some(0),
            );
            if cap == overhead {
                assert_eq!(result.unwrap().dom.node_count() as u64, overhead);
            } else {
                assert!(matches!(result, Err(ParseError::LimitExceeded {
                    kind: LimitKind::DomNodes, limit, ..
                }) if limit == cap));
            }
        }
    }
}
