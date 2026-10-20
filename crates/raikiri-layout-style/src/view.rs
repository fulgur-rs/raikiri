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
    pass: &'a [PropertyOverride<'a>],
    scoped: &'a [PropertyOverride<'a>],
}
impl<'a> LayoutStyleView<'a> {
    /// Construct a query without cloning, scanning or allocating computed style.
    pub const fn new(
        prepared: &'a PreparedStyle<'a>,
        context: ViewContext,
        pass: &'a [PropertyOverride<'a>],
        scoped: &'a [PropertyOverride<'a>],
    ) -> Self {
        Self {
            prepared,
            context,
            pass,
            scoped,
        }
    }
    pub(crate) fn overrides(&self) -> impl Iterator<Item = &OverrideKind<'a>> {
        self.scoped
            .iter()
            .rev()
            .chain(self.pass.iter().rev())
            .map(|entry| &entry.0)
    }
    pub(crate) fn flags(&self) -> StyleFlags {
        self.prepared.metadata.flags
    }
    pub(crate) fn vertical(&self) -> bool {
        self.context.layout_wm.is_vertical()
    }
    pub(crate) fn length(&self, property: LengthProperty) -> LengthValue<'a> {
        if let Some(value) = self.overrides().find_map(|entry| match entry {
            OverrideKind::Length(p, v) if *p == property => Some(*v),
            _ => None,
        }) {
            return value;
        }
        self.base_length(property)
    }
    pub(crate) fn base_length(&self, property: LengthProperty) -> LengthValue<'a> {
        use LengthProperty::*;
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
        } || (property == MinHeight
            && flags.contains(StyleFlags::IGNORE_LOGICAL_MIN_BLOCK));
        if ignore {
            value = L::Auto;
        }
        let payload = match value {
            L::Calc(_) | L::FitContentOf(P::Calc(_)) => {
                Some(self.prepared.payload(CalcSlot::Property(source)))
            }
            _ => None,
        };
        LengthValue {
            value,
            payload,
            resolved: false,
        }
    }
    pub(crate) fn size_axes<T>(&self, width: T, height: T) -> taffy::Size<T> {
        if self.vertical() {
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
        match self.context.layout_wm {
            VerticalRl | SidewaysRl => taffy::Rect {
                left: top,
                right: bottom,
                top: right,
                bottom: left,
            },
            VerticalLr => taffy::Rect {
                left: top,
                right: bottom,
                top: left,
                bottom: right,
            },
            SidewaysLr => taffy::Rect {
                left: bottom,
                right: top,
                top: left,
                bottom: right,
            },
            _ => taffy::Rect {
                top,
                right,
                bottom,
                left,
            },
        }
    }
}
