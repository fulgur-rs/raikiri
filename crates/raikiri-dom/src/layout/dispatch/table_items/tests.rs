use super::*;
use crate::Document;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, OofCandidate, OofCandidates, OofPosition,
};
use crate::layout::taffy_adapter::algorithms::compute_flexbox_layout;
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Line;

fn fixture() -> (Document, usize, usize, usize) {
    fixture_with_alignment("")
}

fn fixture_with_alignment(alignment: &str) -> (Document, usize, usize, usize) {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some(&format!("display:flex;flex-wrap:wrap-reverse;width:100px;height:100px;padding:calc(1% + 1px);{alignment}")),
    );
    let table = doc.append_element(
        Some(root),
        "table",
        Some("display:table;border-spacing:0;width:20px"),
    );
    for (height, visibility) in [(40, "collapse"), (20, "visible")] {
        let row = doc.append_element(
            Some(table),
            "tr",
            Some(&format!("display:table-row;visibility:{visibility}")),
        );
        doc.append_element(
            Some(row),
            "td",
            Some(&format!("display:table-cell;height:{height}px;padding:0")),
        );
    }
    let absolute = doc.append_element(
        Some(table),
        "div",
        Some("display:block;position:absolute;width:3px;height:3px"),
    );
    doc.mark_in_document_flags();
    (doc, root, table, absolute)
}

#[test]
fn unstretched_alignment_does_not_acquire_a_reversed_stretch_offset() {
    let (mut doc, root, table, _) = fixture_with_alignment("align-items:center");
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (mut capture, mut output) = capture(&mut tree, root);
    let before = tree.nodes[table].layout;
    // A stale offered size must not turn centered alignment into stretch.
    capture.sizes.get_mut(&table).unwrap().offered.height = 100.0;
    normalize_flex(
        &mut tree,
        root,
        inputs(),
        capture.sizes,
        capture.calls.as_deref(),
        &mut output,
    );
    assert_eq!(tree.nodes[table].layout, before);
}

#[test]
fn a_detached_table_does_not_borrow_a_parents_height() {
    let (mut doc, root, table, _) = fixture();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    capture(&mut tree, root);
    assert!(crate::layout::table::item_owns_height(&tree, table));
    tree.detach_from_parent(table);
    assert_eq!(
        tree.nodes[table].formatting_context,
        FormattingContext::Table
    );
    assert_eq!(tree.taffy_parent_of(table), None);
    assert!(!crate::layout::table::item_owns_height(&tree, table));
}

#[test]
fn scroll_reach_keeps_the_end_padding_in_each_direction() {
    for (direction, overflow, right, bottom) in [
        ("ltr", "visible", 31.0, 43.0),
        ("ltr", "auto", 36.0, 50.0),
        ("rtl", "auto", 100.0, 50.0),
    ] {
        let (mut doc, root, table, _) = fixture_with_alignment(&format!(
            "direction:{direction};overflow:{overflow};padding:3px 5px 7px 11px"
        ));
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.nodes[table].layout.location = Point { x: 11.0, y: 13.0 };
        tree.nodes[table].layout.size = Size {
            width: 20.0,
            height: 30.0,
        };
        let mut output = LayoutOutput::HIDDEN;
        output.size = Size {
            width: 100.0,
            height: 100.0,
        };
        finish(
            &mut tree,
            root,
            inputs(),
            &[table],
            &[(table, Point::ZERO)],
            &mut output,
        );
        assert_eq!(
            output.scrollable_overflow_rect,
            Edges {
                left: 0.0,
                top: 0.0,
                right,
                bottom
            }
        );
    }
}

fn inputs() -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        known_dimensions: Size {
            width: Some(100.0),
            height: Some(100.0),
        },
        parent_size: Size {
            width: Some(800.0),
            height: Some(600.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(100.0),
            height: AvailableSpace::Definite(100.0),
        },
        ..LayoutInput::HIDDEN
    }
}

fn capture(tree: &mut LayoutSession<'_>, root: usize) -> (Capture, LayoutOutput) {
    tree.context.table_item_captures.push(Capture {
        container: root,
        sizes: HashMap::new(),
        calls: Some(Vec::new()),
    });
    let output = compute_flexbox_layout(tree, NodeId::from(root), inputs());
    (tree.context.table_item_captures.pop().unwrap(), output)
}

#[test]
fn replay_rejects_corrupt_recordings_without_writing_layouts() {
    let (mut doc, root, table, _) = fixture();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (capture, _) = capture(&mut tree, root);
    let calls = capture.calls.unwrap();
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    assert_eq!(
        flex_baseline_source(&tree, root.into(), inputs(), &calls, &[table]),
        Ok(Some(table))
    );
    assert_eq!(
        before,
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>()
    );
    for extra in [false, true] {
        let mut damaged = calls.clone();
        if extra {
            damaged.push(calls[0].clone());
        } else {
            damaged.remove(0);
        }
        assert_eq!(
            flex_baseline_source(&tree, root.into(), inputs(), &damaged, &[table]),
            Err("flex baseline replay changed its callback schedule")
        );
    }
    for baseline in [f32::MAX, f32::INFINITY] {
        let mut damaged = calls.clone();
        let last = damaged
            .iter_mut()
            .rfind(|call| {
                usize::from(call.node) == table && call.inputs.run_mode == RunMode::PerformLayout
            })
            .unwrap();
        last.output.baselines.first = Some(baseline);
        assert_eq!(
            flex_baseline_source(&tree, root.into(), inputs(), &damaged, &[table]),
            Err("flex baseline replay exceeded its exact identity range")
        );
        assert_eq!(
            before,
            tree.nodes
                .iter()
                .map(|node| node.layout)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn replay_errors_reach_the_layout_error_channel() {
    let (mut doc, root, _, _) = fixture();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (capture, mut output) = capture(&mut tree, root);
    normalize_flex(
        &mut tree,
        root,
        inputs(),
        capture.sizes,
        Some(&[]),
        &mut output,
    );
    assert_eq!(
        tree.context.table_layout_error.as_deref(),
        Some("flex baseline replay changed its callback schedule")
    );
}

#[test]
fn hoisted_static_positions_follow_only_their_moved_ancestor() {
    let (mut doc, root, table, absolute) = fixture();
    let outside = doc.append_element(Some(root), "div", Some("position:absolute"));
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (_, mut output) = capture(&mut tree, root);
    output.oof_candidates = Default::default();
    for node in [absolute, outside] {
        output.oof_candidates.push(OofCandidate {
            node: node.into(),
            order: 0,
            position: OofPosition::Absolute,
            static_position: Point {
                x: AxisStaticPosition::from_edge(10.0, AxisStaticEdge::Start),
                y: AxisStaticPosition::from_edge(20.0, AxisStaticEdge::Start),
            },
        });
    }
    finish(
        &mut tree,
        root,
        inputs(),
        &[table],
        &[(table, Point { x: 7.0, y: 13.0 })],
        &mut output,
    );
    let positions: Vec<_> = output
        .oof_candidates
        .iter()
        .map(|candidate| {
            (
                candidate.static_position.x.area,
                candidate.static_position.y.area,
            )
        })
        .collect();
    assert_eq!(
        positions,
        vec![
            (
                Line {
                    start: 17.0,
                    end: 17.0
                },
                Line {
                    start: 33.0,
                    end: 33.0
                }
            ),
            (
                Line {
                    start: 10.0,
                    end: 10.0
                },
                Line {
                    start: 20.0,
                    end: 20.0
                }
            )
        ]
    );
}

#[test]
fn reversed_alignment_replay_rejects_damaged_calls_and_snapshots() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;flex-wrap:wrap-reverse;width:100px;height:100px"),
    );
    let a = doc.append_element(
        Some(root),
        "div",
        Some("width:100px;height:150px;flex:none"),
    );
    let b = doc.append_element(Some(root), "div", Some("width:100px;height:20px;flex:none"));
    let hidden = doc.append_element(Some(root), "div", Some("display:none"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (capture, output) = capture(&mut tree, root);
    let calls = capture.calls.unwrap();
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    let check = |calls: &[FlexReplayCall],
                 input,
                 output: &LayoutOutput,
                 layouts: &[raikiri_layout_geometry::BoxLayout]| {
        wrap_reverse::candidate(&tree, root.into(), input, calls, output, |child| {
            layouts.get(child).copied()
        })
    };
    let corrected = check(&calls, inputs(), &output, &before).unwrap();
    assert_eq!(
        corrected
            .layouts
            .iter()
            .find(|&&(child, _)| child == a)
            .unwrap()
            .1
            .location
            .y,
        -50.0
    );
    for extra in [false, true] {
        let mut damaged = calls.clone();
        if extra {
            damaged.push(calls[0].clone());
        } else {
            damaged.remove(0);
        }
        assert!(check(&damaged, inputs(), &output, &before).is_none());
    }
    assert!(
        check(
            &calls,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                ..inputs()
            },
            &output,
            &before
        )
        .is_none()
    );
    assert!(
        check(
            &calls,
            inputs(),
            &LayoutOutput {
                size: Size::ZERO,
                ..output.clone()
            },
            &before
        )
        .is_none()
    );
    for mode in 0..7 {
        let mut damaged = before.clone();
        match mode {
            0 => damaged[a].margin.top += 1.0,
            1 => damaged[a].location.x += 1.0,
            2 => damaged[a].location.y = f32::INFINITY,
            3 => damaged[a].location.y = -1000.0,
            4 => damaged[b].location.y -= 1.0,
            5 => {
                damaged[b].location = corrected
                    .layouts
                    .iter()
                    .find(|&&(child, _)| child == b)
                    .unwrap()
                    .1
                    .location
            }
            _ => damaged[hidden].location.y = 1.0,
        }
        assert!(check(&calls, inputs(), &output, &damaged).is_none());
    }
    assert!(check(&calls, inputs(), &output, &[]).is_none());
    assert_eq!(
        before,
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>()
    );
}

#[test]
fn positive_cross_room_retains_stretch_and_used_auto_margins() {
    for child_style in ["height:20px;margin-top:auto", "", "height:20px"] {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Some("display:flex;flex-wrap:wrap-reverse;width:100px;height:100px"),
        );
        let child = doc.append_element(
            Some(root),
            "div",
            Some(&format!("width:100px;{child_style}")),
        );
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let (capture, output) = capture(&mut tree, root);
        let before = tree.nodes[child].layout;
        assert!(
            wrap_reverse::candidate(
                &tree,
                root.into(),
                inputs(),
                capture.calls.as_deref().unwrap(),
                &output,
                |child| Some(tree.nodes[child].layout)
            )
            .is_none()
        );
        if child_style.is_empty() {
            assert_eq!(before.size.height, 100.0);
        } else {
            assert_eq!(before.location.y, 80.0);
            if child_style.contains("margin") {
                assert_eq!(before.margin.top, 80.0);
            } else {
                assert_eq!(before.margin.top, 0.0);
                // A stale position alone must not masquerade as negative
                // free space when both positional replays still have room.
                assert!(
                    wrap_reverse::candidate(
                        &tree,
                        root.into(),
                        inputs(),
                        capture.calls.as_deref().unwrap(),
                        &output,
                        |child| {
                            let mut stale = tree.nodes[child].layout;
                            stale.location.y += 1.0;
                            Some(stale)
                        }
                    )
                    .is_none()
                );
            }
        }
    }
}

#[test]
fn fractional_wrap_baseline_can_change_after_position_rounding() {
    for height in [100.25, 99.75] {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Some("display:flex;flex-wrap:wrap-reverse;width:100px;height:100px"),
        );
        let child = doc.append_element(
            Some(root),
            "div",
            Some(&format!(
                "width:100px;height:{height}px;position:relative;top:10000000px"
            )),
        );
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let (capture, output) = capture(&mut tree, root);
        let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
        assert_eq!(before[child].location.y, 10_000_000.0);
        let corrected = wrap_reverse::candidate(
            &tree,
            root.into(),
            inputs(),
            capture.calls.as_deref().unwrap(),
            &output,
            |child| Some(before[child]),
        );
        if height > 100.0 {
            assert_eq!(output.baselines.first, Some(100.25));
            let corrected = corrected.expect("the baseline retains the fractional overflow shift");
            assert_eq!(corrected.output.baselines.first, Some(100.0));
            assert_eq!(corrected.layouts, vec![(child, before[child])]);
        } else {
            assert_eq!(output.baselines.first, Some(100.0));
            assert!(corrected.is_none());
        }
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| node.layout)
                .collect::<Vec<_>>(),
            before
        );
    }
}

/// A wrapping column container 200px wide and 20px high, whose items align by
/// a baseline: `a` and `b` fill the first line, and `c` and `d` the second. A
/// hidden item and an absolutely positioned one are in it too, and are in no
/// line.
fn wrapped_baselines(style: &str) -> (Document, usize, [usize; 4]) {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some(&format!(
            "display:flex;flex-direction:column;align-items:baseline;width:200px;height:20px;{style}"
        )),
    );
    let items = [(20, 10), (50, 10), (30, 10), (10, 10)].map(|(width, height)| {
        doc.append_element(
            Some(root),
            "div",
            Some(&format!("width:{width}px;height:{height}px")),
        )
    });
    doc.append_element(Some(root), "div", Some("display:none"));
    doc.append_element(
        Some(root),
        "div",
        Some("position:absolute;width:5px;height:5px"),
    );
    doc.mark_in_document_flags();
    (doc, root, items)
}

fn wrapped_inputs() -> LayoutInput {
    LayoutInput {
        known_dimensions: Size {
            width: Some(200.0),
            height: Some(20.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(200.0),
            height: AvailableSpace::Definite(20.0),
        },
        ..inputs()
    }
}

fn wrapped_capture(
    tree: &mut LayoutSession<'_>,
    root: usize,
) -> (Vec<FlexReplayCall>, LayoutOutput) {
    tree.context.table_item_captures.push(Capture {
        container: root,
        sizes: HashMap::new(),
        calls: Some(Vec::new()),
    });
    let output = compute_flexbox_layout(tree, NodeId::from(root), wrapped_inputs());
    let capture = tree.context.table_item_captures.pop().unwrap();
    (capture.calls.unwrap(), output)
}

#[test]
fn a_wrapping_column_lists_the_items_that_align_by_a_baseline() {
    for (style, listed) in [
        ("flex-wrap:wrap", true),
        ("flex-wrap:wrap-reverse", true),
        // The cross axis of a vertical container is its vertical inline axis.
        ("flex-wrap:wrap;writing-mode:vertical-rl", true),
        // Taffy shares the baselines of a row.
        ("flex-direction:row;flex-wrap:wrap", false),
        // One line is the whole content box.
        ("flex-wrap:nowrap", false),
        // Nothing asks for a baseline.
        ("flex-wrap:wrap;align-items:flex-start", false),
    ] {
        let (mut doc, root, items) = wrapped_baselines(style);
        let cascade = cascaded(&doc);
        let tree = LayoutSession::bridged(&mut doc, &cascade);
        // The hidden item and the absolutely positioned one are never in it.
        let expected: HashSet<_> = if listed {
            items.into_iter().collect()
        } else {
            HashSet::new()
        };
        assert_eq!(
            inline_baseline::wrapped_members(&tree, root),
            expected,
            "{style}"
        );
    }
}

#[test]
fn line_rooms_hold_the_items_of_each_line_and_the_edges_of_the_line() {
    for (style, rooms) in [
        // The lines share the 120px that the container has left, and the
        // first one starts at the left.
        (
            "flex-wrap:wrap",
            [(0.0, 110.0), (0.0, 110.0), (110.0, 200.0), (110.0, 200.0)],
        ),
        // The first line starts at the right.
        (
            "flex-wrap:wrap-reverse",
            [(90.0, 200.0), (90.0, 200.0), (0.0, 90.0), (0.0, 90.0)],
        ),
        (
            "flex-wrap:wrap;direction:rtl",
            [(90.0, 200.0), (90.0, 200.0), (0.0, 90.0), (0.0, 90.0)],
        ),
        (
            "flex-wrap:wrap-reverse;direction:rtl",
            [(0.0, 110.0), (0.0, 110.0), (110.0, 200.0), (110.0, 200.0)],
        ),
    ] {
        let (mut doc, root, items) = wrapped_baselines(style);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let (calls, output) = wrapped_capture(&mut tree, root);
        let members: HashSet<_> = items.into_iter().collect();
        let found = baseline_lines::rooms(
            &tree,
            root.into(),
            wrapped_inputs(),
            &calls,
            output.size,
            &members,
            None,
        )
        .unwrap_or_else(|| panic!("no rooms for {style}"));
        assert_eq!(found.len(), 4, "{style}");
        for (item, expected) in items.into_iter().zip(rooms) {
            assert_eq!(found[&item].edges, expected, "{style}");
        }
        // The two items of a line share it, and the lines are told apart.
        let [a, b, c, d] = items.map(|item| found[&item].line);
        assert_eq!((a, c), (b, d), "{style}");
        assert_ne!(a, c, "{style}");
    }
}

#[test]
fn line_rooms_refuse_a_replay_that_does_not_reproduce_the_layout() {
    let (mut doc, root, items) = wrapped_baselines("flex-wrap:wrap");
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (calls, output) = wrapped_capture(&mut tree, root);
    let members: HashSet<_> = items.into_iter().collect();
    let rooms = |calls: &[FlexReplayCall], size| {
        baseline_lines::rooms(
            &tree,
            root.into(),
            wrapped_inputs(),
            calls,
            size,
            &members,
            None,
        )
    };
    assert!(rooms(&calls, output.size).is_some());
    // The container settled on another size.
    let mut larger = output.size;
    larger.width += 1.0;
    assert!(rooms(&calls, larger).is_none());
    // The children reported another schedule.
    assert!(rooms(&calls[1..], output.size).is_none());
    // An item that is not in the container has no room in its lines.
    let stranger: HashSet<_> = [usize::MAX].into_iter().collect();
    assert!(
        baseline_lines::rooms(
            &tree,
            root.into(),
            wrapped_inputs(),
            &calls,
            output.size,
            &stranger,
            None
        )
        .is_none()
    );
}

#[test]
fn a_wrapping_column_aligns_its_items_by_the_lines_it_recorded_and_by_none_other() {
    let (mut doc, root, items) = wrapped_baselines("flex-wrap:wrap-reverse");
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let (calls, output) = wrapped_capture(&mut tree, root);
    let [a, b, ..] = items;
    // The flex algorithm puts each item against the edge of its line.
    let before = tree.nodes[a].layout.location.x - tree.nodes[b].layout.location.x;
    assert_eq!(before, 30.0, "a is 20px wide and b is 50px wide");
    let members: HashSet<_> = items.into_iter().collect();
    let rooms = baseline_lines::rooms(
        &tree,
        root.into(),
        wrapped_inputs(),
        &calls,
        output.size,
        &members,
        None,
    )
    .expect("rooms");

    // Without the lines, nothing is known about where to put the items.
    let mut candidates = OofCandidates::new();
    assert!(
        inline_baseline::align_flex(&mut tree, root, output.size, false, None, &mut candidates,)
            .is_none()
    );
    assert_eq!(
        tree.nodes[a].layout.location.x - tree.nodes[b].layout.location.x,
        before
    );

    // With them, the items of a line share a baseline, and the lines are used up.
    tree.context.flex_baseline_lines.insert(root, rooms);
    inline_baseline::align_flex(&mut tree, root, output.size, false, None, &mut candidates);
    assert_eq!(
        tree.nodes[a].layout.location.x, tree.nodes[b].layout.location.x,
        "the group is packed at the right edge of its line"
    );
    assert!(tree.context.flex_baseline_lines.is_empty());
}
