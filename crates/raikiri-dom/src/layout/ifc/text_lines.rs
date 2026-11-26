//! The lines a text node of an ifc paragraph shows up on.

use crate::Document;
use crate::layout::LayoutError;
use crate::node::{NodeData, NodeFlags};
use raikiri_style::CascadeResult;
use raikiri_traits::NodeKind;
use shodo::Fragment;
use std::collections::{HashMap, HashSet};

/// One line of a text node of an ifc paragraph, in the root's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct IfcTextLine {
    /// Index of the line in the paragraph.
    pub line: usize,
    /// Block-start edge of the line, from the root's content-box top.
    pub top: f32,
    /// Block-end edge of the line, from the root's content-box top.
    pub bottom: f32,
}

/// The lines a text node of an ifc paragraph has, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct IfcTextLines {
    /// Internal paragraph key; anonymous table cells have no DOM node of their own.
    pub root: usize,
    /// Content width the lines were broken at.
    pub width: f32,
    /// The lines the node has glyphs on, or preserved white space alone, in
    /// paragraph order.
    pub lines: Vec<IfcTextLine>,
}

/// The paragraph root of `node`: the node itself when it is a root (a text
/// node laid out as an anonymous flex or grid item), else its nearest
/// `IS_IFC_ROOT` ancestor.
fn root_of(doc: &Document, node: usize) -> Option<usize> {
    if let Some(key) = doc
        .table_objects
        .paragraph_owner
        .get(node)
        .copied()
        .flatten()
    {
        return Some(key);
    }
    doc.inclusive_ancestors(node)
        .find(|&id| doc.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT))
}

/// The DOM owners of the glyph runs on `line`, in order. The glyphs of a ruby
/// annotation belong to the line the annotation is placed over.
pub(crate) fn glyph_owners(line: &shodo::Line) -> Vec<usize> {
    #[cfg(any(test, feature = "continuation-diagnostics"))]
    if crate::continuation_diagnostics::enabled() {
        return glyph_owners_counted::<true>(line);
    }
    glyph_owners_counted::<false>(line)
}

fn glyph_owners_counted<const COUNT: bool>(line: &shodo::Line) -> Vec<usize> {
    if COUNT {
        crate::continuation_diagnostics::glyph_owner_call();
    }
    fn collect(line: &shodo::Line, owners: &mut Vec<usize>) {
        for fragment in line.fragments() {
            match fragment {
                Fragment::GlyphRun(run) => owners.extend(run.node().map(|node| node.0 as usize)),
                Fragment::RubyAnnotation(annotation) => collect(annotation.line(), owners),
                _ => {}
            }
        }
    }
    let mut owners = Vec::new();
    collect(line, &mut owners);
    owners
}

/// The text owners of `line`: the owners of its glyph runs, or, for a line
/// with no glyph and no atomic inline, the text whose preserved line feed
/// ends it. Such a line is the line of a preserved line feed alone, and it
/// moves with that text. A `br` ends its line too, but it is no text.
fn line_owners_counted<const COUNT: bool>(line: &shodo::Line) -> Vec<usize> {
    let owners = glyph_owners_counted::<COUNT>(line);
    if !owners.is_empty()
        || line
            .fragments()
            .any(|fragment| matches!(fragment, Fragment::Atomic(_)))
    {
        return owners;
    }
    line.forced_break()
        .map(|forced| forced.node.0 as usize)
        .into_iter()
        .collect()
}

/// Whether `owner`, the node a glyph run comes from, is a text node of an ifc
/// paragraph: the only kind of node that owns lines. The glyphs of generated
/// content come from nodes that are not.
fn is_text_line_owner(doc: &Document, owner: usize) -> bool {
    doc.nodes.get(owner).is_some_and(|candidate| {
        candidate.kind() == NodeKind::Text
            && candidate
                .flags
                .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
    })
}

/// Whether `owner`, the node a glyph run comes from, is a text node that
/// pagination places lines through (see [`places_lines`]).
fn is_placing_text_owner(doc: &Document, cascade: &CascadeResult, owner: usize) -> bool {
    is_text_line_owner(doc, owner)
        && matches!(
            &doc.nodes[owner].data,
            NodeData::Text(text) if places_lines(cascade, owner, &text.text_content)
        )
}

/// Whether the text node `node`, holding `text`, is a page candidate that
/// places the lines it shows up on: anything but white space that its style
/// collapses away. White space that the style preserves lays out a line of its
/// own, which the page breaks before like any other line (Chromium), and so
/// does a no-break space.
pub(crate) fn places_lines(cascade: &CascadeResult, node: usize, text: &str) -> bool {
    !crate::generated_content::is_collapsed_white_space(
        text,
        cascade.computed[node].effective_white_space_collapse,
    )
}

/// The single line of a paragraph that no text node places (see
/// [`single_unplaced_line`]), or the run of such lines that ends it (see
/// [`unplaced_lines_from`]), from the top of its first line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UnplacedLine {
    /// The line, in the root's content box.
    pub(crate) line: IfcTextLine,
    /// Whether the line holds atomic inlines and no text, as opposed to
    /// generated text or a glyphless line.
    pub(crate) atomic: bool,
}

/// The line of the paragraph rooted at `root` when the paragraph is a single
/// line that no text node places: the line of an empty list item holding only
/// its inside marker, of `::before` or `::after` text alone, or of atomic
/// inlines alone, such as an image. A positive glyphless line, from a `br`
/// or an empty outside-marker list item, needs the same placement.
///
/// Pagination places the lines of a paragraph through the text nodes that own
/// them, and generated text and atomic inlines belong to none, so a line like
/// this one is never placed by itself and a page edge would cut it in two. A
/// line whose only real text is white space that the style collapses is such
/// a line: the white space of an unclosed `<li>` before the next one is a text
/// node, but never a page candidate. White space that the style preserves is
/// one (see [`places_lines`]).
pub(crate) fn single_unplaced_line(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> Option<UnplacedLine> {
    let ifc = doc.ifc_layout_node(root)?.ifc.as_ref()?;
    // The lines of a multicol container sit in its columns, not at the offset
    // the paragraph lays them out at.
    if ifc.multicol_fragments.is_some() {
        return None;
    }
    let lines = ifc.lines.as_ref()?;
    if lines.lines.len() != 1 {
        return None;
    }
    unplaced_line_at(doc, cascade, root, lines, 0)
}

/// The lines of the paragraph rooted at `root` from its line `first` to its
/// last, taken together, when no text node places any of them (see
/// [`single_unplaced_line`]) and the last holds generated text, whatever
/// lines come before them: the lines before them are placed by their own
/// text, or lie above a block of the paragraph. A run of more lines than one
/// is not atomic: a page break may still fall between its lines.
pub(crate) fn unplaced_lines_from(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    first: usize,
) -> Option<UnplacedLine> {
    let ifc = doc.ifc_layout_node(root)?.ifc.as_ref()?;
    if ifc.multicol_fragments.is_some() {
        return None;
    }
    let lines = ifc.lines.as_ref()?;
    let last = lines.lines.len().checked_sub(1)?;
    let mut run = unplaced_line_at(doc, cascade, root, lines, first)?;
    for index in first + 1..=last {
        let line = unplaced_line_at(doc, cascade, root, lines, index)?;
        run.line.bottom = line.line.bottom;
        run.atomic = false;
    }
    // The ids of generated text name no node of the document.
    glyph_owners(&lines.lines[last])
        .iter()
        .any(|&owner| doc.nodes.get(owner).is_none())
        .then_some(run)
}

/// The line `index` of the paragraph rooted at `root`, when no text node
/// places it.
fn unplaced_line_at(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    lines: &super::root::IfcLines,
    index: usize,
) -> Option<UnplacedLine> {
    let line = &lines.lines[index];
    if line_owners_counted::<false>(line)
        .iter()
        .any(|&owner| is_placing_text_owner(doc, cascade, owner))
    {
        return None;
    }
    let owners = glyph_owners(line);
    // The ids of generated text name no node of the document.
    let has_generated_text = owners.iter().any(|&owner| doc.nodes.get(owner).is_none());
    let atomic = !has_generated_text
        && line
            .fragments()
            .any(|fragment| matches!(fragment, Fragment::Atomic(_)));
    let glyphless = !has_generated_text && !atomic;
    let block_size = super::outside_marker::fit_height(line);
    // A generated block can leave one shodo line but extra block content.
    if glyphless
        && (!block_size.is_finite() || block_size <= 0.0 || !lines.placed_blocks.is_empty())
    {
        return None;
    }
    let top = lines.line_top(index) - super::outside_marker::fit_leading(doc, root, index);
    Some(UnplacedLine {
        line: IfcTextLine {
            line: index,
            top,
            bottom: top + block_size,
        },
        atomic,
    })
}

/// The lines `node` has a glyph run on, and the lines of its preserved white
/// space (see [`line_owners_counted`]). A line is owned by every text node
/// one of its glyph runs comes from, so a line shared by two text nodes
/// belongs to both, and a line of atomic boxes only belongs to none.
pub(crate) fn lines_of(doc: &Document, node: usize) -> Option<IfcTextLines> {
    #[cfg(any(test, feature = "continuation-diagnostics"))]
    if crate::continuation_diagnostics::enabled() {
        return lines_of_counted::<true>(doc, node);
    }
    lines_of_counted::<false>(doc, node)
}

fn lines_of_counted<const COUNT: bool>(doc: &Document, node: usize) -> Option<IfcTextLines> {
    if !is_text_line_owner(doc, node) {
        return None;
    }
    let root_id = root_of(doc, node)?;
    let lines = doc.ifc_layout_node(root_id)?.ifc.as_ref()?.lines.as_ref()?;
    let owned: Vec<IfcTextLine> = lines
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            #[cfg(test)]
            crate::layout::test_support::TEXT_LINE_VISITS
                .with(|visits| visits.set(visits.get() + 1));
            if COUNT {
                crate::continuation_diagnostics::text_line_visit();
            }
            line_owners_counted::<COUNT>(line).contains(&node)
        })
        .map(|(index, line)| {
            let top = lines.line_top(index);
            IfcTextLine {
                line: index,
                top,
                bottom: top + super::outside_marker::fit_height(line),
            }
        })
        .collect();
    (!owned.is_empty()).then_some(IfcTextLines {
        root: root_id,
        width: lines.width,
        lines: owned,
    })
}

impl Document {
    /// Collect the text-line owners of an inline formatting root in one scan.
    /// Each text node owns a line once even if it has multiple glyph runs on it.
    #[doc(hidden)]
    pub fn ifc_text_lines_by_node(&self, root: usize) -> HashMap<usize, IfcTextLines> {
        self.checked_ifc_text_lines_by_node(root, || Ok(()))
            .expect("uncontrolled line ownership collection cannot fail")
    }

    fn checked_ifc_text_lines_by_node(
        &self,
        root: usize,
        check: impl FnMut() -> Result<(), LayoutError>,
    ) -> Result<HashMap<usize, IfcTextLines>, LayoutError> {
        #[cfg(any(test, feature = "continuation-diagnostics"))]
        if crate::continuation_diagnostics::enabled() {
            return self.checked_ifc_text_lines_by_node_counted::<true>(root, check);
        }
        self.checked_ifc_text_lines_by_node_counted::<false>(root, check)
    }

    fn checked_ifc_text_lines_by_node_counted<const COUNT: bool>(
        &self,
        root: usize,
        mut check: impl FnMut() -> Result<(), LayoutError>,
    ) -> Result<HashMap<usize, IfcTextLines>, LayoutError> {
        let mut by_node = HashMap::new();
        let Some(lines) = self
            .ifc_layout_node(root)
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
        else {
            return Ok(by_node);
        };
        for (index, line) in lines.lines.iter().enumerate() {
            check()?;
            #[cfg(test)]
            crate::layout::test_support::TEXT_LINE_VISITS
                .with(|visits| visits.set(visits.get() + 1));
            let top = lines.line_top(index);
            let owned = IfcTextLine {
                line: index,
                top,
                bottom: top + super::outside_marker::fit_height(line),
            };
            if COUNT {
                crate::continuation_diagnostics::text_line_visit();
            }
            for owner in line_owners_counted::<COUNT>(line) {
                check()?;
                if !is_text_line_owner(self, owner) {
                    continue;
                }
                let entry = by_node.entry(owner).or_insert_with(|| IfcTextLines {
                    root,
                    width: lines.width,
                    lines: Vec::new(),
                });
                if entry.lines.last().is_none_or(|last| last.line != index) {
                    entry.lines.push(owned);
                }
            }
        }
        Ok(by_node)
    }
}

/// Ownership prepared once per paragraph for pagination or a final projection.
/// Pagination rereads coordinates because it can move collected lines;
/// continuation prepares after those moves and reads the stored final positions.
#[derive(Debug, Default)]
pub(crate) struct TextLineIndex {
    roots: HashSet<usize>,
    owners: HashMap<usize, IfcTextLines>,
}

impl TextLineIndex {
    pub(crate) fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    pub(crate) fn prepare_for(
        &mut self,
        doc: &Document,
        node: usize,
        check: impl FnMut() -> Result<(), LayoutError>,
    ) -> Result<(), LayoutError> {
        if !is_text_line_owner(doc, node) {
            return Ok(());
        }
        let Some(root) = root_of(doc, node) else {
            return Ok(());
        };
        if !self.roots.contains(&root) {
            let owners = doc.checked_ifc_text_lines_by_node(root, check)?;
            self.owners.extend(owners);
            self.roots.insert(root);
        }
        Ok(())
    }

    pub(crate) fn get(&self, node: usize) -> Option<&IfcTextLines> {
        self.owners.get(&node)
    }
}

#[cfg(test)]
mod tests;
