//! The facts of a box's style that pagination reads along the axis it breaks
//! a flow on.
//!
//! The paginator breaks a flow along y. A document whose root lays out its
//! blocks on vertical axes is paginated on transposed axes (see
//! [`crate::layout::page_pipeline`] and its private `transposed_flow`
//! module), so the sizes, margins and insets it reads along y are the
//! physical x ones of the style. [`BlockAxisStyle`] holds those facts of one
//! box on the axes the paginator reads, and [`Document::block_axis_style`]
//! gives them without anything having to be swapped in the document's own
//! styles: retained styles stay on physical axes while pagination runs.

use taffy::{Dimension, LengthPercentageAuto, Overflow, Point};

use crate::Document;

/// The style of a box along the axis pagination breaks the flow on.
///
/// `height` and the `top` and `bottom` sides are along that axis, `top`
/// being the side the flow starts at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockAxisStyle {
    pub(crate) height: Dimension,
    pub(crate) min_height: LengthPercentageAuto,
    pub(crate) max_height: LengthPercentageAuto,
    pub(crate) margin_top: LengthPercentageAuto,
    pub(crate) margin_bottom: LengthPercentageAuto,
    pub(crate) inset_top: LengthPercentageAuto,
    pub(crate) inset_bottom: LengthPercentageAuto,
    /// Exchanged with the other axis on transposed axes.
    pub(crate) overflow: Point<Overflow>,
    /// Inverted on transposed axes.
    pub(crate) aspect_ratio: Option<f32>,
}

impl BlockAxisStyle {
    /// The facts of a box laid out on the axes pagination reads them on.
    pub(crate) fn of_retained(style: &crate::LayoutStyle) -> Self {
        Self {
            height: style.size.height,
            min_height: style.min_size.height,
            max_height: style.max_size.height,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            inset_top: style.inset.top,
            inset_bottom: style.inset.bottom,
            overflow: style.overflow,
            aspect_ratio: style.aspect_ratio,
        }
    }

    /// The facts a style view gives for the axes of its context.
    pub(crate) fn of_view(view: &raikiri_layout_style::LayoutStyleView<'_>) -> Self {
        use taffy::CoreStyle;
        let size = view.size();
        let margin = view.margin();
        let inset = view.inset();
        Self {
            height: size.height,
            min_height: view.min_size().height,
            max_height: view.max_size().height,
            margin_top: margin.top,
            margin_bottom: margin.bottom,
            inset_top: inset.top,
            inset_bottom: inset.bottom,
            overflow: view.overflow(),
            aspect_ratio: view.aspect_ratio(),
        }
    }
}

impl Document {
    /// The style of layout key `key` along the axis pagination breaks the
    /// flow on.
    ///
    /// While a document is paginated on transposed axes the facts of the
    /// keys it transposed are those on the transposed axes; every other box
    /// has them on its physical axes.
    pub(crate) fn block_axis_style(&self, key: usize) -> BlockAxisStyle {
        self.pagination_block_styles
            .get(key)
            .copied()
            .unwrap_or_else(|| {
                BlockAxisStyle::of_retained(
                    self.layout_style_for_key(key)
                        .expect("a laid-out box has a retained style"),
                )
            })
    }
}
