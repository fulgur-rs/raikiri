//! raikiri-style — CSS engine (cssparser + selectors + cascade + GCPM static side).
//!
//! # Implementation status
//!
//! - `Atom` / `RaikiriSelectorImpl` / `parse_selector_list` — initial seed
//!   implementation
//! - [`property`] / [`rule`] / [`ruletree`] / [`computed`] / [`mod@cascade`] —
//!   cascade minimum (type + universal selector、color / font-family / font-size /
//!   font-weight、specificity + !important + source order + inheritance)
//!
//! GCPM static side、@supports、class/id/attribute selector、基本的な
//! combinator matching、L4 の `:not()` / `:is()` / `:where()` / `:has()` は
//! 実装済み。`@media` は `MediaContext` による `all` / `print` / `screen` の
//! 条件評価に対応する。
//!
//! `precomputed-hash` is encapsulated as a direct dep of this crate only. It
//! is intentionally NOT promoted to `[workspace.dependencies]` — see the
//! comment on `Cargo.toml`.

// This crate's doc comments link `crate::…` pointers to crate-internal targets
// (`apply_value`, `expand_shorthand_into`, `INITIAL_BORDER`, …), so the
// "links to private item" lint is allowed — same convention as `raikiri-dom`
// and `raikiri-vrt`. `rustdoc::broken_intra_doc_links` is untouched, so an
// unresolved or ambiguous path still warns (and hard-errors under the
// `-D warnings` this repo's doc commands pass). 規約は AGENTS.md の
// 「`crate::…` pointer は intra-doc link で書く」節。
#![allow(rustdoc::private_intra_doc_links)]
#![allow(missing_docs)] // seed phase; docs come later

pub mod error;
pub use error::CascadeError;

pub mod consumer;
pub use consumer::{ConsumerPropertyGrammar, ConsumerPropertyRegistration};

pub mod style_dom;
pub use style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

pub mod property;
pub use property::{
    BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, BackgroundSize, BasicShape,
    BorderRadius, BoxShadowItem, CircleShape, ClipPath, ColumnCountValue, ColumnWidthValue,
    ColumnsShorthand, CssColor, CssPosition, CssPositionOffset, DisplayValue, EllipseShape,
    FillRule, GeometryBox, InsetBorderRadius, InsetShape, Length, LengthOrAuto, LengthOrNormal,
    ListStylePosition, ListStyleType, ObjectFit, Outline, OutlineColor, OutlineStyle, PathShape,
    PolygonShape, PropertyKey, PropertyValue, ShapeRadius, Sides, TextShadowColor, TextShadowItem,
    VisualBox,
};

pub mod rule;
pub use rule::{Declaration, StyleRule};

pub mod counter_style;
pub use counter_style::{
    CounterRange, CounterStyleRegistry, CounterStyleRule, CounterStyleSystem, CounterSymbol,
    NegativeDescriptor, PadDescriptor, RangeEntry, RangeLimit, parse_counter_style_rules,
    resolve_custom_counter,
};

pub mod font_face;
pub use font_face::{
    FontFaceDisplay, FontFaceRegistry, FontFaceRule, FontFaceSource, FontFaceStretch,
    FontFaceStyle, FontFaceWeight, parse_font_face_rules,
};

pub mod media;
pub use media::{MediaContext, MediaType};

pub mod page;
pub use page::{
    PageBleed, PageBleedDeclaration, PageCascadeResult, PageContextQuery, PageInheritance,
    PageMarginBoxCascadeResult, PageMarginBoxRule, PageMarginBoxSlot, PageMarks,
    PageMarksDeclaration, PageOrientation, PagePseudo, PageRule, PageSelector, PageSelectorEntry,
    PageSize, PageSizeDeclaration, PageSizeKeyword, cascade_page,
};

pub mod ruletree;
pub use ruletree::{
    AtRuleBody, AtRuleRecord, CssRule, CssRuleKind, Origin, QualifiedRuleRecord, RuleNode,
    RuleTree, build_rule_tree, walk_style_elements,
};

pub mod computed;
pub use computed::{ChFontKey, ChLengthProvenance, ComputedValues};

pub mod resolve;
pub use resolve::{
    ComputedBackgroundSize, ComputedBorder, ComputedBorderRadius, ComputedBorderSpacing,
    ComputedBoxShadowItem, ComputedColumnWidth, ComputedCssPosition, ComputedCssPositionOffset,
    ComputedFlexBasis, ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackRepeat, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedLengthPercentageWithCh, ComputedLengthWithCh, ComputedLineHeight, ComputedOutline,
    ComputedTabSize, ComputedTextDecorationInset, ComputedTextShadow, ComputedTextUnderlineOffset,
    ComputedTransformFunction, ResolveContext, lift_border_spacing, lift_font_size,
    lift_length_or_normal, lift_length_percentage, lift_line_height, lift_tab_size,
    lift_text_shadow_item, resolve_background_size, resolve_border, resolve_border_radius,
    resolve_border_spacing, resolve_box_shadow_item, resolve_column_width, resolve_css_position,
    resolve_flex_basis, resolve_font_size, resolve_grid_auto_track_list,
    resolve_grid_inflexible_breadth, resolve_grid_template_tracks, resolve_grid_track_breadth,
    resolve_grid_track_list, resolve_grid_track_size, resolve_length_or_normal,
    resolve_length_or_normal_with_ch, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_length_percentage_with_ch, resolve_line_height,
    resolve_margin_length_or_auto, resolve_outline, resolve_tab_size,
    resolve_text_decoration_inset, resolve_text_shadow_item, resolve_transform_function,
    used_line_height_length,
};

pub mod specified;
pub use specified::SpecifiedValues;

pub mod cascade;
pub use cascade::{
    CascadeResult, cascade, cascade_with_media_context, cascade_with_media_context_for_page,
};

#[cfg(test)]
pub(crate) mod test_dom;

use std::fmt;

use cssparser::{CowRcStr, Parser as CssParser, ParserInput, SourceLocation, ToCss};
use precomputed_hash::PrecomputedHash;
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, Parser as SelectorsParser, PseudoElement, SelectorImpl,
    SelectorList, SelectorParseErrorKind,
};
use smol_str::SmolStr;

// ---------------------------------------------------------------------------
// Atom — SmolStr newtype with a stable u32 precomputed hash.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Atom(pub SmolStr);

impl<'a> From<&'a str> for Atom {
    fn from(s: &'a str) -> Self {
        Atom(SmolStr::new(s))
    }
}

impl From<String> for Atom {
    fn from(s: String) -> Self {
        Atom(SmolStr::new(s))
    }
}

impl fmt::Display for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::borrow::Borrow<str> for Atom {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

impl ToCss for Atom {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl PrecomputedHash for Atom {
    fn precomputed_hash(&self) -> u32 {
        use core::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        self.0.as_str().hash(&mut h);
        h.finish() as u32
    }
}

// ---------------------------------------------------------------------------
// AttrValue — attribute-value type used in selector parsing.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttrValue(pub SmolStr);

impl<'a> From<&'a str> for AttrValue {
    fn from(s: &'a str) -> Self {
        AttrValue(SmolStr::new(s))
    }
}

impl ToCss for AttrValue {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        use fmt::Write as _;
        dest.write_char('"')?;
        write!(cssparser::CssStringWriter::new(dest), "{}", &self.0)?;
        dest.write_char('"')
    }
}

// ---------------------------------------------------------------------------
// Pseudo-class / pseudo-element enums (minimal initial set).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoClass {
    Hover,
    Active,
    /// `:lang(range1, range2, ...)` — CSS Selectors L4 §7.2
    /// <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>. Each `String` is
    /// one comma-separated language range **as written in the selector**
    /// (already unescaped by `cssparser`'s ident/string tokenizing, but
    /// otherwise unvalidated / uncanonicalized — see
    /// [`crate::cascade::language_range_matches`] doc for the scope
    /// simplifications this implies).
    Lang(Vec<String>),
    /// `:dir(ltr)` / `:dir(rtl)` — CSS Selectors L4 §7.1
    /// <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>. See [`Direction`]
    /// doc for how this argument relates to a `dir="auto"` matched element's
    /// own (resolved, not literal) directionality.
    Dir(Direction),
}

/// Explicit directionality value accepted by [`PseudoClass::Dir`] (CSS
/// Selectors L4 §7.1 <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>,
/// CSSWG bikeshed source `selectors-4/Overview.bs` §"The Directionality
/// Pseudo-class: :dir()":
///
/// > The argument to `:dir()` must be a single identifier, otherwise the
/// > selector is invalid. \[...\] Values other than `ltr` and `rtl` are not
/// > invalid, but do not match anything.
///
/// This crate's parser (`RaikiriSelectorParser::parse_non_ts_functional_pseudo_class`)
/// diverges from that last sentence for simplicity: an argument that is
/// syntactically a single identifier but neither `ltr` nor `rtl` (e.g.
/// `:dir(foo)`) is rejected as a parse error rather than accepted as a
/// permanently-non-matching valid selector. Blast radius: **the whole
/// selector list is dropped**, not just the one compound that used
/// `:dir(foo)` — `SelectorList::parse` (used by both this crate's
/// `parse_selector_list` and `ruletree.rs`'s `StyleRuleParser::parse_prelude`)
/// is the `selectors` crate's *non-forgiving* entry point
/// (`RaikiriSelectorParser` does not override `allow_forgiving_selectors`,
/// so `SelectorList::parse_forgiving`'s per-selector recovery never
/// applies), verified directly against `selectors` v0.39.0's own public
/// behavior via `parse_dir_rejects_non_ltr_rtl_identifier_drops_whole_comma_separated_list`
/// below — so `p, :dir(foo) { color: red }` drops the `p` selector too, not
/// just `:dir(foo)`. Low real-world impact (no known content in this
/// repo's corpus uses a bogus `:dir()` argument) and keeps `Direction` a
/// plain 2-variant enum instead of needing a third "other identifier,
/// never matches" variant.
///
/// # `:dir()`'s argument vs. the matched element's directionality
///
/// This enum is used two ways: as `:dir()`'s own selector *argument* (the
/// CSSWG bikeshed quote above already establishes that argument is only
/// ever `ltr`/`rtl`, never a literal `auto` — there is no `:dir(auto)`
/// syntax), and as the *matched element's* resolved directionality
/// ([`crate::cascade::resolve_directionality`]'s return type) that argument
/// is compared against. An element with `dir="auto"` still resolves to a
/// concrete `Ltr`/`Rtl` value for that comparison via the HTML
/// directionality algorithm's own `Auto`-state arm
/// (<https://html.spec.whatwg.org/multipage/dom.html#the-directionality>) —
/// scanning the element's contained text for the first character with a
/// strong bidirectional type — see
/// [`crate::cascade::resolve_directionality`]'s doc for that resolution and
/// [`mod@crate::cascade`]'s `strong_bidi_type` doc for the specific scope
/// cut in this crate's character classification (every Unicode block
/// reserved by default for right-to-left use, via a hand-transcribed range
/// table, not a full per-code-point Bidi_Class table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right (`dir="ltr"`, HTML LS "LTR" state).
    Ltr,
    /// Right-to-left (`dir="rtl"`, HTML LS "RTL" state).
    Rtl,
}

impl ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        match self {
            PseudoClass::Hover => dest.write_str(":hover"),
            PseudoClass::Active => dest.write_str(":active"),
            PseudoClass::Lang(ranges) => {
                dest.write_str(":lang(")?;
                for (i, range) in ranges.iter().enumerate() {
                    if i > 0 {
                        dest.write_str(", ")?;
                    }
                    cssparser::serialize_identifier(range, dest)?;
                }
                dest.write_str(")")
            }
            PseudoClass::Dir(dir) => {
                dest.write_str(":dir(")?;
                dest.write_str(match dir {
                    Direction::Ltr => "ltr",
                    Direction::Rtl => "rtl",
                })?;
                dest.write_str(")")
            }
        }
    }
}

impl NonTSPseudoClass for PseudoClass {
    type Impl = RaikiriSelectorImpl;

    fn is_active_or_hover(&self) -> bool {
        matches!(self, PseudoClass::Hover | PseudoClass::Active)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(self, PseudoClass::Hover | PseudoClass::Active)
    }
}

/// Tree-abiding generated pseudo-elements used by the current layout.
///
/// `::before` / `::after` come from CSS Pseudo-Elements Module Level 4 §4.1;
/// `::marker` comes from CSS Lists 3 §3. Generated list markers are resolved by
/// the downstream layout/paint layer, while author `::marker` declarations are
/// still cascaded here.
///
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElem {
    /// `::before` (also accepted as legacy `:before`).
    Before,
    /// `::after` (also accepted as legacy `:after`).
    After,
    /// `::marker` (CSS Lists 3 §3.7).
    Marker,
}

impl ToCss for PseudoElem {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            PseudoElem::Before => "::before",
            PseudoElem::After => "::after",
            PseudoElem::Marker => "::marker",
        })
    }
}

impl PseudoElement for PseudoElem {
    type Impl = RaikiriSelectorImpl;

    // Other `PseudoElement` methods keep their default false values, so
    // unsupported pseudo-element states remain rejected.
}

// ---------------------------------------------------------------------------
// SelectorImpl.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaikiriSelectorImpl;

impl SelectorImpl for RaikiriSelectorImpl {
    type ExtraMatchingData<'a> = std::marker::PhantomData<&'a ()>;
    type AttrValue = AttrValue;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedNamespaceUrl = Atom;
    type BorrowedLocalName = Atom;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElem;
}

// ---------------------------------------------------------------------------
// Parser adapter — feeds a `cssparser::Parser` into `SelectorList::parse`.
// ---------------------------------------------------------------------------

pub struct RaikiriSelectorParser;

impl<'i> SelectorsParser<'i> for RaikiriSelectorParser {
    type Impl = RaikiriSelectorImpl;
    type Error = SelectorParseErrorKind<'i>;

    /// Enable Selectors Level 4 `of <selector-list>` parsing for
    /// `:nth-child()` and `:nth-last-child()`.
    fn parse_nth_child_of(&self) -> bool {
        true
    }

    /// Enable the Selectors Level 4 logical pseudo-classes `:is()` and
    /// `:where()`. The selector matcher handles their selector-list
    /// arguments recursively.
    fn parse_is_and_where(&self) -> bool {
        true
    }

    /// Enable the Selectors Level 4 relational pseudo-class `:has()`. The
    /// cascade matcher evaluates its relative selector arguments against the
    /// current element's flat-tree descendants and siblings.
    fn parse_has(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<PseudoClass, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("hover") {
            Ok(PseudoClass::Hover)
        } else if name.eq_ignore_ascii_case("active") {
            Ok(PseudoClass::Active)
        } else {
            Err(
                location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                    name,
                )),
            )
        }
    }

    /// `:lang(range, ...)` / `:dir(ltr|rtl)` — CSS Selectors L4 §7.2 / §7.1
    /// (see [`PseudoClass::Lang`] / [`PseudoClass::Dir`] doc for spec
    /// anchors). Grammar (bikeshed source, quoted verbatim on
    /// [`Direction`]'s doc / [`crate::cascade::language_range_matches`]'s
    /// doc): `:lang()` "accepts a comma-separated list of one or more
    /// language ranges \[...\] a valid CSS `<ident>` or `<string>`"; `:dir()`'s
    /// "argument \[...\] must be a single identifier, otherwise the selector
    /// is invalid."
    // `_after_part` is unused: the `selectors` crate only sets it after a
    // successfully-parsed `::part()` (gated by `Parser::parse_part()`, which
    // this impl leaves at its `false` default, so `::part()` itself never
    // parses) or after a pseudo-element whose `parses_as_element_backed()`
    // returns `true` (`selectors-0.39.0/parser.rs`'s `AFTER_PART_LIKE`
    // handling). `crate::PseudoElem` (`::before`/`::after`) leaves that
    // method at its `false` default, so it never sets this flag either —
    // and since it also leaves `is_before_or_after()` at `false`, no
    // pseudo-class can even be attempted after `::before`/`::after` in the
    // first place (rejected at the selector-parsing state check before
    // reaching this function at all). Both preconditions stay unreachable,
    // so `after_part` is always `false` when this function runs — unlike
    // the `selectors` crate's own reference test impl (its internal
    // `"lang" if !after_part => ...` arm, `selectors-0.39.0/parser.rs`),
    // this crate has no live case to guard against.
    fn parse_non_ts_functional_pseudo_class<'t>(
        &self,
        name: CowRcStr<'i>,
        parser: &mut CssParser<'i, 't>,
        _after_part: bool,
    ) -> Result<PseudoClass, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("lang") {
            let ranges = parser.parse_comma_separated(|input| {
                input
                    .expect_ident_or_string()
                    .map(|s| s.as_ref().to_owned())
                    .map_err(Into::into)
            })?;
            Ok(PseudoClass::Lang(ranges))
        } else if name.eq_ignore_ascii_case("dir") {
            let location = parser.current_source_location();
            let ident = parser.expect_ident()?.clone();
            if ident.eq_ignore_ascii_case("ltr") {
                Ok(PseudoClass::Dir(Direction::Ltr))
            } else if ident.eq_ignore_ascii_case("rtl") {
                Ok(PseudoClass::Dir(Direction::Rtl))
            } else {
                // Spec allows this (valid selector, matches nothing) — this
                // crate rejects it as a parse error instead, see
                // `Direction` doc's "single identifier ... otherwise
                // invalid" note for the accepted-scope rationale.
                Err(location.new_custom_error(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ))
            }
        } else {
            Err(
                parser.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                    name,
                )),
            )
        }
    }

    /// `::before` / `::after` / `::marker` (CSS Pseudo-Elements Module Level 4
    /// §4.1 and CSS Lists 3 §3.7, see [`PseudoElem`] doc) — everything else
    /// (`::details-content`, `::part()`, `::slotted()`, any unknown name)
    /// stays a parse error, same fail-closed posture as
    /// [`Self::parse_non_ts_pseudo_class`] above.
    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<PseudoElem, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("before") {
            Ok(PseudoElem::Before)
        } else if name.eq_ignore_ascii_case("after") {
            Ok(PseudoElem::After)
        } else if name.eq_ignore_ascii_case("marker") {
            Ok(PseudoElem::Marker)
        } else {
            Err(
                location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                    name,
                )),
            )
        }
    }
}

/// Parse a selector list from CSS source using raikiri's SelectorImpl.
///
/// Seed helper — returns a `SelectorList<RaikiriSelectorImpl>` and stringifies
/// errors for the feasibility prototype. A future pass will replace the `Result<_, String>` shape
/// with a proper structured error type.
pub fn parse_selector_list(input: &str) -> Result<SelectorList<RaikiriSelectorImpl>, String> {
    let mut parser_input = ParserInput::new(input);
    let mut css_parser = CssParser::new(&mut parser_input);
    SelectorList::parse(&RaikiriSelectorParser, &mut css_parser, ParseRelative::No)
        .map_err(|e| format!("selector parse error: {e:?}"))
}

// ---------------------------------------------------------------------------
// Tests — smoke coverage for the seed. Real cascade / matching tests come later.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_precomputed_hash_is_deterministic() {
        let a1 = Atom::from("btn");
        let a2 = Atom::from("btn");
        let b = Atom::from("card");
        assert_eq!(a1.precomputed_hash(), a2.precomputed_hash());
        assert_ne!(a1.precomputed_hash(), b.precomputed_hash());
    }

    #[test]
    fn atom_tocss_roundtrip() {
        let a = Atom::from("btn");
        let mut out = String::new();
        a.to_css(&mut out).expect("serialize");
        assert_eq!(out, "btn");
    }

    #[test]
    fn parse_dot_btn_hover_roundtrip() {
        let list = parse_selector_list(".btn:hover").expect("parse .btn:hover");
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize selector list");
        assert_eq!(out, ".btn:hover");
    }

    #[test]
    fn parse_multi_selector_list() {
        let list = parse_selector_list("a:hover, .btn:active").expect("parse list");
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize");
        assert!(out.contains(":hover"));
        assert!(out.contains(":active"));
        assert!(out.contains(','));
    }

    #[test]
    fn parse_logical_and_relational_pseudo_classes() {
        use selectors::parser::Component;

        for (source, expected) in [
            ("div:is(.featured, .selected)", "is"),
            ("div:where(.featured, .selected)", "where"),
            ("div:has(> .featured)", "has"),
        ] {
            let list = parse_selector_list(source)
                .unwrap_or_else(|error| panic!("parse {source:?}: {error}"));
            let mut components = list.slice()[0].iter_raw_match_order();
            let found = components.any(|component| {
                matches!(
                    (expected, component),
                    ("is", Component::Is(_))
                        | ("where", Component::Where(_))
                        | ("has", Component::Has(_))
                )
            });
            assert!(found, "{source:?} must contain the {expected} component");
        }

        for source in [
            ":has(a)",
            ":has(#a)",
            ":has(.a)",
            ":has([a])",
            ":has([a=\"b\"])",
            ":has([a|=\"b\"])",
            ":has(:hover)",
            "*:has(.a)",
            ".a:has(.b)",
            ".a:has(> .b)",
            ".a:has(~ .b)",
            ".a:has(+ .b)",
            ".a:has(.b) .c",
            ".a .b:has(.c)",
            ".a .b:has(.c .d)",
            ".a .b:has(.c .d) .e",
            ".a:has(.b:is(.c .d))",
            ".a:is(.b:has(.c) .d)",
            ".a:not(:has(.b))",
            ".a:has(:not(.b))",
            ".a:has(.b):has(.c)",
            "*|*:has(*)",
            ":has(*|*)",
        ] {
            assert!(parse_selector_list(source).is_ok());
        }

        for source in [
            ":has",
            ".a:has",
            ".a:has b",
            ":has()",
            ":has(123)",
            ":has(.a, 123)",
            ".a:has(.b:has(.c))",
        ] {
            assert!(parse_selector_list(source).is_err());
        }

        for source in [
            ":has(:is(:has(*)))",
            ":has(:where(:has(*)))",
            ":has(:is(.a, 123))",
        ] {
            assert!(parse_selector_list(source).is_ok());
        }
    }

    #[test]
    fn parse_wpt_logical_selector_forms_roundtrip() {
        for pseudo in ["is", "where"] {
            for (source, expected) in [
                (
                    format!(":{pseudo}(ul,ol,.list) > [hidden]"),
                    format!(":{pseudo}(ul, ol, .list) > [hidden]"),
                ),
                (
                    format!(":{pseudo}(:hover,:focus)"),
                    format!(":{pseudo}(:hover, :focus)"),
                ),
                (
                    format!("a:{pseudo}(:not(:hover))"),
                    format!("a:{pseudo}(:not(:hover))"),
                ),
                (format!(":{pseudo}(#a)"), format!(":{pseudo}(#a)")),
                (
                    format!(".a.b ~ :{pseudo}(.c.d ~ .e.f)"),
                    format!(".a.b ~ :{pseudo}(.c.d ~ .e.f)"),
                ),
                (
                    format!(".a.b ~ .c.d:{pseudo}(span.e + .f, .g.h > .i.j .k)"),
                    format!(".a.b ~ .c.d:{pseudo}(span.e + .f, .g.h > .i.j .k)"),
                ),
            ] {
                let list = parse_selector_list(&source)
                    .unwrap_or_else(|error| panic!("parse {source:?}: {error}"));
                let mut serialized = String::new();
                list.to_css(&mut serialized)
                    .expect("serialize selector list");
                assert_eq!(serialized, expected);
            }
        }
    }

    #[test]
    fn parse_lang_single_range_roundtrip() {
        let list = parse_selector_list(":lang(ja)").expect("parse :lang(ja)");
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize");
        assert_eq!(out, ":lang(ja)");
    }

    #[test]
    fn parse_lang_multi_range_comma_separated() {
        let list = parse_selector_list(":lang(en, fr-CA, \"*-Hant\")").expect("parse :lang(...)");
        let selector = &list.slice()[0];
        let mut iter = selector.iter();
        let component = iter.next().expect("one component");
        match component {
            selectors::parser::Component::NonTSPseudoClass(PseudoClass::Lang(ranges)) => {
                assert_eq!(ranges, &["en", "fr-CA", "*-Hant"]);
            }
            other => panic!("expected PseudoClass::Lang, got {other:?}"),
        }
    }

    #[test]
    fn parse_dir_ltr_and_rtl_roundtrip() {
        let list = parse_selector_list(":dir(ltr)").expect("parse :dir(ltr)");
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize");
        assert_eq!(out, ":dir(ltr)");

        let list = parse_selector_list(":dir(rtl)").expect("parse :dir(rtl)");
        let mut out = String::new();
        list.to_css(&mut out).expect("serialize");
        assert_eq!(out, ":dir(rtl)");
    }

    #[test]
    fn parse_dir_rejects_non_ltr_rtl_identifier() {
        // `Direction` doc's documented scope cut: unlike the real spec
        // (valid selector, matches nothing), this parser treats any
        // identifier other than ltr/rtl as a parse error.
        assert!(parse_selector_list(":dir(sideways)").is_err());
    }

    #[test]
    fn parse_dir_rejects_non_ltr_rtl_identifier_drops_whole_comma_separated_list() {
        // `Direction` doc's "blast radius" note: `SelectorList::parse` is
        // non-forgiving, so one bad selector in a comma-separated list
        // fails the *whole* list — `p` here is otherwise perfectly valid on
        // its own.
        assert!(parse_selector_list("p, :dir(sideways)").is_err());
    }

    #[test]
    fn parse_nth_child_of_selector_list_uses_nth_of_component() {
        use selectors::parser::{Component, NthType};

        for (source, expected_type) in [
            (
                "p:nth-child(2 of .featured, [data-kind=\"selected\"])",
                NthType::Child,
            ),
            (
                "p:nth-last-child(2 of .featured, [data-kind=\"selected\"])",
                NthType::LastChild,
            ),
        ] {
            let list = parse_selector_list(source).expect("parse selector-list argument");
            let selector = &list.slice()[0];
            let component = selector
                .iter()
                .find(|component| matches!(component, Component::NthOf(_)))
                .expect("selector must contain Component::NthOf");
            match component {
                Component::NthOf(data) => {
                    assert!(data.nth_data().ty == expected_type);
                    assert_eq!(data.nth_data().an_plus_b.0, 0);
                    assert_eq!(data.nth_data().an_plus_b.1, 2);
                    assert_eq!(data.selectors().len(), 2);
                }
                // cov:ignore: `find` above only yields `Component::NthOf`;
                // this arm is unreachable by construction.
                _ => unreachable!("find above guarantees Component::NthOf"),
            }

            let mut serialized = String::new();
            list.to_css(&mut serialized)
                .expect("serialize selector list");
            assert_eq!(serialized, source);
        }
    }

    #[test]
    fn parse_nth_child_of_rejects_invalid_selector_list_forms() {
        assert!(parse_selector_list("p:nth-child(2 of .featured,)").is_err());
        assert!(parse_selector_list("p:nth-of-type(2 of .featured)").is_err());
    }

    // ---- tree-abiding pseudo-element selector parsing ----
    //
    // CSS Pseudo-Elements Module Level 4 §4.1
    // <https://drafts.csswg.org/css-pseudo-4/#generated-content> and CSS Lists
    // 3 §3.7, Selectors Level 4 (pseudo-element grammar). `cascade.rs`'s test module covers
    // matching/cascade behavior once parsed; these tests cover parsing
    // (accept/reject shape) only.

    #[test]
    fn parse_before_and_after_pseudo_element_roundtrip() {
        for (src, expected) in [
            (".foo::before", PseudoElem::Before),
            ("p::after", PseudoElem::After),
            ("li::marker", PseudoElem::Marker),
        ] {
            let list = parse_selector_list(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
            let selector = &list.slice()[0];
            assert_eq!(selector.pseudo_element(), Some(&expected));
            let mut out = String::new();
            list.to_css(&mut out).expect("serialize selector list");
            assert_eq!(out, src);
        }
    }

    #[test]
    fn parse_bare_pseudo_element_implies_universal_originating_selector() {
        // `::before` alone parses like `*::before` — no explicit type/class
        // required on the originating-element side.
        let list = parse_selector_list("::before").expect("parse ::before");
        let selector = &list.slice()[0];
        assert_eq!(selector.pseudo_element(), Some(&PseudoElem::Before));
    }

    #[test]
    fn parse_pseudo_element_rejects_unknown_name() {
        // `::marker` is the one additional generated-content pseudo-element
        // supported by the list-item pipeline. Other unsupported names remain
        // fail-closed (same posture `parse_non_ts_pseudo_class` already has for
        // unrecognized pseudo-classes).
        assert!(parse_selector_list("::marker").is_ok());
        assert!(parse_selector_list("::details-content").is_err());
        assert!(parse_selector_list("::bogus").is_err());
    }

    #[test]
    fn parse_pseudo_element_must_be_selector_tail() {
        // A pseudo-element must be the rightmost component — nothing may
        // follow it in the same selector.
        assert!(parse_selector_list("a::before b").is_err());
    }

    #[test]
    fn parse_pseudo_element_rejects_chaining_after_before_or_after() {
        // Fail-closed posture (see `PseudoElem`/`RaikiriSelectorParser` doc):
        // no pseudo-class, and no other pseudo-element, may follow
        // `::before`/`::after` in this crate — `PseudoElem` overrides none
        // of `is_before_or_after`/`accepts_state_pseudo_classes`/
        // `parses_as_element_backed`'s `false` defaults, so the `selectors`
        // crate's own parser state machine rejects all three forms below.
        assert!(parse_selector_list("::before::after").is_err());
        assert!(parse_selector_list("::before:hover").is_err());
        assert!(parse_selector_list(".foo::before.bar").is_err());
    }

    #[test]
    fn parse_pseudo_element_rejected_inside_nth_child_of_selector_list() {
        // CSS Selectors Level 4 forbids a pseudo-element inside
        // `:nth-child(An+B of S)`'s `S` — the `selectors` crate itself
        // enforces this at parse time (not something this crate's own
        // `is_supported_selector` needs to reject after the fact, though it
        // does so too as defense-in-depth — see `ruletree.rs`
        // `is_supported_selector`'s doc).
        assert!(parse_selector_list("p:nth-child(2 of .x::before)").is_err());
    }

    #[test]
    fn parse_legacy_single_colon_before_and_after_syntax() {
        // CSS Pseudo-Elements Module Level 4 §8 "Compatibility Syntax"
        // <https://drafts.csswg.org/css-pseudo-4/#css2-compat>, verbatim:
        // "For compatibility with existing style sheets written against CSS
        // Level 2 `[...]`, user agents must also accept the previous
        // one-colon notation (:before, :after, :first-letter, :first-line)
        // for the ::before, ::after, ::first-letter, and ::first-line
        // pseudo-elements." The `selectors` crate's own
        // `is_css2_pseudo_element` already special-cases exactly these two
        // names (plus `first-line`/`first-letter`, which this crate's
        // `parse_pseudo_element` still rejects by name) into the same
        // pseudo-element parse path the double-colon syntax uses — so this
        // MUST-level requirement already works without any extra code in
        // this crate, but was previously untested.
        //
        // Deliberately does NOT assert a source round-trip via `to_css`
        // (unlike `parse_before_and_after_pseudo_element_roundtrip` above):
        // `:before`/`:after` and `::before`/`::after` denote the same
        // pseudo-element (Selectors Level 4 §3.10 "Syntax"
        // <https://www.w3.org/TR/selectors-4/#pseudo-element-syntax>, same
        // one-colon compatibility rule), and this crate's `PseudoElem` has
        // no field to remember which spelling the author used, so
        // `ToCss for PseudoElem` always serializes the double-colon form
        // regardless of which one was parsed. A naive `assert_eq!(out,
        // src)` against `:before` input would therefore fail spuriously;
        // the correct assertion is on the parsed `PseudoElem` value only.
        for (src, expected) in [
            (":before", PseudoElem::Before),
            (":after", PseudoElem::After),
        ] {
            let list = parse_selector_list(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
            let selector = &list.slice()[0];
            assert_eq!(selector.pseudo_element(), Some(&expected));
        }
    }
}
