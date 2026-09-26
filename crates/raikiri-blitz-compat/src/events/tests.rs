use super::*;

#[test]
fn default_state_is_all_false() {
    let state = EventState::default();
    assert!(!state.is_cancelled());
    assert!(!state.propagation_is_stopped());
    assert!(!state.redraw_is_requested());
}

#[test]
fn prevent_default_sets_only_cancelled() {
    let mut state = EventState::default();
    state.prevent_default();
    assert!(state.is_cancelled());
    assert!(!state.propagation_is_stopped());
    assert!(!state.redraw_is_requested());
}

#[test]
fn stop_propagation_sets_only_stopped() {
    let mut state = EventState::default();
    state.stop_propagation();
    assert!(!state.is_cancelled());
    assert!(state.propagation_is_stopped());
    assert!(!state.redraw_is_requested());
}

#[test]
fn request_redraw_sets_only_redraw() {
    let mut state = EventState::default();
    state.request_redraw();
    assert!(!state.is_cancelled());
    assert!(!state.propagation_is_stopped());
    assert!(state.redraw_is_requested());
}

#[test]
fn merge_ors_each_flag() {
    let mut left = EventState::default();
    left.prevent_default();
    let mut right = EventState::default();
    right.stop_propagation();
    right.request_redraw();
    let merged = left.merge(&right);
    assert!(merged.is_cancelled());
    assert!(merged.propagation_is_stopped());
    assert!(merged.redraw_is_requested());
}

#[test]
fn merge_two_defaults_stays_false() {
    let merged = EventState::default().merge(&EventState::default());
    assert!(!merged.is_cancelled());
    assert!(!merged.propagation_is_stopped());
    assert!(!merged.redraw_is_requested());
}

#[test]
fn dom_event_data_variants_construct() {
    let click = DomEvent {
        target: 7,
        bubbles: true,
        cancelable: false,
        data: DomEventData::Click,
        request_redraw: false,
    };
    assert!(click.bubbles);
    let input = DomEventData::Input("hello".to_string());
    assert!(matches!(input, DomEventData::Input(_)));
    let key = DomEventData::KeyDown("Enter".to_string());
    assert!(matches!(key, DomEventData::KeyDown(_)));
    let other = DomEventData::Other(SmolStr::new("custom"));
    assert!(matches!(other, DomEventData::Other(_)));
    let ui = UiEvent::PointerDown;
    assert!(matches!(ui.clone(), UiEvent::PointerDown));
}
