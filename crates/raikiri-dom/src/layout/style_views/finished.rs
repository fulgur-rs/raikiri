//! Style reads that run after layout has finished.
//!
//! A [`LayoutSession`](crate::layout::context::LayoutSession) answers the
//! style queries of layout while it runs. Readers that work on the laid out
//! document, such as the page projection, have no session. They read the box
//! through the facts the last layout left in the document: the computed values
//! of the cascade, the box's recorded document facts and its pass overrides.
//! No scoped override is active once layout has returned.

use super::{LayoutBase, ViewContext};
use crate::Document;
use raikiri_layout_style::{
    ClaimsPolicy, LayoutStyleView, OwnedPropertyOverride, PrepareError, PreparedStyle,
};
use raikiri_style::CascadeResult;

/// A box's style as the last layout read it, owning what its view borrows.
pub(crate) struct FinishedStyle<'a> {
    prepared: PreparedStyle<'a>,
    pass: &'a [OwnedPropertyOverride],
}

impl FinishedStyle<'_> {
    /// The box's own style with no role context, like
    /// [`LayoutSession::physical_style`](crate::layout::context::LayoutSession::physical_style).
    pub(crate) fn view(&self) -> LayoutStyleView<'_> {
        let context = ViewContext {
            claims_policy: ClaimsPolicy::Positioning,
            ..ViewContext::default()
        };
        LayoutStyleView::with_owned_overrides(&self.prepared, context, self.pass, &[])
    }
}

impl Document {
    /// The style layout read for `idx`, for a reader that runs after layout.
    ///
    /// `None` when the document holds no style record for the box, or the
    /// record no longer fits the computed values of `cascade`, which happens
    /// only when the document changed after its last layout.
    pub(crate) fn finished_style<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        idx: usize,
    ) -> Option<FinishedStyle<'a>> {
        let record = self.box_style_data.get(self.nodes.box_key(idx)?)?;
        let prepared: Result<PreparedStyle<'a>, PrepareError> = match &record.base {
            LayoutBase::Source(source) => PreparedStyle::from_data(
                cascade.computed.get(*source)?,
                record.metadata,
                record.prepared.clone(),
            ),
            LayoutBase::Synthetic(values) => PreparedStyle::from_shared_data(
                values.clone(),
                record.metadata,
                record.prepared.clone(),
            ),
        };
        Some(FinishedStyle {
            prepared: prepared.ok()?,
            pass: &record.pass,
        })
    }

    /// Reads the style of box `idx` as the last layout left it, through
    /// `read`, for a tool that compares what two builds make of the same
    /// document. The view is the box's own style with no role context, like
    /// `crate::layout::context::LayoutSession::physical_style`, and handles
    /// into calc payloads it returns stay valid while `read` runs.
    ///
    /// `None` under the conditions of `Self::finished_style`.
    #[doc(hidden)]
    pub fn read_finished_style<R>(
        &self,
        cascade: &CascadeResult,
        idx: usize,
        read: impl FnOnce(LayoutStyleView<'_>) -> R,
    ) -> Option<R> {
        let style = self.finished_style(cascade, idx)?;
        Some(read(style.view()))
    }
}
