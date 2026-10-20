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

pub(crate) fn neutral_initial_values() -> ComputedValues {
    use std::sync::OnceLock;
    static EMPTY_TRACKS: OnceLock<Arc<Vec<raikiri_style::ComputedGridTrackSize>>> = OnceLock::new();
    let mut values = ComputedValues::initial();
    values.box_sizing = raikiri_style::property::BoxSizing::BorderBox;
    let empty = EMPTY_TRACKS.get_or_init(|| Arc::new(Vec::new()));
    values.grid_auto_columns = empty.clone();
    values.grid_auto_rows = empty.clone();
    values
}

pub(super) fn neutral_source_owner() -> Arc<ComputedValues> {
    use std::sync::OnceLock;
    static VALUES: OnceLock<Arc<ComputedValues>> = OnceLock::new();
    VALUES
        .get_or_init(|| Arc::new(neutral_initial_values()))
        .clone()
}

pub(super) fn neutral_values(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
) -> ComputedValues {
    use raikiri_style::property::Direction;
    let mut values = neutral_initial_values();
    let key = doc.nodes.box_key(idx).expect("recipe has a box key");
    let source = match key {
        BoxKey::Source(_) if doc.nodes[idx].kind() == NodeKind::Element => Some(idx),
        BoxKey::AnonymousCell(_) | BoxKey::AnonymousTable(_) => doc
            .ancestors(idx)
            .find(|&ancestor| matches!(doc.nodes.box_key(ancestor), Some(BoxKey::Source(_)))),
        _ => None,
    };
    if let Some(source) = source {
        let inherited = &cascade.computed[source];
        let direction = super::super::bridge::direction_for_axes(
            inherited,
            super::super::bridge::uses_vertical_axes(cascade, source),
        );
        values.direction = match direction {
            taffy::Direction::Ltr => Direction::Ltr,
            taffy::Direction::Rtl => Direction::Rtl,
        };
        if !matches!(key, BoxKey::Source(_)) {
            values.border_collapse = inherited.border_collapse;
            values.border_spacing = inherited.border_spacing;
            values.caption_side = inherited.caption_side;
        }
    }
    values
}
