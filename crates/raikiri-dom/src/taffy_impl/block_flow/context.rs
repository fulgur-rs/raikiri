//! Shared block and inline flow state over public float placement.

use std::sync::Arc;
use taffy::{
    BfcSlot, Clear, CollapsibleMarginSet, ContentSlot, Direction, FloatContext, FloatDirection,
    Point, Size,
};

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
}

impl FlowState {
    fn new(adjoining: [bool; 2]) -> Self {
        Self {
            float_end: f32::NEG_INFINITY,
            adjoining,
            leading_adjoining: None,
            resolved: false,
            leading_strut: None,
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

    pub(crate) fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.owner
            .floats
            .cleared_threshold(clear)
            .map(|end| end - self.origin)
    }

    pub(crate) fn has_adjoining_float(&self, clear: Clear) -> bool {
        match clear {
            Clear::None => false,
            Clear::Left => self.state.adjoining[0],
            Clear::Right => self.state.adjoining[1],
            Clear::Both => self.state.adjoining[0] || self.state.adjoining[1],
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
