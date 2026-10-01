//! The lines a text node of an ifc paragraph shows up on.

use crate::Document;
use crate::node::NodeFlags;
use raikiri_traits::NodeKind;
use shodo::Fragment;

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
    /// DOM node id of the paragraph root.
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
    let lines = doc.nodes[root_id].ifc.as_ref()?.lines.as_ref()?;
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
            let top = line.block_offset();
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

#[cfg(test)]
mod tests;
