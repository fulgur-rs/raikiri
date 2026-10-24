use crate::overrides::OverrideKind;
use crate::*;
use raikiri_style::{ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P};

/// A lightweight query over borrowed computed values and prepared context.
///
/// Taffy getter results may contain raw calc handles. Their owners must
/// remain alive until the algorithm has finished resolving those handles.
#[derive(Clone, Copy)]
pub struct LayoutStyleView<'a> {
    pub(crate) prepared: &'a PreparedStyle<'a>,
    pub(crate) context: ViewContext,
    pub(crate) use_prepared_claims: bool,
    pass: OverrideLayer<'a>,
    scoped: OverrideLayer<'a>,
    /// Whether either layer holds an entry: most queries find none and skip
    /// the scan.
    has_overrides: bool,
}
#[derive(Clone, Copy)]
enum OverrideLayer<'a> {
    Borrowed(&'a [PropertyOverride<'a>]),
    Owned(&'a [OwnedPropertyOverride]),
}
impl<'a> OverrideLayer<'a> {
    fn len(self) -> usize {
        match self {
            Self::Borrowed(values) => values.len(),
            Self::Owned(values) => values.len(),
        }
    }
    fn get(self, index: usize) -> PropertyOverride<'a> {
        match self {
            Self::Borrowed(values) => values[index],
            Self::Owned(values) => values[index].as_borrowed(),
        }
    }
}
impl<'a> LayoutStyleView<'a> {
    /// Construct a query without cloning, scanning or allocating computed style.
    #[inline]
    pub const fn new(
        prepared: &'a PreparedStyle<'a>,
        context: ViewContext,
        pass: &'a [PropertyOverride<'a>],
        scoped: &'a [PropertyOverride<'a>],
    ) -> Self {
        Self {
            prepared,
            context,
            use_prepared_claims: true,
            pass: OverrideLayer::Borrowed(pass),
            scoped: OverrideLayer::Borrowed(scoped),
            has_overrides: !pass.is_empty() || !scoped.is_empty(),
        }
    }
    /// Query owning override layers without allocating borrowed entry arrays.
    #[inline]
    pub const fn with_owned_overrides(
        prepared: &'a PreparedStyle<'a>,
        context: ViewContext,
        pass: &'a [OwnedPropertyOverride],
        scoped: &'a [OwnedPropertyOverride],
    ) -> Self {
        Self {
            prepared,
            context,
            use_prepared_claims: true,
            pass: OverrideLayer::Owned(pass),
            scoped: OverrideLayer::Owned(scoped),
            has_overrides: !pass.is_empty() || !scoped.is_empty(),
        }
    }
    /// Preserve positioning-only containing-block claims for Flex/Grid queries.
    ///
    /// The generic view retains prepared claims for consumers that previously
    /// read the node's containing-block policy through the Core container path.
    #[inline]
    pub const fn for_flex_grid(mut self) -> Self {
        self.use_prepared_claims = false;
        self
    }

    /// The query-local context used by diagnostics and role-specific adapters.
    #[inline]
    pub const fn context(&self) -> ViewContext {
        self.context
    }

    /// Replace query-local context without preparing or allocating input.
    #[inline]
    pub const fn with_context(mut self, context: ViewContext) -> Self {
        self.context = context;
        self
    }

    /// Resolve a calc handle that a getter of this view returned.
    ///
    /// A handle is the address of a calc payload the view reads: one prepared
    /// with the computed values, or one an override brought. Any other address
    /// gives `None` and is never dereferenced, so a tool that compares getter
    /// results can resolve what it was handed without the unsafe contract of
    /// `resolve_calc`.
    #[doc(hidden)]
    pub fn resolve_calc_handle(&self, handle: *const (), basis: f32) -> Option<f32> {
        let from_overrides = self.overrides().find_map(|entry| match entry {
            OverrideKind::Length(_, value) => {
                value.payload.filter(|payload| payload.pointer() == handle)
            }
            _ => None,
        });
        from_overrides
            .or_else(|| self.prepared.payload_at(handle))
            .map(|payload| payload.resolve(basis))
    }

    pub(crate) fn overrides(&self) -> impl Iterator<Item = OverrideKind<'a>> {
        self.has_overrides
            .then(|| {
                (0..self.scoped.len())
                    .rev()
                    .map(|index| self.scoped.get(index).0)
                    .chain(
                        (0..self.pass.len())
                            .rev()
                            .map(|index| self.pass.get(index).0),
                    )
            })
            .into_iter()
            .flatten()
    }
    #[inline]
    pub(crate) fn flags(&self) -> StyleFlags {
        self.prepared.metadata.flags
    }
    #[inline]
    pub(crate) fn vertical(&self) -> bool {
        self.context.layout_wm.is_vertical()
    }
    #[inline]
    pub(crate) fn length(&self, property: LengthProperty) -> LengthValue<'a> {
        if self.has_overrides
            && let Some(value) = self.overrides().find_map(|entry| match entry {
                OverrideKind::Length(p, v) if p == property => Some(v),
                _ => None,
            })
        {
            return value;
        }
        self.base_length(property)
    }
    #[inline]
    pub(crate) fn base_length(&self, property: LengthProperty) -> LengthValue<'a> {
        use LengthProperty::*;
        if property.reads_scalar_directly() {
            let value = self.prepared.scalar(property);
            return self.with_payload(value, property);
        }
        let flags = self.flags();
        let mut source = property;
        if flags.contains(StyleFlags::VERTICAL_LOGICAL_SIZE)
            && self.prepared.values.vertical_logical_size.is_some()
        {
            source = match property {
                Width => LogicalWidth,
                Height => LogicalHeight,
                _ => property,
            };
        }
        if property == MinHeight
            && !self.vertical()
            && !flags.contains(StyleFlags::SOURCE_VERTICAL_DIRECTION)
            && self.prepared.values.cssom_writing_mode.is_vertical()
            && self.prepared.values.min_height == L::Auto
            && self.prepared.values.min_block_size.is_some()
        {
            source = MinBlock;
        }
        let mut value = self.prepared.scalar(source);
        let intrinsic = matches!(
            value,
            L::MinContent | L::MaxContent | L::FitContent | L::FitContentOf(_)
        );
        let ignore = match property {
            Width => {
                (flags.contains(StyleFlags::IGNORE_WIDTH_KEYWORD) && intrinsic)
                    || (flags.contains(StyleFlags::MIN_CONTENT_WIDTH_AUTO)
                        && value == L::MinContent)
            }
            Height => flags.contains(StyleFlags::IGNORE_HEIGHT_KEYWORD) && intrinsic,
            MinWidth | MinHeight | MaxWidth | MaxHeight => {
                flags.contains(StyleFlags::IGNORE_LIMIT_KEYWORDS) && intrinsic
            }
            _ => false,
        } || (source == MinBlock
            && flags.contains(StyleFlags::IGNORE_LOGICAL_MIN_BLOCK));
        if ignore {
            value = L::Auto;
        }
        self.with_payload(value, source)
    }
    /// `value` with the calc owner prepared for `slot`, when it is a calc.
    #[inline]
    fn with_payload(&self, value: L, slot: LengthProperty) -> LengthValue<'a> {
        let payload = match value {
            L::Calc(_) | L::FitContentOf(P::Calc(_)) => {
                Some(self.prepared.payload(CalcSlot::Property(slot)))
            }
            _ => None,
        };
        LengthValue {
            value,
            payload,
            resolved: false,
        }
    }
    pub(crate) fn geometry_transposed(&self) -> bool {
        self.vertical()
            != matches!(
                self.context.bridge_projection,
                BridgeProjection::BlockTranspose { .. }
            )
    }
    pub(crate) fn size_axes<T>(&self, width: T, height: T) -> taffy::Size<T> {
        if self.geometry_transposed() {
            taffy::Size {
                width: height,
                height: width,
            }
        } else {
            taffy::Size { width, height }
        }
    }
    pub(crate) fn edges<T>(&self, top: T, right: T, bottom: T, left: T) -> taffy::Rect<T> {
        use raikiri_style::property::WritingMode::*;
        let rect = match self.context.layout_wm {
            VerticalRl | SidewaysRl => taffy::Rect {
                left: top,
                right: bottom,
                top: right,
                bottom: left,
            },
            VerticalLr | SidewaysLr => taffy::Rect {
                left: top,
                right: bottom,
                top: left,
                bottom: right,
            },
            _ => taffy::Rect {
                top,
                right,
                bottom,
                left,
            },
        };
        match self.context.bridge_projection {
            BridgeProjection::Identity => rect,
            BridgeProjection::BlockTranspose { right_to_left } => {
                let (top, bottom) = if right_to_left {
                    (rect.right, rect.left)
                } else {
                    (rect.left, rect.right)
                };
                taffy::Rect {
                    left: rect.top,
                    right: rect.bottom,
                    top,
                    bottom,
                }
            }
        }
    }

    pub(crate) fn percentage_edges(
        &self,
        properties: [LengthProperty; 4],
    ) -> taffy::Rect<taffy::LengthPercentage> {
        // Place the input positions first so each length is read straight into
        // the side it ends up on.
        let from = self.edges(0usize, 1, 2, 3);
        let vertical_basis = match (self.context.percent_basis, self.context.block_item_basis) {
            (PercentBasis::Taffy, Some(basis)) => PercentBasis::Definite(basis),
            (basis, _) => basis,
        };
        taffy::Rect {
            top: self.length(properties[from.top]).lp(vertical_basis),
            bottom: self.length(properties[from.bottom]).lp(vertical_basis),
            left: self
                .length(properties[from.left])
                .lp(self.context.percent_basis),
            right: self
                .length(properties[from.right])
                .lp(self.context.percent_basis),
        }
    }
}
