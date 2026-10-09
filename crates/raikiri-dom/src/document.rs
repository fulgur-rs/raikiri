//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).
//!
//! # Flat tree membership contract
//!
//! Every tree mutation primitive (`append_*` / `attach_child` /
//! `insert_child_before` / `detach_from_parent` / `reparent_children` /
//! `retain_children` / `set_element_namespace` for svg elements) sets
//! [`Document::flags_dirty`] to `true`. Callers observing `Node::is_in_document()`
//! must call [`Document::mark_in_document_flags`] before observation to
//! resynchronize the bit. `mark_in_document_flags` is an O(1) no-op when
//! `!flags_dirty`, so repeated calls are safe.
//!
//! Auto-sync entry:
//! - `raikiri-html::sink::finish()` calls it at the parse observation boundary.
//! - `raikiri-dom::layout_single_page()` calls it at the layout/paint observation boundary.
//!
//! Manual-sync required:
//! - `raikiri-style::cascade()` takes `&D: Dom` and cannot mutate it, so the
//!   caller must perform synchronization. Consumers that call cascade directly
//!   get automatic synchronization only during parsing and must explicitly
//!   call `mark_in_document_flags()` after post-parse mutations.

use smol_str::SmolStr;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::Style;

use raikiri_traits::{QuirksMode, StylesheetKind};

use crate::fragment::{FragmentTree, FragmentationContext};
use crate::layout::LayoutWarn;
use crate::node::{Attr, CanvasBitmap, CanvasBitmapError, Node, NodeData};
use raikiri_style::property::CalcLengthPercentage;

mod mutation;
pub use mutation::DomMutationError;

const XHTML_NAMESPACE_URI: &str = "http://www.w3.org/1999/xhtml";
// A WPT pair can hold one sidecar beside the other document and its paint blob.
// Capping each side at 32 MiB keeps these three canvas buffers at 96 MiB total.
const MAX_DOCUMENT_CANVAS_BITMAP_BYTES: usize = 32 * 1024 * 1024;
// Bound all size variants retained beyond the source cache's current raster.
const MAX_DOCUMENT_LIST_MARKER_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

fn is_xml_name_start(ch: char) -> bool {
    matches!(
        ch,
        ':' | 'A'..='Z'
            | '_'
            | 'a'..='z'
            | '\u{C0}'..='\u{D6}'
            | '\u{D8}'..='\u{F6}'
            | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}'
            | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}'
            | '\u{2C00}'..='\u{2FEF}'
            | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}'
            | '\u{FDF0}'..='\u{FFFD}'
            | '\u{10000}'..='\u{EFFFF}'
    )
}

fn is_xml_name_char(ch: char) -> bool {
    is_xml_name_start(ch)
        || matches!(
            ch,
            '0'..='9' | '-' | '.' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_xml_name_start) && chars.all(is_xml_name_char)
}

fn qualified_name(prefix: Option<&str>, local: &str) -> String {
    prefix.map_or_else(|| local.to_owned(), |prefix| format!("{prefix}:{local}"))
}

fn push_xml_escaped(output: &mut String, value: &str, attribute: bool) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            '\'' if attribute => output.push_str("&apos;"),
            '\n' if attribute => output.push_str("&#xA;"),
            '\r' if attribute => output.push_str("&#xD;"),
            '\t' if attribute => output.push_str("&#x9;"),
            character => output.push(character),
        }
    }
}

/// Strip foreign calc handles from an externally supplied style.
///
/// With the `calc` feature enabled, taffy length types hold a raw pointer
/// through a compact tagged representation. The safe constructor for those
/// handles takes a plain pointer, so any caller can build a style that
/// points outside the document arena. Storing such a style would break the
/// ownership premise behind the `Send` impl, which requires every calc
/// pointer in the arena to point into the same document's stable payload
/// storage. The layout bridge rebuilds owned handles from computed values
/// before each layout pass, so discarding foreign handles here changes no
/// valid behavior: styles without calc pass through untouched.
///
/// Each calc-capable scalar that holds a foreign handle is replaced with a
/// safe keyword fallback. `Dimension` and `LengthPercentageAuto` fall back
/// to `auto`, `LengthPercentage` to zero length, and grid track functions
/// to `auto`. Grid template repetitions keep their count and line names
/// and only have their track functions sanitized.
///
/// The exhaustive field check below pins coverage against future taffy
/// fields. If taffy adds a style field, this function fails to compile
/// until the new field is classified here.
fn sanitize_external_style(mut style: Style) -> Style {
    fn sanitize_dimension(value: taffy::Dimension) -> taffy::Dimension {
        if value.into_raw().is_calc() {
            taffy::Dimension::auto()
        } else {
            value
        }
    }
    fn sanitize_length_percentage_auto(
        value: taffy::LengthPercentageAuto,
    ) -> taffy::LengthPercentageAuto {
        if value.into_raw().is_calc() {
            taffy::LengthPercentageAuto::auto()
        } else {
            value
        }
    }
    fn sanitize_length_percentage(value: taffy::LengthPercentage) -> taffy::LengthPercentage {
        if value.into_raw().is_calc() {
            taffy::LengthPercentage::length(0.0)
        } else {
            value
        }
    }
    fn sanitize_track(value: taffy::TrackSizingFunction) -> taffy::TrackSizingFunction {
        let min = if value.min.into_raw().is_calc() {
            taffy::MinTrackSizingFunction::auto()
        } else {
            value.min
        };
        let max = if value.max.into_raw().is_calc() {
            taffy::MaxTrackSizingFunction::auto()
        } else {
            value.max
        };
        taffy::TrackSizingFunction { min, max }
    }

    {
        let Style {
            dummy: _,
            display: _,
            item_is_table: _,
            item_is_replaced: _,
            box_sizing: _,
            direction: _,
            overflow: _,
            scrollbar_width: _,
            contain: _,
            float: _,
            clear: _,
            position: _,
            inset: _,
            size: _,
            min_size: _,
            max_size: _,
            aspect_ratio: _,
            margin: _,
            padding: _,
            border: _,
            align_items: _,
            align_self: _,
            justify_items: _,
            justify_self: _,
            align_content: _,
            justify_content: _,
            gap: _,
            text_align: _,
            flex_direction: _,
            flex_wrap: _,
            flex_basis: _,
            flex_grow: _,
            flex_shrink: _,
            grid_template_rows: _,
            grid_template_columns: _,
            grid_auto_rows: _,
            grid_auto_columns: _,
            grid_auto_flow: _,
            grid_template_areas: _,
            grid_template_column_names: _,
            grid_template_row_names: _,
            grid_row: _,
            grid_column: _,
        } = &style;
    }

    style.size.width = sanitize_dimension(style.size.width);
    style.size.height = sanitize_dimension(style.size.height);
    style.min_size.width = sanitize_length_percentage_auto(style.min_size.width);
    style.min_size.height = sanitize_length_percentage_auto(style.min_size.height);
    style.max_size.width = sanitize_length_percentage_auto(style.max_size.width);
    style.max_size.height = sanitize_length_percentage_auto(style.max_size.height);
    style.inset.left = sanitize_length_percentage_auto(style.inset.left);
    style.inset.right = sanitize_length_percentage_auto(style.inset.right);
    style.inset.top = sanitize_length_percentage_auto(style.inset.top);
    style.inset.bottom = sanitize_length_percentage_auto(style.inset.bottom);
    style.margin.left = sanitize_length_percentage_auto(style.margin.left);
    style.margin.right = sanitize_length_percentage_auto(style.margin.right);
    style.margin.top = sanitize_length_percentage_auto(style.margin.top);
    style.margin.bottom = sanitize_length_percentage_auto(style.margin.bottom);
    style.padding.left = sanitize_length_percentage(style.padding.left);
    style.padding.right = sanitize_length_percentage(style.padding.right);
    style.padding.top = sanitize_length_percentage(style.padding.top);
    style.padding.bottom = sanitize_length_percentage(style.padding.bottom);
    style.border.left = sanitize_length_percentage(style.border.left);
    style.border.right = sanitize_length_percentage(style.border.right);
    style.border.top = sanitize_length_percentage(style.border.top);
    style.border.bottom = sanitize_length_percentage(style.border.bottom);
    style.gap.width = sanitize_length_percentage(style.gap.width);
    style.gap.height = sanitize_length_percentage(style.gap.height);
    style.flex_basis = sanitize_dimension(style.flex_basis);
    for component in style
        .grid_template_rows
        .iter_mut()
        .chain(style.grid_template_columns.iter_mut())
    {
        match component {
            taffy::GridTemplateComponent::Single(track) => {
                *track = sanitize_track(*track);
            }
            taffy::GridTemplateComponent::Repeat(repetition) => {
                for track in repetition.tracks.iter_mut() {
                    *track = sanitize_track(*track);
                }
            }
        }
    }
    for track in style
        .grid_auto_rows
        .iter_mut()
        .chain(style.grid_auto_columns.iter_mut())
    {
        *track = sanitize_track(*track);
    }
    style
}

fn is_html_raw_text_element(namespace: Option<&str>, tag_name: &str) -> bool {
    namespace.is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI)
        && [
            "script",
            "style",
            "xmp",
            "iframe",
            "noembed",
            "noframes",
            "plaintext",
            "noscript",
        ]
        .iter()
        .any(|tag| tag_name.eq_ignore_ascii_case(tag))
}

// Cloning invalidates this cache because cloned Vec capacities can change.
#[derive(Debug)]
struct CanvasBitmapByteCount(Option<usize>);

impl Clone for CanvasBitmapByteCount {
    fn clone(&self) -> Self {
        Self(None)
    }
}

#[derive(Debug, Clone)]
struct ListMarkerImage {
    url: url::Url,
    pixels: std::sync::Arc<raikiri_traits::DecodedImage>,
    size: raikiri_traits::ImageRasterSize,
}

/// DOM Document (root plus a Vec-backed node arena).
///
/// `nodes` is flat storage keyed by arena indices. Index 0 is the virtual
/// Document root. The usual raikiri-html parse path appends the HTML `<html>`
/// element at index 1 or later, as a child of root index 0.
#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) table_objects: crate::layout::table::anonymous::TableObjects,
    pub(crate) page_projection: crate::page_projection::PageProjection,
    pub(crate) nodes: Vec<Node>,
    pub(crate) resolved_image_urls: std::collections::HashMap<usize, (String, url::Url)>,
    canvas_bitmap_bytes: CanvasBitmapByteCount,
    /// Decoded marker images retained for both sizing and painting.
    list_marker_images: std::collections::HashMap<usize, ListMarkerImage>,
    /// Measured marker advances reserved by the current legacy layout pass.
    pub(crate) legacy_inside_marker_advances: std::collections::HashMap<usize, f32>,
    /// Arena index of the Document root (normally 0, stored explicitly to
    /// accommodate unusual future cases such as detaching the root).
    pub(crate) root: usize,
    /// Layout cache dirty flag. Any tree mutation sets it; the next
    /// `compute_child_layout` lazily clears every node cache and resets it.
    /// O(1) cost per mutation plus O(N) per layout batch gives amortized
    /// O(1) invalidation.
    pub(crate) layout_dirty: bool,
    /// Inline-engine state; `None` until fonts are set or the first layout
    /// takes the installed fonts.
    pub(crate) ifc: Option<crate::layout::ifc::root::IfcState>,
    /// Cascade generation used by the most recent successful layout. Resolved
    /// order projections and Grid row placements are valid only for this run.
    pub(crate) layout_cascade_generation: Option<u64>,
    /// Used left and right margins of `<body>` that the most recent layout
    /// carried as inline padding of the synthetic body root.
    pub(crate) body_inline_margins: (f32, f32),
    /// Used block-start margin of `<body>` that the most recent layout placed
    /// inside the synthetic body root.
    pub(crate) body_block_start_margin: f32,
    /// A forced line break the inline engine inserts before byte `offset`
    /// of text node `text`, so a relayout at another width starts a line
    /// exactly where a page began in an earlier layout.
    pub(crate) continuation_break: Option<(usize, u32)>,
    /// Dirty flag for the IS_IN_DOCUMENT bit. Any tree mutation primitive
    /// (append_* / attach_child / insert_child_before / detach_from_parent /
    /// reparent_children / retain_children) sets it. Observation APIs
    /// (cascade / paint / extract) must call
    /// [`Document::mark_in_document_flags`] before trusting the bit, forcing a
    /// dirty check and lazy recomputation.
    ///
    /// The current parse-only path calls `sink.finish()` explicitly, so this
    /// is handled there. For paths that build a Document with manual `append_*`
    /// calls (raikiri-dom tests, raikiri-paint hello-world setup, and a future
    /// mutation runtime), this flag is essential to correctness.
    pub(crate) flags_dirty: bool,
    /// Stylesheets associated with the Document.
    /// Parsing is deferred to the cascade phase.
    /// Call order determines cascade source_order within each kind.
    stylesheets: Vec<(Cow<'static, str>, StylesheetKind)>,
    /// Buffered [`LayoutWarn`] diagnostic events for the current (or most
    /// recent) `layout_single_page` pass (generalizing
    /// the `fonts.rs` `FontWarn` observer pattern to this crate's other
    /// "silent clamp" site).
    ///
    /// Owned (`Vec`, no borrowed observer) rather than a closure field —
    /// deliberately, not as a simplification of convenience. `<Document as
    /// taffy::LayoutPartialTree>::set_unrounded_layout` is the sole choke
    /// point that writes non-finite-clamped geometry into the arena
    /// ([`crate::layout::sanitize_taffy_layout`]'s doc), but its signature is
    /// fixed by the `taffy` trait — it cannot receive an extra observer
    /// parameter. Storing a borrowed `&mut dyn FnMut` here instead would
    /// require adding a lifetime parameter to `Document` itself, which is a
    /// public-shape break every consumer of this type would have to absorb
    /// (dom→paint wall territory) for a capability nothing external can
    /// plug into yet. An owned buffer sidesteps that: `set_unrounded_layout`
    /// pushes through `self` with no signature change, and
    /// `layout_single_page` drains + replays the buffer through the same
    /// [`crate::diag::emit_warn_via`] mechanism the rest of this module's
    /// diagnostics use, once per pass, after the taffy compute step returns.
    ///
    /// Cleared at the start of each `layout_single_page` call (re-entrance
    /// safety) and drained near its end.
    ///
    /// # Scope boundary: only `layout_single_page` clears/drains this
    ///
    /// A `Document` driven through `taffy::compute_root_layout` directly
    /// (bypassing `layout_single_page` — e.g. this crate's own `lib.rs` unit
    /// tests) still has `set_unrounded_layout` pushing into this buffer, but
    /// nothing clears or drains it. [`LAYOUT_WARN_CAP`]-plus-one bounds the
    /// memory either way, so this is not a leak, but on such a `Document` the
    /// first pathological layout pass fills the buffer and every event after
    /// that collapses into the trailing `Truncated` counter, with nothing
    /// ever reading it back out. Not a problem for `layout_single_page`
    /// callers (the only production path); worth knowing if a future
    /// consumer drives taffy directly and expects these diagnostics.
    ///
    /// [`LAYOUT_WARN_CAP`]: crate::layout::sanitize::LAYOUT_WARN_CAP
    pub(crate) layout_warnings: Vec<LayoutWarn>,
    /// Terminal error raised inside Taffy's infallible table callback.
    pub(crate) table_layout_error: Option<String>,
    /// Stable storage for Taffy calc resolver payloads used by the current
    /// layout pass. The heap allocations keep pointees stable while styles hold raw handles.
    pub(crate) calc_values: Vec<Arc<CalcLengthPercentage>>,
    /// Fragments emitted by the active multicol strategy for this layout pass.
    pub(crate) fragment_tree: FragmentTree,
    /// Column rules retained from final placements, relative to each owner.
    pub(crate) column_rules: std::collections::BTreeMap<usize, Vec<raikiri_traits::PaintRect>>,
    /// Active nested fragmentainer stack while Taffy recursively lays out nodes.
    pub(crate) fragmentation_stack: Vec<FragmentationContext>,
    /// Vertical block containers being laid out whose absolutely positioned
    /// children use the recorded physical height as their containing
    /// block's inline size; see `taffy_impl::vertical_oof`.
    pub(crate) vertical_oof_containing_blocks: Vec<(usize, f32)>,
    /// HTML5 quirks mode for this whole document. Default
    /// [`QuirksMode::NoQuirks`] (matching the type's own `#[default]`) for
    /// `Document`s built by hand (raikiri-dom unit tests, raikiri-paint
    /// hello-world setup). raikiri-html's parse sink calls
    /// [`Document::set_quirks_mode`] with the html5ever-detected value
    /// before handing the `Document` off, so parsed documents carry their
    /// real value. Read back via [`Document::quirks_mode`] and by
    /// `impl raikiri_style::StyleDom for Document`'s `quirks_mode()`
    /// override (`dom_impl.rs`), which converts it to
    /// `raikiri_style::StyleQuirksMode` for cascade's id/class
    /// case-folding (CSS Selectors L4).
    quirks_mode: QuirksMode,
}

impl Document {
    /// Lay text out with the fonts of `fonts`.
    ///
    /// `fonts` is the font layer the inline engine looks families up in, for
    /// example one built from bundled fonts, or a document layer of
    /// `@font-face` faces over a shared layer
    /// ([`crate::build_inline_document_fonts`]). Without this call, the first
    /// layout uses the installed fonts ([`crate::system_font_collection`]).
    pub fn set_font_collection(&mut self, fonts: shodo::font::FontCollection) {
        self.set_font_collection_with_limits(fonts, shodo::limits::Limits::default());
    }

    /// [`Document::set_font_collection`] with explicit resource limits for
    /// the inline engine.
    #[doc(hidden)]
    pub fn set_font_collection_with_limits(
        &mut self,
        fonts: shodo::font::FontCollection,
        limits: shodo::limits::Limits,
    ) {
        self.ifc = Some(crate::layout::ifc::root::IfcState::new(fonts, limits));
        self.layout_dirty = true;
    }

    /// Metrics of a font of the inline engine, at `size`. `None` before the
    /// document has fonts.
    #[doc(hidden)]
    pub fn ifc_font_metrics(
        &self,
        font: shodo::font::FontId,
        size: f32,
    ) -> Option<shodo::font::FontMetrics> {
        self.ifc
            .as_ref()
            .map(|state| state.fonts.metrics(font, size))
    }

    /// Whether the document has fonts for the inline engine: they were set,
    /// or a layout took the installed fonts.
    pub fn has_font_collection(&self) -> bool {
        self.ifc.is_some()
    }

    /// Lower the cap shared by retained layout fragments and break-flow or
    /// page-projection work. A value above the current cap is ignored, so the
    /// built-in bound can only tighten.
    #[doc(hidden)]
    pub fn lower_layout_fragment_limit(&mut self, limit: usize) {
        self.fragment_tree.limit = self.fragment_tree.limit.min(limit);
    }

    /// Paragraphs are built on several threads when a layout pass has at
    /// least `threshold` of them and [`Document::set_ifc_parallel_build`]
    /// allowed it. No effect without the inline engine.
    #[doc(hidden)]
    pub fn set_ifc_parallel_threshold(&mut self, threshold: usize) {
        if let Some(state) = self.ifc.as_mut() {
            state.parallel_threshold = threshold;
        }
    }

    /// Allow building paragraphs on several threads. Only safe when the font
    /// collection has no system faces that are loaded on first use: the face
    /// chosen for text no family covers would then depend on thread
    /// scheduling. Off by default; no effect without the inline engine.
    #[doc(hidden)]
    pub fn set_ifc_parallel_build(&mut self, allowed: bool) {
        if let Some(state) = self.ifc.as_mut() {
            state.parallel_build = allowed;
        }
    }

    /// Whether paragraphs may be built on several threads (`false` without
    /// the inline engine).
    #[doc(hidden)]
    pub fn ifc_parallel_build(&self) -> bool {
        self.ifc.as_ref().is_some_and(|state| state.parallel_build)
    }

    /// How the paragraphs of the last layout pass were built; `None` before
    /// the first pass or without the inline engine.
    #[doc(hidden)]
    pub fn ifc_last_build(&self) -> Option<crate::IfcBuildMode> {
        self.ifc.as_ref().and_then(|state| state.last_build)
    }

    /// Construct a new Document with a Document node at arena index 0.
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(16);
        nodes.push(Node::new_document());
        Self {
            table_objects: crate::layout::table::anonymous::TableObjects::default(),
            page_projection: crate::page_projection::PageProjection::default(),
            nodes,
            resolved_image_urls: Default::default(),
            canvas_bitmap_bytes: CanvasBitmapByteCount(Some(0)),
            list_marker_images: Default::default(),
            legacy_inside_marker_advances: Default::default(),
            root: 0,
            layout_dirty: false,
            ifc: None,
            layout_cascade_generation: None,
            body_inline_margins: (0.0, 0.0),
            body_block_start_margin: 0.0,
            continuation_break: None,
            // Node::new_document() sets IS_IN_DOCUMENT=true on the initial root,
            // consistent with an attached root. There are no templates or
            // detached nodes yet, so the flag is not dirty.
            flags_dirty: false,
            stylesheets: Vec::new(),
            layout_warnings: Vec::new(),
            table_layout_error: None,
            calc_values: Vec::new(),
            fragment_tree: FragmentTree::default(),
            column_rules: std::collections::BTreeMap::new(),
            fragmentation_stack: Vec::new(),
            vertical_oof_containing_blocks: Vec::new(),
            quirks_mode: QuirksMode::default(),
        }
    }

    /// Create an HTML element without attaching it to a parent.
    pub fn create_detached_element(&mut self, tag: &str) -> Result<usize, String> {
        if !is_valid_xml_name(tag) {
            return Err(format!("invalid HTML element name: {tag:?}"));
        }
        Ok(self.append_element(None, tag, Style::default(), None::<&str>))
    }

    /// Add an Element node to the arena. If `parent` is `Some(idx)`, append it
    /// to that node's children. With `None`, it remains detached (for a fragment
    /// unattached to the tree, or for later attachment).
    ///
    /// Pass the raw HTML `style="..."` attribute string as `inline_style`
    /// (`None` means no attribute). raikiri-style::cascade consumes it.
    ///
    /// # Contract
    ///
    /// This method sets [`flags_dirty`](Self#structfield.flags_dirty) to `true`.
    /// Callers observing `Node::is_in_document()` (raikiri-style::cascade /
    /// raikiri-dom::layout_single_page / raikiri-paint::paint_single_page) must
    /// call [`mark_in_document_flags`](Self::mark_in_document_flags) after a
    /// mutation batch to resynchronize the bit. `layout_single_page` does this
    /// automatically on entry, so consumers of only the layout/paint pipeline
    /// need no explicit call. Direct cascade callers must synchronize explicitly.
    ///
    /// Returns: the arena index of the added node.
    ///
    /// Foreign calc handles in `style` are replaced with safe keyword
    /// fallbacks before storage, so the arena never holds a pointer it
    /// does not own. Styles without calc pass through untouched, and the
    /// layout bridge rebuilds owned calc handles from computed values
    /// before each layout pass.
    pub fn append_element(
        &mut self,
        parent: Option<usize>,
        tag: impl Into<SmolStr>,
        style: Style,
        inline_style: Option<impl Into<SmolStr>>,
    ) -> usize {
        let style = sanitize_external_style(style);
        let id = self.nodes.len();
        self.nodes.push(Node::new_element(
            tag.into(),
            style,
            inline_style.map(Into::into),
        ));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
            self.nodes[id].parent = Some(p);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Add a Text node to the arena as a child of `parent` (which is required;
    /// text nodes are always attached).
    ///
    /// Returns: the arena index of the added node.
    pub fn append_text(&mut self, parent: usize, text: impl Into<SmolStr>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_text(text.into()));
        self.nodes[parent].children.push(id);
        self.nodes[id].parent = Some(parent);
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Append character data, merging it into the parent's adjacent final text
    /// node when possible. HTML token streams may split one logical character
    /// run into several callbacks; coalescing those callbacks preserves the
    /// inline formatting run without changing the ordinary [`Document::append_text`]
    /// mutation primitive.
    pub fn append_text_coalesced(&mut self, parent: usize, text: impl Into<SmolStr>) -> usize {
        let text = text.into();
        if let Some(&last) = self.nodes[parent].children.last()
            && let NodeData::Text(data) = &mut self.nodes[last].data
        {
            let mut merged = String::with_capacity(data.text_content.len() + text.len());
            merged.push_str(data.text_content.as_str());
            merged.push_str(text.as_str());
            data.text_content = SmolStr::new(merged);
            self.invalidate_layout_cache();
            self.flags_dirty = true;
            return last;
        }
        self.append_text(parent, text)
    }

    /// Add a Comment node to the arena.
    ///
    /// If `parent` is `Some(idx)`, append it to that node's children. With
    /// `None`, it remains detached, matching html5ever's `TreeSink::create_comment`
    /// primitive: html5ever creates a detached comment and later calls
    /// `append(parent, AppendNode(c))`.
    ///
    /// # Flat tree semantics
    ///
    /// A Comment has `NodeKind::Comment`, not `NodeKind::Element`, so the Element
    /// gate automatically skips it during cascade, paint, and stylesheet extraction.
    /// Also, [`Document::mark_in_document_flags`] clears `IS_IN_DOCUMENT` for
    /// Comment and PI variants, removing them from the Taffy layout tree through
    /// its is_in_document filter. These two gates replaced the old
    /// `strip_non_element_stubs` (physical removal from arena children Vec),
    /// which was removed from the raikiri-html sink.
    ///
    /// Returns: the arena index of the added node.
    pub fn append_comment(&mut self, parent: Option<usize>, text: impl Into<SmolStr>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_comment(text.into()));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
            self.nodes[id].parent = Some(p);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Add a processing instruction node to the arena.
    ///
    /// `parent` has the same semantics as for Comment (Some attaches it; None
    /// leaves it detached). It rarely occurs in HTML but is a valid NodeType
    /// in XML / XHTML (WHATWG DOM §4).
    ///
    /// Flat tree semantics: the same as Comment. The Element gate skips it
    /// during cascade / paint / extraction, and clearing `IS_IN_DOCUMENT`
    /// removes it from Taffy as well.
    ///
    /// Returns: the arena index of the added node.
    pub fn append_processing_instruction(
        &mut self,
        parent: Option<usize>,
        target: impl Into<SmolStr>,
        data: impl Into<SmolStr>,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes
            .push(Node::new_processing_instruction(target.into(), data.into()));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
            self.nodes[id].parent = Some(p);
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Attach an existing detached node as the final child of `parent`.
    ///
    /// Primitive for html5ever's `TreeSink::append(parent, AppendNode(child))`.
    /// `child` must already exist in the arena. If it belongs to another parent,
    /// detach it first with [`Document::detach_from_parent`]; raikiri-dom does not
    /// detach it automatically to prevent duplicate placement in the tree.
    ///
    /// # Fragment-aware semantics (WHATWG DOM §4.2.3 Mutation algorithms)
    ///
    /// If `child` is the [`NodeData::DocumentFragment`] variant, the fragment
    /// node itself is not appended to `parent.children`. Instead, all its
    /// children move to the end of the parent's children, leaving the fragment
    /// empty. This follows insert algorithm steps 1 + 4.1 + 7.2: step 1 takes
    /// the fragment's children as the nodes to insert, step 4.1 drains them,
    /// and step 7.2 appends them to the parent's children. The spec defines
    /// append as "pre-insert node into parent before null", so this method takes
    /// the referenceChild = null branch (step 7.2). The non-null positional
    /// splice in step 7.3 belongs to [`Document::insert_child_before`]. This is
    /// spec-conformant DocumentFragment insertion: the fragment itself remains
    /// an unrendered virtual container.
    ///
    /// Element / Text / Comment / PI / Document nodes are appended directly,
    /// preserving the previous behavior. At runtime, html5ever uses fragments
    /// only as parents returned by `get_template_contents`, never as children.
    /// This branch serves consumers that drive raikiri-dom directly (tests and
    /// a future DOM Mutation API).
    ///
    /// # Panics
    ///
    /// - `parent` or `child` is outside the arena (`nodes[..]` indexing).
    /// - `parent == child` (equivalent to a spec HierarchyRequestError) is not
    ///   currently detected. It does not arise in the current implementation;
    ///   a future spec-conformant mutation API should check and raise it.
    pub fn attach_child(&mut self, parent: usize, child: usize) {
        // Fragment-aware branch: WHATWG DOM insert algorithm steps 1 + 4.1 + 7.2
        // (§4.2.3 Mutation algorithms) has the same effect: exclude the fragment
        // itself from parent.children and move its children to the parent's end.
        // This is the tail-append counterpart of insert_child_before's splice.
        if matches!(self.nodes[child].data, NodeData::DocumentFragment) {
            // Follow the drain + extend pattern of reparent_children. The fragment
            // itself is absent from `parent.children` (see contract test (c)).
            let moved: Vec<usize> = self.nodes[child].children.drain(..).collect();
            self.nodes[parent].children.extend(moved.iter().copied());
            for moved_child in moved {
                if let Some(node) = self.nodes.get_mut(moved_child) {
                    node.parent = Some(parent);
                }
            }
        } else {
            self.nodes[parent].children.push(child);
            if let Some(node) = self.nodes.get_mut(child) {
                node.parent = Some(parent);
            }
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
    }

    /// Append a detached or already-connected child to `parent` using DOM
    /// move semantics, delegating to [`Document::pre_insert`] with
    /// `before = None` (DOM §4.2.3 "pre-insert node into parent before
    /// null").
    ///
    /// Kept as a thin `String`-erroring wrapper for existing callers; new
    /// code should call [`Document::pre_insert`] directly to distinguish
    /// [`DomMutationError::HierarchyRequest`] from
    /// [`DomMutationError::NotFound`].
    pub fn append_child(&mut self, parent: usize, child: usize) -> Result<(), String> {
        self.pre_insert(parent, child, None)
            .map_err(|err| match err {
                DomMutationError::HierarchyRequest(message)
                | DomMutationError::NotFound(message) => message,
            })
    }

    /// Insert `child` immediately before `before` in `parent`'s children array.
    ///
    /// html5ever `TreeSink::append_before_sibling(sibling, AppendNode(child))`
    /// and foster parenting use this primitive. If `before` is not a child of
    /// `parent`, append at the end (defensive; the TreeSink contract rules this out).
    ///
    /// # Fragment-aware semantics (WHATWG DOM §4.2.3 Mutation algorithms)
    ///
    /// If `child` is the [`NodeData::DocumentFragment`] variant, the fragment
    /// node itself is not inserted into `parent.children`. Instead, its children
    /// are spliced in source order at `before`, leaving the fragment empty.
    /// This follows insert algorithm steps 1 + 4.1 + 7.3: step 1 takes the
    /// fragment's children as the nodes to insert, step 4.1 drains them, and
    /// step 7.3 splices them at the referenceChild index. Step 7.2 handles a
    /// null referenceChild (tail append); this method handles the non-null
    /// branch in step 7.3. This is spec-conformant DocumentFragment insertion:
    /// the fragment itself remains an unrendered virtual container.
    ///
    /// Element / Text / Comment / PI / Document nodes are inserted directly,
    /// preserving the previous behavior. At runtime, html5ever never passes a
    /// fragment as `append_before_sibling`'s new_node: fragments occur only as
    /// parents returned by `get_template_contents`. This branch serves direct
    /// raikiri-dom consumers (tests and a future DOM Mutation API). It also
    /// resolves a previously observed asymmetry with [`Document::attach_child`].
    pub fn insert_child_before(&mut self, parent: usize, before: usize, child: usize) {
        // Fragment-aware branch: WHATWG DOM insert algorithm steps 1 + 4.1 + 7.3
        // (§4.2.3 Mutation algorithms) has the same effect: exclude the fragment
        // itself from parent.children and splice its children in source order at
        // `before`. This is the positional counterpart of attach_child's append.
        if matches!(self.nodes[child].data, NodeData::DocumentFragment) {
            // Drain fragment children (move semantics); collect() ends the borrow
            // before the later &mut borrow of parent.children.
            let moved: Vec<usize> = self.nodes[child].children.drain(..).collect();
            let kids = &mut self.nodes[parent].children;
            let pos = kids.iter().position(|&c| c == before).unwrap_or_else(|| {
                // The html5ever TreeSink contract rules this out. Panic on a
                // contract violation in dev/test; fall back to append in release.
                debug_assert!(
                    false,
                    "insert_child_before: `before` ({before}) not a child of parent ({parent})"
                );
                kids.len()
            });
            // Vec::splice(pos..pos, moved) inserts at pos in one pass without
            // removing anything. It completes with O(n+k) allocation.
            kids.splice(pos..pos, moved.iter().copied());
            for moved_child in moved {
                if let Some(node) = self.nodes.get_mut(moved_child) {
                    node.parent = Some(parent);
                }
            }
        } else {
            let kids = &mut self.nodes[parent].children;
            if let Some(pos) = kids.iter().position(|&c| c == before) {
                kids.insert(pos, child);
            } else {
                // The html5ever TreeSink contract rules this out. Panic on a
                // contract violation in dev/test; use the specified tail-append
                // fallback in release.
                debug_assert!(
                    false,
                    "insert_child_before: `before` ({before}) not a child of parent ({parent})"
                );
                kids.push(child);
            }
            if let Some(node) = self.nodes.get_mut(child) {
                node.parent = Some(parent);
            }
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
    }

    /// Return the arena index of the parent holding `child`. Return `None` for
    /// the root (index 0), an unattached node, or an out-of-range `child`.
    ///
    /// O(1) via the per-node parent pointer kept in sync by every tree
    /// mutation primitive. The pointer tracks only `children` edges, not a
    /// `<template>` element's `template_contents` slot (that host link is not
    /// a parent, so a contents fragment root still reports `None`).
    pub fn parent_of(&self, child: usize) -> Option<usize> {
        self.nodes.get(child)?.parent
    }

    /// Used left and right margins of `<body>`, in CSS pixels, as of the most
    /// recent layout.
    ///
    /// Layout lays `<body>` out as the root box spanning the page content
    /// width and carries these margins as extra inline padding of that root,
    /// so the body's `unrounded_layout.padding` includes them and its
    /// content already sits inside them. A consumer that needs the body's
    /// own border box insets the root box by these values; one that needs
    /// the authored padding subtracts them from the laid-out padding. Both
    /// are 0 before the first layout.
    #[doc(hidden)]
    pub fn body_inline_margins(&self) -> (f32, f32) {
        self.body_inline_margins
    }

    /// Used block-start margin of `<body>`, in CSS pixels, as of the most
    /// recent layout.
    ///
    /// This is the body's top margin collapsed with the top margins of its
    /// first in-flow children when they adjoin (CSS 2.1 §8.3.1), or the
    /// body's own margin when they do not. Layout lays `<body>` out as the
    /// root box at the top of the page content box and moves the body's
    /// content down by this value, so laid-out coordinates already include
    /// it. A consumer that needs the body's own border box insets the top of
    /// the root box by it. It can be negative, and is 0 before the first
    /// layout. The `<html>` margin is not included: margins of the root
    /// element's box do not collapse, and the body root does not carry them.
    #[doc(hidden)]
    pub fn body_block_start_margin(&self) -> f32 {
        self.body_block_start_margin
    }

    /// Fragments emitted by the most recent layout pass.
    #[doc(hidden)]
    pub fn layout_fragments(&self) -> &[crate::fragment::LayoutFragment] {
        &self.fragment_tree.fragments
    }

    /// Lines of a text node inside a paragraph laid out by the inline engine,
    /// in the paragraph root's content box.
    ///
    /// `None` for any other node, for a paragraph without lines, and for a
    /// text node that has no glyph on any line.
    pub fn ifc_text_lines(&self, node: usize) -> Option<crate::layout::IfcTextLines> {
        crate::layout::ifc::text_lines::lines_of(self, node)
    }

    /// Remove `child` from its current parent and return that parent's index.
    /// Return `None` for an unattached node (no-op).
    ///
    /// Primitive for html5ever's `TreeSink::remove_from_parent(target)`.
    pub fn detach_from_parent(&mut self, child: usize) -> Option<usize> {
        let parent = self.parent_of(child)?;
        let kids = &mut self.nodes[parent].children;
        if let Some(pos) = kids.iter().position(|&c| c == child) {
            kids.remove(pos);
        }
        if let Some(node) = self.nodes.get_mut(child) {
            node.parent = None;
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        Some(parent)
    }

    /// Move all children of `from` to the end of `to`'s children, leaving
    /// `from` empty. Primitive for html5ever's `TreeSink::reparent_children`.
    pub fn reparent_children(&mut self, from: usize, to: usize) {
        let moved: Vec<usize> = self.nodes[from].children.drain(..).collect();
        self.nodes[to].children.extend(moved.iter().copied());
        for moved_child in moved {
            if let Some(node) = self.nodes.get_mut(moved_child) {
                node.parent = Some(to);
            }
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
    }

    /// Associate a non-HTML namespace URI with an Element node.
    /// `ns = None` denotes the default HTML namespace; this has no effect on
    /// non-elements. Using `None` for HTML saves memory and enables an O(1)
    /// check in `Element::namespace_uri()`.
    ///
    /// The raikiri-html sink calls this from its qual_names metadata table
    /// during `finish()`. It does not call `invalidate_layout_cache` because
    /// it does not mutate the tree.
    ///
    /// Panics (debug and release): if `id` is not an Element. Calling an
    /// attribute-family setter on a Text or Document node is a caller bug, so
    /// it fails early. Replacing the old `debug_assert_eq!` with
    /// `NodeData::as_element_mut().expect(...)` intentionally makes this panic
    /// in release builds too.
    pub fn set_element_namespace(&mut self, id: usize, ns: Option<SmolStr>) {
        self.set_element_namespace_info(id, ns, None);
    }

    /// Set an element namespace URI and its source prefix.
    pub fn set_element_namespace_info(
        &mut self,
        id: usize,
        ns: Option<SmolStr>,
        prefix: Option<SmolStr>,
    ) {
        self.page_projection.clear();
        let (namespace_changed, affects_tree_flags, affects_layout) = {
            let e = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_namespace called on non-Element");
            let changed = e.namespace != ns;
            // Only `<svg>` affects the tree flags: the inline-SVG subtree bits
            // depend on the SVG namespace. A `<template>` element's namespace
            // no longer affects membership — ordinary light-DOM children stay
            // in-document regardless, and the inert contents fragment is
            // unreachable from the Document root in any namespace.
            let affects_tree_flags = changed && e.tag_name.as_str() == "svg";
            let affects_layout = changed && e.tag_name.as_str() == "svg";
            e.namespace = ns;
            e.prefix = prefix;
            (changed, affects_tree_flags, affects_layout)
        };
        if namespace_changed && affects_tree_flags {
            self.flags_dirty = true;
        }
        if affects_layout {
            self.invalidate_layout_cache();
        }
    }

    /// Associate an attribute list with an Element node.
    /// `attrs` contains `(local, value)` pairs for null-namespace attributes.
    /// A Vec preserves html5ever's source order. The caller must exclude the
    /// `style` attribute, which is wired separately by
    /// [`Document::set_element_inline_style`].
    ///
    /// The raikiri-html sink calls this from its attributes metadata table
    /// during `finish()`. It does not call `invalidate_layout_cache` because
    /// it does not mutate the tree.
    ///
    /// Panics (debug and release): if `id` is not an Element.
    pub fn set_element_attributes(&mut self, id: usize, attrs: Vec<(SmolStr, SmolStr)>) {
        self.page_projection.clear();
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_attributes called on non-Element");
        e.attributes = attrs
            .into_iter()
            .map(|(local, value)| Attr {
                namespace: None,
                prefix: None,
                local,
                value,
            })
            .collect();
    }

    /// Set one namespace-qualified attribute on an element.
    pub fn set_element_namespaced_attribute(
        &mut self,
        id: usize,
        namespace: impl Into<SmolStr>,
        prefix: Option<SmolStr>,
        local: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let namespace = namespace.into();
        let local = local.into();
        if namespace.is_empty() || !is_valid_xml_name(local.as_str()) {
            return Err("invalid namespace-qualified attribute name".to_owned());
        }
        let value = value.into();
        self.page_projection.clear();
        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_namespaced_attribute called on non-Element");
        if let Some(existing) = element.attributes.iter_mut().find(|attribute| {
            attribute.namespace.as_deref() == Some(namespace.as_str()) && attribute.local == local
        }) {
            existing.prefix = prefix;
            existing.value = value;
        } else {
            element.attributes.push(Attr {
                namespace: Some(namespace),
                prefix,
                local,
                value,
            });
        }
        Ok(())
    }

    /// Set one null-namespace attribute on an element.
    ///
    /// The first existing entry keeps its source-order position; duplicate
    /// entries with the same local name are removed. A missing attribute is
    /// appended. The `style` attribute is stored in the separate
    /// inline-style slot used by [`Document::set_element_inline_style`], never
    /// in `ElementData::attributes`.
    ///
    /// This updates attribute metadata only; like [`Document::set_element_attributes`],
    /// it does not invalidate layout caches or mark tree membership dirty.
    /// Stored page placements and links are cleared.
    ///
    /// Returns an error when `local` is not an XML name. HTML-namespace element
    /// names are ASCII-lowercased; foreign-content names preserve their case.
    ///
    /// Panics (debug + release): `id` is not an Element.
    pub fn set_element_attribute(
        &mut self,
        id: usize,
        local: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let local = local.into();
        if !is_valid_xml_name(local.as_str()) {
            return Err(format!("invalid attribute name: {local}"));
        }
        self.page_projection.clear();
        let NodeData::Element(element) = &self.nodes[id].data else {
            panic!("set_element_attribute called on non-Element");
        };
        let html_element = element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI);
        let local: SmolStr = if html_element {
            local.to_ascii_lowercase().into()
        } else {
            local
        };
        let value = value.into();
        if local.as_str() == "style" {
            // Keep the storage invariant even if a caller previously populated
            // the full-list setter with a style entry by mistake.
            let element = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_attribute called on non-Element");
            element
                .attributes
                .retain(|attr| attr.namespace.is_some() || attr.local != local);
            self.set_element_inline_style(id, Some(value));
            return Ok(());
        }

        // Whether this write resizes a canvas bitmap. Computed before the
        // exclusive borrow below so the immutable canvas check does not
        // conflict with it.
        let is_canvas_size_attr =
            matches!(local.as_str(), "width" | "height") && self.is_canvas_element(id);
        {
            let element = self.nodes[id]
                .data
                .as_element_mut()
                .expect("set_element_attribute called on non-Element");
            let mut found = false;
            element.attributes.retain_mut(|attr| {
                if attr.namespace.is_none() && attr.local == local {
                    if found {
                        false
                    } else {
                        found = true;
                        attr.value = value.clone();
                        true
                    }
                } else {
                    true
                }
            });
            if !found {
                element.attributes.push(Attr {
                    namespace: None,
                    prefix: None,
                    local: local.clone(),
                    value: value.clone(),
                });
            }
        }
        // A canvas width/height change resizes and clears the bitmap (HTML
        // Standard §4.12.5 always clears, even when the size is unchanged)
        // and changes the intrinsic size, so layout caches must be
        // invalidated the same way image resolution does. Oversized or
        // over-budget bitmaps remain implicitly transparent without storage.
        if is_canvas_size_attr {
            self.reset_canvas_bitmap_for_current_size(id);
            self.invalidate_layout_cache();
        }
        Ok(())
    }

    /// Remove one null-namespace attribute from an element and return its
    /// stored value, if present. All matching entries are removed. The
    /// separate inline-style slot is used for `style` and is cleared by this
    /// method as well.
    ///
    /// This updates attribute metadata only; it does not invalidate layout
    /// caches or mark tree membership dirty.
    /// Stored page placements and links are cleared.
    ///
    /// Returns an error when `local` is not an XML name. HTML-namespace element
    /// names are ASCII-lowercased; foreign-content names preserve their case.
    ///
    /// Panics (debug + release): `id` is not an Element.
    pub fn remove_element_attribute(
        &mut self,
        id: usize,
        local: &str,
    ) -> Result<Option<SmolStr>, String> {
        if !is_valid_xml_name(local) {
            return Err(format!("invalid attribute name: {local}"));
        }
        self.page_projection.clear();
        let NodeData::Element(element) = &self.nodes[id].data else {
            panic!("remove_element_attribute called on non-Element");
        };
        let html_element = element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI);
        let local = if html_element {
            local.to_ascii_lowercase()
        } else {
            local.to_owned()
        };
        let is_canvas_size_attr =
            matches!(local.as_str(), "width" | "height") && self.is_canvas_element(id);
        let first_value = {
            let element = self.nodes[id]
                .data
                .as_element_mut()
                .expect("remove_element_attribute called on non-Element");
            if local == "style" {
                // Remove any legacy/misrouted list entries as well as the actual
                // inline-style value, so `attr("style")` has one source of truth.
                let legacy_value = element
                    .attributes
                    .iter()
                    .find(|attr| attr.namespace.is_none() && attr.local.as_str() == local)
                    .map(|attr| attr.value.clone());
                element
                    .attributes
                    .retain(|attr| attr.namespace.is_some() || attr.local.as_str() != local);
                return Ok(element.inline_style.take().or(legacy_value));
            }

            let first_value = element
                .attributes
                .iter()
                .find(|attr| attr.namespace.is_none() && attr.local.as_str() == local)
                .map(|attr| attr.value.clone());
            element
                .attributes
                .retain(|attr| attr.namespace.is_some() || attr.local.as_str() != local);
            first_value
        };
        // Removing a canvas width/height attribute reverts to the default
        // size (HTML Standard §4.12.5), clearing the bitmap and changing the
        // intrinsic size. Only when the attribute actually existed: removing
        // a missing attribute is a no-op.
        if is_canvas_size_attr && first_value.is_some() {
            self.reset_canvas_bitmap_for_current_size(id);
            self.invalidate_layout_cache();
        }
        Ok(first_value)
    }

    /// Return an element's concatenated descendant text, excluding comments and processing instructions.
    pub fn element_text_content(&self, id: usize) -> Option<String> {
        let element = self.nodes.get(id)?;
        if !matches!(&element.data, NodeData::Element(_)) {
            return None;
        }
        let mut text = String::new();
        let mut pending: Vec<usize> = element.children.iter().rev().copied().collect();
        while let Some(child) = pending.pop() {
            let node = self.nodes.get(child)?;
            if let Some(content) = node.text_content() {
                text.push_str(content);
            } else {
                pending.extend(node.children.iter().rev().copied());
            }
        }
        Some(text)
    }

    /// Replace an element's children with a single text node, or no children for empty text.
    ///
    /// Removed nodes remain allocated in the arena but are detached, preserving stable handles.
    pub fn set_element_text_content(
        &mut self,
        id: usize,
        text: impl Into<SmolStr>,
    ) -> Result<(), String> {
        let Some(node) = self.nodes.get(id) else {
            return Err(format!("textContent target index {id} is out of range"));
        };
        if !matches!(&node.data, NodeData::Element(_)) {
            return Err(format!("textContent target index {id} is not an Element"));
        }
        let text = text.into();
        let removed: Vec<usize> = std::mem::take(&mut self.nodes[id].children);
        for old_child in removed {
            if let Some(node) = self.nodes.get_mut(old_child)
                && node.parent == Some(id)
            {
                node.parent = None;
            }
        }
        if !text.is_empty() {
            let text_id = self.nodes.len();
            self.nodes.push(Node::new_text(text));
            self.nodes[id].children.push(text_id);
            if let Some(node) = self.nodes.get_mut(text_id) {
                node.parent = Some(id);
            }
        }
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        Ok(())
    }

    /// Replace `target_parent`'s children with deep copies of
    /// `source_parent`'s children from another document.
    ///
    /// Existing target child nodes stay allocated in the arena, but are
    /// detached from their parent. New elements preserve their tag name,
    /// namespace, null-namespace attributes, inline style, and source order;
    /// text, comments, and processing instructions are copied as nodes.
    /// Template contents are copied into a fresh detached fragment root.
    /// Attribute `style` remains in the separate inline-style slot.
    ///
    /// Existing children are detached together by clearing their parent's
    /// child list. Tree changes mark layout caches and flat-tree membership
    /// dirty in the usual way. The source document is not modified.
    ///
    /// Copied styles pass through the same foreign calc sanitization as
    /// `append_element`, so the target never takes ownership of the source
    /// arena's calc pointers. The layout bridge rebuilds owned calc handles
    /// from computed values before the next layout pass.
    pub fn replace_children_from(
        &mut self,
        target_parent: usize,
        source_document: &Document,
        source_parent: usize,
    ) {
        let target_parent = self.nodes[target_parent]
            .template_contents()
            .unwrap_or(target_parent);
        if !self.nodes[target_parent].children.is_empty() {
            // The parent is already known. Avoid a whole-arena parent lookup
            // and shifting the remaining child IDs for every removed child.
            let removed: Vec<usize> = std::mem::take(&mut self.nodes[target_parent].children);
            for old_child in removed {
                if let Some(node) = self.nodes.get_mut(old_child)
                    && node.parent == Some(target_parent)
                {
                    node.parent = None;
                }
            }
            self.invalidate_layout_cache();
            self.flags_dirty = true;
        }

        // A LIFO worklist avoids recursion on deeply nested parsed documents.
        // Push siblings in reverse so each subtree is copied in source order.
        let mut pending: Vec<(usize, usize)> = source_document.nodes[source_parent]
            .children
            .iter()
            .rev()
            .map(|&child| (child, target_parent))
            .collect();

        while let Some((source_id, target_parent)) = pending.pop() {
            let source_node = &source_document.nodes[source_id];
            let source_children = source_node.children.clone();

            match &source_node.data {
                NodeData::Element(element) => {
                    let new_id = self.append_element(
                        Some(target_parent),
                        element.tag_name.clone(),
                        source_node.style.clone(),
                        element.inline_style.clone(),
                    );
                    self.set_element_namespace_info(
                        new_id,
                        element.namespace.clone(),
                        element.prefix.clone(),
                    );
                    self.set_element_attributes(
                        new_id,
                        element
                            .attributes
                            .iter()
                            .filter(|attr| {
                                attr.namespace.is_none() && attr.local.as_str() != "style"
                            })
                            .map(|attr| (attr.local.clone(), attr.value.clone()))
                            .collect(),
                    );
                    for attr in element
                        .attributes
                        .iter()
                        .filter(|attr| attr.namespace.is_some())
                    {
                        self.set_element_namespaced_attribute(
                            new_id,
                            attr.namespace.clone().unwrap_or_default(),
                            attr.prefix.clone(),
                            attr.local.clone(),
                            attr.value.clone(),
                        )
                        .expect("source namespace-qualified attribute was valid");
                    }

                    if let Some(source_fragment) = element.template_contents {
                        let target_fragment = self.allocate_template_fragment_root(new_id);
                        pending.extend(
                            source_document.nodes[source_fragment]
                                .children
                                .iter()
                                .rev()
                                .map(|&child| (child, target_fragment)),
                        );
                    }
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::Text(text) => {
                    let new_id = self.append_text(target_parent, text.text_content.clone());
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::Comment(text) => {
                    let new_id = self.append_comment(Some(target_parent), text.clone());
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                NodeData::ProcessingInstruction { target, data } => {
                    let new_id = self.append_processing_instruction(
                        Some(target_parent),
                        target.clone(),
                        data.clone(),
                    );
                    pending.extend(source_children.iter().rev().map(|&child| (child, new_id)));
                }
                // A DocumentFragment is a container, not a child node. Match
                // DOM insertion semantics by splicing its children in place.
                NodeData::DocumentFragment => {
                    pending.extend(
                        source_children
                            .iter()
                            .rev()
                            .map(|&child| (child, target_parent)),
                    );
                }
                NodeData::Document => {
                    panic!("replace_children_from cannot copy a Document node as a child");
                }
            }
        }
    }

    /// Serialize a node's children as an HTML fragment using the current arena
    /// state. This is suitable for reading live `innerHTML`; it does not use or
    /// retain an original source string.
    ///
    /// Follows the HTML fragment serializing algorithm (HTML Standard section
    /// 13.3, serialising HTML fragments): attribute names are emitted as stored
    /// without validation, attribute values and text use the escaping-a-string
    /// algorithm, comments and processing instructions are preserved, and HTML
    /// void elements are emitted without end tags. Template elements serialize
    /// their contents fragment rather than ordinary children.
    ///
    /// Escaping replaces `&` with `&amp;` and no-break space (U+00A0) with
    /// `&nbsp;` in both modes; attribute mode additionally replaces `"` with
    /// `&quot;`, while text mode replaces `<` with `&lt;` and `>` with `&gt;`.
    /// In particular `<`, `>` and `'` stay as-is inside attribute values and
    /// `'` stays as-is in text.
    ///
    /// Returns an error for an out-of-range parent, a non-container parent, or
    /// malformed child/template-fragment indices in the arena.
    pub fn serialize_inner_html(&self, parent: usize) -> Result<String, String> {
        fn push_escaped(output: &mut String, value: &str, attribute: bool) {
            // HTML escaping-a-string (section 13.3): `&` and U+00A0 in both
            // modes, `"` only in attribute mode, `<` and `>` only in text mode.
            for ch in value.chars() {
                match ch {
                    '&' => output.push_str("&amp;"),
                    '\u{a0}' => output.push_str("&nbsp;"),
                    '"' if attribute => output.push_str("&quot;"),
                    '<' if !attribute => output.push_str("&lt;"),
                    '>' if !attribute => output.push_str("&gt;"),
                    _ => output.push(ch),
                }
            }
        }

        enum Task<'a> {
            Node(usize, bool),
            EndTag(&'a str),
        }

        let parent_node = self
            .nodes
            .get(parent)
            .ok_or_else(|| format!("innerHTML parent index {parent} is out of range"))?;
        if !matches!(
            &parent_node.data,
            NodeData::Document | NodeData::DocumentFragment | NodeData::Element(_)
        ) {
            return Err(format!(
                "innerHTML parent index {parent} is not a container node"
            ));
        }

        let initial_children = if let NodeData::Element(element) = &parent_node.data {
            if let Some(fragment_id) = element.template_contents {
                let fragment = self.nodes.get(fragment_id).ok_or_else(|| {
                    format!("template contents fragment index {fragment_id} is out of range")
                })?;
                if !matches!(&fragment.data, NodeData::DocumentFragment) {
                    return Err(format!(
                        "template contents index {fragment_id} is not a fragment"
                    ));
                }
                &fragment.children
            } else {
                &parent_node.children
            }
        } else {
            &parent_node.children
        };
        let parent_raw_text = match &parent_node.data {
            NodeData::Element(element) => {
                is_html_raw_text_element(element.namespace.as_deref(), element.tag_name.as_str())
            }
            _ => false,
        };
        let mut output = String::new();
        let mut pending: Vec<Task> = initial_children
            .iter()
            .rev()
            .map(|&child| Task::Node(child, parent_raw_text))
            .collect();

        while let Some(task) = pending.pop() {
            match task {
                Task::EndTag(tag) => {
                    output.push_str("</");
                    output.push_str(tag);
                    output.push('>');
                }
                Task::Node(id, raw_text_parent) => {
                    let node = self
                        .nodes
                        .get(id)
                        .ok_or_else(|| format!("innerHTML child index {id} is out of range"))?;
                    match &node.data {
                        NodeData::Element(element) => {
                            let tag = element.tag_name.as_str();
                            output.push('<');
                            output.push_str(tag);
                            for attr in &element.attributes {
                                // `style` is represented by inline_style and
                                // must have only one serialized source.
                                // Attribute names are emitted as stored: HTML
                                // fragment serialization never validates them.
                                if attr.local.as_str() == "style" {
                                    continue;
                                }
                                output.push(' ');
                                output.push_str(attr.local.as_str());
                                output.push_str("=\"");
                                push_escaped(&mut output, attr.value.as_str(), true);
                                output.push('"');
                            }
                            if let Some(style) = &element.inline_style {
                                output.push_str(" style=\"");
                                push_escaped(&mut output, style.as_str(), true);
                                output.push('"');
                            }
                            output.push('>');

                            let is_html_void = element.namespace.is_none()
                                && [
                                    "area", "base", "br", "col", "embed", "hr", "img", "input",
                                    "link", "meta", "param", "source", "track", "wbr",
                                ]
                                .iter()
                                .any(|void_tag| tag.eq_ignore_ascii_case(void_tag));
                            if is_html_void {
                                continue;
                            }

                            let children = if let Some(fragment_id) = element.template_contents {
                                let fragment = self.nodes.get(fragment_id).ok_or_else(|| {
                                    format!(
                                        "template contents fragment index {fragment_id} is out of range"
                                    )
                                })?;
                                if !matches!(&fragment.data, NodeData::DocumentFragment) {
                                    return Err(format!(
                                        "template contents index {fragment_id} is not a fragment"
                                    ));
                                }
                                &fragment.children
                            } else {
                                &node.children
                            };
                            pending.push(Task::EndTag(tag));
                            let raw_text = is_html_raw_text_element(
                                element.namespace.as_deref(),
                                element.tag_name.as_str(),
                            );
                            pending.extend(
                                children
                                    .iter()
                                    .rev()
                                    .map(|&child| Task::Node(child, raw_text)),
                            );
                        }
                        NodeData::Text(text) => {
                            if raw_text_parent {
                                output.push_str(text.text_content.as_str());
                            } else {
                                push_escaped(&mut output, text.text_content.as_str(), false);
                            }
                        }
                        NodeData::Comment(text) => {
                            output.push_str("<!--");
                            output.push_str(text.as_str());
                            output.push_str("-->");
                        }
                        NodeData::ProcessingInstruction { target, data } => {
                            output.push_str("<?");
                            output.push_str(target.as_str());
                            if !data.is_empty() {
                                output.push(' ');
                                output.push_str(data.as_str());
                            }
                            output.push_str("?>");
                        }
                        NodeData::DocumentFragment => {
                            pending.extend(
                                node.children
                                    .iter()
                                    .rev()
                                    .map(|&child| Task::Node(child, raw_text_parent)),
                            );
                        }
                        NodeData::Document => {
                            return Err(format!(
                                "innerHTML cannot serialize Document node {id} as a child"
                            ));
                        }
                    }
                }
            }
        }

        Ok(output)
    }

    /// Allocate a new contents fragment root for a `<template>` element and
    /// store its arena index in the element's `template_contents` slot.
    ///
    /// # Fragment root shape (NodeData::DocumentFragment)
    ///
    /// The fragment root is allocated detached in the `Document.nodes` arena
    /// (without a parent and unreachable from the Document root). Previously
    /// represented as an Element with a `"#document-fragment"` pseudo-tag, it
    /// now has the dedicated [`NodeData::DocumentFragment`] variant:
    /// - It has `NodeKind::DocumentFragment`, not Element, so the Element gate
    ///   automatically skips it during CSS selection and cascade.
    /// - `tag_name()` returns `None`, eliminating pseudo-tag pollution.
    /// - `is_in_document()` is false: after `flags_dirty=true`, step 1 of
    ///   `mark_in_document_flags` clears the bit, and step 2 cannot reach it.
    ///
    /// # html5ever integration
    ///
    /// When `ElementFlags::template` passed to html5ever's
    /// `TreeSink::create_element` is true, the sink calls this method to create
    /// a fragment root and store it in the template element's `template_contents`
    /// slot. Thereafter, `TreeSink::get_template_contents` returns the fragment
    /// root index, and html5ever appends template contents as its children
    /// (the template element's own children remain empty).
    ///
    /// Equivalent to `create_template_contents` in blitz's
    /// `blitz-dom::html_sink::HtmlSink::create_element`.
    ///
    /// # Panics
    ///
    /// - `template_id` is not an Element (release and debug). Wiring a fragment
    ///   root to a non-template node is a caller bug, so it fails early.
    /// - The Element at `template_id` has a `tag_name` other than `"template"`
    ///   (release and debug). In html5ever,
    ///   [`ElementFlags::template`](https://docs.rs/markup5ever/latest/markup5ever/interface/tree_builder/struct.ElementFlags.html#structfield.template)
    ///   is true only for an HTML namespace `<template>` element. This entry
    ///   point therefore accepts only template elements; passing an ordinary
    ///   element is a caller bug.
    /// - The `template_contents` slot of `template_id` is already populated
    ///   (debug only). The sink must call this method once per template element;
    ///   a second call silently orphans the old fragment root, so debug builds
    ///   fail. Release builds allow overwriting for a future mutation runtime.
    ///
    /// Returns: the arena index of the newly allocated fragment root.
    pub fn allocate_template_fragment_root(&mut self, template_id: usize) -> usize {
        // Precondition: template_id is an Element with tag_name == "template"
        // and an unpopulated template_contents slot. Validate immutably first
        // to avoid a borrow conflict with the later &mut self operation.
        {
            let data = match &self.nodes[template_id].data {
                NodeData::Element(e) => e.as_ref(),
                _ => panic!("allocate_template_fragment_root called on non-Element"),
            };
            assert_eq!(
                data.tag_name.as_str(),
                "template",
                "allocate_template_fragment_root called on non-<template> element (tag = {:?})",
                data.tag_name.as_str(),
            );
            debug_assert!(
                data.template_contents.is_none(),
                "allocate_template_fragment_root called twice on the same template \
                 (would orphan the previous fragment root at arena index {:?})",
                data.template_contents,
            );
        }
        // Step 1: Allocate the fragment root as a detached DocumentFragment,
        // replacing the old append_element(None, "#document-fragment", ...)
        // pseudo-tag. Setting flags_dirty explicitly lets the later
        // mark_in_document_flags clear the default IS_IN_DOCUMENT bit in step 1.
        let frag_root = self.nodes.len();
        self.nodes.push(Node::new_document_fragment());
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        // Step 2: Store the fragment root index in the template element's
        // template_contents slot. The precondition check already validated the
        // element and tag_name, so no second validation is needed.
        let e = self.nodes[template_id]
            .data
            .as_element_mut()
            .expect("allocate_template_fragment_root: element vanished between checks");
        e.template_contents = Some(frag_root);
        frag_root
    }

    /// Update an Element node's `inline_style` after creation.
    /// During `finish()`, the sink extracts `style="..."` from its metadata
    /// table and calls this method. The raw string is sufficient; the Element
    /// trait implementation ([`raikiri_traits::Element::inline_style_source`])
    /// normalizes an empty `style=""` to `None`. The storage layer does not
    /// repeat that normalization.
    ///
    /// Panics (debug and release): if `id` is not an Element.
    pub fn set_element_inline_style(&mut self, id: usize, inline_style: Option<SmolStr>) {
        self.page_projection.clear();
        let e = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_inline_style called on non-Element");
        e.inline_style = inline_style;
    }

    /// Update an Element's sampled animation declarations without changing
    /// its authored `style` attribute.
    ///
    /// Passing `None` clears the animation-style source. The runtime that
    /// samples animations is responsible for marking style/layout dirty.
    ///
    /// Panics (debug and release): if `id` is not an Element.
    pub fn set_element_animation_style(&mut self, id: usize, animation_style: Option<SmolStr>) {
        self.page_projection.clear();
        let element = self.nodes[id]
            .data
            .as_element_mut()
            .expect("set_element_animation_style called on non-Element");
        element.animation_style = animation_style;
    }

    /// Apply a predicate to every node's children Vec and remove entries for
    /// which it returns `false`. This generic bulk-detach primitive avoids N
    /// calls to `detach_from_parent` at O(K*N), preventing quadratic work on
    /// large, attacker-controlled mutations. It also calls
    /// `invalidate_layout_cache` because it mutates the tree.
    ///
    /// **Historical note**: The old raikiri-html sink used
    /// `strip_non_element_stubs` to remove Comment / PI stubs represented as
    /// unimplemented Elements in bulk. That use disappeared when Comment and
    /// ProcessingInstruction became permanent [`NodeData`] variants in the
    /// tree and `mark_in_document_flags` began clearing IS_IN_DOCUMENT for them.
    /// This helper remains for a future mutation runtime and consumers that
    /// build trees directly. It sets [`flags_dirty`](Self#structfield.flags_dirty)
    /// to `true` on topology changes, like the other mutation primitives.
    pub fn retain_children(&mut self, mut predicate: impl FnMut(usize) -> bool) {
        let mut any_removed = false;
        for parent_id in 0..self.nodes.len() {
            let children = std::mem::take(&mut self.nodes[parent_id].children);
            let mut kept = Vec::with_capacity(children.len());
            for child in children {
                if predicate(child) {
                    kept.push(child);
                } else {
                    any_removed = true;
                    if let Some(node) = self.nodes.get_mut(child)
                        && node.parent == Some(parent_id)
                    {
                        node.parent = None;
                    }
                }
            }
            self.nodes[parent_id].children = kept;
        }
        if any_removed {
            self.invalidate_layout_cache();
            self.flags_dirty = true;
        }
    }

    /// Recompute the flat tree membership bit (`IS_IN_DOCUMENT`) for all arena
    /// nodes when the dirty flag is set, including Comment and PI nodes.
    /// Called by sink.finish() and after mutation batches.
    ///
    /// **Nodes whose bits are cleared** (postcondition):
    /// - Nodes unreachable from the Document root (detached / unreachable).
    ///   This includes a `<template>` element's contents: the parser stores
    ///   them under a detached `NodeData::DocumentFragment` root (see
    ///   [`Document::allocate_template_fragment_root`]), never as children of
    ///   the `<template>` element itself, so the DFS below cannot reach them.
    /// - `NodeData::Comment` / `NodeData::ProcessingInstruction` variant
    ///   (**cleared even when reachable**: unrendered kinds are consistently
    ///   skipped during flat tree rendering traversal).
    ///
    /// Ordinary light-DOM children appended directly under a `<template>`
    /// element (for example via DOM `appendChild`) stay set: per HTML §4.12.3
    /// the template contents fragment is a separate node rather than the
    /// element's children, so those children are in the document tree like
    /// any other reachable node. Only the associated contents fragment is
    /// inert.
    ///
    /// `NodeData::DocumentFragment` is usually detached, so the DFS in step 2
    /// cannot reach it and its bit remains cleared from step 1. No kind-based
    /// clearing is needed.
    ///
    /// Algorithm:
    /// 1. Clear every arena node's bit first, so detached or unreachable nodes
    ///    do not retain the default value of true.
    /// 2. Set bits with an iterative DFS from the Document root.
    ///    Do not set Comment / PI bits even when reachable (kind gate).
    ///
    /// Detailed implementation contracts:
    /// - During foster parenting, `Document::retain_children` or
    ///   `detach_from_parent` can remove only an arena child pointer, leaving a
    ///   node transiently detached. Step 1 clears every node, so such a node
    ///   cannot retain in_document=true.
    /// - An iterative Vec stack avoids stack overflow on deep DOM trees.
    /// - This method changes Taffy's effective child tree, which `TaffyChildIter`
    ///   filters with `is_in_document()`. It also invalidates the layout cache
    ///   afterward; otherwise the next `compute_child_layout` could reuse a
    ///   cached result with stale child ordering.
    /// - When `flags_dirty` is false, this method is an idempotent O(1) no-op.
    ///   Mutation primitives set the dirty flag, so observation code can call
    ///   this method each time with amortized overhead. Consumers must follow
    ///   the "mutation batch → mark → observation" contract to avoid regressions
    ///   if a primitive ever fails to update the flag.
    pub fn mark_in_document_flags(&mut self) {
        const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
        // Return early when clean. Immediately after parse.finish() the flag
        // is dirty and triggers recomputation; repeated calls without a later
        // mutation do not recompute.
        if !self.flags_dirty {
            return;
        }
        self.flags_dirty = false;
        // Step 1: Clear every arena node's bit. Node::new_* constructors set
        // true by default as an optimistic "attached" value; clearing here
        // makes detached and unreachable nodes false.
        for node in &mut self.nodes {
            node.set_in_document(false);
            node.set_inline_svg_content(false);
            node.set_inline_svg_root(false);
        }
        // Step 2: Set bits for nodes reachable from the Document root by DFS.
        //
        // Comment /
        // ProcessingInstruction nodes are unrendered in the flat tree, so keep
        // their `IS_IN_DOCUMENT` bits cleared even when reachable. The
        // TaffyChildIter is_in_document filter then skips them automatically;
        // cascade, paint, and stylesheet extraction use the same filter and
        // likewise avoid Comment/PI (the Node::is_in_document contract in
        // crates/raikiri-traits/src/dom.rs avoids scattered kind gates).
        // DocumentFragment is detached and unreachable by DFS, so its bit stays
        // cleared from step 1 without extra handling.
        let root = self.root_index();
        let mut stack: Vec<(usize, bool)> = vec![(root, false)];
        while let Some((id, in_svg_subtree)) = stack.pop() {
            let node = &mut self.nodes[id];
            let is_unrendered_by_kind = matches!(
                node.data,
                NodeData::Comment(_) | NodeData::ProcessingInstruction { .. }
            );
            node.set_in_document(!is_unrendered_by_kind);
            let is_svg_element = matches!(
                &node.data,
                NodeData::Element(element)
                    if element.tag_name.as_str() == "svg"
                        && element.namespace.as_deref() == Some(SVG_NAMESPACE)
            );
            let svg_subtree_here = in_svg_subtree || is_svg_element;
            node.set_inline_svg_content(svg_subtree_here);
            node.set_inline_svg_root(is_svg_element && !in_svg_subtree);
            // Borrow the child IDs directly to avoid a temporary Vec per parent.
            stack.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&child| (child, svg_subtree_here)),
            );
        }
        // Step 3: Mark the layout cache dirty because Taffy's effective child
        // tree may have changed.
        self.invalidate_layout_cache();
    }

    /// Borrow the node at arena index `id`, or return `None` if out of range.
    ///
    /// Equivalent to blitz-dom's `BaseDocument::get_node`. This wraps O(1)
    /// `Vec::get` for the hot path raikiri-paint calls per node while walking.
    pub fn get_node(&self, id: usize) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Return an element's namespace URI, treating an omitted HTML namespace as XHTML.
    pub fn element_namespace_uri(&self, id: usize) -> Option<&str> {
        let NodeData::Element(element) = &self.nodes.get(id)?.data else {
            return None;
        };
        Some(element.namespace.as_deref().unwrap_or(XHTML_NAMESPACE_URI))
    }

    /// Read a null-namespace attribute, applying HTML's ASCII-case-insensitive
    /// lookup rule to HTML-namespace elements only.
    pub fn element_attribute(&self, id: usize, name: &str) -> Option<&str> {
        let node = self.nodes.get(id)?;
        let NodeData::Element(element) = &node.data else {
            return None;
        };
        if element
            .namespace
            .as_deref()
            .is_none_or(|namespace| namespace == XHTML_NAMESPACE_URI)
            && name.bytes().any(|byte| byte.is_ascii_uppercase())
        {
            return node.attribute(&name.to_ascii_lowercase());
        }
        node.attribute(name)
    }

    /// Read a namespace-qualified attribute by namespace URI and local name.
    ///
    /// DOM `getAttributeNS` semantics: matching is exact, with no ASCII case
    /// folding, on both foreign and HTML-namespace elements. For example SVG
    /// `xlink:href` is `namespace = "http://www.w3.org/1999/xlink"`,
    /// `local = "href"`. Returns `None` for out-of-range ids, non-elements,
    /// and absent attributes. Renderer-neutral: the value is a plain string
    /// slice; see [`crate::node::Node::attribute_ns`] for the node-level
    /// accessor and [`Document::serialize_svg_subtree`] for whole-subtree XML
    /// source reconstruction.
    pub fn element_attribute_ns(&self, id: usize, namespace: &str, local: &str) -> Option<&str> {
        let node = self.nodes.get(id)?;
        node.attribute_ns(namespace, local)
    }

    /// Whether `id` is an HTML `<canvas>` element (HTML Standard §4.12.5).
    ///
    /// Matches the HTML namespace (the parser's `None` default and the
    /// explicit XHTML URI) with an ASCII case-insensitive tag comparison.
    /// Foreign-namespace `canvas` elements are not canvases.
    pub fn is_canvas_element(&self, id: usize) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        let NodeData::Element(element) = &node.data else {
            return false;
        };
        if !element.tag_name.eq_ignore_ascii_case("canvas") {
            return false;
        }
        element
            .namespace
            .as_deref()
            .is_none_or(|ns| ns == XHTML_NAMESPACE_URI)
    }

    /// Parse one canvas width/height content attribute (HTML Standard §4.12.5).
    ///
    /// Uses the rules for parsing non-negative integers: surrounding ASCII
    /// whitespace is ignored, an optional leading `+` is stripped, and the
    /// remainder must be ASCII digits. Returns `None` when parsing fails so
    /// the caller falls back to the default (300 for width, 150 for height).
    fn parse_canvas_dimension(value: &str) -> Option<u32> {
        let trimmed = value.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r'));
        let digits = trimmed.strip_prefix('+').unwrap_or(trimmed);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Strip leading zeros to keep `u32` parsing bounded; overlong digit
        // strings saturate rather than wrap.
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Some(0);
        }
        if digits.len() > 10 {
            return Some(u32::MAX);
        }
        digits
            .parse::<u64>()
            .ok()
            .map(|v| v.min(u64::from(u32::MAX)) as u32)
    }

    /// Current width/height of an HTML `<canvas>` element (HTML Standard §4.12.5).
    ///
    /// Missing or unparsable attributes fall back to 300×150. Returns `None`
    /// for non-canvas nodes.
    pub fn canvas_size(&self, id: usize) -> Option<(u32, u32)> {
        if !self.is_canvas_element(id) {
            return None;
        }
        let node = self.nodes.get(id)?;
        let width = node
            .attribute("width")
            .and_then(Self::parse_canvas_dimension)
            .unwrap_or(300);
        let height = node
            .attribute("height")
            .and_then(Self::parse_canvas_dimension)
            .unwrap_or(150);
        Some((width, height))
    }

    /// Borrow a canvas bitmap, if its pixels have been materialized.
    pub fn canvas_bitmap_ref(&self, id: usize) -> Option<&crate::node::CanvasBitmap> {
        let node = self.nodes.get(id)?;
        let NodeData::Element(element) = &node.data else {
            return None;
        };
        element.canvas_bitmap.as_ref()
    }

    /// Clone a canvas bitmap, if its pixels have been materialized.
    pub fn canvas_bitmap(&self, id: usize) -> Option<crate::node::CanvasBitmap> {
        self.canvas_bitmap_ref(id).cloned()
    }

    fn canvas_bytes_after_replacement(
        &mut self,
        id: usize,
        replacement_capacity: usize,
    ) -> Result<usize, CanvasBitmapError> {
        let document_capacity = if let Some(bytes) = self.canvas_bitmap_bytes.0 {
            bytes
        } else {
            let bytes = self
                .nodes
                .iter()
                .try_fold(0usize, |total, node| {
                    let capacity = match &node.data {
                        NodeData::Element(element) => element
                            .canvas_bitmap
                            .as_ref()
                            .map_or(0, |bitmap| bitmap.rgba.capacity()),
                        _ => 0,
                    };
                    total.checked_add(capacity)
                })
                .ok_or(CanvasBitmapError::DocumentLimitExceeded)?;
            self.canvas_bitmap_bytes.0 = Some(bytes);
            bytes
        };
        let old_capacity = self
            .canvas_bitmap_ref(id)
            .map_or(0, |bitmap| bitmap.rgba.capacity());
        let retained_bytes = document_capacity
            .checked_sub(old_capacity)
            .ok_or(CanvasBitmapError::InvalidRgbaLength)?;
        let total_bytes = retained_bytes
            .checked_add(replacement_capacity)
            .ok_or(CanvasBitmapError::DocumentLimitExceeded)?;
        if total_bytes > MAX_DOCUMENT_CANVAS_BITMAP_BYTES {
            return Err(CanvasBitmapError::DocumentLimitExceeded);
        }
        Ok(total_bytes)
    }

    /// Store a canvas bitmap, replacing any previous one.
    ///
    /// Bitmap storage never affects layout: the intrinsic size comes from
    /// the width/height attributes (see [`crate::image_resolve`]), not from
    /// this bitmap. Callers that changed the size must have updated the
    /// attributes first (see [`Document::set_element_attribute`], which
    /// clears the bitmap automatically). Invalid or over-limit storage is
    /// ignored; use [`Document::try_set_canvas_bitmap`] to observe failure.
    pub fn set_canvas_bitmap(&mut self, id: usize, bitmap: crate::node::CanvasBitmap) {
        let _ = self.try_set_canvas_bitmap(id, bitmap);
    }

    /// Try to store a bitmap matching the canvas's current dimensions, within
    /// the per-canvas and aggregate document memory limits.
    pub fn try_set_canvas_bitmap(
        &mut self,
        id: usize,
        bitmap: CanvasBitmap,
    ) -> Result<(), CanvasBitmapError> {
        let size = self.canvas_size(id).ok_or(CanvasBitmapError::NotCanvas)?;
        if size != (bitmap.width, bitmap.height) {
            return Err(CanvasBitmapError::SizeMismatch);
        }
        if !bitmap.rgba.is_empty() {
            let expected = CanvasBitmap::checked_rgba_len(bitmap.width, bitmap.height)?;
            if bitmap.rgba.len() != expected {
                return Err(CanvasBitmapError::InvalidRgbaLength);
            }
        }
        let total_bytes = self.canvas_bytes_after_replacement(id, bitmap.rgba.capacity())?;
        // cov:ignore: canvas_size above validated this live canvas slot, and no arena mutation occurs before this borrow.
        let Some(node) = self.nodes.get_mut(id) else {
            return Err(CanvasBitmapError::NotCanvas);
        };
        // cov:ignore: canvas_size above proved this slot is a canvas Element; no arena mutation occurs before this check.
        let NodeData::Element(element) = &mut node.data else {
            return Err(CanvasBitmapError::NotCanvas);
        };
        element.canvas_bitmap = Some(bitmap);
        self.canvas_bitmap_bytes.0 = Some(total_bytes);
        Ok(())
    }

    fn clear_canvas_bitmap_storage(&mut self, id: usize) {
        let _ = self.take_canvas_bitmap_storage(id);
    }

    fn take_canvas_bitmap_storage(&mut self, id: usize) -> Option<CanvasBitmap> {
        let bitmap = self.nodes.get_mut(id).and_then(|node| {
            let NodeData::Element(element) = &mut node.data else {
                return None;
            };
            element.canvas_bitmap.take()
        });
        if let Some(bitmap) = &bitmap
            && let Some(document_capacity) = self.canvas_bitmap_bytes.0
        {
            self.canvas_bitmap_bytes.0 =
                Some(document_capacity.saturating_sub(bitmap.rgba.capacity()));
        }
        bitmap
    }

    fn try_create_canvas_bitmap(&mut self, id: usize) -> Result<(), CanvasBitmapError> {
        let (width, height) = self.canvas_size(id).ok_or(CanvasBitmapError::NotCanvas)?;
        let replacement_bytes = CanvasBitmap::checked_rgba_len(width, height)?;
        self.canvas_bytes_after_replacement(id, replacement_bytes)?;
        let bitmap = CanvasBitmap::try_cleared(width, height)?;
        self.try_set_canvas_bitmap(id, bitmap)
    }

    fn reset_canvas_bitmap_for_current_size(&mut self, id: usize) {
        self.clear_canvas_bitmap_storage(id);
        let _ = self.try_create_canvas_bitmap(id);
    }

    /// Ensure a bitmap matching the current width/height attributes exists,
    /// creating a transparent-black one when missing or size-mismatched.
    ///
    /// Returns the current size, `None` for non-canvas nodes, or an error if
    /// the bitmap cannot be materialized within resource limits.
    pub fn try_ensure_canvas_bitmap(
        &mut self,
        id: usize,
    ) -> Result<Option<(u32, u32)>, CanvasBitmapError> {
        let Some((width, height)) = self.canvas_size(id) else {
            return Ok(None);
        };
        let expected_bytes = match CanvasBitmap::checked_rgba_len(width, height) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.clear_canvas_bitmap_storage(id);
                return Err(error);
            }
        };
        if self.canvas_bitmap_ref(id).is_some_and(|bitmap| {
            bitmap.width == width && bitmap.height == height && bitmap.rgba.len() == expected_bytes
        }) {
            return Ok(Some((width, height)));
        }
        self.clear_canvas_bitmap_storage(id);
        self.try_create_canvas_bitmap(id)?;
        Ok(Some((width, height)))
    }

    /// Ensure a bitmap exists, returning `None` for non-canvas nodes or when
    /// resource limits prevent materialization. Use
    /// [`Document::try_ensure_canvas_bitmap`] to distinguish those cases.
    pub fn ensure_canvas_bitmap(&mut self, id: usize) -> Option<(u32, u32)> {
        self.try_ensure_canvas_bitmap(id).ok().flatten()
    }

    /// Fill `x, y, w, h` (in bitmap px, clipped to the bitmap) with `rgba`.
    ///
    /// Opaque fills overwrite; translucent fills composite source-over
    /// against the existing pixels. Returns `false` for non-canvas nodes or
    /// when resource limits prevent bitmap materialization.
    pub fn canvas_fill_rect(
        &mut self,
        id: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        rgba: [u8; 4],
    ) -> bool {
        if !matches!(self.try_ensure_canvas_bitmap(id), Ok(Some(_))) {
            return false;
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return false; // cov:ignore: try_ensure_canvas_bitmap already validated the id, so get_mut cannot fail here
        };
        let NodeData::Element(element) = &mut node.data else {
            return false; // cov:ignore: try_ensure_canvas_bitmap only succeeds for canvas elements, so this is always an Element here
        };
        let Some(bitmap) = element.canvas_bitmap.as_mut() else {
            return false; // cov:ignore: try_ensure_canvas_bitmap creates the bitmap when missing, so it is always present here
        };
        if bitmap.width == 0 || bitmap.height == 0 {
            return true;
        }
        let bw = bitmap.width as i64;
        let bh = bitmap.height as i64;
        let x0 = (i64::from(x)).max(0).min(bw) as u32;
        let y0 = (i64::from(y)).max(0).min(bh) as u32;
        let x1 = (i64::from(x) + i64::from(w)).max(0).min(bw) as u32;
        let y1 = (i64::from(y) + i64::from(h)).max(0).min(bh) as u32;
        if x0 >= x1 || y0 >= y1 {
            return true;
        }
        let (sr, sg, sb, sa) = (
            u16::from(rgba[0]),
            u16::from(rgba[1]),
            u16::from(rgba[2]),
            u16::from(rgba[3]),
        );
        if sa == 255 {
            for row in y0..y1 {
                let base = (row * bitmap.width + x0) as usize * 4;
                let count = (x1 - x0) as usize;
                for i in 0..count {
                    let off = base + i * 4;
                    bitmap.rgba[off] = rgba[0];
                    bitmap.rgba[off + 1] = rgba[1];
                    bitmap.rgba[off + 2] = rgba[2];
                    bitmap.rgba[off + 3] = rgba[3];
                }
            }
            return true;
        }
        if sa == 0 {
            return true;
        }
        let inv = 255 - sa;
        for row in y0..y1 {
            for col in x0..x1 {
                let off = ((row * bitmap.width + col) as usize) * 4;
                let dr = u16::from(bitmap.rgba[off]);
                let dg = u16::from(bitmap.rgba[off + 1]);
                let db = u16::from(bitmap.rgba[off + 2]);
                let da = u16::from(bitmap.rgba[off + 3]);
                // Source-over with non-premultiplied bytes, rounded.
                bitmap.rgba[off] = ((sr * sa + dr * inv + 127) / 255).min(255) as u8;
                bitmap.rgba[off + 1] = ((sg * sa + dg * inv + 127) / 255).min(255) as u8;
                bitmap.rgba[off + 2] = ((sb * sa + db * inv + 127) / 255).min(255) as u8;
                bitmap.rgba[off + 3] = ((sa * 255 + da * inv + 127) / 255).min(255) as u8;
            }
        }
        true
    }

    /// Clear `x, y, w, h` (in bitmap px, clipped) to transparent black.
    ///
    /// Unlike [`Document::canvas_fill_rect`] with a transparent color (which
    /// composites nothing), this overwrites the pixels outright per HTML
    /// Standard §4.12.5 `clearRect`. Returns `false` for non-canvas nodes or
    /// when resource limits prevent bitmap materialization.
    pub fn canvas_clear_rect(&mut self, id: usize, x: i32, y: i32, w: i32, h: i32) -> bool {
        if !matches!(self.try_ensure_canvas_bitmap(id), Ok(Some(_))) {
            return false;
        }
        let Some(node) = self.nodes.get_mut(id) else {
            return false; // cov:ignore: try_ensure_canvas_bitmap already validated the id, so get_mut cannot fail here
        };
        let NodeData::Element(element) = &mut node.data else {
            return false; // cov:ignore: try_ensure_canvas_bitmap only succeeds for canvas elements, so this is always an Element here
        };
        let Some(bitmap) = element.canvas_bitmap.as_mut() else {
            return false; // cov:ignore: try_ensure_canvas_bitmap creates the bitmap when missing, so it is always present here
        };
        if bitmap.width == 0 || bitmap.height == 0 {
            return true;
        }
        let bw = bitmap.width as i64;
        let bh = bitmap.height as i64;
        let x0 = (i64::from(x)).max(0).min(bw) as u32;
        let y0 = (i64::from(y)).max(0).min(bh) as u32;
        let x1 = (i64::from(x) + i64::from(w)).max(0).min(bw) as u32;
        let y1 = (i64::from(y) + i64::from(h)).max(0).min(bh) as u32;
        if x0 >= x1 || y0 >= y1 {
            return true;
        }
        for row in y0..y1 {
            let base = (row * bitmap.width + x0) as usize * 4;
            let count = (x1 - x0) as usize;
            for i in 0..count {
                let off = base + i * 4;
                bitmap.rgba[off] = 0;
                bitmap.rgba[off + 1] = 0;
                bitmap.rgba[off + 2] = 0;
                bitmap.rgba[off + 3] = 0;
            }
        }
        true
    }

    /// Clone materialized canvas bitmaps in tree order.
    ///
    /// Canvas bitmaps are not part of `innerHTML`, so a harness can collect
    /// them here and restore them onto a reparsed document in the same order.
    /// Unmaterialized transparent canvases carry only their dimensions.
    pub fn canvases_in_tree_order(&self) -> Vec<crate::node::CanvasBitmap> {
        let mut out = Vec::new();
        let mut sidecar_bytes = 0usize;
        let mut stack: Vec<usize> = self
            .nodes
            .get(self.root_index())
            .map(|root| root.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue; // cov:ignore: Document arena only appends and children always hold valid indices, so traversal from root never misses.
            };
            if self.is_canvas_element(index) {
                let (width, height) = self.canvas_size(index).unwrap_or((300, 150));
                let bitmap = if let Some(bitmap) = self.canvas_bitmap_ref(index) {
                    if sidecar_bytes
                        .checked_add(bitmap.rgba.len())
                        .is_some_and(|total| total <= MAX_DOCUMENT_CANVAS_BITMAP_BYTES)
                    {
                        sidecar_bytes += bitmap.rgba.len();
                        bitmap.clone()
                    } else {
                        CanvasBitmap::transparent(width, height)
                    }
                } else {
                    CanvasBitmap::transparent(width, height)
                };
                out.push(bitmap);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }

    /// Remove and return materialized canvas bitmaps in tree order.
    ///
    /// This transfers pixel storage without cloning it. It is intended for
    /// consumers that serialize a live document and then discard or replace
    /// it, such as the WPT reftest paint path.
    pub fn take_canvases_in_tree_order(&mut self) -> Vec<crate::node::CanvasBitmap> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self
            .nodes
            .get(self.root_index())
            .map(|root| root.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue; // cov:ignore: Document arena only appends and children always hold valid indices, so traversal from root never misses.
            };
            stack.extend(node.children.iter().rev().copied());
            if self.is_canvas_element(index) {
                let (width, height) = self.canvas_size(index).unwrap_or((300, 150));
                out.push(
                    self.take_canvas_bitmap_storage(index)
                        .unwrap_or_else(|| CanvasBitmap::transparent(width, height)),
                );
            }
        }
        out
    }

    /// Restore cloned bitmaps collected by [`Document::canvases_in_tree_order`].
    ///
    /// Extra bitmaps are ignored; missing ones leave the parsed canvas blank
    /// (transparent). Prefer [`Document::set_canvases_in_tree_order_owned`]
    /// when the caller owns the sidecar and can transfer its pixel storage.
    pub fn set_canvases_in_tree_order(&mut self, bitmaps: &[crate::node::CanvasBitmap]) {
        self.set_canvases_in_tree_order_owned(bitmaps.to_vec());
    }

    /// Restore canvas bitmaps in tree order by moving their pixel storage.
    ///
    /// Extra bitmaps are ignored; missing ones leave the parsed canvas blank
    /// (transparent). Consumers that own the sidecar should prefer this over
    /// [`Document::set_canvases_in_tree_order`] to avoid copying pixel buffers.
    pub fn set_canvases_in_tree_order_owned(&mut self, bitmaps: Vec<crate::node::CanvasBitmap>) {
        let mut ids = Vec::new();
        let mut stack: Vec<usize> = self
            .nodes
            .get(self.root_index())
            .map(|root| root.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue; // cov:ignore: Document arena only appends and children always hold valid indices, so traversal from root never misses.
            };
            if self.is_canvas_element(index) {
                ids.push(index);
            }
            stack.extend(node.children.iter().rev().copied());
        }
        for (id, bitmap) in ids.into_iter().zip(bitmaps) {
            let _ = self.try_set_canvas_bitmap(id, bitmap);
        }
    }

    /// Serialize an inline SVG element and its subtree as a standalone XML
    /// source, retaining element/attribute namespace URIs and prefixes.
    pub fn serialize_svg_subtree(&self, id: usize) -> Result<Option<String>, String> {
        const SVG_NS: &str = "http://www.w3.org/2000/svg";
        const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
        const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

        enum Task {
            Node(usize),
            End(String),
        }

        let Some(root) = self.nodes.get(id) else {
            return Ok(None);
        };
        let NodeData::Element(root_element) = &root.data else {
            return Ok(None);
        };
        if root_element.tag_name.as_str() != "svg"
            || root_element.namespace.as_deref() != Some(SVG_NS)
        {
            return Ok(None);
        }

        let mut output = String::new();
        let mut pending = vec![Task::Node(id)];
        while let Some(task) = pending.pop() {
            match task {
                Task::End(name) => {
                    output.push_str("</");
                    output.push_str(&name);
                    output.push('>');
                }
                Task::Node(node_id) => {
                    let Some(node) = self.nodes.get(node_id) else {
                        return Err(format!("SVG subtree node {node_id} is out of range"));
                    };
                    match &node.data {
                        NodeData::Element(element) => {
                            let name = qualified_name(
                                element.prefix.as_deref(),
                                element.tag_name.as_str(),
                            );
                            if !is_valid_xml_name(element.tag_name.as_str())
                                || element
                                    .prefix
                                    .as_deref()
                                    .is_some_and(|prefix| !is_valid_xml_name(prefix))
                            {
                                return Err(format!("invalid SVG element name on node {node_id}"));
                            }
                            output.push('<');
                            output.push_str(&name);
                            let mut namespace_bindings = HashMap::new();
                            if let Some(namespace) = element.namespace.as_deref() {
                                let prefix = element.prefix.as_deref().unwrap_or("");
                                namespace_bindings.insert(prefix.to_owned(), namespace.to_owned());
                                let declaration = element.prefix.as_deref().map_or_else(
                                    || "xmlns".to_owned(),
                                    |prefix| format!("xmlns:{prefix}"),
                                );
                                output.push(' ');
                                output.push_str(&declaration);
                                output.push_str("=\"");
                                push_xml_escaped(&mut output, namespace, true);
                                output.push('"');
                            } else {
                                output.push_str(" xmlns=\"\"");
                            }

                            let mut generated_prefix = 0usize;
                            for attribute in &element.attributes {
                                if attribute.namespace.as_deref() == Some(XMLNS_NS)
                                    || (attribute.namespace.is_none()
                                        && attribute.local.as_str() == "style")
                                {
                                    continue;
                                }
                                if !is_valid_xml_name(attribute.local.as_str()) {
                                    return Err(format!(
                                        "invalid SVG attribute name {:?} on node {node_id}",
                                        attribute.local
                                    ));
                                }
                                let attribute_name = match attribute.namespace.as_deref() {
                                    None => attribute.local.to_string(),
                                    Some(XML_NS) => format!("xml:{}", attribute.local),
                                    Some(namespace) => {
                                        let source_prefix = attribute.prefix.as_deref();
                                        let prefix = match source_prefix {
                                            Some(prefix)
                                                if namespace_bindings
                                                    .get(prefix)
                                                    .is_none_or(|bound| bound == namespace) =>
                                            {
                                                prefix.to_owned()
                                            }
                                            _ => loop {
                                                generated_prefix += 1;
                                                let candidate =
                                                    format!("_raikiri_ns{generated_prefix}");
                                                if !namespace_bindings.contains_key(&candidate) {
                                                    break candidate;
                                                }
                                            },
                                        };
                                        if !is_valid_xml_name(&prefix) {
                                            return Err(format!(
                                                "invalid SVG attribute prefix {prefix:?} on node {node_id}"
                                            ));
                                        }
                                        if !namespace_bindings.contains_key(&prefix) {
                                            namespace_bindings
                                                .insert(prefix.clone(), namespace.to_owned());
                                            let declaration = format!("xmlns:{prefix}");
                                            output.push(' ');
                                            output.push_str(&declaration);
                                            output.push_str("=\"");
                                            push_xml_escaped(&mut output, namespace, true);
                                            output.push('"');
                                        }
                                        format!("{prefix}:{}", attribute.local)
                                    }
                                };
                                output.push(' ');
                                output.push_str(&attribute_name);
                                output.push_str("=\"");
                                push_xml_escaped(&mut output, attribute.value.as_str(), true);
                                output.push('"');
                            }
                            if let Some(style) = &element.inline_style {
                                output.push_str(" style=\"");
                                push_xml_escaped(&mut output, style.as_str(), true);
                                output.push('"');
                            }
                            output.push('>');
                            pending.push(Task::End(name));
                            pending.extend(node.children.iter().rev().copied().map(Task::Node));
                        }
                        NodeData::Text(text) => {
                            push_xml_escaped(&mut output, text.text_content.as_str(), false);
                        }
                        NodeData::Document | NodeData::Comment(_) => {}
                        NodeData::ProcessingInstruction { .. } | NodeData::DocumentFragment => {}
                    }
                }
            }
        }
        Ok(Some(output))
    }

    /// Return the post-computed Taffy style for a node.
    ///
    /// Layout mutates this style with finite used `ch` lengths before Taffy
    /// runs. Paint-side consumers can therefore observe the same used values
    /// without re-probing fonts or falling back to `ComputedValues`.
    pub fn layout_style(&self, id: usize) -> Option<&Style> {
        self.nodes.get(id).map(|node| &node.style)
    }

    /// Total number of nodes in the arena, including the Document root.
    ///
    /// Used by raikiri-paint and other callers to detect violations of
    /// `cascade.computed.len() == doc.node_count()` early, and by doctests and
    /// smoke tests.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Arena index of the Document root (always 0).
    ///
    /// Inherent counterpart of the `Dom::root_id()` trait method; callable
    /// directly on `&Document` without importing the trait. The `root_index`
    /// name distinguishes its `usize` return type from the trait method's
    /// `NodeId` newtype.
    pub fn root_index(&self) -> usize {
        self.root
    }

    /// Mark the layout cache dirty after a tree mutation. The next
    /// `compute_child_layout` (via taffy_impl) clears it lazily. Each mutation
    /// costs O(1); each layout batch incurs amortized O(N) clearing.
    pub(crate) fn invalidate_layout_cache(&mut self) {
        self.page_projection.clear();
        self.layout_dirty = true;
    }

    // ─── stylesheets ───────────────────

    /// Associate a stylesheet with the Document.
    ///
    /// - Parsing is deferred and batched in the cascade phase.
    /// - Call order determines cascade source_order within the same `kind`.
    /// - `Cow<'static, str>` retains static &str values such as bundled UA CSS
    ///   by borrowing them, without allocation. A consumer-provided `String`
    ///   is stored as Cow::Owned.
    pub fn add_stylesheet(&mut self, source: impl Into<Cow<'static, str>>, kind: StylesheetKind) {
        self.stylesheets.push((source.into(), kind));
    }

    /// Iterate over all currently associated stylesheets as `(source, kind)`
    /// tuples, in `add_stylesheet` call order.
    ///
    /// Intended for the cascade orchestrator (raikiri umbrella) when it builds
    /// the RuleTree.
    pub fn stylesheets(&self) -> impl Iterator<Item = (&str, StylesheetKind)> + '_ {
        self.stylesheets
            .iter()
            .map(|(cow, kind)| (cow.as_ref(), *kind))
    }

    // ─── quirks mode ───────────────────

    /// Set this Document's HTML5 quirks mode. During `finish()`, the raikiri-html
    /// parse sink writes the value received from html5ever's
    /// `TreeSink::set_quirks_mode` callback here (see `RaikiriTreeSink::finish`).
    pub fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.quirks_mode = mode;
    }

    /// This Document's HTML5 quirks mode. A manually constructed `Document`
    /// (in test or setup code that does not parse HTML) retains
    /// [`QuirksMode::NoQuirks`]. The `quirks_mode()` override in
    /// `impl raikiri_style::StyleDom for Document` converts this value to
    /// `StyleQuirksMode` and passes it to cascade (`dom_impl.rs`).
    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

impl Document {
    /// Prepare image markers before layout from already resolved pixels.
    /// A failed image keeps the text fallback; explicit `::marker` content wins.
    /// Retained pixel allocations share a 128 MiB document budget.
    pub fn prepare_list_marker_images(
        &mut self,
        cascade: &raikiri_style::CascadeResult,
        source: &dyn raikiri_traits::ImagePixelSource,
        base_url: Option<&url::Url>,
    ) {
        self.prepare_list_marker_images_with_budget(
            cascade,
            source,
            base_url,
            MAX_DOCUMENT_LIST_MARKER_IMAGE_BYTES,
        );
    }

    fn prepare_list_marker_images_with_budget(
        &mut self,
        cascade: &raikiri_style::CascadeResult,
        source: &dyn raikiri_traits::ImagePixelSource,
        base_url: Option<&url::Url>,
        mut remaining: u64,
    ) {
        self.list_marker_images.clear();
        let mut variants =
            HashMap::<(url::Url, u32, u32), Arc<raikiri_traits::DecodedImage>>::new();
        let mut allocations = std::collections::HashSet::new();
        for (element, cv) in cascade.computed.iter().enumerate() {
            if !crate::generated_content::marker_is_enabled(cascade, element)
                || cascade
                    .pseudo
                    .get(&(
                        raikiri_style::StyleNodeId::new(element as u64),
                        raikiri_style::PseudoElem::Marker,
                    ))
                    .is_some_and(|marker| !marker.content.is_empty())
            {
                continue;
            }
            let raikiri_style::property::BackgroundImage::Url(raw) = &cv.list_style_image else {
                continue;
            };
            let Some(url) = url::Url::parse(raw)
                .ok()
                .or_else(|| base_url.and_then(|base| base.join(raw).ok()))
            else {
                continue;
            };
            let Some(intrinsic) = source.intrinsic_size(&url) else {
                continue;
            };
            let em = cascade
                .pseudo
                .get(&(
                    raikiri_style::StyleNodeId::new(element as u64),
                    raikiri_style::PseudoElem::Marker,
                ))
                .unwrap_or(cv)
                .font_size
                .0;
            let size = marker_image_size(intrinsic, em);
            let key = (url.clone(), size.width.to_bits(), size.height.to_bits());
            let image = if let Some(image) = variants.get(&key) {
                Arc::clone(image)
            } else {
                let Some(image) = source.get_decoded_at_size(&url, size, Some(remaining)) else {
                    continue;
                };
                if image.width == 0 || image.height == 0 {
                    continue;
                }
                let allocation = Arc::as_ptr(&image);
                if !allocations.contains(&allocation) {
                    // Sources may ignore the requested limit or reserve extra capacity.
                    let bytes = image.rgba.capacity() as u64;
                    if bytes > remaining {
                        continue;
                    }
                    remaining -= bytes;
                    allocations.insert(allocation);
                }
                variants.insert(key, Arc::clone(&image));
                image
            };
            self.list_marker_images.insert(
                element,
                ListMarkerImage {
                    url,
                    pixels: image,
                    size,
                },
            );
        }
        self.layout_dirty = true;
    }

    /// Pixels of a marker prepared by [`Self::prepare_list_marker_images`].
    pub fn list_marker_image(&self, element: usize) -> Option<&raikiri_traits::DecodedImage> {
        self.list_marker_images
            .get(&element)
            .map(|marker| marker.pixels.as_ref())
    }

    /// Absolute source URL retained with a prepared marker's pixels.
    pub fn list_marker_image_url(&self, element: usize) -> Option<&url::Url> {
        self.list_marker_images
            .get(&element)
            .map(|marker| &marker.url)
    }

    /// Inline space reserved before legacy ruby/multicol list-item content.
    /// The painter subtracts it from the used padding to recover the marker origin.
    #[doc(hidden)]
    pub fn legacy_inside_marker_advance(&self, element: usize) -> f32 {
        self.legacy_inside_marker_advances
            .get(&element)
            .copied()
            .unwrap_or(0.0)
    }

    /// CSS dimensions of a prepared marker, independent of raster rounding.
    pub fn list_marker_image_size(
        &self,
        element: usize,
    ) -> Option<raikiri_traits::ImageRasterSize> {
        self.list_marker_images
            .get(&element)
            .map(|marker| marker.size)
    }
}

fn marker_image_size(
    intrinsic: raikiri_traits::ImageIntrinsicSize,
    em: f32,
) -> raikiri_traits::ImageRasterSize {
    let valid = |value: f32| value.is_finite() && value > 0.0;
    let width = intrinsic.width.filter(|value| valid(*value));
    let height = intrinsic.height.filter(|value| valid(*value));
    let ratio = intrinsic.aspect_ratio.filter(|value| valid(*value));
    let (width, height) = match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (None, None, Some(ratio)) => {
            let width = em.min(em * ratio);
            (width, width / ratio)
        }
        (Some(width), None, None) => (width, em),
        (None, Some(height), None) => (em, height),
        (None, None, None) => (em, em),
    };
    raikiri_traits::ImageRasterSize { width, height }
}

impl Document {
    /// Absolute image URL resolved during the current layout pass.
    ///
    /// Source attributes remain unchanged. Absolute source URLs also work for
    /// native callers that supplied pixels without an intrinsic-size resolver.
    pub fn resolved_image_url(&self, element: usize) -> Option<url::Url> {
        let node = self.get_node(element)?;
        if node.tag_name() != Some("img") {
            return None;
        }
        let raw = node.attribute("src")?;
        self.resolved_image_urls
            .get(&element)
            .filter(|(source, _)| source == raw)
            .map(|(_, url)| url.clone())
            .or_else(|| url::Url::parse(raw).ok())
    }
}
