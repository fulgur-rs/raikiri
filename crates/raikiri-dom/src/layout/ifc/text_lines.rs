//! The lines a text node of an ifc paragraph shows up on.

use crate::Document;
use crate::node::NodeFlags;
use raikiri_traits::NodeKind;
use shodo::Fragment;
use std::collections::HashMap;

/// One line of a text node of an ifc paragraph, in the root's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
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
pub struct IfcTextLines {
    /// Internal paragraph key; anonymous table cells have no DOM node of their own.
    pub root: usize,
    /// Content width the lines were broken at.
    pub width: f32,
    /// The lines the node has glyphs on, in paragraph order.
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
    let mut current = Some(node);
    while let Some(id) = current {
        if doc.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT) {
            return Some(id);
        }
        current = doc.parent_of(id);
    }
    None
}

/// The lines `node` has a glyph run on. A line is owned by every text node
/// one of its glyph runs comes from, so a line shared by two text nodes
/// belongs to both, and a line of atomic boxes only belongs to none.
pub(crate) fn lines_of(doc: &Document, node: usize) -> Option<IfcTextLines> {
    let candidate = doc.nodes.get(node)?;
    if !candidate
        .flags
        .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
        || candidate.kind() != NodeKind::Text
    {
        return None;
    }
    let root_id = root_of(doc, node)?;
    let lines = doc.ifc_layout_node(root_id)?.ifc.as_ref()?.lines.as_ref()?;
    let owned: Vec<IfcTextLine> = lines
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.fragments().any(|fragment| match fragment {
                Fragment::GlyphRun(run) => run.node().is_some_and(|owner| owner.0 as usize == node),
                _ => false,
            })
        })
        .map(|(index, line)| {
            let top = lines.line_top(index);
            IfcTextLine {
                line: index,
                top,
                bottom: top + line.block_size(),
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
        let mut by_node = HashMap::new();
        let Some(lines) = self
            .ifc_layout_node(root)
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
        else {
            return by_node;
        };
        for (index, line) in lines.lines.iter().enumerate() {
            let top = lines.line_top(index);
            let owned = IfcTextLine {
                line: index,
                top,
                bottom: top + line.block_size(),
            };
            for fragment in line.fragments() {
                let Fragment::GlyphRun(run) = fragment else {
                    continue;
                };
                let Some(owner) = run.node().map(|node| node.0 as usize) else {
                    continue; // cov:ignore: Raikiri assigns node ids to every projected glyph source; anonymous Shodo runs have no DOM owner.
                };
                let Some(candidate) = self.nodes.get(owner) else {
                    continue;
                };
                if candidate.kind() != NodeKind::Text
                    || !candidate
                        .flags
                        .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
                {
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
        by_node
    }
}

#[cfg(test)]
mod tests;
