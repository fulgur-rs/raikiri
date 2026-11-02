use super::*;
use crate::layout::protocol::{
    self, AvailableSpace, LayoutInput, LayoutOutput, RequestedAxis, RunMode, SizingMode,
};
use crate::node::arena::BoxKey;
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::values::{Dimension, LengthPercentage};
use std::cell::Cell;

type SetupInspector = fn(&LayoutSession<'_>, usize, usize);

thread_local! {
    static SETUP_INSPECTOR: Cell<Option<SetupInspector>> = const { Cell::new(None) };
    static SETUP_PANIC_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
    static SAW_APPLIED_SETUP: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn after_setup_entry(tree: &LayoutSession<'_>, index: usize, count: usize) {
    if let Some(inspect) = SETUP_INSPECTOR.get() {
        inspect(tree, index, count);
    }
    if SETUP_PANIC_AFTER.get() == Some(count) {
        assert_eq!(tree.style_view(index).size().width, Dimension::length(11.0));
        SAW_APPLIED_SETUP.set(true);
        panic!("fail after an applied setup entry");
    }
}

pub(crate) fn with_setup_inspector<R>(inspect: SetupInspector, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<SetupInspector>);
    impl Drop for Reset {
        fn drop(&mut self) {
            SETUP_INSPECTOR.set(self.0);
        }
    }
    let _reset = Reset(SETUP_INSPECTOR.replace(Some(inspect)));
    f()
}

struct ResetSetupFailure;

impl Drop for ResetSetupFailure {
    fn drop(&mut self) {
        SETUP_PANIC_AFTER.set(None);
    }
}

fn entry(key: BoxKey, px: f32) -> (BoxKey, OwnedPropertyOverride) {
    (
        key,
        OwnedPropertyOverride::length(LengthProperty::Width, L::Px(px), true).unwrap(),
    )
}

#[test]
fn scoped_override_production_invalidates_a_b_a_cache_and_late_outer_store() {
    use raikiri_layout_style::ViewContext;

    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let id = protocol::NodeId::from(child);
    let input = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
        let a = tree.layout_cache_context.epoch();
        let mut b = a;
        tree.with_overrides(vec![entry(key, 11.0)], |tree| {
            b = tree.layout_cache_context.epoch();
            assert!(b > a);
            assert!(tree.cache_get(id, &input).is_none());
            tree.with_cache_frame(child, ViewContext::default(), |tree| {
                tree.cache_store(id, &input, LayoutOutput::HIDDEN);
                assert!(tree.cache_get(id, &input).is_some());
            });
        });
        assert!(tree.layout_cache_context.epoch() > b);
        assert!(tree.cache_get(id, &input).is_none());
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_none());
    });
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
    });
}

#[test]
fn a_local_override_scope_leaves_the_cache_generation_alone_on_the_way_in_and_out() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let before = tree.style_view(child).size().width;
    let epoch = tree.layout_cache_context.epoch();
    tree.with_local_overrides(vec![entry(key, 11.0)], |tree| {
        assert_eq!(tree.style_view(child).size().width, Dimension::length(11.0));
        assert_eq!(tree.override_checkpoint(), 1);
        assert_eq!(tree.layout_cache_context.epoch(), epoch);
    });
    assert_eq!(tree.style_view(child).size().width, before);
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}

#[test]
fn a_local_scope_around_an_ordinary_one_leaves_the_generation_to_the_ordinary_one() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let epoch = tree.layout_cache_context.epoch();
    let mut inner = epoch;
    tree.with_local_overrides(vec![entry(key, 11.0)], |tree| {
        tree.with_overrides(vec![entry(key, 12.0)], |tree| {
            assert_eq!(tree.style_view(child).size().width, Dimension::length(12.0));
            inner = tree.layout_cache_context.epoch();
            assert!(inner > epoch);
        });
        // The ordinary scope moved the generation on the way out as well, and
        // the local one it is nested in does not take that back.
        assert!(tree.layout_cache_context.epoch() > inner);
        assert_eq!(tree.style_view(child).size().width, Dimension::length(11.0));
    });
    assert!(tree.layout_cache_context.epoch() > inner);
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn a_local_override_scope_that_unwinds_restores_the_style_and_leaves_the_generation_alone() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let before = tree.style_view(child).size().width;
    let epoch = tree.layout_cache_context.epoch();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_local_overrides(vec![entry(key, 11.0)], |_| {
            panic!("local scope body failure")
        })
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"local scope body failure")
    );
    assert_eq!(tree.style_view(child).size().width, before);
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.layout_cache_context.epoch(), epoch);
}

#[test]
fn scoped_override_production_retains_calc_until_successful_rebinding() {
    use raikiri_style::property::CalcLengthPercentage;
    use std::sync::Arc;

    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let value = OwnedPropertyOverride::length(
        LengthProperty::Width,
        L::Calc(CalcLengthPercentage {
            px: -10.0,
            percent: 25.0,
        }),
        false,
    )
    .unwrap();
    let owner = value.calc_owner().unwrap();
    let weak = Arc::downgrade(owner);
    let calc = raikiri_layout_style::values::CalcRef::retained(owner);
    tree.with_overrides(vec![(key, value)], |tree| {
        assert_eq!(tree.style_view(child).size().width, Dimension::calc(calc));
        let owner = weak.upgrade().unwrap();
        assert!(owner.non_negative());
        assert_eq!(owner.resolve(0.0), 0.0);
        assert_eq!(owner.resolve(100.0), 15.0);
    });
    assert!(weak.upgrade().is_some());
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
    drop(tree);
    assert!(weak.upgrade().is_some());
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn scoped_override_production_restores_an_applied_setup_prefix() {
    let (mut doc, cascade, child, text) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let text_key = tree.nodes.box_key(text).unwrap();
    let old_text = tree.style_view(text).size().width;
    let before = tree.layout_cache_context.epoch();
    let _reset = ResetSetupFailure;
    SAW_APPLIED_SETUP.set(false);
    SETUP_PANIC_AFTER.set(Some(1));
    let mut called = false;
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_overrides(vec![entry(key, 11.0), entry(text_key, 17.0)], |_| {
            called = true
        });
    }));
    assert!(failure.is_err());
    assert!(SAW_APPLIED_SETUP.get());
    assert!(!called);
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_override_depth(child), 0);
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
    assert_eq!(tree.style_view(text).size().width, old_text);
    assert!(tree.layout_cache_context.epoch() > before);
}

#[test]
fn scoped_override_production_rejects_preparation_before_publication_changes() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(vec![entry(key, 11.0)], |tree| {
        tree.box_style_data.invalidate_metadata();
        let before = tree.layout_cache_context.publication_epoch();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.refresh_style_views();
        }));
        assert!(failure.is_err());
        assert_eq!(tree.layout_cache_context.publication_epoch(), before);
        // The rejected preparation leaves the scope live; the views are not
        // ready for a query until the next preparation.
        assert_eq!(tree.style_override_depth(child), 1);
        assert_eq!(tree.override_checkpoint(), 1);
    });
    tree.refresh_style_views();
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
}

#[test]
fn scoped_override_production_validates_the_whole_batch_before_applying_it() {
    let (mut doc, _, child, _) = fixture_with_styles("display:block", "width:31px");
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
    let key = session.nodes.box_key(child).unwrap();
    for invalid in [
        BoxKey::Source(usize::MAX),
        BoxKey::AnonymousCell(usize::MAX),
        BoxKey::AnonymousTable(usize::MAX),
        BoxKey::InitialContainingBlock,
    ] {
        let mut called = false;
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.with_overrides(vec![entry(key, 11.0), entry(invalid, 17.0)], |_| {
                called = true;
            });
        }));
        assert!(failure.is_err());
        assert!(!called);
        assert_eq!(
            session.style_view(child).size().width,
            Dimension::length(31.0)
        );
    }
    let mut called = false;
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_overrides(
            vec![
                entry(key, 11.0),
                (
                    key,
                    OwnedPropertyOverride::clear(raikiri_style::property::ClearValue::None),
                ),
            ],
            |_| called = true,
        );
    }));
    assert!(failure.is_err());
    assert!(!called);
    assert_eq!(
        session.style_view(child).size().width,
        Dimension::length(31.0)
    );
}

#[test]
fn scoped_override_production_holds_a_self_alignment_next_to_an_inset() {
    use raikiri_layout_style::values::AlignItems;
    let (mut doc, _, child, _) = fixture_with_styles("display:block", "width:31px");
    let cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &cascade);
    let key = session.nodes.box_key(child).unwrap();
    assert_eq!(session.style_view(child).bridge_justify_self(), None);
    assert_eq!(session.style_view(child).bridge_align_self(), None);
    let centered = Some(AlignItems::CENTER);
    session.with_overrides(
        vec![
            (
                key,
                OwnedPropertyOverride::length(LengthProperty::Left, L::Px(4.0), false).unwrap(),
            ),
            (key, OwnedPropertyOverride::justify_self(centered)),
            (key, OwnedPropertyOverride::align_self(centered)),
        ],
        |tree| {
            let view = tree.style_view(child);
            assert_eq!(view.bridge_justify_self(), centered);
            assert_eq!(view.bridge_align_self(), centered);
            assert_eq!(view.size().width, Dimension::length(31.0));
        },
    );
    assert_eq!(session.style_view(child).bridge_justify_self(), None);
    assert_eq!(session.style_view(child).bridge_align_self(), None);
}

#[test]
fn scoped_override_production_reaches_all_generated_key_kinds() {
    let mut doc = Document::new();
    let source = doc.append_element(Some(0), "div", Some("display:block"));
    // A table row with text in it, and no table around it, gets an anonymous
    // table and an anonymous cell.
    let row = doc.append_element(Some(source), "span", Some("display:table-row"));
    doc.append_text(row, "x");
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
    let icb = session
        .with_style_preparation_batch(|session| {
            Ok::<_, ()>(crate::layout::initial_containing_block::prepare(
                session.document,
                &session_cascade,
                100.0,
                100.0,
            ))
        })
        .unwrap();
    let indices = [
        session.nodes.anonymous_table_key(0),
        session.nodes.anonymous_cell_key(0),
        icb,
    ];
    let originals = indices.map(|idx| session.style_view(idx).size().width);
    let entries = indices
        .iter()
        .map(|&idx| entry(session.nodes.box_key(idx).unwrap(), 19.0))
        .collect();
    session.with_overrides(entries, |tree| {
        for idx in indices {
            assert_eq!(tree.style_view(idx).size().width, Dimension::length(19.0));
        }
    });
    for (idx, original) in indices.into_iter().zip(originals) {
        assert_eq!(session.style_view(idx).size().width, original);
        assert_ne!(original, Dimension::length(19.0));
    }
}

#[test]
fn scoped_override_production_supports_legacy_session_and_nested_error() {
    let mut doc = Document::new();
    let child = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;box-sizing:border-box;width:31px"),
    );
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = LayoutSession::bridged(&mut doc, &session_cascade);
    let key = session.nodes.box_key(child).unwrap();
    let before = session.layout_cache_context.epoch();
    let result: Result<(), ()> = session.with_overrides(vec![entry(key, 11.0)], |tree| {
        assert_eq!(tree.style_view(child).size().width, Dimension::length(11.0));
        let inner: Result<(), ()> = tree.with_overrides(vec![entry(key, 17.0)], |tree| {
            if tree.style_view(child).size().width == Dimension::length(17.0) {
                return Err(());
            }
            Ok(())
        });
        assert!(inner.is_err());
        assert_eq!(tree.style_view(child).size().width, Dimension::length(11.0));
        Err(())
    });
    assert!(result.is_err());
    assert_eq!(
        session.style_view(child).size().width,
        Dimension::length(31.0)
    );
    assert_ne!(session.layout_cache_context.epoch(), before);
}

#[test]
fn scoped_override_production_supports_prepared_block_queries() {
    for child_style in [
        "width:31px;height:7px",
        "position:absolute;width:31px;height:7px",
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles("display:block", child_style);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        assert!(session.has_style_view(child));
        let key = session.nodes.box_key(child).unwrap();
        session.with_overrides(vec![entry(key, 19.0)], |tree| {
            assert!(tree.has_style_view(child));
            assert_eq!(tree.style_view(child).size().width, Dimension::length(19.0));
        });
        assert_eq!(
            session.style_view(child).size().width,
            Dimension::length(31.0)
        );
    }
}

#[test]
fn scoped_override_production_updates_view_and_legacy_then_unwinds_multiple_keys() {
    let (mut doc, cascade, child, text) = fixture();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
    let child_key = session.nodes.box_key(child).unwrap();
    let text_key = session.nodes.box_key(text).unwrap();
    let text_before = session.style_view(text).size().width;
    let mut observed_overrides = false;
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_overrides(
            vec![
                entry(child_key, 11.0),
                entry(text_key, 23.0),
                entry(child_key, 17.0),
            ],
            |tree| {
                assert_eq!(tree.style_view(child).size().width, Dimension::length(17.0));
                assert_eq!(tree.style_view(text).size().width, Dimension::length(23.0));
                observed_overrides = true;
                panic!("unwind a multi-key override scope");
            },
        );
    }));
    assert!(failure.is_err());
    assert!(observed_overrides);
    assert_eq!(
        session.style_view(child).size().width,
        Dimension::length(31.0)
    );
    assert_eq!(session.style_view(text).size().width, text_before);
}

#[test]
fn scoped_override_checkpoint_rejects_future_markers_without_losing_state() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(vec![entry(key, 11.0)], |tree| {
        let checkpoint = tree.override_checkpoint();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.restore_overrides_to(checkpoint + 1)
        }));
        let payload = result.expect_err("a future checkpoint must be rejected");
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
        assert_eq!(message, Some("scope checkpoint is active"));
        assert_eq!(tree.override_checkpoint(), checkpoint);
        assert_eq!(tree.style_override_depth(child), 1);
        assert_eq!(tree.style_view(child).size().width, Dimension::length(11.0));
    });
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
}

fn border_entry(key: BoxKey, px: f32) -> (BoxKey, OwnedPropertyOverride) {
    (
        key,
        OwnedPropertyOverride::length(LengthProperty::BorderLeft, L::Px(px), true).unwrap(),
    )
}

#[test]
fn border_scope_edge_overlap() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:flex", "width:31px;height:7px;padding-left:10%");
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let original = tree.style_view(child).border();
    tree.with_resolved_edges(child, 200.0, false, |tree| {
        tree.with_overrides(vec![border_entry(key, 17.0)], |tree| {
            assert_eq!(
                tree.style_view(child).border().left,
                LengthPercentage::length(17.0)
            );
            let result: Result<(), ()> =
                tree.with_overrides(vec![border_entry(key, 9.0)], |tree| {
                    assert_eq!(
                        tree.style_view(child).padding().left,
                        LengthPercentage::length(20.0)
                    );
                    assert_eq!(
                        tree.style_view(child).border().left,
                        LengthPercentage::length(9.0)
                    );
                    Err(())
                });
            assert_eq!(result, Err(()));
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tree.with_overrides(vec![border_entry(key, 2.0)], |tree| {
                    tree.record_pass_override(
                        child,
                        OwnedPropertyOverride::length(
                            LengthProperty::PaddingTop,
                            L::Px(91.0),
                            true,
                        )
                        .unwrap(),
                    );
                    panic!("border body failure");
                });
            }))
            .unwrap_err();
            assert_eq!(panic.downcast_ref::<&str>(), Some(&"border body failure"));
            assert_eq!(
                tree.style_view(child).padding().top,
                LengthPercentage::length(91.0)
            );
            assert_eq!(
                tree.style_view(child).border().left,
                LengthPercentage::length(17.0)
            );
        });
        assert_eq!(tree.style_view(child).border(), original);
    });
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(
        tree.style_view(child).padding().left,
        LengthPercentage::percent(0.1)
    );
}

#[test]
fn border_scope_partial_setup_restores_applied_fields() {
    fn fail(tree: &LayoutSession<'_>, idx: usize, count: usize) {
        if count == 2 {
            assert_eq!(
                tree.style_view(idx).border().left,
                LengthPercentage::length(17.0)
            );
            assert_eq!(tree.style_view(idx).flex_shrink(), 0.0);
            panic!("border setup failure");
        }
    }
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let original = tree.style_view(child).border();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_setup_inspector(fail, || {
            tree.with_overrides(
                vec![
                    border_entry(key, 17.0),
                    (key, OwnedPropertyOverride::flex_shrink(0.0)),
                ],
                |_| panic!("unreachable body"),
            );
        })
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"border setup failure"));
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(tree.style_view(child).border(), original);
    assert_eq!(tree.style_view(child).flex_shrink(), 1.0);
}

#[test]
fn border_scope_legacy_session_reaches_synthetic_and_wrapperless_keys() {
    let mut doc = Document::new();
    let source = doc.append_element(Some(0), "div", Some("display:block"));
    // A table row with text in it, and no table around it, gets anonymous
    // table boxes.
    let row = doc.append_element(Some(source), "span", Some("display:table-row"));
    doc.append_text(row, "x");
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &session_cascade);
    let anonymous = tree.nodes.anonymous_table_key(0);
    let indices = [source, anonymous];
    let entries = indices
        .iter()
        .flat_map(|&idx| {
            let key = tree.nodes.box_key(idx).unwrap();
            [
                border_entry(key, 17.0),
                (key, OwnedPropertyOverride::flex_shrink(0.0)),
            ]
        })
        .collect();
    tree.with_overrides(entries, |tree| {
        for idx in indices {
            assert_eq!(
                tree.style_view(idx).border().left,
                LengthPercentage::length(17.0)
            );
            assert_eq!(tree.style_view(idx).flex_shrink(), 0.0);
        }
    });
    for idx in indices {
        assert_eq!(
            tree.style_view(idx).border().left,
            LengthPercentage::length(0.0)
        );
        assert_eq!(tree.style_view(idx).flex_shrink(), 1.0);
    }
    assert_eq!(tree.override_checkpoint(), 0);
}

#[test]
fn border_scope_retains_calc_until_successful_rebinding() {
    use raikiri_style::property::CalcLengthPercentage;
    use std::sync::Arc;

    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let value = OwnedPropertyOverride::length(
        LengthProperty::BorderLeft,
        L::Calc(CalcLengthPercentage {
            px: -10.0,
            percent: 25.0,
        }),
        false,
    )
    .unwrap();
    let owner = value.calc_owner().unwrap();
    let weak = Arc::downgrade(owner);
    let calc = raikiri_layout_style::values::CalcRef::retained(owner);
    tree.with_overrides(vec![(key, value)], |tree| {
        assert_eq!(
            tree.style_view(child).border().left,
            LengthPercentage::calc(calc)
        );
        let owner = weak.upgrade().unwrap();
        assert!(owner.non_negative());
        assert_eq!(owner.resolve(0.0), 0.0);
        assert_eq!(owner.resolve(100.0), 15.0);
    });
    assert!(weak.upgrade().is_some());
    assert_eq!(
        tree.style_view(child).border().left,
        LengthPercentage::length(0.0)
    );
    drop(tree);
    assert!(weak.upgrade().is_some());
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn scoped_used_padding_restores_percentage_edges_after_nested_unwind() {
    let (mut doc, cascade, child, _) =
        fixture_with_styles("display:block", "width:31px;padding:10%;border:2px solid");
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let key = tree.nodes.box_key(child).unwrap();
    let padding = |px| {
        (
            key,
            OwnedPropertyOverride::length(LengthProperty::PaddingLeft, L::Px(px), true).unwrap(),
        )
    };
    let original = tree.physical_style(child).padding();
    tree.with_overrides(vec![padding(10.0)], |tree| {
        assert_eq!(
            tree.physical_style(child).padding().left,
            LengthPercentage::length(10.0)
        );
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_overrides(vec![padding(5.0)], |tree| {
                assert_eq!(
                    tree.physical_style(child).padding().left,
                    LengthPercentage::length(5.0)
                );
                panic!("padding scope unwind");
            });
        }));
        assert!(failure.is_err());
        assert_eq!(
            tree.physical_style(child).padding().left,
            LengthPercentage::length(10.0)
        );
        assert_eq!(
            tree.physical_style(child).border().left,
            LengthPercentage::length(2.0)
        );
    });
    assert_eq!(tree.physical_style(child).padding(), original);
    assert_eq!(tree.override_checkpoint(), 0);
}
