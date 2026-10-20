use raikiri_style::property::WritingMode;

/// A physical axis before projection into layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Physical width or columns.
    X,
    /// Physical height or rows.
    Y,
}

/// A scalar length property, including preparation-only logical sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub(crate) const ALL: &'static [Self] = &[
        Self::Width,
        Self::Height,
        Self::MinWidth,
        Self::MinHeight,
        Self::MaxWidth,
        Self::MaxHeight,
        Self::MarginTop,
        Self::MarginRight,
        Self::MarginBottom,
        Self::MarginLeft,
        Self::PaddingTop,
        Self::PaddingRight,
        Self::PaddingBottom,
        Self::PaddingLeft,
        Self::BorderTop,
        Self::BorderRight,
        Self::BorderBottom,
        Self::BorderLeft,
        Self::Top,
        Self::Right,
        Self::Bottom,
        Self::Left,
        Self::FlexBasis,
        Self::ColumnGap,
        Self::RowGap,
        Self::LogicalWidth,
        Self::LogicalHeight,
        Self::MinBlock,
    ];
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
    /// No prepared facts.
    pub const EMPTY: Self = Self(0);
    /// Combine prepared facts.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
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
pub struct StyleMetadata {
    /// Prepared identity and sizing policy.
    pub flags: StyleFlags,
    /// The box's own used writing mode.
    pub used_writing_mode: WritingMode,
    /// Explicit containing-block policy; None uses positioning.
    pub claims: Option<taffy::ContainingBlockClaims>,
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
pub enum PercentBasis {
    /// Leave resolution to Taffy.
    #[default]
    Taffy,
    /// Resolve using this definite inline-size basis.
    Definite(f32),
    /// Resolve using an indefinite basis (percentage contributes zero).
    Indefinite,
}

/// The context in which an algorithm reads a box's style.
#[derive(Clone, Copy, Debug)]
pub struct ViewContext {
    /// Axes returned by size and edge getters; horizontal preserves physical axes.
    pub layout_wm: WritingMode,
    /// Whether alignment is being queried along the inline axis.
    pub align_axis_is_inline: bool,
    /// Optional replacement basis for percentage padding and border edges.
    pub percent_basis: PercentBasis,
}
impl Default for ViewContext {
    fn default() -> Self {
        Self {
            layout_wm: WritingMode::HorizontalTb,
            align_axis_is_inline: false,
            percent_basis: PercentBasis::Taffy,
        }
    }
}
