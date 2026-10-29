use super::*;

#[test]
#[ignore = "manual release-mode positioned-frame scaling measurement"]
fn positioned_order_scaling_probe() {
    use std::time::Instant;
    for n in [1_000, 2_000, 4_000, 8_000] {
        let mut document = Document::new();
        let nodes: Vec<_> = (0..n)
            .map(|_| document.append_element(Some(0), "div", None::<&str>))
            .collect();
        for legacy in [true, false] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let mut frames = Vec::with_capacity(n);
                let start = Instant::now();
                let mut order = DocumentOrder::new(&document);
                for &id in &nodes {
                    if legacy {
                        legacy_insert(&document, &mut frames, PositionedFrame::Box(id), |f| *f);
                    } else {
                        insert_positioned_frame(
                            &mut order,
                            &mut frames,
                            PositionedFrame::Box(id),
                            |f| *f,
                        );
                    }
                }
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                assert_eq!(frames.len(), n);
                std::hint::black_box(&frames);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "positioned ordered legacy={legacy} n={n} median_ms={:.6} samples={samples:?}",
                samples[2]
            );
        }
    }
}

#[test]
fn document_order_indexes_only_queried_components_once() {
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "div", None::<&str>);
    let b = doc.append_element(Some(0), "div", None::<&str>);
    let detached = doc.append_element(None, "section", None::<&str>);
    let child = doc.append_element(Some(detached), "div", None::<&str>);
    for _ in 0..100_000 {
        doc.append_element(None, "span", None::<&str>);
    }
    let mut order = DocumentOrder::new(&doc);
    assert!(order.entries.is_empty());
    assert!(!order.precedes(a, a));
    assert!(!order.precedes(a, usize::MAX));
    assert!(order.entries.is_empty());
    for _ in 0..100 {
        assert!(order.precedes(a, b));
    }
    assert_eq!(order.entries.len(), 3);
    assert_eq!(order.index_work, 5);
    for _ in 0..100 {
        assert!(order.precedes(detached, child));
        assert!(!order.precedes(a, child));
        assert!(order.precedes(a, b));
    }
    assert_eq!(order.entries.len(), 5);
    assert_eq!(order.index_work, 8);
}

fn legacy_precedes(document: &Document, a: usize, b: usize) -> bool {
    if a == b || document.get_node(a).is_none() || document.get_node(b).is_none() {
        return false;
    }
    let chain = |id: usize| {
        let mut chain: Vec<usize> = document.inclusive_ancestors(id).collect();
        chain.reverse();
        chain
    };
    let (a_chain, b_chain) = (chain(a), chain(b));
    let shared = a_chain
        .iter()
        .zip(&b_chain)
        .take_while(|(x, y)| x == y)
        .count();
    match (a_chain.get(shared), b_chain.get(shared)) {
        // `a` is an ancestor of `b`.
        (None, Some(_)) => true,
        (Some(&a_branch), Some(&b_branch)) if shared > 0 => document
            .get_node(a_chain[shared - 1])
            .and_then(|parent| {
                let at = |id| parent.children.iter().position(|&child| child == id);
                Some(at(a_branch)? < at(b_branch)?)
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn legacy_insert<F>(
    document: &Document,
    frames: &mut Vec<F>,
    frame: F,
    kind: impl Fn(&F) -> PositionedFrame,
) {
    let PositionedFrame::Box(id) = kind(&frame) else {
        frames.push(frame);
        return;
    };
    let mut at = frames.len();
    while at > 0 {
        let first = match kind(&frames[at - 1]) {
            PositionedFrame::Box(other) => Some((at - 1, other)),
            PositionedFrame::Close => {
                // The clip around the frames back to the matching open.
                let mut depth = 0;
                (0..at).rev().find_map(|index| match kind(&frames[index]) {
                    PositionedFrame::Close => {
                        depth += 1;
                        None
                    }
                    PositionedFrame::Open(owner) => {
                        depth -= 1;
                        (depth == 0).then_some((index, owner))
                    }
                    PositionedFrame::Box(_) => None,
                })
            }
            PositionedFrame::Open(_) => None,
        };
        match first {
            Some((index, other))
                if legacy_precedes(document, id, other)
                    && !document.is_ancestor_or_self(other, id) =>
            {
                at = index;
            }
            _ => break,
        }
    }
    frames.insert(at, frame);
}

fn assert_matches_legacy(document: &Document) {
    let mut order = DocumentOrder::new(document);
    for a in 0..document.node_count() {
        for b in 0..document.node_count() {
            assert_eq!(
                order.precedes(a, b),
                legacy_precedes(document, a, b),
                "{a} before {b}"
            );
            assert_eq!(
                order.is_ancestor_or_self(a, b),
                document.is_ancestor_or_self(a, b),
                "{a} contains {b}"
            );
        }
    }
}

#[test]
fn source_forest_order_matches_legacy_after_moves_and_template_creation() {
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "section", None::<&str>);
    let b = doc.append_element(Some(0), "section", None::<&str>);
    let mut nodes = vec![a, b];
    for i in 0..48 {
        let parent = nodes[(i * 7 + 1) % nodes.len()];
        nodes.push(doc.append_element(Some(parent), "div", None::<&str>));
    }
    let detached = doc.append_element(None, "aside", None::<&str>);
    doc.append_element(Some(detached), "span", None::<&str>);
    let template = doc.append_element(Some(b), "template", None::<&str>);
    let fragment = doc.allocate_template_fragment_root(template);
    doc.append_element(Some(fragment), "div", None::<&str>);
    doc.append_element(Some(fragment), "div", None::<&str>);
    assert_matches_legacy(&doc);
    doc.detach_from_parent(a);
    doc.attach_child(0, a);
    assert_matches_legacy(&doc);
}

#[test]
fn real_anonymous_table_boxes_do_not_enter_source_order() {
    let mut doc = Document::new();
    let table = doc.append_element(Some(0), "div", Some("display:table"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let block = doc.append_element(Some(row), "div", Some("display:block"));
    doc.mark_in_document_flags();
    let computed = cascaded(&doc);
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let generated = doc.anonymous_cell_of(block).unwrap();
    let icb = doc.set_initial_containing_block(crate::Node::new_element("".into(), None));
    assert!(doc.get_node(generated).is_none());
    assert!(doc.parent_of(generated).is_some());
    assert_eq!(doc.parent_of(block), Some(row));
    for id in 0..doc.node_count() {
        assert!(
            doc.parent_of(id)
                .is_none_or(|parent| doc.get_node(parent).is_some())
        );
    }
    assert_matches_legacy(&doc);
    let mut order = DocumentOrder::new(&doc);
    for generated in [generated, icb, usize::MAX - 1] {
        assert!(!order.precedes(generated, block));
        assert!(!order.precedes(block, generated));
        assert!(!legacy_precedes(&doc, generated, block));
        assert!(!legacy_precedes(&doc, block, generated));
    }
    assert!(order.entries.is_empty());
}

#[test]
fn ordered_registration_indexes_nodes_and_edges_once() {
    for n in [32, 128, 512] {
        let mut doc = Document::new();
        let ids: Vec<_> = (0..n)
            .map(|_| doc.append_element(Some(0), "div", None::<&str>))
            .collect();
        let mut order = DocumentOrder::new(&doc);
        let mut frames = Vec::new();
        let classifications = std::cell::Cell::new(0);
        for &id in &ids {
            insert_positioned_frame(&mut order, &mut frames, PositionedFrame::Box(id), |f| {
                classifications.set(classifications.get() + 1);
                *f
            });
        }
        assert_eq!(
            frames,
            ids.into_iter()
                .map(PositionedFrame::Box)
                .collect::<Vec<_>>()
        );
        assert_eq!(order.entries.len(), n + 1);
        assert_eq!(order.index_work, 2 * n + 1);
        assert_eq!(classifications.get(), 2 * n - 1);
    }
}

#[test]
fn deep_source_order_uses_an_explicit_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut doc = Document::new();
            let mut last = 0;
            for _ in 0..20_000 {
                last = doc.append_element(Some(last), "div", None::<&str>);
            }
            let mut order = DocumentOrder::new(&doc);
            assert!(!order.precedes(last, 0));
            assert!(order.precedes(0, last));
            assert!(order.is_ancestor_or_self(0, last));
            assert!(!order.is_ancestor_or_self(last, 0));
            for _ in 0..100 {
                assert!(order.precedes(last - 1, last));
            }
            assert_eq!(order.index_work, 40_001);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn late_insertions_preserve_legacy_permutations_and_nested_clips() {
    use PositionedFrame::{Box, Close, Open};
    let mut doc = Document::new();
    let a = doc.append_element(Some(0), "div", None::<&str>);
    let a1 = doc.append_element(Some(a), "div", None::<&str>);
    let a2 = doc.append_element(Some(a), "div", None::<&str>);
    let a21 = doc.append_element(Some(a2), "div", None::<&str>);
    let b = doc.append_element(Some(0), "div", None::<&str>);
    let c = doc.append_element(Some(0), "div", None::<&str>);
    let mut nodes = [a, a1, a2, a21, b, c];
    let mut seeds = vec![
        vec![Open(a), Box(a1), Open(a2), Box(a21), Close, Close, Box(c)],
        vec![Open(a), Box(a1), Close, Open(b), Close],
        vec![Box(c), Open(b)],
        vec![],
    ];
    fn permutations(nodes: &mut [usize], at: usize, out: &mut Vec<Vec<PositionedFrame>>) {
        if at == nodes.len() {
            out.push(nodes.iter().copied().map(PositionedFrame::Box).collect());
        } else {
            for i in at..nodes.len() {
                nodes.swap(at, i);
                permutations(nodes, at + 1, out);
                nodes.swap(at, i);
            }
        }
    }
    permutations(&mut nodes, 0, &mut seeds);
    let mut order = DocumentOrder::new(&doc);
    for seed in seeds {
        for next in nodes
            .into_iter()
            .map(Box)
            .chain([Open(a), Close, Box(usize::MAX)])
        {
            let mut actual = seed.clone();
            let mut expected = seed.clone();
            insert_positioned_frame(&mut order, &mut actual, next, |f| *f);
            legacy_insert(&doc, &mut expected, next, |f| *f);
            assert_eq!(actual, expected, "seed={seed:?}, next={next:?}");
        }
    }
}
