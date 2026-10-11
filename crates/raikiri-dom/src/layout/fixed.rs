//! Fixed-position boxes nested below the body.
//!
//! The body is the taffy root, and its border box is sized to the page
//! area, the containing block of a fixed box in paged media (CSS 2.1 §10.1, CSS
//! Position 3 §2.1). Taffy places every absolutely positioned box against
//! the padding box of its parent, and the inline engine places one inside a
//! paragraph against the paragraph. So a fixed box nested below the body is
//! sized against its parent and offset by the parent's position, and even
//! one that is a direct child of the body is moved by the body's border.
//!
//! [`place_fixed_boxes_in_page_area`] lays out each fixed box again against
//! the page area, as taffy lays out absolutely positioned children,
//! and stores its location relative to its parent so that positions summed
//! along the DOM ancestors land on the page area. An axis whose insets are
//! both `auto` keeps the static position taffy gave it.
//!
//! A transformed or filtered ancestor is the containing block of its fixed
//! descendants (CSS Transforms 1 §3, Filter Effects 1 §5), so those keep the
//! layout taffy gave them against their parent.

use super::*;
use taffy::util::{MaybeMath, MaybeResolve, ResolveOrZero};
use taffy::{Line, Position as TaffyPosition};

/// Whether the element's `transform` applies to it.
fn transformable(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#terminology
    // Non-replaced inline and table-column boxes are not transformable.
    let node = &document.nodes[id];
    !matches!(
        cv.display,
        DisplayValue::TableColumn | DisplayValue::TableColumnGroup
    ) && (cv.display != DisplayValue::Inline
        || node.is_inline_svg_root()
        || matches!(
            node.tag_name(),
            Some("img" | "canvas" | "video" | "iframe" | "object" | "embed")
        ))
}

/// Whether `id` is the containing block of its fixed-position descendants.
fn establishes_fixed_containing_block(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#containing-block-for-all-descendants
    // spec: https://drafts.fxtf.org/filter-effects-1/#FilterProperty
    (transformable(document, id, cv) && !cv.transform.is_empty()) || !cv.filter.is_empty()
}

/// Lay out the fixed boxes below the taffy root `root` against the page area
/// of size `area_size`, which starts at the root's border box origin. Run it after the root
/// layout and before anything reads the positions of those boxes.
pub(crate) fn place_fixed_boxes_in_page_area(
    document: &mut Document,
    cascade: &CascadeResult,
    root: usize,
    area_size: Size<f32>,
) {
    if let Some(cv) = cascade.computed.get(root)
        && establishes_fixed_containing_block(document, root, cv)
    {
        // A transformed or filtered body contains its fixed descendants.
        return;
    }
    // The page area takes the direction of the root element (CSS Writing
    // Modes 4 §8), not an override on the body.
    let direction = match document
        .parent_of(root)
        .and_then(|html| cascade.computed.get(html))
    {
        Some(cv) if cv.direction == Direction::Rtl => TaffyDirection::Rtl,
        Some(_) => TaffyDirection::Ltr,
        None => document.nodes[root].style.direction,
    };
    // (node, the origin its location is relative to, measured from the
    // root's border box, whether it is content of a paragraph)
    let in_paragraph = document.nodes[root].is_ifc_root();
    let mut stack: Vec<(usize, Point<f32>, bool)> = document.nodes[root]
        .children
        .iter()
        .rev()
        .map(|&child| (child, Point::ZERO, in_paragraph))
        .collect();
    while let Some((id, parent_origin, in_paragraph)) = stack.pop() {
        let node = &document.nodes[id];
        if !node.is_in_document() || node.style.display == Display::None {
            continue;
        }
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        if cv.position == PositionValue::Fixed && node.style.position == TaffyPosition::Absolute {
            place_fixed_box(document, id, parent_origin, area_size, direction);
        }
        // The box's own transform or filter contains its descendants, not
        // the box itself.
        if establishes_fixed_containing_block(document, id, cv) {
            continue;
        }
        let node = &document.nodes[id];
        // The inline engine places the boxes of a paragraph relative to the
        // paragraph, not to the inline elements between them.
        let inline = in_paragraph
            && !node.is_ifc_root()
            && cv.display == DisplayValue::Inline
            && !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed);
        let (origin, in_paragraph) = if inline {
            (parent_origin, true)
        } else {
            let location = node.unrounded_layout.location;
            let origin = Point {
                x: parent_origin.x + location.x,
                y: parent_origin.y + location.y,
            };
            (origin, node.is_ifc_root())
        };
        stack.extend(
            node.children
                .iter()
                .rev()
                .map(|&child| (child, origin, in_paragraph)),
        );
    }
}

/// Lay out the absolutely positioned box `id` against the area of size
/// `area_size` at the root's origin, as taffy lays out the
/// absolutely positioned children of a block container, and store its
/// location relative to its parent at `parent_origin`.
fn place_fixed_box(
    tree: &mut Document,
    id: usize,
    parent_origin: Point<f32>,
    area_size: Size<f32>,
    direction: TaffyDirection,
) {
    let calc = crate::taffy_impl::resolve_calc;
    let style = &tree.nodes[id].style;
    let area_width = Some(area_size.width);
    let area_height = Some(area_size.height);
    let aspect_ratio = style.aspect_ratio;
    let margin = style
        .margin
        .map(|margin| margin.resolve_to_option(area_size.width, calc));
    let padding = style.padding.resolve_or_zero(area_width, calc);
    let border = style.border.resolve_or_zero(area_width, calc);
    let padding_border_sum = (padding + border).sum_axes();
    let box_sizing_adjustment = if style.box_sizing == TaffyBoxSizing::ContentBox {
        padding_border_sum
    } else {
        Size::ZERO
    };
    let left = style.inset.left.maybe_resolve(area_width, calc);
    let right = style.inset.right.maybe_resolve(area_width, calc);
    let top = style.inset.top.maybe_resolve(area_height, calc);
    let bottom = style.inset.bottom.maybe_resolve(area_height, calc);
    let size_style = style.size;
    let area = area_size.map(Some);
    let style_size = size_style
        .maybe_resolve(area, calc)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let min_size = style
        .min_size
        .maybe_resolve(area, calc)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment)
        .or(padding_border_sum.map(Some))
        .maybe_max(padding_border_sum);
    let max_size = style
        .max_size
        .maybe_resolve(area, calc)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let mut known_dimensions = style_size.maybe_clamp(min_size, max_size);
    // Taffy resolves sizing keywords of an absolutely positioned box
    // privately; keep the content size it gave such an axis, and add the
    // padding and border as they resolve against the page area, unless an
    // aspect ratio already gave the axis its size.
    let current = tree.nodes[id].unrounded_layout;
    let old_padding_border = (current.padding + current.border).sum_axes();
    if size_style.width.is_sizing_keyword() && known_dimensions.width.is_none() {
        known_dimensions.width =
            Some(current.size.width - old_padding_border.width + padding_border_sum.width);
    }
    if size_style.height.is_sizing_keyword() && known_dimensions.height.is_none() {
        known_dimensions.height =
            Some(current.size.height - old_padding_border.height + padding_border_sum.height);
    }
    if let (None, Some(left), Some(right)) = (known_dimensions.width, left, right) {
        let width = area_size
            .width
            .maybe_sub(margin.left)
            .maybe_sub(margin.right)
            - left
            - right;
        known_dimensions.width = Some(width.max(0.0));
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
    }
    if let (None, Some(top), Some(bottom)) = (known_dimensions.height, top, bottom) {
        let height = area_size
            .height
            .maybe_sub(margin.top)
            .maybe_sub(margin.bottom)
            - top
            - bottom;
        known_dimensions.height = Some(height.max(0.0));
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
    }
    let available_space = Size {
        width: AvailableSpace::Definite(
            area_size.width.maybe_clamp(min_size.width, max_size.width),
        ),
        height: AvailableSpace::Definite(
            area_size
                .height
                .maybe_clamp(min_size.height, max_size.height),
        ),
    };
    let input = |known_dimensions, run_mode| LayoutInput {
        known_dimensions,
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: area,
        available_space,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Both,
        run_mode,
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let node_id = TaffyNodeId::from(id);
    let final_size = match (known_dimensions.width, known_dimensions.height) {
        (Some(width), Some(height)) => Size { width, height },
        _ => {
            let measured = tree
                .compute_child_layout(node_id, input(known_dimensions, RunMode::ComputeSize))
                .size;
            known_dimensions.unwrap_or(measured)
        }
    }
    .maybe_clamp(min_size, max_size);
    let output =
        tree.compute_child_layout(node_id, input(final_size.map(Some), RunMode::PerformLayout));

    // CSS 2.1 §10.3.7 and §10.6.4: auto margins take the free space only
    // when both insets of their axis are set, and are zero otherwise.
    let non_auto_margin = Rect {
        left: if left.is_some() {
            margin.left.unwrap_or(0.0)
        } else {
            0.0
        },
        right: if right.is_some() {
            margin.right.unwrap_or(0.0)
        } else {
            0.0
        },
        top: if top.is_some() {
            margin.top.unwrap_or(0.0)
        } else {
            0.0
        },
        bottom: if bottom.is_some() {
            margin.bottom.unwrap_or(0.0)
        } else {
            0.0
        },
    };
    let free_space = Size {
        width: right.map_or(final_size.width, |right| {
            area_size.width - right - left.unwrap_or(0.0)
        }) - final_size.width
            - non_auto_margin.horizontal_axis_sum(),
        height: bottom.map_or(final_size.height, |bottom| {
            area_size.height - bottom - top.unwrap_or(0.0)
        }) - final_size.height
            - non_auto_margin.vertical_axis_sum(),
    };
    let auto_margin = |insets_set: bool, start: Option<f32>, end: Option<f32>, free: f32| {
        let count = start.is_none() as u8 + end.is_none() as u8;
        if !insets_set || count == 0 || (count == 2 && free <= 0.0) {
            0.0
        } else {
            free / f32::from(count)
        }
    };
    let auto_width = auto_margin(
        left.is_some() && right.is_some(),
        margin.left,
        margin.right,
        free_space.width,
    );
    let auto_height = auto_margin(
        top.is_some() && bottom.is_some(),
        margin.top,
        margin.bottom,
        free_space.height,
    );
    let resolved_margin = Rect {
        left: margin.left.unwrap_or(auto_width),
        right: margin.right.unwrap_or(auto_width),
        top: margin.top.unwrap_or(auto_height),
        bottom: margin.bottom.unwrap_or(auto_height),
    };

    let from_end_x =
        |right: f32| area_size.width - final_size.width - right - resolved_margin.right;
    let page_x = match (left, right) {
        (Some(_), Some(right)) if direction == TaffyDirection::Rtl => Some(from_end_x(right)),
        (Some(left), _) => Some(left + resolved_margin.left),
        (None, Some(right)) => Some(from_end_x(right)),
        (None, None) => None,
    };
    let page_y =
        top.map(|top| top + resolved_margin.top).or(bottom
            .map(|bottom| area_size.height - final_size.height - bottom - resolved_margin.bottom));
    // CSS 2.1 §10.3.7: in a right-to-left static containing block the
    // static position is the box's right edge, which stays put when the box
    // changes width.
    let static_rtl = tree
        .taffy_parent_of(id)
        .is_some_and(|parent| tree.nodes[parent].style.direction == TaffyDirection::Rtl);
    // The margins may resolve differently against the page area than
    // against the parent; the static position is that of the margin box.
    let static_x = if static_rtl {
        current.location.x + current.size.width - final_size.width + current.margin.right
            - resolved_margin.right
    } else {
        current.location.x - current.margin.left + resolved_margin.left
    };
    let static_y = current.location.y - current.margin.top + resolved_margin.top;
    let location = Point {
        x: page_x.map_or(static_x, |x| x - parent_origin.x),
        y: page_y.map_or(static_y, |y| y - parent_origin.y),
    };
    let layout = TaffyLayout {
        location,
        size: final_size,
        scrollable_overflow_rect: output.scrollable_overflow_rect,
        padding,
        border,
        margin: resolved_margin,
        ..current
    };
    tree.set_unrounded_layout(node_id, &layout);
}

#[cfg(test)]
mod tests;
