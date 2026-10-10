//! raikiri-style — CSS engine (cssparser + selectors + cascade + GCPM static side).
//!
//! # Implementation status
//!
//! - `Atom` / `RaikiriSelectorImpl` / `parse_selector_list` — initial seed
//!   implementation
//! - [`property`] / [`rule`] / [`ruletree`] / [`computed`] / [`mod@cascade`] —
//!   minimal cascade (type + universal selectors, color / font-family / font-size /
//!   font-weight, specificity + !important + source order + inheritance)
//!
//! GCPM static support, @supports, class/id/attribute selectors, basic
//! combinator matching, and L4 `:not()` / `:is()` / `:where()` / `:has()` are
//! implemented. `@media` evaluates media types and `width` / `height`
//! conditions through `MediaContext`.
//!
//! `precomputed-hash` is encapsulated as a direct dep of this crate only. It
//! is intentionally NOT promoted to `[workspace.dependencies]` — see the
//! comment on `Cargo.toml`.

// This crate's doc comments link `crate::…` pointers to crate-internal targets
// (`apply_value`, `expand_shorthand_into`, `INITIAL_BORDER`, …), so the
// "links to private item" lint is allowed — same convention as `raikiri-dom`
// and `raikiri-vrt`. `rustdoc::broken_intra_doc_links` is untouched, so an
// unresolved or ambiguous path still warns (and hard-errors under the
// `-D warnings` this repo's doc commands pass). See the AGENTS.md section
// "Write `crate::…` pointers as intra-doc links" for the convention.
#![allow(rustdoc::private_intra_doc_links)]
#![allow(missing_docs)] // seed phase; docs come later

pub mod error;
pub use error::{CascadeError, CascadeLimitKind};

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
    FillRule, FontFamilyKind, FontFamilyName, GeometryBox, InsetBorderRadius, InsetShape, Length,
    LengthOrAuto, LengthOrNormal, LetterSpacingValue, ListStylePosition, ListStyleType, ObjectFit,
    Outline, OutlineColor, OutlineStyle, PathShape, PolygonShape, PropertyKey, PropertyValue,
    ShapeRadius, Sides, TextDecorationSkipInk, TextDecorationSkipSpaces, TextOrientation,
    TextShadowColor, TextShadowItem, TextShadowLength, TextSpacingTrim, TextUnderlineOffset,
    ViewportSize, ViewportUnit, VisualBox, WordSpaceTransform, WordSpacingValue,
};

mod layer;

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

mod condition;
mod grammar;
mod supports;

pub mod media;
pub use media::{MediaContext, MediaType};

pub mod page;
pub use page::{
    PageBleed, PageBleedDeclaration, PageCascadeResult, PageContextQuery, PageInheritance,
    PageMarginBoxCascadeResult, PageMarginBoxRule, PageMarginBoxSlot, PageMarks,
    PageMarksDeclaration, PageOrientation, PagePseudo, PageRule, PageSelector, PageSelectorEntry,
    PageSize, PageSizeDeclaration, PageSizeKeyword, cascade_page, cascade_page_with_media_context,
};

pub mod ruletree;
pub use ruletree::{
    AtRuleBody, AtRuleRecord, CssRule, CssRuleKind, Origin, QualifiedRuleRecord, RuleNode,
    RuleTree, RuleTreeLimits, build_rule_tree, walk_style_elements,
};

pub mod computed;
pub use computed::{
    ChFontKey, ChLengthProvenance, ComputedProperty, ComputedValues, VerticalLogicalSize,
};
pub mod computed_api;

pub mod resolve;
pub use resolve::{
    ComputedBackgroundSize, ComputedBorder, ComputedBorderRadius, ComputedBorderSpacing,
    ComputedBoxShadowItem, ComputedColumnWidth, ComputedCssPosition, ComputedCssPositionOffset,
    ComputedFlexBasis, ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackRepeat, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedLengthPercentageWithCh, ComputedLengthWithCh, ComputedLetterSpacing,
    ComputedLineHeight, ComputedOutline, ComputedTabSize, ComputedTextDecorationInset,
    ComputedTextDecorationThickness, ComputedTextIndent, ComputedTextShadow,
    ComputedTextUnderlineOffset, ComputedTransformFunction, ComputedWordSpacing, ResolveContext,
    lift_border_spacing, lift_font_size, lift_length_or_normal, lift_length_percentage,
    lift_letter_spacing, lift_line_height, lift_tab_size, lift_text_indent, lift_text_shadow_item,
    lift_word_spacing, resolve_background_size, resolve_border, resolve_border_radius,
    resolve_border_spacing, resolve_box_shadow_item, resolve_column_width, resolve_css_position,
    resolve_flex_basis, resolve_font_size, resolve_grid_auto_track_list,
    resolve_grid_inflexible_breadth, resolve_grid_template_tracks, resolve_grid_track_breadth,
    resolve_grid_track_list, resolve_grid_track_size, resolve_length_or_normal,
    resolve_length_or_normal_with_ch, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_length_percentage_with_ch, resolve_letter_spacing,
    resolve_letter_spacing_with_ch, resolve_line_height, resolve_margin_length_or_auto,
    resolve_outline, resolve_tab_size, resolve_text_decoration_inset,
    resolve_text_decoration_thickness, resolve_text_indent_calc, resolve_text_shadow_item,
    resolve_transform_function, resolve_word_spacing, resolve_word_spacing_with_ch,
    used_line_height_length,
};

pub mod specified;
pub use specified::SpecifiedValues;

pub mod cascade;
pub use cascade::{
    CascadeLimits, CascadeOptions, CascadeResult, FirstLineCascade, FirstLineStyles,
    SelectorMatcher, SelectorQuery, cascade, cascade_with_first_line, cascade_with_media_context,
    cascade_with_media_context_for_page, cascade_with_options,
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
    /// An unvisited hyperlink; this renderer has no browsing history.
    Link,
    /// Any hyperlink, independently of browsing history.
    AnyLink,
    /// A valid history-dependent selector; always nonmatching in static rendering.
    Visited,
    /// `:lang(range1, range2, ...)` — CSS Selectors L4 §7.2
    /// <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>. Each `String` is
    /// one comma-separated language range **as written in the selector**
    /// (already unescaped by `cssparser`'s ident/string tokenizing, but
    /// otherwise unvalidated / uncanonicalized — see
    /// [`crate::cascade::lang::language_range_matches`] doc for the scope
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
            PseudoClass::Link => dest.write_str(":link"),
            PseudoClass::AnyLink => dest.write_str(":any-link"),
            PseudoClass::Visited => dest.write_str(":visited"),
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

/// Pseudo-elements resolved by this crate's cascade into a per-`(element,
/// pseudo)` [`ComputedValues`] entry (see `CascadeResult::pseudo`).
///
/// `::before` / `::after` / `::marker` are tree-abiding pseudo-elements (CSS
/// Pseudo-Elements Module Level 4 §4.1 `#treelike`; `::marker` also CSS Lists
/// 3 §3.7) — each is expected to yield a single generated box as if it were
/// an immediate child of its originating element. Generated list markers are
/// resolved by the downstream layout/paint layer, while author `::marker`
/// declarations are still cascaded here.
///
/// `::first-line` (CSS Pseudo-Elements Module Level 4 §2.1 `#first-line-pseudo`)
/// is not tree-abiding — it formats part of the originating element's own
/// content rather than generating a child box — but reuses the same
/// `(element, pseudo)` cascade path since it is likewise resolved by
/// selector match against one originating element. §2.1 restricts it to
/// block containers and §2.1.2 `#first-line-styling` restricts which
/// properties apply; this crate does not yet enforce either restriction and
/// exposes the full computed value, same as the tree-abiding set (see
/// `CascadeResult::pseudo` doc).
///
/// `::first-letter` retains its declarations for layout-time resolution over
/// the actual inline parent, including first-line inheritance. Layout supports
/// inline first-letter boxes; floating drop-cap boxes require a separate path.
/// The valid native `::backdrop` and `::file-selector-button` selectors never
/// match, because this renderer does not generate their specialized boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PseudoElem {
    /// `::before` (also accepted as legacy `:before`).
    Before,
    /// `::after` (also accepted as legacy `:after`).
    After,
    /// `::marker` (CSS Lists 3 §3.7).
    Marker,
    /// `::first-line` (also accepted as legacy `:first-line`).
    FirstLine,
    /// The first typographic letter unit of a block container.
    FirstLetter,
    /// Valid top-layer pseudo-element; no box is generated by this renderer.
    Backdrop,
    /// Valid native file-button pseudo-element; no box is generated here.
    FileSelectorButton,
}

impl ToCss for PseudoElem {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(match self {
            PseudoElem::Before => "::before",
            PseudoElem::After => "::after",
            PseudoElem::Marker => "::marker",
            PseudoElem::FirstLine => "::first-line",
            PseudoElem::FirstLetter => "::first-letter",
            PseudoElem::Backdrop => "::backdrop",
            PseudoElem::FileSelectorButton => "::file-selector-button",
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
        } else if name.eq_ignore_ascii_case("link") {
            Ok(PseudoClass::Link)
        } else if name.eq_ignore_ascii_case("any-link") {
            Ok(PseudoClass::AnyLink)
        } else if name.eq_ignore_ascii_case("visited") {
            Ok(PseudoClass::Visited)
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
    /// [`Direction`]'s doc / [`crate::cascade::lang::language_range_matches`]'s
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

    /// Parse the supported generated and typographic pseudo-elements, plus
    /// valid native pseudos whose boxes this renderer never generates.
    /// Unknown names remain invalid in ordinary selector lists.
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
        } else if name.eq_ignore_ascii_case("first-line") {
            Ok(PseudoElem::FirstLine)
        } else if name.eq_ignore_ascii_case("first-letter") {
            Ok(PseudoElem::FirstLetter)
        } else if name.eq_ignore_ascii_case("backdrop") {
            Ok(PseudoElem::Backdrop)
        } else if name.eq_ignore_ascii_case("file-selector-button") {
            Ok(PseudoElem::FileSelectorButton)
        } else {
            Err(
                location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                    name,
                )),
            )
        }
    }
}

mod selector_depth;

/// Parse a selector list from CSS source using raikiri's SelectorImpl.
///
/// Token blocks are limited to 32 nested levels before recursive parsing.
///
/// Seed helper — returns a `SelectorList<RaikiriSelectorImpl>` and stringifies
/// errors for the feasibility prototype. A future pass will replace the `Result<_, String>` shape
/// with a proper structured error type.
pub fn parse_selector_list(input: &str) -> Result<SelectorList<RaikiriSelectorImpl>, String> {
    let mut parser_input = ParserInput::new(input);
    let mut css_parser = CssParser::new(&mut parser_input);
    let start = css_parser.state();
    selector_depth::check_selector_token_depth(&mut css_parser, 0)
        .map_err(|_| "selector token nesting exceeds the supported depth".to_owned())?;
    css_parser.reset(&start);
    SelectorList::parse(&RaikiriSelectorParser, &mut css_parser, ParseRelative::No)
        .map_err(|e| format!("selector parse error: {e:?}"))
}

// ---------------------------------------------------------------------------
// Tests — smoke coverage for the seed. Real cascade / matching tests come later.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
