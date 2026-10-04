//! Logical and physical axes used by ordinary IFC layout.

use shodo::geometry::WritingMode;
use shodo::geometry::{Direction, LogicalRect, PhysicalConverter, PhysicalRect, PhysicalSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IfcAxes {
    writing_mode: WritingMode,
    direction: Direction,
}

impl IfcAxes {
    pub(crate) fn new(writing_mode: WritingMode, direction: Direction) -> Self {
        Self {
            writing_mode,
            direction,
        }
    }

    pub(crate) fn writing_mode(self) -> WritingMode {
        self.writing_mode
    }

    pub(crate) fn direction(self) -> Direction {
        self.direction
    }

    pub(crate) fn inline_extent(self, size: PhysicalSize) -> f32 {
        if self.writing_mode == WritingMode::HorizontalTb {
            size.width
        } else {
            size.height
        }
    }

    pub(crate) fn block_extent(self, size: PhysicalSize) -> f32 {
        if self.writing_mode == WritingMode::HorizontalTb {
            size.height
        } else {
            size.width
        }
    }

    pub(crate) fn rect(self, size: PhysicalSize, rect: LogicalRect) -> PhysicalRect {
        self.converter(size).rect(rect)
    }

    fn converter(self, size: PhysicalSize) -> PhysicalConverter {
        PhysicalConverter::new(self.writing_mode, self.direction, size)
    }
}

#[cfg(test)]
mod tests;
