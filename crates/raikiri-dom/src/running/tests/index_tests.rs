use super::*;
use crate::running_index_diagnostics as diagnostics;

// Preserve the pre-index builder as an independent candidate/placement oracle.
fn literal_index(
    document: &Document,
    cascade: &CascadeResult,
    rendered_pages: &HashMap<usize, (u32, u32)>,
) -> RunningIndex {
    let mut index = RunningIndex::default();
    let order = preorder(document);
    // Positions before this one lie inside a `display: none` subtree.
    let mut hidden_until = 0;
    for (position, &(node, subtree_end)) in order.iter().enumerate() {
        let computed = &cascade.computed[node];
        let in_hidden_subtree = position < hidden_until;
        if computed.display == DisplayValue::None {
            hidden_until = hidden_until.max(subtree_end);
        }
        let generates_box = !in_hidden_subtree
            && !matches!(
                computed.display,
                DisplayValue::None | DisplayValue::Contents
            );
        let template = computed.running_templates.first();
        let sets_strings = generates_box && !computed.string_set.is_empty();
        if template.is_none() && !sets_strings {
            continue;
        }
        let next = order[subtree_end..]
            .iter()
            .find_map(|(id, _)| rendered_pages.get(id).map(|(first, _)| *first));
        // Ancestors of the element enclose it; they do not precede it.
        let previous = order[..position]
            .iter()
            .rev()
            .filter(|(_, end)| *end <= position)
            .find_map(|(id, _)| rendered_pages.get(id).map(|(_, last)| *last));
        let place = |page: u32| Placement {
            node: NodeId(node as u64),
            page,
            starts_page: previous.is_none_or(|last| last < page),
        };
        if let Some(template) = template {
            let page = next.or(previous).unwrap_or(0);
            index
                .pools
                .entry(template.name.clone())
                .or_default()
                .placements
                .push(place(page));
        }
        if sets_strings {
            let own = order[position..subtree_end]
                .iter()
                .find_map(|(id, _)| rendered_pages.get(id).map(|(first, _)| *first));
            let placement = place(own.or(next).or(previous).unwrap_or(0));
            for (name, _) in computed.string_set.iter() {
                let pool = index.strings.entry(name.clone()).or_default();
                // An element that sets a name twice assigns it once, with
                // the later value.
                if pool.placements.last() != Some(&placement) {
                    pool.placements.push(placement);
                }
            }
        }
    }
    for pool in index.pools.values_mut().chain(index.strings.values_mut()) {
        pool.by_page = pool.placements.is_sorted_by_key(|placement| placement.page);
    }
    index
}

fn assert_same(actual: &RunningIndex, expected: &RunningIndex) {
    for (actual, expected) in [
        (&actual.pools, &expected.pools),
        (&actual.strings, &expected.strings),
    ] {
        assert_eq!(actual.len(), expected.len());
        for (name, pool) in expected {
            let other = &actual[name];
            assert_eq!(other.placements, pool.placements, "name={name}");
            assert_eq!(other.by_page, pool.by_page, "name={name}");
            for page in 0..14 {
                for mode in [
                    StringFetchMode::First,
                    StringFetchMode::Start,
                    StringFetchMode::Last,
                    StringFetchMode::FirstExcept,
                ] {
                    assert_eq!(
                        other.select(mode, page),
                        pool.select(mode, page),
                        "name={name} page={page} mode={mode:?}"
                    );
                }
            }
        }
    }
}

fn consecutive(n: usize) -> (Document, CascadeResult, HashMap<usize, (u32, u32)>) {
    let mut document = Document::new();
    let body = document.append_element(Some(0), "body", Some("display:block"));
    for _ in 0..n {
        document.append_element(Some(body), "div", Some("display:block;position:running(h)"));
    }
    let last = document.append_element(Some(body), "p", Some("display:block"));
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    (
        document,
        cascade,
        HashMap::from([(last, (0, 0)), (body, (0, 3))]),
    )
}

#[test]
fn dense_running_queries_have_linear_neighbor_work() {
    for n in [64, 128, 256] {
        let (document, cascade, rendered) = consecutive(n);
        diagnostics::begin_running_index_diagnostics();
        let index = RunningIndex::from_rendered_pages(&document, &cascade, &rendered);
        let work = diagnostics::finish_running_index_diagnostics();
        assert_eq!(index.nodes().count(), n);
        let steps = work.next_candidate_visits
            + work.previous_candidate_visits
            + work.own_candidate_visits
            + work.prepared_nodes
            + work.prepared_slots
            + work.indexed_probes;
        assert!(steps <= 10 * (n as u64 + 4), "n={n} work={work:?}");
        assert_same(&index, &literal_index(&document, &cascade, &rendered));
    }
}

#[test]
fn zero_one_and_two_queries_use_the_expected_sparse_or_indexed_path() {
    for n in [0, 1, 2] {
        let (document, cascade, rendered) = consecutive(n);
        diagnostics::begin_running_index_diagnostics();
        let index = RunningIndex::from_rendered_pages(&document, &cascade, &rendered);
        let work = diagnostics::finish_running_index_diagnostics();
        assert_eq!(index.nodes().count(), n);
        assert_eq!(work.index_builds, u64::from(n >= 2), "n={n} work={work:?}");
        if n < 2 {
            assert_eq!(work.scratch_bytes, 0);
        }
        assert_same(&index, &literal_index(&document, &cascade, &rendered));
    }
}

#[test]
fn arbitrary_nested_trees_and_nonmonotonic_pages_match_literal_builder() {
    let styles = [
        "display:block",
        "display:block;position:running(h)",
        "display:block;string-set:t 'x',t 'later'",
        "display:none",
        "display:contents;string-set:t 'contents'",
        "display:none;string-set:t 'hidden'",
        "display:block;float:left;string-set:t 'float'",
        "display:block;position:absolute;string-set:u 'absolute'",
    ];
    let mut random = 19_u64;
    for case in 0..80 {
        let mut document = Document::new();
        let body = document.append_element(Some(0), "body", Some("display:block"));
        let mut parents = vec![body];
        for _ in 0..32 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let parent = parents[(random >> 32) as usize % parents.len()];
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let id = document.append_element(
                Some(parent),
                "div",
                Some(styles[(random >> 32) as usize % styles.len()]),
            );
            parents.push(id);
        }
        let outside = document.append_element(None, "div", Some("display:block"));
        document.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
        let mut rendered = HashMap::new();
        for (id, _) in preorder(&document) {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            if case % 10 != 0 && random & 3 != 0 {
                let first = (random >> 32) % 10;
                rendered.insert(id, (first as u32, (first + ((random >> 16) & 3)) as u32));
            }
        }
        rendered.insert(outside, (11, 13));
        let actual = RunningIndex::from_rendered_pages(&document, &cascade, &rendered);
        assert_same(&actual, &literal_index(&document, &cascade, &rendered));
    }
}

#[test]
fn rendered_parent_never_overrides_later_preorder_child_at_same_or_later_close() {
    for separate_close in [false, true] {
        let mut document = Document::new();
        let body = document.append_element(Some(0), "body", Some("display:block"));
        let first = document.append_element(
            Some(body),
            "div",
            Some("display:block;string-set:t 'first'"),
        );
        let parent = document.append_element(Some(body), "div", Some("display:block"));
        let child = document.append_element(Some(parent), "div", Some("display:block"));
        if separate_close {
            document.append_element(Some(parent), "div", Some("display:block"));
        }
        let tail =
            document.append_element(Some(body), "div", Some("display:block;string-set:t 'tail'"));
        let next = document.append_element(Some(body), "div", Some("display:block"));
        document.mark_in_document_flags();
        let cascade =
            raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
        let rendered = HashMap::from([
            (parent, (9, 9)),
            (child, (1, 1)),
            (next, (2, 2)),
            (body, (0, 12)),
        ]);
        let actual = RunningIndex::from_rendered_pages(&document, &cascade, &rendered);
        assert_same(&actual, &literal_index(&document, &cascade, &rendered));
        let pool = &actual.strings["t"];
        assert!(!pool.by_page);
        assert_eq!(pool.placements[0].node, NodeId(first as u64));
        assert_eq!(pool.placements[0].page, 9);
        assert_eq!(
            pool.placements[1],
            Placement {
                node: NodeId(tail as u64),
                page: 2,
                starts_page: true
            }
        );
    }
}

#[test]
fn actual_multipage_float_and_positioned_fragments_match_literal_builder() {
    let mut document = Document::new();
    let fonts = crate::fonts::build_bundled_font_collection(
        vec![crate::fonts::BundledFace {
            family: "Ahem".into(),
            bytes: include_bytes!("../../../tests/data/text-autospace/Ahem.ttf").to_vec(),
        }],
        false,
    )
    .unwrap();
    document.set_font_collection(fonts);
    let html = document.append_element(
        Some(0),
        "html",
        Some("display:block;font-family:Ahem;font-size:10px"),
    );
    let body = document.append_element(Some(html), "body", Some("display:block;position:relative"));
    document.append_element(Some(body), "div", Some("display:block;position:running(h)"));
    let first = document.append_element(
        Some(body),
        "p",
        Some("display:block;height:20px;margin:0;string-set:t 'first'"),
    );
    document.append_text(first, "a");
    let float = document.append_element(
        Some(body),
        "div",
        Some("display:block;float:left;width:10px;height:10px;background:blue"),
    );
    document.append_element(Some(body), "div", Some("display:block;position:running(h)"));
    let positioned=document.append_element(Some(body),"div",Some("display:block;position:absolute;top:0;left:0;width:10px;height:10px;background:red;string-set:t 'absolute'"));
    let second = document.append_element(
        Some(body),
        "p",
        Some("display:block;height:20px;margin:0;break-before:page;string-set:t 'second'"),
    );
    document.append_text(second, "b");
    document.append_element(Some(body), "div", Some("display:block;position:running(h)"));
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    let slices = crate::layout_pages(&mut document, &cascade, page).unwrap();
    assert!(slices.len() >= 2);
    let geometries = slices
        .iter()
        .map(|_| (page, PageMargins::default(), PageContentInsets::default()))
        .collect::<Vec<_>>();
    document
        .project_pages(&cascade, page, &slices, &geometries)
        .unwrap();
    let mut rendered = HashMap::<usize, (u32, u32)>::new();
    for page in 0..slices.len() as u32 {
        for fragment in document.page_fragments(page) {
            rendered
                .entry(fragment.node().0 as usize)
                .and_modify(|(first, last)| {
                    *first = (*first).min(page);
                    *last = (*last).max(page);
                })
                .or_insert((page, page));
        }
    }
    assert!(rendered.contains_key(&float));
    assert!(rendered.contains_key(&positioned));
    let actual = RunningIndex::build(
        &document,
        &cascade,
        (0..slices.len() as u32).map(|page| (&document, page)),
    );
    assert_same(&actual, &literal_index(&document, &cascade, &rendered));
}

#[test]
fn own_boundary_and_ancestor_exclusion_use_preorder_not_node_ids() {
    use super::super::neighbors::RenderedNeighbors;
    let order = [(50, 4), (3, 2), (40, 3), (9, 4)];
    let rendered = HashMap::from([(50, (11, 12)), (40, (8, 9)), (999, (0, 13))]);
    let mut lookup = RenderedNeighbors::new(&order, &rendered);
    let mut work = diagnostics::RunningIndexWork::default();
    let sparse = lookup.query::<true>(1, 2, true, &mut work);
    assert_eq!(
        (sparse.next, sparse.previous, sparse.own),
        (Some(8), None, None)
    );
    assert_eq!(work.index_builds, 0);
    let indexed = lookup.query::<true>(1, 2, true, &mut work);
    assert_eq!(
        (indexed.next, indexed.previous, indexed.own),
        (Some(8), None, None)
    );
    assert_eq!(work.index_builds, 1);
    let own = lookup.query::<true>(2, 3, true, &mut work);
    assert_eq!((own.next, own.previous, own.own), (None, None, Some(8)));
    let after = lookup.query::<true>(3, 4, true, &mut work);
    assert_eq!(
        (after.next, after.previous, after.own),
        (None, Some(9), None)
    );
}

#[test]
fn string_neighbor_work_is_counted_and_diagnostics_can_be_reset() {
    let (document, cascade, [first, heading, _, _, _, second, body]) = string_document();
    let rendered = HashMap::from([
        (first, (0, 0)),
        (heading + 1, (1, 2)),
        (second, (2, 2)),
        (body, (0, 9)),
    ]);
    diagnostics::begin_running_index_diagnostics();
    let index = RunningIndex::from_rendered_pages(&document, &cascade, &rendered);
    let work = diagnostics::finish_running_index_diagnostics();
    assert!(work.own_candidate_visits > 0);
    assert_eq!(work.index_builds, 1);
    assert!(work.prepared_nodes > 0);
    assert_same(&index, &literal_index(&document, &cascade, &rendered));
    diagnostics::begin_running_index_diagnostics();
    diagnostics::begin_running_index_diagnostics();
    assert_eq!(
        diagnostics::finish_running_index_diagnostics(),
        Default::default()
    );
    assert_eq!(
        diagnostics::finish_running_index_diagnostics(),
        Default::default()
    );
}
