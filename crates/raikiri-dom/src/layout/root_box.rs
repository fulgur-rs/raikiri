//! The root element's box, which places the canvas background.
//!
//! Layout starts at the body, so the root element gets no box of its own
//! there. Painting still needs one: the images of the canvas background are
//! sized and positioned against the root element's box (CSS Backgrounds 3
//! §2.11.2), and a transform on the root moves the whole canvas (CSS
//! Transforms 1 §6). This module rebuilds that box from the root's computed
//! style and the page area.

use super::page::{page_content_insets, page_margins, used_computed_length_percentage_or_auto};
use crate::Document;
use raikiri_style::property::{BoxSizing, Direction, DisplayValue};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto, ComputedValues};
use raikiri_traits::{NodeId, NodeKind, PageBox, PaintInsets, PaintRect, PaintTransform};

/// The root element's box on one page, in page coordinates.
///
/// Paged media cuts the root element across pages like any other box, and
/// the canvas it paints on runs through every page as one surface. On the
/// page whose flow starts at `content_origin_y`, the box sits that far
/// higher than on the first page, so a later page shows the part of it that
/// continues there, or nothing of it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RootBox {
    /// The root element.
    pub node: NodeId,
    /// The border box, in page coordinates. Its top is above the page on a
    /// page after the first one.
    pub border_box: PaintRect,
    /// Used border widths.
    pub border: PaintInsets,
    /// Used padding.
    pub padding: PaintInsets,
    /// The root's `transform`, with its origin and percentages resolved
    /// against [`Self::border_box`], in page coordinates. `None` when the
    /// root has no transform. The matrix may be singular, in which case the
    /// root and its content are not rendered (CSS Transforms 1 §6).
    pub transform: Option<PaintTransform>,
}

/// The box of the root element on the page whose share of the flow starts
/// at `content_origin_y`, in a document of `page_count` pages, or `None`
/// when there is no root element or it generates no box.
///
/// The box is placed in the page area, inside the page margins, border and
/// padding, moved in by the root's margins. A root with a definite `width`
/// or `height` takes that size; otherwise its width fills the page area
/// between its margins and its height fills the flow of all `page_count`
/// pages, each as tall as this page's area, between its margins. Layout
/// does not size the root by its content, so an auto height is the extent
/// of the pages the flow runs through; pages of different sizes and a
/// root margin cut at a page break are not followed. `auto` margins
/// share what the root's used width leaves of the page area. A root with
/// `display: none` generates no box.
pub fn root_box(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    content_origin_y: f32,
    page_count: u32,
) -> Option<RootBox> {
    let root = root_element(document)?;
    let computed = cascade.computed.get(root)?;
    // A root with `display: none` generates no box.
    if matches!(computed.display, DisplayValue::None) {
        return None;
    }
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    let area_x = margins.left + insets.left;
    let area_y = margins.top + insets.top;
    let area_width = insets.page_area_width(margins, page_box);
    let area_height = insets.page_area_height(margins, page_box);

    // Margins and padding resolve against the width of the root's
    // containing block, the page area. Layout clamps the root's block-start
    // margin to zero; the box follows it.
    let margin = |value| used_computed_length_percentage_or_auto(value, area_width);
    let margin_top = margin(computed.margin.top).unwrap_or(0.0).max(0.0);
    let margin_bottom = margin(computed.margin.bottom).unwrap_or(0.0);
    let border = PaintInsets::new(
        computed.border.top.width().px(),
        computed.border.right.width().px(),
        computed.border.bottom.width().px(),
        computed.border.left.width().px(),
    );
    let padding = PaintInsets::new(
        computed.padding.top.resolve(area_width).max(0.0),
        computed.padding.right.resolve(area_width).max(0.0),
        computed.padding.bottom.resolve(area_width).max(0.0),
        computed.padding.left.resolve(area_width).max(0.0),
    );
    let horizontal_edges = border.left + border.right + padding.left + padding.right;
    let vertical_edges = border.top + border.bottom + padding.top + padding.bottom;

    // CSS 2.1 §10.3.3 and §10.4: the used width comes first, from a
    // definite `width` or by filling what the margins leave, clamped by the
    // width limits. The margins then solve the block-width equation with
    // that width: `auto` margins share what is left, and are zero when the
    // box overflows the page area; an over-constrained equation gives up
    // the end margin of the root's direction.
    let (margin_left, margin_right) = (margin(computed.margin.left), margin(computed.margin.right));
    let width = definite_border_box_size(
        computed,
        computed.width,
        computed.min_width,
        computed.max_width,
        area_width,
        horizontal_edges,
    )
    .unwrap_or_else(|| {
        let fill = area_width - margin_left.unwrap_or(0.0) - margin_right.unwrap_or(0.0);
        clamp_auto_size(
            computed,
            (fill - horizontal_edges).max(0.0),
            computed.min_width,
            computed.max_width,
            area_width,
            horizontal_edges,
        ) + horizontal_edges
    });
    let free = area_width - width - margin_left.unwrap_or(0.0) - margin_right.unwrap_or(0.0);
    let rtl = matches!(computed.direction, Direction::Rtl);
    let margin_left = match (margin_left, margin_right) {
        (None, None) if free >= 0.0 => free / 2.0,
        (None, Some(right)) if free >= 0.0 => area_width - width - right,
        (Some(left), None) if free >= 0.0 => left,
        (_, right) if rtl => area_width - width - right.unwrap_or(0.0),
        (left, _) => left.unwrap_or(0.0),
    };
    let height = definite_border_box_size(
        computed,
        computed.height,
        computed.min_height,
        computed.max_height,
        area_height,
        vertical_edges,
    )
    .unwrap_or_else(|| {
        // The flow runs at least to the end of this page.
        let flow = (area_height * page_count.max(1) as f32).max(content_origin_y + area_height);
        let content = (flow - margin_top - margin_bottom - vertical_edges).max(0.0);
        clamp_auto_size(
            computed,
            content,
            computed.min_height,
            computed.max_height,
            area_height,
            vertical_edges,
        ) + vertical_edges
    });
    let border_box = PaintRect::new(
        area_x + margin_left,
        area_y + margin_top - content_origin_y,
        width,
        height,
    );
    let transform = (!computed.transform.is_empty()).then(|| root_transform(computed, border_box));
    Some(RootBox {
        node: NodeId(root as u64),
        border_box,
        border,
        padding,
        transform,
    })
}

/// The first `<html>` element child of the document node.
fn root_element(document: &Document) -> Option<usize> {
    document
        .get_node(document.root_index())?
        .children
        .iter()
        .copied()
        .find(|&id| {
            document.get_node(id).is_some_and(|node| {
                node.is_in_document()
                    && node.kind() == NodeKind::Element
                    && node.tag_name() == Some("html")
            })
        })
}

/// The border-box size from a definite `width` or `height`, clamped by
/// its `min-` and `max-` limits and read as `box-sizing` says.
fn definite_border_box_size(
    computed: &ComputedValues,
    size: ComputedLengthPercentageOrAuto,
    min: ComputedLengthPercentageOrAuto,
    max: ComputedLengthPercentageOrAuto,
    basis: f32,
    edges: f32,
) -> Option<f32> {
    let size = used_computed_length_percentage_or_auto(size, basis)?.max(0.0);
    let content = if matches!(computed.box_sizing, BoxSizing::BorderBox) {
        (size - edges).max(0.0)
    } else {
        size
    };
    Some(clamp_auto_size(computed, content, min, max, basis, edges) + edges)
}

/// `content` clamped by `max` and then `min`, both read as `box-sizing`
/// says.
fn clamp_auto_size(
    computed: &ComputedValues,
    content: f32,
    min: ComputedLengthPercentageOrAuto,
    max: ComputedLengthPercentageOrAuto,
    basis: f32,
    edges: f32,
) -> f32 {
    let limit = |value| {
        used_computed_length_percentage_or_auto(value, basis).map(|limit| {
            if matches!(computed.box_sizing, BoxSizing::BorderBox) {
                (limit - edges).max(0.0)
            } else {
                limit.max(0.0)
            }
        })
    };
    let content = limit(max).map_or(content, |max| content.min(max));
    limit(min).map_or(content, |min| content.max(min))
}

/// The root's transform about its `transform-origin`, in page coordinates.
// spec: https://www.w3.org/TR/css-transforms-1/#transform-rendering
fn root_transform(computed: &ComputedValues, border_box: PaintRect) -> PaintTransform {
    let width = f64::from(border_box.width);
    let height = f64::from(border_box.height);
    let origin = (
        f64::from(border_box.x)
            + crate::image_geometry::position_offset(computed.transform_origin.horizontal, width),
        f64::from(border_box.y)
            + crate::image_geometry::position_offset(computed.transform_origin.vertical, height),
    );
    use crate::page_projection::paint_transform::{Affine, function_matrix};
    let matrix = Affine::translate(origin.0, origin.1)
        .then(function_matrix(computed, width, height))
        .then(Affine::translate(-origin.0, -origin.1));
    let [a, b, c, d, e, f] = matrix.0.map(|value| value as f32);
    PaintTransform::new(a, b, c, d, e, f)
}

#[cfg(test)]
#[path = "root_box/tests.rs"]
mod tests;
