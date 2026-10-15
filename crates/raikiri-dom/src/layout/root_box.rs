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
/// page whose flow starts at `content_origin_y`, the box sits that far back
/// along the flow from where it sits on the first page, so a later page
/// shows the part of it that continues there, or nothing of it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RootBox {
    /// The root element.
    pub node: NodeId,
    /// The border box, in page coordinates. Its block-start edge is before
    /// the page on a page after the first one.
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
/// or `height` takes that size. Otherwise, across the pages it fills the
/// page area between its margins, and along them it fills the flow of all
/// `page_count` pages, each as long as this page's area, between its
/// margins. Layout does not size the root by its content, so an auto size
/// along the pages is the extent of the pages the flow runs through; pages
/// of different sizes and a root margin cut at a page break are not
/// followed. `auto` margins share what the root's used size leaves of the
/// page area across the pages. A root with `display: none` generates no
/// box.
///
/// The pages run down the flow, or across it along x for a root whose
/// blocks stack on vertical axes; there, `content_origin_y` is the distance
/// along x, and a later page sees the box moved by it.
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
    // The pages of a root whose blocks stack on vertical axes follow the x
    // axis: the root's block axis runs across them, and its inline axis
    // down each page.
    let pages_along_x = super::pages_follow_x_axis(document, cascade);
    let inline_size = if pages_along_x.is_some() {
        area_height
    } else {
        area_width
    };

    // Margins and padding resolve against the inline size of the root's
    // containing block, the page area (CSS Writing Modes 4 §7.2). Layout
    // clamps the root's block-start margin to zero; the box follows it.
    let margin = |value| used_computed_length_percentage_or_auto(value, inline_size);
    let border = PaintInsets::new(
        computed.border.top.width().px(),
        computed.border.right.width().px(),
        computed.border.bottom.width().px(),
        computed.border.left.width().px(),
    );
    let padding = PaintInsets::new(
        computed.padding.top.resolve(inline_size).max(0.0),
        computed.padding.right.resolve(inline_size).max(0.0),
        computed.padding.bottom.resolve(inline_size).max(0.0),
        computed.padding.left.resolve(inline_size).max(0.0),
    );
    let horizontal = Axis {
        size: computed.width,
        min: computed.min_width,
        max: computed.max_width,
        area: area_width,
        edges: border.left + border.right + padding.left + padding.right,
        margins: (margin(computed.margin.left), margin(computed.margin.right)),
    };
    let vertical = Axis {
        size: computed.height,
        min: computed.min_height,
        max: computed.max_height,
        area: area_height,
        edges: border.top + border.bottom + padding.top + padding.bottom,
        margins: (margin(computed.margin.top), margin(computed.margin.bottom)),
    };
    let rtl = matches!(computed.direction, Direction::Rtl);
    let border_box = match pages_along_x {
        None => {
            let (margin_left, width) = horizontal.across_pages(computed, rtl);
            let (margin_top, height) =
                vertical.along_pages(computed, false, content_origin_y, page_count);
            PaintRect::new(
                area_x + margin_left,
                area_y + margin_top - content_origin_y,
                width,
                height,
            )
        }
        Some(right_to_left) => {
            let (margin_top, height) = vertical.across_pages(computed, rtl);
            let (margin_start, width) =
                horizontal.along_pages(computed, right_to_left, content_origin_y, page_count);
            // A later page sees the box moved by the flow before it, away
            // from the side the pages continue to.
            let x = if right_to_left {
                area_x + area_width - margin_start - width + content_origin_y
            } else {
                area_x + margin_start - content_origin_y
            };
            PaintRect::new(x, area_y + margin_top, width, height)
        }
    };
    let transform = (!computed.transform.is_empty()).then(|| root_transform(computed, border_box));
    Some(RootBox {
        node: NodeId(root as u64),
        border_box,
        border,
        padding,
        transform,
    })
}

/// The sizing inputs of the root along one physical axis.
struct Axis {
    size: ComputedLengthPercentageOrAuto,
    min: ComputedLengthPercentageOrAuto,
    max: ComputedLengthPercentageOrAuto,
    /// Extent of the page area along the axis.
    area: f32,
    /// Border and padding on both sides.
    edges: f32,
    /// Used margins on the start and end side, in physical order; `None`
    /// for `auto`.
    margins: (Option<f32>, Option<f32>),
}

impl Axis {
    /// The used start margin and border-box size of the root across the
    /// pages, where each page holds all of it.
    ///
    /// CSS 2.1 §10.3.3 and §10.4: the used size comes first, from a
    /// definite size or by filling what the margins leave, clamped by the
    /// limits. The margins then solve the box equation with that size:
    /// `auto` margins share what is left, and are zero when the box
    /// overflows the page area; an over-constrained equation gives up the
    /// end margin, the start one when `reversed`.
    fn across_pages(&self, computed: &ComputedValues, reversed: bool) -> (f32, f32) {
        let (start, end) = self.margins;
        let size = definite_border_box_size(
            computed, self.size, self.min, self.max, self.area, self.edges,
        )
        .unwrap_or_else(|| {
            let fill = self.area - start.unwrap_or(0.0) - end.unwrap_or(0.0);
            clamp_auto_size(
                computed,
                (fill - self.edges).max(0.0),
                self.min,
                self.max,
                self.area,
                self.edges,
            ) + self.edges
        });
        let free = self.area - size - start.unwrap_or(0.0) - end.unwrap_or(0.0);
        let start = match (start, end) {
            (None, None) if free >= 0.0 => free / 2.0,
            (None, Some(end)) if free >= 0.0 => self.area - size - end,
            (Some(start), None) if free >= 0.0 => start,
            (_, end) if reversed => self.area - size - end.unwrap_or(0.0),
            (start, _) => start.unwrap_or(0.0),
        };
        (start, size)
    }

    /// The used block-start margin and border-box size of the root along
    /// the flow of `page_count` pages, which starts at the end side when
    /// `reversed`. The page holding the flow from `content_origin` is
    /// among them.
    ///
    /// An auto size fills the flow of all the pages, each as long as this
    /// page's area, between the margins. Layout does not size the root by
    /// its content, so the flow is the extent of the pages it runs through;
    /// pages of different sizes and a root margin cut at a page break are
    /// not followed.
    fn along_pages(
        &self,
        computed: &ComputedValues,
        reversed: bool,
        content_origin: f32,
        page_count: u32,
    ) -> (f32, f32) {
        let (start, end) = if reversed {
            (self.margins.1, self.margins.0)
        } else {
            self.margins
        };
        let start = start.unwrap_or(0.0).max(0.0);
        let end = end.unwrap_or(0.0);
        let size = definite_border_box_size(
            computed, self.size, self.min, self.max, self.area, self.edges,
        )
        .unwrap_or_else(|| {
            // The flow runs at least to the end of this page.
            let flow = (self.area * page_count.max(1) as f32).max(content_origin + self.area);
            let content = (flow - start - end - self.edges).max(0.0);
            clamp_auto_size(computed, content, self.min, self.max, self.area, self.edges)
                + self.edges
        });
        (start, size)
    }
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
