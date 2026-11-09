//! The quirks mode fill quirks: the root element fills the initial containing
//! block, and the body fills the root element.
//!
//! In quirks mode, a root element or body whose `height` is `auto` is at least
//! as tall as the box around it, less its own margins
//! (<https://quirks.spec.whatwg.org/#the-html-element-fills-the-viewport-quirk>,
//! <https://quirks.spec.whatwg.org/#the-body-element-fills-the-html-element-quirk>).
//! As Chromium does, the filled height is also the height its children
//! resolve percentages and `stretch` against, even when its content makes it
//! taller. On a page, the box around the root element is the page area.
//!
//! Chromium leaves out a box that is out of flow, floated or inline-level.
//! A box with a vertical writing mode, or with a sizing keyword for its
//! `min-height` or `max-height`, keeps its usual height here.

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, SizingMode};
use raikiri_layout_geometry::MaybeMath;
use raikiri_layout_style::values::{ExpandedDimension, MaybeResolve, ResolveOrZero};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::property::{BoxSizing, DisplayValue, FloatValue, PositionValue};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto, ComputedValues};
use raikiri_traits::QuirksMode;

/// Record the fill of the root element and of the body for this layout pass:
/// a `min-height` for each, and the height their children resolve
/// percentages against.
///
/// `page_width` and `page_height` are the size of the page area, the initial
/// containing block.
pub(super) fn prepare(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    body: usize,
    page_width: f32,
    page_height: f32,
) {
    doc.quirks_percentage_height_bases.clear();
    if doc.quirks_mode() == QuirksMode::Quirks && !doc.skips_quirks_fill {
        fill_root_and_body(doc, cascade, body, page_width, page_height);
    }
}

/// The work of [`prepare`]; `None` once nothing more fills.
fn fill_root_and_body(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    body: usize,
    page_width: f32,
    page_height: f32,
) -> Option<()> {
    let root = super::initial_containing_block::root_element(doc)?;
    let root_content = match fill(doc, cascade, root, true, page_width, page_height) {
        Some(filled) => {
            doc.quirks_percentage_height_bases
                .push((root, filled.content_height));
            filled.content_height
        }
        None => stated_content_height(&cascade.computed[root], page_width, page_height)?,
    };
    // Only a body that is a child of the root element fills it.
    (doc.nodes[body].parent == Some(root)).then_some(())?;
    // The body's margins and padding resolve against the root element's
    // content box, which the root's own inline edges narrow.
    let body_basis = (page_width - inline_edges(&cascade.computed[root], page_width)).max(0.0);
    let filled = fill(doc, cascade, body, false, body_basis, root_content)?;
    doc.quirks_percentage_height_bases
        .push((body, filled.content_height));
    Some(())
}

/// `inputs` for box `idx` with the height a `stretch` height fills when the
/// box is a child of a filled root element or body: the filled content box,
/// less the margins that do not collapse through the parent
/// (CSS Sizing 4 §3.1). Chromium takes that height as definite for the
/// child, whatever the parent's content makes of the parent itself.
pub(super) fn with_stretch_height(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.known_dimensions.height.is_some() || inputs.sizing_mode != SizingMode::InherentSize {
        return inputs;
    }
    let Some(parent) = tree.nodes[idx].parent else {
        return inputs;
    };
    let Some(&(_, room)) = tree
        .quirks_percentage_height_bases
        .iter()
        .find(|(owner, _)| *owner == parent)
    else {
        return inputs;
    };
    let style = tree.physical_style(idx);
    if !matches!(style.size().height.expand(), ExpandedDimension::Stretch) {
        return inputs;
    }
    let basis = inputs.parent_size.width;
    let margin = style.margin().resolve_or_zero(basis);
    let parent_style = tree.physical_style(parent);
    let parent_edges = parent_style.padding().resolve_or_zero(basis)
        + parent_style.border().resolve_or_zero(basis);
    // A block-level child's margin collapses through a side of the parent
    // that has no padding or border, and fills nothing there.
    let block_level = !tree.nodes[idx].display().is_inline_level();
    let kept = |margin: f32, parent_edge: f32| {
        if block_level && parent_edge == 0.0 {
            0.0
        } else {
            margin
        }
    };
    let margins = kept(margin.top, parent_edges.top) + kept(margin.bottom, parent_edges.bottom);
    let edges = style.padding().resolve_or_zero(basis) + style.border().resolve_or_zero(basis);
    let adjustment = if style.box_sizing() == raikiri_layout_style::values::BoxSizing::ContentBox {
        edges.top + edges.bottom
    } else {
        0.0
    };
    let bound = |value: Option<f32>| value.map(|value| value + adjustment);
    let height = (room - margins)
        .maybe_min(bound(style.max_size().height.maybe_resolve(Some(room))))
        .maybe_max(bound(style.min_size().height.maybe_resolve(Some(room))))
        .max(edges.top + edges.bottom);
    inputs.known_dimensions.height = Some(height);
    inputs.known_dimensions_are_definite.height = true;
    inputs
}

/// The height a filled box takes.
struct Filled {
    /// The content-box height its children resolve percentages against.
    content_height: f32,
}

/// Give `idx` the `min-height` that fills `room`, the content-box height of
/// the box around it, when the quirk applies to it. `basis` is the width its
/// margins and padding resolve against; a percentage `min-height` or
/// `max-height` resolves against `room`.
fn fill(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    is_root: bool,
    basis: f32,
    room: f32,
) -> Option<Filled> {
    let cv = &cascade.computed[idx];
    if !fills(cv, is_root) || doc.nodes[idx].used_writing_mode.is_vertical() {
        return None;
    }
    // Chromium clamps the filled height by `min-height` and `max-height`
    // like any other height.
    let minimum = limit(&cv.min_height, room)?.unwrap_or(0.0);
    let maximum = limit(&cv.max_height, room)?.unwrap_or(f32::INFINITY);
    let margins = margin(&cv.margin.top, basis) + margin(&cv.margin.bottom, basis);
    let edges = block_edges(cv, basis);
    let border_box = (room - margins).max(edges);
    let border_box_sizing = cv.box_sizing == BoxSizing::BorderBox;
    let filled = if border_box_sizing {
        border_box
    } else {
        border_box - edges
    };
    let used = filled.min(maximum).max(minimum);
    doc.record_pass_override(
        idx,
        OwnedPropertyOverride::length(
            LengthProperty::MinHeight,
            ComputedLengthPercentageOrAuto::Px(used),
            true,
        )
        .expect("resolved quirks fill minimum"),
    );
    let content_height = if border_box_sizing {
        (used - edges).max(0.0)
    } else {
        used
    };
    Some(Filled { content_height })
}

/// Whether the quirk applies to a root element or body with these values: an
/// in-flow block-level box with an `auto` height. The root element is
/// block-level whatever its `display` (CSS Display 3 §2.7).
fn fills(cv: &ComputedValues, is_root: bool) -> bool {
    cv.height == ComputedLengthPercentageOrAuto::Auto
        && matches!(cv.position, PositionValue::Static | PositionValue::Relative)
        && matches!(cv.float, FloatValue::None)
        && (is_root || !cv.display.is_inline_level())
        && !matches!(cv.display, DisplayValue::None | DisplayValue::Contents)
}

/// The content-box height of a root element that states its height, which
/// the body then fills. A percentage resolves against the page area.
fn stated_content_height(cv: &ComputedValues, basis: f32, page_height: f32) -> Option<f32> {
    let height = length(&cv.height, page_height)?;
    Some(if cv.box_sizing == BoxSizing::BorderBox {
        (height - block_edges(cv, basis)).max(0.0)
    } else {
        height.max(0.0)
    })
}

/// A `min-height` or `max-height` against `basis`: `Some(None)` for `auto`
/// or `none`, and `None` for a sizing keyword, which the quirk leaves alone.
fn limit(value: &ComputedLengthPercentageOrAuto, basis: f32) -> Option<Option<f32>> {
    match value {
        ComputedLengthPercentageOrAuto::Auto => Some(None),
        value => length(value, basis).map(Some),
    }
}

/// A length, percentage or `calc()` against `basis`; `None` for `auto` and
/// the sizing keywords.
fn length(value: &ComputedLengthPercentageOrAuto, basis: f32) -> Option<f32> {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(*px),
        ComputedLengthPercentageOrAuto::Percent(percent) => Some(basis * percent / 100.0),
        ComputedLengthPercentageOrAuto::Calc(calc) => Some(calc.px + basis * calc.percent / 100.0),
        ComputedLengthPercentageOrAuto::Math(math) => Some(math.resolve(basis)),
        _ => None,
    }
}

/// A margin against `basis`; `auto` is zero.
fn margin(value: &ComputedLengthPercentageOrAuto, basis: f32) -> f32 {
    length(value, basis).unwrap_or(0.0)
}

/// The top and bottom padding and border.
fn block_edges(cv: &ComputedValues, basis: f32) -> f32 {
    cv.padding.top.resolve(basis)
        + cv.padding.bottom.resolve(basis)
        + cv.border.top.width().px()
        + cv.border.bottom.width().px()
}

/// The left and right margin, padding and border.
fn inline_edges(cv: &ComputedValues, basis: f32) -> f32 {
    margin(&cv.margin.left, basis)
        + margin(&cv.margin.right, basis)
        + cv.padding.left.resolve(basis)
        + cv.padding.right.resolve(basis)
        + cv.border.left.width().px()
        + cv.border.right.width().px()
}

#[cfg(test)]
mod tests;
