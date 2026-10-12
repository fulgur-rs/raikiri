use super::DomView;
use crate::render::PipelineOutput;
use raikiri_traits::{NodeId, NodeKind, PaintRect};
use std::collections::{HashMap, HashSet};

/// Destination of an in-document link.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Anchor {
    /// Zero-based page index.
    pub page_index: u32,
    /// Top-left of the target's first fragment, CSS px from the page box
    /// origin, where the built-in painter draws it: the layout corner mapped
    /// through the fragment's transform (see [`super::Page::paint_transform`]).
    /// A target a singular transform hides keeps its layout corner.
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

    /// Add the anchors of `other` on pages before `before_page` whose names
    /// this index does not hold yet.
    pub(crate) fn insert_missing(&mut self, other: &AnchorIndex, before_page: u32) {
        for (name, anchor) in &other.by_name {
            if anchor.page_index < before_page && !self.by_name.contains_key(name) {
                self.by_name.insert(name.clone(), anchor.clone());
            }
        }
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
    /// Clickable areas in layout space, CSS px from the page box origin,
    /// before relative offsets and transforms move them.
    pub quads: &'a [PaintRect],
    /// The clickable areas where the built-in painter draws them, CSS px
    /// from the page box origin: the corners of each of [`Self::quads`]
    /// (top-left, top-right, bottom-right, bottom-left of the layout
    /// rectangle) mapped through the transform of its placement, as
    /// [`super::Page::paint_transform`] reports it. A rotated or skewed
    /// placement gives a quadrilateral that is not a rectangle. Quads of
    /// placements a singular transform hides are left out.
    pub paint_quads: Vec<[(f32, f32); 4]>,
}

pub(crate) fn build_rendered(out: &PipelineOutput) -> HashSet<NodeId> {
    out.slices
        .iter()
        .flat_map(|slice| {
            out.layout_for_page(slice.page_index)
                .0
                .page_fragments(slice.page_index)
                .map(|fragment| fragment.node())
        })
        .collect()
}

pub(crate) fn build_anchors(dom: DomView<'_>, out: &PipelineOutput) -> AnchorIndex {
    let mut first_fragment: HashMap<NodeId, Anchor> = HashMap::new();
    for slice in &out.slices {
        let document = out.layout_for_page(slice.page_index).0;
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
            if let Some(anchor) = first_fragment.get(&node).filter(|_| !names.is_empty()) {
                // Only named nodes become anchors, so only their points are
                // moved to where the painter draws them.
                let (document, cascade) = out.layout_for_page(anchor.page_index);
                let point = document
                    .page_paint_transform(cascade, anchor.page_index, node)
                    .map_or(anchor.point, |transform| transform.apply(anchor.point));
                let anchor = Anchor {
                    point,
                    ..anchor.clone()
                };
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
