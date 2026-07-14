//! raikiri-style — CSS engine (cssparser + selectors + cascade + GCPM static side).
//!
//! M0 status: seed. Provides a minimal `RaikiriSelectorImpl` that satisfies
//! every associated-type / trait bound of `selectors::SelectorImpl` without
//! depending on `stylo`. This is enough for `raikiri-feasibility` to prove
//! nzv.8 (`selectors` standalone, no stylo) and forms the starting point for
//! M1's real cascade / matching implementation.
//!
//! `precomputed-hash` is encapsulated as a direct dep of this crate only. It
//! is intentionally NOT promoted to `[workspace.dependencies]` — see the
//! comment on `Cargo.toml`.

#![allow(missing_docs)] // M0 seed; docs come with M1

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
// Pseudo-class / pseudo-element enums (minimal M0 set).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoClass {
    Hover,
    Active,
}

impl ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        match self {
            PseudoClass::Hover => dest.write_str(":hover"),
            PseudoClass::Active => dest.write_str(":active"),
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
            Err(location.new_custom_error(
                SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
            ))
        }
    }
}

/// Parse a selector list from CSS source using raikiri's SelectorImpl.
///
/// M0 seed helper — returns a `SelectorList<RaikiriSelectorImpl>` and stringifies
/// errors for the feasibility spike. M1 will replace the `Result<_, String>` shape
/// with a proper `raikiri_traits`-defined error type.
pub fn parse_selector_list(
    input: &str,
) -> Result<SelectorList<RaikiriSelectorImpl>, String> {
    let mut parser_input = ParserInput::new(input);
    let mut css_parser = CssParser::new(&mut parser_input);
    SelectorList::parse(&RaikiriSelectorParser, &mut css_parser, ParseRelative::No)
        .map_err(|e| format!("selector parse error: {e:?}"))
}

// ---------------------------------------------------------------------------
// Tests — smoke coverage for the seed. Real cascade / matching tests come in M1.
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
}
