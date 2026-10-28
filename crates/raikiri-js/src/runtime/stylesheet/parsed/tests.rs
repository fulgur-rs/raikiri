use super::*;

#[test]
fn memo_hits_require_both_sheet_and_shared_source_identity() {
    let mut memo = RuleMemo::default();
    let source: Rc<str> = "p {color:red}".into();
    let first = memo.get(SheetKey::Element(1), Rc::clone(&source));
    let hit = memo.get(SheetKey::Element(1), Rc::clone(&source));
    assert!(Rc::ptr_eq(&first, &hit));
    let equal_source: Rc<str> = source.as_ref().into();
    let rewritten = memo.get(SheetKey::Element(1), equal_source);
    assert!(!Rc::ptr_eq(&first, &rewritten));
    let another = memo.get(SheetKey::Detached(1), Rc::clone(&source));
    assert!(!Rc::ptr_eq(&first, &another));
    let back = memo.get(SheetKey::Element(1), source);
    assert!(!Rc::ptr_eq(&first, &back));
}

#[test]
fn memo_retains_only_one_sheet_and_matching_invalidation_releases_it() {
    let mut memo = RuleMemo::default();
    let source: Rc<str> = "p {color:red}".into();
    let weak = Rc::downgrade(&source);
    drop(memo.get(SheetKey::Element(1), source));
    assert!(weak.upgrade().is_some());
    memo.clear_for(SheetKey::Element(2));
    assert!(weak.upgrade().is_some());
    drop(memo.get(SheetKey::Element(2), "div {}".into()));
    assert!(weak.upgrade().is_none());
    let source: Rc<str> = "span {}".into();
    let weak = Rc::downgrade(&source);
    drop(memo.get(SheetKey::Detached(2), source));
    memo.clear_for(SheetKey::Detached(2));
    assert!(weak.upgrade().is_none());
    let source: Rc<str> = "a {}".into();
    let weak = Rc::downgrade(&source);
    drop(memo.get(SheetKey::Element(3), source));
    drop(memo);
    assert!(weak.upgrade().is_none());
}

#[test]
fn page_margin_ranges_and_bodies_share_a_coherent_snapshot() {
    let text: Rc<str> = "@page { color:red; @top-left { content:'x' } @bottom-right {color:blue} } p {color:green} @import 'x';".into();
    let parsed = ParsedSheet::new(Rc::clone(&text));
    assert_eq!(parsed.rules.len(), 3);
    assert!(parsed.is_page(0));
    assert!(!parsed.is_page(1));
    assert!(!parsed.is_page(99));
    let ranges = parsed.margins(0);
    assert_eq!(ranges.len(), 2);
    assert!(Rc::ptr_eq(&ranges, &parsed.margins(0)));
    assert_eq!(&text[ranges[0].clone()], "@top-left { content:'x' }");
    assert_eq!(parsed.margin_body(0, 0).as_deref(), Some(" content:'x' "));
    assert_eq!(parsed.margin_body(0, 1).as_deref(), Some("color:blue"));
    assert_eq!(parsed.margin_body(0, 2), None);
    assert!(parsed.margins(1).is_empty());
    assert!(parsed.margins(99).is_empty());
    assert_eq!(parsed.rule_body(1).as_deref(), Some("color:green"));
    assert_eq!(parsed.rule_body(2).as_deref(), Some(""));
    assert_eq!(parsed.rule_body(99), None);
}
