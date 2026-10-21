use crate::overrides::OverrideKind;
use crate::*;
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::{
        BoxSizing, Contain, Direction, OverflowValue, PositionValue, TextOrientation, WritingMode,
    },
};
use taffy::CoreStyle;

impl CoreStyle for LayoutStyleView<'_> {
    type CustomIdent = raikiri_style::SmolStr;
    #[inline]
    fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        if self.flags().contains(StyleFlags::HIDDEN) {
            taffy::BoxGenerationMode::None
        } else {
            taffy::BoxGenerationMode::Normal
        }
    }
    #[inline]
    fn is_block(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK)
    }
    #[inline]
    fn is_compressible_replaced(&self) -> bool {
        self.flags().contains(StyleFlags::COMPRESSIBLE_REPLACED)
    }
    #[inline]
    fn is_replaced(&self) -> bool {
        self.flags().contains(StyleFlags::REPLACED)
    }
    #[inline]
    fn box_sizing(&self) -> taffy::BoxSizing {
        let c = &*self.prepared.values;
        let side_borders = c.border.left.width().px() > 0.0 || c.border.right.width().px() > 0.0;
        let vertical_clean = c.border.top.width().px() == 0.0
            && c.border.bottom.width().px() == 0.0
            && matches!(c.padding.top, P::Px(0.0) | P::Percent(0.0))
            && matches!(c.padding.bottom, P::Px(0.0) | P::Percent(0.0));
        if matches!(c.box_sizing, BoxSizing::BorderBox)
            || (matches!(self.base_length(LengthProperty::Width).value, L::Auto)
                && side_borders
                && vertical_clean)
        {
            taffy::BoxSizing::BorderBox
        } else {
            taffy::BoxSizing::ContentBox
        }
    }
    #[inline]
    fn direction(&self) -> taffy::Direction {
        let c = &*self.prepared.values;
        let vertical =
            self.vertical() || self.flags().contains(StyleFlags::SOURCE_VERTICAL_DIRECTION);
        let upright = vertical
            && c.text_orientation == TextOrientation::Upright
            && matches!(
                c.cssom_writing_mode,
                WritingMode::VerticalRl | WritingMode::VerticalLr
            );
        let rtl = c.direction == Direction::Rtl && !upright;
        let reversed = vertical && c.cssom_writing_mode == WritingMode::SidewaysLr;
        if rtl != reversed {
            taffy::Direction::Rtl
        } else {
            taffy::Direction::Ltr
        }
    }
    #[inline]
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        let get = |axis, base| {
            self.overrides()
                .find_map(|o| match o {
                    OverrideKind::Overflow(a, v) if a == axis => Some(v),
                    _ => None,
                })
                .unwrap_or(base)
        };
        let c = &*self.prepared.values;
        let (x, y) = (
            overflow(get(Axis::X, c.overflow.x)),
            overflow(get(Axis::Y, c.overflow.y)),
        );
        if self.vertical() {
            taffy::Point { x: y, y: x }
        } else {
            taffy::Point { x, y }
        }
    }
    #[inline]
    fn scrollbar_width(&self) -> f32 {
        0.0
    }
    #[inline]
    fn position(&self) -> taffy::Position {
        match self.prepared.values.position {
            PositionValue::Absolute | PositionValue::Fixed => taffy::Position::Absolute,
            PositionValue::Relative | PositionValue::Sticky => taffy::Position::Relative,
            _ => taffy::Position::Static,
        }
    }
    #[inline]
    fn is_containing_block(&self) -> taffy::ContainingBlockClaims {
        self.prepared
            .metadata
            .claims
            .unwrap_or(taffy::ContainingBlockClaims {
                absolute: self.position().is_positioned(),
                fixed: false,
            })
    }
    #[inline]
    fn size(&self) -> taffy::Size<taffy::Dimension> {
        self.size_axes(
            self.length(LengthProperty::Width).dimension(),
            self.length(LengthProperty::Height).dimension(),
        )
    }
    #[inline]
    fn min_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MinWidth).auto(false),
            self.length(LengthProperty::MinHeight).auto(false),
        )
    }
    #[inline]
    fn max_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MaxWidth).auto(false),
            self.length(LengthProperty::MaxHeight).auto(false),
        )
    }
    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        let c = &*self.prepared.values;
        let width = self.base_length(LengthProperty::Width).value;
        let height = self.base_length(LengthProperty::Height).value;
        let base = if matches!(width, L::Auto)
            || (!self.flags().contains(StyleFlags::CONTAIN_WIDTH) && width.is_content_keyword())
            || matches!(height, L::Auto)
        {
            c.aspect_ratio.preferred(None)
        } else {
            None
        };
        let ratio = self
            .overrides()
            .find_map(|o| match o {
                OverrideKind::AspectRatio(r) => Some(r),
                _ => None,
            })
            .unwrap_or(base);
        if self.vertical() {
            ratio.map(|r| 1.0 / r)
        } else {
            ratio
        }
    }
    #[inline]
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        use LengthProperty::*;
        self.edges(
            self.length(MarginTop).auto(true),
            self.length(MarginRight).auto(true),
            self.length(MarginBottom).auto(true),
            self.length(MarginLeft).auto(true),
        )
    }
    #[inline]
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        use LengthProperty::*;
        self.edges(
            self.length(Top).auto(false),
            self.length(Right).auto(false),
            self.length(Bottom).auto(false),
            self.length(Left).auto(false),
        )
    }
    #[inline]
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        use LengthProperty::*;
        let get = |p| self.length(p).lp(self.context.percent_basis);
        self.edges(
            get(PaddingTop),
            get(PaddingRight),
            get(PaddingBottom),
            get(PaddingLeft),
        )
    }
    #[inline]
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        use LengthProperty::*;
        let get = |p| self.length(p).lp(self.context.percent_basis);
        self.edges(
            get(BorderTop),
            get(BorderRight),
            get(BorderBottom),
            get(BorderLeft),
        )
    }
    #[inline]
    fn contain(&self) -> taffy::Contain {
        let c = self.prepared.values.contain;
        match (c.contains(Contain::LAYOUT), c.contains(Contain::PAINT)) {
            (true, true) => taffy::Contain::CONTENT,
            (true, false) => taffy::Contain::LAYOUT,
            (false, true) => taffy::Contain::PAINT,
            _ => taffy::Contain::NONE,
        }
    }
}
fn overflow(value: OverflowValue) -> taffy::Overflow {
    match value {
        OverflowValue::Hidden => taffy::Overflow::Hidden,
        OverflowValue::Clip => taffy::Overflow::Clip,
        OverflowValue::Auto | OverflowValue::Scroll => taffy::Overflow::Scroll,
        _ => taffy::Overflow::Visible,
    }
}
