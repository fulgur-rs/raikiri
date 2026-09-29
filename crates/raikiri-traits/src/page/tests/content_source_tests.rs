use super::super::*;

#[test]
fn content_source_default_is_empty() {
    let cs = ContentSource::default();
    assert!(cs.items.is_empty());
}

#[test]
fn content_source_new_wraps_vec() {
    let items = vec![
        ContentValueItem::Literal(String::from("Ch. ")),
        ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::default(),
        },
    ];
    let cs = ContentSource::new(items.clone());
    assert_eq!(cs.items, items);
}

#[test]
fn content_source_struct_update_from_default() {
    // #[non_exhaustive] public struct の consumer construct pattern
    // (sibling PageBox の struct-update pattern 継承)。
    let cs = ContentSource {
        items: vec![ContentValueItem::Literal(String::from("hello"))],
        ..Default::default()
    };
    assert_eq!(cs.items.len(), 1);
}
