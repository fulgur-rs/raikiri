//! The box facts the auto-width projection reads from computed values.
//!
//! The projection runs while the document's styles are still being prepared,
//! and again when paragraphs are re-broken outside a layout session, so it
//! cannot query style views. These accessors give the values that the style
//! bridge, and through it the views, take from the same computed values.

use super::*;
use crate::layout::sanitize::sanitize_taffy;
use raikiri_layout_style::AlignedCalc;
use raikiri_style::property::{BoxSizing, CalcLengthPercentage};

#[derive(Clone, Copy)]
pub(super) struct AuthoredBox<'a> {
    doc: &'a Document,
    cascade: &'a CascadeResult,
    id: usize,
}

impl<'a> AuthoredBox<'a> {
    pub(super) fn new(doc: &'a Document, cascade: &'a CascadeResult, id: usize) -> Self {
        Self { doc, cascade, id }
    }

    fn values(&self) -> &'a ComputedValues {
        &self.cascade.computed[self.id]
    }

    /// The width and height that layout reads: the logical sizes of a vertical
    /// box, and `auto` for a sizing keyword that the box is not sized by.
    fn width_and_height(
        &self,
    ) -> (
        ComputedLengthPercentageOrAuto,
        ComputedLengthPercentageOrAuto,
    ) {
        use super::super::bridge::is_sizing_keyword;
        use raikiri_layout_style::StyleFlags as F;
        let cv = self.values();
        let (mut width, mut height) =
            super::super::bridge::vertical_logical_size(self.cascade, self.id)
                .map_or((cv.width, cv.height), |size| (size.width, size.height));
        if is_sizing_keyword(width) || is_sizing_keyword(height) {
            let flags =
                super::super::style_views::metadata::sizing_flags(self.doc, self.cascade, self.id);
            if (flags.contains(F::IGNORE_WIDTH_KEYWORD) && is_sizing_keyword(width))
                || (flags.contains(F::MIN_CONTENT_WIDTH_AUTO)
                    && width == ComputedLengthPercentageOrAuto::MinContent)
            {
                width = ComputedLengthPercentageOrAuto::Auto;
            }
            if flags.contains(F::IGNORE_HEIGHT_KEYWORD) && is_sizing_keyword(height) {
                height = ComputedLengthPercentageOrAuto::Auto;
            }
        }
        (width, height)
    }

    /// Whether the box is sized as `border-box`. A content-box with an `auto`
    /// width and side borders but no block-axis padding or border is sized as
    /// a border box too, as the bridge does.
    pub(super) fn is_border_box(&self) -> bool {
        use ComputedLengthPercentageOrCalc as P;
        let cv = self.values();
        if cv.box_sizing == BoxSizing::BorderBox {
            return true;
        }
        let (width, _) = self.width_and_height();
        let side_borders = cv.border.left.width().px() > 0.0 || cv.border.right.width().px() > 0.0;
        let vertical_clean = cv.border.top.width().px() == 0.0
            && cv.border.bottom.width().px() == 0.0
            && matches!(cv.padding.top, P::Px(0.0) | P::Percent(0.0))
            && matches!(cv.padding.bottom, P::Px(0.0) | P::Percent(0.0));
        side_borders && vertical_clean && matches!(width, ComputedLengthPercentageOrAuto::Auto)
    }

    /// The left and right padding and border, resolved against `basis`.
    pub(super) fn horizontal_edges(&self, basis: f32) -> f32 {
        let cv = self.values();
        let padding = |value| padding_edge(value, basis);
        let border = |width: raikiri_style::ComputedLength| {
            inset(sanitize_taffy(width.px(), "border-width", &mut Vec::new()))
        };
        padding(cv.padding.left)
            + padding(cv.padding.right)
            + border(cv.border.left.width())
            + border(cv.border.right.width())
    }

    /// The definite `min-width` against `basis`. A sizing keyword is no
    /// constraint: layout takes it from the box's content limits instead.
    pub(super) fn min_width(&self, basis: f32) -> Option<f32> {
        definite_size(self.values().min_width, basis)
    }

    /// The definite `max-width` against `basis`.
    pub(super) fn max_width(&self, basis: f32) -> Option<f32> {
        definite_size(self.values().max_width, basis)
    }

    /// Whether the box floats. Only `left` and `right` float in the style
    /// bridge, so `inline-start`, `inline-end` and `footnote` do not.
    pub(super) fn is_floated(&self) -> bool {
        matches!(self.values().float, FloatValue::Left | FloatValue::Right)
    }

    pub(super) fn is_replaced(&self) -> bool {
        super::super::style_views::metadata::is_replaced(self.doc, self.cascade, self.id)
    }

    /// Whether the box has a preferred aspect ratio: its own `aspect-ratio`,
    /// or the natural ratio of its content. Size containment removes the
    /// natural ratio.
    pub(super) fn has_preferred_aspect_ratio(&self) -> bool {
        let node = &self.doc.nodes[self.id];
        let containment = node.size_containment;
        if containment.width || containment.height {
            let (width, height) = self.width_and_height();
            return raikiri_layout_style::preferred_aspect_ratio(
                self.values().aspect_ratio,
                width,
                height,
                None,
                containment.width,
            )
            .is_some();
        }
        node.preferred_aspect_ratio.is_some()
    }
}

fn inset(resolved: f32) -> f32 {
    if resolved.is_finite() {
        resolved.max(0.0)
    } else {
        0.0
    }
}

fn padding_edge(value: ComputedLengthPercentageOrCalc, basis: f32) -> f32 {
    inset(match value {
        ComputedLengthPercentageOrCalc::Px(px) => sanitize_taffy(px, "padding", &mut Vec::new()),
        ComputedLengthPercentageOrCalc::Percent(percent) => {
            basis * sanitize_taffy(percent / 100.0, "padding", &mut Vec::new())
        }
        ComputedLengthPercentageOrCalc::Calc(calc) => resolve_calc(calc, basis),
    })
}

fn definite_size(value: ComputedLengthPercentageOrAuto, basis: f32) -> Option<f32> {
    let resolved = match value {
        ComputedLengthPercentageOrAuto::Px(px) => sanitize_taffy(px, "size", &mut Vec::new()),
        ComputedLengthPercentageOrAuto::Percent(percent) => {
            basis * sanitize_taffy(percent / 100.0, "size", &mut Vec::new())
        }
        ComputedLengthPercentageOrAuto::Calc(calc) => resolve_calc(calc, basis),
        ComputedLengthPercentageOrAuto::Auto
        | ComputedLengthPercentageOrAuto::MinContent
        | ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_) => return None,
    };
    resolved.is_finite().then_some(resolved.max(0.0))
}

fn resolve_calc(calc: CalcLengthPercentage, basis: f32) -> f32 {
    AlignedCalc::resolve_expression(calc, true, basis)
}
