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

use crate::NodeFlags;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, SizingMode};
use crate::layout::taffy_adapter::CollapsibleMarginSet;
use raikiri_layout_geometry::MaybeMath;
use raikiri_layout_style::values::{ExpandedDimension, MaybeResolve, ResolveOrZero};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::property::{
    BoxSizing, DisplayValue, FloatValue, OverflowValue, OverflowXY, PositionValue,
};
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
    doc.quirks_body_fill = None;
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
        Some(content_height) => {
            doc.quirks_percentage_height_bases
                .push((root, content_height));
            content_height
        }
        None => stated_content_height(&cascade.computed[root], page_width, page_height)?,
    };
    // Only a body that is a child of the root element fills it.
    (doc.nodes[body].parent == Some(root)).then_some(())?;
    // The body's margins and padding resolve against the width of the root
    // element's content box.
    let body_basis = content_width(&cascade.computed[root], page_width);
    let body_content = fill(doc, cascade, body, false, body_basis, root_content)?;
    doc.quirks_percentage_height_bases
        .push((body, body_content));
    Some(())
}

/// `inputs` for box `idx` when it is an in-flow child of a filled root
/// element or body: the filled content box is the height its percentages
/// resolve against, and a `stretch` height fills it, less the margins that
/// do not collapse through the parent (CSS Sizing 4 §3.1). Chromium takes
/// that height as definite for the child, whatever the parent's content
/// makes of the parent itself.
pub(super) fn with_filled_parent(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if tree.quirks_percentage_height_bases.is_empty() {
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
    // An absolutely positioned box takes its sizes from its containing
    // block instead.
    if style.position().is_out_of_flow() {
        return inputs;
    }
    inputs.parent_size.height = Some(room);
    if inputs.known_dimensions.height.is_some()
        || inputs.sizing_mode != SizingMode::InherentSize
        || !matches!(style.size().height.expand(), ExpandedDimension::Stretch)
    {
        return inputs;
    }
    let basis = inputs.parent_size.width;
    let margin = style.margin().resolve_or_zero(basis);
    let parent_style = tree.physical_style(parent);
    let parent_edges = parent_style.padding().resolve_or_zero(basis)
        + parent_style.border().resolve_or_zero(basis);
    // An in-flow block-level child's margin collapses through a side of a
    // parent that has no padding or border there, and fills nothing there.
    // The root element, a scroll container and a parent that lays out no
    // block flow of its own keep their children's margins inside.
    let parent_node = &tree.nodes[parent];
    let collapses_through = !parent_node.flags.contains(NodeFlags::IS_ROOT_ELEMENT)
        && parent_node.display() == DisplayValue::Block
        && !parent_node.roots_block_context()
        && !parent_style.overflow().x.is_scroll_container()
        && !parent_style.overflow().y.is_scroll_container()
        && !tree.nodes[idx].display().is_inline_level()
        && !style.float().is_floated();
    let kept = |margin: f32, parent_edge: f32| {
        if collapses_through && parent_edge == 0.0 {
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

/// The fill of a body whose margins collapse with those of its children.
/// Its `min-height` override fills what its own margins leave, which other
/// layouts use; block layout uses this instead, once it knows the margins
/// that adjoin the body's edges.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyFill {
    /// The body.
    node: usize,
    /// The content-box height of the root element.
    room: f32,
    /// The body's own top and bottom margins.
    margin_top: f32,
    margin_bottom: f32,
    /// The body's authored minimum and maximum border-box heights.
    minimum: f32,
    maximum: f32,
    /// The body's top and bottom padding and border.
    edges: f32,
}

impl BodyFill {
    /// The fill of `node` when it is the filled body.
    pub(crate) fn of(tree: &LayoutSession<'_>, node: usize) -> Option<Self> {
        tree.quirks_body_fill.filter(|fill| fill.node == node)
    }

    /// The border-box height of the body when its content makes it
    /// `content` tall and the margin sets `leading` and `trailing` adjoin its
    /// top and bottom edges, and whether its last child's bottom margin no
    /// longer adjoins its bottom edge. The body is at least what fills the
    /// root element less its margins collapsed with theirs. As in Chromium,
    /// the last margin stays inside, though the fill still leaves room for
    /// it, only when the authored minimum gives the height above the content.
    pub(crate) fn height(
        &self,
        content: f32,
        leading: CollapsibleMarginSet,
        trailing: CollapsibleMarginSet,
    ) -> (f32, bool) {
        let clamp = |height: f32| height.min(self.maximum).max(self.minimum).max(self.edges);
        let top = leading.collapse_with_margin(self.margin_top).resolve();
        let bottom = trailing.collapse_with_margin(self.margin_bottom).resolve();
        let height = clamp(content).max(clamp(self.room - top - bottom));
        let separates = self.minimum > 0.0 && self.minimum > content && self.minimum >= height;
        (height, separates)
    }
}

/// Give `idx` the `min-height` that fills `room`, the content-box height of
/// the box around it, when the quirk applies to it, and return the
/// content-box height its children resolve percentages against. `basis` is the width its
/// margins and padding resolve against; a percentage `min-height` or
/// `max-height` resolves against `room`.
fn fill(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    is_root: bool,
    basis: f32,
    room: f32,
) -> Option<f32> {
    let cv = &cascade.computed[idx];
    if !fills(cv, is_root) || doc.nodes[idx].used_writing_mode.is_vertical() {
        return None;
    }
    // Chromium clamps the filled height by `min-height` and `max-height`
    // like any other height.
    let minimum = limit(&cv.min_height, room)?.unwrap_or(0.0);
    let maximum = limit(&cv.max_height, room)?.unwrap_or(f32::INFINITY);
    let edges = block_edges(cv, basis);
    let border_box_sizing = cv.box_sizing == BoxSizing::BorderBox;
    // The height in `box-sizing` terms that fills `room` less `margins`.
    let used = |margins: f32| {
        let border_box = (room - margins).max(edges);
        let filled = if border_box_sizing {
            border_box
        } else {
            border_box - edges
        };
        filled.min(maximum).max(minimum)
    };
    let margin_top = margin(&cv.margin.top, basis);
    let margin_bottom = margin(&cv.margin.bottom, basis);
    doc.record_pass_override(
        idx,
        OwnedPropertyOverride::length(
            LengthProperty::MinHeight,
            ComputedLengthPercentageOrAuto::Px(used(margin_top + margin_bottom)),
            true,
        )
        .expect("resolved quirks fill minimum"),
    );
    if !is_root && collapses_with_children(doc, cascade, idx, idx) {
        // A body whose margins collapse with those of its children fills
        // what its margins leave once they have collapsed, as in Chromium;
        // block layout finds them (see [`BodyFill::height`]).
        let border_box = |height: f32| {
            if border_box_sizing {
                height
            } else {
                height + edges
            }
        };
        doc.quirks_body_fill = Some(BodyFill {
            node: idx,
            room,
            margin_top,
            margin_bottom,
            minimum: border_box(minimum),
            maximum: border_box(maximum),
            edges,
        });
    }
    // Its children resolve percentages against what its own margins leave.
    let used = used(margin_top + margin_bottom);
    let content_height = if border_box_sizing {
        (used - edges).max(0.0)
    } else {
        used
    };
    Some(content_height)
}

/// Whether block `idx` lays out block flow whose margins can collapse with
/// its own. `body` is the body, whose `overflow` goes to the viewport when
/// the root element's is `visible`.
fn collapses_with_children(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    body: usize,
) -> bool {
    let node = &doc.nodes[idx];
    let scrolls = |overflow: OverflowXY| {
        !matches!(overflow.x, OverflowValue::Visible | OverflowValue::Clip)
            || !matches!(overflow.y, OverflowValue::Visible | OverflowValue::Clip)
    };
    let propagated = idx == body
        && node
            .parent
            .is_some_and(|root| !scrolls(cascade.computed[root].overflow));
    node.display() == DisplayValue::Block
        && !node.roots_block_context()
        && (propagated || !scrolls(cascade.computed[idx].overflow))
}

/// Whether the quirk applies to a root element or body with these values: an
/// in-flow block-level box with an `auto` height. The root element is a
/// block box whatever its `display` other than `none` (CSS Display 3 §2.7,
/// §2.5).
fn fills(cv: &ComputedValues, is_root: bool) -> bool {
    cv.height == ComputedLengthPercentageOrAuto::Auto
        && matches!(cv.position, PositionValue::Static | PositionValue::Relative)
        && matches!(cv.float, FloatValue::None)
        && (is_root || !cv.display.is_inline_level())
        && cv.display != DisplayValue::None
        && (is_root || cv.display != DisplayValue::Contents)
}

/// The content-box height of a root element that states its height, which
/// the body then fills. Percentages resolve against the page area.
fn stated_content_height(cv: &ComputedValues, basis: f32, page_height: f32) -> Option<f32> {
    let height = length(&cv.height, page_height)?;
    let minimum = limit(&cv.min_height, page_height)?.unwrap_or(0.0);
    let maximum = limit(&cv.max_height, page_height)?.unwrap_or(f32::INFINITY);
    let height = height.min(maximum).max(minimum);
    Some(if cv.box_sizing == BoxSizing::BorderBox {
        (height - block_edges(cv, basis)).max(0.0)
    } else {
        height.max(0.0)
    })
}

/// The width of the content box of a block-level box with values `cv` in a
/// containing block `available` wide, with its `min-width` and `max-width`
/// applied.
fn content_width(cv: &ComputedValues, available: f32) -> f32 {
    let edges = cv.padding.left.resolve(available)
        + cv.padding.right.resolve(available)
        + cv.border.left.width().px()
        + cv.border.right.width().px();
    // A width in `box-sizing` terms, as the content box takes it.
    let content = |width: f32| {
        if cv.box_sizing == BoxSizing::BorderBox {
            width - edges
        } else {
            width
        }
    };
    let width = length(&cv.width, available)
        .map(content)
        .unwrap_or_else(|| {
            available
                - margin(&cv.margin.left, available)
                - margin(&cv.margin.right, available)
                - edges
        });
    let maximum = length(&cv.max_width, available).map(content);
    let minimum = length(&cv.min_width, available).map(content);
    width
        .min(maximum.unwrap_or(f32::INFINITY))
        .max(minimum.unwrap_or(0.0))
        .max(0.0)
}

/// A `min-height` or `max-height` against `basis`: `Some(None)` for `auto`
/// or `none`, and `None` for a sizing keyword, which the quirk leaves alone.
fn limit(value: &ComputedLengthPercentageOrAuto, basis: f32) -> Option<Option<f32>> {
    match value {
        ComputedLengthPercentageOrAuto::Auto => Some(None),
        value => length(value, basis).map(Some),
    }
}

/// A length, percentage or `calc()` against `basis`; `None` for `auto`, the
/// sizing keywords and a value that is not finite.
fn length(value: &ComputedLengthPercentageOrAuto, basis: f32) -> Option<f32> {
    let length = match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(*px),
        ComputedLengthPercentageOrAuto::Percent(percent) => Some(basis * percent / 100.0),
        ComputedLengthPercentageOrAuto::Calc(calc) => Some(calc.px + basis * calc.percent / 100.0),
        ComputedLengthPercentageOrAuto::Math(math) => Some(math.resolve(basis)),
        _ => None,
    };
    length.filter(|length| length.is_finite())
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

#[cfg(test)]
mod tests;
