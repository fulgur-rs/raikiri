//! A `calc()` length must stay safe to use after the owner of its payload is
//! gone. These tests use only the public, safe API of the crate: before the
//! payloads were checked, each of them read freed memory.

use raikiri_layout_style::values::{
    CalcRef, Dimension, ExpandedDimension, LengthPercentage, LengthPercentageAuto, MaybeResolve,
    resolve_projected_calc,
};
use raikiri_layout_style::{
    AlignedCalc, CalcSlot, LayoutStyleView, LengthProperty, LengthValue, PreparedCalc,
    PreparedData, PreparedStyle, PropertyOverride, StyleMetadata, ViewContext,
};
use raikiri_style::property::CalcLengthPercentage;
use raikiri_style::{ComputedLengthPercentageOrAuto as L, ComputedValues};
use std::sync::Arc;

const EXPRESSION: CalcLengthPercentage = CalcLengthPercentage {
    px: 5.0,
    percent: 25.0,
};

fn width_values() -> ComputedValues {
    let mut values = ComputedValues::initial();
    values.width = L::Calc(EXPRESSION);
    values
}

fn width_of(view: &LayoutStyleView<'_>) -> Dimension {
    view.size().width
}

fn calc_of(dimension: Dimension) -> CalcRef {
    match dimension.expand() {
        ExpandedDimension::Calc(calc) => calc,
        other => panic!("the width is a calc, not {other:?}"),
    }
}

#[test]
fn a_reference_resolves_while_its_owner_is_retained_and_not_after() {
    let owner = Arc::new(AlignedCalc::new(EXPRESSION, true));
    let calc = CalcRef::retained(&owner);
    assert!(calc.is_live());
    assert!(calc.has_percentage());
    assert_eq!(calc.resolve(100.0), 30.0);

    drop(owner);

    assert!(!calc.is_live());
    assert!(!calc.has_percentage());
    assert_eq!(calc.resolve(100.0), 0.0);
}

#[test]
fn a_clone_of_the_owner_keeps_the_reference_live() {
    let owner = Arc::new(AlignedCalc::new(EXPRESSION, true));
    let clone = Arc::clone(&owner);
    let calc = CalcRef::retained(&owner);

    drop(owner);
    assert_eq!(calc.resolve(100.0), 30.0);

    drop(clone);
    assert_eq!(calc.resolve(100.0), 0.0);
}

#[test]
fn a_length_made_from_a_dropped_owner_resolves_to_zero() {
    let owner = Arc::new(AlignedCalc::new(EXPRESSION, true));
    let dimension = Dimension::calc(CalcRef::retained(&owner));
    let length = LengthPercentage::calc(CalcRef::retained(&owner));
    assert_eq!(dimension.maybe_resolve(Some(100.0)), Some(30.0));
    assert_eq!(length.maybe_resolve(Some(100.0)), Some(30.0));

    drop(owner);

    assert_eq!(dimension.maybe_resolve(Some(100.0)), Some(0.0));
    assert_eq!(length.maybe_resolve(Some(100.0)), Some(0.0));
    assert_eq!(calc_of(dimension).resolve(100.0), 0.0);
}

#[test]
fn a_length_a_getter_returned_outlives_the_prepared_style_it_came_from() {
    let values = width_values();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let width = width_of(&view);
    assert_eq!(width.maybe_resolve(Some(100.0)), Some(30.0));

    drop(prepared);

    assert_eq!(width.maybe_resolve(Some(100.0)), Some(0.0));
    assert!(!calc_of(width).has_percentage());
}

#[test]
fn a_length_a_getter_returned_outlives_shared_prepared_data() {
    let values = width_values();
    let data = Arc::new(
        PreparedData::prepare_with(&values, |_, expression, range| {
            Arc::new(AlignedCalc::new(expression, range))
        })
        .unwrap(),
    );
    let prepared =
        PreparedStyle::from_data(&values, StyleMetadata::default(), Arc::clone(&data)).unwrap();
    let width = width_of(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &[],
        &[],
    ));
    assert_eq!(width.maybe_resolve(Some(100.0)), Some(30.0));

    drop(prepared);
    assert_eq!(width.maybe_resolve(Some(100.0)), Some(30.0));

    drop(data);
    assert_eq!(width.maybe_resolve(Some(100.0)), Some(0.0));
}

#[test]
fn a_length_a_getter_returned_from_a_borrowed_binding_goes_stale_with_it() {
    let values = width_values();
    let payload = AlignedCalc::new(EXPRESSION, true);
    let bindings = [PreparedCalc {
        slot: CalcSlot::Property(LengthProperty::Width),
        payload: &payload,
    }];
    let prepared = PreparedStyle::new(&values, StyleMetadata::default(), &bindings).unwrap();
    let width = width_of(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &[],
        &[],
    ));
    assert_eq!(width.maybe_resolve(Some(100.0)), Some(30.0));

    drop(prepared);
    drop(payload);

    assert_eq!(width.maybe_resolve(Some(100.0)), Some(0.0));
}

#[test]
fn a_length_an_override_brought_goes_stale_with_its_payload() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let payload = AlignedCalc::new(EXPRESSION, true);
    let pass =
        [PropertyOverride::length(LengthProperty::Width, LengthValue::calc(&payload)).unwrap()];
    let width = width_of(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &pass,
        &[],
    ));
    assert_eq!(width.maybe_resolve(Some(200.0)), Some(55.0));

    drop(payload);

    assert_eq!(width.maybe_resolve(Some(200.0)), Some(0.0));
}

#[test]
fn a_stale_reference_does_not_resolve_a_payload_that_took_over_its_storage() {
    let first = Arc::new(AlignedCalc::new(
        CalcLengthPercentage {
            px: 7.0,
            percent: 0.0,
        },
        true,
    ));
    let stale = CalcRef::retained(&first);
    drop(first);

    // Many payloads come and go, so the storage of the dropped one is reused.
    let successors: Vec<_> = (0..1000)
        .map(|i| {
            Arc::new(AlignedCalc::new(
                CalcLengthPercentage {
                    px: 1000.0 + i as f32,
                    percent: 0.0,
                },
                true,
            ))
        })
        .collect();

    assert_eq!(stale.resolve(100.0), 0.0);
    for successor in &successors {
        let live = CalcRef::retained(successor);
        assert_ne!(live, stale);
        assert_eq!(live.resolve(0.0), successor.resolve(0.0));
    }
}

#[test]
fn a_foreign_handle_resolves_to_zero() {
    let _owner = Arc::new(AlignedCalc::new(EXPRESSION, true));
    assert_eq!(resolve_projected_calc(std::ptr::null(), 100.0), 0.0);
    let garbage = [1_usize, 8, 0x40, 0xdead_beef_usize & !7, usize::MAX & !7];
    for address in garbage {
        let handle = std::ptr::without_provenance(address);
        assert_eq!(resolve_projected_calc(handle, 100.0), 0.0, "{address:#x}");
    }
}

#[test]
fn payloads_made_and_dropped_on_many_threads_never_resolve_each_other() {
    let threads: Vec<_> = (0..8)
        .map(|thread| {
            std::thread::spawn(move || {
                let px = 10.0 * (thread + 1) as f32;
                let expression = CalcLengthPercentage { px, percent: 0.0 };
                let mut kept = Vec::new();
                for round in 0..2000 {
                    let owner = Arc::new(AlignedCalc::new(expression, true));
                    let calc = CalcRef::retained(&owner);
                    assert_eq!(calc.resolve(0.0), px);
                    if round % 7 == 0 {
                        kept.push((owner, calc));
                    } else {
                        drop(owner);
                        assert_eq!(calc.resolve(0.0), 0.0);
                    }
                }
                for (owner, calc) in kept {
                    assert_eq!(calc.resolve(0.0), px);
                    drop(owner);
                    assert!(!calc.is_live());
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn lengths_move_to_other_threads_and_go_stale_there() {
    fn assert_send<T: Send>() {}
    assert_send::<Dimension>();
    assert_send::<LengthPercentage>();
    assert_send::<LengthPercentageAuto>();

    let owner = Arc::new(AlignedCalc::new(EXPRESSION, true));
    let dimension = Dimension::calc(CalcRef::retained(&owner));
    let length_percentage = LengthPercentage::calc(CalcRef::retained(&owner));
    let length_percentage_auto = LengthPercentageAuto::calc(CalcRef::retained(&owner));
    // Each length moves to a thread of its own, and all three name the one slot.
    let resolve_elsewhere = move || {
        let threads = [
            std::thread::spawn(move || dimension.maybe_resolve(Some(100.0))),
            std::thread::spawn(move || length_percentage.maybe_resolve(Some(100.0))),
            std::thread::spawn(move || length_percentage_auto.maybe_resolve(Some(100.0))),
        ];
        threads.map(|thread| thread.join().unwrap())
    };
    assert_eq!(resolve_elsewhere(), [Some(30.0); 3]);
    drop(owner);
    assert_eq!(resolve_elsewhere(), [Some(0.0); 3]);
}
