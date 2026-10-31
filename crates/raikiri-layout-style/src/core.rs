use crate::overrides::OverrideKind;
use crate::values::{self, Dimension, LengthPercentage, LengthPercentageAuto};
use crate::*;
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::{
        BoxSizing, Contain, Direction, OverflowValue, PositionValue, TextOrientation, WritingMode,
    },
};

impl LayoutStyleView<'_> {
    /// Whether the box generates a layout box at all.
    #[inline]
    pub fn box_generation_mode(&self) -> values::BoxGenerationMode {
        if self.flags().contains(StyleFlags::HIDDEN) {
            values::BoxGenerationMode::None
        } else {
            values::BoxGenerationMode::Normal
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
    /// Whether the box keeps its border box clear of the floats in its
    /// formatting context although it does not establish one.
    #[inline]
    pub fn avoids_floats(&self) -> bool {
        self.flags().contains(StyleFlags::AVOIDS_FLOATS)
    }
    /// The used box-sizing, including the border-box exception for side borders.
    #[inline]
    pub fn box_sizing(&self) -> values::BoxSizing {
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
            values::BoxSizing::BorderBox
        } else {
            values::BoxSizing::ContentBox
        }
    }
    /// The used inline direction of the box.
    #[inline]
    pub fn direction(&self) -> values::Direction {
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
            values::Direction::Rtl
        } else {
            values::Direction::Ltr
        }
    }
    /// The used overflow of both axes, in the axes the view returns.
    #[inline]
    pub fn overflow(&self) -> Point<values::Overflow> {
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
            Point { x: y, y: x }
        } else {
            Point { x, y }
        }
    }
    /// The used position scheme.
    #[inline]
    pub fn position(&self) -> values::Position {
        match self.prepared.values.position {
            PositionValue::Absolute | PositionValue::Fixed => values::Position::Absolute,
            PositionValue::Relative => values::Position::Relative,
            PositionValue::Sticky => values::Position::Sticky,
            _ => values::Position::Static,
        }
    }
    /// Which kinds of absolutely positioned descendants this box contains.
    #[inline]
    pub fn is_containing_block(&self) -> values::ContainingBlockClaims {
        let positioning = values::ContainingBlockClaims {
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
    pub fn size(&self) -> Size<Dimension> {
        self.size_axes(
            self.length(LengthProperty::Width).dimension(),
            self.library_height(LengthProperty::Height).dimension(),
        )
    }
    /// The minimum size.
    #[inline]
    pub fn min_size(&self) -> Size<LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MinWidth).auto(false),
            self.library_height(LengthProperty::MinHeight).auto(false),
        )
    }
    /// The maximum size.
    #[inline]
    pub fn max_size(&self) -> Size<LengthPercentageAuto> {
        self.size_axes(
            self.length(LengthProperty::MaxWidth).auto(false),
            self.library_height(LengthProperty::MaxHeight).auto(false),
        )
    }
    /// The preferred aspect ratio after projection into the returned axes.
    #[inline]
    pub fn aspect_ratio(&self) -> Option<f32> {
        match self.context.flex_item_ratio {
            FlexItemRatio::CrossFollowsMain => return None,
            FlexItemRatio::BasisFromCross => return self.stated_aspect_ratio(),
            FlexItemRatio::Declared => {}
        }
        // A percentage size that resolves leaves the ratio nothing to do,
        // except that the ratio carries a `min-*` or `max-*` size across to the
        // other axis and clamps the size stated there. Only a box without those
        // keeps the ratio for the measures in which the percentage is cyclic.
        self.projected_aspect_ratio(true, || !self.states_min_or_max_size())
    }
    /// The preferred aspect ratio as block flow reads it.
    ///
    /// A percentage size keeps the ratio even when the box states a `min-*` or
    /// `max-*` size. Block flow knows whether the percentage resolves, and it
    /// carries a bound across the ratio only into an axis whose size is `auto`,
    /// so a percentage that resolves leaves the ratio nothing to do, while one
    /// that does not still gets the size the ratio gives and the bounds that
    /// clamp it.
    #[inline]
    pub fn block_aspect_ratio(&self) -> Option<f32> {
        self.projected_aspect_ratio(true, || true)
    }
    /// The preferred aspect ratio the style states, after projection into the
    /// returned axes, whether or not the sizes the box states leave it anything
    /// to do.
    #[inline]
    pub fn stated_aspect_ratio(&self) -> Option<f32> {
        self.projected_aspect_ratio(false, || true)
    }
    /// [`Self::aspect_ratio`], where `sizes_matter` says whether the ratio is
    /// dropped for a box that states both sizes, and `keep_for_percentage` says
    /// whether a percentage size keeps the ratio. The latter is asked only when
    /// a percentage size is what makes the box keep it.
    fn projected_aspect_ratio(
        &self,
        sizes_matter: bool,
        keep_for_percentage: impl FnOnce() -> bool,
    ) -> Option<f32> {
        let c = &*self.prepared.values;
        let ratio = if let Some(value) = self.overrides().find_map(|entry| match entry {
            OverrideKind::AspectRatio(value) => Some(value),
            _ => None,
        }) {
            value
        } else if !sizes_matter {
            c.aspect_ratio.preferred(None)
        } else {
            let width = self.base_length(LengthProperty::Width).value;
            let height = self.base_length(LengthProperty::Height).value;
            let contained = self.flags().contains(StyleFlags::CONTAIN_WIDTH);
            preferred_aspect_ratio_with(c.aspect_ratio, width, height, None, contained, false)
                .or_else(|| {
                    preferred_aspect_ratio_with(
                        c.aspect_ratio,
                        width,
                        height,
                        None,
                        contained,
                        true,
                    )
                    .filter(|_| keep_for_percentage())
                })
        };
        if self.geometry_transposed() {
            ratio.map(|r| 1.0 / r)
        } else {
            ratio
        }
    }
    /// Whether the box states a minimum or maximum size in either axis.
    fn states_min_or_max_size(&self) -> bool {
        let (min, max) = (self.min_size(), self.max_size());
        !(min.width.is_auto()
            && min.height.is_auto()
            && max.width.is_auto()
            && max.height.is_auto())
    }
    /// The margin edges; `auto` is preserved.
    #[inline]
    pub fn margin(&self) -> Edges<LengthPercentageAuto> {
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
    pub fn inset(&self) -> Edges<LengthPercentageAuto> {
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
    pub fn padding(&self) -> Edges<LengthPercentage> {
        if let Some(padding) = self.context.resolved_padding {
            return padding.map(LengthPercentage::length);
        }
        use LengthProperty::*;
        self.percentage_edges([PaddingTop, PaddingRight, PaddingBottom, PaddingLeft])
    }
    /// The border edges.
    #[inline]
    pub fn border(&self) -> Edges<LengthPercentage> {
        use LengthProperty::*;
        self.percentage_edges([BorderTop, BorderRight, BorderBottom, BorderLeft])
    }
    /// The layout and paint containment.
    #[inline]
    pub fn contain(&self) -> values::Contain {
        let c = self.prepared.values.contain;
        match (c.contains(Contain::LAYOUT), c.contains(Contain::PAINT)) {
            (true, true) => values::Contain::CONTENT,
            (true, false) => values::Contain::LAYOUT,
            (false, true) => values::Contain::PAINT,
            _ => values::Contain::NONE,
        }
    }
}
pub(crate) fn overflow(value: OverflowValue) -> values::Overflow {
    match value {
        OverflowValue::Hidden => values::Overflow::Hidden,
        OverflowValue::Clip => values::Overflow::Clip,
        OverflowValue::Auto | OverflowValue::Scroll => values::Overflow::Scroll,
        _ => values::Overflow::Visible,
    }
}

impl LayoutStyleView<'_> {
    /// The physical clearance, including fragmentation-pass overrides.
    pub fn clear(&self) -> values::Clear {
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

pub(crate) fn clear(value: raikiri_style::property::ClearValue) -> values::Clear {
    use raikiri_style::property::ClearValue as C;
    match value {
        C::Left => values::Clear::Left,
        C::Right => values::Clear::Right,
        C::Both => values::Clear::Both,
        _ => values::Clear::None,
    }
}

/// Preferred ratio after automatic-size and containment policy, before axis projection.
#[doc(hidden)]
pub fn preferred_aspect_ratio(
    ratio: raikiri_style::property::AspectRatioValue,
    width: L,
    height: L,
    natural: Option<f32>,
    size_contained: bool,
) -> Option<f32> {
    preferred_aspect_ratio_with(ratio, width, height, natural, size_contained, true)
}

/// [`preferred_aspect_ratio`], where `cyclic_percentage` says whether a
/// percentage size counts as `auto`: it is cyclic while an intrinsic size is
/// measured and then behaves as `auto` (CSS Sizing 3 §5.2.1). A definite basis
/// resolves both sizes and leaves the ratio nothing to do.
fn preferred_aspect_ratio_with(
    ratio: raikiri_style::property::AspectRatioValue,
    width: L,
    height: L,
    natural: Option<f32>,
    size_contained: bool,
    cyclic_percentage: bool,
) -> Option<f32> {
    let cyclic = |size: L| match size {
        L::Auto => true,
        L::Percent(_) => cyclic_percentage,
        L::Calc(calc) => cyclic_percentage && calc.percent != 0.0,
        _ => false,
    };
    // A `stretch` height fills a containing block that may have no definite
    // height, and then behaves as `auto`: like a percentage, it leaves the
    // ratio its work where it does not resolve. A `stretch` width always
    // resolves, so the ratio has nothing to do for it.
    let stretch_height = cyclic_percentage && matches!(height, L::Stretch);
    if cyclic(width)
        || (natural.is_none() && !size_contained && width.is_content_keyword())
        || cyclic(height)
        || stretch_height
    {
        ratio.preferred(natural)
    } else {
        None
    }
}
