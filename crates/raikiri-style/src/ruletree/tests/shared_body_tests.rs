use super::*;
use std::collections::HashMap;

fn at(node: &RuleNode) -> &AtRuleRecord {
    let RuleNode::AtRule(record) = node else {
        panic!("expected at-rule")
    };
    record
}
fn shared(body: &AtRuleBody) -> &SharedAtRuleBlock {
    let AtRuleBody::SharedBlock(body) = body else {
        panic!("expected shared block")
    };
    body
}

#[test]
fn deep_inspection_uses_one_shared_backing_and_keeps_its_logical_budget() {
    let depth = 64;
    let css = format!(
        "{}/*{}*/ .probe {{ color: red }}{}",
        "@opaque {".repeat(depth),
        "x".repeat(32768),
        "}".repeat(depth)
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::Author);
    let root = &tree.opaque_at_rules()[0];
    assert!(matches!(&root.body, AtRuleBody::Block(_)));
    let mut owners = HashMap::new();
    let mut owned = root.body.as_block().unwrap().len();
    let mut logical = 0;
    let mut count = 1;
    let mut nonzero_offset = false;
    let mut pending: Vec<_> = root.children.iter().collect();
    while let Some(node) = pending.pop() {
        count += 1;
        match node {
            RuleNode::AtRule(record) => {
                let body = shared(&record.body);
                logical += body.as_str().len();
                nonzero_offset |= body.range.start != 0;
                owners
                    .entry(Arc::as_ptr(&body.source) as *const u8)
                    .or_insert(body.source.len());
                pending.extend(record.children.iter());
            }
            RuleNode::Qualified(record) => {
                logical += record.body.len();
                owned += record.body.len();
            }
        }
    }
    assert_eq!(count, depth + 1);
    assert_eq!(owners.len(), 1);
    assert!(nonzero_offset);
    assert!(owners.values().sum::<usize>() + owned <= 3 * css.len());
    assert!(logical > 32768 * 60);
    assert_eq!(
        tree.opaque_budget.body_bytes,
        MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES - logical
    );
    assert_eq!(tree.opaque_budget.nodes, MAX_NESTED_OPAQUE_NODES - depth);
}

#[test]
fn unicode_offsets_and_siblings_do_not_retain_unrelated_parent_comments() {
    let huge = "z".repeat(65536);
    let css = format!(
        "@outer {{ /*{huge}*/ @一 雪 {{ @二 {{ a {{ content: '🌱' }} }} }} /*{huge}*/ @sibling {{ q {{ z: w }} }} }}"
    );
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(&css, Origin::User);
    let nodes = tree.opaque_at_rules()[0].children();
    assert_eq!(nodes.len(), 2);
    let first = at(&nodes[0]);
    let sibling = at(&nodes[1]);
    let nested = at(&first.children()[0]);
    assert_eq!(first.name, "一");
    assert_eq!(first.prelude, " 雪 ");
    assert_eq!(first.body.as_block(), Some(" @二 { a { content: '🌱' } } "));
    assert_eq!(nested.body.as_block(), Some(" a { content: '🌱' } "));
    assert_eq!(sibling.body.as_block(), Some(" q { z: w } "));
    assert_eq!((first.source_order, sibling.source_order), (0, 1));
    assert_eq!(
        (first.origin, nested.origin, sibling.origin),
        (Origin::User, Origin::User, Origin::User)
    );
    let first = shared(&first.body);
    let nested = shared(&nested.body);
    let sibling = shared(&sibling.body);
    assert!(Arc::ptr_eq(&first.source, &nested.source));
    assert!(!Arc::ptr_eq(&first.source, &sibling.source));
    assert!(nested.range.start > first.range.start);
    assert!(first.source.len() + sibling.source.len() < 1024);
}

#[test]
fn shared_value_equality_and_debug_use_only_the_view_text() {
    let prefix = "前置:";
    let value = "本体";
    let backing = SharedAtRuleBlock::new("前置:本体:後置");
    let view = backing.slice(prefix.len()..prefix.len() + value.len());
    let other = SharedAtRuleBlock::new(value);
    assert!(!Arc::ptr_eq(&view.source, &other.source));
    assert_eq!(view, other);
    assert_eq!(format!("{view:?}"), format!("{value:?}"));
    assert_eq!(
        AtRuleBody::SharedBlock(view.clone()),
        AtRuleBody::Block(value.into())
    );
    assert_eq!(
        AtRuleBody::Block(value.into()),
        AtRuleBody::SharedBlock(view.clone())
    );
    assert_ne!(AtRuleBody::Statement, AtRuleBody::SharedBlock(view.clone()));
    assert_eq!(
        AtRuleBody::SharedBlock(SharedAtRuleBlock::new("")),
        AtRuleBody::Block(String::new())
    );
    let record = AtRuleRecord {
        name: "future".into(),
        prelude: " p ".into(),
        body: AtRuleBody::SharedBlock(view),
        children: Vec::new(),
        source_order: 0,
        origin: Origin::Author,
    };
    assert_eq!(record.to_css(), "@future p {本体}");
    assert_record_round_trips(&record);
}

#[test]
fn a_cloned_child_owns_its_text_after_input_and_parent_drop() {
    let cloned = {
        let source = String::from("@root { @first { @second { value: 雪 } } }");
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&source, Origin::Author);
        let first = at(&tree.opaque_at_rules()[0].children()[0]);
        at(&first.children()[0]).clone()
    };
    assert_eq!(cloned.body.as_block(), Some(" value: 雪 "));
    assert_eq!(cloned.to_css(), "@second { value: 雪 }");
    assert_record_round_trips(&cloned);
}

#[test]
fn top_level_and_empty_blocks_keep_the_legacy_representation() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@root { @empty{} @full { x } @statement; }", Origin::Author);
    let record = &tree.opaque_at_rules()[0];
    assert!(matches!(&record.body, AtRuleBody::Block(_)));
    let children = record.children();
    assert_eq!(children.len(), 3);
    assert!(matches!(&at(&children[0]).body,AtRuleBody::Block(value) if value.is_empty()));
    assert_eq!(at(&children[0]).body.as_block(), Some(""));
    assert!(matches!(&at(&children[1]).body, AtRuleBody::SharedBlock(_)));
    assert_eq!(at(&children[2]).body, AtRuleBody::Statement);
    assert_eq!(at(&children[2]).body.as_block(), None);
    assert_record_round_trips(record);
}

#[test]
fn unchanged_values_borrow_and_eof_edits_own_the_completed_text() {
    for source in ["", " x ", " p { color: red }", " f([x]) /* c */"] {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let value = consume_raw_component_values_view(&mut parser, source).unwrap();
        assert!(matches!(value, Cow::Borrowed(_)));
        assert_eq!(value, source);
    }
    for tail in [
        " a: \"x",
        " a: \"x\\",
        " a: x\\",
        " a: url(x\\",
        " /* note",
        " a: f([x",
        " p { a: (\"x",
    ] {
        let expected = complete_raw_block(tail, true).unwrap();
        let mut input = ParserInput::new(tail);
        let mut parser = Parser::new(&mut input);
        let value = consume_raw_component_values_view(&mut parser, tail).unwrap();
        assert!(matches!(value, Cow::Owned(_)));
        assert_eq!(value, expected);
        let source = format!("@a {{ @b {{{tail}");
        let nodes = parse_nested_rule_nodes(&source, &mut OpaqueBudget::new());
        let outer = at(&nodes[0]);
        let inner = at(&outer.children()[0]);
        assert_eq!(inner.body.as_block(), Some(expected.as_str()));
        assert!(Arc::ptr_eq(
            &shared(&outer.body).source,
            &shared(&inner.body).source
        ));
        assert_record_round_trips(outer);
    }
}

#[test]
fn eof_completion_in_a_shared_frame_starts_a_new_backing_root() {
    let original = SharedAtRuleBlock::new("前置 @a { a: x\\");
    let view = original.slice("前置 ".len()..original.as_str().len());
    let nodes =
        parse_nested_rule_nodes_from(RawRuleSource::Shared(&view), 0, &mut OpaqueBudget::new());
    let block = shared(&at(&nodes[0]).body);
    assert_eq!(block.as_str(), " a: x\u{FFFD}");
    assert!(!Arc::ptr_eq(&original.source, &block.source));
    assert_eq!(block.range, 0..block.as_str().len());
}

#[test]
fn logical_budgets_count_normalized_text_before_source_retention() {
    let mut budget = OpaqueBudget {
        body_bytes: 3,
        nodes: 4,
    };
    let nodes = parse_nested_rule_nodes("@a { x } @b {y}", &mut budget);
    assert_eq!(nodes.len(), 1);
    assert_eq!((budget.body_bytes, budget.nodes), (0, 3));
    let mut budget = OpaqueBudget {
        body_bytes: 3,
        nodes: 1,
    };
    assert!(parse_nested_rule_nodes("@a{a\\", &mut budget).is_empty());
    assert_eq!((budget.body_bytes, budget.nodes), (3, 1));
    let mut budget = OpaqueBudget {
        body_bytes: 4,
        nodes: 1,
    };
    let nodes = parse_nested_rule_nodes("@a{a\\", &mut budget);
    assert_eq!(at(&nodes[0]).body.as_block(), Some("a\u{FFFD}"));
    assert_eq!((budget.body_bytes, budget.nodes), (0, 0));
}
