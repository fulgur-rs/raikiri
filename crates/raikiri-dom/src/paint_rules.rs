//! Paint-order and paint-eligibility predicates shared by the paint walker and
//! the page paint-order event generator.

use crate::Document;
use raikiri_style::property::{
    BackgroundImage, BorderCollapseValue, ContentComponent, DisplayValue, EmptyCellsValue,
    FloatValue, OverflowValue, OverflowXY, PositionValue, Visibility, WhiteSpaceCollapse,
    ZIndexValue,
};
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem, StyleNodeId};
use raikiri_traits::{NodeKind, PaintClip, PaintInsets, PaintRect};

/// Whether `node_id` is painted on the page whose name is `active_page_name`.
///
/// `None` skips the check. An inline `<canvas>` and floated boxes stay on
/// the page layout placed them on even when their `page` value names
/// another page.
pub fn named_page_matches(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    active_page_name: Option<Option<&str>>,
) -> bool {
    match active_page_name {
        None => true,
        Some(active) => {
            // An inline canvas is a boundary marker for pagination, but its
            // `page` value does not assign the replaced inline box to the
            // named page. Keep painting it on the page selected by layout.
            let is_inline_canvas = document.get_node(node_id).is_some_and(|node| {
                node.tag_name()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("canvas"))
                    && matches!(cascade.computed[node_id].display, DisplayValue::Inline)
            });
            if is_inline_canvas {
                true
            } else {
                match cascade.page_values.get(node_id) {
                    Some(raikiri_style::property::PageValue::Named(name)) => {
                        // Floats retain the preceding page in the page-name-float
                        // cases; do not hide the floated box merely because its
                        // inherited page value names the following page.
                        if matches!(
                            cascade.computed[node_id].float,
                            FloatValue::Left
                                | FloatValue::Right
                                | FloatValue::InlineStart
                                | FloatValue::InlineEnd
                                | FloatValue::Footnote
                        ) {
                            true
                        } else {
                            active.is_some_and(|page| name.0.as_str() == page)
                        }
                    }
                    _ => true,
                }
            }
        }
    }
}

/// Whether the element clips its descendants to its padding box.
pub fn clips_overflow(cv: &ComputedValues) -> bool {
    // Internal table tracks are not block, flex, or grid containers, so
    // overflow does not turn their materialized boxes into clipping boxes.
    if matches!(
        cv.display,
        DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup
    ) {
        return false;
    }
    !matches!(cv.overflow.x, OverflowValue::Visible)
        || !matches!(cv.overflow.y, OverflowValue::Visible)
}

pub(crate) fn clips_element_overflow(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let Some(node) = document.get_node(node_id) else {
        return false;
    };
    let Some(cv) = cascade.computed.get(node_id) else {
        return false;
    };
    if !node.is_in_document()
        || node.is_non_rendered_html_element()
        || node.kind() != NodeKind::Element
        || !clips_overflow(cv)
        || matches!(cv.display, DisplayValue::None | DisplayValue::Contents)
        || (cv.display == DisplayValue::Inline
            && !node.is_inline_svg_root()
            && !matches!(node.tag_name(), Some("img" | "canvas")))
    {
        return false;
    }
    if node.tag_name() == Some("html") && node.parent == Some(document.root_index()) {
        return false;
    }
    if node.tag_name() == Some("body")
        && let Some(parent_id) = node.parent
        && let Some(parent) = document.get_node(parent_id)
        && parent.tag_name() == Some("html")
        && parent.parent == Some(document.root_index())
        && cascade
            .computed
            .get(parent_id)
            .is_some_and(|root| !clips_overflow(root))
    {
        return false;
    }
    true
}

/// Resolve the local overflow clip of a whole border box in paint coordinates.
///
/// The caller supplies the box before page cuts and its used physical border
/// widths. Percentages and overlap are resolved on that box before insetting;
/// padding-edge curves are never normalized again after an opposite edge crops
/// them. Open axes impose no bound and do not form rounded corners.
///
/// The padding origin is floored to include pixel-snapped descendants. An
/// `overflow: clip` far edge is floored too. Viewport-propagated overflow and
/// ordinary inline boxes do not produce a local clip.
pub fn overflow_clip(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    border_box: PaintRect,
    border: PaintInsets,
) -> Option<PaintClip> {
    if !clips_element_overflow(document, cascade, node_id) {
        return None;
    }
    Some(OverflowClipGeometry::new(&cascade.computed[node_id], border_box, border).at(border_box))
}

/// Position-independent clip geometry shared by every placement of a source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OverflowClipGeometry {
    border: PaintInsets,
    overflow: OverflowXY,
    corner_radii: Option<[[f32; 2]; 4]>,
}

impl OverflowClipGeometry {
    pub(crate) fn new(cv: &ComputedValues, border_box: PaintRect, border: PaintInsets) -> Self {
        let corner_radii =
            if cv.overflow.x != OverflowValue::Visible && cv.overflow.y != OverflowValue::Visible {
                let mut radii = cv.border_radius.used(border_box.width, border_box.height);
                for (corner, [x, y]) in radii.iter_mut().zip([
                    [border.left, border.top],
                    [border.right, border.top],
                    [border.right, border.bottom],
                    [border.left, border.bottom],
                ]) {
                    let rx = (corner[0] - x).max(0.0);
                    let ry = (corner[1] - y).max(0.0);
                    *corner = if rx > 0.0 && ry > 0.0 {
                        [rx, ry]
                    } else {
                        [0.0; 2]
                    };
                }
                radii.iter().any(|corner| corner[0] > 0.0).then_some(radii)
            } else {
                None
            };
        Self {
            border,
            overflow: cv.overflow,
            corner_radii,
        }
    }

    /// Snap edges only after translating the whole border box to its page.
    pub(crate) fn at(self, border_box: PaintRect) -> PaintClip {
        let x0 = (border_box.x + self.border.left).floor();
        let y0 = (border_box.y + self.border.top).floor();
        let right = border_box.x + border_box.width - self.border.right;
        let bottom = border_box.y + border_box.height - self.border.bottom;
        let x1 = if self.overflow.x == OverflowValue::Clip {
            right.floor()
        } else {
            right
        };
        let y1 = if self.overflow.y == OverflowValue::Clip {
            bottom.floor()
        } else {
            bottom
        };
        let mut clip = PaintClip::new(PaintRect::new(
            x0,
            y0,
            (x1 - x0).max(0.0),
            (y1 - y0).max(0.0),
        ));
        clip.clip_x = self.overflow.x != OverflowValue::Visible;
        clip.clip_y = self.overflow.y != OverflowValue::Visible;
        clip.corner_radii = self.corner_radii;
        clip
    }
}

/// The group opacity the element paints its subtree with, if any.
///
/// An element with `display: contents` generates no box (CSS Display 3
/// §2.5), so its opacity has nothing to apply to: its children paint as if
/// they were children of its parent.
pub fn opacity_layer(cv: &ComputedValues) -> Option<f32> {
    (cv.opacity < 1.0 && cv.display != DisplayValue::Contents).then_some(cv.opacity)
}

/// A `visibility: hidden` table does not paint its own box.
pub fn is_visibility_hidden_table(cv: &ComputedValues) -> bool {
    cv.visibility == Visibility::Hidden
        && matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable)
}

/// A hidden table row or row group does not paint its own decorations.
pub fn is_visibility_hidden_table_part(cv: &ComputedValues) -> bool {
    cv.visibility == Visibility::Hidden
        && matches!(
            cv.display,
            DisplayValue::TableRow
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
        )
}

/// Whether a table row or row group has a background or border to paint.
///
/// A part without one paints nothing inside its cells, so it needs no
/// per-cell clip.
pub fn paints_table_part_decoration(cv: &ComputedValues) -> bool {
    cv.background_color.a > 0
        || !matches!(cv.background_image, BackgroundImage::None)
        || [
            &cv.border.top,
            &cv.border.right,
            &cv.border.bottom,
            &cv.border.left,
        ]
        .into_iter()
        .any(|side| side.width().px() > 0.0)
}

/// Stable-sorts a parent's children into paint order.
pub fn sort_paint_children(
    children: &mut [usize],
    parent_display: DisplayValue,
    cascade: &CascadeResult,
) {
    let order_sensitive_container = matches!(
        parent_display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Grid
            | DisplayValue::InlineGrid
    );
    // Stable tuple sorting keeps DOM order for equal (stack, order) values.
    // Direct abspos/fixed children are not flex/grid items: paint them as if
    // their order were zero, while in-flow (including relative) items use the
    // computed `order` value. Stacking buckets stay primary.
    children.sort_by_key(|&child| {
        // Anonymous paragraph keys are outside the source DOM arena. Their
        // non-inherited position, float and order use initial values.
        let Some(computed) = cascade.computed.get(child) else {
            return ((1, 0), 0, 0);
        };
        let stack =
            if order_sensitive_container && matches!(computed.position, PositionValue::Static) {
                // A flex/grid item can use integer z-index even when static.
                // Match positioned items' stack levels so order only breaks
                // ties within the same z-index level.
                match computed.z_index {
                    ZIndexValue::Auto => (1, 0),
                    ZIndexValue::Integer(value) if value < 0 => (0, value),
                    ZIndexValue::Integer(value) => (2, value),
                    _ => paint_order_key(cascade, child), // cov:ignore: defensive fallback for future non-exhaustive z-index variants.
                }
            } else {
                paint_order_key(cascade, child)
            };
        let order = if order_sensitive_container
            && !matches!(
                computed.position,
                PositionValue::Absolute | PositionValue::Fixed
            ) {
            computed.order
        } else {
            0
        };
        let float_paint_order = if !order_sensitive_container
            && matches!(
                computed.float,
                FloatValue::Left
                    | FloatValue::Right
                    | FloatValue::InlineStart
                    | FloatValue::InlineEnd
            ) {
            1
        } else {
            0
        };
        (stack, float_paint_order, order)
    });
}

fn paint_order_key(cascade: &CascadeResult, node_id: usize) -> (u8, i32) {
    let computed = &cascade.computed[node_id];
    match (&computed.position, computed.z_index) {
        // In-flow boxes paint before positioned descendants with an auto
        // z-index. Keep ordinary static boxes in the normal bucket, but
        // place flex containers with positioned descendants alongside
        // auto-z siblings so their later absolute child paints in tree order.
        (PositionValue::Static, _)
            if matches!(
                computed.display,
                DisplayValue::Flex | DisplayValue::InlineFlex
            ) =>
        {
            (2, 0)
        }
        (PositionValue::Static, _) => (1, 0),
        (_, ZIndexValue::Auto) => (2, 0),
        (_, ZIndexValue::Integer(value)) if value < 0 => (0, value),
        (_, ZIndexValue::Integer(value)) => (2, value),
        // PositionValue and ZIndexValue are non-exhaustive.  New variants
        // retain the default/source-order bucket until stacking support grows.
        _ => (1, 0),
    }
}

/// Walk the Document arena in DFS order; return the first `<body>` element index.
///
/// Use an iterative `Vec` stack (as in cascade §deep_nesting) to avoid
/// stack overflow on deep DOMs. Return `None` for fragments (no `<body>`).
///
/// Skip subtrees with `!is_in_document()` (such as `<template>` descendants)
/// so a hypothetical `<body>` in an inert subtree is not selected. The
/// paint-side and layout-side body lookups are separate to avoid crossing
/// module boundaries, but share the same contract.
pub fn find_paint_root(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Whether separate-border `empty-cells: hide` suppresses this cell's box ink.
/// Out-of-flow content does not make a cell nonempty; floated and in-flow
/// elements do, even when their own box has no content (CSS 2.2 section 17.6.1.1).
pub fn hides_empty_table_cell(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    let cv = &cascade.computed[node_id];
    if cv.display != DisplayValue::TableCell
        || cv.empty_cells != EmptyCellsValue::Hide
        || cv.border_collapse != BorderCollapseValue::Separate
    {
        return false;
    }
    if cv.visibility == Visibility::Hidden {
        return true;
    }
    lacks_visible_table_cell_content(
        document,
        cascade,
        Some(node_id),
        &document.nodes[node_id].children,
    )
}

/// Classify a layout-only cell using its inherited values and its own content
/// group. Pseudo-elements on the source row belong to that row, not this box.
pub(crate) fn hides_anonymous_table_cell(
    document: &Document,
    cascade: &CascadeResult,
    owner: usize,
    children: &[usize],
) -> bool {
    let cv = &cascade.computed[owner];
    cv.empty_cells == EmptyCellsValue::Hide
        && cv.border_collapse == BorderCollapseValue::Separate
        && (cv.visibility == Visibility::Hidden
            || lacks_visible_table_cell_content(document, cascade, None, children))
}

fn lacks_visible_table_cell_content(
    document: &Document,
    cascade: &CascadeResult,
    generated_owner: Option<usize>,
    children: &[usize],
) -> bool {
    let has_generated_box = |id| {
        [PseudoElem::Before, PseudoElem::After]
            .iter()
            .any(|pseudo| {
                cascade
                    .pseudo
                    .get(&(StyleNodeId::new(id as u64), *pseudo))
                    .is_some_and(|cv| {
                        cv.display != DisplayValue::None
                            && cv.visibility != Visibility::Hidden
                            && !matches!(
                                cv.position,
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            && !cv.content.is_empty()
                            && !cv
                                .content
                                .iter()
                                .any(|component| matches!(component, ContentComponent::None))
                    })
            })
    };
    // Generated boxes count as in-flow or floating content, including a
    // generated empty inline box, just like an authored empty inline element.
    if generated_owner.is_some_and(has_generated_box) {
        return false;
    }
    let mut stack = children.to_vec();
    while let Some(id) = stack.pop() {
        let node = &document.nodes[id];
        let cv = &cascade.computed[id];
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || cv.display == DisplayValue::None
            || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        {
            continue;
        }
        if cv.visibility == Visibility::Hidden {
            if has_generated_box(id) {
                return false;
            }
            stack.extend(&node.children);
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                let text = node.text_content().unwrap_or("");
                if text.chars().any(|c| {
                    !matches!(c, ' ' | '\t' | '\n' | '\r')
                        || match cv.effective_white_space_collapse {
                            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard => false,
                            WhiteSpaceCollapse::PreserveBreaks => matches!(c, '\n' | '\r'),
                            _ => true,
                        }
                }) {
                    return false;
                }
            }
            NodeKind::Element if cv.display == DisplayValue::Contents => {
                if has_generated_box(id) {
                    return false;
                }
                stack.extend(&node.children)
            }
            NodeKind::Element => return false,
            _ => {} // cov:ignore: Comment/PI membership is cleared by mark_in_document_flags; document/fragment roots cannot occur in a valid cell subtree.
        }
    }
    true
}

#[cfg(test)]
mod tests;
