use super::*;
use std::cell::Cell;

struct CountingClasses {
    attrs: Vec<(String, String)>,
    opt_in: bool,
    calls: Cell<usize>,
    attributes: Cell<usize>,
    tokens: Cell<usize>,
}

impl StyleElement for CountingClasses {
    fn tag_name(&self) -> &str {
        "p"
    }
    fn attr(&self, local: &str) -> Option<&str> {
        self.attrs.iter().find_map(|(name, value)| {
            self.attributes.set(self.attributes.get() + 1);
            (name == local).then_some(value.as_str())
        })
    }
    fn has_class(&self, class: &str) -> bool {
        self.check(class, false)
    }
    fn has_class_ascii_case_insensitive(&self, class: &str) -> bool {
        self.check(class, true)
    }
    fn class_attribute_for_matching(&self) -> Option<&str> {
        self.opt_in.then(|| self.attr("class").unwrap_or(""))
    }
}

impl CountingClasses {
    fn new(c: usize, a: usize, opt_in: bool) -> Self {
        let mut attrs = (0..a)
            .map(|i| (format!("a{i}"), String::new()))
            .collect::<Vec<_>>();
        attrs.push((
            "class".into(),
            (0..c)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(" "),
        ));
        Self {
            attrs,
            opt_in,
            calls: Cell::new(0),
            attributes: Cell::new(0),
            tokens: Cell::new(0),
        }
    }
    fn check(&self, class: &str, folded: bool) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.attr("class").is_some_and(|value| {
            value.split([' ', '\t', '\n', '\r', '\x0c']).any(|token| {
                self.tokens.set(self.tokens.get() + 1);
                if folded {
                    token.eq_ignore_ascii_case(class)
                } else {
                    token == class
                }
            })
        })
    }
}

fn compound(c: usize) -> String {
    (0..c).map(|i| format!(".c{i}")).collect()
}

fn match_classes(
    elem: &impl StyleElement,
    source: &str,
    quirks: StyleQuirksMode,
    limit: Option<u64>,
) -> (Result<Option<Specificity>, CascadeError>, u64) {
    let doc = TestDoc::new();
    let list = crate::parse_selector_list(source).unwrap();
    let caches = MatchCaches::with_work_limit(limit);
    let ctx = MatchContext::new(&doc, quirks, None, false, &caches);
    let result = match_complex_selector_list(&list, ctx, elem, StyleNodeId::new(1), &[]);
    (result, caches.work.count())
}

#[test]
fn many_class_compound_reads_opted_in_attribute_once_after_warmup() {
    for a in [0, 32, 128] {
        for c in [32, 128] {
            let elem = CountingClasses::new(c, a, true);
            assert!(
                match_classes(&elem, &compound(c), StyleQuirksMode::NoQuirks, None)
                    .0
                    .unwrap()
                    .is_some()
            );
            assert!(elem.calls.get() <= 3);
            assert!(elem.attributes.get() <= 4 * (a + 1));
            assert!(elem.tokens.get() <= 3 * c);
        }
    }
}

#[test]
fn custom_class_methods_and_short_or_failing_compounds_keep_their_path() {
    struct Virtual;
    impl StyleElement for Virtual {
        fn tag_name(&self) -> &str {
            "p"
        }
        fn has_class(&self, class: &str) -> bool {
            class == "virtual"
        }
        fn has_class_ascii_case_insensitive(&self, class: &str) -> bool {
            class.eq_ignore_ascii_case("virtual")
        }
    }
    for mode in [
        StyleQuirksMode::NoQuirks,
        StyleQuirksMode::LimitedQuirks,
        StyleQuirksMode::Quirks,
    ] {
        assert!(
            match_classes(&Virtual, &".virtual".repeat(32), mode, None)
                .0
                .unwrap()
                .is_some()
        );
    }
    let elem = CountingClasses::new(128, 32, false);
    assert!(
        match_classes(&elem, &compound(128), StyleQuirksMode::NoQuirks, None)
            .0
            .unwrap()
            .is_some()
    );
    assert_eq!(elem.calls.get(), 128);
    assert_eq!(elem.attributes.get(), 128 * 33);
    for c in [1, 2, 3] {
        let elem = CountingClasses::new(128, 32, true);
        assert!(
            match_classes(&elem, &compound(c), StyleQuirksMode::NoQuirks, None)
                .0
                .unwrap()
                .is_some()
        );
        assert_eq!(elem.calls.get(), c);
        assert_eq!(elem.attributes.get(), c * 33);
    }
    let elem = CountingClasses::new(128, 32, true);
    assert!(
        match_classes(
            &elem,
            &".missing".repeat(128),
            StyleQuirksMode::NoQuirks,
            None
        )
        .0
        .unwrap()
        .is_none()
    );
    assert_eq!(elem.calls.get(), 1);
    assert_eq!(elem.attributes.get(), 33);
}

#[test]
fn indexed_class_comparison_preserves_whitespace_and_quirks_and_mutations() {
    for mode in [
        StyleQuirksMode::NoQuirks,
        StyleQuirksMode::LimitedQuirks,
        StyleQuirksMode::Quirks,
    ] {
        for value in [
            (0..32)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(" \t\n\r\x0c"),
            (0..32)
                .map(|i| format!("C{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            format!(
                "{} Ä",
                (0..32)
                    .map(|i| format!("c{i}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            format!(
                "{} c0\u{00a0}c1",
                (2..32)
                    .map(|i| format!("c{i}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ] {
            for selector in [
                compound(32),
                format!("{}.ä", compound(32)),
                format!("{}.Ä", compound(32)),
            ] {
                let mut standard = CountingClasses::new(32, 0, false);
                standard.attrs[0].1 = value.clone();
                let expected = match_classes(&standard, &selector, mode, None).0.unwrap();
                standard.opt_in = true;
                assert_eq!(
                    match_classes(&standard, &selector, mode, None).0.unwrap(),
                    expected
                );
                standard.attrs[0].1.clear();
                assert!(
                    match_classes(&standard, &selector, mode, None)
                        .0
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn class_index_reservation_has_exact_and_rejected_bulk_work_boundaries() {
    let elem = CountingClasses::new(128, 0, true);
    let source = compound(128);
    let expected = 128 + elem.attrs[0].1.len() as u64;
    assert_eq!(
        match_classes(&elem, &source, StyleQuirksMode::NoQuirks, Some(expected)).1,
        expected
    );
    assert!(
        match_classes(
            &elem,
            &source,
            StyleQuirksMode::NoQuirks,
            Some(expected - 1)
        )
        .0
        .is_err()
    );
    let elem = CountingClasses::new(128, 0, true);
    let (result, used) = match_classes(&elem, &source, StyleQuirksMode::NoQuirks, Some(8));
    assert!(result.is_err());
    assert_eq!(used, 4);
    assert_eq!(elem.calls.get(), 3);
    assert_eq!(elem.attributes.get(), 4);
    let elem = CountingClasses::new(128, 0, true);
    assert!(
        match_classes(&elem, &source, StyleQuirksMode::NoQuirks, Some(0))
            .0
            .is_err()
    );
    assert_eq!(elem.calls.get(), 0);
    assert_eq!(elem.attributes.get(), 0);
}

#[test]
fn byte_or_token_oversize_falls_back_without_missing_late_tokens() {
    for count in [512, 513] {
        let elem = CountingClasses::new(count, 0, true);
        assert!(
            match_classes(&elem, &compound(count), StyleQuirksMode::NoQuirks, None)
                .0
                .unwrap()
                .is_some()
        );
        assert_eq!(elem.calls.get(), if count == 512 { 3 } else { count });
    }
    for bytes in [63, 64, 65_536, 65_537] {
        let mut elem = CountingClasses::new(4, 0, true);
        elem.attrs[0].1 = format!("c0 c1 c2 c3{}", " ".repeat(bytes - 11));
        assert_eq!(elem.attrs[0].1.len(), bytes);
        assert!(
            match_classes(&elem, &compound(4), StyleQuirksMode::NoQuirks, None)
                .0
                .unwrap()
                .is_some()
        );
        assert_eq!(
            elem.calls.get(),
            if (64..=65_536).contains(&bytes) { 3 } else { 4 }
        );
    }
}

#[test]
fn actual_class_operations_scale_with_tokens_instead_of_compound_squared() {
    for c in [8, 32, 128] {
        for a in [0, 32, 128] {
            let fallback = CountingClasses::new(c, a, false);
            let indexed = CountingClasses::new(c, a, true);
            assert!(
                match_classes(&fallback, &compound(c), StyleQuirksMode::NoQuirks, None)
                    .0
                    .unwrap()
                    .is_some()
            );
            class_index::reset_stats();
            assert!(
                match_classes(&indexed, &compound(c), StyleQuirksMode::NoQuirks, None)
                    .0
                    .unwrap()
                    .is_some()
            );
            let (tokens, hashed_bytes, comparisons) = class_index::stats();
            assert_eq!(fallback.tokens.get(), c * (c + 1) / 2);
            if c >= 32 {
                assert_eq!(tokens, c);
                assert!(indexed.tokens.get() + tokens + comparisons < c * 4);
                assert_eq!(indexed.attributes.get(), 4 * (a + 1));
            } else {
                assert_eq!(tokens, 0);
            }
            println!(
                "CLASS_OPS c={c} a={a} old_attrs={} new_attrs={} old_tokens={} new_fallback_tokens={} index_tokens={tokens} index_hash_bytes={hashed_bytes} index_equalities={comparisons}",
                fallback.attributes.get(),
                indexed.attributes.get(),
                fallback.tokens.get(),
                indexed.tokens.get()
            );
        }
    }
}
