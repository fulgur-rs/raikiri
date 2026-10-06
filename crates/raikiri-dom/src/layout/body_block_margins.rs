//! Block-start margin of `<body>`, the synthetic page root.
//!
//! The body is laid out as the taffy root, sized to the page content box, and
//! taffy ignores the margins of a root. The body's horizontal margins become
//! inline padding of the root ([`super::page::move_body_inline_margins_into_padding`]);
//! this module places its block-start margin, which unlike the horizontal
//! ones can collapse with the top margin of its first in-flow child
//! (CSS 2.1 §8.3.1). The used offset is recorded for
//! [`Document::body_block_start_margin`].
//!
//! The `<html>` margin is not part of this offset: margins of the root
//! element's box do not collapse (CSS 2.1 §8.3.1), so it is added outside the
//! body by the paginator and by consumers that place the body on the page.

use super::*;
use taffy::util::{MaybeMath, MaybeResolve, ResolveOrZero};
use taffy::{CollapsibleMarginSet, CoreStyle, TraversePartialTree};

/// Used block-start margin of `<body>` against the page content width
/// `content_width` (percentages resolve against the containing block's inline
/// size, CSS 2.1 §8.3). `auto` is 0 (§10.6.3). Negative values are kept.
pub(crate) fn body_margin_top(
    doc: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    content_width: f32,
) -> f32 {
    used_style_length_percentage_auto(doc.nodes[body_id].style.margin.top, content_width)
        .or_else(|| {
            used_computed_length_percentage_or_auto(
                cascade.computed[body_id].margin.top,
                content_width,
            )
        })
        .unwrap_or(0.0)
}

/// CSS Overflow 3 §3.3: when the root element's `overflow` is `visible`, the
/// body's `overflow` applies to the viewport instead and the body's used
/// value is `visible`. Give the body root that used value, so a body with
/// `overflow: hidden` does not become a scroll container that keeps its first
/// child's margin from collapsing with its own.
pub(crate) fn propagate_body_overflow_to_viewport(
    doc: &mut Document,
    cascade: &CascadeResult,
    body_id: usize,
) {
    let Some(html_id) = doc.parent_of(body_id) else {
        return; // cov:ignore: layout finds the body below the document root, so it has a parent.
    };
    if doc.nodes[html_id].tag_name() != Some("html") {
        return;
    }
    let html = cascade.computed[html_id].overflow;
    if html.x == OverflowValue::Visible && html.y == OverflowValue::Visible {
        doc.nodes[body_id].style.overflow = Point {
            x: TaffyOverflow::Visible,
            y: TaffyOverflow::Visible,
        };
    }
}

/// Whether the body's block-start margin may collapse with the top margin of
/// its first in-flow child (CSS 2.1 §8.3.1): the body is a block container
/// in its parent's formatting context, not a flow root, float or absolutely
/// positioned box, and its content is not inline content laid out as one
/// paragraph. Taffy additionally rules out a body with top padding or border,
/// a scroll container and other boxes that establish a new block formatting
/// context.
pub(crate) fn body_top_margin_may_collapse(
    doc: &Document,
    cascade: &CascadeResult,
    body_id: usize,
) -> bool {
    let computed = &cascade.computed[body_id];
    let style = &doc.nodes[body_id].style;
    !doc.nodes[body_id].is_ifc_root()
        && matches!(
            computed.display,
            DisplayValue::Block | DisplayValue::ListItem
        )
        && style.display == Display::Block
        && style.float == TaffyFloat::None
        && style.position != TaffyPosition::Absolute
}

/// The body root's top padding and height before its block-start margin is
/// carried as extra top padding (see [`Self::apply`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct BodyTopPadding {
    padding_top: f32,
    height: Option<f32>,
}

impl BodyTopPadding {
    /// Record the root's resolved top padding and, with `box-sizing:
    /// content-box`, its definite height.
    pub(crate) fn capture(doc: &Document, body_id: usize, content_width: f32) -> Self {
        let style = &doc.nodes[body_id].style;
        Self {
            padding_top: used_style_length_percentage(style.padding.top, content_width)
                .unwrap_or(0.0)
                .max(0.0),
            height: (style.box_sizing == TaffyBoxSizing::ContentBox)
                .then(|| style_dimension_length(style.size.height))
                .flatten(),
        }
    }

    /// Carry `offset` of the body's block-start margin as extra top padding
    /// of the root, for a body whose content is one paragraph laid out by the
    /// inline engine: its lines, and the boxes among them, then start below
    /// it. As for the horizontal margins, a negative offset is clamped to 0
    /// because padding cannot be negative, and with `box-sizing: content-box`
    /// the root's height shrinks by the offset so its border box keeps its
    /// height. Returns the offset applied.
    pub(crate) fn apply(self, doc: &mut Document, body_id: usize, offset: f32) -> f32 {
        let offset = offset.max(0.0);
        let style = &mut doc.nodes[body_id].style;
        style.padding.top = LengthPercentage::length(self.padding_top + offset);
        if let Some(height) = self.height {
            style.size.height = Dimension::length((height - offset).max(0.0));
        }
        offset
    }
}

/// The collapsed top margins of a block box that starts the paragraph of a
/// body laid out by the inline engine, with no line box, float or other block
/// above it. Nothing separates them from the body's own top margin, so they
/// collapse with it (CSS 2.1 §8.3.1); the inline engine keeps them inside the
/// paragraph.
pub(crate) fn body_paragraph_leading_margin(
    doc: &Document,
    body_id: usize,
) -> Option<CollapsibleMarginSet> {
    doc.nodes[body_id]
        .ifc
        .as_ref()?
        .lines
        .as_ref()?
        .leading_block_margin
}

/// Lay out the body as the root of the page, as `taffy::compute_root_layout`
/// does, but let the top margins of its first in-flow children escape the
/// root when `collapse_top` is set, and return the margins that escaped.
///
/// `taffy::compute_root_layout` lays the root out with non-collapsible
/// margins, so the first child's top margin stays inside the root and the
/// body's own top margin could only be added to it, never collapsed with it.
/// With `collapse_top`, taffy's block algorithm places the first child at the
/// root's content top (unless the root's padding, border or formatting
/// context keep the margins in) and reports the collapsed margins of the
/// first child chain, positive and negative ones apart, so the caller can
/// collapse them with the body's own margin.
pub(crate) fn compute_body_root_layout(
    tree: &mut Document,
    root: TaffyNodeId,
    available_space: Size<AvailableSpace>,
    collapse_top: bool,
) -> CollapsibleMarginSet {
    let parent_size = available_space.into_options();
    let calc = |val, basis| crate::taffy_impl::resolve_calc(val, basis);
    let mut known_dimensions = Size::NONE;
    {
        let style = tree.get_core_container_style(root);
        if style.is_block() {
            let aspect_ratio = style.aspect_ratio();
            let margin = style.margin().resolve_or_zero(parent_size.width, calc);
            let padding = style.padding().resolve_or_zero(parent_size.width, calc);
            let border = style.border().resolve_or_zero(parent_size.width, calc);
            let padding_border_size = (padding + border).sum_axes();
            let box_sizing_adjustment = if style.box_sizing() == TaffyBoxSizing::ContentBox {
                padding_border_size
            } else {
                Size::ZERO
            };
            let min_size = style
                .min_size()
                .maybe_resolve(parent_size, calc)
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment);
            let max_size = style
                .max_size()
                .maybe_resolve(parent_size, calc)
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment);
            let clamped_style_size = style
                .size()
                .maybe_resolve(parent_size, calc)
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment)
                .maybe_clamp(min_size, max_size);
            // Equal or crossed min and max sizes fix the size in that axis.
            let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
                (Some(min), Some(max)) if max <= min => Some(min),
                _ => None,
            });
            // A block stretches to fill a definite available width.
            let available_space_based_size = Size {
                width: available_space
                    .width
                    .into_option()
                    .maybe_sub(margin.horizontal_axis_sum()),
                height: None,
            };
            known_dimensions = known_dimensions
                .or(min_max_definite_size)
                .or(clamped_style_size)
                .or(available_space_based_size)
                .maybe_max(padding_border_size);
        }
    }

    let output = tree.compute_child_layout(
        root,
        LayoutInput {
            known_dimensions,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size,
            available_space,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            run_mode: RunMode::PerformLayout,
            vertical_margins_are_collapsible: TaffyLine {
                start: collapse_top,
                end: false,
            },
        },
    );

    let style = tree.get_core_container_style(root);
    let width = available_space.width.into_option();
    let padding = style.padding().resolve_or_zero(width, calc);
    let border = style.border().resolve_or_zero(width, calc);
    let margin = style.margin().resolve_or_zero(width, calc);
    let scrollbar_size = Size {
        width: if style.overflow().y == TaffyOverflow::Scroll {
            style.scrollbar_width()
        } else {
            0.0
        },
        height: if style.overflow().x == TaffyOverflow::Scroll {
            style.scrollbar_width()
        } else {
            0.0
        },
    };
    let location = Point {
        x: if style.direction() == TaffyDirection::Rtl {
            width.map_or(0.0, |width| width - output.size.width)
        } else {
            0.0
        },
        y: 0.0,
    };
    tree.set_unrounded_layout(
        root,
        &TaffyLayout {
            order: 0,
            location,
            size: output.size,
            scrollable_overflow_rect: output.scrollable_overflow_rect,
            scrollbar_size,
            padding,
            border,
            margin,
        },
    );
    if collapse_top {
        output.top_margin
    } else {
        CollapsibleMarginSet::ZERO
    }
}

/// Move the body's laid-out content down by `offset`, the body's used
/// block-start margin, so that the margin lies inside the synthetic root as
/// its horizontal margins do. Every child box moves: in-flow and floated
/// boxes, and absolutely positioned boxes placed at their static position.
/// An absolutely positioned box with a `top` or `bottom` inset is placed
/// against the containing block, which the root stands in for, and stays.
pub(crate) fn shift_body_content(doc: &mut Document, body_id: usize, offset: f32) {
    if offset == 0.0 || !offset.is_finite() {
        return;
    }
    let children: Vec<usize> = doc
        .child_ids(TaffyNodeId::from(body_id))
        .map(usize::from)
        .collect();
    for child in children {
        let style = &doc.nodes[child].style;
        if style.position == TaffyPosition::Absolute
            && !(style.inset.top.is_auto() && style.inset.bottom.is_auto())
        {
            continue;
        }
        doc.nodes[child].unrounded_layout.location.y += offset;
    }
}

#[cfg(test)]
mod tests;
