use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

use crate::{LayoutStyle, node::arena::BoxKey};
use raikiri_layout_style::{LayoutStyleView, ViewContext};
use taffy::{
    BlockContainerStyle, BlockItemStyle, CoreStyle, FlexboxContainerStyle, FlexboxItemStyle,
    GridContainerStyle, GridItemStyle, OofItemStyle,
};

mod compare;
mod iterators;
pub(crate) mod legacy;
use compare::{CheckedValue, Diagnostic};
use iterators::{
    CheckedAreas, CheckedIter, CheckedNames, CheckedRepeat, CheckedTemplate, CheckedTracks,
};

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub(crate) enum ReadRole {
    FlexContainer,
    FlexItem,
    GridContainer,
    GridItem,
    CoreContainer,
    BlockContainer,
    BlockItem,
    Oof,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadContext {
    pub(crate) node: BoxKey,
    pub(crate) role: ReadRole,
    pub(crate) context: ViewContext,
    pub(crate) pass_entries: usize,
    pub(crate) scope_depth: usize,
    pub(crate) was_scoped: bool,
}
impl ReadContext {
    pub(crate) fn new(node: BoxKey, role: ReadRole) -> Self {
        Self {
            node,
            role,
            context: ViewContext::default(),
            pass_entries: 0,
            scope_depth: 0,
            was_scoped: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ComparisonStats {
    pub(crate) getter_reads: [Cell<usize>; 8],
    pub(crate) iterator_reads: Cell<usize>,
    pub(crate) calc_comparisons: Cell<usize>,
    pub(crate) pass_reads: Cell<usize>,
    pub(crate) scoped_reads: Cell<usize>,
    pub(crate) restored_reads: Cell<usize>,
    pub(crate) properties: RefCell<BTreeMap<&'static str, usize>>,
}
impl ComparisonStats {
    fn getter(&self, property: &'static str, context: ReadContext) {
        let count = &self.getter_reads[context.role as usize];
        count.set(count.get() + 1);
        *self.properties.borrow_mut().entry(property).or_default() += 1;
        if context.pass_entries > 0 {
            self.pass_reads.set(self.pass_reads.get() + 1);
        }
        if context.scope_depth > 0
            || context.context.block_item_basis.is_some()
            || context.context.percent_basis != raikiri_layout_style::PercentBasis::Taffy
        {
            self.scoped_reads.set(self.scoped_reads.get() + 1);
        } else if context.was_scoped {
            self.restored_reads.set(self.restored_reads.get() + 1);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CheckedStyle<'a> {
    legacy: &'a LayoutStyle,
    legacy_query: legacy::LegacyQuery<'a>,
    view: Option<LayoutStyleView<'a>>,
    context: ReadContext,
    stats: &'a ComparisonStats,
}
impl<'a> CheckedStyle<'a> {
    pub(crate) fn new(
        legacy: &'a LayoutStyle,
        view: Option<LayoutStyleView<'a>>,
        context: ReadContext,
        stats: &'a ComparisonStats,
    ) -> Self {
        Self {
            legacy,
            legacy_query: legacy::LegacyQuery::plain(legacy),
            view,
            context,
            stats,
        }
    }
    pub(crate) fn with_legacy_query(mut self, legacy: legacy::LegacyQuery<'a>) -> Self {
        self.legacy_query = legacy;
        self
    }
    pub(crate) fn bridge_align_self(&self) -> Option<taffy::AlignSelf> {
        self.read("Bridge::align_self", self.legacy.align_self, |view| {
            view.bridge_align_self()
        })
    }
    pub(crate) fn bridge_justify_self(&self) -> Option<taffy::AlignSelf> {
        self.read("Bridge::justify_self", self.legacy.justify_self, |view| {
            view.bridge_justify_self()
        })
    }
    pub(crate) fn bridge_align_content(&self) -> taffy::AlignContent {
        self.read("Bridge::align_content", self.legacy.align_content, |view| {
            view.bridge_align_content()
        })
    }
    fn begin(&self, property: &'static str) -> Diagnostic<'a> {
        if self.view.is_some() {
            self.stats.getter(property, self.context);
        }
        Diagnostic::new(self.context, self.stats, property)
    }
    fn read<T: CheckedValue>(
        &self,
        property: &'static str,
        legacy: T,
        candidate: impl FnOnce(&LayoutStyleView<'a>) -> T,
    ) -> T {
        let diagnostic = self.begin(property);
        if let Some(view) = &self.view {
            legacy.check(&candidate(view), diagnostic);
        }
        legacy
    }
}

macro_rules! getters {
    ($trait:ident; $($method:ident: $value:ty),+ $(,)?) => { $(
        fn $method(&self) -> $value {
            self.read(concat!(stringify!($trait), "::", stringify!($method)), $trait::$method(self.legacy), |view| $trait::$method(view))
        }
    )+ };
}

macro_rules! query_getters {
    ($trait:ident; $($method:ident: $value:ty),+ $(,)?) => { $(
        fn $method(&self) -> $value {
            self.read(concat!(stringify!($trait), "::", stringify!($method)), $trait::$method(&self.legacy_query), |view| $trait::$method(view))
        }
    )+ };
}

impl CoreStyle for CheckedStyle<'_> {
    type CustomIdent = raikiri_style::SmolStr;
    query_getters!(CoreStyle;
        box_generation_mode: taffy::BoxGenerationMode, is_block: bool,
        is_compressible_replaced: bool, is_replaced: bool, box_sizing: taffy::BoxSizing,
        direction: taffy::Direction, overflow: taffy::Point<taffy::Overflow>,
        scrollbar_width: f32, position: taffy::Position, inset: taffy::Rect<taffy::LengthPercentageAuto>,
        is_containing_block: taffy::ContainingBlockClaims,
        size: taffy::Size<taffy::Dimension>, min_size: taffy::Size<taffy::LengthPercentageAuto>,
        max_size: taffy::Size<taffy::LengthPercentageAuto>, aspect_ratio: Option<f32>,
        margin: taffy::Rect<taffy::LengthPercentageAuto>, padding: taffy::Rect<taffy::LengthPercentage>,
        border: taffy::Rect<taffy::LengthPercentage>, contain: taffy::Contain
    );
}
impl FlexboxContainerStyle for CheckedStyle<'_> {
    getters!(FlexboxContainerStyle;
        flex_direction: taffy::FlexDirection, flex_wrap: taffy::FlexWrap,
        gap: taffy::Size<taffy::LengthPercentage>, align_content: taffy::AlignContent,
        justify_content: taffy::JustifyContent, align_items: taffy::AlignItems
    );
}
impl FlexboxItemStyle for CheckedStyle<'_> {
    getters!(FlexboxItemStyle;
        flex_basis: taffy::Dimension, flex_grow: f32, flex_shrink: f32, align_self: Option<taffy::AlignSelf>
    );
}
impl GridItemStyle for CheckedStyle<'_> {
    getters!(GridItemStyle;
        grid_row: taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>>,
        grid_column: taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>>,
        align_self: Option<taffy::AlignSelf>, justify_self: Option<taffy::AlignSelf>
    );
}

impl GridContainerStyle for CheckedStyle<'_> {
    type Repetition<'a>
        = CheckedRepeat<'a>
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = CheckedTemplate<'a>
    where
        Self: 'a;
    type AutoTrackList<'a>
        = CheckedTracks<'a>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = CheckedNames<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = CheckedAreas<'a>
    where
        Self: 'a;
    getters!(GridContainerStyle;
        grid_auto_flow: taffy::GridAutoFlow, gap: taffy::Size<taffy::LengthPercentage>,
        align_content: taffy::AlignContent, justify_content: taffy::JustifyContent,
        align_items: taffy::AlignItems, justify_items: taffy::JustifyItems,
        grid_template_area_row_count: u16, grid_template_area_column_count: u16
    );
    fn grid_template_rows(&self) -> Option<CheckedTemplate<'_>> {
        let diag = self.begin("GridContainerStyle::grid_template_rows");
        let old = GridContainerStyle::grid_template_rows(self.legacy);
        let view = self
            .view
            .as_ref()
            .and_then(|view| GridContainerStyle::grid_template_rows(view));
        if self.view.is_some() {
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedTemplate::new(old, view, diag))
    }
    fn grid_template_columns(&self) -> Option<CheckedTemplate<'_>> {
        let diag = self.begin("GridContainerStyle::grid_template_columns");
        let old = GridContainerStyle::grid_template_columns(self.legacy);
        let view = self
            .view
            .as_ref()
            .and_then(|view| GridContainerStyle::grid_template_columns(view));
        if self.view.is_some() {
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedTemplate::new(old, view, diag))
    }
    fn grid_auto_rows(&self) -> CheckedTracks<'_> {
        CheckedIter::new(
            GridContainerStyle::grid_auto_rows(self.legacy),
            self.view.as_ref().map(|view| view.grid_auto_rows()),
            self.begin("GridContainerStyle::grid_auto_rows"),
        )
    }
    fn grid_auto_columns(&self) -> CheckedTracks<'_> {
        CheckedIter::new(
            GridContainerStyle::grid_auto_columns(self.legacy),
            self.view.as_ref().map(|view| view.grid_auto_columns()),
            self.begin("GridContainerStyle::grid_auto_columns"),
        )
    }
    fn grid_template_column_names(&self) -> Option<CheckedNames<'_>> {
        let diag = self.begin("GridContainerStyle::grid_template_column_names");
        let old = GridContainerStyle::grid_template_column_names(self.legacy);
        let view = self
            .view
            .as_ref()
            .and_then(|view| GridContainerStyle::grid_template_column_names(view));
        if self.view.is_some() {
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedNames::new(old, view, diag))
    }
    fn grid_template_row_names(&self) -> Option<CheckedNames<'_>> {
        let diag = self.begin("GridContainerStyle::grid_template_row_names");
        let old = GridContainerStyle::grid_template_row_names(self.legacy);
        let view = self
            .view
            .as_ref()
            .and_then(|view| GridContainerStyle::grid_template_row_names(view));
        if self.view.is_some() {
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedNames::new(old, view, diag))
    }
    fn grid_template_areas(&self) -> Option<CheckedAreas<'_>> {
        let diag = self.begin("GridContainerStyle::grid_template_areas");
        let old = GridContainerStyle::grid_template_areas(self.legacy);
        let view = self
            .view
            .as_ref()
            .and_then(|view| GridContainerStyle::grid_template_areas(view));
        if self.view.is_some() {
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedIter::new(old, view, diag))
    }
}

#[cfg(test)]
mod tests;

impl BlockContainerStyle for CheckedStyle<'_> {
    query_getters!(BlockContainerStyle;
        text_align: taffy::TextAlign, align_content: taffy::AlignContent, justify_items: taffy::AlignItems
    );
}
impl BlockItemStyle for CheckedStyle<'_> {
    query_getters!(BlockItemStyle;
        is_table: bool, align_self: Option<taffy::AlignSelf>, justify_self: Option<taffy::AlignSelf>,
        align_content: taffy::AlignContent, float: taffy::Float, clear: taffy::Clear
    );
}
impl OofItemStyle for CheckedStyle<'_> {
    query_getters!(OofItemStyle;
        align_self: Option<taffy::AlignSelf>, justify_self: Option<taffy::AlignSelf>, is_table: bool,
        grid_row: taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>>,
        grid_column: taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>>
    );
}
