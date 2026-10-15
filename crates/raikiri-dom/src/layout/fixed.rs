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
//! descendants (CSS Transforms 1 §3, Filter Effects 1 §5), and a positioned
//! ancestor that of its absolutely positioned descendants (CSS 2.1 §10.1).
//! When that ancestor is not the box's parent, the box is laid out again the
//! same way against the ancestor's padding box. A containing block that is an
//! inline box keeps the layout taffy gave.
//!
//! An absolutely positioned box with no positioned, transformed or filtered
//! ancestor has the initial containing block as its containing block (CSS 2.1
//! §10.1), which in paged media has the size of the page area and starts at
//! the root's border box origin like the page area of the first page. Taffy
//! places such a box against its parent too, so it is laid out again here
//! the same way.

use super::*;
use raikiri_style::property::WillChange;
use taffy::util::{MaybeMath, MaybeResolve, ResolveOrZero};
use taffy::{
    AlignContentKeyword, AlignItemsKeyword, AlignmentSafety, Line, Position as TaffyPosition,
};

/// Whether the element generates a non-atomic inline box: an inline box that
/// was not blockified and is not a replaced element or form control.
pub fn generates_inline_box(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
    // spec: https://www.w3.org/TR/css-display-3/#transformations
    // Out-of-flow boxes and flex and grid items are blockified.
    let node = &document.nodes[id];
    let tag = node.tag_name().unwrap_or("");
    cv.display.is_inline_flow()
        && !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        && cv.float == FloatValue::None
        && !node.is_inline_svg_root()
        && !ATOMIC_TAGS.contains(&tag)
        && !REPLACED_BOX_TAGS.contains(&tag)
        && !document.taffy_parent_of(id).is_some_and(|parent| {
            matches!(
                document.nodes[parent].style.display,
                Display::Flex | Display::Grid
            )
        })
}

/// Whether the element's `transform` applies to it: layout makes it the
/// containing block of its fixed descendants, and the painter moves it.
pub fn is_transformable(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#terminology
    // Non-replaced inline and table-column boxes are not transformable.
    !matches!(
        cv.display,
        DisplayValue::TableColumn | DisplayValue::TableColumnGroup
    ) && !generates_inline_box(document, id, cv)
}

/// Whether layout and paint containment apply to the element.
fn containable(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
    // They have no effect on internal table boxes other than cells, and on
    // non-atomic inline boxes.
    !matches!(
        cv.display,
        DisplayValue::Contents
            | DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup
    ) && !generates_inline_box(document, id, cv)
}

/// Whether `id` is the containing block of its fixed-position descendants.
fn establishes_fixed_containing_block(document: &Document, id: usize, cv: &ComputedValues) -> bool {
    // spec: https://www.w3.org/TR/css-transforms-1/#containing-block-for-all-descendants
    // spec: https://drafts.fxtf.org/filter-effects-1/#FilterProperty
    // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
    // spec: https://www.w3.org/TR/css-will-change-1/#will-change
    // A box that is not generated contains nothing (CSS Display 3 §2.5).
    let transformable = is_transformable(document, id, cv);
    let containable = containable(document, id, cv);
    // A filter on the document root does not make it a containing block.
    let root = document
        .parent_of(id)
        .is_some_and(|parent| document.parent_of(parent).is_none());
    cv.display != DisplayValue::Contents
        && ((transformable
            && (!cv.transform.is_empty() || cv.will_change.contains(WillChange::TRANSFORM)))
            || (!root && (!cv.filter.is_empty() || cv.will_change.contains(WillChange::FILTER)))
            || (containable
                && (cv.contain.contains_positioned_descendants()
                    || cv.will_change.contains(WillChange::CONTAIN))))
}

/// Whether `id` is the containing block of its absolutely positioned
/// descendants.
pub(super) fn establishes_absolute_containing_block(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
) -> bool {
    // spec: https://www.w3.org/TR/CSS21/visudet.html#containing-block-details
    // spec: https://www.w3.org/TR/css-display-3/#box-generation
    (cv.display != DisplayValue::Contents
        && (cv.position != PositionValue::Static || cv.will_change.contains(WillChange::POSITION)))
        || establishes_fixed_containing_block(document, id, cv)
}

/// The containing block an out-of-flow box is laid out against.
#[derive(Clone, Copy)]
enum ContainingBlock {
    /// The page area, which also stands in for the initial containing block.
    Page,
    /// The padding box of the box `id`, at `origin` from the root's border
    /// box origin.
    Box {
        id: usize,
        origin: Point<f32>,
        size: Size<f32>,
        direction: TaffyDirection,
    },
    /// A containing block the layout taffy gave stays against: an inline box,
    /// or an ancestor of the root.
    Kept,
}

/// The area an out-of-flow box is placed in: its origin, measured from the
/// root's border box origin, its size, its direction, and whether its
/// writing mode is vertical.
#[derive(Clone, Copy)]
struct Area {
    origin: Point<f32>,
    size: Size<f32>,
    direction: TaffyDirection,
    vertical: bool,
}

/// Whether the writing mode `id` lays its lines out in is vertical.
fn is_vertical(document: &Document, id: usize) -> bool {
    !matches!(
        crate::taffy_impl::used_writing_mode(document, id),
        WritingMode::HorizontalTb
    )
}

/// Lay out the fixed boxes below the taffy root `root`, and the nested
/// absolutely positioned boxes whose containing block is the initial
/// containing block, against the page area of size `area_size`, which starts
/// at the root's border box origin. Out-of-flow boxes whose containing block
/// is an ancestor other than their parent are laid out again against that
/// ancestor's padding box. Run it after the root layout and before anything
/// reads the positions of those boxes.
pub(crate) fn place_fixed_boxes_in_page_area(
    document: &mut Document,
    cascade: &CascadeResult,
    root: usize,
    area_size: Size<f32>,
) {
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
    // So does its writing mode.
    let page = Area {
        origin: Point::ZERO,
        size: area_size,
        direction,
        vertical: is_vertical(document, document.parent_of(root).unwrap_or(root)),
    };
    visit_out_of_flow_boxes(
        document,
        cascade,
        root,
        |document, id, parent_origin, cb| {
            let area = match cb {
                ContainingBlock::Page => page,
                ContainingBlock::Box {
                    id: cb,
                    origin,
                    size,
                    direction,
                } => Area {
                    origin,
                    size,
                    direction,
                    vertical: is_vertical(document, cb),
                },
                ContainingBlock::Kept => return, // cov:ignore: kept boxes are not visited
            };
            place_fixed_box(document, id, parent_origin, area);
        },
    );
}

/// Restore explicit fixed block insets after pagination moves ancestors,
/// preserving the box's size, descendants, and existing layout corrections.
pub(super) fn restore_fixed_box_block_positions(
    document: &mut Document,
    cascade: &CascadeResult,
    root: usize,
    area_height: f32,
) {
    visit_out_of_flow_boxes(
        document,
        cascade,
        root,
        |document, id, parent_origin, cb| {
            if !matches!(cb, ContainingBlock::Page) {
                return;
            }
            let style = &document.nodes[id].style;
            let layout = document.nodes[id].unrounded_layout;
            let calc = crate::taffy_impl::resolve_calc;
            let top = style.inset.top.maybe_resolve(Some(area_height), calc);
            let bottom = style.inset.bottom.maybe_resolve(Some(area_height), calc);
            let page_y = top.map(|top| top + layout.margin.top).or_else(|| {
                bottom
                    .map(|bottom| area_height - layout.size.height - bottom - layout.margin.bottom)
            });
            if let Some(page_y) = page_y {
                document.nodes[id].unrounded_layout.location.y = page_y - parent_origin.y;
            }
        },
    );
}

/// The containing block `id` establishes for its descendants, whose box
/// starts at `origin` from the root's border box origin.
fn containing_block_of(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    origin: Point<f32>,
) -> ContainingBlock {
    if generates_inline_box(document, id, cv) {
        return ContainingBlock::Kept;
    }
    let layout = document.nodes[id].unrounded_layout;
    let border = layout.border;
    ContainingBlock::Box {
        id,
        origin: Point {
            x: origin.x + border.left,
            y: origin.y + border.top,
        },
        size: Size {
            width: (layout.size.width - border.left - border.right).max(0.0),
            height: (layout.size.height - border.top - border.bottom).max(0.0),
        },
        direction: if cv.direction == Direction::Rtl {
            TaffyDirection::Rtl
        } else {
            TaffyDirection::Ltr
        },
    }
}

/// Visit the fixed and absolutely positioned boxes below the root whose
/// layout taffy got wrong: those whose containing block is the page area or
/// the initial containing block, and those whose containing block is an
/// ancestor other than their taffy parent. Each is visited before its
/// descendants, with the origin its location is relative to and its
/// containing block.
fn visit_out_of_flow_boxes(
    document: &mut Document,
    cascade: &CascadeResult,
    root: usize,
    mut visit: impl FnMut(&mut Document, usize, Point<f32>, ContainingBlock),
) {
    let root_cv = cascade.computed.get(root);
    // A transformed or filtered body contains its fixed descendants.
    let fixed_cb = match root_cv {
        Some(cv) if establishes_fixed_containing_block(document, root, cv) => {
            containing_block_of(document, root, cv, Point::ZERO)
        }
        _ => ContainingBlock::Page,
    };
    // Taffy places absolutely positioned boxes against their taffy parent;
    // those that no ancestor contains are placed again against the initial
    // containing block, which the page area stands in for.
    let abs_cb = match root_cv {
        Some(cv) if establishes_absolute_containing_block(document, root, cv) => {
            containing_block_of(document, root, cv, Point::ZERO)
        }
        _ if std::iter::successors(document.parent_of(root), |&id| document.parent_of(id)).any(
            |id| {
                cascade
                    .computed
                    .get(id)
                    .is_some_and(|cv| establishes_absolute_containing_block(document, id, cv))
            },
        ) =>
        {
            ContainingBlock::Kept
        }
        _ => ContainingBlock::Page,
    };
    // (node, the origin its location is relative to, measured from the
    // root's border box, whether it is content of a paragraph, the
    // containing blocks of absolutely positioned and fixed descendants)
    let in_paragraph = document.nodes[root].is_ifc_root();
    let mut stack: Vec<(usize, Point<f32>, bool, ContainingBlock, ContainingBlock)> = document
        .nodes[root]
        .children
        .iter()
        .rev()
        .map(|&child| (child, Point::ZERO, in_paragraph, abs_cb, fixed_cb))
        .collect();
    while let Some((id, parent_origin, in_paragraph, abs_cb, fixed_cb)) = stack.pop() {
        let node = &document.nodes[id];
        if !node.is_in_document() || node.style.display == Display::None {
            continue;
        }
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        let out_of_flow = node.style.position == TaffyPosition::Absolute;
        let cb = match cv.position {
            PositionValue::Fixed => fixed_cb,
            PositionValue::Absolute => abs_cb,
            _ => ContainingBlock::Kept,
        };
        let visited = match cb {
            ContainingBlock::Page => true,
            // Taffy and the inline engine place a box against its parent.
            ContainingBlock::Box { id: cb_id, .. } => document.taffy_parent_of(id) != Some(cb_id),
            ContainingBlock::Kept => false,
        };
        if out_of_flow && visited {
            visit(document, id, parent_origin, cb);
        }
        let node = &document.nodes[id];
        // The inline engine places the boxes of a paragraph relative to the
        // paragraph, not to the inline elements between them.
        let inline = in_paragraph
            && !node.is_ifc_root()
            && cv.display.is_inline_flow()
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
        // The box's own transform or filter contains its descendants, not
        // the box itself.
        let contains_fixed = establishes_fixed_containing_block(document, id, cv);
        let abs_cb = if contains_fixed || establishes_absolute_containing_block(document, id, cv) {
            containing_block_of(document, id, cv, origin)
        } else {
            abs_cb
        };
        let fixed_cb = if contains_fixed { abs_cb } else { fixed_cb };
        stack.extend(
            node.children
                .iter()
                .rev()
                .map(|&child| (child, origin, in_paragraph, abs_cb, fixed_cb)),
        );
    }
}

/// Lay out the absolutely positioned box `id` against `area`, as taffy lays
/// out the absolutely positioned children of a block container, and store
/// its location relative to its parent at `parent_origin`.
fn place_fixed_box(tree: &mut Document, id: usize, parent_origin: Point<f32>, area: Area) {
    let Area {
        origin: area_origin,
        size: area_size,
        direction,
        vertical,
    } = area;
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
    let current = tree.nodes[id].unrounded_layout;
    // A sizing keyword measures the box again against the page area, and
    // the min and max sizes resolved against it clamp the result, unless an
    // aspect ratio already gave the axis its size.
    if size_style.width.is_sizing_keyword() || size_style.height.is_sizing_keyword() {
        let insets = Rect {
            left,
            right,
            top,
            bottom,
        };
        resolve_sizing_keywords(
            tree,
            id,
            &mut known_dimensions,
            size_style,
            area_size,
            insets,
            margin,
        );
        known_dimensions = known_dimensions
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
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
    // Two auto margins share the free space equally. Only the rule for the
    // inline axis of the containing block (§10.3.7) zeroes them when that
    // would make them negative; the block axis rule (§10.6.4) has no such
    // exception (CSS Positioned Layout 3 §4.1).
    let auto_margin =
        |insets_set: bool, start: Option<f32>, end: Option<f32>, free: f32, zero_negative: bool| {
            let count = start.is_none() as u8 + end.is_none() as u8;
            if !insets_set || count == 0 || (count == 2 && zero_negative && free <= 0.0) {
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
        !vertical,
    );
    let auto_height = auto_margin(
        top.is_some() && bottom.is_some(),
        margin.top,
        margin.bottom,
        free_space.height,
        vertical,
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
    // The margins may resolve differently against the page area than
    // against the parent; the static position is that of the margin box,
    // which keeps the edge or center its parent aligns it by.
    let margin_box = |size: Size<f32>, margin: Rect<f32>| Size {
        width: size.width + margin.horizontal_axis_sum(),
        height: size.height + margin.vertical_axis_sum(),
    };
    let old_margin_box = margin_box(current.size, current.margin);
    let new_margin_box = margin_box(final_size, resolved_margin);
    // The margin box sits at `alignment * (space - margin box)` from the
    // start of the space its parent aligns it in; the alignment itself may
    // change when the new size overflows that space.
    let (old_alignment, space) = static_position_alignment(tree, id, old_margin_box);
    let (new_alignment, _) = static_position_alignment(tree, id, new_margin_box);
    let shift = |space: f32, old: f32, new: f32, old_alignment: f32, new_alignment: f32| {
        new_alignment * (space - new) - old_alignment * (space - old)
    };
    let static_x = current.location.x - current.margin.left
        + resolved_margin.left
        + shift(
            space.width,
            old_margin_box.width,
            new_margin_box.width,
            old_alignment.width,
            new_alignment.width,
        );
    let static_y = current.location.y - current.margin.top
        + resolved_margin.top
        + shift(
            space.height,
            old_margin_box.height,
            new_margin_box.height,
            old_alignment.height,
            new_alignment.height,
        );
    let location = Point {
        x: page_x.map_or(static_x, |x| area_origin.x + x - parent_origin.x),
        y: page_y.map_or(static_y, |y| area_origin.y + y - parent_origin.y),
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

/// How a sizing keyword of an absolutely positioned box resolves in one axis.
enum KeywordSize {
    /// Measure the box under this available space.
    Measure(AvailableSpace),
    /// The box takes this size without measuring.
    Exact(f32),
}

/// Resolve a sizing keyword against the containing block, whose stretch size
/// in this axis is `stretch` and whose percentage basis is `basis`.
fn resolve_sizing_keyword(style: Dimension, stretch: f32, basis: f32) -> Option<KeywordSize> {
    // spec: https://www.w3.org/TR/css-sizing-3/#sizing-values
    // spec: https://drafts.csswg.org/css-sizing-4/#stretch-fit-sizing
    match style.tag() {
        CompactLength::MIN_CONTENT_TAG => Some(KeywordSize::Measure(AvailableSpace::MinContent)),
        CompactLength::MAX_CONTENT_TAG => Some(KeywordSize::Measure(AvailableSpace::MaxContent)),
        CompactLength::FIT_CONTENT_PX_TAG => Some(KeywordSize::Measure(AvailableSpace::Definite(
            style.value(),
        ))),
        CompactLength::FIT_CONTENT_PERCENT_TAG => Some(KeywordSize::Measure(
            AvailableSpace::Definite(basis * style.value()),
        )),
        CompactLength::FIT_CONTENT_KEYWORD_TAG => {
            Some(KeywordSize::Measure(AvailableSpace::Definite(stretch)))
        }
        CompactLength::STRETCH_TAG => Some(KeywordSize::Exact(stretch)),
        _ => None,
    }
}

/// Fill in the axes of `known_dimensions` that a sizing keyword sizes, by
/// measuring the box `id` against the containing block of size `area_size`,
/// as taffy sizes an absolutely positioned box against its parent.
fn resolve_sizing_keywords(
    tree: &mut Document,
    id: usize,
    known_dimensions: &mut Size<Option<f32>>,
    size_style: Size<Dimension>,
    area_size: Size<f32>,
    inset: Rect<Option<f32>>,
    margin: Rect<Option<f32>>,
) {
    // The stretch size is the containing block less the insets and margins.
    let stretch = Size {
        width: (area_size.width
            - inset.left.unwrap_or(0.0)
            - inset.right.unwrap_or(0.0)
            - margin.left.unwrap_or(0.0)
            - margin.right.unwrap_or(0.0))
        .max(0.0),
        height: (area_size.height
            - inset.top.unwrap_or(0.0)
            - inset.bottom.unwrap_or(0.0)
            - margin.top.unwrap_or(0.0)
            - margin.bottom.unwrap_or(0.0))
        .max(0.0),
    };
    let width = known_dimensions
        .width
        .is_none()
        .then(|| resolve_sizing_keyword(size_style.width, stretch.width, area_size.width))
        .flatten();
    let height = known_dimensions
        .height
        .is_none()
        .then(|| resolve_sizing_keyword(size_style.height, stretch.height, area_size.height))
        .flatten();
    let node_id = TaffyNodeId::from(id);
    let measure = |tree: &mut Document, known, available, axis| {
        tree.compute_child_layout(
            node_id,
            LayoutInput {
                known_dimensions: known,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: area_size.map(Some),
                available_space: available,
                sizing_mode: SizingMode::ContentSize,
                axis,
                run_mode: RunMode::ComputeSize,
                vertical_margins_are_collapsible: Line::FALSE,
            },
        )
        .size
    };
    if let (Some(KeywordSize::Measure(width)), Some(KeywordSize::Measure(height))) =
        (&width, &height)
    {
        let size = measure(
            tree,
            Size::NONE,
            Size {
                width: *width,
                height: *height,
            },
            RequestedAxis::Both,
        );
        *known_dimensions = size.map(Some);
        return;
    }
    if let Some(width) = width {
        known_dimensions.width = Some(match width {
            KeywordSize::Exact(width) => width,
            KeywordSize::Measure(available) => {
                measure(
                    tree,
                    *known_dimensions,
                    Size {
                        width: available,
                        height: AvailableSpace::Definite(stretch.height),
                    },
                    RequestedAxis::Horizontal,
                )
                .width
            }
        });
    }
    if let Some(height) = height {
        known_dimensions.height = Some(match height {
            KeywordSize::Exact(height) => height,
            KeywordSize::Measure(available) => {
                let width = known_dimensions.width.unwrap_or(stretch.width);
                measure(
                    tree,
                    *known_dimensions,
                    Size {
                        width: AvailableSpace::Definite(width),
                        height: available,
                    },
                    RequestedAxis::Vertical,
                )
                .height
            }
        });
    }
}

/// How the parent of the absolutely positioned box `id` aligns a margin box
/// of size `margin_box` at its static position, in each axis: the share of
/// the free space before the box (0 at the start edge, 1 at the end edge and
/// 0.5 centered) and the space the box is aligned in.
fn static_position_alignment(
    tree: &Document,
    id: usize,
    margin_box: Size<f32>,
) -> (Size<f32>, Size<f32>) {
    let Some(parent) = tree.taffy_parent_of(id) else {
        return (Size::ZERO, Size::ZERO); // cov:ignore: a placed box has a parent
    };
    let parent_style = &tree.nodes[parent].style;
    let rtl = parent_style.direction == TaffyDirection::Rtl;
    // CSS 2.1 §10.3.7: in a right-to-left static containing block the
    // static position is the box's right edge.
    let block = Size {
        width: if rtl { 1.0 } else { 0.0 },
        height: 0.0,
    };
    if parent_style.display != Display::Flex {
        // The alignment does not depend on the size, so neither does the
        // shift.
        return (block, Size::ZERO);
    }
    // CSS Flexbox 1 §4.1: the static position of an absolutely positioned
    // child of a flex container is that of the sole flex item, aligned by
    // the container's justify-content and the child's align-self.
    // spec: https://www.w3.org/TR/css-flexbox-1/#abspos-items
    let (is_row, reverse) = match parent_style.flex_direction {
        TaffyFlexDirection::Row => (true, false),
        TaffyFlexDirection::RowReverse => (true, true),
        TaffyFlexDirection::Column => (false, false),
        TaffyFlexDirection::ColumnReverse => (false, true),
    };
    let main_is_rtl = is_row && rtl;
    let cross_is_rtl = !is_row && rtl;
    let main_reversed = reverse ^ main_is_rtl;
    let cross_reversed = (parent_style.flex_wrap == TaffyFlexWrap::WrapReverse) ^ cross_is_rtl;
    let flex = |end: bool, reversed: bool| if end ^ reversed { 1.0 } else { 0.0 };
    let layout = tree.nodes[parent].unrounded_layout;
    let insets = (layout.padding + layout.border).sum_axes();
    let space = Size {
        width: layout.size.width - insets.width,
        height: layout.size.height - insets.height,
    };
    let (main_space, main_size, cross_space, cross_size) = if is_row {
        (
            space.width,
            margin_box.width,
            space.height,
            margin_box.height,
        )
    } else {
        (
            space.height,
            margin_box.height,
            space.width,
            margin_box.width,
        )
    };
    // CSS Box Alignment 3 §4.4: safe alignment that would overflow the
    // container aligns as start, and so do the distributed values, whose
    // fallback is safe, for an overflowing box.
    // spec: https://www.w3.org/TR/css-align-3/#overflow-values
    // spec: https://www.w3.org/TR/css-align-3/#distribution-values
    let justify = parent_style
        .justify_content
        .unwrap_or(TaffyAlignContent::FLEX_START);
    // `start` and `end` follow the writing mode; `flex-start`, `flex-end`
    // and the fallbacks of the distributed values follow the flex direction.
    let main_overflows = main_size > main_space;
    let main_keyword = match justify.keyword() {
        AlignContentKeyword::SpaceAround | AlignContentKeyword::SpaceEvenly if main_overflows => {
            AlignContentKeyword::Start
        }
        _ if justify.safety == AlignmentSafety::Safe && main_overflows => {
            AlignContentKeyword::Start
        }
        keyword => keyword,
    };
    let main = match main_keyword {
        AlignContentKeyword::Start => flex(false, main_is_rtl),
        AlignContentKeyword::End => flex(true, main_is_rtl),
        AlignContentKeyword::Center
        | AlignContentKeyword::SpaceAround
        | AlignContentKeyword::SpaceEvenly => 0.5,
        AlignContentKeyword::FlexEnd => flex(true, main_reversed),
        AlignContentKeyword::FlexStart
        | AlignContentKeyword::Stretch
        | AlignContentKeyword::SpaceBetween => flex(false, main_reversed),
    };
    let style = &tree.nodes[id].style;
    let align = style
        .align_self
        .or(parent_style.align_items)
        .unwrap_or(TaffyAlignItems::STRETCH);
    // `self-start` and `self-end` follow the box's own direction where the
    // cross axis is its inline axis.
    let flip = !is_row && style.direction != parent_style.direction;
    let keyword = if align.safety == AlignmentSafety::Safe && cross_size > cross_space {
        AlignItemsKeyword::Start
    } else {
        align.keyword
    };
    let cross = match keyword {
        AlignItemsKeyword::Start | AlignItemsKeyword::Baseline => flex(false, cross_is_rtl),
        AlignItemsKeyword::End => flex(true, cross_is_rtl),
        AlignItemsKeyword::SelfStart => flex(flip, cross_is_rtl),
        AlignItemsKeyword::SelfEnd => flex(!flip, cross_is_rtl),
        AlignItemsKeyword::Center => 0.5,
        AlignItemsKeyword::FlexEnd => flex(true, cross_reversed),
        AlignItemsKeyword::FlexStart | AlignItemsKeyword::Stretch => flex(false, cross_reversed),
    };
    let alignment = if is_row {
        Size {
            width: main,
            height: cross,
        }
    } else {
        Size {
            width: cross,
            height: main,
        }
    };
    (alignment, space)
}

#[cfg(test)]
mod tests;
