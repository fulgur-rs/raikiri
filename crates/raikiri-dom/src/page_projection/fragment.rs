use super::records::{PageFragmentItem, PageFragmentKind, PageFragmentRect};
use raikiri_traits::{NodeId, PaintClip, PaintRect};

/// What a fragment places on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FragmentKind {
    /// An element's border box.
    Box,
    /// A text node's lines.
    Text,
    /// A replaced element such as an image.
    Replaced,
}

/// How a repeated fragment is repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepeatKind {
    /// Painted on every page (`position: fixed` in paged media).
    EveryPage,
    /// A complete copy of the first header group on a table's pages.
    TableHeader,
}

/// A local overflow clip that applies to content on one page.
///
/// The clipping element's own box need not intersect the page: descendants
/// can reach another page along an open axis. Geometry is in page-local CSS
/// pixels and retains the element's whole box before page cuts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct OverflowClip {
    /// The element that establishes this clip.
    pub node: NodeId,
    /// The element's whole border box, used to select a clipping placement.
    pub border_box: PaintRect,
    /// The resolved padding-edge clip.
    pub clip: PaintClip,
}

/// One placement of a source node on a page.
///
/// Rectangles are in CSS px with the origin at the top-left of the page box.
#[derive(Debug, Clone, Copy)]
pub struct Fragment<'a> {
    item: &'a PageFragmentItem,
    origin: (f32, f32),
    overflow_clip: Option<PaintClip>,
}

impl<'a> Fragment<'a> {
    pub(crate) fn new(item: &'a PageFragmentItem, content_box: PageFragmentRect) -> Self {
        Self {
            item,
            origin: (content_box.x, content_box.y),
            overflow_clip: None,
        }
    }

    pub(crate) fn with_overflow_clip(mut self, clip: Option<PaintClip>) -> Self {
        self.overflow_clip = clip;
        self
    }

    /// The element's resolved padding-edge overflow clip, if it clips locally.
    ///
    /// This is the shape of the whole box before page cuts, in page-local
    /// coordinates. It can extend beyond this fragment and this page. Root or
    /// body overflow propagated to the viewport does not create a local clip.
    /// Inline boxes and text fragments do not create local overflow clips.
    pub fn overflow_clip(&self) -> Option<PaintClip> {
        self.overflow_clip
    }

    /// The source node.
    pub fn node(&self) -> NodeId {
        self.item.node_id
    }

    /// What this fragment places.
    pub fn kind(&self) -> FragmentKind {
        match self.item.kind {
            PageFragmentKind::Text => FragmentKind::Text,
            PageFragmentKind::Replaced => FragmentKind::Replaced,
            PageFragmentKind::Box => FragmentKind::Box,
        }
    }

    /// Border box in layout space (before translate and relative offsets).
    pub fn rect(&self) -> PaintRect {
        let r = self.item.rect;
        PaintRect::new(r.x + self.origin.0, r.y + self.origin.1, r.width, r.height)
    }

    /// Border box in paint space, where the painter draws it. Layout space
    /// already includes the body's horizontal margins, so this equals
    /// [`Self::rect`].
    pub fn paint_rect(&self) -> PaintRect {
        self.rect()
    }

    /// Whole element content box in CSS page layout coordinates, before page cuts.
    ///
    /// Border and padding use the layout engine's resolved values, including
    /// percentage padding. A continuation may start above the page. Text
    /// fragments and inline elements with multiple pieces return `None`.
    /// Paint-time transforms, relative offsets and fixed-position adjustments
    /// are not applied, as with [`Self::rect`].
    pub fn content_rect(&self) -> Option<PaintRect> {
        self.item
            .content_rect
            .map(|r| PaintRect::new(r.x + self.origin.0, r.y + self.origin.1, r.width, r.height))
    }

    /// Ordinal of this fragment among the node's fragments.
    pub fn fragment_index(&self) -> u32 {
        self.item.fragment_index
    }

    /// Fragmentainer (column) number on the page. Always 0 until multi-column
    /// fragments are exposed.
    pub fn fragmentainer(&self) -> u32 {
        0
    }

    /// Text lines covered by this fragment, if it is a text fragment.
    pub fn line_range(&self) -> Option<std::ops::Range<u32>> {
        self.item.line_range.map(|range| range.start..range.end)
    }

    /// Whether this is the node's first fragment. Always known for a
    /// completed layout.
    pub fn is_first_fragment(&self) -> Option<bool> {
        Some(self.item.fragment_index == 0)
    }

    /// Whether this is the node's last fragment. Always known for a
    /// completed layout.
    pub fn is_last_fragment(&self) -> Option<bool> {
        Some(self.item.fragment_index + 1 >= self.item.fragment_count)
    }

    /// How this fragment is repeated, if it is.
    pub fn repeat(&self) -> Option<RepeatKind> {
        self.item
            .is_repeat
            .then_some(if self.item.is_table_header_repeat {
                RepeatKind::TableHeader
            } else {
                RepeatKind::EveryPage
            })
    }

    /// Whether this fragment is the continuation of an absolutely positioned
    /// parent on a page its own box does not reach. Always false until
    /// continuation fragments are exposed.
    pub fn continuation(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests;
