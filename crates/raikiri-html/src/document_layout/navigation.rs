use super::DomView;
use raikiri_traits::{NodeId, NodeKind, PaintRect};
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

pub(crate) fn build_rendered(
    document: &raikiri_dom::Document,
    slices: &[raikiri_dom::PageSlice],
) -> HashSet<NodeId> {
    slices
        .iter()
        .flat_map(|slice| {
            document
                .page_fragments(slice.page_index)
                .map(|fragment| fragment.node())
        })
        .collect()
}

pub(crate) fn build_anchors(
    dom: DomView<'_>,
    document: &raikiri_dom::Document,
    slices: &[raikiri_dom::PageSlice],
) -> AnchorIndex {
    let mut first_fragment: HashMap<NodeId, Anchor> = HashMap::new();
    for slice in slices {
        for fragment in document.page_fragments(slice.page_index) {
            first_fragment.entry(fragment.node()).or_insert_with(|| {
                let rect = fragment.rect();
                Anchor {
                    page_index: slice.page_index,
                    point: (rect.x, rect.y),
                }
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
