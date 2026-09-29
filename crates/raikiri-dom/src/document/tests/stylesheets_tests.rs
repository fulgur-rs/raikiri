use super::*;
use raikiri_traits::StylesheetKind;
use std::borrow::Cow;

#[test]
fn document_add_stylesheet_appends_in_call_order() {
    let mut doc = Document::new();
    doc.add_stylesheet(Cow::Borrowed("a { color: red }"), StylesheetKind::UserAgent);
    doc.add_stylesheet(
        Cow::Owned("b { color: blue }".to_string()),
        StylesheetKind::Author,
    );

    let collected: Vec<(&str, StylesheetKind)> = doc.stylesheets().collect();
    assert_eq!(collected.len(), 2);
    assert_eq!(
        collected[0],
        ("a { color: red }", StylesheetKind::UserAgent)
    );
    assert_eq!(collected[1], ("b { color: blue }", StylesheetKind::Author));
}

#[test]
fn document_add_stylesheet_borrow_variant_zero_alloc() {
    // Passing Cow::Borrowed retains Borrowed in internal storage;
    // verify through as_ref() (pointer equality with the same 'static address).
    let mut doc = Document::new();
    let ua: &'static str = "html { display: block }";
    doc.add_stylesheet(Cow::Borrowed(ua), StylesheetKind::UserAgent);
    let (source, _kind) = doc.stylesheets().next().expect("has one");
    assert!(
        std::ptr::eq(source, ua),
        "borrowed source should keep &'static identity"
    );
}

#[test]
fn document_stylesheets_iterates_mixed_kinds() {
    let mut doc = Document::new();
    doc.add_stylesheet(Cow::Borrowed("ua1"), StylesheetKind::UserAgent);
    doc.add_stylesheet(Cow::Borrowed("au1"), StylesheetKind::Author);
    doc.add_stylesheet(Cow::Borrowed("ua2"), StylesheetKind::UserAgent);

    let kinds: Vec<StylesheetKind> = doc.stylesheets().map(|(_, k)| k).collect();
    assert_eq!(
        kinds,
        vec![
            StylesheetKind::UserAgent,
            StylesheetKind::Author,
            StylesheetKind::UserAgent,
        ]
    );
}
