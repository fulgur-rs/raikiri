use super::*;

#[test]
fn admission_never_exposes_a_partial_token_set() {
    for (count, admitted) in [(0, true), (512, true), (513, false)] {
        let value = (0..count)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let index = ClassIndex::new(&value, false);
        assert_eq!(index.is_some(), admitted);
        if let Some(index) = index {
            assert_eq!(index.tokens.len(), count);
            assert!(!index.contains(""));
            if count != 0 {
                assert!(index.contains(&format!("c{}", count - 1)));
            }
        }
    }
}

struct Classes(String);
impl StyleElement for Classes {
    fn tag_name(&self) -> &str {
        "p"
    }
    fn attr(&self, local: &str) -> Option<&str> {
        (local == "class").then_some(self.0.as_str())
    }
    fn class_attribute_for_matching(&self) -> Option<&str> {
        Some(&self.0)
    }
}

fn classes(count: usize) -> Classes {
    Classes(
        (0..count)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[test]
fn raw_tail_scans_promote_only_on_next_call_strictly_after_four_sources() {
    let elem = classes(128);
    let work = SelectorWork::new(None);
    let mut lookup = ClassLookup::default();
    reset_stats();
    for check in 1..=8 {
        assert!(lookup.matches(&elem, "c127", false, &work).unwrap());
        assert_eq!(stats().0, 0);
        assert_eq!(work.count(), 0);
        if check >= 3 {
            assert_eq!(raw_stats().1, (check - 3) * elem.0.len());
        }
    }
    assert!(lookup.matches(&elem, "c127", false, &work).unwrap());
    assert_eq!(stats().0, 128);
    assert_eq!(work.count(), elem.0.len() as u64);
}

#[test]
fn raw_prefix_checks_and_final_misses_do_not_build_unused_indexes() {
    let elem = classes(128);
    for query in ["c0", "c127"] {
        let work = SelectorWork::new(None);
        let mut lookup = ClassLookup::default();
        reset_stats();
        let checks = if query == "c0" { 128 } else { 7 };
        for _ in 0..checks {
            assert!(lookup.matches(&elem, query, false, &work).unwrap());
        }
        assert!(!lookup.matches(&elem, "missing", false, &work).unwrap());
        assert_eq!(stats().0, 0);
        assert_eq!(work.count(), 0);
    }
}

#[test]
fn raw_scan_counts_ascii_separators_and_preserves_token_semantics() {
    let source = " \t\n\r\x0cfirst\tÄ\u{a0}end  LAST ";
    assert_eq!(scan_matches(source, "first", false), (true, 10));
    assert_eq!(scan_matches(source, "last", false), (false, source.len()));
    assert_eq!(scan_matches(source, "last", true), (true, source.len() - 1));
    assert_eq!(scan_matches(source, "Ä\u{a0}end", false), (true, 18));
    assert_eq!(
        scan_matches(source, "ä\u{a0}end", true),
        (false, source.len())
    );
    assert_eq!(scan_matches(source, "", false), (false, 0));
    assert_eq!(scan_matches("", "missing", false), (false, 0));
}

#[test]
fn byte_and_token_rejection_never_retry_or_expose_partial_indexes() {
    for bytes in [63, 64, 65_536, 65_537] {
        let elem = Classes(format!("{}tail", " ".repeat(bytes - 4)));
        let work = SelectorWork::new(None);
        let mut lookup = ClassLookup::default();
        reset_stats();
        for _ in 0..16 {
            assert!(lookup.matches(&elem, "tail", false, &work).unwrap());
        }
        let admitted = (64..=65_536).contains(&bytes);
        assert_eq!(stats().0, usize::from(admitted));
        assert_eq!(work.count(), if admitted { bytes as u64 } else { 0 });
    }
    for count in [512, 513] {
        let elem = classes(count);
        let work = SelectorWork::new(None);
        let mut lookup = ClassLookup::default();
        reset_stats();
        for _ in 0..32 {
            assert!(
                lookup
                    .matches(&elem, &format!("c{}", count - 1), false, &work)
                    .unwrap()
            );
            assert!(lookup.matches(&elem, "c0", false, &work).unwrap());
        }
        assert_eq!(stats().0, count);
        assert_eq!(work.count(), elem.0.len() as u64);
    }
}

#[test]
fn empty_opted_in_source_and_empty_requested_classes_never_match() {
    let work = SelectorWork::new(None);
    for source in ["", " \t\n\r\x0c", "c0 c1"] {
        let elem = Classes(source.into());
        let mut lookup = ClassLookup::default();
        reset_stats();
        for _ in 0..16 {
            assert!(!lookup.matches(&elem, "", true, &work).unwrap());
        }
        assert_eq!(stats().0, 0);
    }
}

#[test]
fn rejected_promotion_reserves_before_construction_and_stays_terminal() {
    let elem = classes(128);
    let work = SelectorWork::new(Some(8));
    work.charge(7).unwrap();
    let mut lookup = ClassLookup::default();
    reset_stats();
    for _ in 0..8 {
        assert!(lookup.matches(&elem, "c127", false, &work).unwrap());
    }
    let first = lookup.matches(&elem, "c127", false, &work).unwrap_err();
    assert_eq!(work.count(), 7);
    assert_eq!(stats().0, 0);
    assert_eq!(
        lookup
            .matches(&elem, "c127", false, &work)
            .unwrap_err()
            .to_string(),
        first.to_string()
    );
    assert_eq!(work.count(), 7);
    assert_eq!(stats().0, 0);
}
