use super::super::*;

#[test]
fn pagedefaults_default_uses_a4() {
    let d = PageDefaults::default();
    assert_eq!(d.page_box, PageBox::A4);
}

#[test]
fn pagedefaults_new_is_default() {
    assert_eq!(
        PageDefaults::new().page_box,
        PageDefaults::default().page_box
    );
}

#[test]
fn pagedefaults_builder_sets_page_box() {
    let d = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
    assert_eq!(d.page_box, PageBox::US_LETTER);
}

#[test]
fn pagedefaults_builder_default_matches_pagedefaults_default() {
    let via_builder = PageDefaults::builder().build();
    let via_default = PageDefaults::default();
    assert_eq!(via_builder.page_box, via_default.page_box);
}
