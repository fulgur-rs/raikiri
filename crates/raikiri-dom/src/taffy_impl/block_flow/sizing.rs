//! Resolve the physical box constraints used by project block flow.

use crate::taffy_impl::resolve_calc;
use taffy::{
    BoxSizing, CoreStyle, LayoutInput, MaybeMath, MaybeResolve, Overflow, Rect, ResolveOrZero,
    Size, SizingMode, Style,
};

#[derive(Clone, Copy)]
pub(crate) struct Metrics {
    pub padding: Rect<f32>,
    pub border: Rect<f32>,
    pub inset: Rect<f32>,
    pub scrollbar: Size<f32>,
    pub preferred: Size<Option<f32>>,
    pub min: Size<Option<f32>>,
    pub max: Size<Option<f32>>,
    /// The maximum size the box states itself, without the part carried over
    /// from the other axis through the aspect ratio.
    pub own_max: Size<Option<f32>>,
    pub percent_height: Option<f32>,
    /// The preferred height comes only from `aspect-ratio`, so the content may
    /// raise it: an automatic `min-height` is the height of the content.
    pub ratio_height: bool,
    /// As `ratio_height`, for a width the box takes from its height.
    pub ratio_width: bool,
}

/// The minimum and maximum size of one axis from the ones the box states,
/// `[min, max]`, and the ones carried over from the other axis through the
/// aspect ratio. A carried minimum never overrides a maximum the box states,
/// nor a carried maximum a minimum it states.
fn bounds(own: [Option<f32>; 2], carried: [Option<f32>; 2]) -> (Option<f32>, Option<f32>) {
    let [own_min, own_max] = own;
    let carried_min = carried[0].map(|size| own_max.map_or(size, |max| size.min(max)));
    let carried_max = carried[1].map(|size| own_min.map_or(size, |min| size.max(min)));
    // A bound is only carried into an axis that states none of its own, so at
    // most one of each pair is set.
    let min = own_min.or(carried_min);
    let max = own_max.or(carried_max);
    (min, max)
}

impl Metrics {
    /// `replaced` is whether the box is a replaced element. `content_box_ratio`
    /// is whether its `aspect-ratio` is an `auto <ratio>`, which relates the
    /// content box sizes whatever the `box-sizing` is.
    pub(crate) fn resolve(
        style: &Style,
        replaced: bool,
        content_box_ratio: bool,
        inputs: LayoutInput,
    ) -> Self {
        let padding = style
            .padding()
            .resolve_or_zero(inputs.parent_size.width, resolve_calc);
        let border = style
            .border()
            .resolve_or_zero(inputs.parent_size.width, resolve_calc);
        let padding_border = padding + border;
        let adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border.sum_axes()
        } else {
            Size::ZERO
        };
        let specified = style.size().maybe_resolve(inputs.parent_size, resolve_calc);
        let ratio = style.aspect_ratio();
        // The box the ratio relates is the content box of a box sized by its
        // content box and of an `auto <ratio>`, else the border box.
        let ratio_edges = if style.box_sizing() == BoxSizing::ContentBox || content_box_ratio {
            padding_border.sum_axes()
        } else {
            Size::ZERO
        };
        // The other size of a size stated in the box `box-sizing` names.
        let across_ratio = |size: Size<Option<f32>>| {
            if style.box_sizing() == BoxSizing::ContentBox || !content_box_ratio {
                size.maybe_apply_aspect_ratio(ratio)
            } else {
                size.maybe_max(ratio_edges)
                    .maybe_sub(ratio_edges)
                    .maybe_apply_aspect_ratio(ratio)
                    .maybe_add(ratio_edges)
            }
        };
        // A minimum or maximum carries across the aspect ratio only into an
        // axis whose preferred size is `auto`. Under `ContentSize` the
        // authored size is ignored, so every axis is `auto`. Replaced content
        // always carries them: an `auto` size of an inline `<svg>` is stood
        // in for by a percentage.
        let carried = |own: Size<Option<f32>>| {
            let across = across_ratio(own);
            let auto = |preferred: Option<f32>| {
                replaced || inputs.sizing_mode != SizingMode::InherentSize || preferred.is_none()
            };
            Size {
                width: across
                    .width
                    .filter(|_| own.width.is_none() && auto(specified.width)),
                height: across
                    .height
                    .filter(|_| own.height.is_none() && auto(specified.height)),
            }
        };
        let own_min = style
            .min_size()
            .maybe_resolve(inputs.parent_size, resolve_calc);
        let own_max = style
            .max_size()
            .maybe_resolve(inputs.parent_size, resolve_calc);
        let carried_min = carried(own_min);
        let carried_max = carried(own_max);
        let (min_width, max_width) = bounds(
            [own_min.width, own_max.width],
            [carried_min.width, carried_max.width],
        );
        let (min_height, max_height) = bounds(
            [own_min.height, own_max.height],
            [carried_min.height, carried_max.height],
        );
        let min = Size {
            width: min_width,
            height: min_height,
        }
        .maybe_add(adjustment);
        let max = Size {
            width: max_width,
            height: max_height,
        }
        .maybe_add(adjustment);
        let own_max = own_max.maybe_add(adjustment);
        let authored = across_ratio(specified).maybe_add(adjustment);
        // Whether the content may raise an axis that the aspect ratio gives:
        // unless the box scrolls or `min-*` says otherwise, an automatic
        // minimum keeps it from ending up smaller than its content.
        let ratio_eligible = |specified: Option<f32>, min_is_auto: bool, overflow: Overflow| {
            style.aspect_ratio().is_some()
                && (inputs.sizing_mode != SizingMode::InherentSize || specified.is_none())
                && min_is_auto
                && !overflow.is_scroll_container()
        };
        let width_eligible = ratio_eligible(
            specified.width,
            style.min_size().width.is_auto(),
            style.overflow().x,
        );
        let height_eligible = ratio_eligible(
            specified.height,
            style.min_size().height.is_auto(),
            style.overflow().y,
        );
        // A size the parent imposed on such an axis was settled with that
        // minimum in mind, so only the maximum the box states caps it, not
        // one carried over from the other axis.
        let limit = Size {
            width: if inputs.known_dimensions.width.is_some() && width_eligible {
                own_max.width
            } else {
                max.width
            },
            height: if inputs.known_dimensions.height.is_some() && height_eligible {
                own_max.height
            } else {
                max.height
            },
        };
        let preferred = inputs
            .known_dimensions
            .or(if inputs.sizing_mode == SizingMode::InherentSize {
                authored
            } else {
                Size::NONE
            })
            .maybe_clamp(min, limit)
            .maybe_max(padding_border.sum_axes())
            .maybe_sub(ratio_edges)
            .maybe_apply_aspect_ratio(style.aspect_ratio())
            .maybe_add(ratio_edges)
            .maybe_clamp(min, limit);
        // An axis is ratio-dependent when its size is not authored or imposed
        // but follows from the other axis through the preferred aspect ratio.
        let ratio_width =
            width_eligible && inputs.known_dimensions.width.is_none() && preferred.width.is_some();
        let ratio_height = height_eligible
            && inputs.known_dimensions.height.is_none()
            && preferred.height.is_some();
        let gutter = |overflow| {
            if overflow == Overflow::Scroll {
                style.scrollbar_width()
            } else {
                0.0
            }
        };
        let scrollbar = Size {
            width: gutter(style.overflow().y),
            height: gutter(style.overflow().x),
        };
        let mut inset = padding_border;
        inset.right += scrollbar.width;
        inset.bottom += scrollbar.height;
        let percent_height = inputs
            .known_dimensions
            .height
            .filter(|_| inputs.known_dimensions_are_definite.height)
            .or_else(|| {
                (inputs.sizing_mode == SizingMode::InherentSize)
                    .then_some(authored.height)
                    .flatten()
            })
            .map(|height| {
                (height.maybe_clamp(min.height, max.height) - inset.vertical_axis_sum()).max(0.0)
            });
        Self {
            padding,
            border,
            inset,
            scrollbar,
            preferred,
            min,
            max,
            own_max,
            percent_height,
            ratio_height,
            ratio_width,
        }
    }

    pub(super) fn clamp_width(self, width: f32) -> f32 {
        width
            .maybe_clamp(self.min.width, self.max.width)
            .max((self.padding + self.border).horizontal_axis_sum())
    }

    pub(super) fn clamp_height(self, height: f32) -> f32 {
        height
            .maybe_clamp(self.min.height, self.max.height)
            .max((self.padding + self.border).vertical_axis_sum())
    }
}
