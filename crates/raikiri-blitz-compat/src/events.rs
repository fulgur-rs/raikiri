//! blitz_traits::events compatible shape (minimal).

use smol_str::SmolStr;

#[derive(Default)]
pub struct EventState {
    cancelled: bool,
    propagation_stopped: bool,
    redraw_requested: bool,
}
impl EventState {
    pub fn prevent_default(&mut self) {
        self.cancelled = true;
    }
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }
    pub fn request_redraw(&mut self) {
        self.redraw_requested = true;
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    pub fn propagation_is_stopped(&self) -> bool {
        self.propagation_stopped
    }
    pub fn redraw_is_requested(&self) -> bool {
        self.redraw_requested
    }
    pub fn merge(&self, other: &EventState) -> EventState {
        EventState {
            cancelled: self.cancelled | other.cancelled,
            propagation_stopped: self.propagation_stopped | other.propagation_stopped,
            redraw_requested: self.redraw_requested | other.redraw_requested,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    PointerMove,
    PointerUp,
    PointerDown,
    Wheel,
    KeyUp,
    KeyDown,
    Ime,
}

#[derive(Debug, Clone)]
pub struct DomEvent {
    pub target: usize,
    pub bubbles: bool,
    pub cancelable: bool,
    pub data: DomEventData,
    pub request_redraw: bool,
}

#[derive(Debug, Clone)]
pub enum DomEventData {
    Click,
    Input(String),
    KeyDown(String),
    Other(SmolStr),
}
