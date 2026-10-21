use crate::*;
use raikiri_style::{
    ComputedFlexBasis as F, ComputedGridTemplateTracks as G, ComputedGridTrackBreadth as B,
    ComputedGridTrackListComponent as C, ComputedGridTrackSize as S,
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    ComputedLengthPercentageOrNormal as N, ComputedValues, property::CalcLengthPercentage,
};
use std::{ops::Deref, sync::Arc};

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

/// Computed style and calc bindings prepared before layout queries.
///
/// The original constructor borrows both inputs. The owning constructors
/// retain calc owners without making references into their own fields.
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
///
/// Payload owners are borrowed as well, even when the computed values survive.
///
/// ```compile_fail
/// use raikiri_layout_style::*;
/// use raikiri_style::{ComputedValues, ComputedLengthPercentageOrAuto as L, property::CalcLengthPercentage};
/// let mut values = ComputedValues::initial();
/// let expression = CalcLengthPercentage { px: 1.0, percent: 20.0 };
/// values.width = L::Calc(expression);
/// let prepared = {
///     let payload = AlignedCalc::new(expression, true);
///     let bindings = [PreparedCalc { slot: CalcSlot::Property(LengthProperty::Width), payload: &payload }];
///     PreparedStyle::new(&values, StyleMetadata::default(), &bindings).unwrap()
/// };
/// let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
/// let _ = taffy::CoreStyle::size(&view);
/// ```
///
/// Keeping both owners and bindings alive makes the same query valid.
///
/// ```
/// use raikiri_layout_style::*;
/// use raikiri_style::{ComputedValues, ComputedLengthPercentageOrAuto as L, property::CalcLengthPercentage};
/// let mut values = ComputedValues::initial();
/// let expression = CalcLengthPercentage { px: 1.0, percent: 20.0 };
/// values.width = L::Calc(expression);
/// let payload = AlignedCalc::new(expression, true);
/// let bindings = [PreparedCalc { slot: CalcSlot::Property(LengthProperty::Width), payload: &payload }];
/// let prepared = PreparedStyle::new(&values, StyleMetadata::default(), &bindings).unwrap();
/// let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
/// let _ = taffy::CoreStyle::size(&view);
/// ```
#[derive(Clone, Debug)]
pub struct PreparedStyle<'a> {
    pub(crate) values: ComputedSource<'a>,
    pub(crate) metadata: StyleMetadata,
    calcs: CalcBindings<'a>,
}
#[derive(Clone, Debug)]
pub(crate) enum ComputedSource<'a> {
    Borrowed(&'a ComputedValues),
    Shared(Arc<ComputedValues>),
}
impl Deref for ComputedSource<'_> {
    type Target = ComputedValues;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(values) => values,
            Self::Shared(values) => values,
        }
    }
}
#[derive(Clone, Debug)]
enum CalcBindings<'a> {
    Borrowed(&'a [PreparedCalc<'a>]),
    Owned(Box<[(CalcSlot, Arc<AlignedCalc>)]>),
}
impl<'a> PreparedStyle<'a> {
    /// Refresh document facts without rebuilding computed values or calc bindings.
    pub fn set_metadata(&mut self, metadata: StyleMetadata) {
        self.metadata = metadata;
    }

    /// Borrow computed values and allocate owners for their calc slots once.
    pub fn prepare(values: &'a ComputedValues, metadata: StyleMetadata) -> Self {
        Self::prepare_source(ComputedSource::Borrowed(values), metadata)
    }

    /// Retain shared computed values and allocate their calc owners once.
    pub fn prepare_shared(
        values: Arc<ComputedValues>,
        metadata: StyleMetadata,
    ) -> PreparedStyle<'static> {
        PreparedStyle::prepare_source(ComputedSource::Shared(values), metadata)
    }

    /// Borrow computed values and retain payloads supplied by an adapter.
    /// The factory runs only during preparation, never from a layout getter.
    ///
    /// # Errors
    /// Rejects a payload whose expression or range differs from its calc slot.
    pub fn prepare_with(
        values: &'a ComputedValues,
        metadata: StyleMetadata,
        owner: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Arc<AlignedCalc>,
    ) -> Result<Self, PrepareError> {
        Self::prepare_source_with(ComputedSource::Borrowed(values), metadata, owner)
    }

    /// Retain shared computed values and adapter-supplied calc payloads.
    ///
    /// # Errors
    /// Rejects a payload whose expression or range differs from its calc slot.
    pub fn prepare_shared_with(
        values: Arc<ComputedValues>,
        metadata: StyleMetadata,
        owner: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Arc<AlignedCalc>,
    ) -> Result<PreparedStyle<'static>, PrepareError> {
        PreparedStyle::prepare_source_with(ComputedSource::Shared(values), metadata, owner)
    }

    fn prepare_source(values: ComputedSource<'a>, metadata: StyleMetadata) -> Self {
        Self::prepare_source_with(values, metadata, |_, expression, range| {
            Arc::new(AlignedCalc::new(expression, range))
        })
        .expect("fresh calc payloads match their source expressions")
    }

    fn prepare_source_with(
        values: ComputedSource<'a>,
        metadata: StyleMetadata,
        mut owner: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Arc<AlignedCalc>,
    ) -> Result<Self, PrepareError> {
        let mut prepared = Self {
            values,
            metadata,
            calcs: CalcBindings::Owned(Box::default()),
        };
        let mut calcs = Vec::new();
        prepared.visit_calcs(|slot, expression, range| {
            let payload = owner(slot, expression, range);
            if !payload.matches(expression, range) {
                return Err(PrepareError::Mismatch(slot));
            }
            calcs.push((slot, payload));
            Ok(())
        })?;
        prepared.calcs = CalcBindings::Owned(calcs.into_boxed_slice());
        Ok(prepared)
    }
    /// Validate the complete property/component table once, outside layout getters.
    ///
    /// # Errors
    /// Returns an error for missing, duplicate, mismatched or unexpected bindings.
    pub fn new(
        values: &'a ComputedValues,
        metadata: StyleMetadata,
        calcs: &'a [PreparedCalc<'a>],
    ) -> Result<Self, PrepareError> {
        for (i, binding) in calcs.iter().enumerate() {
            if calcs[..i].iter().any(|other| other.slot == binding.slot) {
                return Err(PrepareError::Duplicate(binding.slot));
            }
        }
        let prepared = Self {
            values: ComputedSource::Borrowed(values),
            metadata,
            calcs: CalcBindings::Borrowed(calcs),
        };
        let mut count = 0;
        prepared.visit_calcs(|slot, expression, range| {
            count += 1;
            let binding = calcs
                .iter()
                .find(|b| b.slot == slot)
                .ok_or(PrepareError::Missing(slot))?;
            if binding.payload.matches(expression, range) {
                Ok(())
            } else {
                Err(PrepareError::Mismatch(slot))
            }
        })?;
        if count != calcs.len() {
            return Err(PrepareError::Unexpected);
        }
        Ok(prepared)
    }
    pub(crate) fn payload(&self, slot: CalcSlot) -> &AlignedCalc {
        match &self.calcs {
            CalcBindings::Borrowed(bindings) => {
                bindings
                    .iter()
                    .find(|binding| binding.slot == slot)
                    .expect("validated immutable calc bindings")
                    .payload
            }
            CalcBindings::Owned(bindings) => {
                &bindings
                    .iter()
                    .find(|(binding_slot, _)| *binding_slot == slot)
                    .expect("constructed immutable calc bindings")
                    .1
            }
        }
    }
    pub(crate) fn scalar(&self, property: LengthProperty) -> L {
        use LengthProperty::*;
        let c = &*self.values;
        let padding = |v| match v {
            P::Px(v) => L::Px(v),
            P::Percent(v) => L::Percent(v),
            P::Calc(v) => L::Calc(v),
        };
        let gap = |v| match v {
            N::Px(v) => L::Px(v),
            N::Percent(v) => L::Percent(v),
            N::Calc(v) => L::Calc(v),
            N::Normal => L::Px(0.0),
        };
        match property {
            Width => c.width,
            Height => c.height,
            MinWidth => c.min_width,
            MinHeight => c.min_height,
            MaxWidth => c.max_width,
            MaxHeight => c.max_height,
            MarginTop => c.margin.top,
            MarginRight => c.margin.right,
            MarginBottom => c.margin.bottom,
            MarginLeft => c.margin.left,
            PaddingTop => padding(c.padding.top),
            PaddingRight => padding(c.padding.right),
            PaddingBottom => padding(c.padding.bottom),
            PaddingLeft => padding(c.padding.left),
            BorderTop => L::Px(c.border.top.width().px()),
            BorderRight => L::Px(c.border.right.width().px()),
            BorderBottom => L::Px(c.border.bottom.width().px()),
            BorderLeft => L::Px(c.border.left.width().px()),
            Top => c.top,
            Right => c.right,
            Bottom => c.bottom,
            Left => c.left,
            ColumnGap => gap(c.column_gap),
            RowGap => gap(c.row_gap),
            FlexBasis => match c.flex_basis {
                F::Auto | F::Content => L::Auto,
                F::MinContent => L::MinContent,
                F::MaxContent => L::MaxContent,
                F::FitContent => L::FitContent,
                F::Px(v) => L::Px(v),
                F::Percent(v) => L::Percent(v),
                F::Calc(v) => L::Calc(v),
            },
            LogicalWidth => c
                .vertical_logical_size
                .as_ref()
                .map_or(c.width, |s| s.width),
            LogicalHeight => c
                .vertical_logical_size
                .as_ref()
                .map_or(c.height, |s| s.height),
            MinBlock => c.min_block_size.unwrap_or(L::Auto),
        }
    }
    fn visit_calcs(
        &self,
        mut visit: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Result<(), PrepareError>,
    ) -> Result<(), PrepareError> {
        for &property in LengthProperty::ALL {
            // Logical aliases only need their own bindings when they are present.
            if matches!(
                property,
                LengthProperty::LogicalWidth | LengthProperty::LogicalHeight
            ) && self.values.vertical_logical_size.is_none()
            {
                continue;
            }
            let expression = match self.scalar(property) {
                L::Calc(c) | L::FitContentOf(P::Calc(c)) => Some(c),
                _ => None,
            };
            if let Some(c) = expression {
                visit(CalcSlot::Property(property), c, property.non_negative())?;
            }
        }
        let mut track = |axis, list, component, repeat_track, size| {
            let (min, max) = match size {
                S::Breadth(v) => (Some(v), Some(v)),
                S::MinMax(a, b) => (Some(a), Some(b)),
                S::FitContent(_) => (None, None),
            };
            for (part, value) in [(TrackPart::Min, min), (TrackPart::Max, max)] {
                if let Some(B::Calc(c)) = value {
                    visit(
                        CalcSlot::Grid {
                            axis,
                            list,
                            component,
                            repeat_track,
                            part,
                        },
                        c,
                        true,
                    )?;
                }
            }
            Ok::<_, PrepareError>(())
        };
        let c = &*self.values;
        for (axis, template, auto) in [
            (Axis::X, &c.grid_template_columns, &c.grid_auto_columns),
            (Axis::Y, &c.grid_template_rows, &c.grid_auto_rows),
        ] {
            if let G::List(list) = template {
                for (i, component) in list.components.iter().enumerate() {
                    match component {
                        C::Size(size) => track(axis, GridList::Template, i, None, *size)?,
                        C::Repeat(repeat) => {
                            for (j, size) in repeat.tracks.iter().enumerate() {
                                track(axis, GridList::Template, i, Some(j), *size)?;
                            }
                        }
                    }
                }
            }
            for (i, size) in auto.iter().enumerate() {
                track(axis, GridList::Auto, i, None, *size)?;
            }
        }
        Ok(())
    }
}
