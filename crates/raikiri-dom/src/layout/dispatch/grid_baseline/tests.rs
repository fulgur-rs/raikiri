use super::*;
use std::cell::Cell;

thread_local! {
    static WORK: Cell<Work> = const { Cell::new(Work { insertions: 0, queries: 0 }) };
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Work {
    insertions: usize,
    queries: usize,
}

pub(super) fn record_insertions(insertions: usize) {
    WORK.set(Work {
        insertions: WORK.get().insertions + insertions,
        ..WORK.get()
    });
}

pub(super) fn record_query() {
    WORK.set(Work {
        queries: WORK.get().queries + 1,
        ..WORK.get()
    });
}

#[test]
fn many_excluded_baseline_candidates_have_linear_membership_work() {
    let mut measurements = Vec::new();
    for count in [100, 500, 2_000, 8_000] {
        let mut doc = Document::new();
        let node = &mut doc.nodes[0];
        node.grid_item_row_starts = (0..=count).map(|id| (id, 1)).collect();
        node.grid_item_column_starts = (0..=count).map(|id| (id, (id + 1) as u16)).collect();
        node.grid_items_without_baseline = (0..count).collect();
        WORK.set(Work::default());
        assert_eq!(first_baseline_item(node, |_| true, |_| true), Some(count));
        let work = WORK.get();
        eprintln!("grid baseline membership: N={count}, {work:?}");
        measurements.push((count, work));
    }
    assert!(measurements.iter().all(|&(count, work)| work
        == Work {
            insertions: count,
            queries: count + 1
        }));
}

#[test]
fn selection_preserves_filtered_rows_stable_columns_and_alignment_short_circuit() {
    let mut doc = Document::new();
    let node = &mut doc.nodes[0];
    node.grid_item_row_starts = Box::new([(10, 0), (5, 2), (7, 2), (3, 2), (4, 1), (6, 2)]);
    node.grid_item_column_starts = Box::new([(10, 8), (5, 3), (7, 1), (3, 1), (4, 0), (6, 2)]);
    node.grid_items_without_baseline = Box::new([3, 6, 10, 3]);
    let calls = std::cell::RefCell::new(Vec::new());
    WORK.set(Work::default());
    let selected = first_baseline_item(
        node,
        |id| id != 10 && id != 4,
        |id| {
            calls.borrow_mut().push(id);
            id != 7
        },
    );
    assert_eq!(selected, Some(5));
    assert_eq!(*calls.borrow(), [7, 3, 6, 5]);
    assert_eq!(
        WORK.get(),
        Work {
            insertions: 4,
            queries: 3
        }
    );

    node.grid_items_without_baseline = Box::new([]);
    WORK.set(Work::default());
    assert_eq!(
        first_baseline_item(node, |id| id != 10 && id != 4, |id| id != 7),
        Some(3)
    );
    assert_eq!(
        WORK.get(),
        Work {
            insertions: 0,
            queries: 1
        }
    );
}

#[test]
fn no_eligible_baseline_falls_back_to_the_first_column_and_stable_tie() {
    let mut doc = Document::new();
    let node = &mut doc.nodes[0];
    node.grid_item_row_starts = Box::new([(8, 2), (6, 2), (4, 2), (2, 3)]);
    node.grid_item_column_starts = Box::new([(8, 2), (6, 1), (4, 1), (2, 0)]);
    node.grid_items_without_baseline = Box::new([8, 6, 4]);
    assert_eq!(first_baseline_item(node, |_| true, |_| true), Some(6));
    WORK.set(Work::default());
    assert_eq!(first_baseline_item(node, |_| true, |_| false), Some(6));
    assert_eq!(
        WORK.get(),
        Work {
            insertions: 3,
            queries: 0
        }
    );
    WORK.set(Work::default());
    assert_eq!(
        first_baseline_item(node, |_| false, |_| panic!("no item")),
        None
    );
    assert_eq!(WORK.get(), Work::default());
    node.grid_item_row_starts = Box::new([]);
    node.grid_item_column_starts = Box::new([]);
    assert_eq!(
        first_baseline_item(node, |_| true, |_| panic!("empty grid")),
        None
    );
}

#[test]
fn public_layout_records_percentage_exclusions_and_selects_the_last_eligible_item() {
    for count in [100, 500, 2_000, 8_000] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
        let grid = doc.append_element(Some(body), "div", Some(&format!(
            "display:grid;width:{}px;height:10px;grid-template-columns:repeat({},1px);grid-template-rows:auto;align-items:baseline", count + 1, count + 1
        )));
        let children = (0..=count)
            .map(|index| {
                doc.append_element(
                    Some(grid),
                    "div",
                    Some(&format!(
                        "display:block;width:1px;height:{};grid-row:1;grid-column:{}",
                        if index < count { "50%" } else { "1px" },
                        index + 1
                    )),
                )
            })
            .collect::<Vec<_>>();
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        crate::layout::layout_single_page(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let node = &doc.nodes[grid];
        assert_eq!(&*node.grid_items_without_baseline, &children[..count]);
        assert_eq!(node.exported_first_baseline, Some(1.0));
        for (index, &child) in children.iter().enumerate() {
            assert_eq!(doc.nodes[child].layout.size.width, 1.0);
            assert_eq!(doc.nodes[child].layout.location.x, index as f32);
        }
        WORK.set(Work::default());
        assert_eq!(
            first_baseline_item(node, |_| true, |_| true),
            children.last().copied()
        );
        assert_eq!(
            WORK.get(),
            Work {
                insertions: count,
                queries: count + 1
            }
        );
        eprintln!("public grid baseline: N={count}, {:?}", WORK.get());
    }
}
