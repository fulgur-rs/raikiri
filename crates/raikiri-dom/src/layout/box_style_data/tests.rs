use super::*;
use crate::{Document, node::arena::BoxKey};
use raikiri_layout_style::{AlignedCalc, OwnedPropertyOverride, PreparedData, StyleMetadata};
use raikiri_style::{ComputedValues, property::CalcLengthPercentage};
use std::sync::Arc;

fn record(base: LayoutBase, stamp: PreparedStamp) -> BoxStyleData {
    let values = ComputedValues::initial();
    let recipe =
        matches!(&base, LayoutBase::Synthetic(_)).then_some(recipes::RecipeKind::NeutralSource);
    BoxStyleData {
        base,
        recipe,
        metadata: StyleMetadata::default(),
        pass: Vec::new(),
        prepared: Arc::new(
            PreparedData::prepare_with(&values, |_, expression, range| {
                Arc::new(AlignedCalc::new(expression, range))
            })
            .unwrap(),
        ),
        stamp,
    }
}

#[test]
fn source_and_generated_records_have_separate_identities() {
    let mut store = BoxStyleStore::default();
    let stamp = store.stamp();
    store.insert(BoxKey::Source(3), record(LayoutBase::Source(3), stamp));
    store.insert(
        BoxKey::AnonymousCell(3),
        record(
            LayoutBase::Synthetic(Arc::new(ComputedValues::initial())),
            stamp,
        ),
    );
    assert!(matches!(
        store.get(BoxKey::Source(3)).unwrap().base,
        LayoutBase::Source(3)
    ));
    assert!(matches!(
        store.get(BoxKey::AnonymousCell(3)).unwrap().base,
        LayoutBase::Synthetic(_)
    ));
    assert!(store.get(BoxKey::Source(2)).is_none());
    assert!(store.get(BoxKey::Source(4)).is_none());
    assert!(store.finish_publication(stamp));
    assert!(store.is_ready());
}

#[test]
fn recycled_generated_key_drops_pass_and_requires_a_new_recipe_stamp() {
    let mut store = BoxStyleStore::default();
    let old_stamp = store.stamp();
    let mut old = record(
        LayoutBase::Synthetic(Arc::new(ComputedValues::initial())),
        old_stamp,
    );
    old.pass
        .push(OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    store.insert(BoxKey::AnonymousTable(0), old);
    assert!(store.finish_publication(old_stamp));
    store.invalidate_key(BoxKey::AnonymousTable(0));
    assert!(!store.is_ready());
    assert!(store.get(BoxKey::AnonymousTable(0)).is_none());
    let stamp = store.stamp();
    assert_ne!(old_stamp.recipe, stamp.recipe);
    store.insert(
        BoxKey::AnonymousTable(0),
        record(
            LayoutBase::Synthetic(Arc::new(ComputedValues::initial())),
            stamp,
        ),
    );
    assert!(
        store
            .get(BoxKey::AnonymousTable(0))
            .unwrap()
            .pass
            .is_empty()
    );
    assert!(!store.finish_publication(old_stamp));
    assert!(store.finish_publication(stamp));
}

#[test]
fn interrupted_publication_keeps_old_and_new_payloads_alive() {
    let mut store = BoxStyleStore::default();
    let old = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 5.0,
            percent: 25.0,
        },
        true,
    ));
    let old_weak = Arc::downgrade(&old);
    store.calc_values.push(old);
    assert!(store.finish_publication(store.stamp()));
    store.begin_publication();
    let added = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 6.0,
            percent: 25.0,
        },
        true,
    ));
    let added_weak = Arc::downgrade(&added);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.calc_values.push(added);
        panic!("interrupted publication");
    }));
    assert!(result.is_err());
    assert!(!store.is_ready());
    assert_eq!(old_weak.upgrade().unwrap().resolve(100.0), 30.0);
    assert_eq!(added_weak.upgrade().unwrap().resolve(100.0), 31.0);
    drop(store);
    assert!(old_weak.upgrade().is_none());
    assert!(added_weak.upgrade().is_none());
}

#[test]
fn document_clone_retains_calc_handles_without_inheriting_readiness() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "div", crate::LayoutStyle::default(), None::<&str>);
    let owner = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 5.0,
            percent: 25.0,
        },
        true,
    ));
    let weak = Arc::downgrade(&owner);
    let pointer = Arc::as_ptr(&owner).cast();
    doc.box_style_data.calc_values.push(owner);
    doc.retained_layout_styles
        .get_mut(BoxKey::Source(id))
        .unwrap()
        .size
        .width = taffy::Dimension::calc(pointer);
    let stamp = doc.box_style_data.stamp();
    doc.box_style_data
        .insert(BoxKey::Source(id), record(LayoutBase::Source(id), stamp));
    assert!(doc.box_style_data.finish_publication(stamp));
    let cloned = doc.clone();
    assert!(!cloned.box_style_data.is_ready());
    drop(doc);
    assert!(weak.upgrade().is_some());
    let handle = cloned
        .layout_style(id)
        .unwrap()
        .size
        .width
        .into_raw()
        .calc_value();
    assert_eq!(handle, pointer);
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 0.0), 5.0);
    assert_eq!(crate::taffy_impl::resolve_calc(handle, 100.0), 30.0);
    drop(cloned);
    assert!(weak.upgrade().is_none());
}

#[test]
fn anonymous_cell_reconstruction_invalidates_the_recycled_record() {
    let mut doc = Document::new();
    let key = BoxKey::AnonymousCell(0);
    let old_stamp = doc.box_style_data.stamp();
    let mut old = record(
        LayoutBase::Synthetic(Arc::new(ComputedValues::initial())),
        old_stamp,
    );
    old.pass
        .push(OwnedPropertyOverride::aspect_ratio(Some(2.0)));
    doc.box_style_data.insert(key, old);
    assert!(doc.box_style_data.finish_publication(old_stamp));
    doc.set_anonymous_cells(vec![(
        crate::node::Node::new_element("td".into(), crate::LayoutStyle::default(), None),
        crate::LayoutStyle::default(),
    )]);
    assert!(!doc.box_style_data.is_ready());
    assert!(doc.box_style_data.get(key).is_none());
    assert_ne!(doc.box_style_data.stamp().recipe, old_stamp.recipe);
}

#[test]
fn source_topology_mutation_rejects_a_pre_mutation_stamp() {
    let mut doc = Document::new();
    let stamp = doc.box_style_data.stamp();
    assert!(doc.box_style_data.finish_publication(stamp));
    doc.append_element(Some(0), "div", crate::LayoutStyle::default(), None::<&str>);
    assert!(!doc.box_style_data.is_ready());
    assert_ne!(doc.box_style_data.stamp().source, stamp.source);
    assert_ne!(doc.box_style_data.stamp().topology, stamp.topology);
    assert!(!doc.box_style_data.finish_publication(stamp));
}

#[test]
fn epoch_rollover_does_not_recertify_an_old_store_publication() {
    let mut store = BoxStyleStore::default();
    store.stamp.source = CacheEpoch::for_test(0, u64::MAX);
    let old = store.stamp();
    assert!(store.finish_publication(old));
    store.invalidate_source();
    assert!(store.stamp().source > old.source);
    assert!(!store.finish_publication(old));
    assert!(store.finish_publication(store.stamp()));
}

#[test]
fn exhausted_recipe_epoch_keeps_store_permanently_unready() {
    let mut store = BoxStyleStore::default();
    store.stamp.recipe = CacheEpoch::for_test(u64::MAX, u64::MAX);
    assert!(store.finish_publication(store.stamp()));
    store.invalidate_key(BoxKey::AnonymousCell(0));
    assert!(!store.is_ready());
    assert!(!store.finish_publication(store.stamp()));
    store.invalidate_source();
    assert!(!store.finish_publication(store.stamp()));
    assert!(!store.is_ready());
}

#[test]
fn old_records_survive_interrupted_bridge_until_successful_publication() {
    let mut store = BoxStyleStore::default();
    let key = BoxKey::Source(1);
    let old = record(LayoutBase::Source(1), store.stamp());
    let weak = Arc::downgrade(&old.prepared);
    store.insert(key, old);
    assert!(store.finish_publication(store.stamp()));
    store.begin_bridge();
    assert!(weak.upgrade().is_some());
    assert!(!store.is_ready());
    store.insert(key, record(LayoutBase::Source(1), store.stamp()));
    assert!(weak.upgrade().is_some());
    assert!(store.finish_publication(store.stamp()));
    assert!(weak.upgrade().is_none());
}
