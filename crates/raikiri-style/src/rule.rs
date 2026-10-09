//! Style rules and declaration parsing shared by stylesheet and inline styles.
//! Nested rules and at-rules are handled by the rule-tree parsers.

use std::fmt;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::RaikiriSelectorImpl;
use crate::cascade::rollback::{Rollback, rollback_kind};
use crate::consumer::{ConsumerPropertyGrammar, ConsumerPropertyRegistration};
use crate::property::{
    BackgroundShorthand, Border, BorderColor, BorderStyle, CustomProperty, DeferredValue, FlexFlow,
    FlexShorthand, FontFeatureSettings, FontKerning, FontLanguageOverride, FontOpticalSizing,
    FontShorthand, FontShorthandSize, FontVariantEastAsian, FontVariantEmoji, FontVariantLigatures,
    FontVariantNumeric, FontVariantPosition, FontVariationSettings, GapShorthand,
    GridLineShorthand, Length, LengthOrAuto, Outline, OverflowXY, PlaceContentShorthand,
    PlaceItemsShorthand, PlaceSelfShorthand, PropertyKey, PropertyValue, Sides, StartEnd,
    TextDecorationShorthand, TextEmphasisShorthand, consume_deferred_value,
    parse_consumer_text_value, parse_value,
};

/// One property declaration = value + `!important` flag.
///
/// Because `value` is private, external crates cannot construct this through
/// either a struct literal or functional update.
///
/// # Compile-fail check for the absence of a write path
///
/// `value` is crate-private, so external consumers can read it through
/// [`Declaration::value`] but cannot construct or replace it directly. The
/// following fences document that API boundary:
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Declaration, PropertyValue};
///
/// let _ = Declaration {
///     value: PropertyValue::Color(CssColor::BLACK),
///     important: false,
/// };
/// ```
///
/// Construction through functional update (`..base`) is rejected for the same
/// reason (`value` is private).
///
/// Only the third fence below (field assignment after `.clone()`) separately
/// isolates the visibility of `value`:
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Declaration, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let base = tree.style_rules()[0].declarations()[0].clone();
///
/// let _ = Declaration {
///     value: PropertyValue::Color(CssColor::BLACK),
///     ..base
/// };
/// ```
///
/// A cloned declaration still cannot replace its private value:
///
/// ```compile_fail
/// use raikiri_style::{CssColor, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let mut decl = tree.style_rules()[0].declarations()[0].clone();
/// decl.value = PropertyValue::Color(CssColor::BLACK);
/// ```
///
/// A compiling control keeps this example tied to the public read API:
///
/// ```
/// use raikiri_style::{CssColor, Origin, PropertyValue, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("p { color: red; }", Origin::Author);
/// let decl = tree.style_rules()[0].declarations()[0].clone();
/// assert_eq!(
///     decl.value(),
///     &PropertyValue::Color(CssColor {
///         r: 255,
///         g: 0,
///         b: 0,
///         a: 255
///     })
/// );
/// assert!(!decl.important, "`color: red` に `!important` は付かない");
/// let _ = PropertyValue::Color(CssColor::BLACK);
/// ```
///
/// Every declaration is a longhand or a value the cascade keeps in shorthand
/// form (see [`classify`]): the parsers produce a [`ParsedDeclaration`], and
/// [`expand_shorthand_into`] turns it into the declarations it stands for.
#[derive(Clone, PartialEq)]
pub struct Declaration {
    /// Resolved property value.
    pub(crate) value: PropertyValue,
    /// `!important` flag (true increases importance).
    pub important: bool,
    /// The cascade slot of `value`, computed once from it.
    pub(crate) key: PropertyKey,
    /// How `value` rolls the cascade back (`revert`, `revert-layer`), computed
    /// once from it.
    pub(crate) rollback: Rollback,
}

// The cascade clones one declaration per matched candidate.
const _: () = assert!(
    std::mem::size_of::<Declaration>() <= 152,
    "Declaration grew past 152 bytes: raise the bound together with a cascade memory measurement"
);

impl Declaration {
    /// An expanded declaration of `value`.
    ///
    /// Shorthand expansion and the cascade's own declaration sources call this;
    /// a value whose key [`classify`] calls a shorthand never reaches the
    /// cascade, because it would occupy a slot no longhand reads.
    pub(crate) fn new(value: PropertyValue, important: bool) -> Self {
        let key = value.key();
        debug_assert!(
            !matches!(classify(key), KeyClass::Shorthand { .. }),
            "{key:?} is expanded into longhands and must not be declared as one"
        );
        Self {
            rollback: rollback_kind(&value),
            key,
            value,
            important,
        }
    }

    /// Changes the value in place, keeping `key` and `rollback` in step with
    /// it.
    pub(crate) fn update_value(&mut self, update: impl FnOnce(&mut PropertyValue)) {
        update(&mut self.value);
        *self = Self::new(
            std::mem::replace(&mut self.value, PropertyValue::AllRevertLayer),
            self.important,
        );
    }

    /// Read-only accessor for the resolved property value.
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

// `key` and `rollback` are functions of `value`, so the declared fields say
// everything.
impl fmt::Debug for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Declaration")
            .field("value", &self.value)
            .field("important", &self.important)
            .finish()
    }
}

/// A declaration as the parsers produce it, before shorthand expansion.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParsedDeclaration {
    pub(crate) value: PropertyValue,
    pub(crate) important: bool,
}

/// How the cascade treats a property key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyClass {
    /// A longhand: one cascade slot.
    Longhand,
    /// A shorthand that expansion replaces with these longhands, in this
    /// order, so it never becomes a cascade slot itself.
    Shorthand { longhands: &'static [PropertyKey] },
    /// A shorthand whose value the cascade keeps in shorthand form as one
    /// candidate when it cannot be expanded at parse time (a `var()` value),
    /// resolving it into longhands only after substitution.
    Retained,
}

/// The [`KeyClass`] of `key`.
///
/// The match has no wildcard arm, so a new [`PropertyKey`] needs an explicit
/// decision here. The longhand lists are also the order in which a deferred
/// (`var()`) shorthand value is expanded.
pub(crate) const fn classify(key: PropertyKey) -> KeyClass {
    use PropertyKey::*;
    let longhands: &'static [PropertyKey] = match key {
        BorderRadius => &[
            BorderRadiusTopLeft,
            BorderRadiusTopRight,
            BorderRadiusBottomRight,
            BorderRadiusBottomLeft,
        ],
        Padding => &[PaddingTop, PaddingRight, PaddingBottom, PaddingLeft],
        Margin => &[MarginTop, MarginRight, MarginBottom, MarginLeft],
        MarginInline => &[MarginLeft, MarginRight],
        MarginBlock => &[MarginTop, MarginBottom],
        PaddingInline => &[PaddingLeft, PaddingRight],
        PaddingBlock => &[PaddingTop, PaddingBottom],
        Border => &[
            BorderTopWidth,
            BorderTopStyle,
            BorderTopColor,
            BorderRightWidth,
            BorderRightStyle,
            BorderRightColor,
            BorderBottomWidth,
            BorderBottomStyle,
            BorderBottomColor,
            BorderLeftWidth,
            BorderLeftStyle,
            BorderLeftColor,
        ],
        BorderTop => &[BorderTopWidth, BorderTopStyle, BorderTopColor],
        BorderRight => &[BorderRightWidth, BorderRightStyle, BorderRightColor],
        BorderBottom => &[BorderBottomWidth, BorderBottomStyle, BorderBottomColor],
        BorderLeft => &[BorderLeftWidth, BorderLeftStyle, BorderLeftColor],
        BorderStyle => &[
            BorderTopStyle,
            BorderRightStyle,
            BorderBottomStyle,
            BorderLeftStyle,
        ],
        BorderWidth => &[
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
        ],
        BorderColor => &[
            BorderTopColor,
            BorderRightColor,
            BorderBottomColor,
            BorderLeftColor,
        ],
        ListStyle => &[ListStyleType, ListStylePosition, ListStyleImage],
        Overflow => &[OverflowX, OverflowY],
        TextDecoration => &[
            TextDecorationLine,
            TextDecorationThickness,
            TextDecorationStyle,
            TextDecorationColor,
        ],
        TextEmphasis => &[TextEmphasisStyle, TextEmphasisColor],
        Outline => &[OutlineWidth, OutlineStyle, OutlineColor],
        // `font` sets 6 grammar longhands plus 10 reset-only subproperties.
        // `font-variant-caps` is already a grammar longhand; `size` is one
        // key whether absolute or relative.
        Font => &[
            FontStyle,
            FontVariantCaps,
            FontWeight,
            FontSize,
            LineHeight,
            FontFamily,
            FontKerning,
            FontLanguageOverride,
            FontOpticalSizing,
            FontVariantEastAsian,
            FontVariantEmoji,
            FontVariantLigatures,
            FontVariantNumeric,
            FontVariantPosition,
            FontVariationSettings,
            FontFeatureSettings,
        ],
        Flex => &[FlexGrow, FlexShrink, FlexBasis],
        FlexFlow => &[FlexDirection, FlexWrap],
        Columns => &[ColumnWidth, ColumnCount],
        Gap => &[RowGap, ColumnGap],
        PlaceContent => &[AlignContent, JustifyContent],
        GridRow => &[GridRowStart, GridRowEnd],
        GridColumn => &[GridColumnStart, GridColumnEnd],
        PlaceItems => &[AlignItems, JustifyItems],
        PlaceSelf => &[AlignSelf, JustifySelf],
        // `BackgroundShorthand`'s own field order. Each key lands in its own
        // `SpecifiedValues` field, so the order does not affect the result.
        Background => &[
            BackgroundColor,
            BackgroundImage,
            BackgroundRepeat,
            BackgroundAttachment,
            BackgroundPosition,
            BackgroundSize,
            BackgroundClip,
            BackgroundOrigin,
        ],
        All | Grid | GridArea | WhiteSpace | TextWrap | TextSpacing => return KeyClass::Retained,
        Color
        | BackgroundColor
        | FontFamily
        | FontSize
        | FontWeight
        | LineHeight
        | Display
        | CounterReset
        | CounterIncrement
        | CounterSet
        | Content
        | StringSet
        | Position
        | Top
        | Right
        | Bottom
        | Left
        | TextAlign
        | TextIndent
        | PaddingTop
        | PaddingRight
        | PaddingBottom
        | PaddingLeft
        | MarginTop
        | MarginRight
        | MarginBottom
        | MarginLeft
        | BorderTopWidth
        | BorderRightWidth
        | BorderBottomWidth
        | BorderLeftWidth
        | BorderTopStyle
        | BorderRightStyle
        | BorderBottomStyle
        | BorderLeftStyle
        | BorderTopColor
        | BorderRightColor
        | BorderBottomColor
        | BorderLeftColor
        | Width
        | Height
        | MaxWidth
        | MaxHeight
        | MinWidth
        | MinHeight
        | BoxSizing
        | Direction
        | OverflowX
        | OverflowY
        | TextDecorationLine
        | TextDecorationStyle
        | TextDecorationColor
        | VerticalAlign
        | FontStyle
        | TextTransform
        | Visibility
        | ZIndex
        | WordBreak
        | OverflowWrap
        | LetterSpacing
        | WordSpacing
        | BreakBefore
        | BreakAfter
        | BreakInside
        | Float
        | Clear
        | FlexDirection
        | FlexWrap
        | FlexGrow
        | FlexShrink
        | FlexBasis
        | Order
        | JustifyContent
        | AlignContent
        | AlignItems
        | AlignSelf
        | RowGap
        | ColumnGap
        | Hyphens
        | TabSize
        | FontVariantCaps
        | Quotes
        | TextShadow
        | GridTemplateColumns
        | GridTemplateRows
        | GridTemplateAreas
        | GridAutoColumns
        | GridAutoRows
        | GridAutoFlow
        | GridRowStart
        | GridRowEnd
        | GridColumnStart
        | GridColumnEnd
        | JustifyItems
        | JustifySelf
        | Orphans
        | Widows
        | Custom
        | BorderRadiusTopLeft
        | BorderRadiusTopRight
        | BorderRadiusBottomRight
        | BorderRadiusBottomLeft
        | BoxShadow
        | OutlineWidth
        | OutlineStyle
        | OutlineColor
        | OutlineOffset
        | WritingMode
        | RubyPosition
        | BackgroundRepeat
        | BackgroundAttachment
        | BackgroundClip
        | BackgroundOrigin
        | BackgroundSize
        | BackgroundPosition
        | BackgroundImage
        | ObjectFit
        | ObjectPosition
        | Opacity
        | Isolation
        | MixBlendMode
        | MaskImage
        | ClipPath
        | Transform
        | TransformOrigin
        | Filter
        | LineBreak
        | TextJustify
        | TextAlignAll
        | TextAlignLast
        | TextCombineUpright
        | TextOrientation
        | UnicodeBidi
        | TableLayout
        | BorderCollapse
        | BorderSpacing
        | CaptionSide
        | EmptyCells
        | TextDecorationSkipInk
        | TextDecorationSkipSpaces
        | TextDecorationThickness
        | TextDecorationInset
        | TextEmphasisPosition
        | TextUnderlinePosition
        | Page
        | ListStyleType
        | ListStylePosition
        | ListStyleImage
        | ColumnCount
        | ColumnWidth
        | MinBlockSize
        | TextUnderlineOffset
        | HangingPunctuation
        | TextAutospace
        | WhiteSpaceCollapse
        | TextWrapStyle
        | HyphenateCharacter
        | HyphenateLimitChars
        | TextSpacingTrim
        | WordSpaceTransform
        | TextEmphasisStyle
        | TextEmphasisColor
        | FontKerning
        | FontOpticalSizing
        | FontVariantEmoji
        | FontLanguageOverride
        | FontVariantLigatures
        | FontSynthesis
        | FontVariantPosition
        | FontPalette
        | FontVariantNumeric
        | FontVariantEastAsian
        | FontVariationSettings
        | ColumnFill
        | FontFeatureSettings
        | InlineSize
        | BlockSize
        | TextOverflow => {
            return KeyClass::Longhand;
        }
    };
    KeyClass::Shorthand { longhands }
}

/// Qualified style rule (`selectors { declarations }`).
///
/// `source_order` is numbered from 0 across a `RuleTree`. It breaks cascade ties
/// (later wins at equal specificity).
/// `origin` is the origin defined in CSS Cascading L4 §6.2. It determines the
/// rank in the cascade tuple (with reversed ordering for `!important`).
/// Future fields (specificity cache / invalidation hint, etc.) can be added
/// without breaking consumers thanks to `#[non_exhaustive]`.
#[non_exhaustive]
pub struct StyleRule {
    /// Parsed selector list. Only type + universal are accepted (others drop at build time).
    pub selectors: SelectorList<RaikiriSelectorImpl>,
    /// This rule's declaration list (excluding invalid declarations).
    pub(crate) declarations: Vec<Declaration>,
    /// Zero-indexed source order across the entire RuleTree.
    pub source_order: u32,
    /// The cascade origin to which this rule belongs.
    pub origin: crate::ruletree::Origin,
    /// Stable layer identity; its precedence depends on the media context.
    pub(crate) layer: Option<crate::layer::LayerId>,
}

impl StyleRule {
    /// Read-only accessor for this rule's declaration list
    /// (excluding invalid declarations).
    ///
    /// # Inaccessibility of the `declarations` field itself
    ///
    /// The `declarations` field is `pub(crate)`; external crates can only use
    /// this accessor. `StyleRule` does not derive `Clone`, so an external crate
    /// cannot obtain an owned `StyleRule` with `.clone()`. Also, `#[non_exhaustive]`
    /// prevents new construction with a struct literal or functional update.
    /// These are independent barriers: an external crate cannot obtain an owned
    /// `StyleRule` through either route. It can only access the `&[Declaration]`
    /// returned by this accessor. No check can meaningfully distinguish a
    /// write path here: no visibility permits writing through `&StyleRule`.
    /// The following instead checks the visibility of the field itself.
    /// Only this accessor is available externally. If the `declarations` field
    /// becomes `pub`, direct access to its name becomes possible and the
    /// compile-fail example below will compile:
    ///
    /// ```compile_fail
    /// use raikiri_style::{Origin, RuleTree};
    ///
    /// let mut tree = RuleTree::empty();
    /// tree.add_stylesheet("p { color: red; }", Origin::Author);
    /// let _ = &tree.style_rules()[0].declarations;
    /// ```
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
}

/// Consume a declaration list to produce `Vec<Declaration>`.
/// Silently drop unrecognized property names and invalid values.
///
/// # Shorthand expansion
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
/// property sets all of its longhand sub-properties, exactly as if expanded
/// in place." Accordingly, [`PropertyValue::Margin`] shorthand declarations
/// expand into four longhand declarations at the exit of this function. This
/// spec-correct expansion enables natural per-side winner selection in the
/// cascade; see the [`expand_shorthand_into`] docs for details.
pub(crate) fn parse_declaration_block(input: &mut Parser<'_, '_>) -> Vec<Declaration> {
    parse_declaration_block_with_consumer_properties(input, &[])
}

/// Parse a declaration block with an optional set of consumer-owned property
/// registrations.  The default wrapper above intentionally keeps all existing
/// callers on the zero-overhead path.
pub(crate) fn parse_declaration_block_with_consumer_properties(
    input: &mut Parser<'_, '_>,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Vec<Declaration> {
    let mut parser = DeclParser {
        consumer_properties,
    };
    let mut out = Vec::new();
    for decl in RuleBodyParser::new(input, &mut parser).flatten() {
        expand_shorthand_into(&decl, |d| out.push(d));
    }
    out
}

/// Expand a shorthand declaration into the longhand declarations it represents.
///
/// CSS Cascading Level 4 §3 defines a shorthand as setting its longhand
/// sub-properties as if they had been declared in place:
/// <https://www.w3.org/TR/css-cascade-4/#shorthand>.
///
/// The match is exhaustive on [`PropertyValue`]. Non-shorthand variants are
/// listed explicitly instead of using a wildcard, so adding a new variant
/// requires an explicit decision about its expansion here.
#[inline]
pub(crate) fn expand_shorthand_into(d: &ParsedDeclaration, mut push: impl FnMut(Declaration)) {
    let mut push_longhand = |value| push(Declaration::new(value, d.important));
    match d.value {
        PropertyValue::ListStyle(ref value) => {
            push_longhand(PropertyValue::ListStyleType(value.kind.clone()));
            push_longhand(PropertyValue::ListStylePosition(value.position));
            push_longhand(PropertyValue::ListStyleImage(value.image.clone()));
        }
        PropertyValue::BorderRadius(radius) => {
            push_longhand(PropertyValue::BorderRadiusTopLeft(radius.top_left));
            push_longhand(PropertyValue::BorderRadiusTopRight(radius.top_right));
            push_longhand(PropertyValue::BorderRadiusBottomRight(radius.bottom_right));
            push_longhand(PropertyValue::BorderRadiusBottomLeft(radius.bottom_left));
        }
        PropertyValue::BorderRadiusInherit => {
            for (key, property) in [
                (PropertyKey::BorderRadiusTopLeft, "border-top-left-radius"),
                (PropertyKey::BorderRadiusTopRight, "border-top-right-radius"),
                (PropertyKey::BorderRadiusBottomRight, "border-bottom-right-radius"),
                (PropertyKey::BorderRadiusBottomLeft, "border-bottom-left-radius"),
            ] {
                push_longhand(PropertyValue::Deferred(DeferredValue {
                    key,
                    property: property.into(),
                    value: "inherit".into(),
                }));
            }
        }
        PropertyValue::Margin(sides) => expand_margin(sides, push_longhand),
        PropertyValue::MarginInherit => expand_margin_inherit(push_longhand),
        PropertyValue::Padding(sides) => expand_padding(sides, push_longhand),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, push_longhand),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, push_longhand),
        PropertyValue::PaddingInline(pair) => expand_padding_inline(pair, push_longhand),
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, push_longhand),
        PropertyValue::Border(sides) => expand_border(sides, push_longhand),
        PropertyValue::BorderCssWide(kw) => expand_border_css_wide(kw, push_longhand),
        PropertyValue::BorderTop(sides) => expand_border_top(sides, push_longhand),
        PropertyValue::BorderTopCssWide(kw) => expand_border_top_css_wide(kw, push_longhand),
        PropertyValue::BorderRight(sides) => expand_border_right(sides, push_longhand),
        PropertyValue::BorderRightCssWide(kw) => expand_border_right_css_wide(kw, push_longhand),
        PropertyValue::BorderBottom(sides) => expand_border_bottom(sides, push_longhand),
        PropertyValue::BorderBottomCssWide(kw) => expand_border_bottom_css_wide(kw, push_longhand),
        PropertyValue::BorderLeft(sides) => expand_border_left(sides, push_longhand),
        PropertyValue::BorderLeftCssWide(kw) => expand_border_left_css_wide(kw, push_longhand),
        PropertyValue::BorderStyle(sides) => expand_border_style(sides, push_longhand),
        PropertyValue::BorderWidth(sides) => expand_border_width(sides, push_longhand),
        PropertyValue::BorderColor(sides) => expand_border_color(sides, push_longhand),
        PropertyValue::Overflow(pair) => expand_overflow(pair, push_longhand),
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, push_longhand)
        }
        PropertyValue::TextEmphasis(ref shorthand) => {
            expand_text_emphasis(shorthand, push_longhand)
        }
        PropertyValue::Outline(outline) => expand_outline(outline, push_longhand),
        PropertyValue::TextWrapShorthand(shorthand) => {
            push_longhand(PropertyValue::TextWrap(shorthand.mode));
            push_longhand(PropertyValue::TextWrapStyle(shorthand.style));
        }
        PropertyValue::TextSpacingShorthand(shorthand) => {
            push_longhand(PropertyValue::TextSpacingTrim(shorthand.trim));
            push_longhand(PropertyValue::TextAutospace(shorthand.autospace));
        }
        // `FontShorthand` owns `family: Arc<Vec<FontFamilyName>>` and is not `Copy`;
        // unlike other shorthand payloads (`Sides<..>` / `FlexShorthand`, etc., all `Copy`),
        // its value cannot be moved. Pass it by reference with a `ref` binding
        // (as for the `Background` arm; see the `GridRow` arm comment).
        PropertyValue::Font(ref shorthand) => expand_font(shorthand, push_longhand),
        PropertyValue::Deferred(ref deferred) => {
            expand_deferred(d, deferred, d.important, push)
        }
        // No longhand variant exists for expansion: push one declaration unchanged.
        // Do not collapse this into `_` (see "Why there is no wildcard arm (contract)" above).
        // Adding a variant here asserts that it has no expansion target.
        PropertyValue::AllRevertLayer
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::Color(_)
        | PropertyValue::ContextualColor(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::FontSize(_)
        | PropertyValue::FontSizeRelative(_)
        | PropertyValue::FontWeight(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::TextAutospace(_)
        | PropertyValue::WordSpaceTransform(_)
        | PropertyValue::TextSpacingTrim(_)
        | PropertyValue::TextIndent(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginTopInherit
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeft(_)
        | PropertyValue::MarginLeftInherit
        | PropertyValue::BorderTopWidth(_)
        | PropertyValue::BorderTopWidthCssWide(_)
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderRightWidthCssWide(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderBottomWidthCssWide(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderLeftWidthCssWide(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderTopStyleCssWide(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderRightStyleCssWide(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderBottomStyleCssWide(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderLeftStyleCssWide(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderTopColorCssWide(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderRightColorCssWide(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderBottomColorCssWide(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::BorderLeftColorCssWide(_)
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::MaxWidth(_)
        | PropertyValue::MaxHeight(_)
        | PropertyValue::MinWidth(_)
        | PropertyValue::MinHeight(_)
        | PropertyValue::MinBlockSize(_)
        | PropertyValue::InlineSize(_)
        | PropertyValue::BlockSize(_)
        | PropertyValue::Top(_)
        | PropertyValue::Right(_)
        | PropertyValue::Bottom(_)
        | PropertyValue::Left(_)
        | PropertyValue::BoxSizing(_)
        | PropertyValue::Direction(_)
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextDecorationThicknessInherit
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextDecorationInset(_)
        | PropertyValue::TextUnderlineOffset(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextEmphasisStyle(_)
        | PropertyValue::TextEmphasisColor(_)
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::VerticalAlign(_)
        | PropertyValue::FontStyle(_)
        | PropertyValue::FontKerning(_)
        | PropertyValue::FontOpticalSizing(_)
        | PropertyValue::FontVariantEmoji(_)
        | PropertyValue::FontLanguageOverride(_)
        | PropertyValue::FontVariantLigatures(_)
        | PropertyValue::FontSynthesis(_)
        | PropertyValue::FontVariantPosition(_)
        | PropertyValue::FontPalette(_)
        | PropertyValue::FontVariantNumeric(_)
        | PropertyValue::FontVariantEastAsian(_)
        | PropertyValue::FontVariationSettings(_)
        | PropertyValue::TextTransform(_)
        | PropertyValue::Visibility(_)
        | PropertyValue::ZIndex(_)
        | PropertyValue::WordBreak(_)
        | PropertyValue::OverflowWrap(_)
        | PropertyValue::LetterSpacing(_)
        | PropertyValue::WordSpacing(_)
        | PropertyValue::TabSize(_)
        // break-before / break-after / break-inside (+ legacy shorthand
        // page-break-before / page-break-after / page-break-inside, which
        // parse directly to the same variants — `BreakBetween`/
        // `BreakInside` docs' "legacy shorthand" sections explain why they
        // skip this function's fan-out machinery: each maps to exactly one
        // longhand, not several).
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        | PropertyValue::BreakInside(_)
        | PropertyValue::Float(_)
        | PropertyValue::Clear(_)
        | PropertyValue::WhiteSpace(_)
        | PropertyValue::WhiteSpaceCollapse(_)
        | PropertyValue::TextWrap(_)
        | PropertyValue::TextWrapStyle(_)
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        | PropertyValue::FlexBasis(_)
        | PropertyValue::Order(_)
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        | PropertyValue::RowGap(_)
        | PropertyValue::ColumnGap(_)
        // `hyphens` (CSS Text Module Level 3 §5.3) has no shorthand form.
        | PropertyValue::Hyphens(_)
        // `hyphenate-character` / `hyphenate-limit-chars` (CSS Text 4) are
        // standalone properties.
        | PropertyValue::HyphenateCharacter(_)
        | PropertyValue::HyphenateLimitChars(_)
        | PropertyValue::FontVariantCaps(_)
        | PropertyValue::Quotes(_)
        | PropertyValue::TextShadow(_)
        | PropertyValue::BorderRadiusTopLeft(_)
        | PropertyValue::BorderRadiusTopRight(_)
        | PropertyValue::BorderRadiusBottomRight(_)
        | PropertyValue::BorderRadiusBottomLeft(_)
        | PropertyValue::BoxShadow(_)
        | PropertyValue::OutlineWidth(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::OutlineOffset(_)
        // grid-template-columns/-rows/-areas + grid-auto-columns/-rows/-flow
        // + grid-row-start/-end + grid-column-start/-end: individual properties
        // with no corresponding expansion longhands (CSS Grid Layout Module Level 1
        // §7.2/§7.3/§7.6/§7.7/§8.3).
        | PropertyValue::GridTemplateColumns(_)
        | PropertyValue::GridTemplateRows(_)
        | PropertyValue::GridTemplateAreas(_)
        | PropertyValue::GridAutoColumns(_)
        | PropertyValue::GridAutoRows(_)
        | PropertyValue::GridAutoFlow(_)
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        // justify-items / justify-self (CSS Box Alignment Module Level 3
        // §7.1/§6.1): likewise have no expansion longhands.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        // orphans / widows (CSS Fragmentation Module Level 3 §3.3): likewise have no
        // expansion longhands.
        | PropertyValue::Orphans(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Widows(_)
        // writing-mode (CSS Writing Modes 4 §3.2): likewise has no expansion
        // longhands.
        | PropertyValue::WritingMode(_)
        | PropertyValue::RubyPosition(_)
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position /
        // background-image (CSS Backgrounds and Borders 3 §2.3-§2.9) —
        // Likewise have no expansion longhands (the `background` shorthand has
        // a separate `PropertyValue::Background` variant, expanded by the
        // `expand_background` arm below).
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)
        | PropertyValue::BackgroundSize(_)
        | PropertyValue::BackgroundPosition(_)
        | PropertyValue::BackgroundImage(_)
        // object-fit / object-position (CSS Images Module Level 3 §5.1/§5.2)
        // / opacity (CSS Color 4 §3.3): likewise have no expansion longhands
        // (standalone properties without shorthands).
        | PropertyValue::ObjectFit(_)
        | PropertyValue::ObjectPosition(_)
        | PropertyValue::TransformOrigin(..)
        | PropertyValue::Opacity(_)
        // isolation / mix-blend-mode (CSS Compositing and Blending Level 1
        // §3.4.2/§3.4.1) — same shape as object-fit/opacity above.
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)
        // mask-image / clip-path (CSS Masking Level 1 §7.1/§5.1) — same
        // shape as isolation/mix-blend-mode above (`mask`/`mask-border`
        // shorthands stay unimplemented, pre-existing silent drop).
        | PropertyValue::MaskImage(_)
        | PropertyValue::ClipPath(_)
        // transform (CSS Transforms Level 1 §4) / filter (CSS Filter
        // Effects Level 1 §5) — same shape as mask-image/clip-path above.
        | PropertyValue::Transform(_)
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::TextOverflow(_)
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_) | PropertyValue::TextCombineUpright(_) | PropertyValue::TextOrientation(_) | PropertyValue::UnicodeBidi(_) | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(_)
        | PropertyValue::ColumnFill(_)
        | PropertyValue::FontFeatureSettings(_) => expand_none(d, push),
        PropertyValue::Columns(shorthand) => {
            push_longhand(PropertyValue::ColumnWidth(shorthand.width));
            push_longhand(PropertyValue::ColumnCount(shorthand.count));
        }
        PropertyValue::Flex(f) => expand_flex(f, push_longhand),
        PropertyValue::FlexFlow(f) => expand_flex_flow(f, push_longhand),
        PropertyValue::Gap(g) => expand_gap(g, push_longhand),
        PropertyValue::PlaceContent(p) => expand_place_content(p, push_longhand),
        // `GridLineShorthand` contains `SmolStr` and is not `Copy`. Unlike other
        // shorthand payload (`Sides<..>` / `FlexShorthand` / `GapShorthand`
        // payloads (all `Copy`), its value cannot be moved out of `d.value` (accessed
        // through `&Declaration`), so pass it by reference with a `ref` binding
        // to `expand_grid_row` / `expand_grid_column`
        // (see the `expand_grid_row` docs).
        PropertyValue::GridRow(ref shorthand) => expand_grid_row(shorthand, push_longhand),
        PropertyValue::GridColumn(ref shorthand) => expand_grid_column(shorthand, push_longhand),
        PropertyValue::PlaceItems(p) => expand_place_items(p, push_longhand),
        PropertyValue::PlaceSelf(p) => expand_place_self(p, push_longhand),
        // `BackgroundShorthand` holds a `BackgroundImage` field, which is not
        // `Copy` (it can carry a `Gradient`) — same reason `GridLineShorthand`
        // needs `ref` binding here (`GridRow` arm's comment above), not the
        // `Sides<..>`/`FlexShorthand`/`GapShorthand` by-value pattern the
        // other shorthand arms use.
        PropertyValue::Background(ref shorthand) => expand_background(shorthand, push_longhand),
    }
}

/// Shared path for non-shorthands. Split from `expand_shorthand_into` to minimize
/// code size on the hot path (versus the per-family helpers below).
#[inline(always)]
fn expand_none(d: &ParsedDeclaration, mut push: impl FnMut(Declaration)) {
    push(Declaration::new(d.value.clone(), d.important));
}

/// A deferred shorthand still expands before per-key cascade winner selection.
/// The raw value is cloned once per longhand and re-parsed only after the
/// winning longhand has been selected, preserving shorthand/longhand order.
/// A deferred longhand, and a shorthand the cascade keeps in shorthand form
/// ([`KeyClass::Retained`]), stays one declaration.
#[inline(never)]
fn expand_deferred(
    d: &ParsedDeclaration,
    deferred: &DeferredValue,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    let KeyClass::Shorthand { longhands } = classify(deferred.key) else {
        return expand_none(d, push);
    };
    for key in longhands {
        let mut value = deferred.clone();
        value.key = *key;
        push(Declaration::new(PropertyValue::Deferred(value), important));
    }
}

/// Cold helper that expands the `margin` shorthand into four longhands.
#[inline(never)]
pub(crate) fn expand_margin(sides: Sides<LengthOrAuto>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::MarginTop(sides.top));
    push(PropertyValue::MarginRight(sides.right));
    push(PropertyValue::MarginBottom(sides.bottom));
    push(PropertyValue::MarginLeft(sides.left));
}

/// Page-context `margin: inherit` shorthand marker expansion.
#[inline(never)]
pub(crate) fn expand_margin_inherit(mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::MarginTopInherit);
    push(PropertyValue::MarginRightInherit);
    push(PropertyValue::MarginBottomInherit);
    push(PropertyValue::MarginLeftInherit);
}

/// Cold helper that expands the `padding` shorthand into four longhands.
#[inline(never)]
pub(crate) fn expand_padding(sides: Sides<Length>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::PaddingTop(sides.top));
    push(PropertyValue::PaddingRight(sides.right));
    push(PropertyValue::PaddingBottom(sides.bottom));
    push(PropertyValue::PaddingLeft(sides.left));
}

/// Cold helper expanding `margin-inline` into two longhands, `margin-left`/
/// `margin-right`. Follows the margin/padding/overflow shorthand pattern
/// (two values require only two pushes).
///
/// For the rationale behind the physical mapping (inline axis → left/right,
/// approximating `direction: ltr`), see the [`crate::property::PropertyValue::MarginInline`] docs.
#[inline(never)]
pub(crate) fn expand_margin_inline(
    pair: StartEnd<LengthOrAuto>,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::MarginLeft(pair.start));
    push(PropertyValue::MarginRight(pair.end));
}

/// Cold helper expanding `margin-block` into two longhands, `margin-top`/
/// `margin-bottom`: the block-axis sibling of [`expand_margin_inline`]
/// (the block axis maps exactly regardless of `direction`; see the "asymmetry"
/// section of the [`crate::property::PropertyValue::PaddingInline`] docs).
#[inline(never)]
pub(crate) fn expand_margin_block(
    pair: StartEnd<LengthOrAuto>,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::MarginTop(pair.start));
    push(PropertyValue::MarginBottom(pair.end));
}

/// Cold helper expanding `padding-inline` into two longhands, `padding-left`/
/// `padding-right`: the padding sibling of [`expand_margin_inline`].
/// sibling.
#[inline(never)]
pub(crate) fn expand_padding_inline(pair: StartEnd<Length>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::PaddingLeft(pair.start));
    push(PropertyValue::PaddingRight(pair.end));
}

/// Cold helper expanding `padding-block` into two longhands, `padding-top`/
/// `padding-bottom`: the padding sibling of [`expand_margin_block`].
/// sibling.
#[inline(never)]
pub(crate) fn expand_padding_block(pair: StartEnd<Length>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::PaddingTop(pair.start));
    push(PropertyValue::PaddingBottom(pair.end));
}

/// `border` shorthand (CSS Backgrounds 3 §3.4 "Border Shorthand Properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>) into 12 longhands
/// (four sides × three sub-properties: width / style / color).
/// Follows the margin / padding shorthand pattern.
/// The spec's `border` grammar applies to all four sides (`Sides::all(border)`),
/// but writing per-side longhands at cascade time makes overrides such as
/// `border: 1px solid red; border-top-color: blue;` resolve per side
/// deterministically.
#[inline(never)]
pub(crate) fn expand_border(sides: Sides<Border>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderTopWidth(sides.top.width));
    push(PropertyValue::BorderTopStyle(sides.top.style));
    push(PropertyValue::BorderTopColor(sides.top.color));
    push(PropertyValue::BorderRightWidth(sides.right.width));
    push(PropertyValue::BorderRightStyle(sides.right.style));
    push(PropertyValue::BorderRightColor(sides.right.color));
    push(PropertyValue::BorderBottomWidth(sides.bottom.width));
    push(PropertyValue::BorderBottomStyle(sides.bottom.style));
    push(PropertyValue::BorderBottomColor(sides.bottom.color));
    push(PropertyValue::BorderLeftWidth(sides.left.width));
    push(PropertyValue::BorderLeftStyle(sides.left.style));
    push(PropertyValue::BorderLeftColor(sides.left.color));
}

/// Expand `border-top: <line-width> || <line-style> || <color>` into three
/// top-side longhands.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines the single-side shorthand as setting its three sub-properties as if
/// expanded in place (see [`expand_shorthand_into`]). Order is width, style, color —
/// the per-side order [`expand_border`] uses — so declaration ordering is preserved
/// and later longhands in the same block win per CSS Cascading 4 §6.1 order of appearance.
#[inline(never)]
pub(crate) fn expand_border_top(border: Border, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderTopWidth(border.width));
    push(PropertyValue::BorderTopStyle(border.style));
    push(PropertyValue::BorderTopColor(border.color));
}

/// Expand `border-right: <line-width> || <line-style> || <color>` into three
/// right-side longhands.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines the single-side shorthand as setting its three sub-properties as if
/// expanded in place (see [`expand_shorthand_into`]). Order is width, style, color —
/// the per-side order [`expand_border`] uses — so declaration ordering is preserved
/// and later longhands in the same block win per CSS Cascading 4 §6.1 order of appearance.
#[inline(never)]
pub(crate) fn expand_border_right(border: Border, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderRightWidth(border.width));
    push(PropertyValue::BorderRightStyle(border.style));
    push(PropertyValue::BorderRightColor(border.color));
}

/// Expand `border-bottom: <line-width> || <line-style> || <color>` into three
/// bottom-side longhands.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines the single-side shorthand as setting its three sub-properties as if
/// expanded in place (see [`expand_shorthand_into`]). Order is width, style, color —
/// the per-side order [`expand_border`] uses — so declaration ordering is preserved
/// and later longhands in the same block win per CSS Cascading 4 §6.1 order of appearance.
#[inline(never)]
pub(crate) fn expand_border_bottom(border: Border, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderBottomWidth(border.width));
    push(PropertyValue::BorderBottomStyle(border.style));
    push(PropertyValue::BorderBottomColor(border.color));
}

/// Expand `border-left: <line-width> || <line-style> || <color>` into three
/// left-side longhands.
///
/// CSS Backgrounds 3 §3.4 <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>
/// defines the single-side shorthand as setting its three sub-properties as if
/// expanded in place (see [`expand_shorthand_into`]). Order is width, style, color —
/// the per-side order [`expand_border`] uses — so declaration ordering is preserved
/// and later longhands in the same block win per CSS Cascading 4 §6.1 order of appearance.
#[inline(never)]
pub(crate) fn expand_border_left(border: Border, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderLeftWidth(border.width));
    push(PropertyValue::BorderLeftStyle(border.style));
    push(PropertyValue::BorderLeftColor(border.color));
}

/// Expand `border: <css-wide-keyword>` into twelve longhand CSS-wide markers.
///
/// CSS Cascading 4 §3 requires a shorthand with a CSS-wide keyword to set every
/// longhand to that keyword as if expanded in place. Order matches [`expand_border`]
/// so per-side, per-sub-property winners resolve deterministically.
#[inline(never)]
pub(crate) fn expand_border_css_wide(
    kw: crate::property::CssWideKeyword,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::BorderTopWidthCssWide(kw));
    push(PropertyValue::BorderTopStyleCssWide(kw));
    push(PropertyValue::BorderTopColorCssWide(kw));
    push(PropertyValue::BorderRightWidthCssWide(kw));
    push(PropertyValue::BorderRightStyleCssWide(kw));
    push(PropertyValue::BorderRightColorCssWide(kw));
    push(PropertyValue::BorderBottomWidthCssWide(kw));
    push(PropertyValue::BorderBottomStyleCssWide(kw));
    push(PropertyValue::BorderBottomColorCssWide(kw));
    push(PropertyValue::BorderLeftWidthCssWide(kw));
    push(PropertyValue::BorderLeftStyleCssWide(kw));
    push(PropertyValue::BorderLeftColorCssWide(kw));
}

/// Expand `border-top: <css-wide-keyword>` into three top-side longhand CSS-wide
/// markers (see [`expand_border_top`] for ordering).
#[inline(never)]
pub(crate) fn expand_border_top_css_wide(
    kw: crate::property::CssWideKeyword,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::BorderTopWidthCssWide(kw));
    push(PropertyValue::BorderTopStyleCssWide(kw));
    push(PropertyValue::BorderTopColorCssWide(kw));
}

/// Expand `border-right: <css-wide-keyword>` into three right-side longhand CSS-wide
/// markers (see [`expand_border_right`] for ordering).
#[inline(never)]
pub(crate) fn expand_border_right_css_wide(
    kw: crate::property::CssWideKeyword,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::BorderRightWidthCssWide(kw));
    push(PropertyValue::BorderRightStyleCssWide(kw));
    push(PropertyValue::BorderRightColorCssWide(kw));
}

/// Expand `border-bottom: <css-wide-keyword>` into three bottom-side longhand CSS-wide
/// markers (see [`expand_border_bottom`] for ordering).
#[inline(never)]
pub(crate) fn expand_border_bottom_css_wide(
    kw: crate::property::CssWideKeyword,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::BorderBottomWidthCssWide(kw));
    push(PropertyValue::BorderBottomStyleCssWide(kw));
    push(PropertyValue::BorderBottomColorCssWide(kw));
}

/// Expand `border-left: <css-wide-keyword>` into three left-side longhand CSS-wide
/// markers (see [`expand_border_left`] for ordering).
#[inline(never)]
pub(crate) fn expand_border_left_css_wide(
    kw: crate::property::CssWideKeyword,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::BorderLeftWidthCssWide(kw));
    push(PropertyValue::BorderLeftStyleCssWide(kw));
    push(PropertyValue::BorderLeftColorCssWide(kw));
}

/// Cold helper expanding `border-style` into four longhands (`border-*-style`);
/// follows the margin/padding/border shorthand pattern.
#[inline(never)]
pub(crate) fn expand_border_style(sides: Sides<BorderStyle>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderTopStyle(sides.top));
    push(PropertyValue::BorderRightStyle(sides.right));
    push(PropertyValue::BorderBottomStyle(sides.bottom));
    push(PropertyValue::BorderLeftStyle(sides.left));
}

/// Cold helper expanding `border-width` into four longhands (`border-*-width`);
/// follows the margin/padding/border shorthand pattern.
#[inline(never)]
pub(crate) fn expand_border_width(sides: Sides<Length>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderTopWidth(sides.top));
    push(PropertyValue::BorderRightWidth(sides.right));
    push(PropertyValue::BorderBottomWidth(sides.bottom));
    push(PropertyValue::BorderLeftWidth(sides.left));
}

/// Cold helper expanding `border-color` into four longhands (`border-*-color`);
/// follows the margin/padding/border shorthand pattern.
#[inline(never)]
pub(crate) fn expand_border_color(sides: Sides<BorderColor>, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::BorderTopColor(sides.top));
    push(PropertyValue::BorderRightColor(sides.right));
    push(PropertyValue::BorderBottomColor(sides.bottom));
    push(PropertyValue::BorderLeftColor(sides.left));
}

/// `overflow` shorthand (CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>) into
/// two longhands, `overflow-x` / `overflow-y`.
/// Follows the margin / padding / border shorthand
/// pattern: two axes require only two pushes.
#[inline(never)]
pub(crate) fn expand_overflow(pair: OverflowXY, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::OverflowX(pair.x));
    push(PropertyValue::OverflowY(pair.y));
}

/// Cold helper expanding the `flex` shorthand into three longhands:
/// `flex-grow` / `flex-shrink` / `flex-basis`. Follows the margin / padding / border
/// pattern. The shorthand parser (`property.rs`'s
/// `parse_flex_shorthand`, a private function that cannot be linked directly)
/// already fills in shorthand-local defaults (grow=1 / shrink=1 / basis=0px);
/// see the "Omitted-component defaults" section of the [`FlexShorthand`] docs.
/// This function only distributes the three fields into three declarations.
#[inline(never)]
pub(crate) fn expand_flex(f: FlexShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::FlexGrow(f.grow));
    push(PropertyValue::FlexShrink(f.shrink));
    push(PropertyValue::FlexBasis(f.basis));
}

/// Cold helper expanding `flex-flow` into two longhands, `flex-direction` /
/// `flex-wrap`. As in `expand_flex`, the shorthand parser
/// (`parse_flex_flow`) already fills in omitted initial components (row / nowrap).
/// This function only distributes the two fields into two declarations.
#[inline(never)]
pub(crate) fn expand_flex_flow(f: FlexFlow, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::FlexDirection(f.direction));
    push(PropertyValue::FlexWrap(f.wrap));
}

/// Cold helper expanding `gap` into two longhands, `row-gap` / `column-gap`.
/// The [`GapShorthand`] docs' second-value-omitted copy rule is already
/// applied by the parser (`parse_gap_shorthand`). This function only distributes
/// the two fields into two declarations.
#[inline(never)]
pub(crate) fn expand_gap(g: GapShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::RowGap(g.row));
    push(PropertyValue::ColumnGap(g.column));
}

/// Cold helper expanding `place-content` into two longhands, `align-content` /
/// `justify-content`. Has the same shape as [`expand_gap`]
/// (see the [`PlaceContentShorthand`] docs).
#[inline(never)]
pub(crate) fn expand_place_content(p: PlaceContentShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::AlignContent(p.align));
    push(PropertyValue::JustifyContent(p.justify));
}

/// Cold helper expanding `grid-row` into two longhands, `grid-row-start` /
/// `grid-row-end`. The parser (`parse_grid_line_shorthand` in `property.rs`,
/// a private function that cannot be linked directly) already applies the
/// second-value-omitted copy rule in the [`GridLineShorthand`] docs.
///
/// `shorthand: &GridLineShorthand`: see the `PropertyValue::GridRow(ref shorthand)`
/// arm in the [`expand_shorthand_into`] docs. Unlike other shorthand expansion
/// helpers (such as `expand_flex`), this takes a reference and calls `.clone()`
/// because `GridLineValue` contains `SmolStr` and is not `Copy`.
#[inline(never)]
pub(crate) fn expand_grid_row(shorthand: &GridLineShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::GridRowStart(shorthand.start.clone()));
    push(PropertyValue::GridRowEnd(shorthand.end.clone()));
}

/// Cold helper expanding `grid-column` into two longhands, `grid-column-start` /
/// `grid-column-end`. Has the same shape as [`expand_grid_row`].
#[inline(never)]
pub(crate) fn expand_grid_column(
    shorthand: &GridLineShorthand,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::GridColumnStart(shorthand.start.clone()));
    push(PropertyValue::GridColumnEnd(shorthand.end.clone()));
}

/// Cold helper expanding `place-items` into two longhands, `align-items` /
/// `justify-items`. Has the same shape as [`expand_gap`]
/// (see the [`PlaceItemsShorthand`] docs).
#[inline(never)]
pub(crate) fn expand_place_items(p: PlaceItemsShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::AlignItems(p.align));
    push(PropertyValue::JustifyItems(p.justify));
}

/// Cold helper expanding `place-self` into two longhands, `align-self` /
/// `justify-self`. Has the same shape as [`expand_place_items`]
/// (see the [`PlaceSelfShorthand`] docs).
#[inline(never)]
pub(crate) fn expand_place_self(p: PlaceSelfShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::AlignSelf(p.align));
    push(PropertyValue::JustifySelf(p.justify));
}

/// `text-decoration` shorthand (CSS Text Decoration 4 ED §2.6
/// <https://drafts.csswg.org/css-text-decor-4/#text-decoration-property>) into
/// four longhands: `text-decoration-line` / `-thickness` / `-style` / `-color`.
/// Follows the margin / padding / border / overflow shorthand
/// pattern: the four longhands are disjoint fields in a 1:1 mapping
/// (see the "cross-axis coupling is absent" section of the `TextDecorationShorthand` docs),
/// so only four pushes are needed.
///
/// The shorthand parser (`parse_text_decoration_shorthand` in `property.rs`,
/// a private function that cannot be linked directly) already fills omitted
/// components with spec initial values (see "Initial value fill" in the
/// [`TextDecorationShorthand`] docs). This function only distributes the four
/// fields into four declarations, just as the margin/padding/border sides
/// already contain values filled with their initial values.
#[inline(never)]
pub(crate) fn expand_text_decoration(
    shorthand: TextDecorationShorthand,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::TextDecorationLine(shorthand.line));
    push(PropertyValue::TextDecorationThickness(shorthand.thickness));
    push(PropertyValue::TextDecorationStyle(shorthand.style));
    push(PropertyValue::TextDecorationColor(shorthand.color));
}

/// Expand `text-emphasis` into its style and color longhands.
#[inline(never)]
pub(crate) fn expand_text_emphasis(
    shorthand: &TextEmphasisShorthand,
    mut push: impl FnMut(PropertyValue),
) {
    push(PropertyValue::TextEmphasisStyle(shorthand.style.clone()));
    push(PropertyValue::TextEmphasisColor(shorthand.color));
}

/// Expand the `outline` shorthand into width/style/color longhands.
#[inline(never)]
pub(crate) fn expand_outline(outline: Outline, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::OutlineWidth(outline.width));
    push(PropertyValue::OutlineStyle(outline.style));
    push(PropertyValue::OutlineColor(outline.color));
}

/// Cold helper expanding the `font` shorthand into 16 longhands (six grammar
/// values plus ten reset-only subproperties; see the [`FontShorthand`] docs).
/// The shorthand parser fills omitted grammar components with spec initial
/// values, so this function distributes the six fields and assigns initial
/// values to reset-only subproperties under CSS Fonts 4 §2.1. `font-variant-caps`
/// is already distributed as a grammar longhand. Cloning `family` only bumps
/// the `Arc` (as when cloning `image` in [`BackgroundShorthand`]). Both forms of
/// `size` map directly to the corresponding [`PropertyValue`] variants (both use
/// [`PropertyKey::FontSize`]).
#[inline(never)]
pub(crate) fn expand_font(shorthand: &FontShorthand, mut push: impl FnMut(PropertyValue)) {
    push(PropertyValue::FontStyle(shorthand.style));
    push(PropertyValue::FontVariantCaps(shorthand.variant));
    push(PropertyValue::FontWeight(shorthand.weight));
    push(match shorthand.size {
        FontShorthandSize::Absolute(length) => PropertyValue::FontSize(length),
        FontShorthandSize::Relative(relative) => PropertyValue::FontSizeRelative(relative),
    });
    push(PropertyValue::LineHeight(shorthand.line_height));
    push(PropertyValue::FontFamily(shorthand.family.clone()));
    push(PropertyValue::FontKerning(FontKerning::Auto));
    push(PropertyValue::FontLanguageOverride(
        FontLanguageOverride::Normal,
    ));
    push(PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto));
    push(PropertyValue::FontVariantEastAsian(
        FontVariantEastAsian::initial(),
    ));
    push(PropertyValue::FontVariantEmoji(FontVariantEmoji::Normal));
    push(PropertyValue::FontVariantLigatures(
        FontVariantLigatures::Normal,
    ));
    push(PropertyValue::FontVariantNumeric(
        FontVariantNumeric::initial(),
    ));
    push(PropertyValue::FontVariantPosition(
        FontVariantPosition::Normal,
    ));
    push(PropertyValue::FontVariationSettings(
        FontVariationSettings::Normal,
    ));
    push(PropertyValue::FontFeatureSettings(
        FontFeatureSettings::Normal,
    ));
}

/// Cold helper expanding `background` into eight longhands (color/image/repeat/
/// attachment/position/size/clip/origin; see the [`BackgroundShorthand`] docs).
/// `image` is not `Copy`, so it is cloned, for the same reason
/// `expand_grid_row`/`expand_grid_column` clone `GridLineValue`'s
/// `Named`/`NamedLine` components.
#[inline(never)]
pub(crate) fn expand_background(
    shorthand: &BackgroundShorthand,
    mut push: impl FnMut(PropertyValue),
) {
    if let Some(source) = &shorthand.color_expression {
        push(PropertyValue::ContextualColor(
            crate::property::ContextualColor {
                source: source.clone(),
                key: PropertyKey::BackgroundColor,
            },
        ));
    } else {
        push(PropertyValue::BackgroundColor(shorthand.color));
    }
    push(PropertyValue::BackgroundImage(shorthand.image.clone()));
    push(PropertyValue::BackgroundRepeat(shorthand.repeat));
    push(PropertyValue::BackgroundAttachment(shorthand.attachment));
    push(PropertyValue::BackgroundPosition(shorthand.position));
    push(PropertyValue::BackgroundSize(shorthand.size));
    push(PropertyValue::BackgroundClip(shorthand.clip));
    push(PropertyValue::BackgroundOrigin(shorthand.origin));
}

/// Validate a consumer-owned value with the grammar shared by declarations and feature queries.
pub(crate) fn parse_registered_consumer_value(
    name: &str,
    input: &mut Parser<'_, '_>,
    registrations: &[ConsumerPropertyRegistration],
) -> Option<PropertyValue> {
    let registration = registrations
        .iter()
        .find(|registration| registration.matches_css_name(name))?;
    let start = input.state();
    let raw = consume_deferred_value(input)?;
    let has_deferred_substitution = raw.to_ascii_lowercase().contains("var(");
    if !has_deferred_substitution {
        let valid = match registration.grammar() {
            ConsumerPropertyGrammar::Integer => {
                let mut parser_input = cssparser::ParserInput::new(raw.as_str());
                let mut parser = Parser::new(&mut parser_input);
                parser
                    .parse_entirely(|parser| {
                        let value = parser
                            .expect_integer()
                            .map_err(|_| parser.new_custom_error(()))?;
                        Ok::<_, cssparser::ParseError<'_, ()>>(value)
                    })
                    .is_ok()
            }
            ConsumerPropertyGrammar::IntegerOrNone => {
                let mut parser_input = cssparser::ParserInput::new(raw.as_str());
                let mut parser = Parser::new(&mut parser_input);
                parser
                    .parse_entirely(|parser| -> Result<(), cssparser::ParseError<'_, ()>> {
                        if parser
                            .try_parse(|parser| parser.expect_ident_matching("none"))
                            .is_ok()
                        {
                            Ok(())
                        } else {
                            parser
                                .expect_integer()
                                .map(|_| ())
                                .map_err(|_| parser.new_custom_error(()))
                        }
                    })
                    .is_ok()
            }
            ConsumerPropertyGrammar::Text => parse_consumer_text_value(raw.as_str()).is_some(),
        };
        if !valid {
            input.reset(&start);
            return None;
        }
    }
    Some(PropertyValue::CustomProperty(CustomProperty {
        name: registration.storage_name(),
        value: raw,
    }))
}

/// Parse one declaration, including importance and exhaustive consumption.
pub(crate) fn parse_declaration_value<'i>(
    name: CowRcStr<'i>,
    input: &mut Parser<'i, '_>,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Result<ParsedDeclaration, ParseError<'i, ()>> {
    let value = parse_registered_consumer_value(name.as_ref(), input, consumer_properties)
        .or_else(|| parse_value(name.as_ref(), input))
        .ok_or_else(|| input.new_custom_error(()))?;
    let important = input.try_parse(cssparser::parse_important).is_ok();
    // Exhaustive consumption: trailing garbage after the value (and optional
    // `!important`) must reject the whole declaration rather than silently
    // accepting a prefix (e.g. `color: red garbage` / `font-size: 16px 20px`).
    input
        .expect_exhausted()
        .map_err(|e: cssparser::BasicParseError<'i>| -> ParseError<'i, ()> { e.into() })?;
    Ok(ParsedDeclaration { value, important })
}

/// Per-declaration parser for cssparser::RuleBodyParser.
struct DeclParser<'a> {
    consumer_properties: &'a [ConsumerPropertyRegistration],
}

impl<'i, 'a> DeclarationParser<'i> for DeclParser<'a> {
    type Declaration = ParsedDeclaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<ParsedDeclaration, ParseError<'i, Self::Error>> {
        parse_declaration_value(name, input, self.consumer_properties)
    }
}

// The at-rule parser does nothing (drops any @rule inside a block).
impl<'i, 'a> AtRuleParser<'i> for DeclParser<'a> {
    type Prelude = ();
    type AtRule = ParsedDeclaration;
    type Error = ();
}

// The qualified-rule parser (nested rules) also does nothing: nested rules in a block are dropped.
impl<'i, 'a> QualifiedRuleParser<'i> for DeclParser<'a> {
    type Prelude = ();
    type QualifiedRule = ParsedDeclaration;
    type Error = ();
}

impl<'i, 'a> RuleBodyItemParser<'i, ParsedDeclaration, ()> for DeclParser<'a> {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests;
