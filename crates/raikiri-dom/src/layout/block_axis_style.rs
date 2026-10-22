//! The facts of a box's style that pagination reads along the axis it breaks
//! a flow on.
//!
//! The paginator breaks a flow along y. A document whose root lays out its
//! blocks on vertical axes is paginated on transposed axes (see
//! [`crate::layout::page_pipeline`] and its private `transposed_flow`
//! module), so the sizes and margins it reads along y are the physical x ones
//! of the style. (The insets it reads come from the computed values of the
//! cascade it is given, which carries them on those axes.)
//! [`BlockAxisStyle`] holds those facts of one box on the axes the paginator
//! reads, and `LayoutSession::block_axis` gives them without anything having
//! to be swapped in the document's own styles: retained styles stay on
//! physical axes while pagination runs.

use taffy::{Dimension, LengthPercentageAuto, Rect};

#[cfg(test)]
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
    pub(crate) margin: Rect<LengthPercentageAuto>,
    /// Inverted on transposed axes.
    pub(crate) aspect_ratio: Option<f32>,
}

impl BlockAxisStyle {
    /// The facts of a box laid out on the axes pagination reads them on.
    #[cfg(test)]
    pub(crate) fn of_retained(style: &crate::LayoutStyle) -> Self {
        Self {
            height: style.size.height,
            min_height: style.min_size.height,
            max_height: style.max_size.height,
            margin: style.margin,
            aspect_ratio: style.aspect_ratio,
        }
    }

    /// The facts a style view gives for the axes of its context.
    pub(crate) fn of_view(view: &raikiri_layout_style::LayoutStyleView<'_>) -> Self {
        use taffy::CoreStyle;
        let size = view.size();
        Self {
            height: size.height,
            min_height: view.min_size().height,
            max_height: view.max_size().height,
            margin: view.margin(),
            aspect_ratio: view.aspect_ratio(),
        }
    }
}

#[cfg(test)]
impl Document {
    /// The style of layout key `key` along the axis pagination breaks the
    /// flow on, from the snapshot of a transposed pagination or else from
    /// the retained style. Tests compare it with what the style views give.
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
