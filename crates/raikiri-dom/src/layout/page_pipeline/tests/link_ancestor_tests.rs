use super::*;
use smol_str::SmolStr;

#[cfg(debug_assertions)]
thread_local! {
    static PROBES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(debug_assertions)]
pub(in crate::layout::page_pipeline) fn note_probe() {
    PROBES.with(|count| count.set(count.get() + 1));
}

fn item(node: usize, kind: PageFragmentKind, fragment: u32) -> PageFragmentItem {
    PageFragmentItem::new(
        NodeId::new(node as u64),
        PageFragmentRect::new(fragment as f32, node as f32, 10.0, 5.0),
        kind,
        fragment,
        2,
        fragment == 1,
    )
    .with_line_range(PageFragmentLineRange::new(fragment, fragment + 1))
}

fn anchor(doc: &mut Document, parent: usize, href: &str) -> usize {
    let id = doc.append_element(Some(parent), "a", Style::default(), None::<&str>);
    doc.set_element_attributes(id, vec![(SmolStr::new("href"), SmolStr::new(href))]);
    id
}

fn workload(
    anchors: usize,
    boxes: usize,
    leaves: usize,
    depth: usize,
) -> (Document, Vec<PageFragment>, Vec<(usize, usize)>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut items = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..anchors {
        // Equal href strings still belong to distinct anchor identities.
        let owner = anchor(&mut doc, body, "#same");
        items.push(item(owner, PageFragmentKind::Box, 0));
        let mut parent = owner;
        for _ in 0..depth {
            parent = doc.append_element(Some(parent), "span", Style::default(), None::<&str>);
            items.push(item(parent, PageFragmentKind::Box, 0));
        }
        for _ in 0..boxes {
            let id = doc.append_element(Some(parent), "span", Style::default(), None::<&str>);
            items.push(item(id, PageFragmentKind::Box, 0));
            expected.push((owner, id));
        }
        for _ in 0..leaves {
            let id = doc.append_text(parent, "leaf");
            items.push(item(id, PageFragmentKind::Text, 0));
            expected.push((owner, id));
        }
        if boxes == 0 && leaves == 0 {
            expected.push((owner, parent));
        }
    }
    let pages = vec![PageFragment {
        items,
        ..Default::default()
    }];
    (doc, pages, expected)
}

fn assert_workload(anchors: usize, boxes: usize, leaves: usize, depth: usize) {
    let (doc, pages, expected) = workload(anchors, boxes, leaves, depth);
    let events = page_fragment_events_from_pages(&doc, &pages);
    assert_eq!(events.len(), expected.len());
    for (event, (owner, placement)) in events.iter().zip(expected) {
        let PageFragmentEvent::Link(event) = event;
        let source = pages[0]
            .items
            .iter()
            .find(|source| source.node_id == NodeId::new(placement as u64))
            .unwrap();
        assert_eq!(event.anchor_node_id, NodeId::new(owner as u64));
        assert_eq!(event.placement_node_id, source.node_id);
        assert_eq!(event.rect, source.rect);
        assert_eq!(event.fragment_index, source.fragment_index);
        assert_eq!(event.fragment_count, source.fragment_count);
        assert_eq!(event.is_repeat, source.is_repeat);
        assert_eq!(event.line_range, source.line_range);
        assert_eq!(event.link.href, "#same");
    }
}

#[test]
fn linked_boxes_keep_empty_fallbacks_and_leaf_geometry_across_tree_shapes() {
    for (anchors, boxes, leaves, depth) in [
        (1, 0, 0, 0),
        (1, 0, 0, 24),
        (16, 0, 0, 1),
        (1, 12, 24, 0),
        (1, 12, 24, 32),
        (16, 3, 4, 8),
    ] {
        assert_workload(anchors, boxes, leaves, depth);
    }
}

#[test]
fn pages_without_linked_boxes_keep_each_leaf() {
    let (doc, mut pages, expected) = workload(1, 0, 32, 0);
    pages[0]
        .items
        .retain(|item| item.kind != PageFragmentKind::Box);
    let events = page_fragment_events_from_pages(&doc, &pages);
    assert_eq!(events.len(), expected.len());
    for (event, (owner, placement)) in events.iter().zip(expected) {
        let PageFragmentEvent::Link(event) = event;
        assert_eq!(event.anchor_node_id, NodeId::new(owner as u64));
        assert_eq!(event.placement_node_id, NodeId::new(placement as u64));
        assert_eq!(event.rect, item(placement, PageFragmentKind::Text, 0).rect);
    }
}

#[test]
fn suppression_preserves_anchor_boundaries_fragments_and_page_local_fallbacks() {
    let (mut doc, _, _) = workload(1, 0, 0, 0);
    let outer = 3;
    let inner = anchor(&mut doc, outer, "#inner");
    let image = doc.append_element(Some(inner), "img", Style::default(), None::<&str>);
    let empty = anchor(&mut doc, outer, "  ");
    let zero_child = doc.append_text(empty, "zero");
    let no_href = doc.append_element(Some(outer), "a", Style::default(), None::<&str>);
    let leaf = doc.append_text(no_href, "outer leaf");
    let mut zero = item(zero_child, PageFragmentKind::Text, 0);
    zero.rect.width = 0.0;
    let mut zero_height = item(zero_child, PageFragmentKind::Text, 1);
    zero_height.rect.height = 0.0;
    let page0 = PageFragment {
        items: vec![
            item(outer, PageFragmentKind::Box, 0),
            item(inner, PageFragmentKind::Box, 0),
            item(image, PageFragmentKind::Replaced, 0),
            item(empty, PageFragmentKind::Box, 0),
            item(empty, PageFragmentKind::Box, 1),
            zero,
            zero_height,
        ],
        ..Default::default()
    };
    let page1 = PageFragment {
        page_index: 1,
        items: vec![
            item(outer, PageFragmentKind::Box, 1),
            item(no_href, PageFragmentKind::Box, 1),
            item(leaf, PageFragmentKind::Text, 1),
        ],
        ..Default::default()
    };
    let pages = [page0, page1];
    let events = page_fragment_events_from_pages(&doc, &pages);
    let expected = [
        (0, outer, outer, 0),
        (0, inner, image, 0),
        (0, empty, empty, 0),
        (0, empty, empty, 1),
        (1, outer, leaf, 1),
    ];
    assert_eq!(events.len(), expected.len());
    for (event, (page, owner, placement, fragment)) in events.iter().zip(expected) {
        let PageFragmentEvent::Link(event) = event;
        assert_eq!(event.page_index, page);
        assert_eq!(event.anchor_node_id, NodeId::new(owner as u64));
        assert_eq!(event.placement_node_id, NodeId::new(placement as u64));
        assert_eq!(event.fragment_index, fragment);
        assert_eq!(event.fragment_count, 2);
        assert_eq!(
            event.rect,
            item(placement, PageFragmentKind::Box, fragment).rect
        );
        assert_eq!(event.is_repeat, fragment == 1);
        assert_eq!(
            event.line_range,
            Some(PageFragmentLineRange::new(fragment, fragment + 1))
        );
        assert_eq!(
            event.link.href,
            if owner == empty {
                ""
            } else if owner == inner {
                "#inner"
            } else {
                "#same"
            }
        );
    }
}

#[cfg(debug_assertions)]
#[test]
fn linked_box_suppression_probes_scale_with_items_and_ancestor_nodes() {
    for (anchors, boxes, leaves, depth) in [
        (128, 0, 0, 0),
        (1, 128, 128, 1),
        (1, 32, 32, 96),
        (16, 16, 16, 8),
    ] {
        let (doc, pages, _) = workload(anchors, boxes, leaves, depth);
        PROBES.with(|count| count.set(0));
        page_fragment_events_from_pages(&doc, &pages);
        let probes = PROBES.with(Cell::get);
        let bound = 4 * (pages[0].items.len() + doc.nodes.len());
        println!(
            "anchors={anchors} boxes={boxes} leaves={leaves} depth={depth}: probes={probes} bound={bound}"
        );
        assert!(probes > 0);
        assert!(
            probes <= bound,
            "{probes} probes exceed linear bound {bound}"
        );
    }
}

#[test]
#[ignore = "manual release performance measurement"]
fn linked_box_suppression_scaling_probe() {
    use std::hint::black_box;
    use std::time::Instant;
    for (name, anchors, boxes, leaves, depth) in [
        ("small", 1, 0, 1, 0),
        ("different_256", 256, 0, 0, 0),
        ("different_1024", 1024, 0, 0, 0),
        ("shared_256", 1, 256, 256, 1),
        ("shared_1024", 1, 1024, 1024, 1),
        ("deep_64", 1, 64, 64, 64),
        ("leaf_only", 1, 0, 1024, 0),
    ] {
        let (doc, mut pages, _) = workload(anchors, boxes, leaves, depth);
        if name == "leaf_only" {
            pages[0]
                .items
                .retain(|item| item.kind != PageFragmentKind::Box);
        }
        let initial = page_fragment_events_from_pages(&doc, &pages);
        let started = Instant::now();
        for _ in 0..20 {
            black_box(page_fragment_events_from_pages(
                black_box(&doc),
                black_box(&pages),
            ));
        }
        println!(
            "{name}: us_per_iteration={:.3} items={} events={}",
            started.elapsed().as_secs_f64() * 1e6 / 20.0,
            pages[0].items.len(),
            initial.len()
        );
    }
}
