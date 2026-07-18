//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3.
//!
//! # Status
//!
//! Scaffolding only (raikiri-spike-rbo). The rule tree stores parsed `@page`
//! rules; cascade application, per-page `PageBox` derivation, and margin-box
//! slot layout are deferred to M4.
//!
//! # Primary source
//!
//! - CSS Paged Media Module Level 3, "Page selectors syntax" — the section
//!   number varies across W3C TR revisions, so we cite only the stable
//!   fragment anchor:
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
//! ways; M4 relaxes these as the cascade side lands (tracked separately —
//! see the bd task ledger for M4 handoff followups):
//!
//! 1. Only a *single* page-selector is accepted (no comma-list). The spec
//!    allows `@page :first, :left { … }`; we drop such rules for now.
//! 2. Only *zero or one* `<pseudo-page>` per selector, and only when the
//!    ident is absent. The spec allows `<ident> <pseudo-page>+`, so
//!    `@page named:first { … }` and `@page :first :left { … }` are both
//!    spec-valid but rejected here.
//!
//! # Note on `:nth-page`
//!
//! Earlier scaffolding drafts included a `PageSelector::NthPage` variant for
//! `:nth-page(An+B)`. That variant was removed after reviewer verification
//! that `:nth-page` is not part of CSS Paged Media Level 3 nor the Level 4
//! Editor's Draft. Accepting it under autonomous authority would emit a
//! `PageSelector` variant no primary source defines, forcing invented cascade
//! semantics at M4. The decision on whether raikiri should ship a spec-outside
//! `:nth-page` extension (e.g. for GCPM prototyping) is deferred to a human
//! ledger — see the bd task filed as an M4-handoff escalation.

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

use crate::Atom;
use crate::rule::Declaration;
use crate::ruletree::Origin;

/// Parsed `@page` selector.
///
/// Corresponds to a single `<page-selector>` production from CSS Paged Media
/// L3 (see module docs for the deliberate narrowings; anchor fragment
/// `#page-selectors-syntax` in the spec).
///
/// TODO(M4 shape debt): full L3 `<page-selector-list>` coverage requires a
/// `Vec<PageSelectorEntry { ident: Option<Atom>, pseudos: Vec<PagePseudo> }>`
/// shape. The current single-variant enum is a scaffolding compromise —
/// relaxing it is bd-tracked as an M4 followup.
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
}

/// Parsed `@page` rule — selector + declaration list + source order + origin.
///
/// `source_order` numbers `@page` rules *independently* of style rules; the
/// two rule kinds cascade in different tuple positions in the CSS spec, so a
/// separate 0-indexed counter keeps their bookkeeping decoupled and leaves
/// `StyleRule::source_order` semantics untouched. Cross-kind ordering will be
/// reconstructed by M4 cascade code if needed.
///
/// Field order (selector → declarations → source_order → origin) mirrors
/// [`crate::StyleRule`] so both rule kinds present the same shape to M4
/// cascade code (raikiri-spike-jzv M4 pre-work).
#[derive(Clone, Debug)]
pub struct PageRule {
    /// Which pages this rule applies to.
    pub selector: PageSelector,
    /// Declarations from the block body — parsed with the same
    /// `parse_declaration_block` used by qualified rules, so unsupported
    /// properties are silently dropped (matching the M1.4 policy).
    ///
    /// M4 note: `@page`-specific descriptors (`size`, `marks`, `bleed`, and
    /// the margin-box at-rules `@top-left` etc. per L3 §5) are also dropped
    /// by this reuse; wiring them is M4 scope.
    pub declarations: Vec<Declaration>,
    /// 0-indexed source order among `@page` rules across all
    /// `RuleTree::add_stylesheet` calls.
    pub source_order: u32,
    /// Cascade origin this rule was parsed under. See [`Origin`] for the
    /// current variant set — under M1.4a the crate exposes only UA and
    /// Author because User declarations are folded into Author (that
    /// folding rationale lives on [`Origin`] itself).
    ///
    /// Two primary sources back the wiring; the fragment anchors are the
    /// stable form of each citation:
    ///
    /// - CSS Paged Media Level 3, "Cascading in the page context" —
    ///   "Declarations in page and margin contexts cascade just like
    ///   declarations in style rule for elements", i.e. `@page`
    ///   *participates* in the cascade:
    ///   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
    /// - CSS Cascading Level 4, "Cascade Origins" — the per-origin
    ///   ordering mechanism (UA < Author, `!important` reversal) that
    ///   `@page` rules cascade through:
    ///   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
    ///
    /// M4 pre-work (raikiri-spike-jzv): the field is populated at parse
    /// time so M4 cascade wiring never has to re-index page rules by
    /// origin. The cascade *ordering* itself is M4 scope and not wired
    /// here.
    pub origin: Origin,
}

/// Parse the prelude of an `@page` rule.
///
/// The caller (a cssparser `AtRuleParser::parse_prelude`) hands us a parser
/// that ends at the block's opening `{`. We accept exactly one of:
///
/// - empty (→ [`PageSelector::Default`])
/// - `<ident>` (→ [`PageSelector::Named`])
/// - `:` `ident` where ident matches one of the four L3 pseudo-pages
///
/// Anything else — comma-list, ident+pseudo combo, two pseudos, unknown
/// pseudo-keyword, or a functional pseudo — is an `Err`, which cssparser
/// converts into "drop the whole @page rule".
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
/// consumed. Only the four L3 pseudo-page idents (`first` / `left` / `right`
/// / `blank`) are accepted; any functional pseudo (e.g. `:nth-page(...)`) is
/// rejected as an unknown pseudo per L3.
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
        _ => Err(input.new_custom_error(())),
    }
}
