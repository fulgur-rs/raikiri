use super::*;
use crate::Document;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticEdge, AxisStaticPosition, OofCandidate, OofPosition,
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
