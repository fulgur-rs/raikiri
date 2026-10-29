use super::*;

fn parse_chain(html: &str) -> crate::HtmlDocument {
    crate::parse_html(
        html.as_bytes(),
        &ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap()
}

#[test]
fn avoided_sibling_chains_visit_entries_linearly() {
    for n in [32, 64, 128] {
        for gaps in [false, true] {
            let item = if gaps {
                "text<!--gap--><div></div>"
            } else {
                "<div></div>"
            };
            let html = format!(
                "<style>div{{break-before:avoid}}div:first-of-type{{break-before:auto}}</style>{}",
                item.repeat(n)
            );
            let input = parse_chain(&html);
            let ids: Vec<_> = (0..input.dom().node_count())
                .filter(|&id| input.dom().get_node(id).unwrap().tag_name() == Some("div"))
                .collect();
            SIBLING_VISITS.with(|visits| visits.set(0));
            assert_eq!(
                join_avoided_breaks(input.dom(), input.cascade(), *ids.last().unwrap()),
                ids[0]
            );
            let visits = SIBLING_VISITS.with(std::cell::Cell::get);
            eprintln!("frontier n={n} gaps={gaps} visits={visits}");
            assert!(visits <= 8 * n + 16, "n={n} gaps={gaps} visits={visits}");
        }
    }
}

fn old_join_avoided_breaks(document: &Document, cascade: &CascadeResult, id: usize) -> usize {
    let mut result = id;
    let mut current = id;
    let mut avoid_before = false;
    loop {
        avoid_before |=
            computed(cascade, current).is_some_and(|values| avoids_break(values.break_before));
        match old_previous_element_sibling(document, current) {
            Some(previous) => {
                let avoid_after = computed(cascade, previous)
                    .is_some_and(|values| avoids_break(values.break_after));
                if !(avoid_before || avoid_after) {
                    return result;
                }
                result = previous;
                current = previous;
                avoid_before = false;
            }
            None => match document.parent_of(current) {
                Some(parent) if parent != document.root_index() => current = parent,
                _ => return result,
            },
        }
    }
}

fn old_previous_element_sibling(document: &Document, id: usize) -> Option<usize> {
    let parent = document.parent_of(id)?;
    let siblings = &document.get_node(parent)?.children;
    let position = siblings.iter().position(|&sibling| sibling == id)?;
    siblings[..position].iter().rev().copied().find(|&sibling| {
        document
            .get_node(sibling)
            .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
    })
}

#[test]
fn avoided_chains_keep_linear_work_while_ascending_multiple_parents() {
    for n in [32, 64, 128] {
        let mut html = "<style>div,section{break-before:avoid-column}.outer>div:first-of-type{break-before:auto}</style>".to_owned();
        for level in 0..3 {
            html.push_str(if level == 0 {
                "<section class='outer' style='break-before:auto'>"
            } else {
                "<section>"
            });
            html.push_str(&"text<!--gap--><div></div>".repeat(n));
        }
        html.push_str("</section></section></section>");
        let input = parse_chain(&html);
        let ids: Vec<_> = (0..input.dom().node_count())
            .filter(|&id| input.dom().get_node(id).unwrap().tag_name() == Some("div"))
            .collect();
        assert_eq!(
            old_join_avoided_breaks(input.dom(), input.cascade(), *ids.last().unwrap()),
            ids[0]
        );
        SIBLING_VISITS.with(|visits| visits.set(0));
        assert_eq!(
            join_avoided_breaks(input.dom(), input.cascade(), *ids.last().unwrap()),
            ids[0]
        );
        let visits = SIBLING_VISITS.with(std::cell::Cell::get);
        eprintln!("frontier ancestors n={n} visits={visits}");
        assert!(visits <= 24 * n + 48, "n={n} visits={visits}");
    }
}

#[test]
fn avoided_chain_matches_original_for_break_values_and_source_shapes() {
    use raikiri_style::property::BreakBetween;
    let values = [
        BreakBetween::Auto,
        BreakBetween::Avoid,
        BreakBetween::AvoidPage,
        BreakBetween::Page,
        BreakBetween::Always,
        BreakBetween::AvoidColumn,
        BreakBetween::Column,
    ];
    for html in [
        "<div></div><div></div><div></div>",
        "text<!--gap--><div></div>more<!--gap--><div></div>",
        "<h2></h2><section><article><p>x</p></article></section>",
        "<div style='display:none'></div><svg><g></g><g></g></svg><div></div>",
    ] {
        let (mut input, mut cascade) = parse_chain(html).into_parts();
        for detached in [false, true] {
            if detached {
                let id = (0..input.dom.node_count())
                    .find(|&id| input.dom.get_node(id).unwrap().tag_name() == Some("body"))
                    .unwrap();
                input.dom.detach_from_parent(id);
                input.dom.mark_in_document_flags();
            }
            for &before in &values {
                for &after in &values {
                    for computed in &mut cascade.computed {
                        computed.break_before = before;
                        computed.break_after = after;
                    }
                    for id in 0..=input.dom.node_count() {
                        assert_eq!(
                            join_avoided_breaks(&input.dom, &cascade, id),
                            old_join_avoided_breaks(&input.dom, &cascade, id),
                            "html={html} detached={detached} before={before:?} after={after:?} id={id}"
                        );
                    }
                }
            }
        }
        cascade.computed.truncate(cascade.computed.len() / 2);
        for id in 0..=input.dom.node_count() {
            assert_eq!(
                join_avoided_breaks(&input.dom, &cascade, id),
                old_join_avoided_breaks(&input.dom, &cascade, id)
            );
        }
    }
}

#[test]
fn first_child_avoidance_survives_ancestor_promotion_without_changing_unjoined_result() {
    let input = parse_chain(
        "<h2>heading</h2><section><article><p style='break-before:avoid-page'>text</p></article></section>",
    );
    let heading = (0..input.dom().node_count())
        .find(|&id| input.dom().get_node(id).unwrap().tag_name() == Some("h2"))
        .unwrap();
    let text = (0..input.dom().node_count())
        .rfind(|&id| input.dom().get_node(id).unwrap().kind() == NodeKind::Text)
        .unwrap();
    assert_eq!(
        join_avoided_breaks(input.dom(), input.cascade(), text),
        heading
    );
    let (input, mut cascade) = input.into_parts();
    for computed in &mut cascade.computed {
        computed.break_before = BreakBetween::Auto;
        computed.break_after = BreakBetween::Auto;
    }
    assert_eq!(join_avoided_breaks(&input.dom, &cascade, text), text);
}

#[test]
fn missing_current_computed_values_still_join_an_avoiding_previous_sibling() {
    let (input, mut cascade) =
        parse_chain("<div></div><div style='break-after:avoid-page'></div><div></div>")
            .into_parts();
    let ids: Vec<_> = (0..input.dom.node_count())
        .filter(|&id| input.dom.get_node(id).unwrap().tag_name() == Some("div"))
        .collect();
    cascade.computed.truncate(ids[2]);
    assert_eq!(join_avoided_breaks(&input.dom, &cascade, ids[2]), ids[1]);
    assert_eq!(
        old_join_avoided_breaks(&input.dom, &cascade, ids[2]),
        ids[1]
    );
}
