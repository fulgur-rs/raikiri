//! Shared block and inline flow state over public float placement.

use crate::layout::protocol::{
    BfcSlot, CollapsibleMarginSet, ContentSlot, FloatContext, FloatDirection,
};
use raikiri_layout_geometry::{Point, Size};
use raikiri_layout_style::values::{Clear, Direction};
use std::sync::Arc;

/// How much wider than a slot a box may be and still count as fitting it.
const FIT_SLACK: f32 = 0.001;

/// What [`BlockContext::find_box_slot`] asks its caller to measure.
#[derive(Clone, Copy)]
pub(crate) enum BoxProbe {
    /// The border-box width of the box in a slot whose `auto` width resolves
    /// to `stretch`.
    Width { stretch: f32 },
    /// The border-box height of the box when it is `width` wide.
    Height { width: f32 },
}

/// The room [`BlockContext::find_box_slot`] found for a box, and the
/// border-box width the box has in it.
#[derive(Clone, Copy)]
pub(crate) struct BoxSlot {
    pub(crate) slot: BfcSlot,
    pub(crate) width: f32,
}

/// The edges of the room `slot` leaves, left to right: where a border box
/// that fits starts and ends, then where an `auto` width stretches from and
/// to. The side that leads in `direction` is the same for both; a trailing
/// margin that no float reaches is left out of the stretch.
fn room_edges(slot: BfcSlot, direction: Direction) -> [f32; 4] {
    let fit = (slot.x, slot.x + slot.border_width);
    let stretch = match direction {
        Direction::Ltr => (slot.x, slot.x + slot.stretch_width),
        Direction::Rtl => (fit.1 - slot.stretch_width, fit.1),
    };
    [fit.0, fit.1, stretch.0, stretch.1]
}

/// Float placement storage owned by one CSS block formatting context.
pub(crate) struct BlockFormattingContext {
    floats: Arc<FloatContext>,
}

impl BlockFormattingContext {
    pub(crate) fn new() -> Self {
        Self {
            floats: Arc::new(FloatContext::new()),
        }
    }

    pub(crate) fn root_block_context(&mut self) -> BlockContext<'_> {
        BlockContext {
            owner: self,
            origin: 0.0,
            border_insets: [0.0; 2],
            content_insets: [0.0; 2],
            root: true,
            state: FlowState::new([false; 2]),
        }
    }
}

#[derive(Clone, Copy)]
struct LeadingStrut {
    anchor: f32,
    margins: CollapsibleMarginSet,
}

#[derive(Clone, Copy)]
struct FlowState {
    float_end: f32,
    adjoining: [bool; 2],
    leading_adjoining: Option<[bool; 2]>,
    resolved: bool,
    leading_strut: Option<LeadingStrut>,
    /// How low the floats reached before the float that starts a page, in the
    /// float storage's own coordinates.
    page_start_floor: Option<f32>,
}

impl FlowState {
    fn new(adjoining: [bool; 2]) -> Self {
        Self {
            float_end: f32::NEG_INFINITY,
            adjoining,
            leading_adjoining: None,
            resolved: false,
            leading_strut: None,
            page_start_floor: None,
        }
    }
}

/// A transaction includes both float geometry and the flow's margin lifecycle.
/// Saved float storage is shared until placement or width changes require a copy.
pub(crate) struct BlockContextSnapshot {
    floats: Arc<FloatContext>,
    state: FlowState,
    content_insets: [f32; 2],
}

/// Coordinates and margin lifecycle of one block within its owning context.
/// The borrow ties nested blocks to the same float storage without exposing
/// or depending on the private fields of Taffy's native block context.
pub(crate) struct BlockContext<'a> {
    owner: &'a mut BlockFormattingContext,
    origin: f32,
    border_insets: [f32; 2],
    content_insets: [f32; 2],
    root: bool,
    state: FlowState,
}

impl BlockContext<'_> {
    pub(crate) fn sub_context(&mut self, offset: f32, insets: [f32; 2]) -> BlockContext<'_> {
        let border_insets = [
            self.border_insets[0] + insets[0],
            self.border_insets[1] + insets[1],
        ];
        BlockContext {
            origin: self.origin + offset,
            border_insets,
            content_insets: border_insets,
            root: false,
            state: FlowState::new(self.state.adjoining),
            owner: self.owner,
        }
    }

    /// Carry the unresolved signed strut separately from the candidate origin.
    /// Clearance is evaluated at the hypothetical position with clear disabled.
    pub(crate) fn sub_context_for_collapsing_child(
        &mut self,
        offset: f32,
        insets: [f32; 2],
        anchor: f32,
        margins: CollapsibleMarginSet,
        joins_top_strut: bool,
    ) -> BlockContext<'_> {
        let leading_strut =
            if let Some(inherited) = self.state.leading_strut.filter(|_| joins_top_strut) {
                LeadingStrut {
                    anchor: inherited.anchor,
                    margins: inherited.margins.collapse_with_set(margins),
                }
            } else {
                LeadingStrut {
                    anchor: self.origin + anchor,
                    margins,
                }
            };
        let mut child = self.sub_context(offset, insets);
        child.state.leading_strut = Some(leading_strut);
        child
    }

    pub(crate) fn hypothetical_top_position(&self, margins: CollapsibleMarginSet) -> Option<f32> {
        self.state.leading_strut.map(|strut| {
            strut.anchor + strut.margins.collapse_with_set(margins).resolve() - self.origin
        })
    }

    pub(crate) fn snapshot(&self) -> BlockContextSnapshot {
        BlockContextSnapshot {
            floats: Arc::clone(&self.owner.floats),
            state: self.state,
            content_insets: self.content_insets,
        }
    }

    pub(crate) fn restore(&mut self, saved: &BlockContextSnapshot) {
        self.owner.floats = Arc::clone(&saved.floats);
        self.state = saved.state;
        self.content_insets = saved.content_insets;
    }

    pub(crate) fn is_bfc_root(&self) -> bool {
        self.root
    }

    pub(crate) fn origin_resolved(&self) -> bool {
        self.state.resolved
    }

    pub(crate) fn set_origin_resolved(&mut self, resolved: bool) {
        self.state.resolved = resolved;
    }

    pub(crate) fn set_width(&mut self, width: f32) {
        Arc::make_mut(&mut self.owner.floats).set_width(width);
    }

    pub(crate) fn apply_content_box_inset(&mut self, insets: [f32; 2]) {
        self.content_insets = [
            self.border_insets[0] + insets[0],
            self.border_insets[1] + insets[1],
        ];
    }

    pub(crate) fn has_floats(&self) -> bool {
        self.owner.floats.has_floats()
    }

    pub(crate) fn has_active_floats(&self, from: f32) -> bool {
        self.owner.floats.has_active_floats(self.origin + from)
    }

    pub(crate) fn place_floated_box(
        &mut self,
        size: Size<f32>,
        from: f32,
        direction: FloatDirection,
        clear: Clear,
        adjoins_unresolved_strut: bool,
    ) -> Point<f32> {
        if adjoins_unresolved_strut {
            self.state.adjoining[direction as usize] = true;
        }
        let absolute = Arc::make_mut(&mut self.owner.floats).place_floated_box(
            size,
            self.origin + from,
            self.content_insets,
            direction,
            clear,
        );
        let local = Point {
            x: absolute.x - self.border_insets[0],
            y: absolute.y - self.origin,
        };
        self.state.float_end = self.state.float_end.max(local.y + size.height);
        local
    }

    pub(crate) fn find_content_slot(
        &self,
        from: f32,
        clear: Clear,
        after: Option<usize>,
    ) -> ContentSlot {
        // The BFC slot query distinguishes actual floats on each side from
        // the containing-block insets of another block's earlier placement.
        // Zero margins give inline content exactly the space between floats.
        let slot = self.find_bfc_slot(from, [0.0; 2], Direction::Ltr, clear, after);
        ContentSlot {
            segment_id: slot.segment_id,
            x: slot.x,
            y: slot.y,
            width: slot.border_width,
            height: f32::INFINITY,
        }
    }

    pub(crate) fn find_bfc_slot(
        &self,
        from: f32,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        after: Option<usize>,
    ) -> BfcSlot {
        let mut slot = self.owner.floats.find_bfc_slot(
            self.origin + from,
            self.content_insets,
            margins,
            direction,
            clear,
            after,
        );
        slot.x -= self.border_insets[0];
        slot.y -= self.origin;
        slot
    }

    /// Where a box that establishes a formatting context goes among the
    /// floats at or below `from`, as CSS 2.1 9.5 asks: its border box must
    /// not overlap any float, so it is set where it fits in the room the
    /// floats leave along its whole height, and below them where it does
    /// not. `margins` are its resolved side margins; `probe` measures it.
    ///
    /// The box is first tried in the segment the floats leave at its top.
    /// When it is taller than that segment, the room is narrowed to what the
    /// segments below it share. A box of `auto` width is measured again for
    /// the narrower room; a box of fixed width that does not fit moves to
    /// the next segment.
    pub(crate) fn find_box_slot(
        &self,
        from: f32,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        mut probe: impl FnMut(BoxProbe) -> f32,
    ) -> BoxSlot {
        let mut after = None;
        loop {
            let first = self.find_bfc_slot(from, margins, direction, clear, after);
            let width = probe(BoxProbe::Width {
                stretch: first.stretch_width,
            });
            let Some(segment) = first.segment_id else {
                return BoxSlot { slot: first, width };
            };
            if width <= first.border_width + FIT_SLACK
                && let Some(placed) =
                    self.settle_across_height(first, width, margins, direction, clear, &mut probe)
            {
                return placed;
            }
            after = Some(segment);
        }
    }

    /// `first`, the room in the segment where a box of border-box `width`
    /// starts, narrowed to the room every segment the box reaches shares.
    /// `None` when the box does not fit that room.
    fn settle_across_height(
        &self,
        first: BfcSlot,
        width: f32,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        probe: &mut impl FnMut(BoxProbe) -> f32,
    ) -> Option<BoxSlot> {
        // Nothing below the segment: the box needs no height to be placed.
        if self
            .next_segment(first, margins, direction, clear)
            .is_none()
        {
            return Some(BoxSlot { slot: first, width });
        }
        let mut slot = first;
        let mut width = width;
        let mut shifts = [0.0; 4];
        // The room is cut by the segments the box reaches at the width it
        // has, so a box whose height follows its width can win room back when
        // a narrower room makes it shorter. Rooms that come round again mean
        // width and height chase each other; the farthest bottom measured
        // then decides, and the room only narrows from there.
        let mut tried = vec![shifts];
        let mut tallest = f32::NEG_INFINITY;
        let mut cycled = false;
        loop {
            let measured = first.y + probe(BoxProbe::Height { width });
            tallest = tallest.max(measured);
            let mut narrowed = self.narrowed_across(
                first,
                margins,
                direction,
                clear,
                if cycled { tallest } else { measured },
            );
            if !cycled && narrowed != shifts && tried.contains(&narrowed) {
                cycled = true;
                narrowed = self.narrowed_across(first, margins, direction, clear, tallest);
            }
            if narrowed == shifts {
                return Some(BoxSlot { slot, width });
            }
            tried.push(narrowed);
            shifts = narrowed;
            slot = BfcSlot {
                x: first.x + shifts[0],
                border_width: first.border_width - shifts[0] - shifts[1],
                stretch_width: first.stretch_width - shifts[2] - shifts[3],
                ..first
            };
            width = probe(BoxProbe::Width {
                stretch: slot.stretch_width,
            });
            if width > slot.border_width + FIT_SLACK {
                return None;
            }
        }
    }

    /// The segment below the one `slot` starts in, if floats reach into one.
    fn next_segment(
        &self,
        slot: BfcSlot,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
    ) -> Option<BfcSlot> {
        let segment = slot.segment_id?;
        let next = self.find_bfc_slot(slot.y, margins, direction, clear, Some(segment));
        next.segment_id.map(|_| next)
    }

    /// How far the segments that start above `bottom` cut into the room of
    /// `first`: at the start and end edge of the room a border box fits, then
    /// at the start and end edge of the room an `auto` width stretches over.
    /// The two differ where a trailing margin is, which overflows for fit
    /// but not for stretch, so each is intersected on its own.
    fn narrowed_across(
        &self,
        first: BfcSlot,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        bottom: f32,
    ) -> [f32; 4] {
        let reference = room_edges(first, direction);
        let mut shifts = [0.0_f32; 4];
        let mut segment = first;
        while let Some(next) = self.next_segment(segment, margins, direction, clear) {
            if next.y >= bottom - FIT_SLACK {
                break;
            }
            let edges = room_edges(next, direction);
            for (index, shift) in shifts.iter_mut().enumerate() {
                // Starts move in, ends move back.
                let cut = if index % 2 == 0 {
                    edges[index] - reference[index]
                } else {
                    reference[index] - edges[index]
                };
                *shift = shift.max(cut);
            }
            segment = next;
        }
        shifts
    }

    pub(crate) fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.owner
            .floats
            .cleared_threshold(clear)
            .map(|end| end - self.origin)
    }

    /// Notes how low the floats placed so far reach, as seen by `clear`, for
    /// the float about to be placed when it starts a page: the box after it
    /// is held below them but not below that float.
    pub(crate) fn record_page_start_float_floor(&mut self, clear: Clear) {
        self.state.page_start_floor = self.owner.floats.cleared_threshold(clear);
    }

    /// The floor recorded for the last float that started a page.
    pub(crate) fn page_start_float_floor(&self) -> Option<f32> {
        self.state.page_start_floor.map(|floor| floor - self.origin)
    }

    pub(crate) fn has_adjoining_float(&self, clear: Clear) -> bool {
        match clear {
            Clear::None => false,
            Clear::Left => self.state.adjoining[0],
            Clear::Right => self.state.adjoining[1],
            Clear::Both => self.state.adjoining[0] || self.state.adjoining[1],
        }
    }

    pub(crate) fn has_top_adjoining_float(&self, clear: Clear) -> bool {
        let adjoining = self.top_adjoining_floats();
        match clear {
            Clear::None => false,
            Clear::Left => adjoining[0],
            Clear::Right => adjoining[1],
            Clear::Both => adjoining[0] || adjoining[1],
        }
    }

    pub(crate) fn commit_strut(&mut self) {
        self.state
            .leading_adjoining
            .get_or_insert(self.state.adjoining);
        self.state.adjoining = [false; 2];
    }

    pub(crate) fn top_adjoining_floats(&self) -> [bool; 2] {
        self.state.leading_adjoining.unwrap_or(self.state.adjoining)
    }

    pub(crate) fn merge_adjoining_floats(&mut self, flags: [bool; 2]) {
        self.state.adjoining[0] |= flags[0];
        self.state.adjoining[1] |= flags[1];
    }

    pub(crate) fn add_child_floated_content_height_contribution(&mut self, end: f32) {
        self.state.float_end = self.state.float_end.max(end);
    }

    pub(crate) fn floated_content_height_contribution(&self) -> f32 {
        self.state.float_end
    }
}

#[cfg(test)]
mod tests;
