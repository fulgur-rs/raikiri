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
//! GCPM static side、@page / @media / @supports、L4 selectors、class/id/attribute
//! selector、combinator は将来追加予定。
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

pub mod style_dom;
pub use style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

pub mod property;
pub use property::{
    BorderRadius, BoxShadowItem, CssColor, DisplayValue, Length, LengthOrAuto, LengthOrNormal,
    Outline, PropertyKey, PropertyValue, Sides, TextShadowColor, TextShadowItem,
};

pub mod rule;
pub use rule::{Declaration, StyleRule};

pub mod counter_style;
pub use counter_style::{
    CounterRange, CounterStyleRegistry, CounterStyleRule, CounterStyleSystem, CounterSymbol,
    NegativeDescriptor, PadDescriptor, RangeEntry, RangeLimit, parse_counter_style_rules,
    resolve_custom_counter,
};

pub mod page;
pub use page::{
    PageBleed, PageBleedDeclaration, PageCascadeResult, PageContextQuery, PageInheritance,
    PageMarks, PageMarksDeclaration, PageOrientation, PagePseudo, PageRule, PageSelector,
    PageSelectorEntry, PageSize, PageSizeDeclaration, PageSizeKeyword, cascade_page,
};

pub mod ruletree;
pub use ruletree::{Origin, RuleTree, build_rule_tree, walk_style_elements};

pub mod computed;
pub use computed::ComputedValues;

pub mod resolve;
pub use resolve::{
    ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackRepeat, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedLineHeight, ComputedOutline, ComputedTabSize, ComputedTextShadow, ResolveContext,
    lift_font_size, lift_length_or_normal, lift_length_percentage, lift_line_height, lift_tab_size,
    lift_text_shadow_item, resolve_border, resolve_border_radius, resolve_box_shadow_item,
    resolve_flex_basis, resolve_font_size, resolve_grid_auto_track_list,
    resolve_grid_inflexible_breadth, resolve_grid_template_tracks, resolve_grid_track_breadth,
    resolve_grid_track_list, resolve_grid_track_size, resolve_length_or_normal,
    resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_line_height, resolve_margin_length_or_auto,
    resolve_outline, resolve_tab_size, resolve_text_shadow_item, used_line_height_length,
};

pub mod specified;
pub use specified::SpecifiedValues;

pub mod cascade;
pub use cascade::{CascadeResult, cascade};

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
/// Pseudo-class: :dir()" — the published TR page
/// truncates before reaching this section for this crate's WebFetch tool,
/// same truncation `mod@crate::cascade`'s combinator doc already notes for
/// this spec):
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoElem {}

impl ToCss for PseudoElem {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl PseudoElement for PseudoElem {
    type Impl = RaikiriSelectorImpl;
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
    // `_after_part` is unused: the `selectors` crate only ever sets it after
    // a successfully-parsed `::part()` (gated by `Parser::parse_part()`,
    // which this impl leaves at its `false` default) or another
    // element-backed pseudo-element (gated by `parse_pseudo_element`
    // succeeding, which can never happen here — `crate::PseudoElem` is an
    // uninhabited enum, so no pseudo-element construct parses successfully
    // in this crate at all yet). Both preconditions are unreachable today,
    // so `after_part` is always `false` when this function runs — unlike
    // the `selectors` crate's own reference test impl (its internal
    // `"lang" if !after_part => ...` arm, `selectors-0.39.0/parser.rs`),
    // this crate has no live case to guard against. Revisit if/when
    // pseudo-element support is ever added.
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
}

/// Parse a selector list from CSS source using raikiri's SelectorImpl.
///
/// Seed helper — returns a `SelectorList<RaikiriSelectorImpl>` and stringifies
/// errors for the feasibility spike. A future pass will replace the `Result<_, String>` shape
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
}
