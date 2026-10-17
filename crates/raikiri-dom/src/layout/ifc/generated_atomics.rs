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
    /// inline size is `basis`, or not known yet for `None`, and whose
    /// definite content-box height is `definite_height`. Without content an
    /// `auto` size is zero. A percentage width counts only against the
    /// width of a horizontal paragraph; a percentage height counts against
    /// the definite height of a horizontal paragraph, or against the inline
    /// size of a vertical one, which is its height (CSS 2.1 10.5).
    pub(crate) fn size(
        &self,
        basis: Option<f32>,
        definite_height: Option<f32>,
        writing_mode: WritingMode,
    ) -> shodo::AtomicSize {
        let horizontal = writing_mode == WritingMode::HorizontalTb;
        let edges = self.edges(basis);
        let width = self.used_size(
            self.width,
            edges.left + edges.right,
            basis.filter(|_| horizontal),
        );
        let height_basis = if horizontal { definite_height } else { basis };
        let height = self.used_size(self.height, edges.top + edges.bottom, height_basis);
        let margin = |value| length_or_auto(value, Some(basis.unwrap_or(0.0))).unwrap_or(0.0);
        let (top, right, bottom, left) = (
            margin(self.margin.top),
            margin(self.margin.right),
            margin(self.margin.bottom),
            margin(self.margin.left),
        );
        if horizontal {
            shodo::AtomicSize {
                inline_size: width,
                block_size: height,
                baseline: None,
                margins: shodo::node::Sides {
                    inline_start: left,
                    inline_end: right,
                    block_start: top,
                    block_end: bottom,
                },
            }
        } else {
            shodo::AtomicSize {
                inline_size: height,
                block_size: width,
                baseline: None,
                margins: shodo::node::Sides {
                    inline_start: top,
                    inline_end: bottom,
                    block_start: left,
                    block_end: right,
                },
            }
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
