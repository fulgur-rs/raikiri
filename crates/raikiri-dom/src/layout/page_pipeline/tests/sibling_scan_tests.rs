use super::*;
use crate::layout::ifc::test_support::block_fixture;
use crate::layout::test_support::page_box_800x600;

fn reset() {
    FLOAT_SUBTREE_WALKS.with(|count| count.set(0));
    FLOAT_PREFILTER_VISITS.with(|count| count.set(0));
}

fn counts() -> (usize, usize) {
    (
        FLOAT_SUBTREE_WALKS.with(|count| count.get()),
        FLOAT_PREFILTER_VISITS.with(|count| count.get()),
    )
}

/// `count` blocks under one section, each 60px tall and avoiding a break
/// inside, on 100px pages, so that every one after the first moves to the
/// next page. `floated` gives the blocks that hold a float. Returns the top of
/// each block below the section.
fn paginated_blocks(count: usize, floated: &[usize]) -> Vec<f32> {
    paginated_ys(
        |doc, body| {
            let section = doc.append_element(Some(body), "section", Some("display:block"));
            (0..count)
                .map(|index| {
                    let block = doc.append_element(
                        Some(section),
                        "div",
                        Some("display:block;height:60px;break-inside:avoid"),
                    );
                    if floated.contains(&index) {
                        doc.append_element(
                            Some(block),
                            "span",
                            Some("display:block;float:left;width:10px;height:10px"),
                        );
                    }
                    doc.append_text(block, "x");
                    block
                })
                .collect()
        },
        &[],
    )
}

#[test]
fn normal_block_sibling_updates_do_not_grow_quadratically() {
    let mut samples = Vec::new();
    for count in [16, 32, 64] {
        NORMAL_SIBLING_WORK.with(|work| work.set([0; 3]));
        let ys = paginated_blocks(count, &[]);
        let work = NORMAL_SIBLING_WORK.with(|work| work.get());
        assert_eq!(ys, (0..count).map(|i| i as f32 * 100.0).collect::<Vec<_>>());
        eprintln!("normal_sibling_work count={count} work={work:?}");
        samples.push((count, work));
    }
    for pair in samples.windows(2) {
        for kind in 0..3 {
            assert!(
                pair[1].1[kind] < pair[0].1[kind] * 3,
                "work kind {kind}: {pair:?}"
            );
        }
    }
}

#[test]
fn blocks_without_floats_never_start_the_float_walk_when_one_after_another_moves() {
    for count in [16, 32, 64] {
        reset();
        let ys = paginated_blocks(count, &[]);
        let (walks, visits) = counts();
        // Each block starts its own 100px page.
        let expected: Vec<f32> = (0..count).map(|index| index as f32 * 100.0).collect();
        assert_eq!(ys, expected, "{count}");
        assert_eq!(walks, 0, "{count}: {visits} prefilter visits");
        // The ordinary-parent summary avoids starting even a prefilter walk.
        assert_eq!(visits, 0, "{count}");
    }
}

#[test]
fn only_the_boxes_around_a_float_start_the_float_walk() {
    let count = 32;
    reset();
    let ys = paginated_blocks(count, &[5]);
    let (walks, _) = counts();
    let expected: Vec<f32> = (0..count).map(|index| index as f32 * 100.0).collect();
    assert_eq!(ys, expected);
    // The walk starts for the block with the float and the section around
    // it, and for no other block.
    assert!(walks > 0, "the float is found");
    assert!(walks <= 4 * count, "{walks} walks for {count} blocks");
}

/// Whether the walk of the subtree of `branch` reaches a left or right float,
/// by the rules of the walk written out plainly.
fn reachable_float_below(
    doc: &crate::layout::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    branch: usize,
) -> bool {
    doc.nodes[branch].children.iter().any(|&child| {
        let node = &doc.nodes[child];
        if node.kind() != NodeKind::Element
            || node.display() == DisplayValue::None
            || matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            return false;
        }
        if matches!(
            cascade.computed[child].float,
            FloatValue::Left | FloatValue::Right
        ) {
            return true;
        }
        if node.display() != DisplayValue::Contents && holds_floats(doc, cascade, child) {
            return false;
        }
        reachable_float_below(doc, cascade, child)
    })
}

#[test]
fn the_float_prefilter_stops_where_the_walk_does() {
    // Boxes in which the prefilter finds a float, and boxes in which every
    // float is held by a box in between or skipped by the walk.
    let (mut open, mut closed) = (Vec::new(), Vec::new());
    let float = "display:block;float:left";
    let mut fixture = block_fixture("", |doc, root| {
        let plain = doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_text(plain, "x");
        closed.push(plain);
        let nested = doc.append_element(Some(root), "div", Some("display:block"));
        let inner = doc.append_element(Some(nested), "div", Some("display:block"));
        let deep = doc.append_element(Some(inner), "div", Some("display:block"));
        doc.append_element(Some(deep), "span", Some("display:block;float:right"));
        open.push(nested);
        let wrapper = doc.append_element(Some(root), "div", Some("display:contents"));
        let beside = doc.append_element(Some(wrapper), "div", Some("display:block"));
        doc.append_element(Some(beside), "span", Some(float));
        open.push(wrapper);
        // Each of these holds, or leaves out, the float inside it.
        for css in [
            "display:flow-root",
            "display:block;overflow:hidden",
            "display:flex",
            "display:grid",
            "display:inline-block",
            "display:block;contain:paint",
            "display:block;column-count:2",
            "display:none",
            "position:absolute",
        ] {
            let outer = doc.append_element(Some(root), "div", Some("display:block"));
            let holder = doc.append_element(Some(outer), "div", Some(css));
            let child = doc.append_element(Some(holder), "div", Some("display:block"));
            doc.append_element(Some(child), "span", Some(float));
            closed.push(outer);
        }
        let lifted = doc.append_element(Some(root), "div", Some("display:contents"));
        let held = doc.append_element(Some(lifted), "div", Some("display:flow-root"));
        doc.append_element(Some(held), "span", Some(float));
        closed.push(lifted);
        let sideways = doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_element(Some(sideways), "span", Some("float:inline-start"));
        doc.append_element(Some(sideways), "span", Some("float:none"));
        doc.append_element(Some(root), "div", Some("display:block"));
    });
    let nodes: Vec<usize> = fixture.doc.preorder(0).collect();
    let session = paginating(&mut fixture.doc, &fixture.cascade);
    let (mut seen_float, mut seen_none) = (false, false);
    for node in nodes {
        let reached = reachable_float_below(&session, &fixture.cascade, node);
        let found = has_float_below(&session, &fixture.cascade, node);
        // The walk never reaches a float that the prefilter misses.
        assert!(found || !reached, "{node}");
        seen_float |= reached;
        seen_none |= !found;
    }
    assert!(seen_float && seen_none);
    for &node in &open {
        assert!(has_float_below(&session, &fixture.cascade, node), "{node}");
    }
    for &node in &closed {
        assert!(!has_float_below(&session, &fixture.cascade, node), "{node}");
    }
}

#[test]
fn a_branch_that_holds_its_floats_is_rejected_before_its_subtree_is_read() {
    let mut ids = Vec::new();
    let mut fixture = block_fixture("", |doc, root| {
        for css in [
            "display:flow-root",
            "display:block;overflow:hidden",
            "display:block",
        ] {
            let branch = doc.append_element(Some(root), "div", Some(css));
            let mut parent = branch;
            for _ in 0..8 {
                parent = doc.append_element(Some(parent), "div", Some("display:block"));
                doc.append_element(Some(parent), "span", Some("display:block;float:left"));
            }
            ids.push(branch);
        }
    });
    let body = fixture.doc.parent_of(fixture.root).unwrap();
    let session = paginating(&mut fixture.doc, &fixture.cascade);
    let ends: Vec<(f32, usize)> = ids
        .iter()
        .map(|&branch| {
            reset();
            let end = same_bfc_float_end_in_subtree(&session, &fixture.cascade, body, branch, 0.0);
            (end, counts().1)
        })
        .collect();
    // The boxes that hold their floats are not read, and the end of their
    // floats is not part of the answer.
    assert_eq!(ends[0], (f32::NEG_INFINITY, 0));
    assert_eq!(ends[1], (f32::NEG_INFINITY, 0));
    // A plain block gives the end of the floats inside it.
    assert!(ends[2].0 > f32::NEG_INFINITY && ends[2].1 > 0, "{ends:?}");
}

/// The prefilter reads the boxes of `blocks` blocks that avoid a break inside
/// and hold their floats by `css`, each with `inside` boxes in it, while they
/// move to the pages after one another.
fn prefilter_reads_with(css: &str, blocks: usize, inside: usize) -> usize {
    reset();
    paginated_ys(
        |doc, body| {
            let section = doc.append_element(Some(body), "section", Some("display:block"));
            (0..blocks)
                .map(|_| {
                    let block = doc.append_element(Some(section), "div", Some(css));
                    for _ in 0..inside {
                        doc.append_element(Some(block), "div", Some("display:block;height:1px"));
                    }
                    block
                })
                .collect()
        },
        &[],
    );
    counts().1
}

#[test]
fn the_boxes_inside_a_block_that_holds_its_floats_are_not_read_as_blocks_move() {
    for css in [
        "display:flow-root;height:60px;break-inside:avoid",
        "display:block;overflow:hidden;height:60px;break-inside:avoid",
    ] {
        let bare = prefilter_reads_with(css, 12, 0);
        let full = prefilter_reads_with(css, 12, 40);
        assert_eq!(bare, 0, "{css}");
        assert_eq!(bare, full, "{css}");
    }
}

/// The boxes after `child` in `parent`, moved as the walk that collects all
/// of `parent`'s boxes moves them.
fn shift_by_collecting_all_boxes(doc: &mut Document, parent: usize, child: usize, delta: f32) {
    if !moves_later_siblings(doc, parent) || delta == 0.0 || !delta.is_finite() {
        return;
    }
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = doc.nodes[parent].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if doc.nodes[id].display() == DisplayValue::Contents {
            stack.extend(doc.nodes[id].children.iter().rev().copied());
        } else {
            boxes.push(id);
        }
    }
    let Some(index) = boxes.iter().position(|&id| id == child) else {
        return;
    };
    for &sibling in &boxes[index + 1..] {
        if doc.nodes[sibling].kind() != NodeKind::Element
            || doc.nodes[sibling].display() == DisplayValue::None
            || matches!(
                doc.nodes[sibling].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
        {
            continue;
        }
        doc.table_layout_node_mut(sibling).layout.location.y += delta;
    }
}

/// A laid-out block of children that includes `display: contents` wrappers,
/// one inside another, boxes that stay where they are and text. The ids of
/// the container and of every child, in document order.
fn block_with_wrapped_children(text_between: bool) -> (Document, usize, Vec<usize>) {
    let mut ids = Vec::new();
    let fixture = block_fixture("width:100px", |doc, root| {
        let mut add = |doc: &mut Document, parent: usize, css: &str| {
            let id = doc.append_element(Some(parent), "div", Some(css));
            ids.push(id);
            id
        };
        add(doc, root, "display:block;height:10px");
        let outer = add(doc, root, "display:contents");
        add(doc, outer, "display:block;height:10px");
        let inner = add(doc, outer, "display:contents");
        add(doc, inner, "display:block;height:10px");
        add(doc, root, "display:block;height:10px;position:absolute");
        add(doc, root, "display:none");
        add(doc, root, "display:block;height:10px;position:relative");
        let late = add(doc, root, "display:contents");
        add(doc, late, "display:block;height:10px");
        if text_between {
            doc.append_text(root, "x");
        }
        add(doc, root, "display:block;height:10px");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).unwrap();
    (doc, fixture.root, ids)
}

#[test]
fn later_siblings_move_as_when_all_the_boxes_of_the_parent_are_collected() {
    for text_between in [false, true] {
        let (probe, root, ids) = block_with_wrapped_children(text_between);
        let mut tested = ids.clone();
        // A box that is not a child of the container at all.
        tested.push(root);
        let mut moved_any = false;
        for &child in &tested {
            for delta in [7.0, 0.0, f32::NAN] {
                let (mut fast, _, _) = block_with_wrapped_children(text_between);
                let (mut slow, _, _) = block_with_wrapped_children(text_between);
                shift_later_block_siblings(&mut fast, root, child, delta);
                shift_by_collecting_all_boxes(&mut slow, root, child, delta);
                for id in probe.preorder(0) {
                    let (a, b) = (
                        fast.nodes[id].layout.location.y,
                        slow.nodes[id].layout.location.y,
                    );
                    assert_eq!(
                        a.to_bits(),
                        b.to_bits(),
                        "child {child}, delta {delta}, node {id}"
                    );
                    moved_any |= a.to_bits() != probe.nodes[id].layout.location.y.to_bits();
                }
            }
        }
        // The lines of a paragraph follow a moved block child through their
        // own shifts, so none of the boxes around it moves.
        assert_eq!(moved_any, !text_between, "some case moves a box");
    }
}

#[test]
fn the_boxes_after_a_box_inside_a_contents_wrapper_follow_it() {
    let (mut doc, root, ids) = block_with_wrapped_children(false);
    // `ids`: first, outer wrapper, its block, inner wrapper, its block,
    // positioned box, hidden box, relative box, late wrapper, its block, last.
    let (wrapped, last) = (ids[2], ids[10]);
    let (wrapped_y, last_y) = (
        doc.nodes[wrapped].layout.location.y,
        doc.nodes[last].layout.location.y,
    );
    let inner_block_y = doc.nodes[ids[4]].layout.location.y;
    let absolute_y = doc.nodes[ids[5]].layout.location.y;
    shift_later_block_siblings(&mut doc, root, wrapped, 5.0);
    // The block itself stays, and the ones after it in the flow move,
    // through the wrapper that holds them and past the positioned box.
    assert_eq!(doc.nodes[wrapped].layout.location.y, wrapped_y);
    assert_eq!(doc.nodes[ids[4]].layout.location.y, inner_block_y + 5.0);
    assert_eq!(doc.nodes[ids[5]].layout.location.y, absolute_y);
    assert_eq!(doc.nodes[last].layout.location.y, last_y + 5.0);
}

#[test]
fn blocks_around_a_contents_wrapper_each_start_their_own_page() {
    let block = "display:block;height:60px;break-inside:avoid";
    // A block that moves leaves the boxes after it in the flow where they
    // follow it, whether they sit in the section or in a `display: contents`
    // wrapper in it.
    let ys = paginated_ys(
        |doc, body| {
            let section = doc.append_element(Some(body), "section", Some("display:block"));
            let first = doc.append_element(Some(section), "div", Some(block));
            let second = doc.append_element(Some(section), "div", Some(block));
            let wrapper = doc.append_element(Some(section), "div", Some("display:contents"));
            let third = doc.append_element(Some(wrapper), "div", Some(block));
            let fourth = doc.append_element(Some(wrapper), "div", Some(block));
            let fifth = doc.append_element(Some(section), "div", Some(block));
            vec![first, second, third, fourth, fifth]
        },
        &[],
    );
    assert_eq!(ys, [0.0, 100.0, 200.0, 300.0, 400.0]);
}

#[test]
fn a_first_middle_or_last_float_keeps_normal_sibling_work_below_quadratic() {
    for position in 0..3 {
        let mut samples = Vec::new();
        for count in [16, 32, 64] {
            let float = match position {
                0 => 0,
                1 => count / 2,
                _ => count - 1,
            };
            reset();
            NORMAL_SIBLING_WORK.with(|work| work.set([0; 3]));
            let ys = paginated_blocks(count, &[float]);
            assert_eq!(
                ys,
                (0..count)
                    .map(|index| index as f32 * 100.0)
                    .collect::<Vec<_>>()
            );
            let normal = NORMAL_SIBLING_WORK.with(std::cell::Cell::get);
            let (walks, visits) = counts();
            let work = [normal[0], normal[1], normal[2], walks, visits];
            eprintln!("normal_sibling_float_work count={count} position={position} work={work:?}");
            samples.push(work);
        }
        for pair in samples.windows(2) {
            for (kind, (&before, &after)) in pair[0].iter().zip(&pair[1]).enumerate() {
                assert!(
                    after <= before * 3,
                    "position={position} kind={kind} samples={samples:?}"
                );
            }
        }
    }
}

#[test]
fn ordinary_documents_up_to_the_default_page_limit_keep_deferred_updates() {
    for count in [1024, 4096, 10_000] {
        super::super::normal_block_shifts::FALLBACKS.with(|count| count.set(0));
        NORMAL_SIBLING_WORK.with(|work| work.set([0; 3]));
        let ys = paginated_blocks(count, &[]);
        assert_eq!(
            ys,
            (0..count)
                .map(|index| index as f32 * 100.0)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            super::super::normal_block_shifts::FALLBACKS.with(std::cell::Cell::get),
            0,
            "count={count}"
        );
        let work = NORMAL_SIBLING_WORK.with(std::cell::Cell::get);
        assert!(
            work[0] <= count + 2 && work[1] <= count && work[2] <= count * 5,
            "count={count} work={work:?}"
        );
        eprintln!("normal_long_document count={count} work={work:?} fallback=0");
    }
}
