//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3.
//!
//! # Status
//!
//! Scaffolding only. The rule tree stores parsed `@page` rules; cascade
//! application, per-page `PageBox` derivation, and margin-box slot layout
//! are deferred to M4.
//!
//! # Primary source
//!
//! Three anchors are cited, one per production the parser consumes:
//!
//! - `<page-selector-list>` / `<page-selector>` grammar and the "No whitespace
//!   is allowed between the productions" compound rule — CSS Paged Media
//!   Module Level 3, §4.3 "@page rule grammar":
//!   <https://www.w3.org/TR/css-page-3/#syntax-page-selector>
//! - `<pseudo-page>` grammar production (the four `:first` / `:left` /
//!   `:right` / `:blank` idents) — CSS Paged Media Module Level 3 §4.3, dfn
//!   anchor for the production:
//!   <https://www.w3.org/TR/css-page-3/#typedef-pseudo-page>
//! - `<ident-token>` = named-page ident. This ident originates from the
//!   `page` property (§8.1 "Using named pages: page"); its value is a
//!   `<custom-ident>` and therefore case-sensitive even in ASCII (see
//!   [`PageSelectorEntry::ident`] for the case-sensitivity citation chain):
//!   <https://www.w3.org/TR/css-page-3/#using-named-pages>
//!
//! # Grammar coverage (raikiri-spike-mvu, M4 pre-work)
//!
//! The spec grammar is:
//!
//! ```text
//! <page-selector-list> = <page-selector>#
//! <page-selector>      = [ <ident-token>? <pseudo-page>* ]!
//! <pseudo-page>        = ':' [ left | right | first | blank ]
//! ```
//!
//! The `<page-selector>` and `<pseudo-page>` productions are *compound*: the
//! spec explicitly states "No whitespace is allowed between the productions
//! in `<page-selector>` or `<pseudo-page>` (similar to the rule for
//! `<compound-selector>`)". Whitespace around the `,` separator of
//! `<page-selector-list>` is permitted (it is not a compound at that level).
//!
//! This parser accepts the full L3 shape:
//!
//! - Empty prelude (`@page { … }`) — matches every page.
//! - Named-page ident alone (`@page named { … }`).
//! - One-or-more `<pseudo-page>` alone (`@page :first { … }`,
//!   `@page :first:left { … }`).
//! - Ident followed by one-or-more `<pseudo-page>` (`@page named:first { … }`).
//! - Comma-separated list of the above (`@page :first, :left { … }`).
//!
//! Whitespace *within* a compound (e.g. `@page : left`, `@page named :first`,
//! `@page :first :left`) is rejected per the compound rule — the whole
//! `@page` rule is dropped. This tightening addresses the codex §8.3 F3
//! finding on the raikiri-spike-rbo scaffolding.
//!
//! # Note on `:nth-page`
//!
//! Earlier scaffolding drafts included an `NthPage` pseudo variant for
//! `:nth-page(An+B)`. That variant was removed after reviewer verification
//! that `:nth-page` is not part of CSS Paged Media Level 3 nor the Level 4
//! Editor's Draft. Accepting it under autonomous authority would emit a
//! [`PagePseudo`] variant no primary source defines, forcing invented
//! cascade semantics at M4. The decision on whether raikiri should ship a
//! spec-outside `:nth-page` extension (e.g. for GCPM prototyping) is
//! deferred to a human ledger — see the bd task filed as an M4-handoff
//! escalation.

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

use crate::Atom;
use crate::rule::Declaration;
use crate::ruletree::Origin;

/// Parsed `@page` selector list — a comma-separated list of compound
/// `<page-selector>` productions from CSS Paged Media L3 §4.3 (anchor
/// [`#syntax-page-selector`](https://www.w3.org/TR/css-page-3/#syntax-page-selector)).
///
/// An empty prelude (`@page { … }`) is represented as a single empty
/// [`PageSelectorEntry`] so the M4 cascade code can uniformly iterate
/// `entries` without a special "default" enum arm.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSelector {
    /// The list of compound page-selectors (comma-separated in source
    /// order). Always non-empty for a successfully-parsed `@page` rule.
    pub entries: Vec<PageSelectorEntry>,
}

/// A single compound `<page-selector>` = `[ <ident-token>? <pseudo-page>* ]!`.
///
/// The `!` marker in the L3 grammar means at least one component (ident or
/// pseudo) must be present, EXCEPT for the special case where the entire
/// `@page` prelude is empty — that maps to `PageSelector { entries: vec![
/// PageSelectorEntry::default() ] }`. Anything appearing between compound
/// components (whitespace, extra tokens) is a compound-rule violation and
/// causes the whole `@page` rule to be dropped.
///
/// See [`PageSelector`] for the surrounding list shape.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageSelectorEntry {
    /// Optional named-page ident, e.g. `named` in `@page named { … }`.
    ///
    /// The ident originates from the `page` property (CSS Paged Media L3
    /// §8.1, anchor
    /// [`#using-named-pages`](https://www.w3.org/TR/css-page-3/#using-named-pages))
    /// and is typed as `<custom-ident>`; per CSS Values L4 §4.2
    /// [`#custom-idents`](https://www.w3.org/TR/css-values-4/#custom-idents)
    /// `<custom-ident>` is "fully case-sensitive … even in the ASCII range".
    /// The stored [`Atom`] therefore preserves the source casing verbatim,
    /// and rules like `@page Cover { … }` vs `@page cover { … }` are two
    /// distinct named-pages (pinned by
    /// `page_named_ident_is_case_sensitive`).
    pub ident: Option<Atom>,
    /// Zero-or-more pseudo-pages, e.g. `[First, Left]` for `@page :first:left`.
    pub pseudos: Vec<PagePseudo>,
}

/// The four `<pseudo-page>` idents defined in CSS Paged Media L3 §4.3
/// (production dfn anchor
/// [`#typedef-pseudo-page`](https://www.w3.org/TR/css-page-3/#typedef-pseudo-page)).
///
/// Case is not preserved — parsing is ASCII case-insensitive per CSS Syntax
/// keyword rules (`match_ignore_ascii_case!`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PagePseudo {
    /// `:first` — the first page of the document.
    First,
    /// `:left` — left (verso) pages.
    Left,
    /// `:right` — right (recto) pages.
    Right,
    /// `:blank` — blank pages inserted by forced page breaks.
    Blank,
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
/// that ends at the block's opening `{`. We accept the full L3
/// `<page-selector-list>` grammar (see module docs). An empty prelude
/// (`@page { … }`) becomes a [`PageSelector`] with one default-constructed
/// entry so downstream cascade code can treat "matches every page" as an
/// entry that matches unconditionally.
///
/// Anything violating the compound rule — whitespace between an ident and
/// its pseudo-page, whitespace between a `:` and its pseudo keyword, or
/// whitespace between two pseudo-pages of the same compound — returns
/// `Err`, which cssparser converts into "drop the whole @page rule".
pub(crate) fn parse_page_prelude<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSelector, ParseError<'i, ()>> {
    // Empty prelude → `@page { … }` matches every page. Represented as a
    // single empty entry so M4 cascade code can iterate uniformly.
    if input.is_exhausted() {
        return Ok(PageSelector {
            entries: vec![PageSelectorEntry::default()],
        });
    }

    // `parse_comma_separated` skips whitespace around each `,` (which is
    // spec-permitted since the list is not a compound) and, per its
    // `parse_until_before` / `parse_entirely` internals, enforces that each
    // closure invocation consumes every token up to the delimiter — so any
    // trailing garbage inside a compound propagates as an Err and the whole
    // `@page` rule is dropped.
    let entries = input.parse_comma_separated(parse_compound_selector)?;
    Ok(PageSelector { entries })
}

/// Parse a single compound `<page-selector>` = `[ <ident-token>?
/// <pseudo-page>* ]!`.
///
/// The compound rule (spec: "No whitespace is allowed between the
/// productions in `<page-selector>` or `<pseudo-page>`") is enforced by
/// using `Parser::next_including_whitespace` at every intra-compound
/// boundary — that primitive surfaces `Token::WhiteSpace(_)` but is
/// transparent to comments (comments are stripped at CSS Syntax L3
/// tokenization, so `:first/*x*/:left` must parse the same as
/// `:first:left`). Only the *first* component is read with
/// `Parser::next` (whitespace-skipping) because that skip covers the
/// spec-permitted whitespace between commas of the outer list and before
/// the very first entry.
fn parse_compound_selector<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PageSelectorEntry, ParseError<'i, ()>> {
    let mut entry = PageSelectorEntry::default();

    // First component: whitespace-skipping OK — we're either at the start of
    // the prelude or just past a `,`, both non-compound boundaries.
    let first = input.next()?.clone();
    match first {
        Token::Ident(name) => {
            entry.ident = Some(Atom::from(name.as_ref()));
        }
        Token::Colon => {
            parse_and_push_pseudo(input, &mut entry.pseudos)?;
        }
        other => {
            return Err(input.new_unexpected_token_error(other));
        }
    }

    // Additional pseudo-pages: each must be adjacent to the previous
    // component. Save the state before each peek so a non-`:` (or an EOI /
    // whitespace token) can rewind and let the outer `parse_comma_separated`
    // see what actually comes next (whitespace before a `,`, or EOI).
    //
    // `next_including_whitespace` is used (not `..._and_comments`) because
    // CSS Syntax L3 §4 strips comments at tokenization: `:first/*x*/:left`
    // MUST parse identically to `:first:left`. Whitespace, however, remains
    // a compound boundary — so this primitive surfaces `Token::WhiteSpace(_)`
    // and lets the "no whitespace within compound" rule stay strict.
    loop {
        let checkpoint = input.state();
        let peek = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => break, // EOI — compound is complete
        };
        match peek {
            Token::Colon => {
                parse_and_push_pseudo(input, &mut entry.pseudos)?;
            }
            _ => {
                // Not a `:` (could be whitespace, comma, or anything else).
                // Rewind — outer machinery handles it. Whitespace here is
                // fine because it's *after* the compound, not within it.
                input.reset(&checkpoint);
                break;
            }
        }
    }

    Ok(entry)
}

/// Consume the ident that must immediately follow a `:` in a `<pseudo-page>`
/// and push the matching [`PagePseudo`] variant.
///
/// `Parser::next_including_whitespace` is used so that a `@page : left { … }`
/// shape (whitespace between colon and ident) surfaces as
/// `Token::WhiteSpace(_)` rather than being silently skipped past — matching
/// the compound rule from L3. Comments between `:` and the ident
/// (`@page :/*x*/left`) are transparent per CSS Syntax L3 §4 (stripped at
/// tokenization); this primitive strips them but preserves whitespace.
fn parse_and_push_pseudo<'i>(
    input: &mut Parser<'i, '_>,
    pseudos: &mut Vec<PagePseudo>,
) -> Result<(), ParseError<'i, ()>> {
    let tok = input.next_including_whitespace()?.clone();
    match tok {
        Token::Ident(name) => {
            let pseudo = match_ignore_ascii_case! { &name,
                "first" => PagePseudo::First,
                "left"  => PagePseudo::Left,
                "right" => PagePseudo::Right,
                "blank" => PagePseudo::Blank,
                _ => return Err(input.new_custom_error(())),
            };
            pseudos.push(pseudo);
            Ok(())
        }
        // WhiteSpace here (`@page : left`), functional pseudo (`:nth-page(…)`),
        // or any other token type is invalid per the compound rule.
        _ => Err(input.new_custom_error(())),
    }
}
