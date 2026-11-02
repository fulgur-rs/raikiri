use super::*;

#[test]
fn buffered_text_is_preserved_by_extraction_and_replacement() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "p", None::<&str>);
    let text = doc.append_text(parent, "x".repeat(4096));
    doc.append_character_data(text, "&雪").unwrap();
    let expected = format!("{}&雪", "x".repeat(4096));
    let mut extracted = doc.extract_nodes(&[0, parent, text]);
    extracted.append_character_data(2, "extracted").unwrap();
    assert!(
        extracted
            .character_data(2)
            .unwrap()
            .ends_with("&雪extracted")
    );
    assert_eq!(doc.character_data(text), Some(expected.as_str()));
    assert_eq!(
        doc.serialize_inner_html(parent).unwrap(),
        format!("{}&amp;雪", "x".repeat(4096))
    );
    doc.set_character_data(text, "replacement").unwrap();
    assert_eq!(doc.character_data(text), Some("replacement"));
}

#[test]
fn empty_and_nonempty_buffered_writes_still_invalidate_layout() {
    for addition in ["", "more"] {
        let mut doc = Document::new();
        let text = doc.create_detached_text(&"x".repeat(4096));
        doc.append_character_data(text, "seed").unwrap();
        assert_eq!(doc.character_data(text).unwrap().len(), 4100);
        doc.layout_dirty = false;
        doc.append_character_data(text, addition).unwrap();
        assert!(doc.layout_dirty);
        assert_eq!(
            doc.character_data(text).unwrap().len(),
            4100 + addition.len()
        );
    }
}

#[cfg(feature = "logical-snapshot")]
#[test]
fn buffered_text_is_preserved_by_logical_snapshots() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "p", None::<&str>);
    let text = doc.append_text(parent, "x".repeat(4096));
    doc.append_character_data(text, "&雪").unwrap();
    let snapshot = doc.logical_snapshot();
    let mut restored = Document::from_logical_snapshot(snapshot, 1024).unwrap();
    assert_eq!(restored.character_data(text), doc.character_data(text));
    restored.append_character_data(text, "restored").unwrap();
    assert!(
        restored
            .character_data(text)
            .unwrap()
            .ends_with("&雪restored")
    );
    assert!(doc.character_data(text).unwrap().ends_with("&雪"));
}
