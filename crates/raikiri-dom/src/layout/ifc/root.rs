//! Per-root state of the shodo inline engine: the built paragraph, the lines of
//! the last performed layout, and the document-level engine handles.

use super::projection::ProjectedIfc;
use raikiri_style::ComputedTextIndent;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::style::LineOptions;
use shodo::{LayoutContext, Line, Paragraph};
use std::fmt;

/// A block laid out as one shodo paragraph.
#[derive(Clone)]
pub(crate) struct IfcRoot {
    /// `Paragraph` is a cheap clone over shared data, so it is held directly.
    pub(crate) paragraph: Paragraph,
    pub(crate) options: LineOptions,
    pub(crate) indent: ComputedTextIndent,
    /// Lines of the last performed layout, if any.
    pub(crate) lines: Option<IfcLines>,
}

/// Lines broken for one content-box width.
#[derive(Clone)]
pub(crate) struct IfcLines {
    /// Content-box width the lines were broken at.
    pub(crate) width: f32,
    pub(crate) lines: Vec<Line>,
    /// Sum of the line advances.
    pub(crate) height: f32,
}

impl IfcRoot {
    pub(crate) fn new(projected: ProjectedIfc) -> Self {
        Self {
            paragraph: projected.paragraph,
            options: projected.options,
            indent: projected.indent,
            lines: None,
        }
    }
}

/// Document-level engine handles, present only when the switch is on.
pub(crate) struct IfcState {
    pub(crate) fonts: FontCollection,
    pub(crate) limits: Limits,
    pub(crate) layout_cx: LayoutContext,
}

impl IfcState {
    pub(crate) fn new(fonts: FontCollection, limits: Limits) -> Self {
        Self {
            fonts,
            limits,
            layout_cx: LayoutContext::new(),
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
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for IfcLines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IfcLines")
            .field("width", &self.width)
            .field("lines", &self.lines.len())
            .field("height", &self.height)
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

// The layout context is per-owner scratch space, so a clone starts fresh.
impl Clone for IfcState {
    fn clone(&self) -> Self {
        Self::new(self.fonts.clone(), self.limits.clone())
    }
}

#[cfg(test)]
mod tests;
