use crate::*;
use raikiri_style::{
    ComputedValues, ComputedLengthPercentageOrAuto as L,
    ComputedLengthPercentageOrCalc as P, ComputedLengthPercentageOrNormal as N,
    ComputedFlexBasis as F, ComputedGridTemplateTracks as G,
    ComputedGridTrackListComponent as C, ComputedGridTrackSize as S,
    ComputedGridTrackBreadth as B, property::CalcLengthPercentage,
};

/// A preparation error, detected before any getter runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareError {
    /// A required payload is absent.
    Missing(CalcSlot),
    /// Multiple payloads claim the same slot.
    Duplicate(CalcSlot),
    /// Expression or range differs from the computed property.
    Mismatch(CalcSlot),
    /// A binding does not describe a calc in these computed values.
    Unexpected,
}
impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid prepared calc bindings: {self:?}")
    }
}
impl std::error::Error for PrepareError {}

/// A validated borrow of computed style and prepared calc bindings.
///
/// Preparation does not clone the computed values or own any payloads.
/// The adapter retains payload owners separately, for example in Arcs.
///
/// ```compile_fail
/// use raikiri_layout_style::{PreparedStyle, StyleMetadata};
/// use raikiri_style::ComputedValues;
/// let prepared = {
///     let values = ComputedValues::initial();
///     PreparedStyle::new(&values, StyleMetadata::default(), &[]).unwrap()
/// };
/// drop(prepared);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct PreparedStyle<'a> {
    pub(crate) values: &'a ComputedValues,
    pub(crate) metadata: StyleMetadata,
    pub(crate) calcs: &'a [PreparedCalc<'a>],
}
impl<'a> PreparedStyle<'a> {
    /// Validate the complete property/component table once, outside layout getters.
    ///
    /// # Errors
    /// Returns an error for missing, duplicate, mismatched or unexpected bindings.
    pub fn new(values: &'a ComputedValues, metadata: StyleMetadata, calcs: &'a [PreparedCalc<'a>]) -> Result<Self, PrepareError> {
        for (i, binding) in calcs.iter().enumerate() {
            if calcs[..i].iter().any(|other| other.slot == binding.slot) {
                return Err(PrepareError::Duplicate(binding.slot));
            }
        }
        let prepared = Self { values, metadata, calcs };
        let mut count = 0;
        prepared.visit_calcs(|slot, expression, range| {
            count += 1;
            let binding = calcs.iter().find(|b| b.slot == slot).ok_or(PrepareError::Missing(slot))?;
            if binding.payload.matches(expression, range) { Ok(()) } else { Err(PrepareError::Mismatch(slot)) }
        })?;
        if count != calcs.len() { return Err(PrepareError::Unexpected); }
        Ok(prepared)
    }
    pub(crate) fn payload(&self, slot: CalcSlot) -> &'a AlignedCalc {
        self.calcs.iter().find(|b| b.slot == slot).expect("validated immutable calc bindings").payload
    }
    pub(crate) fn scalar(&self, property: LengthProperty) -> L {
        use LengthProperty::*;
        let c = self.values;
        let padding = |v| match v { P::Px(v) => L::Px(v), P::Percent(v) => L::Percent(v), P::Calc(v) => L::Calc(v) };
        let gap = |v| match v { N::Px(v) => L::Px(v), N::Percent(v) => L::Percent(v), N::Calc(v) => L::Calc(v), N::Normal => L::Px(0.0) };
        match property {
            Width => c.width, Height => c.height, MinWidth => c.min_width, MinHeight => c.min_height,
            MaxWidth => c.max_width, MaxHeight => c.max_height,
            MarginTop => c.margin.top, MarginRight => c.margin.right, MarginBottom => c.margin.bottom, MarginLeft => c.margin.left,
            PaddingTop => padding(c.padding.top), PaddingRight => padding(c.padding.right), PaddingBottom => padding(c.padding.bottom), PaddingLeft => padding(c.padding.left),
            BorderTop => L::Px(c.border.top.width().px()), BorderRight => L::Px(c.border.right.width().px()), BorderBottom => L::Px(c.border.bottom.width().px()), BorderLeft => L::Px(c.border.left.width().px()),
            Top => c.top, Right => c.right, Bottom => c.bottom, Left => c.left,
            ColumnGap => gap(c.column_gap), RowGap => gap(c.row_gap),
            FlexBasis => match c.flex_basis { F::Auto | F::Content => L::Auto, F::MinContent => L::MinContent, F::MaxContent => L::MaxContent, F::FitContent => L::FitContent, F::Px(v) => L::Px(v), F::Percent(v) => L::Percent(v), F::Calc(v) => L::Calc(v) },
            LogicalWidth => c.vertical_logical_size.as_ref().map_or(c.width, |s| s.width),
            LogicalHeight => c.vertical_logical_size.as_ref().map_or(c.height, |s| s.height),
            MinBlock => c.min_block_size.unwrap_or(L::Auto),
        }
    }
    fn visit_calcs(&self, mut visit: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Result<(), PrepareError>) -> Result<(), PrepareError> {
        for &property in LengthProperty::ALL {
            // Logical aliases only need their own bindings when they are present.
            if matches!(property, LengthProperty::LogicalWidth | LengthProperty::LogicalHeight) && self.values.vertical_logical_size.is_none() { continue; }
            let expression = match self.scalar(property) { L::Calc(c) | L::FitContentOf(P::Calc(c)) => Some(c), _ => None };
            if let Some(c) = expression {
                visit(CalcSlot::Property(property), c, property.non_negative())?;
            }
        }
        let mut track = |axis, list, component, repeat_track, size| {
            let (min, max) = match size { S::Breadth(v) => (Some(v), Some(v)), S::MinMax(a,b) => (Some(a), Some(b)), S::FitContent(_) => (None, None) };
            for (part, value) in [(TrackPart::Min, min), (TrackPart::Max, max)] {
                if let Some(B::Calc(c)) = value {
                    visit(CalcSlot::Grid { axis, list, component, repeat_track, part }, c, true)?;
                }
            }
            Ok::<_, PrepareError>(())
        };
        let c = self.values;
        for (axis, template, auto) in [(Axis::X, &c.grid_template_columns, &c.grid_auto_columns), (Axis::Y, &c.grid_template_rows, &c.grid_auto_rows)] {
            if let G::List(list) = template {
                for (i, component) in list.components.iter().enumerate() {
                    match component {
                        C::Size(size) => track(axis, GridList::Template, i, None, *size)?,
                        C::Repeat(repeat) => for (j, size) in repeat.tracks.iter().enumerate() {
                            track(axis, GridList::Template, i, Some(j), *size)?;
                        },
                    }
                }
            }
            for (i, size) in auto.iter().enumerate() { track(axis, GridList::Auto, i, None, *size)?; }
        }
        Ok(())
    }
}
