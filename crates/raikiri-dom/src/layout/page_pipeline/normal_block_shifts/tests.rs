use super::*;

fn leaf(node: usize, end: f32, follows: bool) -> Leaf {
    Leaf {
        node,
        ends: Ends([end, end + 10.0, end + 20.0]),
        follows,
        float_follows: follows,
    }
}

#[test]
fn suffix_additions_queries_and_point_updates_match_an_eager_model() {
    let mut state = NormalBlockShifts::default();
    let leaves = vec![
        leaf(17, 3.0, true),
        leaf(9001, 4.0, false),
        leaf(42, -8.0, true),
        leaf(99, 7.0, true),
    ];
    let mut eager: Vec<_> = leaves.iter().map(|leaf| leaf.ends).collect();
    let mut pending = [0.0; 4];
    state.insert(2, leaves.clone());
    for (after, delta) in [(0, 0.25), (2, -30.5), (1, 7.75), (0, -2.0), (3, 100.0)] {
        assert!(state.shift_after(2, leaves[after].node, delta));
        for index in after + 1..leaves.len() {
            if leaves[index].follows {
                eager[index] = eager[index].translated(delta);
                pending[index] += delta;
            }
        }
        for start in 0..=leaves.len() {
            for end in start..=leaves.len() {
                let expected = eager[start..end]
                    .iter()
                    .copied()
                    .fold(Ends::EMPTY, Ends::max);
                assert_eq!(state.range(2, start..end), Some(expected));
            }
        }
        for index in 0..leaves.len() {
            assert_eq!(state.pending(leaves[index].node), pending[index]);
        }
    }
    let before = state.range(2, 0..4);
    assert_eq!(state.consume(42), pending[2]);
    assert_eq!(state.pending(42), 0.0);
    assert_eq!(state.pending(99), pending[3]);
    assert_eq!(state.range(2, 0..4), before);
    state.set_ends(42, Ends([100.0, 110.0, 120.0]));
    assert_eq!(state.range(2, 1..3), Some(Ends([100.0, 110.0, 120.0])));
    assert_eq!(state.pending(42), 0.0);
}

#[test]
fn cloned_projection_keeps_unconsumed_local_moves_independently() {
    let mut state = NormalBlockShifts::default();
    state.insert(2, vec![leaf(3, 10.0, true), leaf(4, 20.0, true)]);
    state.shift_after(2, 3, 12.0);
    let mut clone = state.clone();
    assert_eq!(clone.consume(4), 12.0);
    assert_eq!(clone.pending(4), 0.0);
    assert_eq!(state.pending(4), 12.0);
    assert!(!state.shift_after(100, 3, 2.0));
    assert!(!state.shift_after(2, 999, 2.0));
    assert_eq!(state.pending(999), 0.0);
    assert_eq!(state.consume(999), 0.0);
    assert_eq!(state.range(100, 0..1), None);
}

#[test]
fn empty_parent_and_missing_extents_remain_empty_after_negative_shifts() {
    let mut state = NormalBlockShifts::default();
    state.insert(2, vec![]);
    assert_eq!(state.range(2, 0..0), Some(Ends::EMPTY));
    state.insert(
        3,
        vec![
            leaf(4, 1.0, true),
            Leaf {
                node: 5,
                ends: Ends::EMPTY,
                follows: true,
                float_follows: true,
            },
        ],
    );
    assert!(state.shift_after(3, 4, -50.0));
    assert_eq!(state.range(3, 1..2), Some(Ends::EMPTY));
    assert_eq!(state.pending(5), -50.0);
}

#[test]
fn virtual_local_coordinates_are_consumed_once_and_extraction_discards_layout_state() {
    let mut doc = crate::Document::new();
    let parent = doc.append_element(Some(0), "div", Some("display:block"));
    let first = doc.append_element(Some(parent), "div", Some("display:block"));
    let second = doc.append_element(Some(parent), "div", Some("display:block"));
    doc.nodes[second].layout.location.y = 20.0;
    let mut state = NormalBlockShifts::default();
    state.insert(
        parent,
        vec![leaf(first, 10.0, true), leaf(second, 30.0, true)],
    );
    state.shift_after(parent, first, 12.5);
    doc.normal_block_shifts = Some(Box::new(state));
    assert_eq!(doc.pagination_local_y(second), 32.5);
    let mut cloned = doc.clone();
    cloned.translate_pagination_y(second, -2.0);
    assert_eq!(cloned.pagination_local_y(second), 30.5);
    assert_eq!(doc.pagination_local_y(second), 32.5);
    cloned.set_pagination_local_y(second, 50.0);
    cloned.finish_normal_block_shifts();
    assert_eq!(cloned.nodes[second].layout.location.y, 50.0);
    assert!(cloned.normal_block_shifts.is_none());
    assert!(
        doc.extract_nodes(&[0, parent, first, second])
            .normal_block_shifts
            .is_none()
    );
    doc.finish_normal_block_shifts();
    assert_eq!(doc.nodes[second].layout.location.y, 32.5);
    doc.finish_normal_block_shifts();
    assert_eq!(doc.nodes[second].layout.location.y, 32.5);
}

#[test]
fn pending_static_owner_translation_matches_materialized_oof_origins() {
    use crate::layout::ifc::test_support::block_fixture;
    use crate::layout::test_support::page_box_800x600;
    let mut fixture = block_fixture("position:relative", |doc, root| {
        let first = doc.append_element(Some(root), "div", Some("display:block;height:10px"));
        let second = doc.append_element(Some(root), "div", Some("display:block;height:20px"));
        doc.append_element(
            Some(second),
            "div",
            Some("position:absolute;width:5px;height:5px"),
        );
        doc.append_element(
            Some(first),
            "div",
            Some("position:absolute;top:3px;width:5px;height:5px"),
        );
    });
    crate::layout::layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600())
        .unwrap();
    let root = fixture.root;
    let first = fixture.doc.nodes[root].children[0];
    let second = fixture.doc.nodes[root].children[1];
    let mut eager = fixture.doc.clone();
    eager.nodes[second].layout.location.y += 12.5;
    let mut state = NormalBlockShifts::default();
    state.insert(
        root,
        vec![leaf(first, 10.0, true), leaf(second, 30.0, true)],
    );
    state.shift_after(root, first, 12.5);
    fixture.doc.normal_block_shifts = Some(Box::new(state));
    for node in 0..fixture.doc.node_count() {
        assert_eq!(
            fixture.doc.layout_origin_of(node),
            eager.layout_origin_of(node)
        );
        assert_eq!(
            fixture.doc.layout_offset_of(node),
            eager.layout_offset_of(node)
        );
    }
    fixture.doc.finish_normal_block_shifts();
    for node in 0..fixture.doc.node_count() {
        assert_eq!(
            fixture.doc.nodes[node].layout.location,
            eager.nodes[node].layout.location
        );
    }
}

struct EagerGuard;
impl Drop for EagerGuard {
    fn drop(&mut self) {
        FORCE_EAGER.with(|enabled| enabled.set(false));
    }
}

#[test]
fn deferred_pagination_matches_eager_geometry_pages_and_text_across_block_variants() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    for variant in 0..18 {
        let fixture = block_fixture_with_ahem("font:20px/30px Ahem", |doc, root| {
            let section = doc.append_element(
                Some(root),
                "section",
                Some(match variant {
                    3 => "display:block;min-height:70px;max-height:950px;padding-bottom:3px",
                    6 => "display:flow-root;padding-bottom:3px",
                    13 => "display:block;min-height:70.1px;max-height:950.1px;padding-bottom:3.1px",
                    15 => "display:block;min-height:25599.99px;max-height:25600.01px",
                    17 => "display:block;min-height:25599.99px;max-height:25600.01px;padding-bottom:.01px",
                    7 => "display:block;position:relative;top:7px",
                    _ => "display:block",
                }),
            );
            for index in 0..if variant >= 14 { 256 } else { 32 } {
                let parent = if variant == 1 || variant == 2 {
                    doc.append_element(
                        Some(section),
                        "div",
                        Some(if variant == 1 {
                            "display:contents"
                        } else {
                            "display:block"
                        }),
                    )
                } else {
                    section
                };
                let block = doc.append_element(Some(parent), "div", Some(match variant {
                    4 => "display:block;height:60px;break-inside:avoid;margin-top:-2px;margin-bottom:-3px",
                    7 => "display:block;height:60px;break-inside:avoid;position:relative;top:2px",
                    10 => "display:block;height:60.125px;break-inside:avoid;margin-top:0.0625px",
                    11 => "display:list-item;height:60px;break-inside:avoid;list-style-position:outside",
                    12 | 13 => "display:block;height:60.1px;break-inside:avoid;margin-top:.1px;margin-bottom:-.3px",
                    14 | 15 => "display:block;height:99.99px;break-inside:avoid",
                    16 | 17 => "display:block;height:99.99px;break-inside:avoid;margin-top:.01px;margin-bottom:-.01px",
                    _ => "display:block;height:60px;break-inside:avoid",
                }));
                if (variant == 5 && [0, 15, 31].contains(&index)) || variant == 6 {
                    doc.append_element(
                        Some(block),
                        "span",
                        Some("display:block;float:left;width:10px;height:75px"),
                    );
                }
                if variant == 8 {
                    doc.append_element(
                        Some(block),
                        "span",
                        Some("position:absolute;width:5px;height:5px"),
                    );
                }
                if variant == 9 {
                    doc.append_text(block, "x ");
                    doc.append_element(Some(block), "div", Some("display:block;height:5px"));
                }
                doc.append_text(block, "x");
            }
        });
        let mut lazy = fixture.doc;
        let mut eager = lazy.clone();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 100.0;
        page.height = if variant >= 16 {
            100.01
        } else if variant >= 14 {
            100.0
        } else if variant >= 12 {
            100.1
        } else {
            100.0
        };
        let lazy_pages = super::super::layout_pages(&mut lazy, &fixture.cascade, page).unwrap();
        FORCE_EAGER.with(|enabled| enabled.set(true));
        let guard = EagerGuard;
        let eager_pages = super::super::layout_pages(&mut eager, &fixture.cascade, page).unwrap();
        drop(guard);
        assert_eq!(lazy_pages, eager_pages, "variant={variant}");
        assert!(lazy.normal_block_shifts.is_none());
        for node in 0..lazy.node_count() {
            assert_eq!(
                lazy.nodes[node].layout, eager.nodes[node].layout,
                "variant={variant} node={node}"
            );
            assert_eq!(
                lazy.layout_origin_of(node),
                eager.layout_origin_of(node),
                "variant={variant} node={node}"
            );
        }
        lazy.project_pages(&fixture.cascade, page, &lazy_pages, &[])
            .unwrap();
        eager
            .project_pages(&fixture.cascade, page, &eager_pages, &[])
            .unwrap();
        for index in 0..lazy_pages.len() as u32 {
            let lazy_text: String = lazy
                .page_text_runs(&fixture.cascade, index)
                .iter()
                .map(|run| run.text)
                .collect();
            let eager_text: String = eager
                .page_text_runs(&fixture.cascade, index)
                .iter()
                .map(|run| run.text)
                .collect();
            assert_eq!(lazy_text, eager_text, "variant={variant} page={index}");
        }
    }
}

#[test]
fn exported_float_summaries_match_the_visitor_after_internal_and_negative_moves() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    use crate::layout::test_support::page_box_800x600;
    let mut fixture = block_fixture_with_ahem("", |doc, root| {
        for style in [
            "display:block;position:relative;top:7px",
            "display:flow-root",
            "display:block",
        ] {
            let branch = doc.append_element(Some(root), "div", Some(style));
            for _ in 0..3 {
                let holder =
                    doc.append_element(Some(branch), "div", Some("display:block;height:20px"));
                doc.append_element(
                    Some(holder),
                    "span",
                    Some("display:block;float:left;width:10px;height:35px;margin-bottom:-2px"),
                );
                doc.append_text(holder, "x");
            }
        }
    });
    crate::layout::layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600())
        .unwrap();
    let root = fixture.root;
    let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
    begin(&mut session, &fixture.cascade, root);
    for delta in [0.0, 12.5, -25.25, 3.125] {
        let branches = session.nodes[root].children.to_vec();
        for branch in branches {
            let first = session.nodes[branch].children[0];
            super::super::shift_later_block_siblings(&mut session, branch, first, delta);
        }
        refresh(&mut session, &fixture.cascade, root, root);
        // Ancestor exports stay queued until their consumer asks for them.
        refresh(&mut session, &fixture.cascade, root, 0);
        let positions: Vec<_> = session
            .normal_block_shifts
            .as_ref()
            .unwrap()
            .positions
            .iter()
            .map(|(&node, &position)| (node, position))
            .collect();
        for (child, (parent, index)) in positions {
            let top = super::super::pagination_child_parent_y(
                &session,
                parent,
                child,
                session.pagination_local_y(child),
            );
            let mut expected = f32::NEG_INFINITY;
            super::super::visit_same_bfc_floats_in_subtree(
                &session,
                &fixture.cascade,
                root,
                child,
                top,
                |_, end| expected = expected.max(end),
            );
            let actual = session
                .normal_block_shifts
                .as_ref()
                .unwrap()
                .range(parent, index..index + 1)
                .unwrap()
                .0[2];
            assert_eq!(actual, expected, "child={child} delta={delta}");
        }
    }
    session.finish_normal_block_shifts();
}

#[test]
fn page_limit_errors_and_provisional_results_flush_the_pending_state() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    for provisional in [false, true] {
        let fixture = block_fixture_with_ahem("font:20px/30px Ahem", |doc, root| {
            for _ in 0..16 {
                let child = doc.append_element(
                    Some(root),
                    "div",
                    Some("display:block;height:60px;break-inside:avoid"),
                );
                doc.append_text(child, "x");
            }
        });
        let mut lazy = fixture.doc;
        let mut eager = lazy.clone();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        let control = if provisional {
            super::super::PageLayoutControl::for_geometry_discovery(Some(3))
        } else {
            super::super::PageLayoutControl::new(Some(3))
        };
        let run = |doc: &mut crate::Document| {
            super::super::layout_pages_with_options(
                doc,
                &fixture.cascade,
                page,
                crate::PageLayoutOptions {
                    control: Some(&control),
                    ..Default::default()
                },
            )
        };
        let lazy_result = run(&mut lazy);
        FORCE_EAGER.with(|enabled| enabled.set(true));
        let guard = EagerGuard;
        let eager_result = run(&mut eager);
        drop(guard);
        assert_eq!(lazy_result.is_ok(), provisional);
        assert_eq!(lazy_result.is_ok(), eager_result.is_ok());
        assert!(lazy.normal_block_shifts.is_none());
        for node in 0..lazy.node_count() {
            assert_eq!(
                lazy.nodes[node].layout, eager.nodes[node].layout,
                "provisional={provisional} node={node}"
            );
        }
    }
}

#[test]
fn decimal_updates_preserve_range_maxima_when_pending_offsets_are_consumed() {
    let mut state = NormalBlockShifts::default();
    state.insert(2, (0..4).map(|node| leaf(node + 3, 1000.0, true)).collect());
    for _ in 0..200 {
        state.shift_after(2, 3, 0.01);
    }
    let before = state.range(2, 2..4).unwrap();
    let first = state.range(2, 2..3).unwrap();
    let second = state.range(2, 3..4).unwrap();
    assert_eq!(before, first.max(second));
    state.consume(5);
    state.consume(6);
    assert_eq!(state.range(2, 2..4).unwrap(), before);
    assert_eq!(
        state
            .range(2, 2..3)
            .unwrap()
            .max(state.range(2, 3..4).unwrap()),
        before
    );
}

#[test]
fn negative_last_margin_replaces_the_unadjusted_last_end_in_the_aggregate() {
    let mut doc = crate::Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let first = doc.append_element(Some(parent), "div", None::<&str>);
    let last = doc.append_element(Some(parent), "div", None::<&str>);
    doc.nodes[parent].layout.padding.bottom = 1.0;
    doc.nodes[last].layout.margin.bottom = -50.0;
    let mut state = NormalBlockShifts::default();
    state.insert(
        parent,
        vec![leaf(first, 90.0, true), leaf(last, 100.0, true)],
    );
    doc.normal_block_shifts = Some(Box::new(state));
    let result = extents(&doc, parent, first, 10.0, 10.0, 0.0, false).unwrap();
    assert_eq!(result[0], (80.0, 90.0));
    assert_eq!(result[1], (90.0, 100.0));
}

#[test]
fn indexing_and_refreshing_a_deep_chain_do_not_repeat_depth_walks() {
    use crate::layout::ifc::test_support::block_fixture;
    let mut samples = Vec::new();
    for depth in [16, 32, 64] {
        let mut last = 0;
        let mut fixture = block_fixture("", |doc, root| {
            last = root;
            for _ in 0..depth {
                last = doc.append_element(Some(last), "div", Some("display:block"));
            }
        });
        let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
        DEPTH_WORK.with(|work| work.set(0));
        begin(&mut session, &fixture.cascade, fixture.root);
        session.mark_pagination_extent_dirty(last);
        refresh(&mut session, &fixture.cascade, fixture.root, fixture.root);
        let work = DEPTH_WORK.with(std::cell::Cell::get);
        samples.push(work);
        eprintln!("normal_sibling_depth_work depth={depth} visits={work}");
        session.finish_normal_block_shifts();
    }
    for pair in samples.windows(2) {
        assert!(pair[1] < 3 * pair[0], "{samples:?}");
    }
}

#[test]
fn extreme_cancellation_falls_back_before_deferred_coordinates_lose_their_base() {
    use crate::layout::ifc::test_support::block_fixture;
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_element(Some(root), "div", Some("display:block"));
    });
    let root = fixture.root;
    let first = fixture.doc.nodes[root].children[0];
    let last = fixture.doc.nodes[root].children[1];
    fixture.doc.nodes[last].layout.location.y = 1.0;
    let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
    begin(&mut session, &fixture.cascade, root);
    assert!(session.normal_block_shifts.is_some());
    super::super::shift_later_block_siblings(&mut session, root, first, 1e20);
    assert!(session.normal_block_shifts.is_none());
    super::super::shift_later_block_siblings(&mut session, root, first, -1e20);
    assert_eq!(session.pagination_local_y(last), 0.0);
    assert_eq!(session.nodes[last].layout.location.y, 0.0);
}

#[test]
fn extreme_geometry_is_checked_before_cancellation_hides_its_magnitude() {
    use crate::layout::ifc::test_support::block_fixture;
    for initially_large in [false, true] {
        let mut fixture = block_fixture("", |doc, root| {
            doc.append_element(Some(root), "div", Some("display:block"));
        });
        let root = fixture.root;
        let child = fixture.doc.nodes[root].children[0];
        if initially_large {
            fixture.doc.nodes[child].layout.location.y = -1e20;
            fixture.doc.nodes[child].layout.size.height = 1e20;
        }
        let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
        begin(&mut session, &fixture.cascade, root);
        if !initially_large {
            assert!(session.normal_block_shifts.is_some());
            session.nodes[child].layout.size.height = 1e20;
            session.set_pagination_local_y(child, -1e20);
        }
        assert!(session.normal_block_shifts.is_none());
    }
}

#[test]
fn exported_float_ends_ignore_pending_offsets_of_noncontributing_branches() {
    use crate::layout::ifc::test_support::block_fixture_with_ahem;
    use crate::layout::test_support::page_box_800x600;
    let mut fixture = block_fixture_with_ahem("", |doc, root| {
        doc.append_element(Some(root), "div", Some("display:block;height:10px"));
        let branch = doc.append_element(Some(root), "div", Some("display:block"));
        let holder = doc.append_element(Some(branch), "div", Some("display:block;height:20px"));
        doc.append_element(
            Some(holder),
            "span",
            Some("display:block;float:left;width:10px;height:35px"),
        );
        doc.append_text(holder, "x");
    });
    crate::layout::layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600())
        .unwrap();
    let root = fixture.root;
    let first = fixture.doc.nodes[root].children[0];
    let branch = fixture.doc.nodes[root].children[1];
    let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
    session.nodes[branch]
        .flags
        .insert(crate::node::NodeFlags::IN_IFC_SUBTREE);
    assert!(!session.contributes_layout_offset(branch));
    begin(&mut session, &fixture.cascade, root);
    let before = branch_float_end(
        &session,
        &fixture.cascade,
        root,
        branch,
        session.pagination_local_y(branch),
    );
    super::super::shift_later_block_siblings(&mut session, root, first, 12.5);
    let top = session.pagination_local_y(branch);
    let after = branch_float_end(&session, &fixture.cascade, root, branch, top);
    let mut reference = f32::NEG_INFINITY;
    super::super::visit_same_bfc_floats_in_subtree(
        &session,
        &fixture.cascade,
        root,
        branch,
        top,
        |_, end| reference = reference.max(end),
    );
    assert!(reference.is_finite());
    assert_eq!(after, reference);
    assert_eq!(after, before);
    session.finish_normal_block_shifts();
    assert_eq!(
        branch_float_end(
            &session,
            &fixture.cascade,
            root,
            branch,
            session.pagination_local_y(branch)
        ),
        reference
    );
}

#[test]
fn refreshing_an_extent_at_the_arithmetic_limit_flushes_other_pending_siblings() {
    use crate::layout::ifc::test_support::block_fixture;
    let mut fixture = block_fixture("", |doc, root| {
        doc.append_element(Some(root), "div", Some("display:block"));
        doc.append_element(Some(root), "div", Some("display:block"));
    });
    let root = fixture.root;
    let first = fixture.doc.nodes[root].children[0];
    let last = fixture.doc.nodes[root].children[1];
    fixture.doc.nodes[last].layout.location.y = 1_000_000.0;
    let mut session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
    begin(&mut session, &fixture.cascade, root);
    super::super::shift_later_block_siblings(&mut session, root, first, 7.5);
    session.mark_pagination_extent_dirty(first);
    let state = session.normal_block_shifts.as_mut().unwrap();
    while state.can_defer(0.0) {
        state.arithmetic_updates += 1.0;
    }
    state.arithmetic_updates -= 1.0;
    refresh(&mut session, &fixture.cascade, root, root);
    assert!(session.normal_block_shifts.is_none());
    assert_eq!(session.nodes[last].layout.location.y, 1_000_007.5);
}

#[test]
fn extent_queries_reject_untracked_children_and_keep_a_trailing_float_separate() {
    let mut doc = crate::Document::new();
    let parent = doc.append_element(Some(0), "div", None::<&str>);
    let flow = doc.append_element(Some(parent), "div", None::<&str>);
    let float = doc.append_element(Some(parent), "div", None::<&str>);
    let hidden = doc.append_element(Some(parent), "div", None::<&str>);
    let float_parent = doc.append_element(Some(0), "div", None::<&str>);
    let only_float = doc.append_element(Some(float_parent), "div", None::<&str>);
    let mut state = NormalBlockShifts::default();
    state.insert(
        parent,
        vec![
            Leaf {
                node: flow,
                ends: Ends([90.0, 90.0, f32::NEG_INFINITY]),
                follows: true,
                float_follows: true,
            },
            Leaf {
                node: float,
                ends: Ends([f32::NEG_INFINITY, f32::NEG_INFINITY, 110.0]),
                follows: true,
                float_follows: true,
            },
            Leaf {
                node: hidden,
                ends: Ends::EMPTY,
                follows: false,
                float_follows: false,
            },
        ],
    );
    state.insert(
        float_parent,
        vec![Leaf {
            node: only_float,
            ends: Ends([f32::NEG_INFINITY, f32::NEG_INFINITY, 20.0]),
            follows: true,
            float_follows: true,
        }],
    );
    doc.normal_block_shifts = Some(Box::new(state));
    assert_eq!(
        extents(&doc, float_parent, flow, 10.0, 20.0, 30.0, false),
        None
    );
    assert_eq!(extents(&doc, parent, hidden, 10.0, 20.0, 30.0, false), None);
    assert_eq!(extents(&doc, parent, 999, 10.0, 20.0, 30.0, false), None);
    assert_eq!(
        extents(&doc, parent, float, 10.0, 20.0, 30.0, false),
        Some([(90.0, 90.0), (90.0, 90.0), (80.0, 110.0)])
    );
    assert_eq!(
        extents(&doc, float_parent, only_float, 5.0, 5.0, 5.0, true),
        Some([
            (f32::NEG_INFINITY, f32::NEG_INFINITY),
            (f32::NEG_INFINITY, f32::NEG_INFINITY),
            (15.0, 20.0)
        ])
    );
}

#[test]
fn out_of_flow_and_footnote_boundaries_match_the_existing_float_visitor() {
    use crate::layout::ifc::test_support::block_fixture;
    for style in ["position:absolute", "position:fixed", "float:footnote"] {
        let mut fixture = block_fixture(style, |doc, root| {
            let child = doc.append_element(Some(root), "div", Some("display:block"));
            doc.append_element(
                Some(child),
                "div",
                Some("display:block;float:left;width:10px;height:35px"),
            );
        });
        let root = fixture.root;
        let session = LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
        let mut reference = f32::NEG_INFINITY;
        super::super::visit_same_bfc_floats_in_subtree(
            &session,
            &fixture.cascade,
            root,
            root,
            0.0,
            |_, end| reference = reference.max(end),
        );
        assert_eq!(
            branch_float_end(&session, &fixture.cascade, root, root, 0.0),
            reference
        );
        eprintln!(
            "float_boundary style={style} holds={} active={}",
            super::super::holds_floats(&session, &fixture.cascade, root),
            super::super::is_floating_box_for_pagination(&session, &fixture.cascade, root)
        );
    }
}
