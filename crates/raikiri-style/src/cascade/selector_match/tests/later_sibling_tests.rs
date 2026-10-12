use super::*;

/// A parent wide enough for [`SiblingInfo`], with class tokens from a fixed
/// pseudo-random sequence and non-element children between the elements.
fn wide_list(len: usize) -> (TestDoc, usize, Vec<(usize, &'static str)>) {
    const CLASSES: [&str; 4] = ["a", "b", "t", "a t"];
    let mut doc = TestDoc::new();
    let list = doc.push_element(0, "ul", None);
    let mut state = 7u32;
    let mut items = Vec::new();
    for i in 0..len {
        if i % 3 == 0 {
            doc.push_text(list, " ");
        }
        if i % 7 == 0 {
            doc.push_comment(list, "c");
        }
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let class = CLASSES[(state >> 16) as usize % CLASSES.len()];
        let item = doc.push_element(list, "li", None);
        doc.set_attr(item, "class", class);
        items.push((item, class));
    }
    (doc, list, items)
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

fn has(class: &str, token: &str) -> bool {
    class.split(' ').any(|t| t == token)
}

#[test]
fn shared_later_sibling_records_agree_with_a_direct_oracle() {
    let (doc, list, items) = wide_list(40);
    let ancestors = [StyleNodeId::new(list as u64)];
    let selector = crate::parse_selector_list(".a ~ .b ~ .t").unwrap();
    // `.a ~ .b ~ .t`: some `.b` before the item that itself follows an `.a`.
    let expected = |k: usize| {
        has(items[k].1, "t")
            && (0..k).any(|j| has(items[j].1, "b") && (0..j).any(|i| has(items[i].1, "a")))
    };
    let mut forward_orders: Vec<Vec<usize>> =
        vec![(0..items.len()).collect(), (0..items.len()).rev().collect()];
    // Middle out, so records are first built from an element in the middle.
    let mut middle_out = Vec::new();
    let mid = items.len() / 2;
    for step in 0..=mid {
        middle_out.extend(
            [mid + step, mid.wrapping_sub(step)]
                .into_iter()
                .filter(|&k| k < items.len()),
        );
    }
    middle_out.dedup();
    forward_orders.push(middle_out);
    let mut any_match = false;
    for order in forward_orders {
        let caches = MatchCaches::default();
        for k in order {
            let got = matches_with(&doc, &caches, &selector, items[k].0, &ancestors);
            assert_eq!(got, expected(k), "item {k} ({})", items[k].1);
            any_match |= got;
        }
        assert!(caches.later_sibling_entries() > 0);
    }
    assert!(any_match, "the fixture must exercise a match");
}

#[test]
fn shared_later_sibling_records_agree_with_fresh_caches() {
    let (doc, list, items) = wide_list(48);
    let ancestors = [StyleNodeId::new(list as u64)];
    for source in [
        ".a ~ .b",
        ".t ~ .t",
        ".a ~ .b ~ .t",
        ".a + .b ~ .t",
        ".a ~ .b + .t",
        "ul > .a ~ .t",
        ".b ~ :not(.a) ~ .t",
        ".a ~ li ~ li ~ .b",
        ":has(~ .b) ~ .t",
        ".t:has(~ .a ~ .b)",
        "li:nth-child(2n of .t) ~ .b",
    ] {
        let selector = crate::parse_selector_list(source).unwrap();
        for reverse in [false, true] {
            let shared = MatchCaches::default();
            let mut order = (0..items.len()).collect::<Vec<_>>();
            if reverse {
                order.reverse();
            }
            for k in order {
                let item = items[k].0;
                assert_eq!(
                    matches_with(&doc, &shared, &selector, item, &ancestors),
                    matches_with(&doc, &MatchCaches::default(), &selector, item, &ancestors),
                    "{source} on item {k} ({}), reverse = {reverse}",
                    items[k].1
                );
            }
        }
    }
}

#[test]
fn later_sibling_narrow_parent_keeps_the_direct_scan() {
    let mut doc = TestDoc::new();
    let list = doc.push_element(0, "ul", None);
    let a = doc.push_element(list, "li", None);
    doc.set_attr(a, "class", "a");
    let b = doc.push_element(list, "li", None);
    let selector = crate::parse_selector_list(".a ~ li").unwrap();
    let caches = MatchCaches::default();
    let ancestors = [StyleNodeId::new(list as u64)];
    assert!(matches_with(&doc, &caches, &selector, b, &ancestors));
    assert!(!matches_with(&doc, &caches, &selector, a, &ancestors));
    assert_eq!(caches.later_sibling_entries(), 0);
}

#[test]
fn later_sibling_cascade_matches_every_follower_of_a_wide_list() {
    // The cascade path shares one record across every list item.
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, ".a ~ li { background-color: red }");
    let list = doc.push_element(0, "ul", None);
    let items = (0..30)
        .map(|i| {
            let item = doc.push_element(list, "li", None);
            if i == 12 {
                doc.set_attr(item, "class", "a");
            }
            item
        })
        .collect::<Vec<_>>();
    let tree = build_rule_tree(&doc);
    let r = cascade(&doc, &tree).expect("cascade Ok");
    for (i, &item) in items.iter().enumerate() {
        let red = r.computed[item].background_color == RED;
        assert_eq!(red, i > 12, "item {i}");
    }
}

#[test]
fn later_sibling_cache_limit_falls_back_to_the_direct_scan() {
    let (doc, list, items) = wide_list(24);
    let ancestors = [StyleNodeId::new(list as u64)];
    let selector = crate::parse_selector_list(".a ~ .b ~ .t").unwrap();
    // Room for the outer `~` frame's record only; the nested frame scans.
    let limited = MatchCaches::with_later_sibling_limit(1);
    for &(item, class) in &items {
        assert_eq!(
            matches_with(&doc, &limited, &selector, item, &ancestors),
            matches_with(&doc, &MatchCaches::default(), &selector, item, &ancestors),
            "item {item} ({class})"
        );
    }
    assert_eq!(limited.later_sibling_entries(), 1);
}
