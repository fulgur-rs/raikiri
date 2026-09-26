//! GcpmDirective canonical 6 variant construction pins。
//!
//! design doc §7.1 line 1913-1920 verbatim shape。variant 追加 / rename /
//! payload type 変更で fail、`#[non_exhaustive]` catch-all は無し
//! (crate-local match は non_exhaustive の enforce 外)。

use super::super::*;

#[test]
fn counter_increment_construct_and_payload_visible() {
    let d = GcpmDirective::CounterIncrement {
        name: Symbol::new("chapter"),
        delta: 1,
    };
    match d {
        GcpmDirective::CounterIncrement { name, delta } => {
            assert_eq!(name.as_str(), "chapter");
            assert_eq!(delta, 1);
        }
        other => panic!("expected CounterIncrement, got {other:?}"),
    }
}

#[test]
fn counter_reset_construct_negative_value() {
    let d = GcpmDirective::CounterReset {
        name: Symbol::new("section"),
        value: -3,
    };
    match d {
        GcpmDirective::CounterReset { name, value } => {
            assert_eq!(name.as_str(), "section");
            assert_eq!(value, -3);
        }
        other => panic!("expected CounterReset, got {other:?}"),
    }
}

#[test]
fn counter_set_construct_zero_value() {
    let d = GcpmDirective::CounterSet {
        name: Symbol::new("page"),
        value: 0,
    };
    match d {
        GcpmDirective::CounterSet { name, value } => {
            assert_eq!(name.as_str(), "page");
            assert_eq!(value, 0);
        }
        other => panic!("expected CounterSet, got {other:?}"),
    }
}

#[test]
fn string_set_carries_content_source_items() {
    let source = ContentSource::new(vec![ContentValueItem::Literal(String::from("Ch. "))]);
    let d = GcpmDirective::StringSet {
        name: Symbol::new("heading"),
        source: source.clone(),
    };
    match d {
        GcpmDirective::StringSet { name, source: s } => {
            assert_eq!(name.as_str(), "heading");
            assert_eq!(s, source);
            assert_eq!(s.items.len(), 1);
        }
        other => panic!("expected StringSet, got {other:?}"),
    }
}

#[test]
fn register_running_carries_template_id() {
    let template_id = RunningTemplateId::new(NodeId::new(42));
    let d = GcpmDirective::RegisterRunning {
        name: Symbol::new("header"),
        template_id,
    };
    match d {
        GcpmDirective::RegisterRunning {
            name,
            template_id: tid,
        } => {
            assert_eq!(name.as_str(), "header");
            assert_eq!(tid, template_id);
            assert_eq!(tid.0, NodeId::new(42));
        }
        other => panic!("expected RegisterRunning, got {other:?}"),
    }
}

#[test]
fn register_target_carries_fragment_id() {
    let d = GcpmDirective::RegisterTarget {
        fragment_id: Symbol::new("intro"),
    };
    match d {
        GcpmDirective::RegisterTarget { fragment_id } => {
            assert_eq!(fragment_id.as_str(), "intro");
        }
        other => panic!("expected RegisterTarget, got {other:?}"),
    }
}
