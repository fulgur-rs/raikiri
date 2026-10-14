//! Per-root state of the shodo inline engine: the built paragraph, the lines of
//! the last performed layout, and the document-level engine handles.

use super::boxes::IfcBox;
use super::clear_breaks::ClearBreakIndex;
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
    /// Fixed-size images of `::before` and `::after` content, independent of
    /// the DOM box arena.
    pub(crate) generated_atomics: Vec<(shodo::node::NodeId, shodo::AtomicSize)>,
    /// Block-level `::before` and `::after` boxes of the paragraph, which the
    /// line loop places between the lines at their tokens.
    pub(crate) generated_blocks: Arc<[super::generated_blocks::GeneratedBlock]>,
    /// The root's `direction` is `rtl`: its lines start at the right edge.
    pub(crate) rtl: bool,
    /// Paint offsets of the relatively positioned inline elements of the
    /// paragraph, by DOM node id.
    pub(crate) offsets: Vec<(usize, (f32, f32))>,
    /// The insets of the elements in `offsets` whose offset depends on the
    /// root's size or writing mode, by index in `offsets`, which
    /// [`Self::resolve_percent_offsets`] applies.
    pub(crate) percent_insets: Vec<(usize, super::style::InlineRelativeInsets)>,
    /// The root's content height when it is definite, which a percentage
    /// `top` or `bottom` of its inline elements refers to.
    pub(crate) definite_height: Option<f32>,
    /// The root's physical content width, which a percentage `left` or
    /// `right` of its inline elements refers to. Lines may be narrower when
    /// the root lays them out in columns.
    pub(crate) containing_width: Option<f32>,
    /// `<br>` elements with a physical `clear`: the line after each starts
    /// below the floats it clears.
    pub(crate) cleared_breaks: ClearBreakIndex,
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
    /// The root's text does not wrap (`text-wrap-mode: nowrap`).
    pub(crate) nowrap: bool,
    /// Whether each text source (a DOM text node or a generated
    /// pseudo-element), atomic inline or inline box of the paragraph sits in
    /// content that does not wrap, by node id.
    pub(crate) text_nowrap: Arc<HashMap<u64, bool>>,
    pub(crate) letter_styles: Vec<super::first_letter::LetterStyle>,
    /// The opacity of the translucent `::first-line` boxes around the first
    /// line, multiplied.
    pub(crate) first_line_opacity: Option<f32>,
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
    pub(crate) shifts: LineShifts,
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

/// Block offsets added to the lines of a paragraph by pagination, as
/// `(first line, delta)` entries in the order they were added.
///
/// The running totals of the deltas answer the shift of one line with a
/// binary search while the entries are in line order, which is how
/// pagination adds them; otherwise the entries are summed directly. Both
/// add the same deltas in the same order, so the results are identical.
#[derive(Clone, Debug)]
pub(crate) struct LineShifts {
    entries: Vec<(usize, f32)>,
    /// `totals[k]` is the sum of the first `k` deltas.
    totals: Vec<f32>,
    /// Some entry starts at an earlier line than the one before it.
    unordered: bool,
}

impl Default for LineShifts {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            totals: vec![std::iter::empty::<f32>().sum()],
            unordered: false,
        }
    }
}

impl LineShifts {
    /// Move line `first` and every later line down by `delta`.
    pub(crate) fn push(&mut self, first: usize, delta: f32) {
        self.unordered |= self.entries.last().is_some_and(|&(last, _)| last > first);
        self.entries.push((first, delta));
        let total = self.totals[self.totals.len() - 1] + delta;
        self.totals.push(total);
    }

    /// The entries in the order they were added.
    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(usize, f32)> {
        self.entries.iter()
    }

    /// The sum of every delta.
    pub(crate) fn total(&self) -> f32 {
        self.totals[self.totals.len() - 1]
    }

    /// How far the entries move line `index` down.
    fn of(&self, index: usize) -> f32 {
        if self.unordered {
            return self
                .entries
                .iter()
                .filter(|(first, _)| *first <= index)
                .map(|(_, delta)| delta)
                .sum();
        }
        self.totals[self.entries.partition_point(|&(first, _)| first <= index)]
    }
}

impl IntoIterator for LineShifts {
    type Item = (usize, f32);
    type IntoIter = std::vec::IntoIter<(usize, f32)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl IfcLines {
    /// How far pagination moved line `index` down.
    pub(crate) fn shift_of(&self, index: usize) -> f32 {
        self.shifts.of(index)
    }

    /// The block offset of the end of the paragraph's content from its
    /// content-box top: its laid-out height, moved as its last line was.
    pub(crate) fn shifted_height(&self) -> f32 {
        self.height
            + self
                .lines
                .len()
                .checked_sub(1)
                .map_or(0.0, |last| self.shift_of(last))
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

    /// Whether `line` cannot wrap: its text and atomic inlines all sit in
    /// `text-wrap-mode: nowrap` content. A line with neither, such as one
    /// holding only an empty padded inline, is decided by its inline boxes,
    /// and a line with no recorded content by the root.
    pub(crate) fn line_is_nowrap(&self, line: &shodo::Line) -> bool {
        let content = |fragment: shodo::Fragment<'_>| match fragment {
            shodo::Fragment::GlyphRun(run) => run.node().map(|node| node.0),
            shodo::Fragment::Atomic(atomic) => Some(atomic.node.0),
            _ => None,
        };
        let inline_box = |fragment: shodo::Fragment<'_>| match fragment {
            shodo::Fragment::InlineBox(inline) => Some(inline.node.0),
            _ => None,
        };
        self.nodes_are_nowrap(line, content)
            .or_else(|| self.nodes_are_nowrap(line, inline_box))
            .unwrap_or(self.nowrap)
    }

    /// Whether every recorded node `pick` takes from `line`'s fragments does
    /// not wrap; `None` when it takes no recorded node.
    fn nodes_are_nowrap(
        &self,
        line: &shodo::Line,
        pick: impl Fn(shodo::Fragment<'_>) -> Option<u64>,
    ) -> Option<bool> {
        let mut any = false;
        for fragment in line.fragments() {
            let Some(nowrap) = pick(fragment).and_then(|node| self.text_nowrap.get(&node).copied())
            else {
                continue;
            };
            if !nowrap {
                return Some(false);
            }
            any = true;
        }
        any.then_some(true)
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
            generated_atomics: projected.generated_atomics,
            generated_blocks: projected.generated_blocks.into(),
            rtl: projected.rtl,
            offsets: projected.offsets,
            percent_insets: projected.percent_insets,
            definite_height: None,
            containing_width: None,
            cleared_breaks: projected.cleared_breaks.into_iter().collect(),
            fixed: projected.fixed,
            multicol_fragments: None,
            multicol_fragment_origins_recorded: false,
            ellipsis: projected.ellipsis,
            nowrap: projected.nowrap,
            text_nowrap: projected.text_nowrap,
            letter_styles: projected.letter_styles,
            first_line_opacity: projected.first_line_opacity,
            letter_style_index,
        }
    }

    /// Resolve the offsets that depend on the root's size against a content
    /// box [`Self::containing_width`] (else `width`) wide and
    /// [`Self::definite_height`] high, the containing block of its inline
    /// elements.
    pub(crate) fn resolve_percent_offsets(&mut self, width: f32) {
        let width = self.containing_width.unwrap_or(width);
        for (index, insets) in &self.percent_insets {
            self.offsets[*index].1 =
                insets.offset_in(width, self.definite_height, self.writing_mode, self.rtl);
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
            generated_atomics: self.generated_atomics.clone(),
            generated_blocks: self.generated_blocks.clone(),
            rtl: self.rtl,
            offsets: self.offsets.clone(),
            percent_insets: self.percent_insets.clone(),
            definite_height: self.definite_height,
            containing_width: self.containing_width,
            cleared_breaks: self.cleared_breaks.clone(),
            fixed: self.fixed,
            multicol_fragments: None,
            multicol_fragment_origins_recorded: false,
            ellipsis: self.ellipsis,
            nowrap: self.nowrap,
            text_nowrap: self.text_nowrap.clone(),
            letter_styles: self.letter_styles.clone(),
            first_line_opacity: self.first_line_opacity,
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
    /// How many times a paragraph was scanned for its min- and max-content
    /// widths.
    #[cfg(test)]
    pub(crate) intrinsic_measurements: usize,
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
    /// Whether layout passes keep the paragraphs they build, for later passes
    /// over the same document and cascade to reuse.
    pub(crate) retain_paragraphs: bool,
    /// The paragraphs of the last layout pass, before any line was broken,
    /// when [`Self::retain_paragraphs`] is set.
    pub(crate) retained: Option<RetainedParagraphs>,
}

/// Paragraphs built by one layout pass, with what they were built from.
///
/// A paragraph depends on the document's content and the element cascade,
/// never on the page geometry, so passes that only change the page sizes can
/// reuse it and break its lines again.
pub(crate) struct RetainedParagraphs {
    pub(crate) key: RetainedKey,
    /// Each root with its paragraph, in ascending index order.
    pub(crate) roots: Vec<(usize, IfcRoot)>,
}

/// What a set of retained paragraphs was built from. Document mutations drop
/// the retained paragraphs; the key catches the inputs that change without
/// one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetainedKey {
    pub(crate) element_generation: u64,
    pub(crate) node_count: usize,
    pub(crate) continuation_break: Option<(usize, u32)>,
    pub(crate) anonymous_cells: usize,
}

/// How a layout pass built its paragraphs.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfcBuildMode {
    /// One after the other, on the calling thread.
    Sequential,
    /// On several threads, one layout context per worker.
    Parallel,
    /// Reused from the previous pass over the same document and cascade.
    Retained,
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
            #[cfg(test)]
            intrinsic_measurements: 0,
            parallel_threshold: DEFAULT_PARALLEL_THRESHOLD,
            parallel_build: true,
            last_build: None,
            page_width: None,
            retain_paragraphs: false,
            retained: None,
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

// The layout context is per-owner scratch space and the call count, the
// last build and the retained paragraphs belong to its document, so a clone
// starts fresh. The build policy is kept: a clone holds the same font
// collection.
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
