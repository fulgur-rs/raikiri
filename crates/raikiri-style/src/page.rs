//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3 §4.3.
//!
//! # Status
//!
//! Scaffolding only (raikiri-spike-rbo). The rule tree stores parsed `@page`
//! rules; cascade application, per-page `PageBox` derivation, and margin-box
//! slot layout are deferred to M4.
//!
//! # Primary source
//!
//! - CSS Paged Media Module Level 3, §4.3 "@page rule grammar":
//!   <https://www.w3.org/TR/css-page-3/#page-selectors-syntax>
//!
//! # Grammar coverage
//!
//! The spec grammar is:
//!
//! ```text
//! <page-selector-list> = <page-selector>#
//! <page-selector>      = [ <ident-token>? <pseudo-page>* ]!
//! <pseudo-page>        = ':' [ left | right | first | blank ]
//! ```
//!
//! This scaffolding is *narrower* than the spec grammar in two deliberate
//! ways; M4 may relax these as the cascade side lands:
//!
//! 1. Only a *single* page-selector is accepted (no comma-list). The spec
//!    allows `@page :first, :left { … }`; we drop such rules for now.
//! 2. Only *zero or one* `<pseudo-page>` per selector, and only when the
//!    ident is absent. The spec allows `<ident> <pseudo-page>+`, so
//!    `@page named:first { … }` and `@page :first :left { … }` are both
//!    spec-valid but rejected here.
//!
//! In addition, [`PageSelector::NthPage`] models `:nth-page(An+B)`, which is
//! **not** part of Level 3. It is included as a task-directed scaffolding
//! extension (raikiri-spike-rbo scope item 2) so downstream code can pattern
//! on the variant when the CSS Paged Media Level 4 addition lands. No cascade
//! matching is wired up yet.

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case, parse_nth};

use crate::Atom;
use crate::rule::Declaration;

/// Parsed `@page` selector.
///
/// Corresponds to a single `<page-selector>` production from CSS Paged Media
/// L3 §4.3 (see module docs for the deliberate narrowings). [`Self::NthPage`]
/// is a task-directed extension beyond L3 scope.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageSelector {
    /// `@page { … }` — matches every page (empty selector).
    Default,
    /// `@page :first { … }` — first page of the document.
    First,
    /// `@page :left { … }` — left (verso) pages.
    Left,
    /// `@page :right { … }` — right (recto) pages.
    Right,
    /// `@page :blank { … }` — blank pages inserted by forced page breaks.
    Blank,
    /// `@page <ident> { … }` — named page selector.
    Named(Atom),
    /// `@page :nth-page(An+B) { … }` — task-directed extension. Not part of
    /// CSS Paged Media Level 3; kept as a scaffolding placeholder so cascade
    /// (M4) can pattern-match without another shape churn.
    NthPage {
        /// `A` coefficient of the `An+B` micro-syntax.
        a: i32,
        /// `B` constant of the `An+B` micro-syntax.
        b: i32,
    },
}

/// Parsed `@page` rule — selector + declaration list + source order.
///
/// `source_order` numbers `@page` rules *independently* of style rules; the
/// two rule kinds cascade in different tuple positions in the CSS spec, so a
/// separate 0-indexed counter keeps their bookkeeping decoupled and leaves
/// `StyleRule::source_order` semantics untouched. Cross-kind ordering will be
/// reconstructed by M4 cascade code if needed.
#[derive(Clone, Debug)]
pub struct PageRule {
    /// Which pages this rule applies to.
    pub selector: PageSelector,
    /// Declarations from the block body — parsed with the same
    /// `parse_declaration_block` used by qualified rules, so unsupported
    /// properties are silently dropped (matching the M1.4 policy).
    pub declarations: Vec<Declaration>,
    /// 0-indexed source order among `@page` rules across all
    /// `RuleTree::add_stylesheet` calls.
    pub source_order: u32,
}

/// Parse the prelude of an `@page` rule.
///
/// The caller (a cssparser `AtRuleParser::parse_prelude`) hands us a parser
/// that ends at the block's opening `{`. We accept exactly one of:
///
/// - empty (→ [`PageSelector::Default`])
/// - `<ident>` (→ [`PageSelector::Named`])
/// - `:` `ident` where ident matches one of the four L3 pseudo-pages
/// - `:` `nth-page(An+B)` (task-directed extension)
///
/// Anything else — comma-list, ident+pseudo combo, two pseudos, unknown
/// pseudo-keyword — is an `Err`, which cssparser converts into "drop the
/// whole @page rule".
pub(crate) fn parse_page_prelude<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSelector, ParseError<'i, ()>> {
    // Empty prelude → `@page { … }` matches all pages.
    if input.is_exhausted() {
        return Ok(PageSelector::Default);
    }

    // Clone the token so the parser can move past it without a borrow tangle.
    let first = input.next()?.clone();
    let selector = match first {
        Token::Ident(name) => PageSelector::Named(Atom::from(name.as_ref())),
        Token::Colon => parse_pseudo_page(input)?,
        _ => return Err(input.new_custom_error(())),
    };

    // Enforce the single-selector, single-pseudo scaffolding narrowing
    // (see module docs). If M4 relaxes to L3-full grammar, the check moves.
    input
        .expect_exhausted()
        .map_err::<ParseError<'i, ()>, _>(Into::into)?;
    Ok(selector)
}

/// Parse the `<pseudo-page>` production after the leading `:` has been
/// consumed. Also handles the `:nth-page(An+B)` task-directed extension.
fn parse_pseudo_page<'i>(input: &mut Parser<'i, '_>) -> Result<PageSelector, ParseError<'i, ()>> {
    let tok = input.next()?.clone();
    match tok {
        Token::Ident(name) => {
            let selector = match_ignore_ascii_case! { &name,
                "first" => PageSelector::First,
                "left"  => PageSelector::Left,
                "right" => PageSelector::Right,
                "blank" => PageSelector::Blank,
                _ => return Err(input.new_custom_error(())),
            };
            Ok(selector)
        }
        Token::Function(name) if name.eq_ignore_ascii_case("nth-page") => {
            let (a, b) = input.parse_nested_block(|nested| {
                let ab = parse_nth(nested).map_err::<ParseError<'i, ()>, _>(Into::into)?;
                // Reject `:nth-page(2n+1 garbage)` — matches the same
                // exhaustive-consumption discipline as DeclParser::parse_value.
                nested
                    .expect_exhausted()
                    .map_err::<ParseError<'i, ()>, _>(Into::into)?;
                Ok::<(i32, i32), ParseError<'i, ()>>(ab)
            })?;
            Ok(PageSelector::NthPage { a, b })
        }
        _ => Err(input.new_custom_error(())),
    }
}
