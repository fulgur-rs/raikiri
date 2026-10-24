//! The projection of a [`LayoutStyleView`] onto Taffy's style traits.
//!
//! [`LayoutStyleView`] answers every style query in the layout engine's own
//! terms. Taffy's algorithms read the same facts through its style traits, so
//! [`TaffyStyle`] implements those traits for a view by forwarding each getter
//! to the view's and converting the value to the library's. The used values
//! themselves are computed once, in the view.
//!
//! This module and its children are the only code of the crate that names the
//! layout library's types: the conversions, the grid track lists that Taffy
//! reads through its own traits, the flex and grid getters that answer in
//! Taffy's vocabulary, and the compact form that style lengths are stored in.

mod convert;
mod flex;
mod grid;
mod length;
#[cfg(test)]
mod tests;

pub use convert::ToTaffy;
pub use grid::{Areas, GridRepetition, LineNames, TemplateTracks, Tracks};
pub use length::{Dimension, LengthPercentage, LengthPercentageAuto};

use crate::{BoxStyle, LayoutStyleView};
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

/// A [`BoxStyle`] as the style Taffy reads to measure a leaf.
#[derive(Clone, Copy)]
pub struct LeafStyle<'a>(&'a BoxStyle);

impl<'a> LeafStyle<'a> {
    /// Project `style` onto Taffy's core style trait.
    #[inline]
    pub const fn new(style: &'a BoxStyle) -> Self {
        Self(style)
    }
}

impl CoreStyle for LeafStyle<'_> {
    type CustomIdent = SmolStr;

    #[inline]
    fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        self.0.box_generation_mode().to_taffy()
    }
    #[inline]
    fn is_block(&self) -> bool {
        self.0.is_block()
    }
    #[inline]
    fn is_compressible_replaced(&self) -> bool {
        self.0.is_compressible_replaced()
    }
    #[inline]
    fn is_replaced(&self) -> bool {
        self.0.item_is_replaced
    }
    #[inline]
    fn box_sizing(&self) -> taffy::BoxSizing {
        self.0.box_sizing.to_taffy()
    }
    #[inline]
    fn direction(&self) -> taffy::Direction {
        self.0.direction.to_taffy()
    }
    #[inline]
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        self.0.overflow.to_taffy()
    }
    #[inline]
    fn position(&self) -> taffy::Position {
        self.0.position.to_taffy()
    }
    #[inline]
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.0.inset.to_taffy()
    }
    #[inline]
    fn size(&self) -> taffy::Size<taffy::Dimension> {
        self.0.size.to_taffy()
    }
    #[inline]
    fn min_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.0.min_size.to_taffy()
    }
    #[inline]
    fn max_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.0.max_size.to_taffy()
    }
    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        self.0.aspect_ratio
    }
    #[inline]
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.0.margin.to_taffy()
    }
    #[inline]
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.0.padding.to_taffy()
    }
    #[inline]
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.0.border.to_taffy()
    }
    #[inline]
    fn contain(&self) -> taffy::Contain {
        self.0.contain.to_taffy()
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

/// Forward getters to the view's getters of the same name, which answer in the
/// engine's own types, and convert the answer to the library's.
macro_rules! convert {
    ($($name:ident -> $ret:ty),+ $(,)?) => {$(
        #[inline]
        fn $name(&self) -> $ret {
            self.0.$name().to_taffy()
        }
    )+};
}

impl CoreStyle for TaffyStyle<'_> {
    type CustomIdent = SmolStr;
    forward! {
        is_block -> bool,
        is_compressible_replaced -> bool,
        is_replaced -> bool,
        aspect_ratio -> Option<f32>,
    }
    convert! {
        box_generation_mode -> taffy::BoxGenerationMode,
        box_sizing -> taffy::BoxSizing,
        direction -> taffy::Direction,
        overflow -> taffy::Point<taffy::Overflow>,
        position -> taffy::Position,
        is_containing_block -> taffy::ContainingBlockClaims,
        size -> taffy::Size<taffy::Dimension>,
        min_size -> taffy::Size<taffy::LengthPercentageAuto>,
        max_size -> taffy::Size<taffy::LengthPercentageAuto>,
        margin -> taffy::Rect<taffy::LengthPercentageAuto>,
        inset -> taffy::Rect<taffy::LengthPercentageAuto>,
        padding -> taffy::Rect<taffy::LengthPercentage>,
        border -> taffy::Rect<taffy::LengthPercentage>,
        contain -> taffy::Contain,
    }
}

impl BlockContainerStyle for TaffyStyle<'_> {
    convert! {
        align_content -> taffy::AlignContent,
        justify_items -> taffy::AlignItems,
    }
}

impl BlockItemStyle for TaffyStyle<'_> {
    forward! {
        is_table -> bool,
    }
    convert! {
        float -> taffy::Float,
        clear -> taffy::Clear,
    }
}

impl OofItemStyle for TaffyStyle<'_> {
    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_align_self().to_taffy()
    }
    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_justify_self().to_taffy()
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
    }
    convert! {
        gap -> taffy::Size<taffy::LengthPercentage>,
        align_content -> taffy::AlignContent,
        justify_content -> taffy::JustifyContent,
        align_items -> taffy::AlignItems,
    }
}

impl FlexboxItemStyle for TaffyStyle<'_> {
    forward! {
        flex_grow -> f32,
        flex_shrink -> f32,
    }
    convert! {
        flex_basis -> taffy::Dimension,
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
    }
    convert! {
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
        self.0.bridge_align_self().to_taffy()
    }
    #[inline]
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.0.bridge_justify_self().to_taffy()
    }
    forward! {
        grid_row -> taffy::Line<taffy::GridPlacement<SmolStr>>,
        grid_column -> taffy::Line<taffy::GridPlacement<SmolStr>>,
    }
}
