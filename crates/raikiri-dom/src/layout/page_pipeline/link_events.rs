//! Page-local link events projected from page snapshots.

use super::{
    Document, HashMap, HashSet, NodeId, NodeKind, PageFragment, PageFragmentEvent,
    PageFragmentItem, PageFragmentKind, PageFragmentLink, PageFragmentLinkEvent, find_body,
};

/// Collect deterministic page-local link events from page snapshots.
///
/// The event geometry is copied from the correlated `PageFragmentItem`, so a
/// consumer can join on `(placement_node_id, page_index)` without seeing the
/// layout engine's internal tree. `anchor_node_id` preserves the owning `<a>`
/// identity when a link wraps text or replaced descendants. Raw trimmed
/// `href` values, including empty and relative values, are not URL-parsed.
/// A box ancestor is omitted when a more specific text or replaced placement
/// already represents the same link, retaining one useful hit rectangle per
/// visible leaf and a fallback for empty/non-text links.
pub(crate) fn page_fragment_events_from_pages(
    document: &Document,
    pages: &[PageFragment],
) -> Vec<PageFragmentEvent> {
    let Some(body_id) = find_body(document) else {
        return Vec::new();
    };

    let mut owners = HashMap::<usize, usize>::new();
    let mut hrefs = HashMap::<usize, String>::new();
    let mut stack = vec![(body_id, None::<usize>)];
    while let Some((node_id, inherited_owner)) = stack.pop() {
        let node = &document.layout_node_with_style(node_id).unwrap();
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let owner = if node.kind() == NodeKind::Element && node.tag_name() == Some("a") {
            node.attribute("href")
                .map(str::trim)
                .map(|href| {
                    hrefs.entry(node_id).or_insert_with(|| href.to_owned());
                    node_id
                })
                .or(inherited_owner)
        } else {
            inherited_owner
        };
        if let Some(owner) = owner {
            owners.insert(node_id, owner);
        }
        for &child_id in node.children.iter().rev() {
            stack.push((child_id, owner));
        }
    }

    let mut events = Vec::new();
    for page in pages {
        let linked_items: Vec<&PageFragmentItem> = page
            .items
            .iter()
            .filter(|item| {
                usize::try_from(item.node_id.0)
                    .ok()
                    .and_then(|node_id| owners.get(&node_id))
                    .and_then(|owner| hrefs.get(owner))
                    .is_some()
                    && item.rect.width > 0.0
                    && item.rect.height > 0.0
            })
            .collect();
        let mut suppressed_boxes = HashSet::new();
        if linked_items
            .iter()
            .any(|item| item.kind == PageFragmentKind::Box)
        {
            for item in &linked_items {
                let node_id = usize::try_from(item.node_id.0)
                    .expect("linked item NodeId must fit the local arena index");
                let owner = owners.get(&node_id);
                // Start at the parent: another fragment of this node is not
                // a descendant. A marked ancestor's remaining chain has
                // already been visited for this page and anchor identity.
                let mut parent = document.parent_of(node_id);
                while let Some(id) = parent {
                    #[cfg(all(test, debug_assertions))]
                    super::tests::link_ancestor_tests::note_probe();
                    if owners.get(&id) != owner || !suppressed_boxes.insert(id) {
                        break;
                    }
                    parent = document.parent_of(id);
                }
            }
        }
        for item in linked_items.iter().copied() {
            // `linked_items` has already validated all three lookups above.
            // Keeping the invariant explicit here avoids a second set of
            // impossible branches in the hot event projection loop.
            let placement_node_id = usize::try_from(item.node_id.0)
                .expect("linked item NodeId must fit the local arena index");
            let anchor_id = *owners
                .get(&placement_node_id)
                .expect("linked item must have an anchor owner");
            let href = hrefs
                .get(&anchor_id)
                .expect("anchor owner must retain its href");
            if item.kind == PageFragmentKind::Box {
                #[cfg(all(test, debug_assertions))]
                super::tests::link_ancestor_tests::note_probe();
                if suppressed_boxes.contains(&placement_node_id) {
                    continue;
                }
            }
            events.push(PageFragmentEvent::Link(PageFragmentLinkEvent::new(
                NodeId::new(anchor_id as u64),
                item.node_id,
                page.page_index,
                item.rect,
                item.fragment_index,
                item.fragment_count,
                item.is_repeat,
                item.line_range,
                PageFragmentLink::new(href.clone()),
            )));
        }
    }
    events.sort_by(|left, right| match (left, right) {
        (PageFragmentEvent::Link(left), PageFragmentEvent::Link(right)) => left
            .page_index
            .cmp(&right.page_index)
            .then_with(|| left.anchor_node_id.cmp(&right.anchor_node_id))
            .then_with(|| left.placement_node_id.cmp(&right.placement_node_id))
            .then_with(|| left.rect.y.total_cmp(&right.rect.y))
            .then_with(|| left.rect.x.total_cmp(&right.rect.x))
            .then_with(|| left.fragment_index.cmp(&right.fragment_index)),
    });
    events
}
