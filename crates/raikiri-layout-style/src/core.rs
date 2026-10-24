use crate::overrides::OverrideKind;
use crate::*;
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::{
        BoxSizing, Contain, Direction, OverflowValue, PositionValue, TextOrientation, WritingMode,
    },
};

impl LayoutStyleView<'_> {
    /// Whether the box generates a layout box at all.
    #[inline]
    pub fn box_generation_mode(&self) -> taffy::BoxGenerationMode {
        if self.flags().contains(StyleFlags::HIDDEN) {
            taffy::BoxGenerationMode::None
        } else {
            taffy::BoxGenerationMode::Normal
        }
    }
    /// Whether the box lays out as a block-level box.
    #[inline]
    pub fn is_block(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK)
    }
    /// Whether the box is a replaced element whose automatic size may shrink.
    #[inline]
    pub fn is_compressible_replaced(&self) -> bool {
        self.flags().contains(StyleFlags::COMPRESSIBLE_REPLACED)
    }
    /// Whether the box is a replaced element.
    #[inline]
    pub fn is_replaced(&self) -> bool {
        self.flags().contains(StyleFlags::REPLACED)
    }
    /// The used box-sizing, including the border-box exception for side borders.
    #[inline]
    pub fn box_sizing(&self) -> taffy::BoxSizing {
        let c = &*self.prepared.values;
        let side_borders = c.border.left.width().px() > 0.0 || c.border.right.width().px() > 0.0;
        let vertical_clean = c.border.top.width().px() == 0.0
            && c.border.bottom.width().px() == 0.0
            && matches!(c.padding.top, P::Px(0.0) | P::Percent(0.0))
            && matches!(c.padding.bottom, P::Px(0.0) | P::Percent(0.0));
        if matches!(c.box_sizing, BoxSizing::BorderBox)
            || (side_borders
                && vertical_clean
                && matches!(self.base_length(LengthProperty::Width).value, L::Auto))
        {
            taffy::BoxSizing::BorderBox
        } else {
            taffy::BoxSizing::ContentBox
        }
    }
    /// The used inline direction of the box.
    #[inline]
    pub fn direction(&self) -> taffy::Direction {
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
    /// The used overflow of both axes, in the axes the view returns.
    #[inline]
    pub fn overflow(&self) -> taffy::Point<taffy::Overflow> {
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
        if self.geometry_transposed() {
            taffy::Point { x: y, y: x }
        } else {
            taffy::Point { x, y }
        }
    }
    /// The used position scheme.
    #[inline]
    pub fn position(&self) -> taffy::Position {
        match self.prepared.values.position {
            PositionValue::Absolute | PositionValue::Fixed => taffy::Position::Absolute,
            PositionValue::Relative => taffy::Position::Relative,
            PositionValue::Sticky => taffy::Position::Sticky,
            _ => taffy::Position::Static,
        }
    }
    /// Which kinds of absolutely positioned descendants this box contains.
    #[inline]
    pub fn is_containing_block(&self) -> taffy::ContainingBlockClaims {
        let positioning = taffy::ContainingBlockClaims {
            absolute: self.position().is_positioned(),
            fixed: false,
        };
        if self.use_prepared_claims && self.context.claims_policy == ClaimsPolicy::Prepared {
            self.prepared.metadata.claims.unwrap_or(positioning)
        } else {
            positioning
        }
    }
    /// The preferred size.
    #[inline]
    pub fn size(&self) -> taffy::Size<taffy::Dimension> {
        self.size_axes(
            self.length(LengthProperty::Width).dimension(),
            self.length(LengthProperty::Height).dimension(),
        )
    }
    /// The minimum size.
    #[inline]
    pub fn min_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MinWidth).auto(false),
            self.length(LengthProperty::MinHeight).auto(false),
        )
    }
    /// The maximum size.
    #[inline]
    pub fn max_size(&self) -> taffy::Size<taffy::LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MaxWidth).auto(false),
            self.length(LengthProperty::MaxHeight).auto(false),
        )
    }
    /// The preferred aspect ratio after projection into the returned axes.
    #[inline]
    pub fn aspect_ratio(&self) -> Option<f32> {
        let c = &*self.prepared.values;
        let ratio = if let Some(value) = self.overrides().find_map(|entry| match entry {
            OverrideKind::AspectRatio(value) => Some(value),
            _ => None,
        }) {
            value
        } else {
            preferred_aspect_ratio(
                c.aspect_ratio,
                self.base_length(LengthProperty::Width).value,
                self.base_length(LengthProperty::Height).value,
                None,
                self.flags().contains(StyleFlags::CONTAIN_WIDTH),
            )
        };
        if self.geometry_transposed() {
            ratio.map(|r| 1.0 / r)
        } else {
            ratio
        }
    }
    /// The margin edges; `auto` is preserved.
    #[inline]
    pub fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        use LengthProperty::*;
        self.edges(
            self.length(MarginTop).auto(true),
            self.length(MarginRight).auto(true),
            self.length(MarginBottom).auto(true),
            self.length(MarginLeft).auto(true),
        )
    }
    /// The inset edges.
    #[inline]
    pub fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        use LengthProperty::*;
        self.edges(
            self.length(Top).auto(false),
            self.length(Right).auto(false),
            self.length(Bottom).auto(false),
            self.length(Left).auto(false),
        )
    }
    /// The padding edges.
    #[inline]
    pub fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        if let Some(padding) = self.context.resolved_padding {
            return padding.map(taffy::LengthPercentage::length);
        }
        use LengthProperty::*;
        self.percentage_edges([PaddingTop, PaddingRight, PaddingBottom, PaddingLeft])
    }
    /// The border edges.
    #[inline]
    pub fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        use LengthProperty::*;
        self.percentage_edges([BorderTop, BorderRight, BorderBottom, BorderLeft])
    }
    /// The layout and paint containment.
    #[inline]
    pub fn contain(&self) -> taffy::Contain {
        let c = self.prepared.values.contain;
        match (c.contains(Contain::LAYOUT), c.contains(Contain::PAINT)) {
            (true, true) => taffy::Contain::CONTENT,
            (true, false) => taffy::Contain::LAYOUT,
            (false, true) => taffy::Contain::PAINT,
            _ => taffy::Contain::NONE,
        }
    }
}
pub(crate) fn overflow(value: OverflowValue) -> taffy::Overflow {
    match value {
        OverflowValue::Hidden => taffy::Overflow::Hidden,
        OverflowValue::Clip => taffy::Overflow::Clip,
        OverflowValue::Auto | OverflowValue::Scroll => taffy::Overflow::Scroll,
        _ => taffy::Overflow::Visible,
    }
}

impl LayoutStyleView<'_> {
    /// The physical clearance, including fragmentation-pass overrides.
    pub fn clear(&self) -> taffy::Clear {
        let value = self
            .overrides()
            .find_map(|entry| match entry {
                OverrideKind::Clear(value) => Some(value),
                _ => None,
            })
            .unwrap_or(self.prepared.values.clear);
        clear(value)
    }
}

pub(crate) fn clear(value: raikiri_style::property::ClearValue) -> taffy::Clear {
    use raikiri_style::property::ClearValue as C;
    match value {
        C::Left => taffy::Clear::Left,
        C::Right => taffy::Clear::Right,
        C::Both => taffy::Clear::Both,
        _ => taffy::Clear::None,
    }
}

/// Preferred ratio after automatic-size and containment policy, before axis projection.
pub fn preferred_aspect_ratio(
    ratio: raikiri_style::property::AspectRatioValue,
    width: L,
    height: L,
    natural: Option<f32>,
    size_contained: bool,
) -> Option<f32> {
    if matches!(width, L::Auto)
        || (natural.is_none() && !size_contained && width.is_content_keyword())
        || matches!(height, L::Auto)
    {
        ratio.preferred(natural)
    } else {
        None
    }
}
