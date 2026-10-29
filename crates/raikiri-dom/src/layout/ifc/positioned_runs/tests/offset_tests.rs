use super::*;
use std::cell::Cell;

thread_local! {
    static INDEX_ENTRIES: Cell<usize> = const { Cell::new(0) };
    static PROBES: Cell<usize> = const { Cell::new(0) };
    static ANCESTORS: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn note_index_entry() {
    INDEX_ENTRIES.set(INDEX_ENTRIES.get() + 1);
}

pub(crate) fn note_probe() {
    PROBES.set(PROBES.get() + 1);
}

pub(crate) fn note_ancestor() {
    ANCESTORS.set(ANCESTORS.get() + 1);
}

#[test]
fn relative_span_run_lookup_work_is_linear() {
    for count in [32, 64, 128] {
        for relative in [false, true] {
            INDEX_ENTRIES.set(0);
            let fixture = block_fixture("width:100000px", |doc, root| {
                for _ in 0..count {
                    let span = doc.append_element(
                        Some(root),
                        "span",
                        Some(if relative {
                            "position:relative;left:1px"
                        } else {
                            ""
                        }),
                    );
                    doc.append_text(span, "X ");
                }
            });
            let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
            layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
            let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
            assert_eq!(view.offsets.len(), if relative { count } else { 0 });
            assert!(INDEX_ENTRIES.get() <= 4 * count);
            PROBES.set(0);
            ANCESTORS.set(0);
            let runs: Vec<_> = view.lines().flat_map(|line| line.runs).collect();
            assert_eq!(runs.len(), count);
            assert!(
                runs.iter()
                    .all(|run| run.offset == (if relative { 1.0 } else { 0.0 }, 0.0))
            );
            let probes = PROBES.get();
            let ancestors = ANCESTORS.get();
            assert!(probes <= 3 * count, "{count}: {probes} probes");
            assert!(ancestors <= 3 * count, "{count}: {ancestors} ancestors");
            if !relative {
                assert_eq!((probes, ancestors), (0, 0));
            }
        }
    }
}

#[test]
fn indexed_and_independent_slices_keep_nested_generated_offsets_in_each_writing_mode() {
    use crate::generated_content::generated_node_id;
    use crate::layout::ifc::test_support::sheet_fixture;
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        let fixture = sheet_fixture(
            "span::before {content:'p';position:relative;left:2px;top:3px}",
            &format!("width:100px;height:80px;writing-mode:{mode};direction:rtl"),
            |doc, root| {
                let outer = doc.append_element(
                    Some(root),
                    "span",
                    Some("position:relative;left:10%;top:5px"),
                );
                let inner = doc.append_element(
                    Some(outer),
                    "span",
                    Some("position:relative;left:-3px;top:2px"),
                );
                doc.append_text(inner, "abc");
            },
        );
        let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        let offsets = doc.ifc_layout_node(root).unwrap().ifc_relative_offsets();
        let independent = offsets.to_vec();
        assert!(offsets.len() >= 4);
        for node in 0..doc.node_count() {
            for owner in [
                node,
                generated_node_id(node, PseudoElem::Before),
                generated_node_id(node, PseudoElem::After),
            ] {
                assert_eq!(
                    cumulative_offset(&doc, root, offsets, owner),
                    cumulative_offset(&doc, root, &independent, owner)
                );
            }
        }
        let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
        for run in view.lines().flat_map(|line| line.runs) {
            assert_eq!(
                run.offset,
                cumulative_offset(&doc, root, &independent, run.owner)
            );
        }
        // A subslice has its own first entry and cannot use the full index.
        assert_eq!(
            cumulative_offset(&doc, root, &offsets[1..], offsets[0].0),
            cumulative_offset(&doc, root, &independent[1..], offsets[0].0),
        );
    }
}

#[test]
fn independent_offsets_preserve_first_match_and_root_exclusion() {
    let mut doc = Document::new();
    let root = doc.append_element(Some(0), "div", None::<&str>);
    let child = doc.append_element(Some(root), "span", None::<&str>);
    let text = doc.append_text(child, "x");
    let offsets = [
        (root, (100.0, 100.0)),
        (child, (2.0, 3.0)),
        (child, (7.0, 9.0)),
    ];
    assert_eq!(cumulative_offset(&doc, root, &offsets, text), (2.0, 3.0));
    assert_eq!(
        cumulative_offset(&doc, root, &offsets[2..], text),
        (7.0, 9.0)
    );
    assert_eq!(
        cumulative_offset(&doc, usize::MAX, &offsets, text),
        (102.0, 103.0)
    );
    assert_eq!(cumulative_offset(&doc, root, &[], text), (0.0, 0.0));
}

#[test]
fn anonymous_table_cell_offsets_use_the_paragraph_index() {
    let fixture = block_fixture("", |doc, root| {
        let table = doc.append_element(Some(root), "table", Some("display:table"));
        let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
        for _ in 0..32 {
            let span = doc.append_element(Some(row), "span", Some("position:relative;left:2px"));
            doc.append_text(span, "x ");
        }
    });
    let (mut doc, cascade) = (fixture.doc, fixture.cascade);
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    let root = doc
        .nodes
        .layout_keys()
        .find(|&id| {
            id >= doc.node_count()
                && doc
                    .ifc_layout_node(id)
                    .is_some_and(|node| !node.ifc_relative_offsets().is_empty())
        })
        .expect("anonymous cell with relative inline content");
    let view = PositionedLines::new(&doc, &cascade, root, None).unwrap();
    PROBES.set(0);
    let runs: Vec<_> = view.lines().flat_map(|line| line.runs).collect();
    assert_eq!(runs.len(), 32);
    assert!(runs.iter().all(|run| run.offset == (2.0, 0.0)));
    // The anonymous cell is not a DOM ancestor, so the existing walk also
    // visits the row, table and document ancestors. Each visit is one probe.
    assert!(PROBES.get() <= 8 * runs.len(), "{} probes", PROBES.get());
}
