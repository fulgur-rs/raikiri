use super::*;
use raikiri_layout_style::StyleFlags;

pub(super) fn prepare_metadata(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
) -> StyleMetadata {
    let node = &doc.nodes[idx];
    let source = matches!(doc.nodes.box_key(idx), Some(BoxKey::Source(_)));
    let neutral_contents = node.display() == raikiri_style::property::DisplayValue::Contents;
    let mut flags = if source && !neutral_contents {
        super::super::bridge::view_sizing_flags(doc, cascade, idx)
    } else {
        StyleFlags::EMPTY
    };
    if source && !neutral_contents && super::super::bridge::uses_vertical_axes(cascade, idx) {
        flags = flags.union(StyleFlags::SOURCE_VERTICAL_DIRECTION);
    }
    if !neutral_contents && node.taffy_display() == taffy::Display::Block {
        flags = flags.union(StyleFlags::BLOCK);
    }
    if node.taffy_display() == taffy::Display::None {
        flags = flags.union(StyleFlags::HIDDEN);
    }
    if source
        && !neutral_contents
        && node.kind() == NodeKind::Element
        && crate::paint_rules::lays_out_as_replaced(doc, idx)
    {
        flags = flags
            .union(StyleFlags::REPLACED)
            .union(StyleFlags::COMPRESSIBLE_REPLACED);
    }
    if node.size_containment.width {
        flags = flags.union(StyleFlags::CONTAIN_WIDTH);
    }
    if node.size_containment.height {
        flags = flags.union(StyleFlags::CONTAIN_HEIGHT);
    }
    StyleMetadata {
        flags,
        used_writing_mode: node.used_writing_mode,
        claims: None,
    }
}
