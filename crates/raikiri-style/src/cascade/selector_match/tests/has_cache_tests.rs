use super::*;

/// `body > section.card*` where every other card holds an `img`, each card
/// holding `paragraphs` paragraphs. Returns the doc, the body, the cards and
/// every paragraph with its card index.
fn cards(
    card_count: usize,
    paragraphs: usize,
) -> (TestDoc, usize, Vec<usize>, Vec<(usize, usize)>) {
    let mut doc = TestDoc::new();
    let body = doc.push_element(0, "body", None);
    let mut card_ids = Vec::new();
    let mut paragraph_ids = Vec::new();
    for card in 0..card_count {
        let section = doc.push_element(body, "section", None);
        doc.set_attr(section, "class", "card");
        if card % 2 == 0 {
            let figure = doc.push_element(section, "figure", None);
            doc.push_element(figure, "img", None);
        }
        for _ in 0..paragraphs {
            let p = doc.push_element(section, "p", None);
            doc.push_text(p, "text");
            paragraph_ids.push((p, card));
        }
        card_ids.push(section);
    }
    (doc, body, card_ids, paragraph_ids)
}

fn matches_with(
    doc: &TestDoc,
    caches: &MatchCaches,
    list: &SelectorList<RaikiriSelectorImpl>,
    item: usize,
    ancestors: &[StyleNodeId],
) -> bool {
    let id = StyleNodeId::new(item as u64);
    let node = doc.node(id).unwrap();
    let elem = node.as_element().unwrap();
    let ctx = MatchContext::new(doc, StyleQuirksMode::NoQuirks, None, false, caches);
    match_complex_selector_list(list, ctx, &elem, id, ancestors)
        .unwrap()
        .is_some()
}

#[test]
fn has_result_is_recorded_once_per_anchor() {
    let (doc, body, card_ids, paragraphs) = cards(6, 5);
    let selector = crate::parse_selector_list(".card:has(img) p").unwrap();
    let caches = MatchCaches::default();
    for &(p, card) in &paragraphs {
        let ancestors = [
            StyleNodeId::new(body as u64),
            StyleNodeId::new(card_ids[card] as u64),
        ];
        assert_eq!(
            matches_with(&doc, &caches, &selector, p, &ancestors),
            card % 2 == 0,
            "paragraph {p} in card {card}"
        );
    }
    // One record per card, however many paragraphs asked, and the argument
    // list's selectors are kept once for all of them.
    assert_eq!(caches.has_entries(), card_ids.len());
    assert_eq!(caches.has_lists(), 1);
}

#[test]
fn shared_has_records_agree_with_fresh_caches() {
    let (doc, body, card_ids, paragraphs) = cards(5, 3);
    for source in [
        ".card:has(img) p",
        ".card:has(> figure > img) > p",
        ":has(+ .card > figure) ~ .card p",
        "body:has(p) .card:not(:has(img)) p",
        ":is(.card:has(img), .card:has(p)) p:has(+ p)",
    ] {
        let selector = crate::parse_selector_list(source).unwrap();
        for limit in [usize::MAX, 1] {
            let shared = MatchCaches::with_has_limit(limit);
            for &(p, card) in paragraphs.iter().chain(paragraphs.iter().rev()) {
                let ancestors = [
                    StyleNodeId::new(body as u64),
                    StyleNodeId::new(card_ids[card] as u64),
                ];
                assert_eq!(
                    matches_with(&doc, &shared, &selector, p, &ancestors),
                    matches_with(&doc, &MatchCaches::default(), &selector, p, &ancestors),
                    "{source} on paragraph {p}, limit {limit}"
                );
            }
            assert!(shared.has_entries() <= limit, "{source}");
        }
    }
}

#[test]
fn has_records_are_kept_apart_per_argument_list() {
    // The same anchor answers different `:has()` lists differently.
    let (doc, body, card_ids, paragraphs) = cards(2, 1);
    let caches = MatchCaches::default();
    let with_img = crate::parse_selector_list(".card:has(img) p").unwrap();
    let with_video = crate::parse_selector_list(".card:has(video) p").unwrap();
    let (p, card) = paragraphs[0];
    let ancestors = [
        StyleNodeId::new(body as u64),
        StyleNodeId::new(card_ids[card] as u64),
    ];
    assert!(matches_with(&doc, &caches, &with_img, p, &ancestors));
    assert!(!matches_with(&doc, &caches, &with_video, p, &ancestors));
    assert!(matches_with(&doc, &caches, &with_img, p, &ancestors));
    assert_eq!(caches.has_entries(), 2);
}

#[test]
fn content_language_records_the_ancestors_it_walks() {
    // An html element with lang=en, three nested divs and a span, then a second span under the
    // innermost div answers from the recorded ancestors.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    doc.set_attr(html, "lang", "en");
    let mut chain = vec![html];
    for _ in 0..3 {
        let div = doc.push_element(*chain.last().unwrap(), "div", None);
        chain.push(div);
    }
    let first = doc.push_element(*chain.last().unwrap(), "span", None);
    let second = doc.push_element(*chain.last().unwrap(), "span", None);
    let ancestors = chain
        .iter()
        .map(|&id| StyleNodeId::new(id as u64))
        .collect::<Vec<_>>();
    let caches = MatchCaches::default();
    for span in [first, second] {
        let id = StyleNodeId::new(span as u64);
        let node = doc.node(id).unwrap();
        let elem = node.as_element().unwrap();
        assert_eq!(
            &*caches
                .content_language(&doc, &elem, id, &ancestors)
                .unwrap(),
            "en"
        );
    }
    // The three divs and both spans; `html` has its own `lang`.
    assert_eq!(caches.languages.borrow().len(), 5);
    for &div in &chain[1..] {
        assert_eq!(
            caches
                .languages
                .borrow()
                .get(&StyleNodeId::new(div as u64))
                .map(|lang| lang.to_string()),
            Some("en".to_owned())
        );
    }
}

#[test]
fn content_language_without_any_lang_records_the_empty_tag() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let div = doc.push_element(html, "div", None);
    let span = doc.push_element(div, "span", None);
    let ancestors = [StyleNodeId::new(html as u64), StyleNodeId::new(div as u64)];
    let caches = MatchCaches::default();
    let id = StyleNodeId::new(span as u64);
    let node = doc.node(id).unwrap();
    let elem = node.as_element().unwrap();
    assert_eq!(
        &*caches
            .content_language(&doc, &elem, id, &ancestors)
            .unwrap(),
        ""
    );
    assert_eq!(caches.languages.borrow().len(), 3);
}

struct SiblingCountingDoc<'a> {
    doc: &'a TestDoc,
    visits: std::cell::Cell<usize>,
}

struct SiblingCountingIter<'a> {
    inner: <TestDoc as StyleDom>::ChildIter<'a>,
    visits: &'a std::cell::Cell<usize>,
}

impl Iterator for SiblingCountingIter<'_> {
    type Item = StyleNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let child = self.inner.next()?;
        self.visits.set(self.visits.get() + 1);
        Some(child)
    }
}

impl StyleDom for SiblingCountingDoc<'_> {
    type NodeRef<'a>
        = <TestDoc as StyleDom>::NodeRef<'a>
    where
        Self: 'a;
    type ChildIter<'a>
        = SiblingCountingIter<'a>
    where
        Self: 'a;

    fn root_id(&self) -> StyleNodeId {
        self.doc.root_id()
    }
    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        self.doc.node(id)
    }
    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        SiblingCountingIter {
            inner: self.doc.child_ids(id),
            visits: &self.visits,
        }
    }
    fn node_count(&self) -> usize {
        self.doc.node_count()
    }
    fn parent_id(&self, id: StyleNodeId) -> Option<StyleNodeId> {
        self.doc.parent_id(id)
    }
}

#[test]
fn adjacent_has_visits_wide_sibling_lists_linearly() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "main", None);
    let items: Vec<_> = (0..512)
        .map(|_| doc.push_element(parent, "div", None))
        .collect();
    doc.set_attr(*items.last().unwrap(), "class", "next");
    let counted = SiblingCountingDoc {
        doc: &doc,
        visits: std::cell::Cell::new(0),
    };
    let query = crate::SelectorQuery::parse("div:has(+ .next)").unwrap();
    let matcher = query.matcher(&counted, None);
    let ancestors = [StyleNodeId::new(parent as u64)];
    let matched: Vec<_> = items
        .iter()
        .copied()
        .filter(|&id| matcher.matches(StyleNodeId::new(id as u64), &ancestors))
        .collect();
    assert_eq!(matched, [items[items.len() - 2]]);
    assert!(
        counted.visits.get() <= 8 * items.len(),
        "visited {} children",
        counted.visits.get()
    );
}

#[test]
fn adjacent_has_skips_non_elements_and_respects_detached_candidates() {
    for filler in [0, 8] {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "main", None);
        for _ in 0..filler {
            doc.push_element(parent, "i", None);
        }
        let anchor = doc.push_element(parent, "div", None);
        doc.push_text(parent, "text");
        doc.push_comment(parent, "comment");
        let inert = doc.push_element(parent, "div", None);
        doc.set_in_document(inert, false);
        let next = doc.push_element(parent, "div", None);
        doc.set_attr(next, "class", "next");
        let leaf = doc.push_element(next, "span", None);
        doc.set_attr(leaf, "class", "leaf");
        let tail = doc.push_element(parent, "div", None);
        doc.set_attr(tail, "class", "tail");
        let ancestors = [StyleNodeId::new(parent as u64)];
        let caches = MatchCaches::default();
        for source in [
            "div:has(+ .next)",
            "div:has(+ .next > .leaf)",
            "div:has(+ .next + .tail)",
        ] {
            let list = crate::parse_selector_list(source).unwrap();
            assert!(
                matches_with(&doc, &caches, &list, anchor, &ancestors),
                "{source}, filler {filler}"
            );
            assert!(
                !matches_with(&doc, &caches, &list, tail, &ancestors),
                "{source}, filler {filler}"
            );
            let query = crate::SelectorQuery::parse(source).unwrap();
            assert!(
                !query
                    .matcher(&doc, None)
                    .matches(StyleNodeId::new(anchor as u64), &ancestors),
                "{source}, filler {filler}"
            );
        }
    }
}

#[test]
fn single_adjacent_has_miss_does_not_index_a_wide_parent() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "main", None);
    let anchor = doc.push_element(parent, "div", None);
    for _ in 0..511 {
        doc.push_element(parent, "div", None);
    }
    let counted = SiblingCountingDoc {
        doc: &doc,
        visits: std::cell::Cell::new(0),
    };
    let query = crate::SelectorQuery::parse("div:has(+ .missing)").unwrap();
    assert!(!query.matches(
        &counted,
        StyleNodeId::new(anchor as u64),
        &[StyleNodeId::new(parent as u64)]
    ));
    assert!(
        counted.visits.get() <= 8,
        "visited {} children",
        counted.visits.get()
    );
}

#[test]
fn different_length_has_argument_lists_keep_independent_results() {
    let (doc, body, cards, paragraphs) = cards(2, 1);
    let caches = MatchCaches::default();
    let sources = [
        (".card:has(video) p", false),
        (".card:has(video, img) p", true),
        (".card:has(video, audio, canvas) p", false),
    ];
    let lists = sources.map(|(source, _)| crate::parse_selector_list(source).unwrap());
    let ancestors = [
        StyleNodeId::new(body as u64),
        StyleNodeId::new(cards[0] as u64),
    ];
    for index in [0, 1, 2, 2, 1, 0] {
        assert_eq!(
            matches_with(&doc, &caches, &lists[index], paragraphs[0].0, &ancestors),
            sources[index].1,
            "{}",
            sources[index].0
        );
    }
    assert_eq!(caches.has_lists(), 3);
    assert_eq!(caches.has_entries(), 3);
}
