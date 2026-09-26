use super::*;

#[test]
fn block_entry_default_pins_css_initials() {
    let entry = BlockEntry::default();
    assert_eq!(entry.background_color, (0, 0, 0, 0));
    assert_eq!(entry.border_widths, (0.0, 0.0, 0.0, 0.0));
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
    assert_eq!(entry.id, None);
    assert_eq!(entry.layout_size, None);
    assert!(entry.clip_descendants.is_empty());
    assert!(entry.opacity_descendants.is_empty());
}

#[test]
fn paragraph_entry_default_pins_css_initials() {
    let entry = ParagraphEntry::default();
    assert_eq!(entry.line_count, 0);
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
    assert_eq!(entry.id, None);
}

#[test]
fn image_entry_default() {
    let entry = ImageEntry::default();
    assert_eq!(entry.width, 0.0);
    assert_eq!(entry.height, 0.0);
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
}

#[test]
fn svg_entry_default() {
    let entry = SvgEntry::default();
    assert_eq!(entry.width, 0.0);
    assert_eq!(entry.height, 0.0);
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
}

#[test]
fn table_entry_default() {
    let entry = TableEntry::default();
    assert_eq!(entry.background_color, (0, 0, 0, 0));
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
    assert_eq!(entry.width, 0.0);
    assert_eq!(entry.cached_height, 0.0);
    assert!(entry.clip_descendants.is_empty());
}

#[test]
fn list_item_entry_default() {
    let entry = ListItemEntry::default();
    assert_eq!(entry.marker_line_height, 0.0);
    assert_eq!(entry.opacity, 1.0);
    assert!(entry.visible);
}

#[test]
fn transform_entry_default_is_identity() {
    let entry = TransformEntry::default();
    assert_eq!(entry.matrix, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(entry.origin, (0.0, 0.0));
    assert!(entry.descendants.is_empty());
}

#[test]
fn multicol_bookmark_link_semantic_defaults() {
    let multi = MulticolRuleEntry::default();
    assert_eq!(multi.column_count, 0);
    let bookmark = BookmarkAnchorEntry::default();
    assert_eq!(bookmark.level, 0);
    assert!(bookmark.label.is_empty());
    let _link = LinkSpanEntry::default();
    let semantic = SemanticEntry::default();
    assert_eq!(semantic.tag, None);
    assert_eq!(semantic.parent, None);
    assert_eq!(semantic.alt_text, None);
}
