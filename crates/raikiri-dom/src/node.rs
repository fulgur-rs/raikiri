//! Arena node type for raikiri-dom's Document.
//!
//! A flat struct whose union represents three node types: Element, Text, and
//! Document (root). Fields are crate-private (mutated only within raikiri-dom);
//! external consumers access them through the `raikiri_traits::Dom / Node / Element`
//! traits.

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use crate::fragment::MulticolStyle;

use raikiri_style::property::{
    BorderCollapseValue, BreakBetween, CaptionSideValue, DisplayValue, Sides, TableLayoutValue,
    VerticalAlign, WritingMode,
};
use raikiri_style::{ComputedBorder, ComputedBorderSpacing};
use raikiri_traits::{IntrinsicBox, NodeKind, PaintRect};

bitflags::bitflags! {
    /// Per-node boolean attributes. Their **raw bit values exactly match**
    /// blitz `NodeFlags`.
    ///
    /// blitz reference (blitz-dom/src/node/node.rs:50-58):
    /// ```text
    /// const IS_INLINE_ROOT = 0b00000001;   // = 1 << 0
    /// const IS_TABLE_ROOT  = 0b00000010;   // = 1 << 1
    /// const IS_IN_DOCUMENT = 0b00000100;   // = 1 << 2
    /// ```
    ///
    /// Bit 0 (blitz's `IS_INLINE_ROOT`) is unused: paragraphs are marked by
    /// [`IS_IFC_ROOT`](Self::IS_IFC_ROOT) instead. `IS_TABLE_ROOT` keeps
    /// blitz's bit position.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u32 {
        /// Table formatting context root, reserved for future use at the same
        /// bit position as blitz.
        const IS_TABLE_ROOT = 1 << 1;
        /// Whether this node belongs to the flat tree. Clear for nodes
        /// unreachable from the Document root (detached subtrees, including a
        /// `<template>` element's contents fragment) and for Comment /
        /// ProcessingInstruction nodes; set for nodes reachable from the
        /// Document root through flat-tree parents. Ordinary light-DOM
        /// children appended directly under a `<template>` element stay set —
        /// only the associated contents fragment is inert. Future shadow DOM /
        /// slot support will use this bit for "shadow-including tree"
        /// semantics, including direct host children not assigned to a slot
        /// and light-DOM descendants outside the shadow root.
        ///
        /// Maintenance points:
        /// - Parse: a single-pass DFS sets or clears it in the
        ///   `mark_in_document_flags` phase of `raikiri-html::sink::finish`.
        /// - Future mutation runtime: mutator equivalents of
        ///   `process_added_subtree` / `process_removed_subtree` set or unset it.
        const IS_IN_DOCUMENT = 1 << 2;
        /// Outermost inline SVG root, painted as one replaced item.
        const IS_INLINE_SVG_ROOT = 1 << 3;
        /// Node belongs to the source subtree of an inline SVG root.
        const IN_INLINE_SVG_SUBTREE = 1 << 4;
        /// Block root laid out by the shodo inline engine. Its DOM children
        /// are hidden from taffy (see [`Node::layout_children`]): the root is
        /// measured as a leaf from its paragraph.
        const IS_IFC_ROOT = 1 << 5;
        /// Descendant of an [`IS_IFC_ROOT`](Self::IS_IFC_ROOT) node, laid out
        /// as part of that root's paragraph.
        const IN_IFC_SUBTREE = 1 << 6;
        /// Atomic inline removed from its line by a `text-overflow`
        /// ellipsis: it keeps its box but is not painted.
        const HIDDEN_BY_TEXT_OVERFLOW = 1 << 7;
    }
}

/// Element attribute with its parsed namespace and prefix.
#[derive(Debug, Clone)]
pub(crate) struct Attr {
    pub(crate) namespace: Option<SmolStr>,
    pub(crate) prefix: Option<SmolStr>,
    pub(crate) local: SmolStr,
    pub(crate) value: SmolStr,
}

/// NodeData: a tagged union of kind-specific fields.
///
/// Its shape corresponds to blitz `blitz-dom::node::node::NodeData`. Field names
/// match to allow a future nominal blitz-compatible conversion
/// (`match data { NodeData::Element(e) => BlitzElement { ... }, ... }`). As in blitz,
/// only the `Element` variant is boxed: its many fields therefore do not increase
/// the size of the Text and Document variants.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NodeData {
    /// HTML / XML element (tag_name + attributes + namespace + inline_style +
    /// template_contents slot).
    Element(Box<ElementData>),
    /// Character data node.
    Text(TextData),
    /// Document root (the virtual node at arena index 0).
    Document,
    /// HTML / XML comment node (`<!-- ... -->`). Previously stored as an Element
    /// tagged `"#comment"` and stripped in sink.finish(), it now has a permanent
    /// variant. It holds character data but is not an Element
    /// (`kind() == NodeKind::Comment`, `as_element() == None`).
    /// `mark_in_document_flags` explicitly clears its `IS_IN_DOCUMENT` bit, so
    /// cascade, paint, layout, and stylesheet extraction traversals all skip it
    /// through the `is_in_document()` gate. For defense in depth, this avoids
    /// scattering extra `matches!(kind, Comment)` gates across traversals.
    Comment(SmolStr),
    /// Processing instruction node (`<?target data?>`). Holds the target and
    /// data pair. As above, clearing `IS_IN_DOCUMENT` naturally excludes it
    /// from traversals.
    ProcessingInstruction {
        /// PI target (the `xml-stylesheet` part of `<?xml-stylesheet ...?>`).
        target: SmolStr,
        /// PI data (the `href="..."` part of `<?xml-stylesheet href="..."?>`).
        data: SmolStr,
    },
    /// Document fragment root (the virtual root of a detached subtree, such as
    /// `<template>` contents). This permanent variant replaces the former Element
    /// with a `"#document-fragment"` pseudo-tag.
    /// `kind() == NodeKind::DocumentFragment`, `as_element() == None`.
    /// Because it is unreachable from the Document root,
    /// `mark_in_document_flags` naturally clears its `IS_IN_DOCUMENT` bit.
    DocumentFragment,
}

impl NodeData {
    /// Mutably borrow the Element variant within the crate for Document setters.
    #[inline]
    pub(crate) fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeData::Element(e) => Some(e.as_mut()),
            _ => None,
        }
    }
}

/// Element-only data corresponding to blitz `ElementData`.
///
/// `template_contents` holds the arena index of a `<template>` element's
/// contents fragment root. This live slot is populated through
/// [`crate::Document::allocate_template_fragment_root`] when the raikiri-html
/// sink observes `ElementFlags::template=true` in `create_element`.
/// See the field's documentation for details.
#[derive(Debug, Clone)]
pub struct ElementData {
    /// HTML / XML tag name (for example, `"p"` or `"div"`), copied from
    /// html5ever's QualName.local into a SmolStr.
    pub(crate) tag_name: SmolStr,
    /// Raw string from the HTML `style="..."` attribute (populated only for
    /// Elements). The Element trait contract exposes an empty `style=""` as
    /// `None`, but storage here retains the raw value without normalization.
    pub(crate) inline_style: Option<SmolStr>,
    /// Sampled animation declarations, separate from the authored style attribute.
    pub(crate) animation_style: Option<SmolStr>,
    /// Element namespace URI (`Some` only for non-HTML; the HTML default uses
    /// `None` as the optimized path). For example,
    /// `Some("http://www.w3.org/2000/svg")`.
    pub(crate) namespace: Option<SmolStr>,
    /// Element prefix preserved from the parsed qualified name.
    pub(crate) prefix: Option<SmolStr>,
    /// Attributes with their namespace / prefix (source order retained).
    /// The `style` attribute is stored separately in
    /// [`ElementData::inline_style`] and is not included here.
    pub(crate) attributes: Vec<Attr>,
    /// Arena index of a `<template>` element's contents fragment root
    /// (represented by the `NodeData::DocumentFragment` variant).
    ///
    /// When the raikiri-html sink observes html5ever's
    /// `ElementFlags::template = true` in `create_element`, it allocates a
    /// detached [`NodeData::DocumentFragment`] node through
    /// [`crate::Document::allocate_template_fragment_root`] and stores its arena
    /// index here. `TreeSink::get_template_contents` returns this slot, after
    /// which html5ever appends template contents beneath the fragment root
    /// (the template element itself keeps no children).
    ///
    /// This has the same name and shape as blitz
    /// `blitz-dom::node::element::ElementData::template_contents`. It remains
    /// `None` if the sink does not populate it (for example, in tests that
    /// construct nodes directly without template detection, or in future
    /// manual construction).
    pub(crate) template_contents: Option<usize>,
    /// Resolved intrinsic size (px) for a replaced element (`<img>` only, in
    /// this scope), populated by [`crate::image_resolve::resolve_images`]
    /// before the taffy compute pass. `None` means
    /// either this element is not a resolvable replaced element, or
    /// resolution was not attempted — a missing/relative `src`, or an inert
    /// subtree the pre-pass skips. It never means "resolution failed": a
    /// resolver `Err` aborts the pre-pass and fails the render instead of
    /// leaving a size behind here (no fallback size in this scope — see
    /// `ReplacedResolver` doc for why `Err` is terminal rather than silently
    /// substituting a size).
    pub(crate) image_intrinsic_size: Option<IntrinsicBox>,
    /// Materialized bitmap for an HTML `<canvas>` element, populated through
    /// [`crate::Document::set_canvas_bitmap`] and read by paint. `None` means
    /// the canvas is implicitly transparent because it has not been drawn or
    /// its resource limits prevent materialization. Stored here rather than
    /// as an attribute so `getAttribute` never observes it.
    pub(crate) canvas_bitmap: Option<CanvasBitmap>,
}

/// Live bitmap of an HTML `<canvas>` element (HTML Standard §4.12.5).
///
/// When materialized, `rgba` holds premultiplied-free `width * height * 4`
/// bytes in row-major order. An empty buffer represents an unmaterialized
/// transparent bitmap. Paint reads materialized pixels directly; layout reads
/// only the width/height attributes for the intrinsic size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasBitmap {
    /// Bitmap width in px (the canvas width attribute value).
    pub width: u32,
    /// Bitmap height in px (the canvas height attribute value).
    pub height: u32,
    /// Row-major RGBA8 bytes. Empty storage represents transparent black
    /// without pixel storage, either for a zero-area canvas or because of
    /// resource limits.
    pub rgba: Vec<u8>,
}

const MAX_CANVAS_BITMAP_PIXELS: u64 = 10_000_000;
const MAX_CANVAS_BITMAP_BYTES: usize = 32 * 1024 * 1024;

/// Reason a canvas bitmap could not be materialized or stored safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasBitmapError {
    /// The dimensions exceed the per-canvas pixel or byte limit.
    DimensionsTooLarge,
    /// The allocator could not reserve the backing RGBA8 buffer.
    AllocationFailed,
    /// Storing the bitmap would exceed the document-wide byte limit.
    DocumentLimitExceeded,
    /// A materialized RGBA8 buffer does not match the declared dimensions.
    InvalidRgbaLength,
    /// The bitmap dimensions differ from the canvas element's current size.
    SizeMismatch,
    /// The requested node is not an HTML canvas element.
    NotCanvas,
}

impl std::fmt::Display for CanvasBitmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DimensionsTooLarge => "canvas bitmap dimensions exceed the per-canvas limit",
            Self::AllocationFailed => "canvas bitmap allocation failed",
            Self::DocumentLimitExceeded => "canvas bitmap exceeds the document memory limit",
            Self::InvalidRgbaLength => "canvas bitmap has an invalid RGBA buffer length",
            Self::SizeMismatch => "canvas bitmap dimensions do not match the canvas element",
            Self::NotCanvas => "node is not a canvas element",
        })
    }
}

impl std::error::Error for CanvasBitmapError {}

impl CanvasBitmap {
    /// A transparent-black bitmap of the given size, when within resource
    /// limits. Otherwise, returns an unmaterialized transparent bitmap.
    /// Use [`CanvasBitmap::try_cleared`] when allocation failure must be
    /// distinguished from success.
    pub fn cleared(width: u32, height: u32) -> Self {
        Self::try_cleared(width, height).unwrap_or_else(|_| Self::transparent(width, height))
    }

    /// Try to materialize transparent RGBA8 pixels for a bounded canvas.
    pub fn try_cleared(width: u32, height: u32) -> Result<Self, CanvasBitmapError> {
        let len = Self::checked_rgba_len(width, height)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(len)
            .map_err(|_| CanvasBitmapError::AllocationFailed)?;
        rgba.resize(len, 0);
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Represent transparent black without allocating a pixel buffer.
    pub fn transparent(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: Vec::new(),
        }
    }

    pub(crate) fn checked_rgba_len(width: u32, height: u32) -> Result<usize, CanvasBitmapError> {
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(CanvasBitmapError::DimensionsTooLarge)?;
        if pixels > MAX_CANVAS_BITMAP_PIXELS {
            return Err(CanvasBitmapError::DimensionsTooLarge);
        }
        let bytes = pixels
            .checked_mul(4)
            .ok_or(CanvasBitmapError::DimensionsTooLarge)?;
        let len = usize::try_from(bytes).map_err(|_| CanvasBitmapError::DimensionsTooLarge)?;
        if len > MAX_CANVAS_BITMAP_BYTES {
            return Err(CanvasBitmapError::DimensionsTooLarge);
        }
        Ok(len)
    }
}

/// One line-range fragment painted into a multicolumn column.
///
/// The text layout remains a single shaped object; layout records the line
/// ranges and physical column offsets here so paint can emit each fragment at
/// its own fragmentainer origin without changing DOM node identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MulticolTextFragment {
    /// Inclusive first line in the shaped layout.
    pub line_start: usize,
    /// Exclusive last line in the shaped layout.
    pub line_end: usize,
    /// Fragmentainer/column index that owns this line range.
    pub fragmentainer: usize,
    /// Horizontal offset from the text node's normal origin.
    pub x: f32,
    /// Vertical offset from the text node's normal origin.
    pub y: f32,
}

/// Text-only data corresponding nominally to blitz `TextNodeData`.
#[derive(Debug, Clone)]
pub struct TextData {
    /// Character data.
    pub(crate) text_content: SmolStr,
}

/// Arena node implemented with a NodeData tagged union.
///
/// Kind-independent fields needed by paint, cascade, and layout (children and
/// unrounded_layout) remain on Node; kind-specific fields reside in [`NodeData`]
/// variants. `kind` and `tag_name` are read through accessors
/// (`node.kind()`, `node.tag_name()`); `children` and `unrounded_layout`
/// remain public.
#[derive(Debug, Clone)]
pub struct Node {
    /// Taffy layout style.
    pub(crate) style: Style,
    /// Computed `display` for table dispatch. Bridged to `style.display` for Block/Flex/Grid/None
    /// but retains the full `DisplayValue` (including `Table` family) so table engine routing
    /// does not need post-bridge `style.display` which collapses table variants to `Block`.
    pub(crate) display: DisplayValue,
    /// Computed `table-layout` for the table engine. No `taffy::Style`
    /// counterpart exists (taffy 0.12 has no table layout), so the keyword
    /// is carried here alongside [`Self::display`] and read by
    /// [`crate::layout::table`] (`Node::display` bridge precedent).
    pub(crate) table_layout: TableLayoutValue,
    /// Computed `border-collapse` for the table engine. Same `Node`-side
    /// carrying shape as [`Self::table_layout`] above (`border-collapse`
    /// is inherited — the value here is already the post-inheritance
    /// computed value, seed handling lives in raikiri-style).
    pub(crate) border_collapse: BorderCollapseValue,
    /// Computed border sides retained for collapsed-border conflict resolution.
    pub(crate) computed_border: Option<Sides<ComputedBorder>>,
    /// Border sides a collapsed-border table cell paints after conflict
    /// resolution (CSS 2.1 §17.6.2.1); `None` outside the collapsing model.
    pub(crate) collapsed_border: Option<Sides<ComputedBorder>>,
    /// Computed `border-spacing` used by the separate-border table layout.
    pub(crate) border_spacing: ComputedBorderSpacing,
    /// Computed alignment used by the native table cell placement pass.
    pub(crate) table_vertical_align: VerticalAlign,
    /// Computed caption position consumed by the native table layout.
    pub(crate) caption_side: CaptionSideValue,
    /// The table grid's border box within its wrapper, excluding captions.
    pub(crate) table_grid_box: Option<PaintRect>,
    /// First-row baseline relative to the table wrapper's border edge.
    pub(crate) table_first_baseline: Option<f32>,
    /// Computed `break-before` value consumed by column fragmentation.
    pub(crate) break_before: BreakBetween,
    /// Computed `break-after` value consumed by column fragmentation.
    pub(crate) break_after: BreakBetween,
    /// Computed multicolumn settings consumed by the custom Taffy dispatch.
    pub(crate) multicol: Option<MulticolStyle>,
    /// Authored writing mode retained for layout features that need the logical axes.
    pub(crate) authored_writing_mode: Option<WritingMode>,
    /// Whether this node has an authored logical `min-block-size` constraint
    /// paired with `break-inside: avoid`. The used value is bridged
    /// to Taffy's physical min-size fields, while fragmentation needs the
    /// provenance to avoid changing physical `min-height` behavior.
    pub(crate) has_logical_min_block_size: bool,
    /// Whether manual multicol placement replaced the relative block position.
    pub(crate) needs_relative_block_paint_offset: bool,
    /// Computed CSS `order`; consumed only when this node is a flex/grid item.
    pub(crate) order: i32,
    /// Optional order-modified child view for the last style bridge. This is
    /// derived layout state, never a replacement for the DOM-order `children`.
    pub(crate) order_modified_children: Box<[usize]>,
    /// Taffy's resolved grid row start for each in-flow child, in source-view order.
    pub(crate) grid_item_row_starts: Box<[(usize, u16)]>,
    /// Resolved grid column count from Taffy's detailed layout information.
    pub(crate) grid_column_count: usize,
    /// Child arena indices (`usize` indices into `Document::nodes`).
    pub children: Vec<usize>,
    /// Arena index of the parent holding this node in its `children`, or
    /// `None` for the Document root and detached nodes.
    ///
    /// This is the O(1) backing store for [`crate::Document::parent_of`].
    /// Every tree mutation primitive in `document.rs` / `document/mutation.rs`
    /// (`append_*` / `attach_child` / `insert_child_before` /
    /// `detach_from_parent` / `reparent_children` / `retain_children` /
    /// `set_element_text_content` / `replace_children_from` /
    /// `from_logical_snapshot`) keeps it in sync with the parent's
    /// `children` list. It tracks only `children` edges, not a `<template>`
    /// element's `template_contents` slot (that host link is not a parent).
    ///
    /// Direct `children` pushes outside those primitives bypass this field.
    /// In-crate test setups that do so must also set `parent` (or use the
    /// primitives) to keep the invariant
    /// `parent_of(child) == Some(p)` iff `nodes[p].children.contains(&child)`.
    pub(crate) parent: Option<usize>,
    /// Per-node Taffy layout cache.
    pub(crate) cache: Cache,
    /// Shodo paragraph state; set only on nodes flagged [`NodeFlags::IS_IFC_ROOT`].
    pub(crate) ifc: Option<Box<crate::layout::ifc::root::IfcRoot>>,
    /// Taffy layout result, populated by compute_root_layout.
    ///
    /// # Value range contract: all `f32` fields are finite and clamped to `[-1e7, 1e7]`
    ///
    /// `location.{x,y}` / `size.{width,height}` / `content_size.{width,height}`
    /// / `scrollbar_size.{width,height}` / `border.{left,right,top,bottom}` /
    /// `padding.{left,right,top,bottom}` / `margin.{left,right,top,bottom}` are
    /// always **finite** and symmetrically clamped to
    /// `[-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE]` (`[-1e7, 1e7]`, where
    /// `MAX_TAFFY_MAGNITUDE` is private to the `raikiri_dom::layout` module).
    /// Values that became `NaN` during computation have already fallen back to
    /// `0.0`. Raw taffy output containing `±Inf` is never written to this field.
    /// `order` (`u32`) is outside this contract because it cannot be non-finite.
    ///
    /// Only finiteness and the magnitude limit are guaranteed. Box model
    /// containment (CSS Box 3's content ⊆ padding ⊆ border) is not preserved:
    /// fields are clamped independently, so simultaneous saturation of
    /// `size.width` and `padding.{left,right}` can make
    /// `size.width - padding.left - padding.right` negative.
    ///
    /// [`crate::layout::sanitize_taffy_layout`] enforces this contract at
    /// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
    /// (`taffy_impl.rs`), the sole taffy-to-arena write point. This structural
    /// invariant cannot be broken by forgetting a separate call (see the docs
    /// for [`crate::layout::sanitize_taffy_layout`] for details and evidence).
    /// Although this field is `pub`, no public API yields an external `&mut Node`:
    /// `Document::nodes` and constructors such as [`Node::new_document`] are
    /// `pub(crate)`, and [`crate::Document::get_node`] returns only `&Node`.
    /// External callers therefore cannot write it without passing through this
    /// choke point.
    ///
    /// This `pub` field is directly observable outside the crate through
    /// [`crate::Document::get_node`]. Both the raikiri-paint walker
    /// (`crates/raikiri-paint/src/walk.rs`, which directly accumulates
    /// `location.x`/`y`) and raikiri's page-scene extraction
    /// (`crates/raikiri/src/page_scene.rs`, which copies `location.{x,y}` and
    /// `size.{width,height}` directly into `Fragment`) read it without further
    /// validation. This contract is the sole protection for consumers against
    /// non-finite geometry at the dom-to-paint boundary. Type, field shape, and
    /// access pattern are unchanged; this documentation states only the
    /// behavioral contract for the **set of observable values** at that boundary.
    pub unrounded_layout: Layout,
    /// Per-node metadata bits (IS_IN_DOCUMENT, etc.); mutation is crate-private.
    pub(crate) flags: NodeFlags,
    /// Node kind and kind-specific fields (tagged union).
    pub(crate) data: NodeData,
}

impl Node {
    /// Construct the Document root node for arena index 0.
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Document,
        }
    }

    /// Construct an Element node with its tag name, style, and inline_style.
    /// `namespace`, `attributes`, and `template_contents` start empty or None.
    /// At finish, the raikiri-html sink populates the first two through
    /// [`crate::Document::set_element_namespace`] and
    /// [`crate::Document::set_element_attributes`]. For `<template>` elements
    /// only, the sink eagerly populates `template_contents` in `create_element`
    /// through [`crate::Document::allocate_template_fragment_root`].
    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Element(Box::new(ElementData {
                tag_name: tag,
                inline_style,
                animation_style: None,
                namespace: None,
                prefix: None,
                attributes: Vec::new(),
                template_contents: None,
                image_intrinsic_size: None,
                canvas_bitmap: None,
            })),
        }
    }

    /// Construct a Text node with its character data.
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Text(TextData { text_content: text }),
        }
    }

    /// Construct a Comment node with its character data.
    /// Its `IS_IN_DOCUMENT` bit starts true, but
    /// [`crate::Document::mark_in_document_flags`] always clears it in the step 2
    /// DFS. For blitz compatibility, comments are invisible in the flat tree;
    /// layout, paint, and cascade automatically skip them through the
    /// `is_in_document()` gate.
    pub(crate) fn new_comment(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Comment(text),
        }
    }

    /// Construct a Processing instruction node with its target and data.
    /// As with Comment nodes, [`crate::Document::mark_in_document_flags`]
    /// clears its `IS_IN_DOCUMENT` bit.
    pub(crate) fn new_processing_instruction(target: SmolStr, data: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::ProcessingInstruction { target, data },
        }
    }

    /// Construct a Document fragment root. It is typically placed in the arena
    /// detached (without a parent) and used as the virtual root of `<template>`
    /// contents. Its `IS_IN_DOCUMENT` bit starts true, but
    /// `mark_in_document_flags` clears it because it is unreachable from the
    /// Document root.
    pub(crate) fn new_document_fragment() -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            table_vertical_align: VerticalAlign::Baseline,
            caption_side: CaptionSideValue::Top,
            table_grid_box: None,
            table_first_baseline: None,
            border_collapse: BorderCollapseValue::Separate,
            computed_border: None,
            collapsed_border: None,
            border_spacing: ComputedBorderSpacing {
                horizontal: raikiri_style::ComputedLength(0.0),
                vertical: raikiri_style::ComputedLength(0.0),
            },
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            multicol: None,
            authored_writing_mode: None,
            has_logical_min_block_size: false,
            needs_relative_block_paint_offset: false,
            order: 0,
            order_modified_children: Box::new([]),
            grid_item_row_starts: Box::new([]),
            grid_column_count: 0,
            children: Vec::new(),
            parent: None,
            cache: Cache::new(),
            ifc: None,
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::DocumentFragment,
        }
    }

    // ─── inherent accessor methods ─────────────────

    /// Return this Node's [`NodeKind`].
    ///
    /// Accessor replacement for the former `pub kind: NodeKind` field.
    /// Callers only need to change `node.kind` to `node.kind()`.
    #[inline]
    pub fn kind(&self) -> NodeKind {
        match &self.data {
            NodeData::Element(_) => NodeKind::Element,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Document => NodeKind::Document,
            NodeData::Comment(_) => NodeKind::Comment,
            NodeData::ProcessingInstruction { .. } => NodeKind::ProcessingInstruction,
            NodeData::DocumentFragment => NodeKind::DocumentFragment,
        }
    }

    /// Whether this block is laid out by the shodo inline engine.
    #[doc(hidden)]
    #[inline]
    pub fn is_ifc_root(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IFC_ROOT)
    }

    /// Whether this node is inside a paragraph laid out by the shodo inline
    /// engine (a descendant of an ifc root that is not one of its boxes).
    #[doc(hidden)]
    #[inline]
    pub fn in_ifc_subtree(&self) -> bool {
        self.flags.contains(NodeFlags::IN_IFC_SUBTREE)
    }

    /// The border sides this table cell paints after collapsed-border
    /// conflict resolution, when its table uses `border-collapse: collapse`.
    #[doc(hidden)]
    pub fn collapsed_border(&self) -> Option<&Sides<ComputedBorder>> {
        self.collapsed_border.as_ref()
    }

    /// Lines of the last performed layout of an ifc root, if any.
    #[doc(hidden)]
    pub fn ifc_lines(&self) -> Option<&[shodo::Line]> {
        self.ifc
            .as_ref()?
            .lines
            .as_ref()
            .map(|lines| lines.lines.as_slice())
    }

    /// Pieces of the inline elements of an ifc root on its lines, in the
    /// root's content box; `None` for a node without lines.
    #[doc(hidden)]
    pub fn ifc_inline_boxes(&self) -> Option<Vec<crate::layout::InlineBoxPiece>> {
        let root = self.ifc.as_ref()?;
        let lines = root.lines.as_ref()?;
        let mut content_size = self.ifc_physical_content_size()?;
        // Keep the existing line-local container for horizontal columns.
        if root.writing_mode == shodo::geometry::WritingMode::HorizontalTb {
            content_size.width = lines.width;
        }
        let mut pieces = crate::layout::ifc::inline_boxes::inline_box_pieces(
            &lines.lines,
            root.writing_mode,
            content_size,
        );
        // Lines moved by pagination carry their pieces with them.
        for piece in &mut pieces {
            let shift = lines.shift_of(piece.line);
            piece.border_box.y += shift;
            piece.content_box.y += shift;
        }
        Some(pieces)
    }

    /// Writing mode used by this root's last performed IFC layout.
    #[doc(hidden)]
    pub fn ifc_writing_mode(&self) -> Option<shodo::geometry::WritingMode> {
        let root = self.ifc.as_ref()?;
        root.lines.as_ref()?;
        Some(root.writing_mode)
    }

    /// Physical content-box size of this root's last performed IFC layout.
    #[doc(hidden)]
    pub fn ifc_physical_content_size(&self) -> Option<shodo::geometry::PhysicalSize> {
        self.ifc.as_ref()?.lines.as_ref()?;
        let layout = self.unrounded_layout;
        Some(shodo::geometry::PhysicalSize {
            width: (layout.size.width
                - layout.border.left
                - layout.border.right
                - layout.padding.left
                - layout.padding.right)
                .max(0.0),
            height: (layout.size.height
                - layout.border.top
                - layout.border.bottom
                - layout.padding.top
                - layout.padding.bottom)
                .max(0.0),
        })
    }

    /// Paint offsets of the relatively positioned inline elements of an ifc
    /// root, by DOM node id; empty for any other node.
    #[doc(hidden)]
    pub fn ifc_relative_offsets(&self) -> &[(usize, (f32, f32))] {
        self.ifc
            .as_ref()
            .map_or(&[], |root| root.offsets.as_slice())
    }

    /// Line ranges of an ifc root's lines split across the columns of a
    /// multicol container, with the offset each range is drawn at: `x` from
    /// the content-box start, `y` the block offset its first line is drawn
    /// at. `None` when the lines are not split.
    #[doc(hidden)]
    pub fn ifc_multicol_fragments(&self) -> Option<&[MulticolTextFragment]> {
        self.ifc.as_ref()?.multicol_fragments.as_deref()
    }

    /// Content width the lines of an ifc root were broken at, and their total
    /// height.
    #[doc(hidden)]
    pub fn ifc_size(&self) -> Option<(f32, f32)> {
        let lines = self.ifc.as_ref()?.lines.as_ref()?;
        let moved: f32 = lines.shifts.iter().map(|(_, delta)| delta).sum();
        Some((lines.width, lines.height + moved))
    }

    /// How far pagination moved each line of an ifc root down, by line
    /// index: the lines after a block child that was moved to a later page.
    #[doc(hidden)]
    pub fn ifc_line_shifts(&self) -> Vec<f32> {
        self.ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .map(|lines| {
                (0..lines.lines.len())
                    .map(|index| lines.shift_of(index))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Children of an ifc root that it lays out as boxes of their own
    /// (floats and atomic inlines), in document order. Empty for any other
    /// node.
    #[doc(hidden)]
    pub fn ifc_boxes(&self) -> Vec<usize> {
        self.ifc
            .as_ref()
            .map(|root| root.boxes.iter().map(|b| b.node).collect())
            .unwrap_or_default()
    }

    /// Whether manual column placement needs the relative block inset at paint time.
    #[doc(hidden)]
    pub fn needs_relative_block_paint_offset(&self) -> bool {
        self.needs_relative_block_paint_offset
    }

    /// Children in this node's layout and paint order.
    ///
    /// Flex and grid containers with non-zero item `order` use a derived stable
    /// projection. Other nodes return their DOM-order child slice unchanged.
    #[doc(hidden)]
    #[inline]
    pub fn layout_children(&self) -> &[usize] {
        if self.flags.contains(NodeFlags::IS_INLINE_SVG_ROOT)
            || self.flags.contains(NodeFlags::IS_IFC_ROOT)
        {
            return &[];
        }
        if self.order_modified_children.is_empty() {
            &self.children
        } else {
            &self.order_modified_children
        }
    }

    /// The used table grid border box relative to its wrapper origin.
    /// Captions occupy wrapper space outside this box.
    pub fn table_grid_box(&self) -> Option<PaintRect> {
        self.table_grid_box
    }

    /// Return tag_name for an Element, or `None` otherwise.
    ///
    /// Accessor replacement for the former `pub tag_name: Option<SmolStr>`
    /// field. It projects to `Option<&str>`, an internal SmolStr view with
    /// roughly the cost of a Copy.
    #[inline]
    pub fn tag_name(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(e) => Some(e.tag_name.as_str()),
            _ => None,
        }
    }

    /// Element namespace URI, or `None` for non-elements and the HTML namespace.
    #[inline]
    pub fn namespace_uri(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(element) => element.namespace.as_deref(),
            _ => None,
        }
    }

    /// Return a null-namespace element attribute value, if present.
    ///
    /// The `style` attribute is stored separately from the ordinary attribute
    /// vector and is exposed here as the same raw value for consumers that need
    /// a small DOM-side resource hint (for example a replaced image `src`).
    /// Namespaced attributes (for example SVG `xlink:href`) are not visible
    /// here; use [`Node::attribute_ns`] for those.
    #[inline]
    pub fn attribute(&self, local: &str) -> Option<&str> {
        let NodeData::Element(element) = &self.data else {
            return None;
        };
        if local == "style" {
            return element.inline_style.as_deref();
        }
        element
            .attributes
            .iter()
            .find(|attribute| attribute.namespace.is_none() && attribute.local == local)
            .map(|attribute| attribute.value.as_str())
    }

    /// Return a namespace-qualified element attribute value, if present.
    ///
    /// Lookup is by namespace URI and local name (DOM `getAttributeNS`
    /// semantics), not by prefix. For example SVG `xlink:href` is
    /// `namespace = "http://www.w3.org/1999/xlink"`, `local = "href"`.
    /// Matching is exact; unlike HTML null-namespace lookup, no ASCII
    /// case folding applies. The separate `style` slot is null-namespace
    /// only, so this always returns `None` for namespaced `style` lookups.
    /// Returns `None` for non-elements and when no such attribute exists.
    /// Renderer-neutral: the value is a plain string slice with no SVG or
    /// renderer types. See [`Node::attribute`] for null-namespace lookup and
    /// [`crate::Document::serialize_svg_subtree`] for whole-subtree XML
    /// source reconstruction.
    #[inline]
    pub fn attribute_ns(&self, namespace: &str, local: &str) -> Option<&str> {
        let NodeData::Element(element) = &self.data else {
            return None;
        };
        element
            .attributes
            .iter()
            .find(|attribute| {
                attribute.namespace.as_deref() == Some(namespace) && attribute.local == local
            })
            .map(|attribute| attribute.value.as_str())
    }

    /// Whether this node belongs to an inline SVG source subtree.
    #[inline]
    pub fn is_inline_svg_content(&self) -> bool {
        self.flags.contains(NodeFlags::IN_INLINE_SVG_SUBTREE)
    }

    /// Whether this is the outermost inline SVG root.
    #[inline]
    pub fn is_inline_svg_root(&self) -> bool {
        self.flags.contains(NodeFlags::IS_INLINE_SVG_ROOT)
    }

    /// Return the raw character data for a text node.
    ///
    /// Paint-side generated-content resolution uses this accessor while
    /// walking an element's descendant string value.
    #[inline]
    pub fn text_content(&self) -> Option<&str> {
        match &self.data {
            NodeData::Text(text) => Some(text.text_content.as_str()),
            _ => None,
        }
    }

    /// Resolved intrinsic size (px) for a replaced element, if
    /// [`crate::image_resolve::resolve_images`] has populated one for this
    /// node. `None` for non-`<img>` elements, unresolved images, and all
    /// non-element node kinds.
    pub(crate) fn image_intrinsic_size(&self) -> Option<(f32, f32)> {
        match &self.data {
            NodeData::Element(e) => e.image_intrinsic_size.map(|size| (size.width, size.height)),
            _ => None,
        }
    }

    /// Full intrinsic box, including an optional natural aspect ratio.
    pub(crate) fn image_intrinsic_box(&self) -> Option<IntrinsicBox> {
        match &self.data {
            NodeData::Element(element) => element.image_intrinsic_size,
            _ => None,
        }
    }

    /// Return whether this node has `taffy::Style.display == Display::None`.
    ///
    /// Paint uses this predicate to skip display:none subtrees. Treating size 0
    /// as a proxy would silently drop legitimate zero-size elements with
    /// overflow: visible. The style field remains crate-private; only the
    /// minimal boolean predicate needed by paint is public (gradual exposure).
    #[inline]
    pub fn is_display_none(&self) -> bool {
        self.style.display == taffy::Display::None
    }

    /// Return whether a `text-overflow` ellipsis removed this atomic inline
    /// from its line (CSS Overflow 3 §5.1). Paint skips its subtree, as for
    /// [`Self::is_display_none`].
    #[inline]
    pub fn is_hidden_by_text_overflow(&self) -> bool {
        self.flags.contains(NodeFlags::HIDDEN_BY_TEXT_OVERFLOW)
    }

    /// Return whether this Node belongs to the flat tree.
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// Arena index of a `<template>` element's contents fragment root.
    /// Returns `None` for non-Elements or if the fragment root was not wired.
    ///
    /// The html5ever `TreeSink::get_template_contents` implementation consumes
    /// this through the sink.
    /// This read-side accessor corresponds to the blitz
    /// `blitz-dom::node::element::ElementData::template_contents` field.
    #[inline]
    pub fn template_contents(&self) -> Option<usize> {
        match &self.data {
            NodeData::Element(e) => e.template_contents,
            _ => None,
        }
    }

    /// Identify "non-rendered" elements in the HTML namespace (metadata
    /// content, raw text containers, and ruby parenthesis fallbacks). Paint
    /// uses this gate to skip their entire subtrees.
    ///
    /// Covered elements (HTML LS §15.3.1 "Hidden elements",
    /// <https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>):
    /// `<head>`, `<title>`, `<meta>`, `<link>`, `<base>`, `<noscript>`,
    /// `<script>`, `<style>`, `<template>`,
    /// `<datalist>` (§4.10.8,
    /// <https://html.spec.whatwg.org/multipage/form-elements.html#the-datalist-element>),
    /// `<noembed>` / `<noframes>` (§13.2 RAWTEXT parsing), and `<rp>` (§4.5.12
    /// ruby parenthesis fallback, directly covered by `display: none` in the
    /// §15.3.1 hidden-elements rule). The ruby CSS in §15.3.4 "Phrasing content"
    /// also defines only `ruby { display: ruby }` and
    /// `rt { display: ruby-text }`; it does not make rp visible, even in a UA
    /// that supports ruby. Returns true only for the default HTML namespace
    /// (`Node.namespace == None`) or explicit XHTML namespace
    /// (`"http://www.w3.org/1999/xhtml"`). Same-named elements in SVG or MathML
    /// return false: rendering SVG `<style>` and `<script>` is the SVG renderer's
    /// responsibility, outside the HTML paint filter.
    ///
    /// The §15.3.1 hidden-elements rule also lists `<area>`, `<basefont>`, and
    /// `<param>`. They ordinarily have no child content (`<area>` is void,
    /// `<basefont>` is obsolete-void, and `<param>` acts like an attribute inside
    /// an object), so they offer no content leak path and are omitted here.
    ///
    /// # Motivation
    ///
    /// Author or user CSS can override hiding by UA CSS
    /// (`style { display: none }`, etc.; HTML LS §15.3.1). Attacker-controlled
    /// HTML plus overriding CSS could therefore expose text from `<style>` or
    /// `<script>` in rendered output. A cascade-independent paint gate provides
    /// defense in depth (datalist, noembed, noframes, and rp were added later
    /// to complete §15.3.1 coverage).
    ///
    /// # Non-goals
    ///
    /// - Filtering the `[hidden]` and `inert` attributes is outside this
    ///   predicate's scope (planned for the cascade path behind
    ///   `is_display_none()`).
    /// - The `is_in_document() == false` gate already skips a `<template>`
    ///   element's detached contents, but this predicate includes the element
    ///   itself for defense in depth (both gates fail closed independently).
    #[inline]
    pub fn is_non_rendered_html_element(&self) -> bool {
        let NodeData::Element(e) = &self.data else {
            return false;
        };
        // Namespace check: only the HTML default (None) or explicit XHTML applies.
        // Same-named SVG / MathML elements use their own rendering paths.
        match e.namespace.as_deref() {
            None => {}
            Some("http://www.w3.org/1999/xhtml") => {}
            _ => return false,
        }
        // html5ever has already lowercased HTML tag names (QualName.local).
        // Ordering: keep the original arms first and group the four arms added
        // for §15.3.1 coverage last, preserving the existing style while making
        // the origin of the additions clear.
        matches!(
            e.tag_name.as_str(),
            "head"
                | "title"
                | "meta"
                | "link"
                | "base"
                | "noscript"
                | "script"
                | "style"
                | "template"
                // Added to complete §15.3.1 coverage:
                | "datalist"
                | "noembed"
                | "noframes"
                | "rp"
        )
    }

    /// Explicitly overwrite the [`NodeFlags::IS_IN_DOCUMENT`] bit (crate-private).
    ///
    /// Used by `Document::mark_in_document_flags`, the single-pass DFS called
    /// from sink.finish().
    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }

    pub(crate) fn set_inline_svg_content(&mut self, v: bool) {
        self.flags.set(NodeFlags::IN_INLINE_SVG_SUBTREE, v);
    }

    pub(crate) fn set_inline_svg_root(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_INLINE_SVG_ROOT, v);
    }
}

#[cfg(test)]
mod tests;
