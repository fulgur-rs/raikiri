//! The projection of a [`LayoutStyleView`] onto Taffy's style traits.
//!
//! [`LayoutStyleView`] answers every style query in the layout engine's own
//! terms. Taffy's algorithms read the same facts through its style traits, so
//! [`TaffyStyle`] implements those traits for a view by forwarding each getter
//! to the view's. The used values themselves are computed once, in the view.

use crate::{Areas, GridRepetition, LayoutStyleView, LineNames, TemplateTracks, Tracks};
use raikiri_style::SmolStr;
use taffy::{
    BlockContainerStyle, BlockItemStyle, CoreStyle, FlexboxContainerStyle, FlexboxItemStyle,
    GridContainerStyle, GridItemStyle, OofItemStyle,
};

/// A [`LayoutStyleView`] as the style Taffy's algorithms read.
#[derive(Clone, Copy)]
pub struct TaffyStyle<'a>(LayoutStyleView<'a>);

impl<'a> TaffyStyle<'a> {
    /// Project `view` onto Taffy's style traits.
    #[inline]
    pub const fn new(view: LayoutStyleView<'a>) -> Self {
        Self(view)
    }
}

/// Forward getters of a Taffy style trait to the view's getters of the same name.
macro_rules! forward {
    ($($name:ident -> $ret:ty),+ $(,)?) => {$(
        #[inline]
        fn $name(&self) -> $ret {
            self.0.$name()
        }
    )+};
}

impl CoreStyle for TaffyStyle<'_> {
    type CustomIdent = SmolStr;
    forward! {
        box_generation_mode -> taffy::BoxGenerationMode,
        is_block -> bool,
        is_compressible_replaced -> bool,
        is_replaced -> bool,
        box_sizing -> taffy::BoxSizing,
        direction -> taffy::Direction,
        overflow -> taffy::Point<taffy::Overflow>,
        position -> taffy::Position,
        is_containing_block -> taffy::ContainingBlockClaims,
        size -> taffy::Size<taffy::Dimension>,
        min_size -> taffy::Size<taffy::LengthPercentageAuto>,
        max_size -> taffy::Size<taffy::LengthPercentageAuto>,
        aspect_ratio -> Option<f32>,
        margin -> taffy::Rect<taffy::LengthPercentageAuto>,
        inset -> taffy::Rect<taffy::LengthPercentageAuto>,
        padding -> taffy::Rect<taffy::LengthPercentage>,
        border -> taffy::Rect<taffy::LengthPercentage>,
        contain -> taffy::Contain,
    }
}

impl BlockContainerStyle for TaffyStyle<'_> {
    forward! {
        align_content -> taffy::AlignContent,
        justify_items -> taffy::AlignItems,
    }
}

impl BlockItemStyle for TaffyStyle<'_> {
    forward! {
        is_table -> bool,
        float -> taffy::Float,
        clear -> taffy::Clear,
    }
}

impl OofItemStyle for TaffyStyle<'_> {
    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_align_self()
    }
    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_justify_self()
    }
    #[inline]
    fn is_table(&self) -> bool {
        self.0.is_oof_table()
    }
    forward! {
        grid_row -> taffy::Line<taffy::GridPlacement<SmolStr>>,
        grid_column -> taffy::Line<taffy::GridPlacement<SmolStr>>,
    }
}

impl FlexboxContainerStyle for TaffyStyle<'_> {
    forward! {
        flex_direction -> taffy::FlexDirection,
        flex_wrap -> taffy::FlexWrap,
        gap -> taffy::Size<taffy::LengthPercentage>,
        align_content -> taffy::AlignContent,
        justify_content -> taffy::JustifyContent,
        align_items -> taffy::AlignItems,
    }
}

impl FlexboxItemStyle for TaffyStyle<'_> {
    forward! {
        flex_basis -> taffy::Dimension,
        flex_grow -> f32,
        flex_shrink -> f32,
        align_self -> Option<taffy::AlignSelf>,
    }
}

impl GridContainerStyle for TaffyStyle<'_> {
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
    forward! {
        grid_template_rows -> Option<TemplateTracks<'_>>,
        grid_template_columns -> Option<TemplateTracks<'_>>,
        grid_auto_rows -> Tracks<'_>,
        grid_auto_columns -> Tracks<'_>,
        grid_template_column_names -> Option<LineNames<'_>>,
        grid_template_row_names -> Option<LineNames<'_>>,
        grid_template_areas -> Option<Areas<'_>>,
        grid_template_area_row_count -> u16,
        grid_template_area_column_count -> u16,
        grid_auto_flow -> taffy::GridAutoFlow,
        gap -> taffy::Size<taffy::LengthPercentage>,
        align_content -> taffy::AlignContent,
        justify_content -> taffy::JustifyContent,
        align_items -> taffy::AlignItems,
        justify_items -> taffy::AlignItems,
    }
}

impl GridItemStyle for TaffyStyle<'_> {
    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_align_self()
    }
    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_justify_self()
    }
    forward! {
        grid_row -> taffy::Line<taffy::GridPlacement<SmolStr>>,
        grid_column -> taffy::Line<taffy::GridPlacement<SmolStr>>,
    }
}
