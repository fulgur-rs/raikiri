use super::*;
use crate::node::arena::BoxKey;
use std::cell::Cell;
use taffy::Dimension;

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
        assert_eq!(
            tree.layout_style_for_key(index).unwrap().size.width,
            Dimension::length(11.0)
        );
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
    use taffy::CacheTree;

    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let key = tree.nodes.box_key(child).unwrap();
    let id = taffy::NodeId::from(child);
    let input = taffy::LayoutInput {
        run_mode: taffy::RunMode::ComputeSize,
        sizing_mode: taffy::SizingMode::InherentSize,
        axis: taffy::RequestedAxis::Both,
        known_dimensions: taffy::Size::NONE,
        known_dimensions_are_definite: taffy::Size {
            width: false,
            height: false,
        },
        parent_size: taffy::Size::NONE,
        available_space: taffy::Size {
            width: taffy::AvailableSpace::MaxContent,
            height: taffy::AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: taffy::Line::FALSE,
    };
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
        let a = tree.layout_cache_context.epoch();
        let mut b = a;
        tree.with_overrides(vec![entry(key, 11.0)], |tree| {
            b = tree.layout_cache_context.epoch();
            assert!(b > a);
            assert!(tree.cache_get(id, &input).is_none());
            tree.with_cache_frame(child, ViewContext::default(), |tree| {
                tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
                assert!(tree.cache_get(id, &input).is_some());
            });
        });
        assert!(tree.layout_cache_context.epoch() > b);
        assert!(tree.cache_get(id, &input).is_none());
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_none());
    });
    tree.with_cache_frame(child, ViewContext::default(), |tree| {
        tree.cache_store(id, &input, taffy::LayoutOutput::HIDDEN);
        assert!(tree.cache_get(id, &input).is_some());
    });
}

#[test]
fn scoped_override_production_retains_calc_until_successful_rebinding() {
    use raikiri_style::property::CalcLengthPercentage;
    use std::sync::Arc;

    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
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
    let pointer = std::ptr::from_ref(owner.as_ref()).cast();
    tree.with_overrides(vec![(key, value)], |tree| {
        assert_eq!(
            tree.style_view(child).size().width,
            Dimension::calc(pointer)
        );
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().size.width,
            Dimension::calc(pointer)
        );
        let owner = weak.upgrade().unwrap();
        assert!(owner.non_negative());
        assert_eq!(owner.resolve(0.0), 0.0);
        assert_eq!(owner.resolve(100.0), 15.0);
    });
    assert!(weak.upgrade().is_some());
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
    drop(tree);
    assert!(weak.upgrade().is_some());
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn scoped_override_production_restores_an_applied_setup_prefix() {
    let (mut doc, cascade, child, text) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let key = tree.nodes.box_key(child).unwrap();
    let text_key = tree.nodes.box_key(text).unwrap();
    let old_text = tree.layout_style_for_key(text).unwrap().size.width;
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
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
    assert_eq!(
        tree.layout_style_for_key(text).unwrap().size.width,
        old_text
    );
    assert!(tree.layout_cache_context.epoch() > before);
}

#[test]
fn scoped_override_production_rejects_preparation_before_publication_changes() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
    let key = tree.nodes.box_key(child).unwrap();
    tree.with_overrides(vec![entry(key, 11.0)], |tree| {
        tree.box_style_data.invalidate_metadata();
        let before = tree.layout_cache_context.publication_epoch();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.refresh_style_views();
        }));
        assert!(failure.is_err());
        assert_eq!(tree.layout_cache_context.publication_epoch(), before);
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().size.width,
            Dimension::length(11.0)
        );
    });
    tree.refresh_style_views();
    assert_eq!(tree.style_view(child).size().width, Dimension::length(31.0));
}

#[test]
fn scoped_override_production_validates_the_whole_batch_before_applying_it() {
    let (mut doc, _, child, _) = fixture_with_styles("display:block", "");
    doc.layout_style_for_key_mut(child).unwrap().size.width = Dimension::length(31.0);
    let mut session = LayoutSession::new(&mut doc);
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
            session.layout_style_for_key(child).unwrap().size.width,
            Dimension::length(31.0)
        );
    }
    let mut called = false;
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.with_overrides(
            vec![
                entry(key, 11.0),
                (key, OwnedPropertyOverride::flex_shrink(0.0)),
            ],
            |_| called = true,
        );
    }));
    assert!(failure.is_err());
    assert!(!called);
    assert_eq!(
        session.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
}

#[test]
fn scoped_override_production_reaches_all_generated_key_kinds() {
    let mut doc = Document::new();
    let node = || {
        let style = LayoutStyle {
            size: taffy::Size {
                width: Dimension::length(31.0),
                height: Dimension::auto(),
            },
            ..LayoutStyle::default()
        };
        (
            crate::node::Node::new_element("div".into(), style.clone(), None),
            style,
        )
    };
    doc.set_anonymous_tables(vec![node()]);
    doc.set_anonymous_cells(vec![node()]);
    let (node, style) = node();
    let icb = doc.set_initial_containing_block(node, style);
    let indices = [
        doc.nodes.anonymous_table_key(0),
        doc.nodes.anonymous_cell_key(0),
        icb,
    ];
    let mut session = LayoutSession::new(&mut doc);
    let entries = indices
        .iter()
        .map(|&idx| entry(session.nodes.box_key(idx).unwrap(), 19.0))
        .collect();
    session.with_overrides(entries, |tree| {
        for idx in indices {
            assert_eq!(
                tree.layout_style_for_key(idx).unwrap().size.width,
                Dimension::length(19.0)
            );
        }
    });
    for idx in indices {
        assert_eq!(
            session.layout_style_for_key(idx).unwrap().size.width,
            Dimension::length(31.0)
        );
    }
}

#[test]
fn scoped_override_production_supports_legacy_session_and_nested_error() {
    let mut doc = Document::new();
    let child = doc.append_element(
        Some(0),
        "div",
        LayoutStyle {
            size: taffy::Size {
                width: Dimension::length(31.0),
                height: Dimension::auto(),
            },
            ..LayoutStyle::default()
        },
        None::<&str>,
    );
    let mut session = LayoutSession::new(&mut doc);
    let key = session.nodes.box_key(child).unwrap();
    let before = session.layout_cache_context.epoch();
    let result: Result<(), ()> = session.with_overrides(vec![entry(key, 11.0)], |tree| {
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().size.width,
            Dimension::length(11.0)
        );
        let inner: Result<(), ()> = tree.with_overrides(vec![entry(key, 17.0)], |tree| {
            if tree.layout_style_for_key(child).unwrap().size.width == Dimension::length(17.0) {
                return Err(());
            }
            Ok(())
        });
        assert!(inner.is_err());
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().size.width,
            Dimension::length(11.0)
        );
        tree.layout_style_for_key_mut(child).unwrap().margin.left =
            taffy::LengthPercentageAuto::length(91.0);
        Err(())
    });
    assert!(result.is_err());
    assert_eq!(
        session.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
    assert_eq!(
        session.layout_style_for_key(child).unwrap().margin.left,
        taffy::LengthPercentageAuto::length(91.0)
    );
    assert_ne!(session.layout_cache_context.epoch(), before);
}

#[test]
fn scoped_override_production_supports_block_without_query_wrapper() {
    for child_style in [
        "width:31px;height:7px",
        "position:absolute;width:31px;height:7px",
    ] {
        let (mut doc, cascade, child, _) = fixture_with_styles("display:block", child_style);
        let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
        apply_computed_to_style_in_session(&mut session, &cascade).unwrap();
        assert!(!session.has_style_view(child));
        let key = session.nodes.box_key(child).unwrap();
        session.with_overrides(vec![entry(key, 19.0)], |tree| {
            assert!(!tree.has_style_view(child));
            assert_eq!(
                tree.layout_style_for_key(child).unwrap().size.width,
                Dimension::length(19.0)
            );
        });
        assert_eq!(
            session.layout_style_for_key(child).unwrap().size.width,
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
    let text_before = session.layout_style_for_key(text).unwrap().size.width;
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
                assert_eq!(
                    tree.layout_style_for_key(child).unwrap().size.width,
                    Dimension::length(17.0)
                );
                assert_eq!(tree.style_view(text).size().width, Dimension::length(23.0));
                assert_eq!(
                    tree.layout_style_for_key(text).unwrap().size.width,
                    Dimension::length(23.0)
                );
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
    assert_eq!(
        session.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
    assert_eq!(
        session.layout_style_for_key(text).unwrap().size.width,
        text_before
    );
    assert_eq!(session.style_view(text).size().width, text_before);
}

#[test]
fn scoped_override_checkpoint_rejects_future_markers_without_losing_state() {
    let (mut doc, cascade, child, _) = fixture();
    let mut tree = LayoutSession::with_cascade(&mut doc, &cascade);
    apply_computed_to_style_in_session(&mut tree, &cascade).unwrap();
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
        assert_eq!(
            tree.layout_style_for_key(child).unwrap().size.width,
            Dimension::length(11.0)
        );
    });
    assert_eq!(tree.override_checkpoint(), 0);
    assert_eq!(
        tree.layout_style_for_key(child).unwrap().size.width,
        Dimension::length(31.0)
    );
}
