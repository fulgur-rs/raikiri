//! Collapse metadata for an empty leaf measured on its block-flow axes.

use super::algorithms::compute_leaf_layout;
use crate::layout::protocol::LayoutInput;
use raikiri_layout_geometry::{Edges, Size};
use raikiri_layout_style::{BoxStyle, LeafStyle};

fn swap_sides<T>(sides: Edges<T>) -> Edges<T> {
    Edges {
        left: sides.top,
        right: sides.bottom,
        top: sides.left,
        bottom: sides.right,
    }
}

/// Reuse the leaf's collapse rules on transposed axes, without publishing
/// another geometry result. `inputs` are already transposed, while `style`
/// retains the physical axes and any resolved edges or local sizing patches.
/// Callers restrict this probe to an empty, non-replaced block leaf and keep
/// the physical output's zero block extent as an additional condition.
pub(crate) fn empty_leaf_collapses_on_transposed_axes(
    inputs: LayoutInput,
    style: &BoxStyle,
    resolve_calc: impl Fn(*const (), f32) -> f32,
) -> bool {
    // Margins and insets do not affect leaf sizing; direction only affects
    // the overflow rectangle, which this probe discards.
    let mut style = style.clone();
    std::mem::swap(&mut style.size.width, &mut style.size.height);
    std::mem::swap(&mut style.min_size.width, &mut style.min_size.height);
    std::mem::swap(&mut style.max_size.width, &mut style.max_size.height);
    style.padding = swap_sides(style.padding);
    style.border = swap_sides(style.border);
    std::mem::swap(&mut style.overflow.x, &mut style.overflow.y);
    if let Some(ratio) = style.aspect_ratio {
        let inverse = ratio.recip();
        if !inverse.is_finite() || inverse <= 0.0 {
            return false;
        }
        style.aspect_ratio = Some(inverse);
    }
    compute_leaf_layout(inputs, &LeafStyle::new(&style), resolve_calc, |known, _| {
        Size {
            width: known.width.unwrap_or(0.0),
            height: known.height.unwrap_or(0.0),
        }
    })
    .margins_can_collapse_through
}

#[cfg(test)]
mod tests;
