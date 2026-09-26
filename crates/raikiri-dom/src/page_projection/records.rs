use raikiri_traits::{NodeId, PageBox};
#[cfg(test)]
use std::collections::BTreeMap;

/// Resolved geometry metadata for one page in a page-fragment stream.
///
/// The values are page-local physical metadata: `content_box.x/y` is the
/// offset from the physical page origin, while a [`PageFragmentItem`]'s
/// rectangle is relative to that content-box origin. A producer supplies one
/// value per page when page size, margins, or insets vary; consumers must not
/// recompute these values from `page_name`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragmentPageGeometry {
    /// Zero-based page index to which this metadata belongs.
    pub(crate) page_index: u32,
    /// Resolved physical page box in CSS pixels.
    pub(crate) page_box: PageBox,
    /// Resolved page margins in CSS pixels.
    pub(crate) margins: PageFragmentInsets,
    /// Resolved border/padding inset inside the page margin area.
    pub(crate) content_insets: PageFragmentInsets,
    /// Page-local physical origin and flow dimensions in CSS pixels.
    ///
    /// `x/y` is the offset a consumer adds once to an item rectangle. The
    /// current producer keeps horizontal page insets out of the flow
    /// containing-block width, so `width` is the scheduled flow width while
    /// `x/y` still includes the resolved margin and inset.
    pub(crate) content_box: PageFragmentRect,
    /// Physical orientation derived from `page_box`.
    pub(crate) orientation: PageFragmentOrientation,
}

impl PageFragmentPageGeometry {
    /// Construct resolved metadata for one page.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        page_index: u32,
        page_box: PageBox,
        margins: PageFragmentInsets,
        content_insets: PageFragmentInsets,
        content_box: PageFragmentRect,
        orientation: PageFragmentOrientation,
    ) -> Self {
        Self {
            page_index,
            page_box,
            margins,
            content_insets,
            content_box,
            orientation,
        }
    }

    /// Return the same metadata for another page index.
    pub(crate) fn with_page_index(mut self, page_index: u32) -> Self {
        self.page_index = page_index;
        self
    }
}

/// One committed page of neutral layout geometry.
///
/// The producer is `raikiri-dom`'s pagination layer. The type deliberately
/// carries only CSS-px geometry and neutral identifiers; it does not expose
/// Taffy, Parley, `raikiri_style`, or a renderer-specific drawable payload.
/// Its [`items`](Self::items) are ordered deterministically by the
/// pagination producer.
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragment {
    /// Zero-based page index.
    pub(crate) page_index: u32,
    /// Resolved physical page box in CSS pixels.
    pub(crate) page_box: PageBox,
    /// Resolved page margins in CSS pixels.
    pub(crate) margins: PageFragmentInsets,
    /// Resolved border/padding inset inside the page margin area.
    pub(crate) content_insets: PageFragmentInsets,
    /// Page-local physical origin and flow dimensions in CSS pixels.
    ///
    /// `x/y` is the offset a consumer adds once to an item rectangle. The
    /// current producer keeps horizontal page insets out of the flow
    /// containing-block width, so `width` is the scheduled flow width while
    /// `x/y` still includes the resolved margin and inset.
    pub(crate) content_box: PageFragmentRect,
    /// The page's origin in the shared document body-content coordinate space.
    pub(crate) content_origin_y: f32,
    /// Resolved page name, if the page context selected one.
    pub(crate) page_name: Option<String>,
    /// Physical orientation derived from the resolved page box.
    pub(crate) orientation: PageFragmentOrientation,
    /// Per-node placements intersecting this page, in deterministic order.
    pub(crate) items: Vec<PageFragmentItem>,
}

impl PageFragment {
    /// Construct an empty page fragment snapshot.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Construct a page snapshot from independently resolved page metadata.
    pub(crate) fn with_page_geometry(
        page_index: u32,
        geometry: PageFragmentPageGeometry,
        content_origin_y: f32,
        page_name: Option<String>,
    ) -> Self {
        Self {
            page_index,
            page_box: geometry.page_box,
            margins: geometry.margins,
            content_insets: geometry.content_insets,
            content_box: geometry.content_box,
            content_origin_y,
            page_name,
            orientation: geometry.orientation,
            items: Vec::new(),
        }
    }

    /// Construct a page snapshot with resolved metadata and no items.
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn with_metadata(
        page_index: u32,
        page_box: PageBox,
        margins: PageFragmentInsets,
        content_insets: PageFragmentInsets,
        content_box: PageFragmentRect,
        content_origin_y: f32,
        page_name: Option<String>,
        orientation: PageFragmentOrientation,
    ) -> Self {
        Self::with_page_geometry(
            page_index,
            PageFragmentPageGeometry::new(
                page_index,
                page_box,
                margins,
                content_insets,
                content_box,
                orientation,
            ),
            content_origin_y,
            page_name,
        )
    }

    /// Whether this page contains no visible node placements.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A CSS-px rectangle used by page-fragment metadata and placements.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragmentRect {
    /// Left edge or x coordinate in CSS pixels.
    pub(crate) x: f32,
    /// Top edge or y coordinate in CSS pixels.
    pub(crate) y: f32,
    /// Width in CSS pixels.
    pub(crate) width: f32,
    /// Height in CSS pixels.
    pub(crate) height: f32,
}

impl PageFragmentRect {
    /// Construct a CSS-px rectangle.
    pub(crate) fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Effective page margin or border/padding inset in CSS pixels.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragmentInsets {
    /// Top inset.
    pub(crate) top: f32,
    /// Right inset.
    pub(crate) right: f32,
    /// Bottom inset.
    pub(crate) bottom: f32,
    /// Left inset.
    pub(crate) left: f32,
}

impl PageFragmentInsets {
    /// Construct four CSS-px insets.
    pub(crate) fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// Physical orientation of a resolved page fragment.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum PageFragmentOrientation {
    /// Page width is less than or equal to page height.
    #[default]
    Portrait,
    /// Page width is greater than page height.
    Landscape,
}

/// Neutral kind classification for one page placement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum PageFragmentKind {
    /// Element or container border-box placement.
    #[default]
    Box,
    /// Text-node placement.
    Text,
    /// Replaced-element placement such as an image.
    Replaced,
}

/// One source-node placement on a page.
///
/// This is the neutral counterpart to fulgur's `Fragment`: `page_index` is
/// zero-based and `rect` is expressed in CSS pixels relative to the containing
/// page's `content_box` origin. To place it in the physical page coordinate
/// system, add `PageFragment::content_box.x/y` exactly once; do not add
/// `content_origin_y`, which is only the shared document-space slice selector.
/// The containing [`PageFragment`] carries the resolved page metadata; the
/// duplicated index keeps node-centric geometry usable without retaining the
/// page-vector wrapper.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragmentItem {
    /// Zero-based page index containing this placement.
    pub(crate) page_index: u32,
    /// Stable source DOM node identifier.
    pub(crate) node_id: NodeId,
    /// Fragment-local border-box rectangle in CSS pixels.
    pub(crate) rect: PageFragmentRect,
    /// Neutral placement classification.
    pub(crate) kind: PageFragmentKind,
    /// Zero-based fragment ordinal for this source node.
    pub(crate) fragment_index: u32,
    /// Total number of placements for this source node in the snapshot.
    pub(crate) fragment_count: u32,
    /// True when this placement repeats the complete source content.
    pub(crate) is_repeat: bool,
    /// Optional line range for a text placement (`start..end`, end exclusive).
    /// Non-text placements leave this as `None`.
    pub(crate) line_range: Option<PageFragmentLineRange>,
}

/// Neutral link metadata attached to a page-local event.
///
/// The raw, trimmed `href` is preserved exactly as a string. URL resolution,
/// fragment lookup, and renderer-specific annotation construction remain
/// consumer responsibilities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct PageFragmentLink {
    /// Source element's `href` attribute after surrounding whitespace is trimmed.
    pub(crate) href: String,
}

impl PageFragmentLink {
    /// Construct link metadata from an `href` value.
    pub(crate) fn new(href: impl Into<String>) -> Self {
        Self { href: href.into() }
    }
}

/// One page-local link event.
///
/// `anchor_node_id` identifies the owning `<a>` element while
/// `placement_node_id` identifies the exact [`PageFragmentItem`] whose
/// geometry is copied into this event. This distinction preserves link
/// identity when an anchor wraps text or replaced descendants.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) struct PageFragmentLinkEvent {
    /// Stable source DOM node identifier of the owning anchor element.
    pub(crate) anchor_node_id: NodeId,
    /// Stable source DOM node identifier of the correlated placement.
    pub(crate) placement_node_id: NodeId,
    /// Zero-based page containing the placement.
    pub(crate) page_index: u32,
    /// Page-local CSS-pixel geometry copied from the placement.
    pub(crate) rect: PageFragmentRect,
    /// Fragment ordinal copied from the placement.
    pub(crate) fragment_index: u32,
    /// Total placements for this source node in the snapshot.
    pub(crate) fragment_count: u32,
    /// Whether the placement is a complete repeated copy.
    pub(crate) is_repeat: bool,
    /// Text line range copied from a text placement, if available.
    pub(crate) line_range: Option<PageFragmentLineRange>,
    /// Neutral link destination.
    pub(crate) link: PageFragmentLink,
}

impl PageFragmentLinkEvent {
    /// Construct a link event from a page placement.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        anchor_node_id: NodeId,
        placement_node_id: NodeId,
        page_index: u32,
        rect: PageFragmentRect,
        fragment_index: u32,
        fragment_count: u32,
        is_repeat: bool,
        line_range: Option<PageFragmentLineRange>,
        link: PageFragmentLink,
    ) -> Self {
        Self {
            anchor_node_id,
            placement_node_id,
            page_index,
            rect,
            fragment_index,
            fragment_count,
            is_repeat,
            line_range,
            link,
        }
    }
}

/// Neutral page-local consumer event vocabulary.
///
/// The link variant is deliberately the only initial event. It is sufficient
/// for a consumer to create its own link annotation while keeping PDF,
/// accessibility, and renderer-specific annotation values out of this crate.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) enum PageFragmentEvent {
    /// A visible source link placement.
    Link(PageFragmentLinkEvent),
}

impl PageFragmentItem {
    /// Construct one page placement.
    pub(crate) fn new(
        node_id: NodeId,
        rect: PageFragmentRect,
        kind: PageFragmentKind,
        fragment_index: u32,
        fragment_count: u32,
        is_repeat: bool,
    ) -> Self {
        Self {
            page_index: 0,
            node_id,
            rect,
            kind,
            fragment_index,
            fragment_count,
            is_repeat,
            line_range: None,
        }
    }

    /// Set the zero-based page containing this placement.
    pub(crate) fn with_page_index(mut self, page_index: u32) -> Self {
        self.page_index = page_index;
        self
    }

    /// Attach a line range to a text placement.
    pub(crate) fn with_line_range(mut self, line_range: PageFragmentLineRange) -> Self {
        self.line_range = Some(line_range);
        self
    }

    /// Whether this node is split across multiple non-repeated placements.
    #[cfg(test)]
    pub(crate) fn is_split(&self) -> bool {
        !self.is_repeat && self.fragment_count > 1
    }
}

/// Node-centric geometry collected from page snapshots.
///
/// This is the neutral counterpart to fulgur's `PaginationGeometry`: the
/// `fragments` vector is ordered by page and fragment index, while
/// `is_repeat` distinguishes complete per-page copies from split content. The
/// `raikiri-dom` producer emits repeat placements for fixed-position subtrees
/// when its existing layout produces the subtree geometry. Table header/footer
/// repetition is not synthesized until pagination owns a corresponding
/// repeated placement; that unsupported semantic remains explicit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
#[cfg(test)]
pub(crate) struct PageFragmentGeometry {
    /// Stable source DOM node identifier.
    pub(crate) node_id: NodeId,
    /// Placements for this node in ascending page order.
    pub(crate) fragments: Vec<PageFragmentItem>,
    /// True when every placement repeats the complete source content.
    pub(crate) is_repeat: bool,
}

#[cfg(test)]
impl PageFragmentGeometry {
    /// Construct empty node-centric geometry.
    pub(crate) fn new(node_id: NodeId, is_repeat: bool) -> Self {
        Self {
            node_id,
            fragments: Vec::new(),
            is_repeat,
        }
    }

    /// Whether this node's placements represent split content.
    #[cfg(test)]
    pub(crate) fn is_split(&self) -> bool {
        !self.is_repeat && self.fragments.len() > 1
    }
}

/// Deterministic NodeId-ordered page geometry table.
///
/// `BTreeMap` iteration yields the same stable source-node order as the
/// pagination producer, independent of DOM traversal implementation details.
#[cfg(test)]
pub(crate) type PageFragmentGeometryTable = BTreeMap<NodeId, PageFragmentGeometry>;

/// Inclusive/exclusive line range carried by a text page placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct PageFragmentLineRange {
    /// First line index, inclusive.
    pub(crate) start: u32,
    /// Last line index, exclusive.
    pub(crate) end: u32,
}

impl PageFragmentLineRange {
    /// Construct a line range.
    pub(crate) fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Whether the range contains no lines.
    #[cfg(test)]
    pub(crate) fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

#[cfg(test)]
mod tests;
