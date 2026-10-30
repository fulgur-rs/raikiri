use super::{take_host_failure, with_state, with_state_after_sync, with_state_unchecked};
use crate::runtime::{Abort, DomRuntime, shared, test_host::StubHost};
use std::cell::Cell;

fn runtime() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

#[test]
fn abort_rejects_update_and_observer_without_consuming_notifications() {
    let mut runtime = runtime();
    with_state_unchecked(runtime.context_mut(), |state| {
        state.image_topology_changed = true;
        state.event_loop.aborted = Some(Abort::Nodes);
    })
    .unwrap();
    let updated = Cell::new(false);
    let observed = Cell::new(false);
    let result = with_state_after_sync(
        runtime.context_mut(),
        |_| updated.set(true),
        |state, ()| {
            observed.set(true);
            std::mem::take(&mut state.image_topology_changed)
        },
    );
    assert!(result.is_err());
    assert!(!updated.get());
    assert!(!observed.get());
    assert!(
        with_state_unchecked(runtime.context_mut(), |state| state.image_topology_changed).unwrap()
    );
}

#[test]
fn reentrant_borrow_reports_failure_before_update_or_observer() {
    let mut runtime = runtime();
    let shared = shared(runtime.context_mut());
    let held = shared.0.borrow_mut();
    let updated = Cell::new(false);
    let observed = Cell::new(false);
    let result = with_state_after_sync(
        runtime.context_mut(),
        |_| updated.set(true),
        |_, ()| observed.set(true),
    );
    drop(held);
    assert!(result.is_err());
    assert!(!updated.get());
    assert!(!observed.get());
    assert_eq!(
        take_host_failure(runtime.context_mut()).as_deref(),
        Some("re-entrant DOM runtime access")
    );
    assert_eq!(with_state(runtime.context_mut(), |_| 7).unwrap(), 7);
}

#[test]
fn mark_dirty_propagates_abort_without_changing_runtime_state() {
    let mut runtime = runtime();
    with_state_unchecked(runtime.context_mut(), |state| {
        state.dirty = false;
        state.generation = 7;
        state.image_topology_changed = true;
        state.event_loop.aborted = Some(Abort::Nodes);
    })
    .unwrap();
    assert!(crate::runtime::node::mark_dirty(runtime.context_mut()).is_err());
    let unchanged = with_state_unchecked(runtime.context_mut(), |state| {
        (state.dirty, state.generation, state.image_topology_changed)
    })
    .unwrap();
    assert_eq!(unchanged, (false, 7, true));
}
