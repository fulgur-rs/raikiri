//! Resolve the physical box constraints used by project block flow.

use crate::taffy_impl::resolve_calc;
use taffy::{
    BoxSizing, CoreStyle, LayoutInput, MaybeMath, MaybeResolve, Overflow, Rect, ResolveOrZero,
    Size, SizingMode,
};

#[derive(Clone, Copy)]
pub(super) struct Metrics {
    pub padding: Rect<f32>,
    pub border: Rect<f32>,
    pub inset: Rect<f32>,
    pub scrollbar: Size<f32>,
    pub preferred: Size<Option<f32>>,
    pub min: Size<Option<f32>>,
    pub max: Size<Option<f32>>,
    pub percent_height: Option<f32>,
}

impl Metrics {
    pub(super) fn resolve(style: &impl CoreStyle, inputs: LayoutInput) -> Self {
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
        let min = style
            .min_size()
            .maybe_resolve(inputs.parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(style.aspect_ratio())
            .maybe_add(adjustment);
        let max = style
            .max_size()
            .maybe_resolve(inputs.parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(style.aspect_ratio())
            .maybe_add(adjustment);
        let authored = style
            .size()
            .maybe_resolve(inputs.parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(style.aspect_ratio())
            .maybe_add(adjustment);
        let preferred = inputs
            .known_dimensions
            .or(if inputs.sizing_mode == SizingMode::InherentSize {
                authored
            } else {
                Size::NONE
            })
            .maybe_clamp(min, max)
            .maybe_max(padding_border.sum_axes())
            .maybe_apply_aspect_ratio(style.aspect_ratio())
            .maybe_clamp(min, max);
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
            percent_height,
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
