//! Static positions of absolutely positioned boxes that were inline-level
//! before blockification, among the block-level children of a block
//! container.

use crate::document::Document;
use raikiri_style::property::{Direction, DisplayValue, FloatValue, PositionValue, TextAlign};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto};

/// Move each absolutely positioned box whose `display` was inline-level,
/// and whose horizontal insets are both `auto`, to its static position on
/// the line it would sit on among the block-level children of its block
/// container.
///
/// Taffy's block algorithm places such a box as if it were a block, at the
/// inline-start edge of the content box. Its static position is that of a
/// hypothetical inline box instead (CSS 2.1 §10.3.7): on a line box that
/// floats beside it shorten, aligned by the container's `text-align`. In a
/// right-to-left container that position is the box's right edge.
///
/// Lines of inline content are placed by the inline engine, and this pass
/// only covers a box whose parent is laid out as blocks. Only the floats
/// that precede the box among its siblings shorten the line; floats from
/// other blocks of the same formatting context, and boxes and floats inside
/// a `display: contents` child, are not seen.
///
/// Run it before fixed boxes are placed in the page area: that placement
/// fixes their offsets from their parents, so moving an ancestor afterwards
/// would carry them along.
// spec: https://www.w3.org/TR/CSS2/visudet.html#abs-non-replaced-width
pub(super) fn realign_inline_level_abspos_static_positions(
    document: &mut Document,
    cascade: &CascadeResult,
) {
    for parent_id in 0..document.nodes.len() {
        let parent = &cascade.computed[parent_id];
        let parent_node = &document.nodes[parent_id];
        // Only block containers whose children are blocks; inline content
        // and the inside of inline elements are placed by the inline engine,
        // and the children of a multicol container by its columns.
        if parent_node.ifc.is_some()
            || parent_node.multicol.is_some()
            || crate::taffy_impl::used_writing_mode(document, parent_id).is_vertical()
            || !matches!(
                parent_node.style.display,
                taffy::Display::Block | taffy::Display::FlowRoot
            )
            || !matches!(
                parent.display,
                DisplayValue::Block
                    | DisplayValue::InlineBlock
                    | DisplayValue::FlowRoot
                    | DisplayValue::ListItem
                    | DisplayValue::TableCell
                    | DisplayValue::TableCaption
            )
        {
            continue;
        }
        let rtl = parent.direction == Direction::Rtl;
        // The line box is aligned by the container's text alignment;
        // `justify` does not stretch a line with nothing to justify.
        let share = match parent.text_align {
            TextAlign::Left => 0.0,
            TextAlign::Right => 1.0,
            TextAlign::Center | TextAlign::InternalCenter => 0.5,
            TextAlign::End => {
                if rtl {
                    0.0
                } else {
                    1.0
                }
            }
            _ => {
                if rtl {
                    1.0
                } else {
                    0.0
                }
            }
        };
        let layout = parent_node.unrounded_layout;
        let content_left = layout.border.left + layout.padding.left;
        let content_right = layout.size.width - layout.border.right - layout.padding.right;
        let mut floats: Vec<(FloatValue, taffy::Layout)> = Vec::new();
        for index in 0..document.nodes[parent_id].children.len() {
            let child_id = document.nodes[parent_id].children[index];
            let child = &cascade.computed[child_id];
            let child_layout = document.nodes[child_id].unrounded_layout;
            if matches!(child.float, FloatValue::Left | FloatValue::Right)
                && !matches!(
                    child.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
            {
                floats.push((child.float, child_layout));
                continue;
            }
            if child.position != PositionValue::Absolute
                || !child.display.is_inline_level()
                || !matches!(child.left, ComputedLengthPercentageOrAuto::Auto)
                || !matches!(child.right, ComputedLengthPercentageOrAuto::Auto)
            {
                continue;
            }
            // With both horizontal insets `auto`, auto margins are zero.
            let margin_left = if matches!(child.margin.left, ComputedLengthPercentageOrAuto::Auto) {
                0.0
            } else {
                child_layout.margin.left
            };
            let margin_right = if matches!(child.margin.right, ComputedLengthPercentageOrAuto::Auto)
            {
                0.0
            } else {
                child_layout.margin.right
            };
            // The line sits where the box's top margin edge does. A `top` or
            // `bottom` inset moves the box off its line, whose position is
            // then not known here, so the floats are not consulted.
            let vertical_static = matches!(child.top, ComputedLengthPercentageOrAuto::Auto)
                && matches!(child.bottom, ComputedLengthPercentageOrAuto::Auto);
            let line_y = child_layout.location.y - child_layout.margin.top;
            let (mut left, mut right) = (content_left, content_right);
            for (float, float_layout) in floats.iter().filter(|_| vertical_static) {
                let top = float_layout.location.y - float_layout.margin.top;
                let bottom =
                    float_layout.location.y + float_layout.size.height + float_layout.margin.bottom;
                if !(top <= line_y && line_y < bottom) {
                    continue;
                }
                if *float == FloatValue::Left {
                    left = left.max(
                        float_layout.location.x
                            + float_layout.size.width
                            + float_layout.margin.right,
                    );
                } else {
                    right = right.min(float_layout.location.x - float_layout.margin.left);
                }
            }
            let point = left + (right - left).max(0.0) * share;
            let x = if rtl {
                point - margin_right - child_layout.size.width
            } else {
                point + margin_left
            };
            document.nodes[child_id].unrounded_layout.location.x = x;
        }
    }
}

#[cfg(test)]
mod tests;
