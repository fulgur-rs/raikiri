use super::*;

fn style(grow: f32) -> crate::LayoutStyle {
    crate::LayoutStyle {
        flex_grow: grow,
        ..Default::default()
    }
}

#[test]
fn sparse_source_and_generated_keys_remain_independent() {
    let mut styles = RetainedLayoutStyles::default();
    styles.insert(BoxKey::Source(3), style(3.0));
    styles.insert(BoxKey::AnonymousCell(3), style(30.0));
    assert!(styles.get(BoxKey::Source(2)).is_none());
    assert!(styles.get(BoxKey::Source(4)).is_none());
    assert!(styles.get_mut(BoxKey::Source(2)).is_none());
    assert!(styles.get_mut(BoxKey::Source(4)).is_none());
    styles.insert(BoxKey::Source(1), style(1.0));
    styles.insert(BoxKey::Source(3), style(4.0));
    styles.get_mut(BoxKey::AnonymousCell(3)).unwrap().flex_grow = 31.0;
    assert_eq!(styles.get(BoxKey::Source(1)).unwrap().flex_grow, 1.0);
    assert_eq!(styles.get(BoxKey::Source(3)).unwrap().flex_grow, 4.0);
    assert_eq!(
        styles.get(BoxKey::AnonymousCell(3)).unwrap().flex_grow,
        31.0
    );
}

#[test]
fn retain_operations_preserve_each_box_category() {
    let mut styles = RetainedLayoutStyles::default();
    for (key, grow) in [
        (BoxKey::Source(0), 1.0),
        (BoxKey::AnonymousCell(0), 2.0),
        (BoxKey::AnonymousTable(0), 3.0),
        (BoxKey::InitialContainingBlock, 4.0),
    ] {
        styles.insert(key, style(grow));
    }
    styles.retain_without_cells();
    assert!(styles.get(BoxKey::AnonymousCell(0)).is_none());
    assert_eq!(
        styles.get(BoxKey::AnonymousTable(0)).unwrap().flex_grow,
        3.0
    );
    styles.retain_source_and_initial_containing_block();
    assert!(styles.get(BoxKey::AnonymousTable(0)).is_none());
    assert_eq!(
        styles
            .get(BoxKey::InitialContainingBlock)
            .unwrap()
            .flex_grow,
        4.0
    );
    styles.retain_source_styles();
    assert!(styles.get(BoxKey::InitialContainingBlock).is_none());
    assert_eq!(styles.get(BoxKey::Source(0)).unwrap().flex_grow, 1.0);
}

#[test]
fn cloned_styles_are_independent() {
    let mut styles = RetainedLayoutStyles::default();
    styles.insert(BoxKey::Source(1), style(1.0));
    styles.insert(BoxKey::AnonymousTable(0), style(2.0));
    let mut cloned = styles.clone();
    cloned.get_mut(BoxKey::Source(1)).unwrap().flex_grow = 10.0;
    cloned.get_mut(BoxKey::AnonymousTable(0)).unwrap().flex_grow = 20.0;
    assert_eq!(styles.get(BoxKey::Source(1)).unwrap().flex_grow, 1.0);
    assert_eq!(
        styles.get(BoxKey::AnonymousTable(0)).unwrap().flex_grow,
        2.0
    );
}
