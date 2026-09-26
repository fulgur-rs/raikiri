use super::DomView;
use raikiri_traits::{NodeId, NodeKind, PageFragment, PageFragmentEvent, PaintRect};
use std::collections::{HashMap, HashSet};

/// Destination of an in-document link.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Anchor {
    /// Zero-based page index.
    pub page_index: u32,
    /// Top-left of the target's first fragment, CSS px from the page box
    /// origin.
    pub point: (f32, f32),
}

/// Anchors by name (`id` attributes and `<a name>`). The first node in
/// document order wins; nodes without fragments are not indexed.
#[derive(Debug, Default)]
pub struct AnchorIndex {
    by_name: HashMap<String, Anchor>,
}

impl AnchorIndex {
    /// Look up an anchor by name.
    pub fn get(&self, name: &str) -> Option<&Anchor> {
        self.by_name.get(name)
    }

    /// Number of anchors.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether there are no anchors.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// One link on a page: all quads of one `<a>` element for one target.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Link<'a> {
    /// The `<a>` element.
    pub owner: NodeId,
    /// The `href`, trimmed.
    pub target: &'a str,
    /// Clickable areas, CSS px from the page box origin.
    pub quads: &'a [PaintRect],
}

pub(crate) struct PageLinks {
    pub(crate) entries: Vec<(NodeId, String, Vec<PaintRect>)>,
}

pub(crate) fn build_links(pages: &[PageFragment], events: &[PageFragmentEvent]) -> Vec<PageLinks> {
    let mut per_page: Vec<PageLinks> = pages
        .iter()
        .map(|_| PageLinks {
            entries: Vec::new(),
        })
        .collect();
    for event in events {
        let PageFragmentEvent::Link(link) = event else {
            continue;
        };
        let target = link.link.href.trim();
        if target.is_empty() {
            continue;
        }
        let Some(page) = pages.iter().position(|p| p.page_index == link.page_index) else {
            continue;
        };
        let content_box = pages[page].content_box;
        let quad = PaintRect::new(
            link.rect.x + content_box.x,
            link.rect.y + content_box.y,
            link.rect.width,
            link.rect.height,
        );
        let entries = &mut per_page[page].entries;
        match entries
            .iter_mut()
            .find(|(owner, t, _)| *owner == link.anchor_node_id && t == target)
        {
            Some((_, _, quads)) => quads.push(quad),
            None => entries.push((link.anchor_node_id, target.to_owned(), vec![quad])),
        }
    }
    per_page
}

pub(crate) fn build_rendered(pages: &[PageFragment]) -> HashSet<NodeId> {
    pages
        .iter()
        .flat_map(|p| p.items.iter().map(|item| item.node_id))
        .collect()
}

pub(crate) fn build_anchors(dom: DomView<'_>, pages: &[PageFragment]) -> AnchorIndex {
    let mut first_fragment: HashMap<NodeId, Anchor> = HashMap::new();
    for page in pages {
        for item in &page.items {
            first_fragment
                .entry(item.node_id)
                .or_insert_with(|| Anchor {
                    page_index: page.page_index,
                    point: (
                        item.rect.x + page.content_box.x,
                        item.rect.y + page.content_box.y,
                    ),
                });
        }
    }
    let mut index = AnchorIndex::default();
    let mut stack = vec![dom.root()];
    while let Some(node) = stack.pop() {
        if dom.kind(node) == Some(NodeKind::Element) {
            let mut names = Vec::new();
            if let Some(id) = dom.attr(node, "id") {
                names.push(id);
            }
            if dom.local_name(node) == Some("a")
                && let Some(name) = dom.attr(node, "name")
            {
                names.push(name);
            }
            if let Some(anchor) = first_fragment.get(&node) {
                for name in names {
                    index
                        .by_name
                        .entry(name.to_owned())
                        .or_insert_with(|| anchor.clone());
                }
            }
        }
        let children: Vec<_> = dom.children(node).collect();
        stack.extend(children.into_iter().rev());
    }
    index
}
