use crate::flex::{content, items};
use crate::length::sanitize;
use crate::*;
use raikiri_style::{
    ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
    ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
    ComputedLengthPercentage as LP, SmolStr,
    property::{
        GridAutoFlowValue, GridLineValue, GridRepeatCount, GridTemplateAreaEntry,
        GridTemplateAreasValue,
    },
};
use taffy::{
    GenericGridTemplateComponent, GenericRepetition, GridContainerStyle, GridItemStyle,
    GridPlacement, MaxTrackSizingFunction as Max, MinTrackSizingFunction as Min,
    TrackSizingFunction,
};

/// Borrowed top-level Grid components; cloning preserves the current cursor.
#[derive(Clone)]
pub struct TemplateTracks<'a> {
    prepared: &'a PreparedStyle<'a>,
    axis: Axis,
    components: std::slice::Iter<'a, C>,
    index: usize,
}
impl<'a> Iterator for TemplateTracks<'a> {
    type Item = GenericGridTemplateComponent<SmolStr, GridRepetition<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        let component = self.components.next()?;
        let index = self.index;
        self.index += 1;
        Some(match component {
            C::Size(size) => GenericGridTemplateComponent::Single(track(
                self.prepared,
                self.axis,
                GridList::Template,
                index,
                None,
                *size,
            )),
            C::Repeat(repeat) => GenericGridTemplateComponent::Repeat(GridRepetition {
                prepared: self.prepared,
                axis: self.axis,
                component: index,
                repeat,
            }),
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.components.size_hint()
    }
}
impl ExactSizeIterator for TemplateTracks<'_> {}

/// Borrowed implicit tracks or tracks inside one repeat.
#[derive(Clone)]
pub struct Tracks<'a> {
    prepared: &'a PreparedStyle<'a>,
    axis: Axis,
    list: GridList,
    component: Option<usize>,
    tracks: std::slice::Iter<'a, S>,
    index: usize,
}
impl Iterator for Tracks<'_> {
    type Item = TrackSizingFunction;
    fn next(&mut self) -> Option<Self::Item> {
        let size = *self.tracks.next()?;
        let i = self.index;
        self.index += 1;
        Some(track(
            self.prepared,
            self.axis,
            self.list,
            self.component.unwrap_or(i),
            self.component.map(|_| i),
            size,
        ))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.tracks.size_hint()
    }
}
impl ExactSizeIterator for Tracks<'_> {}

/// The line-name sets of a template or repeat, borrowed without a Vec clone.
#[derive(Clone)]
pub struct LineNames<'a>(std::slice::Iter<'a, Vec<SmolStr>>);
impl<'a> Iterator for LineNames<'a> {
    type Item = std::slice::Iter<'a, SmolStr>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|v| v.iter())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl ExactSizeIterator for LineNames<'_> {}
impl<'a> taffy::TemplateLineNames<'a, SmolStr> for LineNames<'a> {
    type LineNameSet<'b>
        = std::slice::Iter<'b, SmolStr>
    where
        Self: 'b;
}

/// A repeat whose tracks and names still belong to ComputedValues.
#[derive(Clone, Copy)]
pub struct GridRepetition<'a> {
    prepared: &'a PreparedStyle<'a>,
    axis: Axis,
    component: usize,
    repeat: &'a ComputedGridTrackRepeat,
}
impl GenericRepetition for GridRepetition<'_> {
    type CustomIdent = SmolStr;
    type RepetitionTrackList<'a>
        = Tracks<'a>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = LineNames<'a>
    where
        Self: 'a;
    fn count(&self) -> taffy::RepetitionCount {
        match self.repeat.count {
            GridRepeatCount::Count(n) => taffy::RepetitionCount::Count(u16_sat(n)),
            GridRepeatCount::AutoFill => taffy::RepetitionCount::AutoFill,
            GridRepeatCount::AutoFit => taffy::RepetitionCount::AutoFit,
            // cov:ignore: GridRepeatCount is non-exhaustive; all current variants are matched above.
            _ => taffy::RepetitionCount::Count(1),
        }
    }
    fn tracks(&self) -> Tracks<'_> {
        Tracks {
            prepared: self.prepared,
            axis: self.axis,
            list: GridList::Template,
            component: Some(self.component),
            tracks: self.repeat.tracks.iter(),
            index: 0,
        }
    }
    fn lines_names(&self) -> LineNames<'_> {
        LineNames(self.repeat.line_names.iter())
    }
}

/// Grid areas with cheaply cloned shared identifiers.
#[derive(Clone)]
pub struct Areas<'a>(std::slice::Iter<'a, GridTemplateAreaEntry>);
impl Iterator for Areas<'_> {
    type Item = taffy::GridTemplateArea<SmolStr>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|a| taffy::GridTemplateArea {
            name: a.name.clone(),
            row_start: u16_sat(a.row_start),
            row_end: u16_sat(a.row_end),
            column_start: u16_sat(a.column_start),
            column_end: u16_sat(a.column_end),
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl ExactSizeIterator for Areas<'_> {}

impl PreparedStyle<'_> {
    fn template(&self, axis: Axis) -> Option<&ComputedGridTrackList> {
        let value = match axis {
            Axis::X => &self.values.grid_template_columns,
            Axis::Y => &self.values.grid_template_rows,
        };
        match value {
            ComputedGridTemplateTracks::List(list) => Some(list),
            _ => None,
        }
    }
}
impl LayoutStyleView<'_> {
    fn template_tracks(&self, axis: Axis) -> TemplateTracks<'_> {
        let components = self
            .prepared
            .template(axis)
            .map_or(&[][..], |list| list.components.as_slice());
        TemplateTracks {
            prepared: self.prepared,
            axis,
            components: components.iter(),
            index: 0,
        }
    }
    fn template_names(&self, axis: Axis) -> LineNames<'_> {
        let names = self
            .prepared
            .template(axis)
            .map_or(&[][..], |list| list.line_names.as_slice());
        LineNames(names.iter())
    }
    fn auto_tracks(&self, axis: Axis) -> Tracks<'_> {
        let tracks = match axis {
            Axis::X => &self.prepared.values.grid_auto_columns,
            Axis::Y => &self.prepared.values.grid_auto_rows,
        };
        Tracks {
            prepared: self.prepared,
            axis,
            list: GridList::Auto,
            component: None,
            tracks: tracks.iter(),
            index: 0,
        }
    }
}
impl GridContainerStyle for LayoutStyleView<'_> {
    type Repetition<'a>
        = GridRepetition<'a>
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = TemplateTracks<'a>
    where
        Self: 'a;
    type AutoTrackList<'a>
        = Tracks<'a>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = LineNames<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = Areas<'a>
    where
        Self: 'a;
    fn grid_template_rows(&self) -> Option<TemplateTracks<'_>> {
        Some(self.template_tracks(Axis::Y))
    }
    fn grid_template_columns(&self) -> Option<TemplateTracks<'_>> {
        Some(self.template_tracks(Axis::X))
    }
    fn grid_auto_rows(&self) -> Tracks<'_> {
        self.auto_tracks(Axis::Y)
    }
    fn grid_auto_columns(&self) -> Tracks<'_> {
        self.auto_tracks(Axis::X)
    }
    fn grid_template_column_names(&self) -> Option<LineNames<'_>> {
        Some(self.template_names(Axis::X))
    }
    fn grid_template_row_names(&self) -> Option<LineNames<'_>> {
        Some(self.template_names(Axis::Y))
    }
    fn grid_template_areas(&self) -> Option<Areas<'_>> {
        match &self.prepared.values.grid_template_areas {
            GridTemplateAreasValue::Areas(areas) => Some(Areas(areas.areas.iter())),
            _ => None,
        }
    }
    fn grid_template_area_row_count(&self) -> u16 {
        match &self.prepared.values.grid_template_areas {
            GridTemplateAreasValue::Areas(a) => u16_sat(a.row_count),
            _ => 0,
        }
    }
    fn grid_template_area_column_count(&self) -> u16 {
        match &self.prepared.values.grid_template_areas {
            GridTemplateAreasValue::Areas(a) => u16_sat(a.column_count),
            _ => 0,
        }
    }
    fn grid_auto_flow(&self) -> taffy::GridAutoFlow {
        match self.prepared.values.grid_auto_flow {
            GridAutoFlowValue::Column => taffy::GridAutoFlow::Column,
            GridAutoFlowValue::RowDense => taffy::GridAutoFlow::RowDense,
            GridAutoFlowValue::ColumnDense => taffy::GridAutoFlow::ColumnDense,
            _ => taffy::GridAutoFlow::Row,
        }
    }
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        self.gap_value()
    }
    fn align_content(&self) -> taffy::AlignContent {
        content(self.prepared.values.align_content)
    }
    fn justify_content(&self) -> taffy::JustifyContent {
        content(self.prepared.values.justify_content)
    }
    fn align_items(&self) -> taffy::AlignItems {
        items(self.prepared.values.align_items)
    }
    fn justify_items(&self) -> taffy::JustifyItems {
        items(self.prepared.values.justify_items)
    }
}
impl GridItemStyle for LayoutStyleView<'_> {
    fn grid_row(&self) -> taffy::Line<GridPlacement<SmolStr>> {
        taffy::Line {
            start: placement(&self.prepared.values.grid_row_start),
            end: placement(&self.prepared.values.grid_row_end),
        }
    }
    fn grid_column(&self) -> taffy::Line<GridPlacement<SmolStr>> {
        taffy::Line {
            start: placement(&self.prepared.values.grid_column_start),
            end: placement(&self.prepared.values.grid_column_end),
        }
    }
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.self_alignment(false)
    }
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.self_alignment(true)
    }
}
fn u16_sat(value: u32) -> u16 {
    value.min(u16::MAX as u32) as u16
}
fn i16_sat(value: i32) -> i16 {
    value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
fn placement(value: &GridLineValue) -> GridPlacement<SmolStr> {
    match value {
        GridLineValue::Line(n) => taffy::style_helpers::line(i16_sat(*n)),
        // Zero preserves named-area edge lookup before the named-line fallback.
        GridLineValue::Named(s) => GridPlacement::NamedLine(s.clone(), 0),
        GridLineValue::NamedLine(s, n) => GridPlacement::NamedLine(s.clone(), i16_sat(*n)),
        GridLineValue::Span(n) => GridPlacement::Span(u16_sat(*n)),
        GridLineValue::SpanNamed(s, n) => GridPlacement::NamedSpan(s.clone(), u16_sat(*n)),
        _ => GridPlacement::Auto,
    }
}
fn track(
    prepared: &PreparedStyle<'_>,
    axis: Axis,
    list: GridList,
    component: usize,
    repeat_track: Option<usize>,
    size: S,
) -> TrackSizingFunction {
    let pointer = |part| {
        prepared
            .payload(CalcSlot::Grid {
                axis,
                list,
                component,
                repeat_track,
                part,
            })
            .pointer()
    };
    let min = |b| match b {
        B::Px(v) => Min::length(sanitize(v)),
        B::Percent(v) => Min::percent(sanitize(v / 100.0)),
        B::MinContent => Min::min_content(),
        B::MaxContent => Min::max_content(),
        B::Calc(_) => Min::calc(pointer(TrackPart::Min)),
        _ => Min::auto(),
    };
    let max = |b| match b {
        B::Px(v) => Max::length(sanitize(v)),
        B::Percent(v) => Max::percent(sanitize(v / 100.0)),
        B::Flex(v) => Max::fr(sanitize(v)),
        B::MinContent => Max::min_content(),
        B::MaxContent => Max::max_content(),
        B::Calc(_) => Max::calc(pointer(TrackPart::Max)),
        B::Auto => Max::auto(),
    };
    match size {
        S::Breadth(v) => TrackSizingFunction {
            min: min(v),
            max: max(v),
        },
        S::MinMax(a, b) => TrackSizingFunction {
            min: min(a),
            max: max(b),
        },
        S::FitContent(v) => TrackSizingFunction {
            min: Min::auto(),
            max: match v {
                LP::Px(v) => Max::fit_content_px(sanitize(v)),
                LP::Percent(v) => Max::fit_content_percent(sanitize(v / 100.0)),
            },
        },
    }
}
