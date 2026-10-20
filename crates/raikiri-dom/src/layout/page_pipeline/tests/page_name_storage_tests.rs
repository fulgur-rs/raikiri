use super::*;

#[test]
fn unnamed_page_storage_stays_empty_across_breaks_and_overflow() {
    let mut names = PageNames::unnamed();
    for page in [0, 1, 10, 1000] {
        names.name_page(page);
        for overwrite in [false, true] {
            names.continue_onto(page, page + 100, overwrite);
        }
    }
    assert!(names.by_page.is_empty());
    assert_eq!(names.by_page.capacity(), 0);
    let pages = page_slices(3, &PageOrigins::new(&[], 100.0), &names);
    assert_eq!(pages.len(), 4);
    assert!(pages.iter().all(|page| page.page_name.is_none()));
}

#[test]
fn recording_mode_preserves_reached_anonymous_pages_before_the_first_name() {
    for overwrite in [false, true] {
        let mut names = PageNames::new(None);
        names.continue_onto(0, 5, false);
        names.current = Some("chapter".to_owned());
        names.continue_onto(1, 3, overwrite);
        assert_eq!(names.by_page[0], None);
        assert_eq!(names.by_page[1], None);
        assert_eq!(names.by_page[2].as_deref(), overwrite.then_some("chapter"));
        assert_eq!(names.by_page[3].as_deref(), overwrite.then_some("chapter"));
        assert_eq!(names.by_page[4], None);
        assert_eq!(names.by_page[5], None);
        names.continue_onto(5, 6, false);
        assert_eq!(names.by_page[6].as_deref(), Some("chapter"));
        names.current = None;
        names.continue_onto(5, 6, true);
        assert_eq!(names.by_page[6], None);
    }
}
