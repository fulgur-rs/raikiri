//! CSS rule と declaration の shape。type/universal selector を含む
//! qualified rule (StyleRule) のみサポート。at-rule (@page / @media 等) は
//! ruletree.rs 側で skip。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::RaikiriSelectorImpl;
use crate::property::{
    BackgroundShorthand, Border, BorderColor, BorderStyle, DeferredValue, FlexFlow, FlexShorthand,
    FontShorthand, FontShorthandSize, GapShorthand, GridLineShorthand, Length, LengthOrAuto,
    Outline, OverflowXY, PlaceContentShorthand, PlaceItemsShorthand, PlaceSelfShorthand,
    PropertyKey, PropertyValue, Sides, StartEnd, TextDecorationShorthand, parse_value,
};

/// 1 property declaration = value + `!important` flag。
///
/// `value` が私有なので、crate 外からは struct literal / functional-update
/// のいずれでも構築できない。
///
/// # write 経路が無いことの compile-fail check
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
/// functional-update (`..base`) 経由の構築も同じ理由 (`value` が private) で
/// reject される。
///
/// `value` の visibility だけを独立に discriminate するのは下の 3 番目の
/// fence (`.clone()` 後の field 代入) だけである:
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
    /// resolved property value。
    pub(crate) value: PropertyValue,
    /// `!important` flag (true なら importance 上げ)。
    pub important: bool,
}

impl Declaration {
    /// resolved property value への read-only accessor。
    pub fn value(&self) -> &PropertyValue {
        &self.value
    }
}

/// Qualified style rule (`selectors { declarations }`)。
///
/// `source_order` は同一 `RuleTree` 内で 0 から通し番号。cascade tie-break
/// (同 specificity 時に「後勝ち」) に使う。
/// `origin` は CSS Cascading L4 §6.2 の origin。cascade tuple
/// の rank 化 (`!important` 反転扱い) に使用。
/// Future field (specificity cache / invalidation hint 等) は将来追加予定、
/// `#[non_exhaustive]` の恩恵で non-breaking。
#[non_exhaustive]
pub struct StyleRule {
    /// Parse 済 selector list。type + universal のみ受理 (他は build 段で drop)。
    pub selectors: SelectorList<RaikiriSelectorImpl>,
    /// このルールの declaration list (invalid は含まない)。
    pub(crate) declarations: Vec<Declaration>,
    /// RuleTree 全体を通した 0-indexed source order。
    pub source_order: u32,
    /// この rule が属する cascade origin。
    pub origin: crate::ruletree::Origin,
}

impl StyleRule {
    /// このルールの declaration list への read-only accessor
    /// (invalid は含まない)。
    ///
    /// # `declarations` field 自体への到達不能性
    ///
    /// `declarations` field は `pub(crate)` — external crate から届くのは
    /// この accessor だけである。`StyleRule` は `Clone` を derive していない
    /// ので、external crate は `.clone()` で所有値の `StyleRule` を得る経路が
    /// そもそも無い。加えて `#[non_exhaustive]` が struct literal /
    /// functional-update による新規構築も塞いでいる — この 2 つは独立な
    /// gate であり、`StyleRule` を external crate が所有値として保持する
    /// 経路はどちらの意味でも存在しない。触れられるのはこの accessor が返す
    /// `&[Declaration]` だけである。したがって「write 経路」を意味のある形で
    /// discriminate する check は存在しない (どんな可視性でも `&StyleRule` から
    /// は書けない) が、
    /// `declarations` という field 名そのものが private であることは以下で
    /// 直接 check できる — `pub` に戻れば以下は compile が通るようになる:
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

/// declaration-list を消費して `Vec<Declaration>` を produce。
/// 認識できない property name / invalid value は silently drop。
///
/// # Shorthand expansion
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
/// property sets all of its longhand sub-properties, exactly as if expanded
/// in place." に準拠して、[`PropertyValue::Margin`] 系の shorthand declaration
/// は本関数の出口で 4 longhand declaration に展開される。cascade 段の
/// per-side winner selection が自然に成立することを担保するための spec-correct
/// な expansion — 詳細は [`expand_shorthand_into`] doc 参照。
pub(crate) fn parse_declaration_block(input: &mut Parser<'_, '_>) -> Vec<Declaration> {
    let mut parser = DeclParser;
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
    match d.value {
        PropertyValue::Margin(sides) => expand_margin(sides, d.important, push),
        PropertyValue::MarginInherit => expand_margin_inherit(d.important, push),
        PropertyValue::Padding(sides) => expand_padding(sides, d.important, push),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, d.important, push),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, d.important, push),
        PropertyValue::PaddingInline(pair) => expand_padding_inline(pair, d.important, push),
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, d.important, push),
        PropertyValue::Border(sides) => expand_border(sides, d.important, push),
        PropertyValue::BorderStyle(sides) => expand_border_style(sides, d.important, push),
        PropertyValue::BorderWidth(sides) => expand_border_width(sides, d.important, push),
        PropertyValue::BorderColor(sides) => expand_border_color(sides, d.important, push),
        PropertyValue::Overflow(pair) => expand_overflow(pair, d.important, push),
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, d.important, push)
        }
        PropertyValue::Outline(outline) => expand_outline(outline, d.important, push),
        // `FontShorthand` は `family: Arc<Vec<Atom>>` を持ち `Copy` ではない —
        // 他の shorthand payload (`Sides<..>` / `FlexShorthand` 等、全て Copy)
        // と異なり値を move できないため、`ref` binding で参照のまま渡す
        // (`Background` arm と同じ理由、`GridRow` arm の comment 参照)。
        PropertyValue::Font(ref shorthand) => expand_font(shorthand, d.important, push),
        PropertyValue::Deferred(ref deferred) => {
            expand_deferred(d, deferred, d.important, push)
        }
        // 展開先の longhand variant を持たない — そのまま 1 個 push。
        // `_` に潰さないこと (上の「wildcard arm を置かない理由 (契約)」節)。
        // ここへ variant を足すことは「展開先が無い」という主張である。
        PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::CalcLengthPercentage { .. }
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
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
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
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::VerticalAlign(_)
        | PropertyValue::FontStyle(_)
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
        | PropertyValue::TextWrap(_)
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
        // + grid-row-start/-end + grid-column-start/-end — 展開先の longhand
        // を持たない individual property (CSS Grid Layout Module Level 1
        // §7.2/§7.3/§7.6/§7.7/§8.3)。
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
        // §7.1/§6.1) — 同じく展開先の longhand を持たない。
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        // orphans / widows (CSS Fragmentation Module Level 3 §3.3) — 同じく
        // 展開先の longhand を持たない。
        | PropertyValue::Orphans(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Widows(_)
        // writing-mode (CSS Writing Modes 4 §3.2) — 同じく展開先の longhand を
        // 持たない。
        | PropertyValue::WritingMode(_)
        | PropertyValue::RubyPosition(_)
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position /
        // background-image (CSS Backgrounds and Borders 3 §2.3-§2.9) —
        // 同じく展開先の longhand を持たない (`background` shorthand は
        // 別 variant `PropertyValue::Background` で、下の
        // `expand_background` arm が展開する)。
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)
        | PropertyValue::BackgroundSize(_)
        | PropertyValue::BackgroundPosition(_)
        | PropertyValue::BackgroundImage(_)
        // object-fit / object-position (CSS Images Module Level 3 §5.1/§5.2)
        // / opacity (CSS Color 4 §3.3) — 同じく展開先の longhand を持たない
        // (shorthand を持たない standalone property)。
        | PropertyValue::ObjectFit(_)
        | PropertyValue::ObjectPosition(_)
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
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_) | PropertyValue::TextCombineUpright(_) | PropertyValue::TextOrientation(_) | PropertyValue::UnicodeBidi(_) | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_) | PropertyValue::ColumnWidth(_) => expand_none(d, push),
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
        PropertyValue::Flex(f) => expand_flex(f, d.important, push),
        PropertyValue::FlexFlow(f) => expand_flex_flow(f, d.important, push),
        PropertyValue::Gap(g) => expand_gap(g, d.important, push),
        PropertyValue::PlaceContent(p) => expand_place_content(p, d.important, push),
        // `GridLineShorthand` は `SmolStr` を持ち Copy ではない — 他の
        // shorthand payload (`Sides<..>` / `FlexShorthand` / `GapShorthand`
        // 等、全て Copy) と異なり値を `d.value` (`&Declaration` 経由の
        // place) から move できないため、`ref` binding で参照のまま
        // `expand_grid_row` / `expand_grid_column` に渡す
        // (`expand_grid_row` doc 参照)。
        PropertyValue::GridRow(ref shorthand) => expand_grid_row(shorthand, d.important, push),
        PropertyValue::GridColumn(ref shorthand) => {
            expand_grid_column(shorthand, d.important, push)
        }
        PropertyValue::PlaceItems(p) => expand_place_items(p, d.important, push),
        PropertyValue::PlaceSelf(p) => expand_place_self(p, d.important, push),
        // `BackgroundShorthand` holds a `BackgroundImage` field, which is not
        // `Copy` (it can carry a `Gradient`) — same reason `GridLineShorthand`
        // needs `ref` binding here (`GridRow` arm's comment above), not the
        // `Sides<..>`/`FlexShorthand`/`GapShorthand` by-value pattern the
        // other shorthand arms use.
        PropertyValue::Background(ref shorthand) => {
            expand_background(shorthand, d.important, push)
        }
    }
}

/// non-shorthand の共通 path。`expand_shorthand_into` から分離してあるのは
/// hot path の code size を最小に保つため (下の per-family helper と対)。
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
        PropertyKey::Outline => &[
            PropertyKey::OutlineWidth,
            PropertyKey::OutlineStyle,
            PropertyKey::OutlineColor,
        ],
        // `font` shorthand — 6 longhand (style/variant-caps/weight/size/
        // line-height/family)。`size` は `PropertyValue::FontSize` /
        // `PropertyValue::FontSizeRelative` のどちらで勝っても key は
        // `PropertyKey::FontSize` の 1 つのため slice は 6 要素
        // (`property.rs` の `FontShorthandSize` doc 参照)。
        PropertyKey::Font => &[
            PropertyKey::FontStyle,
            PropertyKey::FontVariantCaps,
            PropertyKey::FontWeight,
            PropertyKey::FontSize,
            PropertyKey::LineHeight,
            PropertyKey::FontFamily,
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

/// `margin` shorthand を 4 longhand に展開する cold helper。
#[inline(never)]
fn expand_margin(sides: Sides<LengthOrAuto>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::MarginTop(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginRight(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginBottom(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginLeft(sides.left),
        important,
    });
}

/// Page-context `margin: inherit` shorthand marker expansion.
#[inline(never)]
fn expand_margin_inherit(important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::MarginTopInherit,
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginRightInherit,
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginBottomInherit,
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginLeftInherit,
        important,
    });
}

/// `padding` shorthand を 4 longhand に展開する cold helper。
#[inline(never)]
fn expand_padding(sides: Sides<Length>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::PaddingTop(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingRight(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingBottom(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingLeft(sides.left),
        important,
    });
}

/// `margin-inline` shorthand を `margin-left`/`margin-right` の 2 longhand に
/// 展開する cold helper。margin/padding/overflow shorthand precedent と
/// 同 pattern (2-value なので push は 2 回のみ)。
///
/// 物理写像 (inline axis → left/right、`direction: ltr` 仮定の近似) の
/// rationale は [`crate::property::PropertyValue::MarginInline`] doc 参照。
#[inline(never)]
fn expand_margin_inline(
    pair: StartEnd<LengthOrAuto>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::MarginLeft(pair.start),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginRight(pair.end),
        important,
    });
}

/// `margin-block` shorthand を `margin-top`/`margin-bottom` の 2 longhand に
/// 展開する cold helper — [`expand_margin_inline`] の block-axis sibling
/// (block axis は `direction` に依存しない厳密写像、
/// [`crate::property::PropertyValue::PaddingInline`] doc の「非対称」節参照)。
#[inline(never)]
fn expand_margin_block(
    pair: StartEnd<LengthOrAuto>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::MarginTop(pair.start),
        important,
    });
    push(Declaration {
        value: PropertyValue::MarginBottom(pair.end),
        important,
    });
}

/// `padding-inline` shorthand を `padding-left`/`padding-right` の 2
/// longhand に展開する cold helper — [`expand_margin_inline`] の padding
/// sibling。
#[inline(never)]
fn expand_padding_inline(
    pair: StartEnd<Length>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::PaddingLeft(pair.start),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingRight(pair.end),
        important,
    });
}

/// `padding-block` shorthand を `padding-top`/`padding-bottom` の 2
/// longhand に展開する cold helper — [`expand_margin_block`] の padding
/// sibling。
#[inline(never)]
fn expand_padding_block(
    pair: StartEnd<Length>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::PaddingTop(pair.start),
        important,
    });
    push(Declaration {
        value: PropertyValue::PaddingBottom(pair.end),
        important,
    });
}

/// `border` shorthand (CSS Backgrounds 3 §3.4 "Border Shorthand Properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-shorthands>) を 12 longhand
/// (4 side × 3 sub-property = width / style / color) に展開する。
/// margin / padding shorthand precedent と同 pattern。
/// spec `border` grammar は 4 side 共通 (`Sides::all(border)`) だが、cascade
/// 段では per-side longhand として書き込むことで、`border: 1px solid red;
/// border-top-color: blue;` のような longhand override が per-side
/// determinism で解決する。
#[inline(never)]
fn expand_border(sides: Sides<Border>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::BorderTopWidth(sides.top.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderTopStyle(sides.top.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderTopColor(sides.top.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightWidth(sides.right.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightStyle(sides.right.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightColor(sides.right.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomWidth(sides.bottom.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomStyle(sides.bottom.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomColor(sides.bottom.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftWidth(sides.left.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftStyle(sides.left.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftColor(sides.left.color),
        important,
    });
}

/// `border-style` shorthand を 4 longhand (`border-*-style`) に展開する
/// cold helper。margin/padding/border shorthand precedent と同 pattern。
#[inline(never)]
fn expand_border_style(
    sides: Sides<BorderStyle>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::BorderTopStyle(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightStyle(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomStyle(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftStyle(sides.left),
        important,
    });
}

/// `border-width` shorthand を 4 longhand (`border-*-width`) に展開する
/// cold helper。margin/padding/border shorthand precedent と同 pattern。
#[inline(never)]
fn expand_border_width(sides: Sides<Length>, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::BorderTopWidth(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightWidth(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomWidth(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftWidth(sides.left),
        important,
    });
}

/// `border-color` shorthand を 4 longhand (`border-*-color`) に展開する
/// cold helper。margin/padding/border shorthand precedent と同 pattern。
#[inline(never)]
fn expand_border_color(
    sides: Sides<BorderColor>,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::BorderTopColor(sides.top),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderRightColor(sides.right),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderBottomColor(sides.bottom),
        important,
    });
    push(Declaration {
        value: PropertyValue::BorderLeftColor(sides.left),
        important,
    });
}

/// `overflow` shorthand (CSS Overflow 3 §3.1
/// <https://www.w3.org/TR/css-overflow-3/#overflow-properties>) を
/// `overflow-x` / `overflow-y` の 2 longhand に展開する cold helper。
/// margin / padding / border shorthand precedent と
/// 同 pattern — 2-axis なので push は 2 回のみ。
#[inline(never)]
fn expand_overflow(pair: OverflowXY, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::OverflowX(pair.x),
        important,
    });
    push(Declaration {
        value: PropertyValue::OverflowY(pair.y),
        important,
    });
}

/// `flex` shorthand を `flex-grow` / `flex-shrink` / `flex-basis` の 3
/// longhand に展開する cold helper。margin / padding / border shorthand
/// precedent と同 pattern — shorthand parser (`property.rs` の
/// `parse_flex_shorthand`、private fn のため直接 link 不可) が既に
/// shorthand-local default (grow=1 / shrink=1 / basis=0px) を埋めているため
/// ([`FlexShorthand`] doc の "Omitted-component defaults" 節参照)、本関数は
/// 3 field をそのまま 3 declaration に分配するだけでよい。
#[inline(never)]
fn expand_flex(f: FlexShorthand, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::FlexGrow(f.grow),
        important,
    });
    push(Declaration {
        value: PropertyValue::FlexShrink(f.shrink),
        important,
    });
    push(Declaration {
        value: PropertyValue::FlexBasis(f.basis),
        important,
    });
}

/// `flex-flow` shorthand を `flex-direction` / `flex-wrap` の 2 longhand に
/// 展開する cold helper。`expand_flex` と同 pattern — shorthand parser
/// (`parse_flex_flow`) が既に省略成分の initial (row / nowrap) を埋めて
/// いるため、本関数は 2 field をそのまま 2 declaration に分配するだけでよい。
#[inline(never)]
fn expand_flex_flow(f: FlexFlow, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::FlexDirection(f.direction),
        important,
    });
    push(Declaration {
        value: PropertyValue::FlexWrap(f.wrap),
        important,
    });
}

/// `gap` shorthand を `row-gap` / `column-gap` の 2 longhand に展開する cold
/// helper。[`GapShorthand`] doc の 2nd-value-omitted copy 規則は parser 側
/// (`parse_gap_shorthand`) が既に適用済み — 本関数は 2 field をそのまま 2
/// declaration に分配するだけでよい。
#[inline(never)]
fn expand_gap(g: GapShorthand, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::RowGap(g.row),
        important,
    });
    push(Declaration {
        value: PropertyValue::ColumnGap(g.column),
        important,
    });
}

/// `place-content` shorthand を `align-content` / `justify-content` の 2
/// longhand に展開する cold helper — [`expand_gap`] と同じ shape
/// ([`PlaceContentShorthand`] doc 参照)。
#[inline(never)]
fn expand_place_content(
    p: PlaceContentShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::AlignContent(p.align),
        important,
    });
    push(Declaration {
        value: PropertyValue::JustifyContent(p.justify),
        important,
    });
}

/// `grid-row` shorthand を `grid-row-start` / `grid-row-end` の 2 longhand
/// に展開する cold helper。[`GridLineShorthand`] doc の 2nd-value-omitted
/// copy 規則は parser 側 (`property.rs` の `parse_grid_line_shorthand`、
/// private fn のため直接 link 不可) が既に適用済み。
///
/// `shorthand: &GridLineShorthand` — [`expand_shorthand_into`] の
/// `PropertyValue::GridRow(ref shorthand)` arm の doc 参照。他の shorthand
/// expand helper (`expand_flex` 等) と異なり参照を受け取り `.clone()` する
/// (`GridLineValue` が `SmolStr` を持ち Copy ではないため)。
#[inline(never)]
fn expand_grid_row(
    shorthand: &GridLineShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::GridRowStart(shorthand.start.clone()),
        important,
    });
    push(Declaration {
        value: PropertyValue::GridRowEnd(shorthand.end.clone()),
        important,
    });
}

/// `grid-column` shorthand を `grid-column-start` / `grid-column-end` の 2
/// longhand に展開する cold helper — [`expand_grid_row`] と同じ shape。
#[inline(never)]
fn expand_grid_column(
    shorthand: &GridLineShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::GridColumnStart(shorthand.start.clone()),
        important,
    });
    push(Declaration {
        value: PropertyValue::GridColumnEnd(shorthand.end.clone()),
        important,
    });
}

/// `place-items` shorthand を `align-items` / `justify-items` の 2 longhand
/// に展開する cold helper — [`expand_gap`] と同じ shape
/// ([`PlaceItemsShorthand`] doc 参照)。
#[inline(never)]
fn expand_place_items(p: PlaceItemsShorthand, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::AlignItems(p.align),
        important,
    });
    push(Declaration {
        value: PropertyValue::JustifyItems(p.justify),
        important,
    });
}

/// `place-self` shorthand を `align-self` / `justify-self` の 2 longhand に
/// 展開する cold helper — [`expand_place_items`] と同じ shape
/// ([`PlaceSelfShorthand`] doc 参照)。
#[inline(never)]
fn expand_place_self(p: PlaceSelfShorthand, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::AlignSelf(p.align),
        important,
    });
    push(Declaration {
        value: PropertyValue::JustifySelf(p.justify),
        important,
    });
}

/// `text-decoration` shorthand (CSS Text Decoration 4 ED §2.6
/// <https://drafts.csswg.org/css-text-decor-4/#text-decoration-property>) を
/// `text-decoration-line` / `-thickness` / `-style` / `-color` の 4 longhand
/// に展開する cold helper。margin / padding / border / overflow shorthand
/// precedent と同 pattern — 4 longhand は互いに 1:1 disjoint field
/// (`TextDecorationShorthand` doc の「cross-axis coupling が無い」節参照)
/// なので push は 4 回のみ。
///
/// shorthand parser (`property.rs` の `parse_text_decoration_shorthand`、
/// private fn のため直接 link 不可) が既に省略成分を spec initial value で
/// 埋めているため ([`TextDecorationShorthand`] doc の "Initial value fill"
/// 節)、本関数は 4 field をそのまま 4 declaration に分配するだけでよい —
/// margin/padding/border の各 side にも既に initial fill 済みの値が入って
/// いるのと同じ形。
#[inline(never)]
fn expand_text_decoration(
    shorthand: TextDecorationShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::TextDecorationLine(shorthand.line),
        important,
    });
    push(Declaration {
        value: PropertyValue::TextDecorationThickness(shorthand.thickness),
        important,
    });
    push(Declaration {
        value: PropertyValue::TextDecorationStyle(shorthand.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::TextDecorationColor(shorthand.color),
        important,
    });
}

/// `outline` shorthand を width/style/color の longhand に展開する。
#[inline(never)]
fn expand_outline(outline: Outline, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::OutlineWidth(outline.width),
        important,
    });
    push(Declaration {
        value: PropertyValue::OutlineStyle(outline.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::OutlineColor(outline.color),
        important,
    });
}

/// `font` shorthand を 6 longhand (style/variant-caps/weight/size/
/// line-height/family) に展開する cold helper ([`FontShorthand`] doc 参照)。
/// 省略成分は shorthand parser (`property.rs` の `parse_font_shorthand`、
/// private fn のため直接 link 不可) が既に spec initial value で埋めているため
/// ([`FontShorthand`] doc の "Initial value fill" 節)、本関数は 6 field を
/// そのまま 6 declaration に分配するだけでよい —
/// [`expand_text_decoration`] と同じ形。`family` は `Arc` のため `.clone()` は
/// bump のみ ([`BackgroundShorthand`] の `image.clone()` と同じ理由付け)。
/// `size` の 2 通りは対応する [`PropertyValue`] variant にそのまま載せる
/// (どちらも key は [`PropertyKey::FontSize`])。
#[inline(never)]
fn expand_font(shorthand: &FontShorthand, important: bool, mut push: impl FnMut(Declaration)) {
    push(Declaration {
        value: PropertyValue::FontStyle(shorthand.style),
        important,
    });
    push(Declaration {
        value: PropertyValue::FontVariantCaps(shorthand.variant),
        important,
    });
    push(Declaration {
        value: PropertyValue::FontWeight(shorthand.weight),
        important,
    });
    push(Declaration {
        value: match shorthand.size {
            FontShorthandSize::Absolute(length) => PropertyValue::FontSize(length),
            FontShorthandSize::Relative(relative) => PropertyValue::FontSizeRelative(relative),
        },
        important,
    });
    push(Declaration {
        value: PropertyValue::LineHeight(shorthand.line_height),
        important,
    });
    push(Declaration {
        value: PropertyValue::FontFamily(shorthand.family.clone()),
        important,
    });
}

/// `background` shorthand を 8 longhand (color/image/repeat/attachment/
/// position/size/clip/origin) に展開する cold helper
/// ([`BackgroundShorthand`] doc 参照)。`image` は `Copy` ではないため
/// `.clone()` する — `expand_grid_row`/`expand_grid_column` が `GridLineValue`
/// の `Named`/`NamedLine` 成分に対して行うのと同じ理由。
#[inline(never)]
fn expand_background(
    shorthand: &BackgroundShorthand,
    important: bool,
    mut push: impl FnMut(Declaration),
) {
    push(Declaration {
        value: PropertyValue::BackgroundColor(shorthand.color),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundImage(shorthand.image.clone()),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundRepeat(shorthand.repeat),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundAttachment(shorthand.attachment),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundPosition(shorthand.position),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundSize(shorthand.size),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundClip(shorthand.clip),
        important,
    });
    push(Declaration {
        value: PropertyValue::BackgroundOrigin(shorthand.origin),
        important,
    });
}

/// Per-declaration parser for cssparser::RuleBodyParser。
struct DeclParser;

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<Declaration, ParseError<'i, Self::Error>> {
        let value = parse_value(name.as_ref(), input).ok_or_else(|| input.new_custom_error(()))?;
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

// At-rule parser は no-op (block 内で @rule が現れた場合は drop)。
impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

// Qualified-rule parser (nested rule) も no-op — block 内 nested rule は drop。
impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Declaration, ()> for DeclParser {
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
        // cursor: pointer → 未対応 property (CSS Basic User Interface
        // Module Level 3 <https://www.w3.org/TR/css-ui-3/#cursor>) → drop
        // (`margin` / `width` / `float` は以前 dropped 例に使っていたが、
        // その後順次認識対象になったため差し替え。`cursor` は現状
        // unsupported — property.rs `unknown_property_returns_none` の
        // canary と同じ property を使う)。
        // font-size: math → MathML scaling algorithm が未実装のため drop
        // (`medium` を以前 dropped 例に使っていたが、`<absolute-size>` /
        // `<relative-size>` keyword が認識対象になったため差し替え —
        // property.rs `PropertyValue` doc の「例を差し替えるときは…揃えること」
        // 節参照)。
        // color: red → 残す
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
        // "red garbage" — value 後に余計な token があるので declaration ごと drop。
        let decls = parse_block("color: red garbage;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_length_after_font_size() {
        // "16px 20px" — 2 つ目の length は exhaust しない garbage 扱いで drop。
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
        // regression guard: !important は exhaustive-consumption check の後でも
        // 引き続き受理されなければならない。
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn font_family_leaves_important_alone() {
        // parse_font_family の loop が `!` (from `!important`) を garbage として
        // 拒否してしまうと、declaration ごと drop される (Finding 3)。
        let decls = parse_block("font-family: Arial !important;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::FontFamily(Arc::new(vec![crate::Atom::from("Arial")]))
        );
        assert!(decls[0].important);
    }

    // ── margin shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` は shorthand `margin` を 4 longhand
    // (`MarginTop` / `MarginRight` / `MarginBottom` / `MarginLeft`) に展開する。
    // spec §3 "Shorthand Properties"
    // <https://www.w3.org/TR/css-cascade-4/#shorthand> の "sets all of its
    // longhand sub-properties, exactly as if expanded in place" 準拠、cascade 段
    // に shorthand key を届かせない不変を parse-time で担保する。

    #[test]
    fn margin_shorthand_expands_into_four_longhand_declarations() {
        // `margin: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
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
        // declaring all of its sub-properties to be !important." — shorthand の
        // `!important` は全 longhand に copy される。
        let decls = parse_block("margin: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn margin_shorthand_five_values_declaration_dropped() {
        // 5+ value shorthand: parse_margin_shorthand は 4 value 消費、5th 残り
        // token は expect_exhausted で declaration drop。end-to-end で 0 decl
        // になることを check (property.rs の
        // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` と complementary)。
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
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        // shorthand-only expansion の scope を check する negative test。
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
        // `padding: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
        // (margin の parse-time expansion model を padding に migrate)
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
        // spec CSS Cascading L4 §3: shorthand の `!important` は全 longhand に
        // copy される (margin important 拡張と同じ)。
        let decls = parse_block("padding: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn padding_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        let decls = parse_block("padding-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
    }

    // ── border shorthand expansion (CSS Cascading L4 §3) ──
    //
    // `parse_declaration_block` は shorthand `border` を 12 longhand
    // (4 side × 3 sub-property: width / style / color) に展開する。
    // spec §3 "Shorthand Properties" の "sets all of its longhand sub-properties,
    // exactly as if expanded in place" 準拠、cascade 段に shorthand key を
    // 届かせない不変を parse-time で担保する。margin / padding
    // precedent を 12 longhand shape に拡張。

    #[test]
    fn border_shorthand_expands_into_twelve_longhand_declarations() {
        // `border: 1px solid red` → 12 longhand (4 side × {width, style, color})。
        // order: top-w / top-s / top-c / right-w / right-s / right-c / bottom-* /
        // left-* (`expand_shorthand_into` の hand-written
        // order を check することでcopy-paste regression を検知)。
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
        // color longhand は `BorderColor::Resolved(red)`
        // で cascade に届く (shorthand の author-specified color slot は
        // Resolved variant を渡す — hazard case 3 の pin)。
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
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding important 拡張と同 pattern、12 longhand 全て
        // 検証)。
        let decls = parse_block("border: 5px dashed blue !important;");
        assert_eq!(decls.len(), 12);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn border_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl のまま)。
        // shorthand-only expansion の scope を check する negative test (margin /
        // padding sibling と同 pattern)。
        let decls = parse_block("border-top-width: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::BorderTopWidth(Length::Px(10.0))
        );
    }

    #[test]
    fn border_shorthand_two_widths_declaration_dropped() {
        // property.rs `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
        // の end-to-end 側 check: `border: 1px 2px` は shorthand helper が 1px を
        // width slot に置いた後 2px は他 slot (style/color) に match しないため
        // fall-through 到達で leftover になり、caller の `expect_exhausted` が
        // declaration 全体を drop する (0 decl)。
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
    fn font_shorthand_expands_into_six_longhand_declarations() {
        // CSS Fonts 4 §2.1: the shorthand sets each of font-style,
        // font-variant (here: font-variant-caps), font-weight, font-size,
        // line-height and font-family as if expanded in place, in that
        // order.
        let decls = parse_block("font: italic small-caps bold 12px/1.5 serif;");
        assert_eq!(decls.len(), 6, "shorthand must expand to 6 longhand decls");
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
            PropertyValue::FontFamily(Arc::new(vec!["serif".into()]))
        );
    }

    #[test]
    fn font_shorthand_omitted_components_expand_to_initial_values() {
        // CSS Fonts 4 §2.1: omitted components are set to their initial
        // values (`FontShorthand` doc's "Initial value fill" section).
        let decls = parse_block("font: 12px serif;");
        assert_eq!(decls.len(), 6);
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
    }

    #[test]
    fn font_shorthand_relative_size_expands_to_font_size_relative_longhand() {
        // `larger` keeps its `FontSizeRelative` carrier (same `PropertyKey`
        // as `FontSize`, so cascade still sees a single slot).
        let decls = parse_block("font: italic larger serif;");
        assert_eq!(decls.len(), 6);
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
        // CSS Cascading 4 §3: shorthand `!important` は全 longhand に copy
        // される (outline / overflow important 拡張と同 pattern)。
        let decls = parse_block("font: italic 12px serif !important;");
        assert_eq!(decls.len(), 6);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn font_shorthand_invalid_declaration_expands_to_nothing() {
        // System-font keyword / missing family は declaration ごと drop
        // されるため、block 出口には何も残らない。
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
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding / border important 拡張と同 pattern)。
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
        // の end-to-end 側 check: `parse_overflow_shorthand` は 2 value 消費、3rd
        // 残り token は expect_exhausted で declaration 全体を drop する
        // (0 decl、margin 5-value sibling と同 pattern)。
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
        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl
        // のまま)。shorthand-only expansion の scope を check する negative test
        // (margin / padding / border sibling と同 pattern)。
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
    // `parse_declaration_block` は shorthand `text-decoration` を 3 longhand
    // (`TextDecorationLine` / `TextDecorationStyle` / `TextDecorationColor`)
    // に展開する。margin / padding / border / overflow precedent と同じ
    // parse-time expansion model。

    #[test]
    fn text_decoration_shorthand_expands_into_four_longhand_declarations() {
        use crate::property::{
            TextDecorationColor, TextDecorationLine, TextDecorationStyle, TextDecorationThickness,
        };

        // `text-decoration: underline` → 4 longhand、省略成分
        // (thickness/style/color) は spec initial で埋まる (property.rs
        // `parse_text_decoration_shorthand` の "Initial value fill" 節)。
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
        // spec CSS Cascading L4 §3: shorthand `!important` は全 longhand に copy
        // される (margin / padding / border / overflow important 拡張と同
        // pattern)。
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
        // の end-to-end 側 check: 2nd style keyword は leftover token として
        // expect_exhausted に検知され、declaration 全体が drop される (0 decl)。
        let decls = parse_block("text-decoration: solid wavy;");
        assert!(
            decls.is_empty(),
            "2 style components must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn text_decoration_longhand_declarations_not_expanded() {
        use crate::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

        // longhand は expand_shorthand_into の match arm を no-op で通過 (1 decl
        // のまま)。shorthand-only expansion の scope を check する negative test
        // (margin / padding / border / overflow sibling と同 pattern)。
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
        // `crate::cascade::tests::text_decoration_shorthand_resets_earlier_longhand_declarations` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — rustdoc-blind, confirmed via わざと壊して確かめる
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
