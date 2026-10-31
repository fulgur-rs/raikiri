use raikiri_style::property::WritingMode;

/// A physical axis before projection into layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum Axis {
    /// Physical width or columns.
    X,
    /// Physical height or rows.
    Y,
}

/// A scalar length property, including preparation-only logical sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum LengthProperty {
    /// The width value.
    Width,
    /// The height value.
    Height,
    /// The min width value.
    MinWidth,
    /// The min height value.
    MinHeight,
    /// The max width value.
    MaxWidth,
    /// The max height value.
    MaxHeight,
    /// The margin top value.
    MarginTop,
    /// The margin right value.
    MarginRight,
    /// The margin bottom value.
    MarginBottom,
    /// The margin left value.
    MarginLeft,
    /// The padding top value.
    PaddingTop,
    /// The padding right value.
    PaddingRight,
    /// The padding bottom value.
    PaddingBottom,
    /// The padding left value.
    PaddingLeft,
    /// The border top value.
    BorderTop,
    /// The border right value.
    BorderRight,
    /// The border bottom value.
    BorderBottom,
    /// The border left value.
    BorderLeft,
    /// The top value.
    Top,
    /// The right value.
    Right,
    /// The bottom value.
    Bottom,
    /// The left value.
    Left,
    /// The flex basis value.
    FlexBasis,
    /// The column gap value.
    ColumnGap,
    /// The row gap value.
    RowGap,
    /// The logical width value.
    LogicalWidth,
    /// The logical height value.
    LogicalHeight,
    /// The min block value.
    MinBlock,
}
impl LengthProperty {
    /// Whether the property is read from its scalar as is: no logical alias,
    /// no keyword policy and no source substitution applies to it.
    #[inline]
    pub(crate) fn reads_scalar_directly(self) -> bool {
        !matches!(
            self,
            Self::Width
                | Self::Height
                | Self::MinWidth
                | Self::MinHeight
                | Self::MaxWidth
                | Self::MaxHeight
                | Self::MinBlock
        )
    }
    /// The value a non-atomic inline box in a horizontal writing mode takes
    /// for a property that does not apply to it, or `None` for one that does.
    ///
    /// The sizes are `auto`; the vertical margins, padding and border are
    /// zero, so they neither move the box nor change the height of the
    /// lines around it (CSS 2.1 sections 8.3, 10.3.1 and 10.6.1).
    pub(crate) fn inline_box_value(self) -> Option<raikiri_style::ComputedLengthPercentageOrAuto> {
        use raikiri_style::ComputedLengthPercentageOrAuto as L;
        match self {
            Self::Width
            | Self::Height
            | Self::MinWidth
            | Self::MinHeight
            | Self::MaxWidth
            | Self::MaxHeight => Some(L::Auto),
            Self::MarginTop
            | Self::MarginBottom
            | Self::PaddingTop
            | Self::PaddingBottom
            | Self::BorderTop
            | Self::BorderBottom => Some(L::Px(0.0)),
            _ => None,
        }
    }
    pub(crate) fn non_negative(self) -> bool {
        !matches!(
            self,
            Self::MarginTop
                | Self::MarginRight
                | Self::MarginBottom
                | Self::MarginLeft
                | Self::Top
                | Self::Right
                | Self::Bottom
                | Self::Left
        )
    }
}

/// Node-dependent facts computed by the DOM adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct StyleFlags(u32);
impl StyleFlags {
    /// Prepared block policy.
    pub const BLOCK: Self = Self(1 << 0);
    /// Prepared compressible replaced policy.
    pub const COMPRESSIBLE_REPLACED: Self = Self(1 << 1);
    /// Prepared replaced policy.
    pub const REPLACED: Self = Self(1 << 2);
    /// Prepared hidden policy.
    pub const HIDDEN: Self = Self(1 << 3);
    /// Prepared ignore width keyword policy.
    pub const IGNORE_WIDTH_KEYWORD: Self = Self(1 << 4);
    /// Prepared ignore height keyword policy.
    pub const IGNORE_HEIGHT_KEYWORD: Self = Self(1 << 5);
    /// Prepared ignore limit keywords policy.
    pub const IGNORE_LIMIT_KEYWORDS: Self = Self(1 << 6);
    /// Prepared min content width auto policy.
    pub const MIN_CONTENT_WIDTH_AUTO: Self = Self(1 << 7);
    /// Prepared ignore logical min block policy.
    pub const IGNORE_LOGICAL_MIN_BLOCK: Self = Self(1 << 8);
    /// Prepared vertical logical size policy.
    pub const VERTICAL_LOGICAL_SIZE: Self = Self(1 << 9);
    /// Prepared contain width policy.
    pub const CONTAIN_WIDTH: Self = Self(1 << 10);
    /// Prepared contain height policy.
    pub const CONTAIN_HEIGHT: Self = Self(1 << 11);
    /// Direction follows the source box's vertical axes even for physical size queries.
    pub const SOURCE_VERTICAL_DIRECTION: Self = Self(1 << 12);
    /// Prepared inline formatting context.
    pub const IFC: Self = Self(1 << 13);
    /// Prepared flex formatting context.
    pub const FLEX: Self = Self(1 << 14);
    /// Prepared grid formatting context.
    pub const GRID: Self = Self(1 << 15);
    /// Prepared table formatting context.
    pub const TABLE: Self = Self(1 << 16);
    /// Prepared multicol formatting context.
    pub const MULTICOL: Self = Self(1 << 17);
    /// Prepared text leaf classification.
    pub const TEXT: Self = Self(1 << 18);
    /// The legacy block item table identity.
    pub const BLOCK_TABLE: Self = Self(1 << 19);
    /// CSS table identity in out-of-flow layout.
    pub const OOF_TABLE: Self = Self(1 << 20);
    /// A non-atomic inline box in a horizontal writing mode: the sizes, the
    /// vertical margins and the vertical padding and border do not apply to
    /// it (CSS 2.1 sections 8.3, 10.3.1 and 10.6.1).
    pub const INLINE_BOX: Self = Self(1 << 21);
    /// A block-level box that keeps its border box clear of the margin boxes
    /// of the floats in its formatting context without establishing one: a
    /// replaced element, or a form control that behaves as one (CSS 2.1
    /// section 9.5).
    pub const AVOIDS_FLOATS: Self = Self(1 << 22);
    /// A used table-cell box, after blockification.
    pub const TABLE_CELL: Self = Self(1 << 23);
    /// No prepared facts.
    pub const EMPTY: Self = Self(0);
    /// Combine prepared facts.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    /// Remove prepared facts before publishing refreshed adapter inputs.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
    /// Whether a fact is set.
    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }
}
impl Default for StyleFlags {
    fn default() -> Self {
        Self::BLOCK
    }
}

/// Box facts independent of a particular layout invocation.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct StyleMetadata {
    /// Prepared identity and sizing policy.
    pub flags: StyleFlags,
    /// The box's own used writing mode.
    pub used_writing_mode: WritingMode,
    /// Explicit containing-block policy; None uses positioning.
    pub claims: Option<crate::values::ContainingBlockClaims>,
}
impl Default for StyleMetadata {
    fn default() -> Self {
        Self {
            flags: StyleFlags::default(),
            used_writing_mode: WritingMode::HorizontalTb,
            claims: None,
        }
    }
}

/// How percentage edges are resolved before reaching Taffy.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[doc(hidden)]
pub enum PercentBasis {
    /// Leave resolution to Taffy.
    #[default]
    Taffy,
    /// Resolve using this definite inline-size basis.
    Definite(f32),
    /// Resolve using an indefinite basis (percentage contributes zero).
    Indefinite,
}

/// Geometry conversion applied after the existing bridge policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub enum BridgeProjection {
    /// Preserve the bridge geometry.
    #[default]
    Identity,
    /// Preserve the existing vertical block snapshot conversion.
    BlockTranspose {
        /// Whether physical block-start is the right edge.
        right_to_left: bool,
    },
}

/// Containing-block policy selected by the consuming algorithm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub enum ClaimsPolicy {
    /// Use the node's prepared containing-block facts.
    #[default]
    Prepared,
    /// Preserve the legacy positioning-only trait result.
    Positioning,
}

/// The aspect ratio a flex item reports to the flex algorithm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub enum FlexItemRatio {
    /// The preferred aspect ratio of the box.
    #[default]
    Declared,
    /// None. The item states its main size and leaves its cross size `auto`,
    /// so the cross size follows from the main size the item ends up with once
    /// it has flexed. The flex algorithm would instead derive it from the main
    /// size the item states, which flexing may well change.
    CrossFollowsMain,
    /// The ratio the style states, even though the item states both sizes. A
    /// flex basis of `content` ignores the main size, so the flex base size
    /// comes from the cross size the item states and the ratio.
    BasisFromCross,
}

/// How a grid parent reads the preferred aspect ratio of an item.
#[derive(Clone, Copy, Debug, Default)]
#[doc(hidden)]
pub enum GridItemRatio {
    /// The preferred aspect ratio of the box.
    #[default]
    Declared,
    /// Leave the automatic height to the item's ratio and content at its
    /// final width. Preserve its resolved alignment when hiding the ratio
    /// from the parent, whose `normal` alignment would otherwise stretch it.
    MeasuredByItem {
        /// The resolved block-axis alignment.
        align_self: crate::values::AlignSelf,
        /// The resolved inline-axis alignment, leaving a normal width to the item.
        justify_self: crate::values::AlignSelf,
    },
}

/// The context in which an algorithm reads a box's style.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct ViewContext {
    /// Geometry conversion independent of writing-mode policy.
    pub bridge_projection: BridgeProjection,
    /// Fully resolved padding captured in the returned geometry.
    pub resolved_padding: Option<raikiri_layout_geometry::Edges<f32>>,
    /// Optional basis for only the returned top and bottom block-item edges.
    pub block_item_basis: Option<f32>,
    /// Containing-block policy for the consuming role.
    pub claims_policy: ClaimsPolicy,
    /// Axes returned by size and edge getters; horizontal preserves physical axes.
    pub layout_wm: WritingMode,
    /// Whether alignment is being queried along the inline axis.
    pub align_axis_is_inline: bool,
    /// Optional replacement basis for percentage padding and border edges.
    pub percent_basis: PercentBasis,
    /// The aspect ratio the view reports for a flex item.
    pub flex_item_ratio: FlexItemRatio,
    /// Whether a grid item's automatic height is measured by the item itself.
    pub grid_item_ratio: GridItemRatio,
}
impl Default for ViewContext {
    fn default() -> Self {
        Self {
            bridge_projection: BridgeProjection::Identity,
            resolved_padding: None,
            block_item_basis: None,
            claims_policy: ClaimsPolicy::Prepared,
            layout_wm: WritingMode::HorizontalTb,
            align_axis_is_inline: false,
            percent_basis: PercentBasis::Taffy,
            flex_item_ratio: FlexItemRatio::Declared,
            grid_item_ratio: GridItemRatio::Declared,
        }
    }
}
