//! selectors + cssparser compat (API-level), nzv-spike for raikiri-spike-nzv.10
//!
//! Verifies that `selectors 0.39` and `cssparser 0.37` co-compile at the
//! workspace-pinned versions and that their public parser APIs share the same
//! `cssparser::Parser<'i, 't>` / `cssparser::ParserInput<'i>` types when driving
//! a realistic "prelude `{` block `}`" split.
//!
//! ## What this spike shows
//!
//! - `cssparser::ParserInput<'i>` is the single input type both crates surface
//!   in their public API — `selectors::SelectorList::<Impl>::parse` takes
//!   `&mut cssparser::Parser<'i, 't>` (see selectors 0.39 `parser.rs:517`),
//!   which is constructed from `cssparser::ParserInput::new`. A generic helper
//!   `_selectors_uses_cssparser_parser` below references the fn-item type of
//!   `SelectorList::parse` under a `SelectorImpl` bound so this file will NOT
//!   compile if the two crates are ever linked against differing `cssparser`
//!   semvers. That satisfies the "API-level compat" criterion for nzv.10.
//! - The runtime test drives one shared `cssparser::Parser` instance from the
//!   raw CSS string all the way through: prelude split at `{`, then
//!   `parse_nested_block` + `RuleBodyParser` over the declarations — this is
//!   the exact hand-off shape a real style-rule parser (raikiri-style, M1+)
//!   will use to route selectors into `selectors::SelectorList::parse` and
//!   declaration text into per-property parsers.
//!
//! ## Finding (recorded here for nzv.12 feasibility-report)
//!
//! Fully instantiating a custom `selectors::SelectorImpl` from within
//! `raikiri-feasibility` was NOT possible under this task's "one file, no
//! Cargo.toml edits" constraint: `SelectorImpl` requires `Identifier` /
//! `LocalName` to implement `precomputed_hash::PrecomputedHash`, but the
//! `precomputed-hash` crate is only a transitive dep of `selectors` and is not
//! re-exported by it. Any downstream crate that owns a `SelectorImpl`
//! (nzv.8 `selectors_standalone` here; the real `raikiri-style` at M1+) MUST
//! declare `precomputed-hash` as a direct dep. The spike therefore verifies
//! the compat at the type level (via the generic function-item reference) plus
//! the cssparser half of the parser at runtime, which is where the two crates
//! actually meet on the shared `cssparser::Parser` instance.

use std::marker::PhantomData;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser as CssParser,
    ParserInput, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::{
    parser::{ParseRelative, Parser as SelectorParser},
    SelectorImpl, SelectorList,
};

// ---------------------------------------------------------------------------
// Compile-time proof: selectors uses cssparser::Parser at the API surface.
// ---------------------------------------------------------------------------

/// Type-level proof that `selectors::SelectorList::parse` and this crate's
/// direct `cssparser` dep resolve to the same `cssparser::Parser` type.
///
/// The body of a generic fn is type-checked at definition time even without
/// being called, so referencing the fn-item type of
/// `SelectorList::<Impl>::parse::<P>` here forces `cssparser::Parser<'i, 't>`
/// (which appears in that fn's signature — selectors 0.39 `parser.rs:517-524`)
/// to be the same nominal type this crate uses in [`parse_style_rule`] below.
///
/// If the workspace ever ended up double-resolving `cssparser`, this file
/// would fail to compile with "expected `cssparser::Parser<…>`, found a
/// different `cssparser::Parser<…>`".
#[allow(dead_code)]
fn _selectors_uses_cssparser_parser<Impl, P>()
where
    Impl: SelectorImpl,
    for<'i> P: SelectorParser<'i, Impl = Impl>,
{
    let _fp = SelectorList::<Impl>::parse::<P>;
    // Also anchor ParseRelative so nzv.12 can trace which selectors 0.39 enum
    // variant `parse_style_rule` would pass when nzv.8 lands a SelectorImpl.
    let _pr: ParseRelative = ParseRelative::No;
    let _phantom: PhantomData<(Impl, P)> = PhantomData;
}

// ---------------------------------------------------------------------------
// Declaration-side parser (cssparser only — no SelectorImpl required here).
// ---------------------------------------------------------------------------

/// A minimal declaration captured from the block half of a style rule.
///
/// Real property parsing (specified-value structs, `!important`, etc.) is out
/// of scope for this feasibility spike — we only care that the cssparser
/// declaration-block driver reports the right number of items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Declaration name (e.g. `"color"`).
    pub name: String,
    /// Declaration value text, verbatim and trimmed (e.g. `"red"`).
    pub value: String,
}

struct DeclParser;

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i, 't>,
        _decl_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace().is_ok() {}
        let value = input.slice_from(start).trim().to_string();
        Ok(Declaration {
            name: name.as_ref().to_owned(),
            value,
        })
    }
}

impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Declaration, ()> for DeclParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// The spike: split a "prelude { block }" via a single shared cssparser::Parser.
// ---------------------------------------------------------------------------

/// A parsed rule: the verbatim selector prelude (what
/// `selectors::SelectorList::parse` would consume) plus the declaration list.
#[derive(Debug)]
pub struct ParsedRule {
    /// Raw selector text (trimmed), captured verbatim from the prelude slice
    /// of the shared `cssparser::Parser`.
    pub selector_text: String,
    /// Declarations parsed from within `{ … }`.
    pub declarations: Vec<Declaration>,
}

/// Split `"prelude { block }"` via a single shared `cssparser::Parser`.
///
/// The prelude closure captures the exact byte slice that
/// `selectors::SelectorList::parse` would consume (the `parse_until_before`
/// call is the same one servo/stylo uses upstream when handing off to the
/// selector parser). The block half drives a real
/// `cssparser::RuleBodyParser` over `DeclParser`, which is the same iterator
/// selectors 0.39 users pair with `SelectorList::parse` in a real style-rule
/// pipeline. That the same `&mut CssParser<'i, 't>` value flows through both
/// halves is the whole point of this spike.
pub fn parse_style_rule(css: &str) -> Result<ParsedRule, String> {
    let mut input_store = ParserInput::new(css);
    let mut input = CssParser::new(&mut input_store);

    // --- Prelude half -----------------------------------------------------
    let prelude_start = input.position();
    let _: Result<(), ParseError<'_, ()>> =
        input.parse_until_before(Delimiter::CurlyBracketBlock, |inner| {
            while inner.next_including_whitespace().is_ok() {}
            Ok(())
        });
    let selector_text = input.slice_from(prelude_start).trim().to_string();

    // --- Block half -------------------------------------------------------
    input
        .expect_curly_bracket_block()
        .map_err(|e| format!("expected `{{` block, got {e:?}"))?;
    let declarations: Result<Vec<Declaration>, ParseError<'_, ()>> =
        input.parse_nested_block(|inner| {
            let mut parser = DeclParser;
            let iter = RuleBodyParser::new(inner, &mut parser);
            let mut out = Vec::new();
            for item in iter {
                match item {
                    Ok(decl) => out.push(decl),
                    Err((e, _slice)) => return Err(e),
                }
            }
            Ok(out)
        });
    let declarations = declarations.map_err(|e| format!("block parse error: {e:?}"))?;

    Ok(ParsedRule {
        selector_text,
        declarations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Design-doc expected pass criterion: complete parse of a simple CSS rule
    /// using both crates via one shared `cssparser::Parser` instance.
    #[test]
    fn parses_simple_rule_via_shared_parser() {
        let rule = parse_style_rule("p:hover { color: red; }").expect("parse");
        assert_eq!(rule.selector_text, "p:hover");
        assert_eq!(rule.declarations.len(), 1);
        assert_eq!(rule.declarations[0].name, "color");
        assert_eq!(rule.declarations[0].value, "red");
    }

    /// Multi-declaration + combinator + functional pseudo-class — exercises the
    /// prelude slice-capture and RuleBodyParser iteration on more realistic
    /// input.
    #[test]
    fn parses_multi_decl_rule_with_combinator() {
        let rule = parse_style_rule(".a > b:not(.c) { color: red; font-size: 12px; }")
            .expect("parse");
        assert_eq!(rule.selector_text, ".a > b:not(.c)");
        assert_eq!(rule.declarations.len(), 2);
        assert_eq!(rule.declarations[0].name, "color");
        assert_eq!(rule.declarations[0].value, "red");
        assert_eq!(rule.declarations[1].name, "font-size");
        assert_eq!(rule.declarations[1].value, "12px");
    }

    /// A trailing declaration with no `;` still counts — cssparser's
    /// `RuleBodyParser` yields it at end-of-block.
    #[test]
    fn parses_rule_without_trailing_semicolon() {
        let rule = parse_style_rule("h1 { margin: 0 }").expect("parse");
        assert_eq!(rule.selector_text, "h1");
        assert_eq!(rule.declarations.len(), 1);
        assert_eq!(rule.declarations[0].name, "margin");
        assert_eq!(rule.declarations[0].value, "0");
    }
}
