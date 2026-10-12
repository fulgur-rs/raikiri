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
    match_complex_selector_list(list, ctx, &elem, id, ancestors).is_some()
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
        assert_eq!(&*caches.content_language(&doc, &elem, id, &ancestors), "en");
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
    assert_eq!(&*caches.content_language(&doc, &elem, id, &ancestors), "");
    assert_eq!(caches.languages.borrow().len(), 3);
}
