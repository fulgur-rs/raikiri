use super::*;

fn content(value: &str) -> TextContent {
    SmolStr::new(value).into()
}

#[test]
fn small_unicode_text_stays_inline_until_its_byte_limit() {
    let mut text = content("");
    text.append("雪");
    text.append("é");
    assert!(matches!(&text.0,Repr::Plain(value) if !value.is_heap_allocated()));
    assert_eq!(text.as_str(), "雪é");
    let mut text = content(&"x".repeat(22));
    text.append("é");
    assert_eq!(text.len(), 24);
    assert!(matches!(&text.0, Repr::Appended(_)));
    assert_eq!(text.as_str(), format!("{}é", "x".repeat(22)));
}

#[test]
fn append_and_length_share_the_prefix_without_materializing_it() {
    let prefix = SmolStr::new("x".repeat(4096));
    let pointer = prefix.as_str().as_ptr();
    let mut text = TextContent::from(prefix);
    text.append("é");
    text.append("雪");
    assert_eq!(text.len(), 4101);
    assert!(!text.is_empty());
    let Repr::Appended(pending) = &text.0 else {
        panic!("buffered text")
    };
    assert_eq!(pending.prefix.as_str().as_ptr(), pointer);
    assert_eq!(pending.suffix, "é雪");
    assert!(pending.joined.get().is_none());
    assert!(text.as_str().ends_with("é雪"));
    let pointer = text.as_str().as_ptr();
    assert_eq!(text.as_str().as_ptr(), pointer);
    assert!(pending.joined.get().is_some());
}

#[test]
fn cloned_texts_branch_without_copying_the_joined_cache() {
    for read in [false, true] {
        let mut original = content(&"x".repeat(4096));
        original.append("seed");
        if read {
            assert_eq!(original.as_str().len(), 4100);
        }
        let mut copy = original.clone();
        let (Repr::Appended(a), Repr::Appended(b)) = (&original.0, &copy.0) else {
            panic!("buffered text")
        };
        assert!(Arc::ptr_eq(a, b));
        copy.append("copy");
        let Repr::Appended(copy_data) = &copy.0 else {
            panic!("buffered copy")
        };
        assert!(copy_data.joined.get().is_none());
        assert!(copy.as_str().ends_with("seedcopy"));
        assert!(original.as_str().ends_with("seed"));
        original.append("original");
        assert!(original.as_str().ends_with("seedoriginal"));
        assert!(copy.as_str().ends_with("seedcopy"));
    }
}

#[test]
fn empty_appends_preserve_shared_storage_and_a_materialized_view() {
    let mut original = content(&"x".repeat(4096));
    original.append("seed");
    let pointer = original.as_str().as_ptr();
    let copy = original.clone();
    original.append("");
    let (Repr::Appended(a), Repr::Appended(b)) = (&original.0, &copy.0) else {
        panic!("buffered text")
    };
    assert!(Arc::ptr_eq(a, b));
    assert_eq!(original.as_str().as_ptr(), pointer);
    assert_eq!(copy.as_str().as_ptr(), pointer);
}

#[test]
fn borrowed_string_operations_and_debug_observe_the_latest_text() {
    let mut text = content("abcdefghijklmnopqrstuvwx");
    text.append(" 雪 ");
    assert_eq!(text.trim(), "abcdefghijklmnopqrstuvwx 雪");
    assert_eq!(text.to_string(), "abcdefghijklmnopqrstuvwx 雪 ");
    assert_eq!(
        format!("{text:?}"),
        format!("{:?}", "abcdefghijklmnopqrstuvwx 雪 ")
    );
    text.append("next");
    assert!(text.as_str().ends_with(" 雪 next"));
}
