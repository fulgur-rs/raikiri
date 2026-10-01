//! Per-root state of the shodo inline engine: the built paragraph, the lines of
//! the last performed layout, and the document-level engine handles.

use super::boxes::IfcBox;
use super::projection::ProjectedIfc;
use raikiri_style::ComputedTextIndent;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::style::LineOptions;
use shodo::{LayoutContext, Line, Paragraph};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

/// A block laid out as one shodo paragraph.
#[derive(Clone)]
pub(crate) struct IfcRoot {
    /// `Paragraph` is a cheap clone over shared data, so it is held directly.
    pub(crate) paragraph: Paragraph,
    pub(crate) options: LineOptions,
    pub(crate) indent: ComputedTextIndent,
    /// Lines of the last performed layout, if any.
    pub(crate) lines: Option<IfcLines>,
    /// Children laid out as boxes of their own (floats and atomic inlines),
    /// in document order.
    pub(crate) boxes: Vec<IfcBox>,
    /// The root's `direction` is `rtl`: its lines start at the right edge.
    pub(crate) rtl: bool,
    /// Paint offsets of the relatively positioned inline elements of the
    /// paragraph, by DOM node id.
    pub(crate) offsets: Vec<(usize, (f32, f32))>,
    /// Text nodes whose spaces are preserved: spaces of theirs that hang at
    /// the end of a line stay in their elements' boxes.
    pub(crate) preserved_spaces: Vec<usize>,
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
}

impl IfcRoot {
    pub(crate) fn new(projected: ProjectedIfc) -> Self {
        Self {
            paragraph: projected.paragraph,
            options: projected.options,
            indent: projected.indent,
            lines: None,
            boxes: projected.boxes,
            rtl: projected.rtl,
            offsets: projected.offsets,
            preserved_spaces: projected.preserved_spaces,
        }
    }

    /// A copy without the stored lines, for measuring with lines of its own.
    pub(crate) fn without_lines(&self) -> Self {
        Self {
            paragraph: self.paragraph.clone(),
            options: self.options,
            indent: self.indent,
            lines: None,
            boxes: self.boxes.clone(),
            rtl: self.rtl,
            offsets: self.offsets.clone(),
            preserved_spaces: self.preserved_spaces.clone(),
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

/// Document-level engine handles, present only when the switch is on.
pub(crate) struct IfcState {
    pub(crate) fonts: FontCollection,
    pub(crate) limits: Limits,
    pub(crate) layout_cx: LayoutContext,
    /// Results produced by `Document::shape_standalone_text` for this
    /// document. Atomic because shaping takes `&Document`.
    pub(crate) standalone_calls: AtomicUsize,
}

impl IfcState {
    pub(crate) fn new(fonts: FontCollection, limits: Limits) -> Self {
        Self {
            fonts,
            limits,
            layout_cx: LayoutContext::new(),
            standalone_calls: AtomicUsize::new(0),
        }
    }
}

// `Line` and `FontCollection` have no `Debug`, so these are written by hand.
impl fmt::Debug for IfcRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IfcRoot")
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

// The layout context is per-owner scratch space and the call count belongs
// to its document, so a clone starts fresh.
impl Clone for IfcState {
    fn clone(&self) -> Self {
        Self::new(self.fonts.clone(), self.limits.clone())
    }
}

#[cfg(test)]
mod tests;
