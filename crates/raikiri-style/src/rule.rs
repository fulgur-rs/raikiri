//! CSS rule and declaration structures. Supports qualified rules (StyleRule)
//! with type/universal selectors only. At-rules (@page / @media, etc.) are
//! skipped in ruletree.rs.

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::RaikiriSelectorImpl;
use crate::consumer::{ConsumerPropertyGrammar, ConsumerPropertyRegistration};
use crate::property::longhand_value_pat;
use crate::property::{
    BackgroundShorthand, Border, BorderColor, BorderStyle, CustomProperty, DeferredValue, FlexFlow,
    FlexShorthand, FontKerning, FontLanguageOverride, FontOpticalSizing, FontShorthand,
    FontShorthandSize, FontVariantEastAsian, FontVariantEmoji, FontVariantLigatures,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// Resolved property value.
    pub(crate) value: PropertyValue,
    /// `!important` flag (true increases importance).
    pub important: bool,
}

impl Declaration {
    /// Read-only accessor for the resolved property value.
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
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
pub(crate) fn expand_shorthand_into(d: &Declaration, mut push: impl FnMut(Declaration)) {
    let mut push_longhand = |value| {
        push(Declaration {
            value,
            important: d.important,
        })
    };
    match d.value {
        PropertyValue::Margin(sides) => expand_margin(sides, push_longhand),
        PropertyValue::MarginInherit => expand_margin_inherit(push_longhand),
        PropertyValue::Padding(sides) => expand_padding(sides, push_longhand),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, push_longhand),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, push_longhand),
        PropertyValue::PaddingInline(pair) => expand_padding_inline(pair, push_longhand),
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, push_longhand),
        PropertyValue::Border(sides) => expand_border(sides, push_longhand),
        PropertyValue::BorderCssWide(kw) => expand_border_css_wide(kw, push_longhand),
        PropertyValue::BorderRight(sides) => expand_border_right(sides, push_longhand),
        PropertyValue::BorderRightCssWide(kw) => expand_border_right_css_wide(kw, push_longhand),
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
        PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::Color(_)
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
        | PropertyValue::BorderRadius(_)
        | PropertyValue::BorderRadiusInherit
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
        // mix-blend-mode (CSS Compositing and Blending Level 1 §3.4.1) —
        // same shape as object-fit/opacity above.
        | PropertyValue::MixBlendMode(_)
        // mask-image / clip-path (CSS Masking Level 1 §7.1/§5.1) — same
        // shape as mix-blend-mode above (`mask`/`mask-border`
        // shorthands stay unimplemented, pre-existing silent drop).
        | PropertyValue::MaskImage(_)
        | PropertyValue::ClipPath(_)
        // transform (CSS Transforms Level 1 §4) / filter (CSS Filter
        // Effects Level 1 §5) — same shape as mask-image/clip-path above.
        | PropertyValue::Transform(_)
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_) | PropertyValue::TextCombineUpright(_) | PropertyValue::TextOrientation(_) | PropertyValue::UnicodeBidi(_) | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_) | PropertyValue::ColumnWidth(_)
        // Table-declared longhands (`properties!` in property/decl.rs) are
        // longhands, so nothing expands them.
        | longhand_value_pat!() => expand_none(d, push),
        PropertyValue::Columns(shorthand) => {
            push(Declaration {
                value: PropertyValue::ColumnWidth(shorthand.width),
                important: d.important,
            });
            push(Declaration {
                value: PropertyValue::ColumnCount(shorthand.count),
                important: d.important,
            });
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
fn expand_none(d: &Declaration, mut push: impl FnMut(Declaration)) {
    push(d.clone());
}

/// A deferred shorthand still expands before per-key cascade winner selection.
/// The raw value is cloned once per longhand and re-parsed only after the
/// winning longhand has been selected, preserving shorthand/longhand order.
#[inline(never)]
fn expand_deferred(
    d: &Declaration,
    deferred: &DeferredValue,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    let keys: &[PropertyKey] = match deferred.key {
        PropertyKey::Padding => &[
            PropertyKey::PaddingTop,
            PropertyKey::PaddingRight,
            PropertyKey::PaddingBottom,
            PropertyKey::PaddingLeft,
        ],
        PropertyKey::Margin => &[
            PropertyKey::MarginTop,
            PropertyKey::MarginRight,
            PropertyKey::MarginBottom,
            PropertyKey::MarginLeft,
        ],
        PropertyKey::MarginInline => &[PropertyKey::MarginLeft, PropertyKey::MarginRight],
        PropertyKey::MarginBlock => &[PropertyKey::MarginTop, PropertyKey::MarginBottom],
        PropertyKey::PaddingInline => &[PropertyKey::PaddingLeft, PropertyKey::PaddingRight],
        PropertyKey::PaddingBlock => &[PropertyKey::PaddingTop, PropertyKey::PaddingBottom],
        PropertyKey::Border => &[
            PropertyKey::BorderTopWidth,
            PropertyKey::BorderTopStyle,
            PropertyKey::BorderTopColor,
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderRightColor,
            PropertyKey::BorderBottomWidth,
            PropertyKey::BorderBottomStyle,
            PropertyKey::BorderBottomColor,
            PropertyKey::BorderLeftWidth,
            PropertyKey::BorderLeftStyle,
            PropertyKey::BorderLeftColor,
        ],
        PropertyKey::BorderRight => &[
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderRightColor,
        ],
        PropertyKey::BorderStyle => &[
            PropertyKey::BorderTopStyle,
            PropertyKey::BorderRightStyle,
            PropertyKey::BorderBottomStyle,
            PropertyKey::BorderLeftStyle,
        ],
        PropertyKey::BorderWidth => &[
            PropertyKey::BorderTopWidth,
            PropertyKey::BorderRightWidth,
            PropertyKey::BorderBottomWidth,
            PropertyKey::BorderLeftWidth,
        ],
        PropertyKey::BorderColor => &[
            PropertyKey::BorderTopColor,
            PropertyKey::BorderRightColor,
            PropertyKey::BorderBottomColor,
            PropertyKey::BorderLeftColor,
        ],
        PropertyKey::Overflow => &[PropertyKey::OverflowX, PropertyKey::OverflowY],
        PropertyKey::TextDecoration => &[
            PropertyKey::TextDecorationLine,
            PropertyKey::TextDecorationThickness,
            PropertyKey::TextDecorationStyle,
            PropertyKey::TextDecorationColor,
        ],
        PropertyKey::TextEmphasis => &[
            PropertyKey::TextEmphasisStyle,
            PropertyKey::TextEmphasisColor,
        ],
        PropertyKey::Outline => &[
            PropertyKey::OutlineWidth,
            PropertyKey::OutlineStyle,
            PropertyKey::OutlineColor,
        ],
        // `font` shorthand deferred expansion — 6 grammar longhands plus 9
        // reset-only subproperties. `font-variant-caps` is already a grammar
        // longhand; `size` is one key whether absolute or relative.
        PropertyKey::Font => &[
            PropertyKey::FontStyle,
            PropertyKey::FontVariantCaps,
            PropertyKey::FontWeight,
            PropertyKey::FontSize,
            PropertyKey::LineHeight,
            PropertyKey::FontFamily,
            PropertyKey::FontKerning,
            PropertyKey::FontLanguageOverride,
            PropertyKey::FontOpticalSizing,
            PropertyKey::FontVariantEastAsian,
            PropertyKey::FontVariantEmoji,
            PropertyKey::FontVariantLigatures,
            PropertyKey::FontVariantNumeric,
            PropertyKey::FontVariantPosition,
            PropertyKey::FontVariationSettings,
        ],
        PropertyKey::Flex => &[
            PropertyKey::FlexGrow,
            PropertyKey::FlexShrink,
            PropertyKey::FlexBasis,
        ],
        PropertyKey::FlexFlow => &[PropertyKey::FlexDirection, PropertyKey::FlexWrap],
        PropertyKey::Columns => &[PropertyKey::ColumnWidth, PropertyKey::ColumnCount],
        PropertyKey::Gap => &[PropertyKey::RowGap, PropertyKey::ColumnGap],
        PropertyKey::PlaceContent => &[PropertyKey::AlignContent, PropertyKey::JustifyContent],
        PropertyKey::GridRow => &[PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
        PropertyKey::GridColumn => &[PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
        PropertyKey::PlaceItems => &[PropertyKey::AlignItems, PropertyKey::JustifyItems],
        PropertyKey::PlaceSelf => &[PropertyKey::AlignSelf, PropertyKey::JustifySelf],
        // Order here is `BackgroundShorthand`'s own field order
        // (color/image/repeat/attachment/position/size/clip/origin) — same
        // "grouped by the shorthand's own natural order" shape as `Border`
        // (grouped per-side, not per-`PropertyKey`-declaration-order) and
        // `MarginInline`/`PaddingInline`/`PlaceContent` above. Safe because
        // each key still lands in its own disjoint `SpecifiedValues` field,
        // so the order this slice is walked in does not affect the result.
        PropertyKey::Background => &[
            PropertyKey::BackgroundColor,
            PropertyKey::BackgroundImage,
            PropertyKey::BackgroundRepeat,
            PropertyKey::BackgroundAttachment,
            PropertyKey::BackgroundPosition,
            PropertyKey::BackgroundSize,
            PropertyKey::BackgroundClip,
            PropertyKey::BackgroundOrigin,
        ],
        _ => return expand_none(d, push),
    };

    for key in keys {
        let mut value = deferred.clone();
        value.key = *key;
        push(Declaration {
            value: PropertyValue::Deferred(value),
            important,
        });
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

/// Cold helper expanding the `font` shorthand into 15 longhands (six grammar
/// values plus nine reset-only subproperties; see the [`FontShorthand`] docs).
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
    push(PropertyValue::BackgroundColor(shorthand.color));
    push(PropertyValue::BackgroundImage(shorthand.image.clone()));
    push(PropertyValue::BackgroundRepeat(shorthand.repeat));
    push(PropertyValue::BackgroundAttachment(shorthand.attachment));
    push(PropertyValue::BackgroundPosition(shorthand.position));
    push(PropertyValue::BackgroundSize(shorthand.size));
    push(PropertyValue::BackgroundClip(shorthand.clip));
    push(PropertyValue::BackgroundOrigin(shorthand.origin));
}

fn parse_registered_consumer_value(
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

/// Per-declaration parser for cssparser::RuleBodyParser.
struct DeclParser<'a> {
    consumer_properties: &'a [ConsumerPropertyRegistration],
}

impl<'i, 'a> DeclarationParser<'i> for DeclParser<'a> {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<Declaration, ParseError<'i, Self::Error>> {
        let value = parse_registered_consumer_value(name.as_ref(), input, self.consumer_properties)
            .or_else(|| parse_value(name.as_ref(), input))
            .ok_or_else(|| input.new_custom_error(()))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        // Exhaustive consumption: trailing garbage after the value (and optional
        // `!important`) must reject the whole declaration rather than silently
        // accepting a prefix (e.g. `color: red garbage` / `font-size: 16px 20px`).
        input.expect_exhausted().map_err(
            |e: cssparser::BasicParseError<'i>| -> ParseError<'i, Self::Error> { e.into() },
        )?;
        Ok(Declaration { value, important })
    }
}

// The at-rule parser does nothing (drops any @rule inside a block).
impl<'i, 'a> AtRuleParser<'i> for DeclParser<'a> {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

// The qualified-rule parser (nested rules) also does nothing: nested rules in a block are dropped.
impl<'i, 'a> QualifiedRuleParser<'i> for DeclParser<'a> {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i, 'a> RuleBodyItemParser<'i, Declaration, ()> for DeclParser<'a> {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::property::{
        BorderColor, BorderStyle, CssColor, FontStyle, FontVariantCaps, FontWeightValue, Length,
        LengthOrAuto, LineHeight, OutlineColor, OutlineStyle, OverflowValue, RelativeFontSize,
    };
    use cssparser::ParserInput;

    fn parse_block(source: &str) -> Vec<Declaration> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_declaration_block(&mut parser)
    }

    /// `parse_declaration_block` must not emit shorthand keys. Shorthands are
    /// expanded before the cascade so longhand precedence follows the CSS
    /// shorthand contract.
    #[test]
    fn declaration_block_never_emits_shorthand_keys() {
        use crate::property::PropertyKey;

        let decls = parse_block(
            "margin: 1px; padding: 2px; border: 3px solid red; outline: auto 2px red; \
             margin-top: 4px; padding-left: 5px; border-top-width: 6px; \
             text-decoration: underline overline; color: red; font-size: 10px; \
             font: italic small-caps bold 12px/1.5 serif",
        );
        assert!(
            !decls.is_empty(),
            "parse が空 — test corpus 側の regression"
        );

        for decl in &decls {
            let key = decl.value.key();
            assert!(
                !matches!(
                    key,
                    PropertyKey::Margin
                        | PropertyKey::Padding
                        | PropertyKey::Border
                        | PropertyKey::BorderRight
                        | PropertyKey::BorderStyle
                        | PropertyKey::BorderWidth
                        | PropertyKey::BorderColor
                        | PropertyKey::Outline
                        | PropertyKey::Font
                        | PropertyKey::TextDecoration
                ),
                "shorthand key {key:?} が cascade 段へ漏れている — \
                 `expand_shorthand_into` の展開 arm は exhaustive match により \
                 存在するはずなので、疑うのは `parse_declaration_block` が \
                 同関数を通さなくなったか、当該 variant が展開 arm ではなく \
                 non-shorthand 側の or-pattern に書かれているか (どちらも \
                 compile は通る)"
            );
        }
    }

    #[test]
    fn deferred_shorthand_expands_to_each_longhand_key() {
        use crate::property::{DeferredValue, PropertyKey};

        let cases: &[(PropertyKey, &[PropertyKey])] = &[
            (
                PropertyKey::Padding,
                &[
                    PropertyKey::PaddingTop,
                    PropertyKey::PaddingRight,
                    PropertyKey::PaddingBottom,
                    PropertyKey::PaddingLeft,
                ],
            ),
            (
                PropertyKey::Margin,
                &[
                    PropertyKey::MarginTop,
                    PropertyKey::MarginRight,
                    PropertyKey::MarginBottom,
                    PropertyKey::MarginLeft,
                ],
            ),
            (
                PropertyKey::Border,
                &[
                    PropertyKey::BorderTopWidth,
                    PropertyKey::BorderTopStyle,
                    PropertyKey::BorderTopColor,
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderRightColor,
                    PropertyKey::BorderBottomWidth,
                    PropertyKey::BorderBottomStyle,
                    PropertyKey::BorderBottomColor,
                    PropertyKey::BorderLeftWidth,
                    PropertyKey::BorderLeftStyle,
                    PropertyKey::BorderLeftColor,
                ],
            ),
            (
                PropertyKey::BorderRight,
                &[
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderRightColor,
                ],
            ),
            (
                PropertyKey::Overflow,
                &[PropertyKey::OverflowX, PropertyKey::OverflowY],
            ),
            (
                PropertyKey::BorderStyle,
                &[
                    PropertyKey::BorderTopStyle,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderBottomStyle,
                    PropertyKey::BorderLeftStyle,
                ],
            ),
            (
                PropertyKey::BorderWidth,
                &[
                    PropertyKey::BorderTopWidth,
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderBottomWidth,
                    PropertyKey::BorderLeftWidth,
                ],
            ),
            (
                PropertyKey::BorderColor,
                &[
                    PropertyKey::BorderTopColor,
                    PropertyKey::BorderRightColor,
                    PropertyKey::BorderBottomColor,
                    PropertyKey::BorderLeftColor,
                ],
            ),
            (
                PropertyKey::TextDecoration,
                &[
                    PropertyKey::TextDecorationLine,
                    PropertyKey::TextDecorationThickness,
                    PropertyKey::TextDecorationStyle,
                    PropertyKey::TextDecorationColor,
                ],
            ),
            (
                PropertyKey::TextEmphasis,
                &[
                    PropertyKey::TextEmphasisStyle,
                    PropertyKey::TextEmphasisColor,
                ],
            ),
            (
                PropertyKey::Outline,
                &[
                    PropertyKey::OutlineWidth,
                    PropertyKey::OutlineStyle,
                    PropertyKey::OutlineColor,
                ],
            ),
            (
                PropertyKey::Font,
                &[
                    PropertyKey::FontStyle,
                    PropertyKey::FontVariantCaps,
                    PropertyKey::FontWeight,
                    PropertyKey::FontSize,
                    PropertyKey::LineHeight,
                    PropertyKey::FontFamily,
                    PropertyKey::FontKerning,
                    PropertyKey::FontLanguageOverride,
                    PropertyKey::FontOpticalSizing,
                    PropertyKey::FontVariantEastAsian,
                    PropertyKey::FontVariantEmoji,
                    PropertyKey::FontVariantLigatures,
                    PropertyKey::FontVariantNumeric,
                    PropertyKey::FontVariantPosition,
                    PropertyKey::FontVariationSettings,
                ],
            ),
            (
                PropertyKey::Flex,
                &[
                    PropertyKey::FlexGrow,
                    PropertyKey::FlexShrink,
                    PropertyKey::FlexBasis,
                ],
            ),
            (
                PropertyKey::Columns,
                &[PropertyKey::ColumnWidth, PropertyKey::ColumnCount],
            ),
            (
                PropertyKey::Gap,
                &[PropertyKey::RowGap, PropertyKey::ColumnGap],
            ),
            (
                PropertyKey::PlaceContent,
                &[PropertyKey::AlignContent, PropertyKey::JustifyContent],
            ),
            (
                PropertyKey::GridRow,
                &[PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
            ),
            (
                PropertyKey::GridColumn,
                &[PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
            ),
            (
                PropertyKey::PlaceItems,
                &[PropertyKey::AlignItems, PropertyKey::JustifyItems],
            ),
            (
                PropertyKey::PlaceSelf,
                &[PropertyKey::AlignSelf, PropertyKey::JustifySelf],
            ),
        ];

        for (shorthand, expected) in cases {
            let declaration = Declaration {
                value: PropertyValue::Deferred(DeferredValue {
                    property: "test".into(),
                    value: "raw".into(),
                    key: *shorthand,
                }),
                important: true,
            };
            let mut expanded = Vec::new();
            expand_shorthand_into(&declaration, |declaration| {
                expanded.push((declaration.value.key(), declaration.important));
            });
            assert_eq!(
                expanded,
                expected
                    .iter()
                    .copied()
                    .map(|key| (key, true))
                    .collect::<Vec<_>>()
            );
        }

        let declaration = Declaration {
            value: PropertyValue::Deferred(DeferredValue {
                property: "width".into(),
                value: "raw".into(),
                key: PropertyKey::Width,
            }),
            important: false,
        };
        let mut expanded = Vec::new();
        expand_shorthand_into(&declaration, |declaration| {
            expanded.push((declaration.value.key(), declaration.important));
        });
        assert_eq!(expanded, vec![(PropertyKey::Width, false)]);
    }

    #[test]
    fn parses_single_declaration() {
        let decls = parse_block("color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
        assert!(!decls[0].important);
    }

    #[test]
    fn captures_important_flag() {
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn drops_invalid_property_and_value() {
        // `cursor: pointer` is unsupported (CSS Basic User Interface
        // Module Level 3 <https://www.w3.org/TR/css-ui-3/#cursor>) → drop
        // (`margin` / `width` / `float` were previously used as dropped examples,
        // but were later recognized and replaced here; `cursor` remains
        // unsupported, matching the canary in property.rs's
        // `unknown_property_returns_none`).
        // `font-size: math` drops because the MathML scaling algorithm is not implemented
        // (`medium` was formerly used as a dropped example, but `<absolute-size>` /
        // `<relative-size>` keywords became recognized and were replaced here; see
        // the "keep examples synchronized when replacing them" section of the
        // `PropertyValue` docs in property.rs).
        // Keep `color: red`.
        let decls = parse_block("cursor: pointer; font-size: math; color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
    }

    #[test]
    fn multiple_declarations_in_order() {
        let decls = parse_block("color: red; font-size: 16px; font-weight: 700;");
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].value, PropertyValue::Color(_)));
        assert_eq!(decls[1].value, PropertyValue::FontSize(Length::Px(16.0)));
        assert_eq!(
            decls[2].value,
            PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))
        );
    }

    #[test]
    fn empty_block_returns_empty() {
        assert!(parse_block("").is_empty());
        assert!(parse_block("   ").is_empty());
    }

    #[test]
    fn rejects_trailing_garbage_after_value() {
        // "red garbage": the extra token after the value drops the whole declaration.
        let decls = parse_block("color: red garbage;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_length_after_font_size() {
        // "16px 20px": the second length is unconsumed garbage, so the declaration drops.
        let decls = parse_block("font-size: 16px 20px;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_font_style_oblique_with_angle() {
        // CSS Fonts 4 §2.4's `oblique <angle [-90deg,90deg]>?` grammar lets
        // `oblique` take an optional angle, but this crate accepts `oblique`
        // only as a bare keyword (`FontStyle` doc's "Scope carving"
        // section) — the angle argument is out of scope.
        // `parse_font_style` consumes just the `oblique` ident and succeeds,
        // leaving `14deg` unconsumed, so the whole declaration is dropped by
        // `DeclParser::expect_exhausted` — mirrors
        // `rejects_extra_length_after_font_size` above.
        let decls = parse_block("font-style: oblique 14deg;");
        assert!(decls.is_empty());
    }

    #[test]
    fn accepts_text_transform_keyword_combinations_in_either_order() {
        let decls = parse_block("text-transform: uppercase full-width;");
        assert_eq!(decls.len(), 1);
        let decls = parse_block("text-transform: full-width uppercase;");
        assert_eq!(decls.len(), 1);
    }

    #[test]
    fn accepts_hanging_after_text_indent() {
        // "2em hanging" — now accepted (CSS Text 3 §8.1 hanging keyword).
        let decls = parse_block("text-indent: 2em hanging;");
        assert_eq!(decls.len(), 1);
    }

    #[test]
    fn rejects_extra_length_after_vertical_align() {
        // "10px 20px" — `parse_vertical_align`'s `<length>` fallback doesn't
        // consume to end of declaration itself (mirrors every other
        // single-value property parser); the trailing token is unconsumed
        // garbage from `expect_exhausted`'s point of view and the whole
        // declaration drops — mirrors `rejects_extra_length_after_font_size`.
        let decls = parse_block("vertical-align: 10px 20px;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_ident_after_vertical_align() {
        // "middle top" — `top` is spec-valid (CSS 2.1 §10.8.1) but out of
        // this crate's scope (`VerticalAlign` doc's "Scope carving"
        // section), so it is unconsumed garbage from `expect_exhausted`'s
        // point of view and the whole declaration drops, same shape as
        // `rejects_extra_ident_after_text_indent` above.
        let decls = parse_block("vertical-align: middle top;");
        assert!(decls.is_empty());
    }

    #[test]
    fn still_accepts_important_after_value() {
        // Regression guard: `!important` must still be accepted after the
        // exhaustive-consumption check.
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn font_family_leaves_important_alone() {
        // If the `parse_font_family` loop rejects `!` (from `!important`) as garbage,
        // it drops the entire declaration (Finding 3).
        let decls = parse_block("font-family: Arial !important;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::FontFamily(Arc::new(vec![crate::property::FontFamilyName::named(
                "Arial"
            )]))
        );
        assert!(decls[0].important);
    }

    // ── margin shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` expands the `margin` shorthand into four longhands
    // (`MarginTop` / `MarginRight` / `MarginBottom` / `MarginLeft`).
    // spec §3 "Shorthand Properties"
    // Per "sets all of its longhand sub-properties, exactly as if expanded
    // in place" in <https://www.w3.org/TR/css-cascade-4/#shorthand>, this prevents
    // a shorthand key from reaching the cascade by expanding at parse time.

    #[test]
    fn margin_shorthand_expands_into_four_longhand_declarations() {
        // `margin: 10px 20px` → four longhands (top=10, right=20, bottom=10, left=20).
        let decls = parse_block("margin: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(20.0)))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[3].value,
            PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(20.0)))
        );
    }

    #[test]
    fn margin_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim:
        // "Declaring a shorthand property to be !important is equivalent to
        // declaring all of its sub-properties to be !important": the shorthand's
        // `!important` is copied to all longhands.
        let decls = parse_block("margin: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn margin_shorthand_five_values_declaration_dropped() {
        // For a 5+ value shorthand, `parse_margin_shorthand` consumes four values; the
        // remaining fifth token causes `expect_exhausted` to drop the declaration.
        // Check that this yields zero declarations end to end (complementing
        // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` in property.rs).
        let decls = parse_block("margin: 10px 20px 30px 40px 50px;");
        assert!(
            decls.is_empty(),
            "5-value shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn margin_inline_shorthand_three_values_declaration_dropped() {
        // End-to-end sibling of `margin_shorthand_five_values_declaration_dropped`
        // for the 2-value logical shorthand: 3+ value `margin-inline` must
        // be dropped by `expect_exhausted`, not silently truncated to the
        // first 2 values (property.rs's
        // `margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // pins the bare `parse_value`-level behavior; this pins the
        // end-to-end declaration-block outcome).
        let decls = parse_block("margin-inline: 10px 20px 30px;");
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes when the assertion fails; it passes here, so llvm-cov
        // reports the macro's condition-false region as an uncovered added
        // line even though the assertion itself runs and does its job.
        assert!(
            decls.is_empty(),
            "3-value margin-inline shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn margin_block_shorthand_three_values_declaration_dropped() {
        // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
        // for the block-axis 2-value shorthand.
        let decls = parse_block("margin-block: 10px 20px 30px;");
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes when the assertion fails; it passes here, so llvm-cov
        // reports the macro's condition-false region as an uncovered added
        // line even though the assertion itself runs and does its job.
        assert!(
            decls.is_empty(),
            "3-value margin-block shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn padding_inline_shorthand_three_values_declaration_dropped() {
        // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
        // for `padding-inline`.
        let decls = parse_block("padding-inline: 10px 20px 30px;");
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes when the assertion fails; it passes here, so llvm-cov
        // reports the macro's condition-false region as an uncovered added
        // line even though the assertion itself runs and does its job.
        assert!(
            decls.is_empty(),
            "3-value padding-inline shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn padding_block_shorthand_three_values_declaration_dropped() {
        // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
        // for `padding-block`, the last of the 4 logical 2-value shorthands.
        let decls = parse_block("padding-block: 10px 20px 30px;");
        // cov:ignore: the failure-message branch of this `assert!` only
        // executes when the assertion fails; it passes here, so llvm-cov
        // reports the macro's condition-false region as an uncovered added
        // line even though the assertion itself runs and does its job.
        assert!(
            decls.is_empty(),
            "3-value padding-block shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn margin_longhand_declaration_not_expanded() {
        // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
        // Negative test checking the scope of shorthand-only expansion.
        let decls = parse_block("margin-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
    }

    // ── padding shorthand expansion (CSS Cascading L4 §3) ──

    #[test]
    fn padding_shorthand_expands_into_four_longhand_declarations() {
        // `padding: 10px 20px` → four longhands (top=10, right=20, bottom=10, left=20).
        // Migrate margin's parse-time expansion model to padding.
        let decls = parse_block("padding: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
        assert_eq!(
            decls[1].value,
            PropertyValue::PaddingRight(Length::Px(20.0))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::PaddingBottom(Length::Px(10.0))
        );
        assert_eq!(decls[3].value, PropertyValue::PaddingLeft(Length::Px(20.0)));
    }

    #[test]
    fn padding_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading L4 §3: a shorthand's `!important` is copied to every longhand,
        // as in the margin important extension.
        let decls = parse_block("padding: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn padding_longhand_declaration_not_expanded() {
        // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
        let decls = parse_block("padding-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
    }

    // ── border shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` expands the `border` shorthand into 12 longhands
    // (four sides × three sub-properties: width / style / color).
    // Under spec §3 "Shorthand Properties", which "sets all of its longhand sub-properties,
    // exactly as if expanded in place", parse-time expansion prevents shorthand
    // keys from reaching the cascade. Extends the margin / padding
    // precedent to a 12-longhand shape.

    #[test]
    fn border_shorthand_expands_into_twelve_longhand_declarations() {
        // `border: 1px solid red` → 12 longhands (four sides × {width, style, color}).
        // order: top-w / top-s / top-c / right-w / right-s / right-c / bottom-* /
        // Check the handwritten order of the `left-*` longhands in
        // `expand_shorthand_into` to catch copy-paste regressions.
        let decls = parse_block("border: 1px solid red;");
        assert_eq!(
            decls.len(),
            12,
            "border shorthand must expand to 12 longhand decls"
        );
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        // The color longhand reaches the cascade as `BorderColor::Resolved(red)`:
        // the author's color slot in the shorthand is passed as the
        // Resolved variant (pin for hazard case 3).
        let red_bc = BorderColor::Resolved(red);
        assert_eq!(
            decls[0].value,
            PropertyValue::BorderTopWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::BorderTopStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[2].value, PropertyValue::BorderTopColor(red_bc));
        assert_eq!(
            decls[3].value,
            PropertyValue::BorderRightWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[4].value,
            PropertyValue::BorderRightStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[5].value, PropertyValue::BorderRightColor(red_bc));
        assert_eq!(
            decls[6].value,
            PropertyValue::BorderBottomWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[7].value,
            PropertyValue::BorderBottomStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[8].value, PropertyValue::BorderBottomColor(red_bc));
        assert_eq!(
            decls[9].value,
            PropertyValue::BorderLeftWidth(Length::Px(1.0))
        );
        assert_eq!(
            decls[10].value,
            PropertyValue::BorderLeftStyle(BorderStyle::Solid)
        );
        assert_eq!(decls[11].value, PropertyValue::BorderLeftColor(red_bc));
    }

    #[test]
    fn border_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
        // as in the margin / padding important extensions; check all 12 longhands.
        // as a regression check.
        let decls = parse_block("border: 5px dashed blue !important;");
        assert_eq!(decls.len(), 12);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn border_longhand_declaration_not_expanded() {
        // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
        // Negative test checking shorthand-only expansion scope, as in the margin /
        // padding sibling tests.
        let decls = parse_block("border-top-width: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::BorderTopWidth(Length::Px(10.0))
        );
    }

    #[test]
    fn border_side_family_shorthands_expand_into_four_longhands() {
        use crate::property::PropertyKey as K;
        let decls = parse_block(
            "border-style: solid dashed; border-width: 1px 2px 3px 4px; \
             border-color: red !important;",
        );
        let keys: Vec<_> = decls.iter().map(|d| d.value.key()).collect();
        assert_eq!(
            keys,
            [
                K::BorderTopStyle,
                K::BorderRightStyle,
                K::BorderBottomStyle,
                K::BorderLeftStyle,
                K::BorderTopWidth,
                K::BorderRightWidth,
                K::BorderBottomWidth,
                K::BorderLeftWidth,
                K::BorderTopColor,
                K::BorderRightColor,
                K::BorderBottomColor,
                K::BorderLeftColor,
            ]
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::BorderRightStyle(BorderStyle::Dashed)
        );
        assert_eq!(
            decls[7].value,
            PropertyValue::BorderLeftWidth(Length::Px(4.0))
        );
        assert!(decls[..8].iter().all(|d| !d.important));
        assert!(decls[8..].iter().all(|d| d.important));
    }

    #[test]
    fn border_shorthand_two_widths_declaration_dropped() {
        // property.rs `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
        // End-to-end check: with `border: 1px 2px`, the shorthand helper puts 1px in
        // the width slot, then 2px matches neither style nor color. It falls through
        // and remains unconsumed, so the caller's `expect_exhausted` drops the whole
        // declaration (zero declarations).
        let decls = parse_block("border: 1px 2px;");
        assert!(
            decls.is_empty(),
            "border shorthand with leftover token must be dropped, got {decls:?}"
        );
    }

    // ── outline shorthand expansion (CSS Basic User Interface Module Level 3 §4.1) ──

    #[test]
    fn outline_shorthand_expands_into_three_longhand_declarations() {
        // CSS Cascading 4 §3: a shorthand sets each longhand as if expanded in
        // place. The outline-only `auto` style must stay an `OutlineStyle`.
        let decls = parse_block("outline: auto 2px red;");
        assert_eq!(decls.len(), 3, "shorthand must expand to 3 longhand decls");
        assert_eq!(decls[0].value, PropertyValue::OutlineWidth(Length::Px(2.0)));
        assert_eq!(
            decls[1].value,
            PropertyValue::OutlineStyle(OutlineStyle::Auto)
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::OutlineColor(OutlineColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }))
        );
    }

    #[test]
    fn outline_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading 4 §3: `!important` on a shorthand applies to all of
        // its longhands.
        let decls = parse_block("outline: auto !important;");
        assert_eq!(decls.len(), 3);
        assert!(decls.iter().all(|declaration| declaration.important));
    }

    // ── font shorthand expansion (CSS Fonts 4 §2.1) ──

    #[test]
    fn font_shorthand_expands_into_fifteen_longhand_declarations() {
        // The first 6 declarations are grammar longhands. CSS Fonts 4 §2.1
        // then requires the 9 modeled reset-only subproperties to use their
        // initial values; `font-variant-caps` is already a grammar longhand.
        let decls = parse_block("font: italic small-caps bold 12px/1.5 serif;");
        assert_eq!(decls.len(), 15);
        assert_eq!(decls[0].value, PropertyValue::FontStyle(FontStyle::Italic));
        assert_eq!(
            decls[1].value,
            PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps)
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))
        );
        assert_eq!(decls[3].value, PropertyValue::FontSize(Length::Px(12.0)));
        assert_eq!(
            decls[4].value,
            PropertyValue::LineHeight(LineHeight::Number(1.5))
        );
        assert_eq!(
            decls[5].value,
            PropertyValue::FontFamily(Arc::new(vec![crate::property::FontFamilyName::generic(
                "serif"
            )]))
        );
        assert_eq!(
            decls[6].value,
            PropertyValue::FontKerning(FontKerning::Auto)
        );
        assert_eq!(
            decls[7].value,
            PropertyValue::FontLanguageOverride(FontLanguageOverride::Normal)
        );
        assert_eq!(
            decls[8].value,
            PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto)
        );
        assert_eq!(
            decls[9].value,
            PropertyValue::FontVariantEastAsian(FontVariantEastAsian::initial())
        );
        assert_eq!(
            decls[10].value,
            PropertyValue::FontVariantEmoji(FontVariantEmoji::Normal)
        );
        assert_eq!(
            decls[11].value,
            PropertyValue::FontVariantLigatures(FontVariantLigatures::Normal)
        );
        assert_eq!(
            decls[12].value,
            PropertyValue::FontVariantNumeric(FontVariantNumeric::initial())
        );
        assert_eq!(
            decls[13].value,
            PropertyValue::FontVariantPosition(FontVariantPosition::Normal)
        );
        assert_eq!(
            decls[14].value,
            PropertyValue::FontVariationSettings(FontVariationSettings::Normal)
        );
    }

    #[test]
    fn font_shorthand_omitted_components_expand_to_initial_values() {
        // CSS Fonts 4 §2.1 fills omitted grammar components and resets modeled
        // reset-only subproperties to their initial values.
        let decls = parse_block("font: 12px serif;");
        assert_eq!(decls.len(), 15);
        assert_eq!(decls[0].value, PropertyValue::FontStyle(FontStyle::Normal));
        assert_eq!(
            decls[1].value,
            PropertyValue::FontVariantCaps(FontVariantCaps::Normal)
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::FontWeight(FontWeightValue::Absolute(400.0))
        );
        assert_eq!(
            decls[4].value,
            PropertyValue::LineHeight(LineHeight::Normal)
        );
        assert_eq!(
            decls[6].value,
            PropertyValue::FontKerning(FontKerning::Auto)
        );
        assert_eq!(
            decls[7].value,
            PropertyValue::FontLanguageOverride(FontLanguageOverride::Normal)
        );
        assert_eq!(
            decls[8].value,
            PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto)
        );
        assert_eq!(
            decls[9].value,
            PropertyValue::FontVariantEastAsian(FontVariantEastAsian::initial())
        );
        assert_eq!(
            decls[10].value,
            PropertyValue::FontVariantEmoji(FontVariantEmoji::Normal)
        );
        assert_eq!(
            decls[11].value,
            PropertyValue::FontVariantLigatures(FontVariantLigatures::Normal)
        );
        assert_eq!(
            decls[12].value,
            PropertyValue::FontVariantNumeric(FontVariantNumeric::initial())
        );
        assert_eq!(
            decls[13].value,
            PropertyValue::FontVariantPosition(FontVariantPosition::Normal)
        );
        assert_eq!(
            decls[14].value,
            PropertyValue::FontVariationSettings(FontVariationSettings::Normal)
        );
    }

    #[test]
    fn font_shorthand_relative_size_expands_to_font_size_relative_longhand() {
        // `larger` keeps its `FontSizeRelative` carrier (same `PropertyKey`
        // as `FontSize`, so cascade still sees a single slot).
        let decls = parse_block("font: italic larger serif;");
        assert_eq!(decls.len(), 15);
        assert_eq!(
            decls[3].value,
            PropertyValue::FontSizeRelative(RelativeFontSize::Larger)
        );
        assert_eq!(
            decls[3].value.key(),
            PropertyValue::FontSize(Length::Px(16.0)).key()
        );
    }

    #[test]
    fn font_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading 4 §3: a shorthand's `!important` is copied to all longhands,
        // as in the outline / overflow important extensions.
        let decls = parse_block("font: italic 12px serif !important;");
        assert_eq!(decls.len(), 15);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn font_shorthand_invalid_declaration_expands_to_nothing() {
        // System-font keywords / a missing family drop the whole declaration,
        // leaving nothing at the block exit.
        assert!(parse_block("font: menu;").is_empty());
        assert!(parse_block("font: italic 12px;").is_empty());
    }

    // ── overflow shorthand expansion (CSS Overflow 3 §3.1) ──

    #[test]
    fn overflow_shorthand_expands_into_two_longhand_declarations() {
        // `overflow: hidden scroll` → 2 longhand (x=hidden, y=scroll), spec
        // order per §3.1 "sets the specified values of overflow-x and
        // overflow-y in that order".
        let decls = parse_block("overflow: hidden scroll;");
        assert_eq!(decls.len(), 2, "shorthand must expand to 2 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Hidden)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::OverflowY(OverflowValue::Scroll)
        );
    }

    #[test]
    fn overflow_shorthand_one_value_expands_to_both_axes() {
        // §3.1 "If the second value is omitted, it is copied from the first."
        let decls = parse_block("overflow: auto;");
        assert_eq!(decls.len(), 2);
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Auto)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::OverflowY(OverflowValue::Auto)
        );
    }

    #[test]
    fn overflow_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
        // as in the margin / padding / border important extensions.
        let decls = parse_block("overflow: hidden !important;");
        assert_eq!(decls.len(), 2);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn overflow_shorthand_three_values_declaration_dropped() {
        // property.rs
        // `overflow_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // End-to-end check: `parse_overflow_shorthand` consumes two values. The third,
        // unconsumed token causes `expect_exhausted` to drop the entire declaration
        // (zero declarations, as in the margin 5-value sibling test).
        let decls = parse_block("overflow: hidden scroll auto;");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            decls.is_empty(),
            "3-value shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn overflow_longhand_declaration_not_expanded() {
        // A longhand passes through the `expand_shorthand_into` match arm unchanged
        // (one declaration). Negative test checking the scope of shorthand-only expansion,
        // as in the margin / padding / border sibling tests.
        let decls = parse_block("overflow-x: hidden;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::OverflowX(OverflowValue::Hidden)
        );
    }

    // ── text-decoration shorthand expansion (CSS Text Decoration Module
    // Level 3 §2.4) ──
    //
    // `parse_declaration_block` expands `text-decoration` into three longhands
    // (`TextDecorationLine` / `TextDecorationStyle` / `TextDecorationColor`)
    // following the margin / padding / border / overflow precedent.
    // parse-time expansion model.

    #[test]
    fn text_decoration_shorthand_expands_into_four_longhand_declarations() {
        use crate::property::{
            TextDecorationColor, TextDecorationLine, TextDecorationStyle, TextDecorationThickness,
        };

        // `text-decoration: underline` produces four longhands; omitted components
        // (thickness/style/color) get spec initial values (see "Initial value fill" in
        // `parse_text_decoration_shorthand` in property.rs).
        let decls = parse_block("text-decoration: underline;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::TextDecorationThickness(TextDecorationThickness::Auto)
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
        );
        assert_eq!(
            decls[3].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
        );
    }

    #[test]
    fn text_decoration_shorthand_important_flag_propagates_to_all_longhand() {
        // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
        // as in the margin / padding / border / overflow important extensions.
        // pattern).
        let decls = parse_block("text-decoration: underline !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn text_decoration_shorthand_two_style_components_declaration_dropped() {
        // property.rs
        // `text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check`
        // End-to-end check: a second style keyword is unconsumed; `expect_exhausted`
        // detects it and drops the entire declaration (zero declarations).
        let decls = parse_block("text-decoration: solid wavy;");
        assert!(
            decls.is_empty(),
            "2 style components must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn text_decoration_longhand_declarations_not_expanded() {
        use crate::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

        // A longhand passes through the `expand_shorthand_into` match arm unchanged
        // (one declaration). Negative test checking shorthand-only expansion scope,
        // as in the margin / padding / border / overflow sibling tests.
        let decls = parse_block("text-decoration-line: underline;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
        );

        let decls = parse_block("text-decoration-style: wavy;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
        );

        let decls = parse_block("text-decoration-color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }))
        );
    }

    #[test]
    fn text_decoration_shorthand_always_overwrites_all_four_longhand() {
        // Shorthand-resets-omitted-longhands: per CSS Cascading L4 §3's
        // "exactly as if expanded in place", `text-decoration: underline`
        // (style/color omitted) still emits a `TextDecorationStyle::Solid` /
        // `TextDecorationColor::CurrentColor` declaration alongside the
        // line one — it does not "leave the other two alone". This is what
        // lets a later bare `text-decoration: underline` reset an earlier
        // `text-decoration-style: wavy` back to `solid` through ordinary
        // cascade order-of-appearance (see
        // `crate::cascade::tests::text_decoration_shorthand_resets_earlier_longhand_declarations` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — invisible to rustdoc, confirmed by deliberately breaking the link
        // for the end-to-end cascade pin).
        use crate::property::{TextDecorationColor, TextDecorationStyle};

        let decls = parse_block("text-decoration-style: wavy; text-decoration: underline;");
        assert_eq!(decls.len(), 5, "1 longhand + 4 expanded, in source order");
        assert_eq!(
            decls[0].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
        );
        // The 2nd declaration is the shorthand's TextDecorationLine, the 3rd
        // its thickness — the 4th is the discriminating one: the shorthand's
        // own Solid, appearing *after* the earlier explicit Wavy.
        assert_eq!(
            decls[3].value,
            PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
        );
        assert_eq!(
            decls[4].value,
            PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
        );
    }

    #[test]
    fn text_decoration_line_duplicate_and_none_combination_declarations_dropped() {
        // End-to-end check for the leftover-token cases property.rs's
        // `text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check`
        // and `text_decoration_line_none_combined_with_a_keyword_leaves_leftover`
        // exercise at the `parse_text_decoration_line` helper level: the
        // leftover token they leave unconsumed is caught here by
        // `expect_exhausted` (`rule.rs`'s `DeclParser`), dropping the whole
        // declaration (0 decl), same shape as
        // `rejects_trailing_garbage_after_value`.
        assert!(parse_block("text-decoration-line: underline underline;").is_empty());
        assert!(parse_block("text-decoration-line: none underline;").is_empty());
        assert!(parse_block("text-decoration-line: underline none;").is_empty());
    }
}
