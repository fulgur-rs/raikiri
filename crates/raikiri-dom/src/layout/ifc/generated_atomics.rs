//! Atomic inline-level `::before` and `::after` boxes without content.
//!
//! An `inline-block` pseudo-element whose `content` generates nothing is
//! still a box (CSS Content 3 §2, CSS Display 3 §2): an empty atomic inline
//! sized by its own `width` and `height`, padding, border and margins. It has
//! no in-flow line box, so its baseline is its bottom margin edge (CSS 2.1
//! 10.8.1).

use super::generated_blocks::length_or_auto;
use raikiri_style::property::{BoxSizing, Sides};
use raikiri_style::{
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc, ComputedValues,
};
use shodo::geometry::WritingMode;

/// The box geometry of one empty atomic `::before` or `::after`, as
/// computed: percentages resolve when the paragraph is laid out, against its
/// width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneratedAtomic {
    /// The inline engine's id of the pseudo-element.
    pub(crate) id: u64,
    margin: Sides<ComputedLengthPercentageOrAuto>,
    padding: Sides<ComputedLengthPercentageOrCalc>,
    /// Border widths in px, by side.
    border: Sides<f32>,
    width: [ComputedLengthPercentageOrAuto; 3],
    height: [ComputedLengthPercentageOrAuto; 3],
    border_box: bool,
}

impl GeneratedAtomic {
    /// The geometry of the pseudo-element styled by `cv`, whose inline
    /// engine's id is `id`.
    pub(crate) fn new(id: u64, cv: &ComputedValues) -> Self {
        Self {
            id,
            margin: cv.margin,
            padding: cv.padding,
            border: Sides {
                top: cv.border.top.width().px(),
                right: cv.border.right.width().px(),
                bottom: cv.border.bottom.width().px(),
                left: cv.border.left.width().px(),
            },
            width: [cv.width, cv.min_width, cv.max_width],
            height: [cv.height, cv.min_height, cv.max_height],
            border_box: cv.box_sizing == BoxSizing::BorderBox,
        }
    }

    /// Padding plus border in px on each physical side. Percentages refer to
    /// the paragraph's width `basis` and count as zero without one.
    pub(crate) fn edges(&self, basis: Option<f32>) -> Sides<f32> {
        let padding = |value: ComputedLengthPercentageOrCalc| value.resolve(basis.unwrap_or(0.0));
        Sides {
            top: padding(self.padding.top) + self.border.top,
            right: padding(self.padding.right) + self.border.right,
            bottom: padding(self.padding.bottom) + self.border.bottom,
            left: padding(self.padding.left) + self.border.left,
        }
    }

    /// The atomic size of the box in a paragraph of `writing_mode` whose
    /// inline size is `basis`, or not known yet for `None`, and whose lines
    /// run right to left when `rtl`. A percentage width or height counts
    /// against the definite physical width or height of the containing block
    /// in `containing`, and is `auto` without one (CSS 2.1 10.2, 10.5).
    /// Without content an `auto` size is zero.
    pub(crate) fn size(
        &self,
        basis: Option<f32>,
        containing: (Option<f32>, Option<f32>),
        writing_mode: WritingMode,
        rtl: bool,
    ) -> shodo::AtomicSize {
        let edges = self.edges(basis);
        let width = self.used_size(self.width, edges.left + edges.right, containing.0);
        let height = self.used_size(self.height, edges.top + edges.bottom, containing.1);
        let margin = |value| length_or_auto(value, Some(basis.unwrap_or(0.0))).unwrap_or(0.0);
        let (top, right, bottom, left) = (
            margin(self.margin.top),
            margin(self.margin.right),
            margin(self.margin.bottom),
            margin(self.margin.left),
        );
        // The physical sides at the line-left and block-start ends, as for the
        // edges of an inline box (CSS Writing Modes 4 §6.4). Sideways-lr runs
        // its lines from bottom to top.
        let (inline_size, block_size, line_left, block) = match writing_mode {
            WritingMode::HorizontalTb => (width, height, (left, right), (top, bottom)),
            WritingMode::VerticalLr => (height, width, (top, bottom), (left, right)),
            WritingMode::SidewaysLr => (height, width, (bottom, top), (left, right)),
            WritingMode::VerticalRl | WritingMode::SidewaysRl => {
                (height, width, (top, bottom), (right, left))
            }
        };
        let (inline_start, inline_end) = if rtl {
            (line_left.1, line_left.0)
        } else {
            line_left
        };
        shodo::AtomicSize {
            inline_size,
            block_size,
            baseline: None,
            margins: shodo::node::Sides {
                inline_start,
                inline_end,
                block_start: block.0,
                block_end: block.1,
            },
        }
    }

    /// The border-box size along one axis, from that axis's `[size, min,
    /// max]` properties and its `edges` of padding and border.
    fn used_size(
        &self,
        [size, min_size, max_size]: [ComputedLengthPercentageOrAuto; 3],
        edges: f32,
        basis: Option<f32>,
    ) -> f32 {
        let content_box = |value| {
            length_or_auto(value, basis).map(|size: f32| {
                if self.border_box {
                    (size - edges).max(0.0)
                } else {
                    size
                }
            })
        };
        let mut used = content_box(size).unwrap_or(0.0);
        if let Some(max) = content_box(max_size) {
            used = used.min(max);
        }
        if let Some(min) = content_box(min_size) {
            used = used.max(min);
        }
        used.max(0.0) + edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_style::ComputedLengthPercentageOrAuto::Px;

    /// Each physical margin lands on the logical side it faces in the line.
    #[test]
    fn margins_map_to_the_logical_sides_of_the_line() {
        let mut cv = ComputedValues::initial();
        cv.margin = Sides {
            top: Px(1.0),
            right: Px(2.0),
            bottom: Px(3.0),
            left: Px(4.0),
        };
        let generated = GeneratedAtomic::new(0, &cv);
        let sides = |writing_mode, rtl| {
            let margins = generated
                .size(None, (None, None), writing_mode, rtl)
                .margins;
            (
                margins.inline_start,
                margins.inline_end,
                margins.block_start,
                margins.block_end,
            )
        };
        assert_eq!(
            sides(WritingMode::HorizontalTb, false),
            (4.0, 2.0, 1.0, 3.0)
        );
        assert_eq!(sides(WritingMode::HorizontalTb, true), (2.0, 4.0, 1.0, 3.0));
        assert_eq!(sides(WritingMode::VerticalRl, false), (1.0, 3.0, 2.0, 4.0));
        assert_eq!(sides(WritingMode::SidewaysRl, true), (3.0, 1.0, 2.0, 4.0));
        assert_eq!(sides(WritingMode::VerticalLr, false), (1.0, 3.0, 4.0, 2.0));
        assert_eq!(sides(WritingMode::SidewaysLr, false), (3.0, 1.0, 4.0, 2.0));
    }
}
