use super::*;
use raikiri_traits::NodeId;

#[test]
fn tracked_map_insert_and_deref_order() {
    let mut map: TrackedMap<BlockEntry> = TrackedMap::default();
    assert!(map.is_empty());
    let a = NodeId::new(2);
    let b = NodeId::new(1);
    map.insert(a, BlockEntry::default());
    map.insert(b, BlockEntry::default());
    // Deref exposes BTreeMap with ascending key order.
    let keys: Vec<NodeId> = map.keys().copied().collect();
    assert_eq!(keys, vec![b, a]);
    assert_eq!(map.len(), 2);
}

#[test]
fn tracked_map_overwrite_keeps_single_entry() {
    let mut map: TrackedMap<BlockEntry> = TrackedMap::default();
    let key = NodeId::new(5);
    assert!(map.insert(key, BlockEntry::default()).is_none());
    let old = map.insert(key, BlockEntry::default());
    assert!(old.is_some());
    assert_eq!(map.len(), 1);
}

#[test]
fn tracked_map_get_mut_does_not_add_key() {
    let mut map: TrackedMap<BlockEntry> = TrackedMap::default();
    let key = NodeId::new(9);
    assert!(map.get_mut(&key).is_none());
    map.insert(key, BlockEntry::default());
    let entry = map.get_mut(&key).expect("inserted");
    entry.opacity = 0.5;
    assert_eq!(map.get(&key).expect("present").opacity, 0.5);
    assert_eq!(map.len(), 1);
}

#[test]
fn page_drawables_default_is_empty() {
    let drawables = PageDrawables::default();
    assert!(drawables.block_styles.is_empty());
    assert!(drawables.paragraphs.is_empty());
    assert!(drawables.images.is_empty());
    assert!(drawables.svgs.is_empty());
    assert!(drawables.tables.is_empty());
    assert!(drawables.list_items.is_empty());
    assert!(drawables.transforms.is_empty());
    assert!(drawables.link_spans.is_empty());
}
