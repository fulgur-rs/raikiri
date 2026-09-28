//! Entry types stored in [`PageDrawables`](crate::PageDrawables)
//! per-attribute maps (elements of a struct of arrays).
//!
//! # Landing history
//!
//! Initially, only public types landed, with every struct empty (`{}`).
//! Minimal fields were added later using Fulgur as a shape reference. The
//! proposed rewrite of `raikiri_paint::paint_single_page` was separated out.
//!
//! [`build_page_scene`](crate::page_scene::build_page_scene) currently
//! constructs and inserts only two types into PageDrawables: [`BlockEntry`]
//! for post-layout Element nodes and [`ParagraphEntry`] for post-layout Text
//! nodes. The other nine have reference-shaped fields, but no corresponding
//! raikiri pipeline stage (image decoding, table/list-marker layout, CSS
//! transforms, multicolumn, PDF bookmarks/tags, or link spans). They remain
//! unconstructed, not merely populated with defaults. Each struct documents
//! its specific reason.
//!
//! # Field-type policy
//!
//! Field types avoid **direct raikiri-style, taffy, or parley types**, using
//! only `f32`, `bool`, `u8` tuples, `Option<String>`, [`NodeId`], and
//! `Vec<NodeId>`. This avoids redoing the work if a future crate-topology
//! decision moves PageDrawables/entries to raikiri-traits. Raikiri-traits
//! does not depend on raikiri-style (a dependency-graph constraint distinct
//! from the principle-5 independent-implementation boundary), so moving
//! entries should not force it to import these types. Convert ComputedValues
//! to primitives when populating entries.
//!
//! `raikiri_style::ComputedValues` does not yet include CSS `opacity`,
//! `visibility`, or `overflow`. Their fields therefore use CSS initial
//! values (`opacity: 1`, `visibility: visible`, zero clipped descendants).
//! These are not invented placeholder values: without those cascade
//! properties, the initial values always apply. Switch to real lookups when
//! the cascade gains the properties.
//!
//! Numeric length fields reuse [`crate::page_scene::Pt`] (an `f32` alias).
//! Despite the name "Pt" (PDF points),
//! [`crate::page_scene::build_page_scene`] currently fills it with CSS px;
//! see the existing unit mismatch in the [`crate::page_scene`] module docs.
//! Entries intentionally follow that convention rather than introduce a
//! second, different mismatch.
//!
//! Every struct is `#[non_exhaustive]`, so adding fields is semver-compatible:
//! consumers cannot construct `BlockEntry { .. }` literals whose compilation
//! would break when fields are added.

use crate::page_scene::Pt;
use raikiri_traits::NodeId;

/// Per-node paint state for a block box (background, border, opacity,
/// anchor ID, etc.).
///
/// The Fulgur `BlockEntry` shape (drawables.rs:142-177) has `style`,
/// `opacity`, `visible`, `id`, `layout_size`, `clip_descendants`, and
/// `opacity_descendants`. Only fields actually available in raikiri were
/// selected.
///
/// `build_page_scene` (crate::page_scene) constructs this for each post-layout
/// Element and inserts it into `PageDrawables::block_styles`. This is one of
/// the two populated types; the other is [`ParagraphEntry`].
///
/// # Why `style` is not a bundled type
///
/// Fulgur's `style: BlockStyle` bundles background, border, and box shadow
/// in a dedicated paint-primitive type. Raikiri has no such bundle. Adding
/// one would expand the umbrella's public surface before the item-4 topology
/// decision is settled (see the module field-type policy). Background color
/// and border widths are therefore flattened into this struct.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct BlockEntry {
    /// `background-color` as (r, g, b, a), converted from
    /// `raikiri_style::CssColor` to a primitive tuple rather than keeping
    /// the type directly (see the module field-type policy).
    pub background_color: (u8, u8, u8, u8),
    /// Computed `border-{top,right,bottom,left}-width` in px, in CSS
    /// shorthand order. The CSS computed layer forces width to zero when
    /// `border-style: none` (CSS Backgrounds 3 §3.3), so `0.0` also correctly
    /// describes an effectively absent border. Border style (`solid`, etc.)
    /// and color are omitted because they cannot yet be safely flattened to
    /// primitives; reconsider when paint consumes these fields.
    pub border_widths: (Pt, Pt, Pt, Pt),
    /// `opacity`: always the CSS initial value `1.0` because the property
    /// is absent from the cascade (see module docs).
    pub opacity: f32,
    /// `visibility`: always `true` (= visible) for the same reason.
    pub visible: bool,
    /// The `id` attribute via [`raikiri_traits::Element::id`]. An empty ID
    /// returns `None`, as required by the trait contract.
    pub id: Option<String>,
    /// Taffy's computed border-box size in CSS px (see module unit note).
    /// Post-layout Elements always have this size in practice. The `Option`
    /// merely follows Fulgur's "fallback to fragment" shape.
    pub layout_size: Option<(Pt, Pt)>,
    /// Descendant [`NodeId`] values in an overflow-clip scope. Always
    /// empty because cascade has no `overflow` property and hence no
    /// elements to clip; this represents zero applicable elements, not
    /// a missing implementation disguised as an empty list.
    pub clip_descendants: Vec<NodeId>,
    /// Descendant [`NodeId`] values in an opacity-group scope; also always empty.
    pub opacity_descendants: Vec<NodeId>,
}

impl Default for BlockEntry {
    fn default() -> Self {
        // Derived Default would give opacity: 0.0 (fully transparent) and
        // visible: false, the opposite of the CSS initial values (1, visible).
        // Implement Default manually to avoid silently incorrect data.
        Self {
            background_color: (0, 0, 0, 0),
            border_widths: (0.0, 0.0, 0.0, 0.0),
            opacity: 1.0,
            visible: true,
            id: None,
            layout_size: None,
            clip_descendants: Vec::new(),
            opacity_descendants: Vec::new(),
        }
    }
}

/// Per-node paint state for a paragraph (shaped inline text lines).
///
/// Use Fulgur's `ParagraphEntry` shape (drawables.rs:183-191: `lines`,
/// `opacity`, `visible`, `id`) but select minimal fields compatible with
/// raikiri's current text model. There is no inline formatting context:
/// [`raikiri_dom::Node::text_layout`] belongs to the Text node itself (see
/// `crates/raikiri-paint/src/walk.rs` module docs).
///
/// `build_page_scene` (crate::page_scene) constructs this for every
/// post-layout Text node and inserts it into `PageDrawables::paragraphs`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ParagraphEntry {
    /// Number of shaped lines (`parley::Layout::lines().count()`). The actual
    /// glyph-run positions (Fulgur's `Vec<ShapedLine>`) live in
    /// `parley::Layout<()>`, which is deliberately excluded by the module's
    /// field-type policy. Paint still reads [`raikiri_dom::Node::text_layout`]
    /// directly; this field is only a lightweight count.
    pub line_count: usize,
    /// `opacity`: always the CSS initial value `1.0`, for the same reason
    /// as [`BlockEntry::opacity`].
    pub opacity: f32,
    /// `visibility`: always `true`, as for [`BlockEntry::visible`].
    pub visible: bool,
    /// Anchor ID (`id="..."` on the inline root). The current per-node
    /// granularity is the Text node; reading an ID from its parent Element
    /// (e.g. `<p id="...">`) would require threading the parent ID through
    /// the `build_page_scene` DFS stack. That change is deferred, so this
    /// remains `None`; resolving ID-anchored links is future work.
    pub id: Option<String>,
}

impl Default for ParagraphEntry {
    fn default() -> Self {
        // Manual Default for the same CSS opacity/visibility initial-value
        // reason as BlockEntry.
        Self {
            line_count: 0,
            opacity: 1.0,
            visible: true,
            id: None,
        }
    }
}

/// Per-node raster-image (jpg, png, gif, etc.) paint state.
///
/// Fulgur's `ImageEntry` shape (drawables.rs:205-213) includes `image_data`,
/// `format`, `width`, `height`, `opacity`, and `visible`. Raikiri lacks an
/// image-decoding pipeline (fetching `<img>` bytes, identifying the format,
/// and decoding raster pixels), so `image_data` and `format` are excluded.
/// Including fields that could only contain dummy data would falsely imply
/// they were populated, contrary to the field-type conservatism constraint.
/// Only width, height, opacity, and visibility retain the reference shape.
///
/// **Not populated:** `build_page_scene` (crate::page_scene) never constructs
/// this type. It currently populates only [`BlockEntry`] and
/// [`ParagraphEntry`]; the other nine types only gained reference-shaped
/// fields.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ImageEntry {
    /// Display width in CSS px, per the module unit note.
    pub width: Pt,
    /// Display height (CSS px).
    pub height: Pt,
    /// `opacity`: always CSS initial value `1.0` (see [`BlockEntry::opacity`]).
    pub opacity: f32,
    /// `visibility`: always `true` (see [`BlockEntry::visible`]).
    pub visible: bool,
}

impl Default for ImageEntry {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// Per-node SVG image paint state.
///
/// Fulgur's `SvgEntry` shape (drawables.rs:225-232) includes `tree`, `width`,
/// `height`, `opacity`, and `visible`. Raikiri has no SVG parsing/rendering
/// pipeline equivalent to `usvg`, so `tree` is omitted (see [`ImageEntry`]
/// docs).
///
/// **Not populated:** Never constructed, for the same reason as
/// [`ImageEntry`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SvgEntry {
    /// Display width (CSS px).
    pub width: Pt,
    /// Display height (CSS px).
    pub height: Pt,
    /// `opacity`: always CSS initial value `1.0`.
    pub opacity: f32,
    /// `visibility`: always `true`.
    pub visible: bool,
}

impl Default for SvgEntry {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// Per-node paint state for the outer table frame (background, border).
///
/// Fulgur's `TableEntry` shape (drawables.rs:243-257) includes `style`,
/// `opacity`, `visible`, `id`, `layout_size`, `width`, `cached_height`, and
/// `clip_descendants`. As for [`BlockEntry`], style is flattened, opacity and
/// visibility use CSS initial values, and clipping stays empty because
/// cascade has no overflow property.
///
/// **Not populated:** raikiri-dom has no table-specific layout algorithm;
/// `<table>` uses generic block layout like other Elements.
/// `build_page_scene` treats it as [`BlockEntry`] in `block_styles`. This
/// struct only prepares reference-shaped fields and is never constructed
/// until real table layout lands.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TableEntry {
    /// `background-color` (r, g, b, a); see [`BlockEntry::background_color`].
    pub background_color: (u8, u8, u8, u8),
    /// Four border widths; see [`BlockEntry::border_widths`].
    pub border_widths: (Pt, Pt, Pt, Pt),
    /// `opacity`: always CSS initial value `1.0`.
    pub opacity: f32,
    /// `visibility`: always `true`.
    pub visible: bool,
    /// `id` attribute.
    pub id: Option<String>,
    /// Taffy border-box size in CSS px; see [`BlockEntry::layout_size`].
    pub layout_size: Option<(Pt, Pt)>,
    /// Total table width. Fulgur uses this to calculate continued header
    /// widths across pages; raikiri assumes a single page for tables.
    pub width: Pt,
    /// Total table height. Fulgur calls it "cached", but raikiri can obtain
    /// it from layout every time and retains the name only to match the shape.
    pub cached_height: Pt,
    /// Overflow-clipped descendants. Always empty for the same reason as
    /// [`BlockEntry::clip_descendants`].
    pub clip_descendants: Vec<NodeId>,
}

impl Default for TableEntry {
    fn default() -> Self {
        Self {
            background_color: (0, 0, 0, 0),
            border_widths: (0.0, 0.0, 0.0, 0.0),
            opacity: 1.0,
            visible: true,
            id: None,
            layout_size: None,
            width: 0.0,
            cached_height: 0.0,
            clip_descendants: Vec::new(),
        }
    }
}

/// Per-node paint state for a list-item marker (text, image, or none).
///
/// Fulgur's `ListItemEntry` / `ListItemMarker` shape (drawables.rs:271-299)
/// includes `marker`, `marker_line_height`, `opacity`, and `visible`.
/// `marker` is a Text/Image/None enum containing shaped lines or image data.
/// Raikiri cannot render list-item markers yet; introducing an enum now
/// would prematurely fix its shape before the item-4 topology decision
/// (see the module field-type policy).
///
/// **Not populated:** raikiri-dom does not build marker boxes for `<li>`;
/// it only uses generic block layout. `build_page_scene` treats `<li>` as
/// [`BlockEntry`] until marker layout exists, so this type is never
/// constructed.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ListItemEntry {
    /// Marker line height, intended to vertically center image markers.
    pub marker_line_height: Pt,
    /// `opacity`: always CSS initial value `1.0`.
    pub opacity: f32,
    /// `visibility`: always `true`.
    pub visible: bool,
}

impl Default for ListItemEntry {
    fn default() -> Self {
        Self {
            marker_line_height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// Per-node CSS transform state (matrix, origin, descendant scope).
///
/// Fulgur's `TransformEntry` shape (drawables.rs:392-405) includes `matrix`,
/// `origin`, and `descendants`. `raikiri_style::ComputedValues` has no
/// `transform` property, so entries stay identity/empty and are never
/// constructed; CSS transform support has not landed.
///
/// Store the 2D affine `matrix` as a raw `[a, b, c, d, e, f]` array
/// (row-major, like Fulgur's `Affine2D`) rather than adding a new type, in
/// accordance with the module field-type policy.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TransformEntry {
    /// 2D affine matrix `[a, b, c, d, e, f]`.
    pub matrix: [f32; 6],
    /// Transform origin.
    pub origin: (Pt, Pt),
    /// Descendant [`NodeId`] values in transform scope. Always empty
    /// because the cascade has no `transform` property.
    pub descendants: Vec<NodeId>,
}

impl Default for TransformEntry {
    fn default() -> Self {
        // A derived Default produces an all-zero, singular affine matrix;
        // implement Default manually with identity `[1,0,0,1,0,0]`.
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            origin: (0.0, 0.0),
            descendants: Vec::new(),
        }
    }
}

/// Multicolumn column-rule paint spec and per-column-group geometry.
///
/// Fulgur's `MulticolRuleEntry` / `ColumnRuleGeometry` shape
/// (drawables.rs:316-340) is the reference. ComputedValues has no
/// `column-count`, `column-rule`, or other multicolumn property, so avoid
/// introducing a Fulgur-like `ColumnRuleSpec` or
/// `Vec<ColumnRuleGeometry>` (per the module field-type policy). Keep only
/// minimal shape fields; entries remain empty and unconstructed.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct MulticolRuleEntry {
    /// Number of columns to balance; always `0` because the cascade has
    /// no multicolumn properties.
    pub column_count: u32,
}

/// Per-node bookmark anchor (source of a PDF bookmark tree).
///
/// Fulgur's `BookmarkAnchorEntry` shape (drawables.rs:409-413) has `level`
/// and `label`. Raikiri has no author-controlled CSS/API for choosing
/// bookmarked elements, so no entries are constructed. The primitive
/// `u8` and `String` fields still match the reference shape.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct BookmarkAnchorEntry {
    /// Zero-based depth in the bookmark tree.
    pub level: u8,
    /// Bookmark label text.
    pub label: String,
}

/// Hyperlink target span over a glyph run in a paragraph.
///
/// Fulgur's `LinkSpanEntry` (drawables.rs:418-419) is itself still a unit
/// struct and marked as a "PR 3 target". There is no implemented reference
/// shape for raikiri to follow. Leave this entry empty until Fulgur adds
/// its fields, then reassess the shape.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct LinkSpanEntry {}

/// Per-node tagged-PDF state (tag, parent, alt_text, etc.).
///
/// Fulgur's `SemanticEntry` shape (tagging.rs:57) is the reference.
/// Raikiri does not create tagged PDFs or assemble structural tag trees,
/// so this type is never constructed. Its primitive `Option<String>` and
/// `Option<NodeId>` fields only indicate the intended shape.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SemanticEntry {
    /// PDF structure tag name, e.g. `"P"` or `"H1"`.
    pub tag: Option<String>,
    /// Parent [`NodeId`] in the tag tree.
    pub parent: Option<NodeId>,
    /// Alternative text for an image or similar content.
    pub alt_text: Option<String>,
}

#[cfg(test)]
mod tests;
