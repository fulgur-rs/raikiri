//! Per-root state of the shodo inline engine: the built paragraph, the lines of
//! the last performed layout, and the document-level engine handles.

use super::boxes::IfcBox;
use super::first_letter::LetterStyle;
use super::projection::ProjectedIfc;
use raikiri_style::ComputedTextIndent;
use shodo::font::FontCollection;
use shodo::geometry::WritingMode;
use shodo::limits::Limits;
use shodo::style::LineOptions;
use shodo::{LayoutContext, Line, Paragraph};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

/// A block laid out as one shodo paragraph.
#[derive(Clone)]
pub(crate) struct IfcRoot {
    /// `Paragraph` is a cheap clone over shared data, so it is held directly.
    pub(crate) paragraph: Paragraph,
    /// Writing mode used to shape the paragraph.
    pub(crate) writing_mode: WritingMode,
    pub(crate) options: LineOptions,
    pub(crate) indent: ComputedTextIndent,
    /// Lines of the last performed layout, if any.
    pub(crate) lines: Option<IfcLines>,
    /// Children laid out as boxes of their own (floats and atomic inlines),
    /// in document order.
    pub(crate) boxes: Vec<IfcBox>,
    /// Fixed-size image marker independent of the DOM box arena.
    pub(crate) marker_atomic: Option<(shodo::node::NodeId, shodo::AtomicSize)>,
    /// The root's `direction` is `rtl`: its lines start at the right edge.
    pub(crate) rtl: bool,
    /// Paint offsets of the relatively positioned inline elements of the
    /// paragraph, by DOM node id.
    pub(crate) offsets: Vec<(usize, (f32, f32))>,
    /// `<br>` elements with a physical `clear`: the line after each starts
    /// below the floats it clears.
    pub(crate) cleared_breaks: Vec<(usize, taffy::Clear)>,
    /// The root is a fixed box: its containing block is the page area.
    pub(crate) fixed: bool,
    /// Line ranges of the last performed lines and where each range is drawn
    /// in the columns of a multicol container, when the lines are split
    /// across columns. `y` is the block offset the range's first line is
    /// drawn at.
    pub(crate) multicol_fragments: Option<Vec<crate::node::MulticolTextFragment>>,
    /// Raw root fragments already own the physical origin of each line range.
    pub(crate) multicol_fragment_origins_recorded: bool,
    /// The lines end in an ellipsis where they overflow the root
    /// (`text-overflow: ellipsis`).
    pub(crate) ellipsis: bool,
    pub(crate) letter_styles: Vec<super::first_letter::LetterStyle>,
    letter_style_index: Arc<LetterStyleIndex>,
}

#[derive(Default)]
struct LetterStyleIndex {
    fragments: HashMap<(usize, Option<usize>, Option<usize>), usize>,
    owners: HashMap<(usize, usize), usize>,
    parents: HashMap<usize, usize>,
    dom_sources: HashMap<usize, Vec<usize>>,
    generated_sources: HashMap<usize, usize>,
}

impl LetterStyleIndex {
    fn new(styles: &[LetterStyle]) -> Self {
        let mut index = Self::default();
        for (position, style) in styles.iter().enumerate() {
            for owner in [Some(style.source_owner), None] {
                index
                    .fragments
                    .entry((style.box_id, style.source_container, owner))
                    .or_insert(position);
            }
            index
                .owners
                .entry((style.box_id, style.source_owner))
                .or_insert(position);
            index.parents.entry(style.box_id).or_insert(position);
            if style.source_range.is_some() {
                index
                    .dom_sources
                    .entry(style.source_owner)
                    .or_default()
                    .push(position);
            }
            // Generated runs use the last matching retained style; the other
            // public accessors preserve the first matching projection entry.
            index.generated_sources.insert(style.box_id, position);
        }
        index
    }
}

/// Lines broken for one content-box width.
#[derive(Clone)]
pub(crate) struct IfcLines {
    /// Content-box width the lines were broken at.
    pub(crate) width: f32,
    /// Shared, so cloning a root or a document does not copy them.
    pub(crate) lines: Arc<Vec<Line>>,
    /// Sum of the line advances.
    pub(crate) height: f32,
    /// True when some line was laid out beside a float (its space was
    /// narrower than the content box or did not start at the content start).
    pub(crate) beside_floats: bool,
    /// Bottom margins of a last block child that collapse through the root's
    /// bottom edge with the root's own (CSS 2.1 8.3.1); not in `height`.
    pub(crate) escaping_margin: taffy::CollapsibleMarginSet,
    /// Collapsed top margins of a block child that starts the paragraph, with
    /// no line, float or other block above it; they placed its box inside
    /// the root (CSS 2.1 8.3.1). `None` when the paragraph starts otherwise.
    pub(crate) leading_block_margin: Option<taffy::CollapsibleMarginSet>,
    /// Each block child of the root with the index of the first line after
    /// it.
    pub(crate) block_line_starts: Vec<(usize, usize)>,
    /// Block offsets added to the lines by pagination: `(first line, delta)`
    /// moves that line and every later one down by `delta`, when a box of the
    /// paragraph was moved to a later page.
    pub(crate) shifts: Vec<(usize, f32)>,
    /// Accepted float placements recorded by a fragmenting IFC layout.
    pub(crate) fragment_box_placements: Vec<IfcBoxFragment>,
    /// Line ranges already assigned to fragmentainers by a fragmenting IFC layout.
    pub(crate) fragmentainer_line_ranges: Option<Vec<crate::node::MulticolTextFragment>>,
    /// Column whose final line range overflowed the fragmentainer budget.
    pub(crate) unfragmented_tail_column: Option<usize>,
}

/// Placement of an IFC-owned box accepted in one fragmentainer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct IfcBoxFragment {
    pub(crate) node_id: usize,
    pub(crate) fragmentainer: usize,
    pub(crate) rect: crate::fragment::FragmentRect,
}

impl IfcLines {
    /// How far pagination moved line `index` down.
    pub(crate) fn shift_of(&self, index: usize) -> f32 {
        self.shifts
            .iter()
            .filter(|(first, _)| *first <= index)
            .map(|(_, delta)| delta)
            .sum()
    }

    /// The block offset of line `index` from the content-box top, with the
    /// pagination shift.
    pub(crate) fn line_top(&self, index: usize) -> f32 {
        self.lines
            .get(index)
            .map_or(0.0, |line| line.block_offset())
            + self.shift_of(index)
    }
}

impl IfcRoot {
    /// Baseline of the last line of the last performed layout, from the
    /// content-box top.
    pub(crate) fn last_baseline(&self) -> Option<f32> {
        self.lines.as_ref().and_then(super::flow::last_baseline)
    }

    pub(crate) fn new(projected: ProjectedIfc) -> Self {
        let letter_style_index = Arc::new(LetterStyleIndex::new(&projected.letter_styles));
        Self {
            paragraph: projected.paragraph,
            writing_mode: projected.writing_mode,
            options: projected.options,
            indent: projected.indent,
            lines: None,
            boxes: projected.boxes,
            marker_atomic: projected.marker_atomic,
            rtl: projected.rtl,
            offsets: projected.offsets,
            cleared_breaks: projected.cleared_breaks,
            fixed: projected.fixed,
            multicol_fragments: None,
            multicol_fragment_origins_recorded: false,
            ellipsis: projected.ellipsis,
            letter_styles: projected.letter_styles,
            letter_style_index,
        }
    }

    /// A copy without the stored lines, for measuring with lines of its own.
    pub(crate) fn without_lines(&self) -> Self {
        Self {
            paragraph: self.paragraph.clone(),
            writing_mode: self.writing_mode,
            options: self.options,
            indent: self.indent,
            lines: None,
            boxes: self.boxes.clone(),
            marker_atomic: self.marker_atomic,
            rtl: self.rtl,
            offsets: self.offsets.clone(),
            cleared_breaks: self.cleared_breaks.clone(),
            fixed: self.fixed,
            multicol_fragments: None,
            multicol_fragment_origins_recorded: false,
            ellipsis: self.ellipsis,
            letter_styles: self.letter_styles.clone(),
            letter_style_index: self.letter_style_index.clone(),
        }
    }

    pub(crate) fn typographic_fragment(
        &self,
        box_id: usize,
        container: Option<usize>,
        owner: Option<usize>,
    ) -> Option<&LetterStyle> {
        self.letter_style_index
            .fragments
            .get(&(box_id, container, owner))
            .and_then(|index| self.letter_styles.get(*index))
    }

    pub(crate) fn typographic_owner(&self, box_id: usize, owner: usize) -> Option<&LetterStyle> {
        self.letter_style_index
            .owners
            .get(&(box_id, owner))
            .and_then(|index| self.letter_styles.get(*index))
    }

    pub(crate) fn typographic_parent(&self, box_id: usize) -> Option<usize> {
        self.letter_style_index
            .parents
            .get(&box_id)
            .and_then(|index| self.letter_styles[*index].parent_box)
    }

    pub(crate) fn typographic_source(
        &self,
        source: shodo::node::TextSource,
    ) -> Option<&LetterStyle> {
        match source {
            shodo::node::TextSource::Dom { node, offset } => self
                .letter_style_index
                .dom_sources
                .get(&(node.0 as usize))?
                .iter()
                .rev()
                .map(|index| &self.letter_styles[*index])
                .find(|style| {
                    style
                        .source_range
                        .as_ref()
                        .is_some_and(|range| range.contains(&offset))
                }),
            shodo::node::TextSource::Generated { node } => self
                .letter_style_index
                .generated_sources
                .get(&(node.0 as usize))
                .and_then(|index| self.letter_styles.get(*index)),
        }
    }
}

/// Run `f` with the engine state taken out of the document, then put it back.
///
/// Never hold the state across a call that lays out another node: that node
/// may be an ifc root that needs the state itself. Returns `None` when the
/// document did not opt in.
pub(crate) fn with_state<R>(
    doc: &mut crate::Document,
    f: impl FnOnce(&mut IfcState) -> R,
) -> Option<R> {
    let mut state = doc.ifc.take()?;
    let result = f(&mut state);
    doc.ifc = Some(state);
    Some(result)
}

/// Document-level engine handles, present once the document has fonts.
pub(crate) struct IfcState {
    pub(crate) fonts: FontCollection,
    pub(crate) limits: Limits,
    pub(crate) layout_cx: LayoutContext,
    /// Results produced by `Document::shape_standalone_text` for this
    /// document. Atomic because shaping takes `&Document`.
    pub(crate) standalone_calls: AtomicUsize,
    /// Paragraphs are built on several threads when there are at least this
    /// many roots and `parallel_build` is set.
    pub(crate) parallel_threshold: usize,
    /// Whether many roots may be built on several threads. Either way gives
    /// the same paragraphs, also over the installed fonts: shodo picks a
    /// fallback face from the platform catalog in a fixed order, not from the
    /// order in which faces were first loaded. On by default.
    pub(crate) parallel_build: bool,
    /// How the roots of the last layout pass were built.
    pub(crate) last_build: Option<IfcBuildMode>,
    /// Width of the page area of the current layout pass: the containing
    /// block of fixed boxes.
    pub(crate) page_width: Option<f32>,
}

/// How a layout pass built its paragraphs.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfcBuildMode {
    /// One after the other, on the calling thread.
    Sequential,
    /// On several threads, one layout context per worker.
    Parallel,
}

/// Below this many paragraphs the thread overhead outweighs the gain.
pub(crate) const DEFAULT_PARALLEL_THRESHOLD: usize = 32;

impl IfcState {
    pub(crate) fn new(fonts: FontCollection, limits: Limits) -> Self {
        Self {
            fonts,
            limits,
            layout_cx: LayoutContext::new(),
            standalone_calls: AtomicUsize::new(0),
            parallel_threshold: DEFAULT_PARALLEL_THRESHOLD,
            parallel_build: true,
            last_build: None,
            page_width: None,
        }
    }
}

// `Line` and `FontCollection` have no `Debug`, so these are written by hand.
impl fmt::Debug for IfcRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IfcRoot")
            .field("writing_mode", &self.writing_mode)
            .field("options", &self.options)
            .field("indent", &self.indent)
            .field("lines", &self.lines)
            .field("boxes", &self.boxes)
            .field("rtl", &self.rtl)
            .field("offsets", &self.offsets)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for IfcLines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IfcLines")
            .field("width", &self.width)
            .field("lines", &self.lines.len())
            .field("height", &self.height)
            .field("beside_floats", &self.beside_floats)
            .field("escaping_margin", &self.escaping_margin.resolve())
            .field("shifts", &self.shifts)
            .field(
                "fragment_box_placements",
                &self.fragment_box_placements.len(),
            )
            .finish()
    }
}

impl fmt::Debug for IfcState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IfcState")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

// The layout context is per-owner scratch space and the call count and the
// last build belong to its document, so a clone starts fresh. The build
// policy is kept: a clone holds the same font collection.
impl Clone for IfcState {
    fn clone(&self) -> Self {
        Self {
            parallel_threshold: self.parallel_threshold,
            parallel_build: self.parallel_build,
            ..Self::new(self.fonts.clone(), self.limits.clone())
        }
    }
}

#[cfg(test)]
mod tests;
