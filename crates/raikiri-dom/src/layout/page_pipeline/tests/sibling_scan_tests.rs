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
fn blocks_without_floats_never_start_the_float_walk_when_one_after_another_moves() {
    for count in [16, 32, 64] {
        reset();
        let ys = paginated_blocks(count, &[]);
        let (walks, visits) = counts();
        // Each block starts its own 100px page.
        let expected: Vec<f32> = (0..count).map(|index| index as f32 * 100.0).collect();
        assert_eq!(ys, expected, "{count}");
        assert_eq!(walks, 0, "{count}: {visits} prefilter visits");
        // The prefilter only reads the boxes: the moves of the blocks read
        // each block, and the section, once per move.
        assert!(
            visits > 0 && visits <= 8 * count * count,
            "{count}: {visits}"
        );
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

/// Whether an element below `branch` is a left or right float, found by
/// visiting the whole subtree.
fn float_below_by_walk(doc: &Document, cascade: &CascadeResult, branch: usize) -> bool {
    doc.preorder(branch).skip(1).any(|id| {
        doc.nodes[id].kind() == NodeKind::Element
            && matches!(
                cascade.computed[id].float,
                FloatValue::Left | FloatValue::Right
            )
    })
}

#[test]
fn the_float_prefilter_agrees_with_a_walk_of_every_subtree() {
    let fixture = block_fixture("", |doc, root| {
        let plain = doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_text(plain, "x");
        let nested = doc.append_element(Some(root), "div", Some("display:block"));
        let inner = doc.append_element(Some(nested), "div", Some("display:block"));
        let deep = doc.append_element(Some(inner), "div", Some("display:block"));
        doc.append_element(Some(deep), "span", Some("display:block;float:right"));
        let wrapper = doc.append_element(Some(root), "div", Some("display:contents"));
        let beside = doc.append_element(Some(wrapper), "div", Some("display:block"));
        doc.append_element(Some(beside), "span", Some("display:block;float:left"));
        let sideways = doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_element(
            Some(sideways),
            "span",
            Some("display:block;float:inline-start"),
        );
        doc.append_element(Some(sideways), "span", Some("display:block;float:none"));
        let empty = doc.append_element(Some(root), "div", Some("display:block"));
        let _ = empty;
    });
    let nodes: Vec<usize> = fixture.doc.preorder(0).collect();
    let (mut seen_float, mut seen_none) = (false, false);
    for node in nodes {
        let expected = float_below_by_walk(&fixture.doc, &fixture.cascade, node);
        assert_eq!(
            has_float_below(&fixture.doc, &fixture.cascade, node),
            expected,
            "{node}"
        );
        seen_float |= expected;
        seen_none |= !expected;
    }
    assert!(seen_float && seen_none);
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
