use super::*;
use crate::RuleTree;
use crate::media::{MediaType, parse_media_prelude};
use crate::page::{PageContextQuery, PageInheritance, cascade_page_with_media_context};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    builds: usize,
    node_slots: usize,
    mentions: usize,
}

thread_local! {
    static WORK: Cell<Work> = Cell::new(Work::default());
}

pub(super) fn record_build(nodes: usize) {
    WORK.with(|work| {
        let mut value = work.get();
        value.builds += 1;
        value.node_slots += nodes;
        work.set(value);
    });
}

pub(super) fn record_mention() {
    WORK.with(|work| {
        let mut value = work.get();
        value.mentions += 1;
        work.set(value);
    });
}

fn measured<T>(operation: impl FnOnce() -> T) -> (T, Work) {
    WORK.set(Work::default());
    let value = operation();
    (value, WORK.get())
}

#[test]
fn distinct_page_queries_build_layer_order_once() {
    for count in [16, 64, 256, 16_000] {
        for layered in [false, true] {
            let mut tree = RuleTree::empty();
            let mut css = String::new();
            for index in 0..count {
                css.push_str(&format!("@layer l{index};"));
            }
            css.push_str(if layered {
                "@layer l0 { @page { color: red } }"
            } else {
                "@page { color: red }"
            });
            tree.add_stylesheet(&css, Origin::Author);
            assert!(tree.limit_exceeded().is_none());
            let (_, work) = measured(|| {
                for index in 0..100 {
                    let query = PageContextQuery {
                        page_name: Some(format!("page{index}").into()),
                        ..PageContextQuery::default()
                    };
                    let page = cascade_page_with_media_context(
                        &tree,
                        &query,
                        PageInheritance::LegacyInitialValues,
                        &MediaContext::print(),
                    );
                    assert_eq!(page.declarations().len(), 1);
                }
            });
            assert_eq!(
                work,
                Work {
                    builds: 1,
                    node_slots: count,
                    mentions: count + usize::from(layered)
                }
            );
        }
    }
}

fn declare(table: &mut LayerTable, name: &str, condition: Option<&str>) -> LayerId {
    let condition = condition.map(|text| parse_media_prelude(text).unwrap());
    table.declare(
        None,
        Some(&vec![name.into()]),
        Origin::Author,
        condition.as_ref(),
    )
}

#[test]
fn media_key_includes_type_width_height_and_compatibility_none() {
    let mut table = LayerTable::default();
    let ids = [
        declare(&mut table, "a", Some("print")),
        declare(&mut table, "b", None),
        declare(&mut table, "c", Some("(min-width:600px)")),
        declare(&mut table, "d", Some("(min-height:500px)")),
    ];
    for (context, expected) in [
        (Some(MediaContext::print()), [0, 1, u32::MAX, u32::MAX]),
        (
            Some(MediaContext::with_viewport(MediaType::Screen, 480, 288)),
            [u32::MAX, 0, u32::MAX, u32::MAX],
        ),
        (
            Some(MediaContext::with_viewport(MediaType::Screen, 800, 288)),
            [u32::MAX, 0, 1, u32::MAX],
        ),
        (Some(MediaContext::screen()), [u32::MAX, 0, 1, 2]),
        (None, [u32::MAX, 0, u32::MAX, u32::MAX]),
        (Some(MediaContext::print()), [0, 1, u32::MAX, u32::MAX]),
    ] {
        let (_, work) = measured(|| {
            for _ in 0..2 {
                let order = table.order(context.as_ref());
                assert_eq!(ids.map(|id| order.rank(Some(id), Origin::Author)), expected);
            }
        });
        assert_eq!(
            work,
            Work {
                builds: 1,
                node_slots: 4,
                mentions: 4
            }
        );
    }
}

#[test]
fn stylesheet_additions_invalidate_new_and_reopened_layers() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@media screen { @layer a { @page { color:red } } } @layer b { @page { color:blue } }",
        Origin::Author,
    );
    let a = tree.page_rules[0].layer;
    let b = tree.page_rules[1].layer;
    let context = MediaContext::print();
    let old = tree.layer_order(&context);
    assert_eq!(old.rank(a, Origin::Author), u32::MAX);
    assert_eq!(old.rank(b, Origin::Author), 0);
    drop(old);
    tree.add_stylesheet("@layer a { @page { color:red } }", Origin::Author);
    let ((), work) = measured(|| {
        let order = tree.layer_order(&context);
        assert_eq!(order.rank(a, Origin::Author), 1);
        assert_eq!(order.rank(b, Origin::Author), 0);
    });
    assert_eq!(
        work,
        Work {
            builds: 1,
            node_slots: 2,
            mentions: 3
        }
    );
    tree.add_stylesheet("@layer c { @page { color:green } }", Origin::Author);
    let c = tree.page_rules.last().unwrap().layer;
    let order = tree.layer_order(&context);
    assert_eq!(order.rank(c, Origin::Author), 2);
    assert_eq!(order.rank(a, Origin::Author), 1);
}

#[test]
fn old_order_keeps_ranks_after_another_context_evicts_it() {
    let mut table = LayerTable::default();
    let a = declare(&mut table, "a", Some("print"));
    let b = declare(&mut table, "b", None);
    let print = table.order(Some(&MediaContext::print()));
    let screen = table.order(Some(&MediaContext::screen()));
    assert_eq!(print.rank(Some(a), Origin::Author), 0);
    assert_eq!(print.rank(Some(b), Origin::Author), 1);
    assert_eq!(screen.rank(Some(a), Origin::Author), u32::MAX);
    assert_eq!(screen.rank(Some(b), Origin::Author), 0);
    let again = table.order(Some(&MediaContext::print()));
    assert_eq!(again.rank(Some(b), Origin::Author), 1);
    assert_eq!(print.rank(Some(b), Origin::Author), 1);
}

#[test]
fn concurrent_same_context_builds_once_and_preserves_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LayerTable>();
    assert_send_sync::<RuleTree>();
    let mut table = LayerTable::default();
    let id = declare(&mut table, "a", None);
    let barrier = std::sync::Barrier::new(8);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    measured(|| table.order(Some(&MediaContext::print())))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results.iter().map(|(_, work)| work.builds).sum::<usize>(),
        1
    );
    for (order, _) in &results {
        assert_eq!(order.rank(Some(id), Origin::Author), 0);
        assert_eq!(order.ranks.as_ptr(), results[0].0.ranks.as_ptr());
    }
}

#[test]
fn cached_pages_match_fresh_pages_for_nested_layers_and_rollback() {
    let css = "@layer outer { @layer a { @page { --ink:red; color:var(--ink); size:200px 300px; @top-left { content:'A'; color:red } } } @layer b { @page { color:blue; color:revert-layer; @top-left { color:blue; color:revert-layer } } } } @layer { @page { margin-top:5px!important } } @media screen { @layer first { @page { color:green } } }";
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    tree.add_stylesheet(
        "@layer user { @page { margin-top:7px!important } }",
        Origin::User,
    );
    for context in [
        MediaContext::print(),
        MediaContext::screen(),
        MediaContext::print(),
    ] {
        for index in 0..4 {
            let query = PageContextQuery {
                page_name: Some(format!("q{index}").into()),
                ..PageContextQuery::default()
            };
            let mut fresh = RuleTree::empty();
            fresh.add_stylesheet(css, Origin::Author);
            fresh.add_stylesheet(
                "@layer user { @page { margin-top:7px!important } }",
                Origin::User,
            );
            assert_eq!(
                cascade_page_with_media_context(
                    &tree,
                    &query,
                    PageInheritance::LegacyInitialValues,
                    &context
                ),
                cascade_page_with_media_context(
                    &fresh,
                    &query,
                    PageInheritance::LegacyInitialValues,
                    &context
                )
            );
        }
    }
}

#[test]
fn cached_rank_still_rejects_foreign_owner_and_wrong_origin() {
    let mut first = LayerTable::default();
    let local = declare(&mut first, "a", None);
    let mut second = LayerTable::default();
    let foreign = declare(&mut second, "a", None);
    for _ in 0..2 {
        let order = first.order(Some(&MediaContext::print()));
        assert_eq!(order.rank(Some(local), Origin::Author), 0);
        assert_eq!(order.rank(Some(local), Origin::User), u32::MAX);
        assert_eq!(order.rank(Some(foreign), Origin::Author), u32::MAX);
        assert_eq!(order.priority(None, Origin::Author, true), 0);
        assert_eq!(order.priority(Some(local), Origin::Author, true), u32::MAX);
    }
}

#[test]
fn poisoned_cache_recovers_reads_and_declaration_invalidation() {
    for warm in [false, true] {
        let mut table = LayerTable::default();
        let a = declare(&mut table, "a", Some("screen"));
        let b = declare(&mut table, "b", None);
        if warm {
            drop(table.order(Some(&MediaContext::print())));
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = table.order_cache.lock().unwrap();
                panic!("exercise derived cache poison recovery");
            }))
            .is_err()
        );
        let order = table.order(Some(&MediaContext::print()));
        assert_eq!(order.rank(Some(a), Origin::Author), u32::MAX);
        assert_eq!(order.rank(Some(b), Origin::Author), 0);
        drop(order);
        declare(&mut table, "a", None);
        let order = table.order(Some(&MediaContext::print()));
        assert_eq!(order.rank(Some(a), Origin::Author), 1);
        assert_eq!(order.rank(Some(b), Origin::Author), 0);
    }
}

#[test]
fn cache_drops_evicted_or_invalidated_ranks_when_no_order_retains_them() {
    let mut table = LayerTable::default();
    declare(&mut table, "a", None);
    let order = table.order(Some(&MediaContext::print()));
    let first = Arc::downgrade(&order.ranks);
    drop(order);
    assert!(first.upgrade().is_some());
    let screen = table.order(Some(&MediaContext::screen()));
    assert!(first.upgrade().is_none());
    let second = Arc::downgrade(&screen.ranks);
    drop(screen);
    declare(&mut table, "a", Some("print"));
    assert!(second.upgrade().is_none());
}
