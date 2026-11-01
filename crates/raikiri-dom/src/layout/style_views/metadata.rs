use super::super::bridge::*;
use super::*;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::StyleFlags;
use raikiri_layout_style::values::{ContainingBlockClaims, Display};
use raikiri_style::property::DisplayValue;
use raikiri_style::{ComputedLengthPercentageOrAuto, ComputedValues};

pub(super) fn prepare_metadata(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    inline_box: bool,
) -> StyleMetadata {
    let node = &doc.nodes[idx];
    let source = matches!(doc.nodes.box_key(idx), Some(BoxKey::Source(_)));
    let neutral_contents = node.display() == raikiri_style::property::DisplayValue::Contents;
    let mut flags = if source && !neutral_contents {
        sizing_flags(doc, cascade, idx)
    } else {
        StyleFlags::EMPTY
    };
    if source && !neutral_contents && super::super::bridge::uses_vertical_axes(cascade, idx) {
        flags = flags.union(StyleFlags::SOURCE_VERTICAL_DIRECTION);
    }
    if !neutral_contents && node.layout_display() == Display::Block {
        flags = flags.union(StyleFlags::BLOCK);
    }
    if node.layout_display() == Display::None {
        flags = flags.union(StyleFlags::HIDDEN);
    }
    if source
        && !neutral_contents
        && node.kind() == NodeKind::Element
        && is_replaced(doc, cascade, idx)
    {
        flags = flags
            .union(StyleFlags::REPLACED)
            .union(StyleFlags::COMPRESSIBLE_REPLACED);
    }
    if source && !neutral_contents && avoids_floats(doc, idx) {
        flags = flags.union(StyleFlags::AVOIDS_FLOATS);
    }
    if source && !neutral_contents && inline_box {
        flags = flags.union(StyleFlags::INLINE_BOX);
    }
    if node.size_containment.width {
        flags = flags.union(StyleFlags::CONTAIN_WIDTH);
    }
    if node.size_containment.height {
        flags = flags.union(StyleFlags::CONTAIN_HEIGHT);
    }
    node_metadata(node, flags)
}

/// For every node, whether it is an inline box: a non-atomic inline box in a
/// horizontal writing mode that stays inline. The width, height, minimum and
/// maximum sizes, vertical margins and vertical padding and border of such a
/// box do not apply to it (CSS 2.1 sections 8.3, 10.3.1 and 10.6.1), whatever
/// it holds: text, a block that splits it (section 9.2.1.1), or nothing.
///
/// An inline box with no content that makes a line is laid out as a block in
/// the block flow around it, so these values would otherwise size it.
pub(in crate::layout) fn inline_boxes(doc: &Document, cascade: &CascadeResult) -> Vec<bool> {
    let mut boxes = vec![false; doc.nodes.len()];
    super::super::multicol::visit_top_down(doc, |_, id| {
        boxes[id] = is_plain_inline_box(doc, cascade, id);
    });
    boxes
}

/// Whether `idx` is an inline box of a horizontal writing mode that stays
/// inline: not blockified, not replaced and not inline SVG.
fn is_plain_inline_box(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let node = &doc.nodes[idx];
    node.kind() == NodeKind::Element
        && !node.used_writing_mode.is_vertical()
        && !node.is_inline_svg_content()
        && super::super::fixed::generates_inline_box(doc, idx, &cascade.computed[idx])
}

pub(in crate::layout) fn node_metadata(
    node: &crate::node::Node,
    flags: StyleFlags,
) -> StyleMetadata {
    use crate::node::FormattingContext as C;
    let flags = if node.display() == DisplayValue::TableCell {
        flags.union(StyleFlags::TABLE_CELL)
    } else {
        flags
    };
    let flags = flags.union(match node.formatting_context {
        C::None => StyleFlags::HIDDEN,
        C::Inline => StyleFlags::IFC,
        C::Flex => StyleFlags::FLEX,
        C::Grid => StyleFlags::GRID,
        C::Table => StyleFlags::TABLE,
        C::Multicol => StyleFlags::MULTICOL,
        C::Text => StyleFlags::TEXT,
        C::Block => StyleFlags::EMPTY,
    });
    let flags = if matches!(
        node.display(),
        DisplayValue::Table | DisplayValue::InlineTable
    ) {
        flags.union(StyleFlags::OOF_TABLE)
    } else {
        flags
    };
    StyleMetadata {
        flags,
        used_writing_mode: node.used_writing_mode,
        claims: Some(ContainingBlockClaims {
            absolute: node.claims_absolute_oof || node.layout_position().is_positioned(),
            fixed: node.claims_fixed_oof,
        }),
    }
}

pub(in crate::layout) fn sizing_flags(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
) -> raikiri_layout_style::StyleFlags {
    use raikiri_layout_style::StyleFlags as F;
    if doc.nodes[idx].kind() != NodeKind::Element {
        return F::EMPTY;
    }
    let cv = &cascade.computed[idx];
    let mut flags = F::EMPTY;
    let content_keyword = |size: &ComputedLengthPercentageOrAuto| {
        matches!(
            size,
            ComputedLengthPercentageOrAuto::MaxContent
                | ComputedLengthPercentageOrAuto::FitContent
                | ComputedLengthPercentageOrAuto::FitContentOf(_)
        )
    };
    let logical = vertical_logical_size(cascade, idx);
    let width = logical.as_ref().map_or(&cv.width, |size| &size.width);
    let height = logical.as_ref().map_or(&cv.height, |size| &size.height);
    if logical.is_some() {
        flags = flags.union(F::VERTICAL_LOGICAL_SIZE);
    }
    if min_content_width_is_auto(doc, cascade, idx) {
        flags = flags.union(F::MIN_CONTENT_WIDTH_AUTO);
    }
    if (content_keyword(width) || (logical.is_some() && content_keyword(height)))
        && !sized_by_content_keyword(doc, cascade, idx, cv)
    {
        if content_keyword(width) {
            flags = flags.union(F::IGNORE_WIDTH_KEYWORD);
        }
        if logical.is_some() && content_keyword(height) {
            flags = flags.union(F::IGNORE_HEIGHT_KEYWORD);
        }
    }
    if !honors_content_limits(doc, cascade, idx, cv) {
        flags = flags.union(F::IGNORE_LIMIT_KEYWORDS);
    }
    if cv.min_block_size.as_ref().is_some_and(is_sizing_keyword)
        && (is_supported_flex_grid_item(doc, cascade, idx, cv)
            || is_supported_flex_grid_container(doc, idx, cv))
    {
        flags = flags.union(F::IGNORE_LOGICAL_MIN_BLOCK);
    }
    flags
}

/// The physical axes in which `contain` sizes the box as if it had no
/// content (CSS Containment 2 §3.1 size containment, §3.2 inline-size
/// containment).
///
/// Neither applies to a box that is not generated, to a non-atomic inline
/// box, or to an internal table box (§3.1, §3.2), which a table caption is
/// not. Tables are left out as well: their width algorithm sizes the grid
/// from its cells. Inline-size containment acts on the inline axis of the
/// box's own writing mode.
pub(in crate::layout) fn size_containment(
    doc: &Document,
    idx: usize,
    cv: &ComputedValues,
) -> Size<bool> {
    use raikiri_style::property::Contain;

    let none = Size {
        width: false,
        height: false,
    };
    let applies = (cv.display.is_flex_or_grid_container()
        || (cv.display.is_inline_flow_box()
            || matches!(
                cv.display,
                DisplayValue::Block
                    | DisplayValue::FlowRoot
                    | DisplayValue::ListItem
                    | DisplayValue::TableCaption
                    | DisplayValue::BlockRuby
            )))
        && !super::super::fixed::generates_inline_box(doc, idx, cv);
    if !applies {
        return none;
    }
    if cv.contain.contains(Contain::SIZE) {
        return Size {
            width: true,
            height: true,
        };
    }
    if !cv.contain.contains(Contain::INLINE_SIZE) {
        return none;
    }
    let vertical = crate::layout::dispatch::used_writing_mode(doc, idx).is_vertical();
    Size {
        width: !vertical,
        height: vertical,
    }
}

/// Whether the element, as a block-level box, keeps its border box clear of
/// the margin boxes of the floats in its formatting context: a replaced
/// element, whether or not it has a natural size, and the form controls that
/// browsers lay out as one (CSS 2.1 section 9.5). An `<object>` shows its
/// fallback content here, as an ordinary block, because nothing loads into it.
fn avoids_floats(doc: &Document, idx: usize) -> bool {
    let tag = doc.nodes[idx].tag_name();
    (tag != Some("object") && crate::paint_rules::is_replaced_element(doc, idx))
        || matches!(tag, Some("input" | "select" | "textarea" | "button"))
}

pub(in crate::layout) fn is_replaced(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    if crate::paint_rules::lays_out_as_replaced(doc, idx) {
        return true;
    }
    if doc.nodes[idx].tag_name() != Some("img") {
        return false;
    }
    use ComputedLengthPercentageOrAuto as L;
    use raikiri_layout_style::StyleFlags as F;
    let cv = &cascade.computed[idx];
    let flags = sizing_flags(doc, cascade, idx);
    let logical = vertical_logical_size(cascade, idx);
    let width = logical.as_ref().map_or(&cv.width, |size| &size.width);
    let height = logical.as_ref().map_or(&cv.height, |size| &size.height);
    let width = if flags.contains(F::IGNORE_WIDTH_KEYWORD)
        || (flags.contains(F::MIN_CONTENT_WIDTH_AUTO) && *width == L::MinContent)
    {
        &L::Auto
    } else {
        width
    };
    let height = if flags.contains(F::IGNORE_HEIGHT_KEYWORD) {
        &L::Auto
    } else {
        height
    };
    bridge_known_image_intrinsic_size(doc, idx, width, height).is_some()
}
