use super::*;

fn sheet() -> (Document, usize, usize) {
    let mut document = Document::new();
    let style = document.append_element(Some(0), "style", None::<&str>);
    let text = document.append_text(style, "p {color:red}");
    (document, style, text)
}

#[test]
fn stylesheet_versions_observe_text_and_source_writes_without_an_override() {
    let (mut document, style, text) = sheet();
    let before = document.stylesheet_version().clone();
    document.set_character_data(text, "p {color:blue}").unwrap();
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.append_text_coalesced(style, " div {}");
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document
        .set_stylesheet_source(style, "a {}".into())
        .unwrap();
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.reset_stylesheet_source(style);
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.reset_stylesheet_source(style);
    assert_ne!(&before, document.stylesheet_version());
}

#[test]
fn stylesheet_versions_observe_topology_and_metadata_without_overrides() {
    let (mut document, style, _) = sheet();
    let before = document.stylesheet_version().clone();
    document.detach_from_parent(style);
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.attach_child(0, style);
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.set_element_namespace(style, Some("urn:foreign".into()));
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document
        .set_element_attribute(style, "type", "text/plain")
        .unwrap();
    assert_ne!(&before, document.stylesheet_version());
    let before = document.stylesheet_version().clone();
    document.set_element_attributes(style, vec![]);
    assert_ne!(&before, document.stylesheet_version());
}

#[test]
fn stylesheet_version_reads_are_stable_and_clones_have_distinct_identities() {
    let (mut document, style, _) = sheet();
    let version = document.stylesheet_version().clone();
    assert_eq!(&version, document.stylesheet_version());
    assert!(document.set_stylesheet_source(0, "x {}".into()).is_err());
    assert_eq!(&version, document.stylesheet_version());
    assert_ne!(
        document.stylesheet_version(),
        document.clone().stylesheet_version()
    );
    assert_ne!(
        document.stylesheet_version(),
        document.extract_nodes(&[0, style]).stylesheet_version()
    );
    #[cfg(feature = "logical-snapshot")]
    {
        let restored = Document::from_logical_snapshot(document.logical_snapshot(), 100).unwrap();
        assert_ne!(document.stylesheet_version(), restored.stylesheet_version());
    }
}

#[test]
fn overflowing_stylesheet_revision_replaces_the_document_identity() {
    let (mut document, _, _) = sheet();
    let first = document.stylesheet_version().clone();
    document.stylesheet_revision.version.generation = u64::MAX;
    let last = document.stylesheet_version().clone();
    document.note_stylesheet_change();
    assert_ne!(&first, document.stylesheet_version());
    assert_ne!(&last, document.stylesheet_version());
    assert_eq!(document.stylesheet_version().generation, 0);
}
