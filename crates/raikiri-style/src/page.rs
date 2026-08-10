//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3.
//!
//! # Status
//!
//! The rule tree stores parsed `@page` rules and [`cascade_page`] resolves the
//! winning declarations for a given page context (origin / specificity /
//! source-order cascade, then phase 2 = resolution against the page context's
//! inheritance parent, then phase 3 = absolutization against the page context's
//! own font-size + `border-*-width` style gating — see that function's doc).
//! Per-page `PageBox` derivation and
//! margin-box slot layout remain deferred to M4 and live downstream; this
//! module produces the declaration bag they consume.
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

use std::collections::HashMap;
use std::sync::LazyLock;

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

use crate::Atom;
use crate::cascade::{
    ResolvedAgainstInherited, cascade_rank, resolve_against_inherited, resolve_relative_font_size,
};
use crate::computed::ComputedValues;
use crate::property::{
    Border, BorderColor, BorderStyle, Length, LengthOrAuto, PropertyKey, PropertyValue, Sides,
};
use crate::resolve::{
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ResolveContext,
    lift_line_height, resolve_border, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_line_height, resolve_margin_length_or_auto, used_line_height_length,
};
use crate::rule::{Declaration, expand_shorthand_into};
use crate::ruletree::{Origin, RuleTree};
use crate::specified::INITIAL_BORDER;

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
/// cascade code (raikiri-spike-jzv M4 pre-work). Future field (`@page`-specific
/// descriptor size / marks / bleed / margin-box、cascade-origin cache 等) は
/// M4+ で追加、`#[non_exhaustive]` の恩恵で non-breaking。
#[non_exhaustive]
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
    ///
    /// # Why this field (and `RuleTree::page_rules`) is still `pub`
    ///
    /// Still `pub` after bd raikiri-spike-qzn3 — deliberately, and outside
    /// that task's approved scope. **This is the canonical, docs.rs-visible
    /// statement of what that leaves open**; [`crate::ruletree::RuleTree::page_rules`]
    /// points here rather than restating it (same discipline as
    /// [`crate::page::PageCascadeResult::declarations`] — spell
    /// `PageRule::declarations` verbatim if you add another pointer site;
    /// bd raikiri-spike-ykee).
    ///
    /// qzn3 closed the *element* path — the `style_rules` /
    /// `declarations` / `value` fields are now `pub(crate)`, reachable from
    /// outside the crate only through their fully `pub` read-only accessors
    /// (`RuleTree::style_rules()` / `StyleRule::declarations()` /
    /// `Declaration::value()`) — but left the *`@page`* path alone. The
    /// two are closed by different amounts:
    ///
    /// - **Element path**: closed by visibility alone. A consumer only ever
    ///   reaches a read-only `&[StyleRule]` / `&[Declaration]`; no `&mut`
    ///   path to a `Declaration`'s value is public.
    /// - **`@page` path** (this field, and `RuleTree::page_rules`): a
    ///   consumer can still duplicate / remove / reorder an existing
    ///   `Declaration` in place, flip its `important` flag, or clone an
    ///   existing `PageRule` and push back an edited copy (`PageRule` is
    ///   `#[non_exhaustive]`, so a brand-new one cannot be built from a
    ///   struct literal — a seed rule is required). What stays closed is
    ///   narrower: a consumer cannot manufacture a *new* `Declaration`
    ///   carrying a shorthand `PropertyValue` (`margin` / `padding` /
    ///   `border`). That holds for two reasons: (a) `Declaration::value` is
    ///   private, so no struct-literal / functional-update construction is
    ///   possible, and (b) every `Declaration` a consumer could clone came
    ///   out of `parse_declaration_block`, which never emits a shorthand key
    ///   (pinned by `expand_shorthand_into`'s exhaustive match, bd
    ///   raikiri-spike-ez7b). If either (a) or (b) breaks, the `@page` path
    ///   reopens from outside the crate — re-derive this section before
    ///   adding a public constructor to `Declaration`.
    ///
    /// The full mechanical derivation (all three shorthand-expansion call
    /// sites, and why (a)/(b) above are each load-bearing) lives on
    /// [`crate::rule::expand_shorthand_into`]'s doc. That function is
    /// `pub(crate)`, so its doc does not render on docs.rs — this section is
    /// the summary a docs.rs reader can actually reach.
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

// ---------------------------------------------------------------------------
// @page cascade order 本実装 (raikiri-spike-m4.1)
//
// CSS Paged Media Level 3, §"Cascading and page context" —
//   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
// CSS Cascading and Inheritance Level 4, §"Cascade Origin" —
//   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
//
// # Sibling arm convention (raikiri-spike-37n)
//
// Follows the sibling convention established by
// `crate::cascade::collect_cascaded` + `crate::cascade::pick_winners`:
// per-candidate `(value, important, origin, specificity, source_order)` tuple,
// group by `PropertyKey`, pick winner by `(rank, specificity, source_order)`
// where higher tuples beat lower. `rank` reuses `cascade_rank` verbatim —
// `@page` rules and style rules share the same origin ordering (spec §6.2).
// The only diverging element is the specificity type: `PageSpecificity` is a
// derived-`Ord` `(f, g, h)` triple per L3 §"Cascading and page context",
// whereas style rules use the `selectors` crate's 32-bit packed specificity.
// ---------------------------------------------------------------------------

/// Query describing which `@page` selectors apply to the current page.
///
/// The consumer (raikiri umbrella, dom-level page loop) supplies this bag
/// per page it is about to lay out. All fields are declarative — the caller
/// pre-computes `is_left` / `is_right` from `page_index` parity, `is_first`
/// from the page number, and `is_blank` from the fragmentation state
/// (raikiri-spike design doc §9.1 "page name 遷移ルール"). raikiri-style does
/// not know about page indexes, only about which pseudo-page states are
/// currently true.
///
/// Fields default to `None` / `false`, i.e. an unnamed page with no
/// pseudo-page state — matches only `@page { … }`.
///
/// `#[non_exhaustive]`: future M4 pseudo-pages (e.g. spec-outside extensions)
/// or additional context (media query state, forced-orientation flags) can be
/// added without a semver break. Consumers construct via
/// `PageContextQuery { page_name: …, is_first: …, ..Default::default() }` per
/// the standard `#[non_exhaustive]` pattern used throughout raikiri-style.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageContextQuery {
    /// Named-page ident from the `page:` property. `None` = unnamed page
    /// (only `@page` and `@page :<pseudo>` rules can match).
    ///
    /// Compared case-sensitively per CSS Values L4
    /// [`#custom-idents`](https://www.w3.org/TR/css-values-4/#custom-idents)
    /// `<custom-ident>` — "fully case-sensitive … even in the ASCII range"
    /// (mirrors the parse-time invariant pinned by
    /// `page_named_ident_is_case_sensitive` in ruletree tests, and the
    /// sibling citation on [`PageSelectorEntry::ident`]).
    pub page_name: Option<Atom>,
    /// Whether this is the first page (`:first` matches).
    pub is_first: bool,
    /// Whether this is a left / verso page (`:left` matches).
    pub is_left: bool,
    /// Whether this is a right / recto page (`:right` matches).
    pub is_right: bool,
    /// Whether this is a blank page inserted by a forced page break
    /// (`:blank` matches).
    pub is_blank: bool,
}

/// Winning declarations from an `@page` cascade pass.
///
/// Contains one entry per property that at least one matching `@page` rule
/// declared. `@page`-specific **descriptors** (`size`, `marks`, `bleed`) are
/// absent — the M1.4 property parser silently drops them, and wiring them is
/// M4 scope (see
/// `ruletree::tests::page_body_unsupported_property_drops_declaration`).
///
/// The ordinary box properties are **not** in that category: `margin` /
/// `padding` / `border-*` / `width` / `height` are parsed (the `margin`
/// shorthand has been expanded to longhands since raikiri-spike-0vv.5) and go
/// through both resolution phases below. Downstream page-layout code is
/// expected to translate this bag into its page-box model and future
/// `@page`-descriptor fields when the M4 descriptor property parser lands;
/// raikiri-style remains a leaf crate.
///
/// Iteration order over `declarations` is `HashMap`-random; consumers that
/// need a deterministic order should sort or look up by [`PropertyKey`]
/// (Tests here look up by key rather than iterating).
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageCascadeResult {
    // Private so the "resolved against the inheritance parent" contract
    // documented on `declarations()` is enforced by construction: only
    // `cascade_page` can populate this map (raikiri-spike-ygl0). Read-only
    // access is public API — see the `declarations()` accessor below, in
    // particular its "Why this is a method, not a field" section
    // (raikiri-spike-dyxj).
    declarations: HashMap<PropertyKey, PropertyValue>,
}

impl PageCascadeResult {
    /// Winning `(key, value)` per property, after the two resolution passes
    /// described below.
    ///
    /// **This doc is the canonical statement of that contract.** Other sites
    /// that touch it point here instead of restating it — enumerate them with
    /// `git grep "PageCascadeResult::declarations"` rather than trusting a
    /// list, because a list of pointer sites drifts exactly the way the
    /// duplicated rule statements did (bd raikiri-spike-ygl0 /
    /// raikiri-spike-sshp, twice). **A new site that mentions this contract
    /// must spell `PageCascadeResult::declarations` verbatim** (in an
    /// intra-doc link, or in plain text where links do not resolve, as the
    /// umbrella crate's `pub use` commentary does) — that is what keeps the
    /// grep recipe complete. The layer-vs-type rule the whole contract rests
    /// on ("the type does not name a layer") is canonical on [`Length`].
    ///
    /// # What is pinned mechanically, and what is not
    ///
    /// The **"no specified-layer residue"** claim below is pinned by
    /// `page::tests`' `page_declarations_carry_no_specified_layer_residue`
    /// (raikiri-spike-l3wg; before that, `text-align: match-parent` was the
    /// one documented exception and the same test was named
    /// `page_declarations_carry_exactly_one_specified_layer_residue`),
    /// `cascade_page_output_carries_no_specified_layer_residue` and
    /// `phase_3_variant_classification_matches_the_documented_counts`
    /// (bd raikiri-spike-awjx). The **individual phase-table rows** below (the
    /// `border-*-width` style gate, the `<percentage>` pass-through, `auto`,
    /// `line-height: <number>`) are *not* covered by those three — they each
    /// have their own acceptance test in `page::tests` instead, and their
    /// payloads are deliberately absent from `page_corpus` (one payload per
    /// variant is all it can carry).
    ///
    /// Three further limits, stated here so a consumer need not read the test
    /// module to learn them:
    ///
    /// - Both residue pins terminate at [`cascade_page`]. They assert that
    ///   *this* entry point's output is a computed-value bag; they cannot
    ///   assert it for an entry point that does not exist yet. CSS Paged
    ///   Media 3 §6's page-margin-box cascade ("page-margin boxes inherit from
    ///   the page context") would be a third path. Since bd raikiri-spike-7m33,
    ///   [`absolutize_in_page_context`]'s parameter type forces any caller that
    ///   reuses phase 3 to have gone through phase 2 first; see
    ///   [`crate::cascade::ResolvedAgainstInherited`] for exactly what that
    ///   does and does not close (bd raikiri-spike-m4.2 tracks the remainder).
    /// - The residue detector classifies five payload types exhaustively
    ///   (`Length` / `LengthOrAuto` / `LineHeight` / `FontWeightValue` /
    ///   `TextAlign`); a new payload case in any *other* type is not caught.
    /// - The detector's variant-addition tripwire is **one-way**: adding a
    ///   `PropertyValue` variant is a compile error in the test module, but
    ///   nothing then forces the author to extend `page_corpus`, so a new
    ///   variant can sit outside the corpus with every test green.
    ///
    /// **These are computed values, with no documented exception** (as of
    /// raikiri-spike-l3wg — see the phase 2 bullet below for the property
    /// that used to be the one exception). CSS Paged Media 3 §6 "Page Properties"
    /// (<https://www.w3.org/TR/css-page-3/#page-properties>) states that "both
    /// the page context and the margin context have a computed value for every
    /// property" and that "The page context inherits from the root element";
    /// the inheritance parent is the [`PageInheritance`] argument of
    /// [`cascade_page`].
    ///
    /// **Phase 2** — resolution against the inheritance parent
    /// ([`crate::cascade::resolve_against_inherited`]):
    ///
    /// - `font-weight` — `bolder` / `lighter` are resolved against the
    ///   inherited weight, so **no relative font-weight sentinel** reaches the
    ///   consumer (raikiri-spike-ygl0).
    /// - `font-size` — `em` / `rem` / `%` are absolutized against the root
    ///   element's computed font-size, so the value is always
    ///   [`Length::Px`]. §6 verbatim: "When used on
    ///   the font-size property in the page context, they are relative to the
    ///   font-size of the root element." (raikiri-spike-zls8). `larger` /
    ///   `smaller` (`<relative-size>`) are resolved the same way against the
    ///   root element's computed font-size, so **no relative font-size
    ///   sentinel** reaches the consumer either — same guarantee as the
    ///   `font-weight` bullet above (raikiri-spike-4rmu).
    /// - `text-align` — [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent)
    ///   is resolved against the inheritance parent's computed `text-align`
    ///   **and** `direction` (raikiri-spike-l3wg; origin: raikiri-spike-ygl0
    ///   §8.2 spec lens F1). CSS Text 3 §6.1
    ///   `#valdef-text-align-match-parent`
    ///   (<https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent>)
    ///   verbatim: "This value behaves the same as inherit (computes to its
    ///   parent's computed value) except that an inherited value of start or
    ///   end is interpreted against the parent's direction value and results
    ///   in a computed value of either left or right." Implementation:
    ///   [`crate::property::resolve_text_align_match_parent`], shared with the
    ///   element path's [`crate::specified::SpecifiedValues::finalize`].
    ///
    ///   ⚠️ **Trap avoided here, stated for anyone touching this arm again**:
    ///   the same value definition's second sentence — "Computes to `start`
    ///   when specified on the root element" — does **not** apply to the page
    ///   context. §6 says "The page context *inherits from* the root
    ///   element" — inheriting from the root element does not make the page
    ///   context *be* the root element, so `@page { text-align: match-parent }`
    ///   is always resolved via the parent-direction table, never
    ///   short-circuited to `start`. (The element path's analogous
    ///   short-circuit lives in
    ///   [`crate::specified::SpecifiedValues::finalize_as_root`], which is a
    ///   different entry point entirely — `cascade_page` never calls it.)
    ///
    /// **Phase 3** — absolutization against the page context's *own*
    /// `font-size` (`absolutize_in_page_context`, raikiri-spike-sshp):
    ///
    /// - `padding` / `margin` / `width` / `height` / `border-*-width` /
    ///   `line-height` — [`Length::Em`] / `Rem` /
    ///   `Pt` become [`Length::Px`]. `em` is
    ///   relative to "the font associated with their context" (§6 above), which
    ///   is the page context's own `font-size` — possibly declared by a sibling
    ///   `@page` declaration, hence a phase of its own. `rem` stays relative to
    ///   the root element (CSS Values 4 §6.1.1
    ///   <https://www.w3.org/TR/css-values-4/#rem>).
    /// - `border-*-width` is additionally **gated to `0px` when the side's
    ///   computed `border-*-style` is `none` or `hidden`** — CSS Backgrounds 3
    ///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width> makes that
    ///   part of the *computed* value ("Computed value: absolute length, snapped
    ///   as a border width; zero if the border style is `none` or `hidden`").
    ///   An **undeclared** `border-*-style` counts as its initial value `none`
    ///   — CSS Backgrounds 3 §3.2
    ///   <https://www.w3.org/TR/css-backgrounds-3/#border-style> propdef gives
    ///   `border-*-style`'s `Initial: none`, and §6 (a *different* sentence
    ///   from the ones quoted above for `font-size` and `em`) is why that
    ///   initial value is there to begin with even though the property
    ///   is undeclared — verbatim: "both the page context and the margin
    ///   context have a computed value for every property, even if that
    ///   property does not apply to the page or page-margin box." So
    ///   `@page { border-top-width: 5px }` alone computes to `0px`, matching
    ///   the element path.
    /// - `<percentage>` on `padding` / `margin` / `width` / `height` **stays**
    ///   [`Length::Percent`]. The governing rule is the general one — CSS
    ///   Values 4 §5.5.1 "Computation and Combination of `<percentage>`"
    ///   (<https://www.w3.org/TR/css-values-4/#combine-percentages>): "Unless
    ///   otherwise specified (such as in font-size, which computes its
    ///   `<percentage>` values to `<length>`), the computed value of a
    ///   percentage is the specified percentage." For `padding` / `margin`
    ///   specifically, §6 adds that it is relative to "the dimensions of the
    ///   containing block", i.e. a used-value input (§6 treats `width` /
    ///   `height` in a *separate* sentence about used-value computation rules,
    ///   so do not attribute those two to the margin/padding sentence). Either
    ///   way that is the computed value, not an unresolved one; the element
    ///   path resolves it the same way
    ///   ([`crate::resolve::resolve_length_percentage`]).
    ///
    ///   ⚠️ **`font-size` and `line-height` are the opposite case** — their
    ///   `<percentage>` does *not* survive into the computed layer (§5.5.1's
    ///   own parenthetical names `font-size`; CSS Inline 3 §5.1
    ///   <https://www.w3.org/TR/css-inline-3/#line-height-property> gives
    ///   `line-height` "Percentages: computed relative to 1em"). They are
    ///   handled by phase 2 and phase 3 respectively.
    ///
    /// See [`crate::cascade::resolve_against_inherited`] and
    /// [`absolutize_in_page_context`] for the exact per-phase contracts.
    ///
    /// **This map is not a full computed-value bag.** It contains only the
    /// properties that some matching `@page` rule actually *declared*;
    /// properties whose value would come purely from inheritance or from the
    /// initial value are **absent**. The spec sentence quoted above ("a computed
    /// value for every property") describes the page context as a whole, not
    /// this map — materialising the complete bag is the downstream page-layout
    /// consumer's job, and it starts from the resolved [`PageInheritance`]
    /// plus these declarations.
    ///
    /// # Non-finite values pass through unguarded (bd raikiri-spike-kj2s)
    ///
    /// The absolutization in phase 2 / phase 3 above is IEEE 754 `f32`
    /// arithmetic over untrusted author input. CSS Values 4 §5 "Numeric Data
    /// Types" (<https://www.w3.org/TR/css-values-4/#numeric-types>) requires
    /// out-of-range values to be "converted to the closest value supported by
    /// the implementation" — it does **not** require the result to be finite.
    /// A declaration such as `html { font-size: 0px }` with
    /// `@page { font-size: 1e40em }` overflows the literal `1e40` to
    /// `f32::INFINITY` at parse time; phase 2's `parent_font_size.0 * v`
    /// (`0.0 * inf`) then yields `f32::NAN` per IEEE 754. See
    /// `page::tests::cascade_page_font_size_can_carry_nan_from_pathological_em`
    /// for the pinned reproducer.
    ///
    /// That `1e40em` example's *parse-time* overflow-to-`f32::INFINITY` step
    /// is itself disputed: bd raikiri-spike-9mbo (open, `blocked/human`)
    /// argues that CSS Values 4 §5's "closest value" wording may require
    /// saturating to `f32::MAX` instead, which is a question about
    /// cssparser's `f64`→`f32` cast, not about this crate. The hazard this
    /// section documents does not depend on how that resolves: two already
    /// finite operands can still overflow to infinity on the `*` in phase 2
    /// alone (e.g. a `1e30px` root font-size times a `1e20em` page
    /// declaration), with no contested cast anywhere in the chain. See
    /// `page::tests::cascade_page_font_size_can_carry_infinity_from_finite_operand_multiply`
    /// for that arithmetic-only reproducer, which stays valid regardless of
    /// how 9mbo is resolved.
    ///
    /// **This map does not filter that out**, and neither does
    /// [`ComputedValues`] — the element path's equivalent computed-value bag —
    /// which documents no finiteness contract either. That is not an
    /// oversight: `raikiri-style` applies no `is_finite` / `is_nan` check
    /// anywhere in parsing, cascade, or resolution (grep the crate to
    /// confirm), and the element path's guard against non-finite geometry
    /// lives entirely outside this crate, in `raikiri-dom`'s layout module
    /// (`sanitize_taffy`, decided by bd raikiri-spike-2ui0's PMO ruling:
    /// **the guard belongs at the sink that consumes the computed-value bag,
    /// not at the parse/resolve layer that produces it**). This map is the
    /// same kind of computed-value bag, so the same precedent applies to it.
    ///
    /// No sink for `declarations` exists yet — the consumer that would read
    /// this map (page-margin-box layout, mirroring `apply_computed_to_style`
    /// for the element path) is M4+ scope, and `raikiri-style` currently has
    /// zero consumers of this field outside this crate: grepping the type
    /// name `PageCascadeResult` (not `.declarations`, which `PageRule` and
    /// `StyleRule` also expose as an unrelated field/accessor name) finds
    /// only doc-comment *mentions* elsewhere (`raikiri-dom`, and an umbrella
    /// comment noting that `cascade_page` itself is not re-exported), never
    /// an actual field read, as of this writing. The hazard is therefore
    /// real but currently unreachable; **when the M4 sink is built, its
    /// bridge must add the equivalent guard**, following the same precedent
    /// rather than adding one here.
    ///
    /// # Why this is a method, not a field (bd raikiri-spike-dyxj)
    ///
    /// Before raikiri-spike-dyxj, `declarations` was a `pub` field. The
    /// `#[non_exhaustive]` on this type already blocked external struct-literal
    /// construction, but a `pub` field leaves one hole open regardless:
    /// `let mut r = PageCascadeResult::default(); r.declarations.insert(key,
    /// raw_specified_value)` — parking an unresolved specified-layer value in
    /// the map without going through [`cascade_page`] at all. That is the same
    /// *class* of defect as the raikiri-spike-ygl0 regression this whole doc
    /// pins against, just reached by direct field-write instead of a cascade
    /// bug, so the field is now private and this accessor is the only read
    /// path — the "resolved against the inheritance parent" contract is
    /// enforced by construction (only `cascade_page`, in this module, can
    /// populate the field) rather than by convention.
    ///
    /// ## Compile-fail pin (bd raikiri-spike-ejia precedent)
    ///
    /// "The field is private" is a claim about what does *not* compile, which
    /// no ordinary (must-pass) doctest can pin — the same gap
    /// bd raikiri-spike-ejia closed for `Declaration::value` /
    /// `StyleRule::declarations` / `RuleTree::style_rules` on the same day.
    /// `PageCascadeResult::default()` returns an owned, mutable value (the
    /// type derives [`Default`]), so unlike `Declaration::value` (reached only
    /// through a `&Declaration` behind an accessor) this needs no `.clone()`
    /// detour to discriminate the field's own visibility — direct field
    /// access on an owned binding is exactly what `pub`/private gates, with no
    /// second field's visibility able to confound the result:
    ///
    /// ```compile_fail
    /// use raikiri_style::{CssColor, PageCascadeResult, PropertyKey, PropertyValue};
    ///
    /// let mut r = PageCascadeResult::default();
    /// r.declarations.insert(PropertyKey::Color, PropertyValue::Color(CssColor::BLACK));
    /// ```
    ///
    /// Verified non-vacuous the same way as the ejia precedent: temporarily
    /// restoring `pub` on the field makes the fence above compile (and thus
    /// makes `cargo test --doc` fail on it), confirming the fence fails
    /// *because* the field is private and not for some unrelated reason.
    ///
    /// ### Non-vacuous control
    ///
    /// (Same vocabulary as `raikiri-paint`'s
    /// `draw_glyphs_at_natural_font_size_is_a_non_vacuous_control` and this
    /// crate's `page::tests::specified_layer_residue_detector_is_not_vacuous`.)
    ///
    /// The fence above depends on ingredients that have nothing to do with
    /// `declarations`'s visibility — `PropertyValue::Color`'s payload shape
    /// and `CssColor::BLACK` staying valid names — plus the legitimate read
    /// path, `declarations()`, actually returning what [`cascade_page`]
    /// produced. If any of those drift (rename, shape change), the fence
    /// above would keep compile-failing for the wrong reason and the pin
    /// would go silently vacuous. This ordinary (must-compile-and-pass)
    /// doctest exercises the same ingredients through the accessor, so a
    /// drift breaks it first, not the fence:
    ///
    /// ```
    /// use raikiri_style::{
    ///     CssColor, Origin, PageContextQuery, PageInheritance, PropertyKey, PropertyValue,
    ///     RuleTree, cascade_page,
    /// };
    ///
    /// let mut tree = RuleTree::empty();
    /// tree.add_stylesheet("@page { color: red }", Origin::Author);
    /// let result = cascade_page(
    ///     &tree,
    ///     &PageContextQuery::default(),
    ///     PageInheritance::LegacyInitialValues,
    /// );
    /// assert_eq!(
    ///     result.declarations().get(&PropertyKey::Color),
    ///     Some(&PropertyValue::Color(CssColor {
    ///         r: 255,
    ///         g: 0,
    ///         b: 0,
    ///         a: 255
    ///     })),
    /// );
    /// // Ingredient used by the compile_fail fence above, confirmed still
    /// // valid here.
    /// let _ = PropertyValue::Color(CssColor::BLACK);
    /// ```
    pub fn declarations(&self) -> &HashMap<PropertyKey, PropertyValue> {
        &self.declarations
    }
}

/// Selects the page context's inheritance parent for [`cascade_page`] — CSS
/// Paged Media 3 §6 "Page Properties"
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) states: "As with
/// elements in the document, both the page context and the margin context have
/// a computed value for every property … The normal rules for CSS properties
/// apply with the following exceptions: page-margin boxes inherit from the page
/// context. The page context inherits from the root element."
///
/// Before bd raikiri-spike-mnvr this argument was `Option<&ComputedValues>`,
/// and `None` was accepted with no compile error, warning, or lint marking
/// the choice — bd raikiri-spike-ygl0 §8.2 debt lens D3 (CONFIRMED, latent)
/// found 27 of the 29 call sites existing at that time reached the L3 legacy
/// exception this way, by omission rather than by a deliberate choice, which
/// silently resolves inherited properties against the initial values instead
/// of the root element. Replacing the `Option` with this named 2-variant enum
/// makes that choice a call-site-visible decision instead of something
/// reachable by omission.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum PageInheritance<'a> {
    /// Resolve against the root element's own [`ComputedValues`] — the §6
    /// rule quoted above. **Prefer this variant** wherever a root style is
    /// available:
    ///
    /// ```ignore
    /// // ignore: `dom` / `rule_tree` / `query` are the caller's real values,
    /// // not constructible in a doc-test in isolation; shape only, see
    /// // `cascade_page`'s own doc-tests for a version that actually runs.
    /// let cascaded = cascade(dom, rule_tree)?;
    /// let result = cascade_page(
    ///     rule_tree,
    ///     &query,
    ///     PageInheritance::FromRoot(&cascaded.computed[dom.root_id().0 as usize]),
    /// );
    /// ```
    FromRoot(&'a ComputedValues),
    /// The L3 *legacy exception* the same §6 paragraph grants: "since the
    /// previous revision of CSS Paged Media Level 3 did not specify this
    /// point, an implementation that sets inherited properties in the page
    /// context to their initial values (as for the root element) is also
    /// conformant to CSS Paged Media Level 3." Resolves against
    /// [`ComputedValues::initial()`] — for `font-weight` that is `400`, not
    /// the root element's computed weight.
    ///
    /// The spec states this exception will be removed in Level 4. Choosing
    /// this variant where a root style could have been supplied instead
    /// silently resolves `bolder` / `lighter` against `400` instead of the
    /// root's weight. Since bd raikiri-spike-sshp wired phase 3 through the
    /// same `font-size` basis, it also affects `em` / `rem` lengths in the
    /// page box (`padding` / `margin` / `width` / `height` /
    /// `border-*-width` / `line-height`) — but not uniformly: the `rem`
    /// basis is always the initial `16px` under this variant, and so is the
    /// `em` basis *only when the page context declares no `font-size` of its
    /// own* ([`page_context_font_size`]). An explicit
    /// `@page { font-size: … }` still determines the `em` basis regardless
    /// of this variant (`cascade_page_padding_em_uses_own_font_size_over_inherited`
    /// pins this).
    LegacyInitialValues,
}

/// Cascade all `@page` rules in `rule_tree` against `query` and return the
/// resolved winning declarations.
///
/// # Algorithm (spec §"Cascading and page context" + §"Cascade Origin")
///
/// 1. For each rule in [`RuleTree::page_rules`], find the highest-specificity
///    matching entry in its comma-separated `<page-selector-list>` prelude
///    (each entry is an OR alternative; the entries within an entry — an
///    optional ident + zero-or-more pseudo-pages — are combined AND-wise).
///    A rule contributes its declarations if any entry matched, tagged with
///    that entry's `(f, g, h)` specificity triple (see the crate-internal
///    `PageSpecificity` type for the exact shape).
/// 2. Group candidate declarations by [`PropertyKey`], pick the winner per
///    `(rank, specificity, source_order)` tuple where higher beats lower.
///    `rank` comes from the crate-internal `cascade_rank` (shared with the
///    style-rule cascade) and encodes the L4 §6.2 origin ordering (Normal:
///    UA < Author; Important: reversed — UA `!important` beats Author
///    `!important`).
/// 3. **Phase 2** — resolve each winner against the page context's inheritance
///    parent ([`PageInheritance`]) through the crate-internal
///    [`crate::cascade::resolve_against_inherited`], the sibling of
///    [`crate::cascade::apply_value`] that
///    keeps the `PropertyValue` shape. This covers the two properties that are
///    determined by the inheritance parent alone: `font-weight: bolder` /
///    `lighter` (CSS Fonts 4 §2.2.1 "Relative Weights") and `font-size`.
/// 4. **Phase 3** — absolutize the remaining lengths through the crate-internal
///    `absolutize_in_page_context`, using the page context's own `font-size`
///    (step 3's output, or the inherited value when `@page` declared none) as
///    the `em` basis and the root element's font-size as the `rem` basis, and
///    apply the `border-*-width` style gate. Mirrors the element path's
///    [`crate::specified::SpecifiedValues::finalize`]; the split into two phases
///    is required because `padding: 2em` depends on a sibling declaration whose
///    winner is only known after step 2 (decision raikiri-spike-082k).
///
/// The result is a bag of **computed** values —
/// [`PageCascadeResult::declarations`] documents the one remaining exception
/// (`text-align: match-parent`) and the per-phase citations.
///
/// # The inheritance parent
///
/// See [`PageInheritance`] for the two named choices, their spec basis, and
/// which one to prefer — that is what step 3 above resolves winners against.
///
/// # Compile-fail pin (bd raikiri-spike-mnvr)
///
/// Before bd raikiri-spike-mnvr the third parameter was
/// `Option<&ComputedValues>` and a bare `None` literal compiled silently —
/// see [`PageInheritance`]'s doc for the history. The parameter's type is now
/// [`PageInheritance`] itself (not `impl Into<PageInheritance>`, which would
/// let `None` convert implicitly if a `From<Option<_>>` impl existed) and
/// [`PageInheritance`] has no `Default` impl, so there is no value `None`
/// could coerce or default into — this is a type error, not a silent
/// default:
///
/// ```compile_fail
/// use raikiri_style::{Origin, PageContextQuery, RuleTree, cascade_page};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page { color: red }", Origin::Author);
/// let _ = cascade_page(&tree, &PageContextQuery::default(), None);
/// ```
///
/// Non-vacuous control: swapping `None` above for
/// [`PageInheritance::LegacyInitialValues`] is exactly
/// [`PageCascadeResult::declarations`]'s own doctest, which compiles and
/// passes — confirming the failure above is caused by the removed implicit
/// `None` path and not by an unrelated mistake in the fence.
///
/// # Ties on equal `(rank, specificity, source_order)`
///
/// The spec text at the anchor says @page cascade follows normal cascade
/// tie-breaking. Within a single rule, later declarations of the same
/// property win per CSS Cascading L4 §6.1
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of Appearance";
/// the `>=` in the crate-internal `page_beats` mirrors the style-rule
/// sibling's tie-break (`cascade::beats` uses `>=` on the same triple).
///
/// # Example
///
/// ```ignore
/// use raikiri_style::{Origin, PageContextQuery, PageInheritance, RuleTree, cascade_page};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page :first { color: red }", Origin::Author);
/// let query = PageContextQuery { is_first: true, ..Default::default() };
/// // Real callers pass `PageInheritance::FromRoot(root_style)` — see
/// // `PageInheritance`'s doc for the copy-pasteable form. The legacy variant
/// // appears here only because this snippet has no document to cascade.
/// let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
/// // result.declarations() contains one entry: PropertyKey::Color -> red
/// ```
pub fn cascade_page(
    rule_tree: &RuleTree,
    query: &PageContextQuery,
    inheritance: PageInheritance<'_>,
) -> PageCascadeResult {
    // Candidate: (value, important, origin, specificity, source_order).
    // Shape mirrors `cascade::CascadedDecl` per sibling convention (37n), with
    // `PageSpecificity` in place of `selectors`-crate `Specificity`.
    let mut candidates: Vec<(PropertyValue, bool, Origin, PageSpecificity, u32)> = Vec::new();
    for rule in &rule_tree.page_rules {
        // Comma-separated list = OR: rule contributes if any entry matches.
        // Take the highest-specificity matching entry within this rule (spec
        // examples in §"Cascading and page context" show the (f,g,h) triple
        // determines the effective specificity of the whole rule).
        let mut best_spec: Option<PageSpecificity> = None;
        for entry in &rule.selector.entries {
            if let Some(spec) = match_page_entry(entry, query) {
                best_spec = Some(match best_spec {
                    Some(prev) => prev.max(spec),
                    None => spec,
                });
            }
        }
        if let Some(spec) = best_spec {
            for decl in &rule.declarations {
                // Expand shorthands into longhands before pushing candidates —
                // the parse-time expansion alone does not cover the post-parse
                // mutation path through the `pub` field `PageRule::declarations`
                // (bd raikiri-spike-3svx; the element-path sibling is
                // `crate::cascade`'s `collect_cascaded`, bd raikiri-spike-nqkj).
                // Rationale is consolidated in `crate::rule::expand_shorthand_into`.
                expand_shorthand_into(decl, |d| {
                    candidates.push((d.value, d.important, rule.origin, spec, rule.source_order));
                });
            }
        }
    }

    // Winner selection — sibling arm to `cascade::pick_winners`.
    let mut best: HashMap<PropertyKey, (u8, PageSpecificity, u32, PropertyValue)> = HashMap::new();
    for (value, important, origin, spec, order) in candidates {
        let rank = cascade_rank(origin, important);
        let key = value.key();
        let candidate = (rank, spec, order, value);
        match best.get(&key) {
            Some(existing) => {
                if page_beats(&candidate, existing) {
                    best.insert(key, candidate);
                }
            }
            None => {
                best.insert(key, candidate);
            }
        }
    }
    // Step 3 (phase 2): resolve winners against the page context's inheritance
    // parent. `PageInheritance::LegacyInitialValues` falls back to the initial
    // values, which the L3 legacy exception in CSS Paged Media 3 §6 "Page
    // Properties" permits explicitly (see `PageInheritance`'s doc). Shared
    // static: the fallback is immutable and `initial()` costs a heap allocation,
    // which the `LegacyInitialValues` path would otherwise take on every call
    // (`property.rs` の `empty_counter_entries` と同じ前例)。
    static INITIAL_PAGE_PARENT: LazyLock<ComputedValues> = LazyLock::new(ComputedValues::initial);
    let inherited: &ComputedValues = match inheritance {
        PageInheritance::FromRoot(root) => root,
        PageInheritance::LegacyInitialValues => &INITIAL_PAGE_PARENT,
    };
    // `ctx` carries `inherited`'s used line-height as `root_line_height` (bd
    // raikiri-spike-yh3w) — needed by *both* step 3 below (`resolve_against_inherited`'s
    // `FontSize` arm, for `font-size: 1lh`/`1rlh`'s self-reference basis) and
    // step 4 (phase 3, for `padding: 1lh` etc.'s basis via `own_line_height`).
    // Built once here rather than separately in each step: `inherited` is
    // immutable for the whole function, so the two steps would otherwise
    // compute the exact same value twice.
    //
    // `rlh` (bd raikiri-spike-vxha) always refers to the *root element's* own
    // `lh`, never the page context's — CSS Paged Media 3 §6 "The page context
    // inherits from the root element", so `inherited` (the root element's
    // `ComputedValues`, or the L3 legacy initial-values fallback) is the right
    // source, unconditionally, regardless of what the page context's own
    // `line-height` declares. `resolve_against_inherited`'s `FontSize` arm
    // relies on the same fact for `font-size: 1lh`'s self-reference basis —
    // for the page context specifically, "parent" and "root" coincide (both
    // are `inherited`), so the one `ctx.root_line_height` serves both.
    let ctx = ResolveContext::with_root_line_height(
        inherited.font_size,
        used_line_height_length(inherited.line_height, inherited.font_size),
    );
    let resolved: HashMap<PropertyKey, ResolvedAgainstInherited> = best
        .into_iter()
        .map(|(k, (_, _, _, v))| (k, resolve_against_inherited(v, inherited, &ctx)))
        .collect();

    // Step 4 (phase 3): absolutize the remaining lengths against the page
    // context's own font-size and apply the `border-*-width` style gate.
    // Splitting this out of step 3 is forced by the same constraint the element
    // path has (decision raikiri-spike-082k): `padding: 2em` needs the page
    // context's font-size, which step 3 only finalises once *all* winners have
    // been seen — `best` iteration order is `HashMap`-random.
    let font_size = page_context_font_size(&resolved, inherited);
    // `rem` = "the computed value of the em unit on the root element"
    // (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>). The page
    // context is *not* the root element, so this stays the inheritance parent's
    // font-size even after `font_size` above diverges from it — the element-path
    // sibling is `SpecifiedValues::finalize`, not `finalize_as_root`. `ctx`
    // (built above, before step 3) already carries this basis.
    let border_styles = page_context_border_styles(&resolved);
    // The page context's own `lh` basis (bd raikiri-spike-vxha) — mirrors
    // `font_size` above: `1lh` in `padding`/`margin`/`border-*-width` needs
    // the page context's *own* resolved line-height, not the root's.
    let own_line_height = page_context_line_height_basis(&resolved, inherited, font_size, &ctx);
    PageCascadeResult {
        declarations: resolved
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    absolutize_in_page_context(v, font_size, own_line_height, &ctx, border_styles),
                )
            })
            .collect(),
    }
}

/// The page context's own computed `font-size` — the `em` basis for phase 3.
///
/// CSS Paged Media 3 §6 "Page Properties"
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) verbatim: "Values in
/// units of em and ex are interpreted relative to the font associated with
/// their context." The font associated with the page context is the one its own
/// `font-size` declaration establishes; with no such declaration the same §
/// gives the inherited value ("The page context inherits from the root
/// element", plus "both the page context and the margin context have a computed
/// value for every property").
///
/// `declarations` must already have been through phase 2
/// ([`crate::cascade::resolve_against_inherited`]) — since bd raikiri-spike-7m33
/// this is enforced by the parameter type itself, not merely documented; see
/// [`crate::cascade::ResolvedAgainstInherited`] for the exact scope of that
/// guarantee. Its `FontSize` arm always wraps its result in [`Length::Px`].
/// **That invariant is what makes the read below total**: any other `Length` variant under
/// `PropertyKey::FontSize` is unreachable, and the catch-all falls back to the
/// inherited value rather than panicking (crate policy: no panic surface in
/// the cascade — see the `PropertyValue::Margin` fall-through arm in
/// [`crate::cascade::apply_value`]). If that arm ever stops normalising to
/// `Px`, this function silently starts using the wrong basis, so the two must
/// be changed together.
fn page_context_font_size(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
    inherited: &ComputedValues,
) -> ComputedLength {
    match declarations
        .get(&PropertyKey::FontSize)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::FontSize(Length::Px(px))) => ComputedLength(*px),
        other => {
            // cov:ignore: the `Some(non-`Px`)` half is unreachable — phase 2's
            // `FontSize` arm wraps its result in `Length::Px` unconditionally.
            // The `None` half *is* covered, by
            // `page_context_font_size_falls_back_to_inherited`.
            //
            // The assert exists because the silent-fallback behaviour must not
            // be relied on: if someone adds `font-size: larger | smaller` and
            // stops normalising to `Px`, the page context would quietly use the
            // root's font-size as the `em` basis and every length in the page
            // box would be wrong, with every test still green.
            debug_assert!(
                other.is_none(),
                "phase 2 must normalise `font-size` to `Length::Px`; got {other:?}"
            );
            inherited.font_size
        }
    }
}

/// The page context's own `lh` basis — the resolve basis for `1lh` used in
/// `padding`/`margin`/`border-*-width` in phase 3 (bd raikiri-spike-vxha).
///
/// Mirrors [`page_context_font_size`]'s shape: an undeclared `line-height`
/// falls back to `inherited.line_height` (CSS Paged Media 3 §6 "The page
/// context inherits from the root element", plus "both the page context and
/// the margin context have a computed value for every property" — line-height
/// is an inherited property so the fallback is the inheritance parent's
/// computed value, not the property's own initial `normal`). A declared
/// `line-height` is resolved through [`resolve_line_height`], passing the
/// root element as the page context's self-reference "parent" (`self_reference_parent`
/// local below — CSS Values 4 §6.1.1's self-reference clause, canonically
/// documented on [`resolve_line_height`]; only its `Length::Lh` arm actually
/// reads this argument, `Length::Rlh` reads `ctx.root_line_height` directly
/// regardless of what is passed here, so the two happen to coincide for the
/// page context specifically).
///
/// # Which font-size does an inherited `<number>` multiply by?
///
/// Both branches — declared and undeclared — convert the resulting
/// [`crate::resolve::ComputedLineHeight`] with **this call's own `font_size`
/// argument** (the page context's own, from [`page_context_font_size`]),
/// never the root's — ordinary CSS inheritance semantics for the unitless
/// multiplier ([`crate::resolve::ComputedLineHeight`] doc, "子は number を
/// inherit して自分の font-size に掛ける"), not a page-context-specific
/// carve-out — pinned by
/// `cascade_page_padding_lh_uses_page_context_own_font_size_for_inherited_number`.
///
/// The result is then converted to an absolute length via
/// [`used_line_height_length`], returning `None` when unresolvable (`normal`
/// with no font metrics — the same wall as `cap`/`rcap`).
fn page_context_line_height_basis(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
    inherited: &ComputedValues,
    font_size: ComputedLength,
    ctx: &ResolveContext,
) -> Option<ComputedLength> {
    // CSS Paged Media 3 §6: the page context's self-reference "parent" (CSS
    // Values 4 §6.1.1) is the root element.
    let self_reference_parent = ctx.root_line_height;
    let line_height = match declarations
        .get(&PropertyKey::LineHeight)
        .map(ResolvedAgainstInherited::as_property_value)
    {
        Some(PropertyValue::LineHeight(lh)) => {
            resolve_line_height(*lh, font_size, self_reference_parent, ctx)
        }
        _ => inherited.line_height,
    };
    used_line_height_length(line_height, font_size)
}

/// The page context's computed `border-*-style` per side — the gate input for
/// `border-*-width` in phase 3.
///
/// An **absent** declaration means the initial value: CSS Paged Media 3 §6
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) gives the page context
/// "a computed value for every property", and `border-style` is not inherited
/// (CSS Backgrounds 3 §3.2
/// <https://www.w3.org/TR/css-backgrounds-3/#border-style>), so the computed
/// value of an undeclared side is [`INITIAL_BORDER`]`.style` = `none`.
///
/// The consequence is load-bearing and matches the element path: `@page {
/// border-top-width: 5px }` **on its own** computes to `0px`, exactly as
/// `ComputedValues::initial().border.top.width()` is `0px` for an element that
/// declares no `border-style` (pin:
/// `specified::tests::initial_border_width_is_gated_to_zero_at_computed_layer`).
fn page_context_border_styles(
    declarations: &HashMap<PropertyKey, ResolvedAgainstInherited>,
) -> Sides<BorderStyle> {
    // **Dispatch on the variant, never on the key.** A `PropertyKey` lookup
    // followed by a payload match would make correctness depend on the map's
    // keying invariant (`PropertyValue::key()` agreeing with the key it is
    // stored under) — an invariant that lives only in prose, and whose
    // violation would degrade silently to the initial value. Scanning the
    // values makes the invariant irrelevant: each arm names the side it writes
    // (bd raikiri-spike-sshp §8.2 debt lens D3).
    //
    // Order-independence: `cascade_page` keeps at most one winner per
    // `PropertyKey`, so at most one value matches each arm and the
    // `HashMap`-random iteration order cannot change the result.
    //
    // The `_` arm here is a *filter*, not a classification — it does not weaken
    // the variant-addition tripwire, which lives in
    // `absolutize_in_page_context`: a new `border-*-width`-like variant fails to
    // compile there first, and fixing it forces the author back through this
    // function.
    let mut sides = Sides::all(INITIAL_BORDER.style);
    for value in declarations.values() {
        match value.as_property_value() {
            PropertyValue::BorderTopStyle(s) => sides.top = *s,
            PropertyValue::BorderRightStyle(s) => sides.right = *s,
            PropertyValue::BorderBottomStyle(s) => sides.bottom = *s,
            PropertyValue::BorderLeftStyle(s) => sides.left = *s,
            _ => {}
        }
    }
    sides
}

/// **phase 3** for the page context — absolutize one winner against the page
/// context's own `font-size` and apply the `border-*-width` style gate.
///
/// Sibling of the element path's [`crate::specified::SpecifiedValues::finalize`]
/// second half (`absolutize_with`). Both funnel into the *same*
/// [`crate::resolve`] functions, so the spec rules (`em` / `rem` basis,
/// percentage pass-through, border style gating) have a single implementation
/// per rule; only the plumbing differs, because the page path carries a
/// `PropertyValue` bag instead of a typed struct.
///
/// # What this function does not resolve, and why that is fine
///
/// `<percentage>` on the box properties stays a percentage — it is a
/// used-value-layer input (CSS Values 4 §5.5.1), not a phase-3 concern. The
/// spec citation lives in [`PageCascadeResult::declarations`] (canonical).
///
/// `text-align: match-parent` is **not** handled here either, but for a
/// different reason than it used to be (raikiri-spike-l3wg): it is now fully
/// resolved by **phase 2**
/// ([`crate::cascade::resolve_against_inherited`], which runs before this
/// function) — inherited-value dependence is phase 2's shape, not phase 3's,
/// so by the time a value reaches this function `TextAlign::MatchParent`
/// should never appear. It stays in the pass-through arm below (alongside
/// `Color` / `Display` / …) simply because, once resolved, `text-align`'s
/// computed value carries no length for phase 3 to touch — same as before,
/// just for a different underlying reason. `direction` joins the same arm as
/// a new, ordinary computed-equivalent keyword (CSS Writing Modes 4 §2.1,
/// computed value = specified value, no phase-2 or phase-3 work at all).
///
/// # No wildcard arm
///
/// The match is exhaustive without `_`, like its two siblings
/// ([`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]).
/// A new [`PropertyValue`] variant that carries a length must be classified
/// here explicitly; a catch-all would let it reach the public `declarations`
/// map as a specified value — the exact regression shape of bd
/// raikiri-spike-ygl0. (What this guard does *not* catch is a new **payload**
/// case inside an existing variant — bd raikiri-spike-7m33 gap (a).)
///
/// # The `value` parameter is phase-2 output, enforced by its type
///
/// Since bd raikiri-spike-7m33, `value` is a
/// [`crate::cascade::ResolvedAgainstInherited`]
/// rather than a raw [`PropertyValue`] — see that type's doc for what this
/// does and does not guarantee ("narrowed, not closed").
///
/// # `own_line_height` (bd raikiri-spike-vxha)
///
/// The page context's own `lh` basis — [`page_context_line_height_basis`]'s
/// output, threaded alongside `font_size` for the same reason: `1lh` in
/// `padding`/`margin`/`border-*-width` needs this context's *own* resolved
/// line-height (not the root's — that is `ctx.root_line_height`, used only
/// for `rlh`).
fn absolutize_in_page_context(
    value: ResolvedAgainstInherited,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
    border_styles: Sides<BorderStyle>,
) -> PropertyValue {
    let value = value.into_property_value();
    /// `<length-percentage>` → computed, mapped back into the specified-layer
    /// `Length` shape that `PropertyValue` carries (`Px` / `Percent` only —
    /// `Em` / `Rem` / `Pt` are gone after this).
    fn lp(
        specified: Length,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Length {
        match resolve_length_percentage(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentage::Px(v) => Length::Px(v),
            ComputedLengthPercentage::Percent(p) => Length::Percent(p),
        }
    }
    /// `<length-percentage> | auto` — same mapping, `auto` preserved.
    fn lpa(
        specified: LengthOrAuto,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> LengthOrAuto {
        match resolve_length_percentage_or_auto(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
        }
    }
    /// `<length-percentage> | auto` for `margin-*` specifically (roborev-refine
    /// iter 1 Finding A, bd raikiri-spike-vxha) — same mapping as [`lpa`] but
    /// routed through [`resolve_margin_length_or_auto`], whose unresolvable-`lh`/
    /// `rlh` fallback is `Px(0.0)` (margin's true spec initial), not `Auto`
    /// (which is `width`/`height`'s initial, and which would trigger real
    /// auto-margin layout behaviour if used here — see that function's doc).
    fn margin_lpa(
        specified: LengthOrAuto,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> LengthOrAuto {
        match resolve_margin_length_or_auto(specified, font_size, own_line_height, ctx) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
        }
    }
    /// One `border-*` side: absolutize the width and apply the style gate.
    ///
    /// Routed through [`resolve_border`] rather than re-testing
    /// `none` / `hidden` locally — that function's doc calls itself the single
    /// source of the gate and forbids re-implementing the rule elsewhere.
    fn border(
        specified: Border,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Border {
        let computed = resolve_border(specified, font_size, own_line_height, ctx);
        Border {
            width: Length::Px(computed.width.px()),
            style: computed.style,
            color: computed.color,
        }
    }
    /// A `border-*-width` longhand, gated by the side's computed style.
    ///
    /// `color` is a placeholder — [`resolve_border`] never reads it, and the
    /// longhand carries no colour. Building the `Border` here (instead of
    /// branching on `style` locally) is what keeps the gate single-sourced.
    fn border_width(
        width: Length,
        style: BorderStyle,
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &ResolveContext,
    ) -> Length {
        border(
            Border {
                width,
                style,
                color: BorderColor::CurrentColor,
            },
            font_size,
            own_line_height,
            ctx,
        )
        .width
    }

    match value {
        // ── already computed-equivalent after phase 2 ──────────────────────
        // `font-size` is phase 2's output (`Length::Px`); re-absolutizing it
        // here would be a second application against the *wrong* basis (its
        // own value instead of the inheritance parent's).
        v @ (PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::FontSize(_)
        | PropertyValue::FontWeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::Direction(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::BoxSizing(_)) => v,
        // ── font-size: larger / smaller (raikiri-spike-4rmu) ────────────────
        // ⚠️ **structurally unreachable through `cascade_page`, not a "safety
        // net"** — step 3 (phase 2) in `cascade_page` maps *every* winner
        // through `resolve_against_inherited` before this function ever runs,
        // and that function's `FontSizeRelative` arm always converges to
        // `PropertyValue::FontSize(Length::Px(_))`. There is no second entry
        // point that could skip step 3 (unlike the shorthand fall-throughs
        // below, which *are* reachable via a direct internal call).
        //
        // The arm still exists — not folded into the `Color`/`FontSize`/…
        // bucket above, and not `unreachable!` — for the same two reasons the
        // shorthand fall-throughs keep real arms: the crate keeps the cascade
        // panic-free (reviewer-security policy, see the `Margin` fall-through
        // in `crate::cascade::apply_value`), and this function's `pub(crate)`
        // visibility means test code *can* call it directly with an
        // unresolved `FontSizeRelative`, bypassing step 3 (as
        // `absolutize_in_page_context_font_size_relative_safety_net` does).
        //
        // The basis is deliberately `font_size` (this context's own,
        // phase-2-resolved value) rather than the inheritance parent's — this
        // function has no parent basis available, and the arm is bug-only
        // regardless, so spec correctness here is not a design goal. What
        // matters is: no panic, and convergence to the same `FontSize(Px(_))`
        // shape the real (phase 2) path produces, so a caller reading
        // `declarations` never observes an unresolved `FontSizeRelative`.
        PropertyValue::FontSizeRelative(rel) => {
            PropertyValue::FontSize(Length::Px(resolve_relative_font_size(rel, font_size.px())))
        }
        // ── line-height ───────────────────────────────────────────────────
        // CSS Inline 3 §5.1 <https://www.w3.org/TR/css-inline-3/#propdef-line-height>:
        // `<percentage>` is "computed relative to 1em" of the declaring
        // context. `normal` / `<number>` survive as keywords by spec.
        //
        // `lh`/`rlh` used *within* this declaration's own value (self-reference,
        // bd raikiri-spike-vxha) use `ctx.root_line_height` — the page context's
        // CSS Values 4 §6.1.1 self-reference "parent" is the root element
        // (`page_context_line_height_basis` doc), the same source
        // `ctx.root_line_height` already carries.
        PropertyValue::LineHeight(lh) => PropertyValue::LineHeight(lift_line_height(
            resolve_line_height(lh, font_size, ctx.root_line_height, ctx),
        )),
        // ── padding ───────────────────────────────────────────────────────
        PropertyValue::PaddingTop(v) => {
            PropertyValue::PaddingTop(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingRight(v) => {
            PropertyValue::PaddingRight(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingBottom(v) => {
            PropertyValue::PaddingBottom(lp(v, font_size, own_line_height, ctx))
        }
        PropertyValue::PaddingLeft(v) => {
            PropertyValue::PaddingLeft(lp(v, font_size, own_line_height, ctx))
        }
        // Shorthand fall-through — **unreachable through `cascade_page`**.
        // `crate::rule::expand_shorthand_into` runs at both boundaries that feed
        // this function: the parse exit (`crate::rule::parse_declaration_block`,
        // which `ruletree` reuses) and the `@page` cascade entry (the candidate
        // loop in `cascade_page` itself, bd raikiri-spike-3svx — the sibling of
        // `crate::cascade`'s `collect_cascaded`, bd raikiri-spike-nqkj). The
        // entry-side expansion is what covers the post-parse mutation path
        // through the `pub` field `PageRule::declarations`.
        //
        // ⚠️ **これは "safety" net ではない — 到達したら既に bug である**
        // (element 側 bd raikiri-spike-8kn8 の framing 訂正と同旨)。到達した
        // winner は `PropertyKey::Margin` 等の独立 key に park したまま
        // absolutize され、well-formed に見える値のまま `MarginTop` を読む
        // consumer から黙って消える — bd raikiri-spike-3svx の headline failure
        // mode そのものである。degraded ではなく deterministic に CSS Cascading
        // L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand> 違反であり、
        // 本 arm はそれを穏当に見せない。
        //
        // The arms are kept rather than folded into `unreachable!` for the same
        // reason `cascade::apply_value` keeps its shorthand arms: the guarantee
        // above is only partly compile-time enforced — the carve-outs are in the
        // expansion `match`'s own doc (bd raikiri-spike-ez7b) — and the crate
        // keeps the cascade panic-free per reviewer:security policy. Behaviour
        // is pinned directly
        // by `tests::absolutize_in_page_context_shorthand_fall_throughs`.
        PropertyValue::Padding(sides) => {
            PropertyValue::Padding(sides.map(|l| lp(l, font_size, own_line_height, ctx)))
        }
        // ── margin ────────────────────────────────────────────────────────
        PropertyValue::MarginTop(v) => {
            PropertyValue::MarginTop(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginRight(v) => {
            PropertyValue::MarginRight(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginBottom(v) => {
            PropertyValue::MarginBottom(margin_lpa(v, font_size, own_line_height, ctx))
        }
        PropertyValue::MarginLeft(v) => {
            PropertyValue::MarginLeft(margin_lpa(v, font_size, own_line_height, ctx))
        }
        // Shorthand fall-through (see `Padding` above).
        PropertyValue::Margin(sides) => {
            PropertyValue::Margin(sides.map(|l| margin_lpa(l, font_size, own_line_height, ctx)))
        }
        // ── border-*-width (absolutized **and** style-gated) ──────────────
        PropertyValue::BorderTopWidth(w) => PropertyValue::BorderTopWidth(border_width(
            w,
            border_styles.top,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderRightWidth(w) => PropertyValue::BorderRightWidth(border_width(
            w,
            border_styles.right,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderBottomWidth(w) => PropertyValue::BorderBottomWidth(border_width(
            w,
            border_styles.bottom,
            font_size,
            own_line_height,
            ctx,
        )),
        PropertyValue::BorderLeftWidth(w) => PropertyValue::BorderLeftWidth(border_width(
            w,
            border_styles.left,
            font_size,
            own_line_height,
            ctx,
        )),
        // Shorthand fall-through (see `Padding` above). Each side gates on the
        // style it carries itself, which is where a `border` shorthand's style
        // lives.
        PropertyValue::Border(sides) => {
            PropertyValue::Border(sides.map(|b| border(b, font_size, own_line_height, ctx)))
        }
        // ── width / height ────────────────────────────────────────────────
        PropertyValue::Width(v) => PropertyValue::Width(lpa(v, font_size, own_line_height, ctx)),
        PropertyValue::Height(v) => PropertyValue::Height(lpa(v, font_size, own_line_height, ctx)),
    }
}

/// Page-selector specificity triple `(f, g, h)` per CSS Paged Media L3
/// §"Cascading and page context":
///
/// - `f` = count of page type names (named ident, syntactically 0 or 1)
/// - `g` = count of `:first` or `:blank` pseudo-classes
/// - `h` = count of `:left` or `:right` pseudo-classes
///
/// Compared lexicographically (`f` beats `g` beats `h`), via
/// `#[derive(Ord)]` on the field order.
///
/// Selected examples from the spec:
///
/// - `@page { }`        → `(0, 0, 0)`
/// - `@page :left { }`  → `(0, 0, 1)`
/// - `@page :first { }` → `(0, 1, 0)`
/// - `@page artsy { }`  → `(1, 0, 0)`
///
/// A `u32` per component is more than enough — `f` is bounded to 1 by the
/// grammar, and `g` / `h` in practice count `Vec<PagePseudo>` entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PageSpecificity {
    f: u32,
    g: u32,
    h: u32,
}

/// Match a single compound `<page-selector>` entry against `query`.
///
/// Returns the entry's [`PageSpecificity`] on a match, `None` otherwise.
/// Matching semantics per L3 §"@page rule grammar":
///
/// - Optional ident: if the entry has one, `query.page_name` must be `Some`
///   and byte-equal to it (`<custom-ident>` case-sensitivity, CSS Values L4
///   §4.2). An entry with no ident matches every page.
/// - Reserved-keyword exception: an ident that is `auto` under ASCII
///   case-insensitive comparison never matches. CSS Paged Media L3
///   §"Page selectors" states: "A page type name of auto (ASCII
///   case-insensitive) does not make the rule invalid, but must never
///   match." The parser therefore accepts `@page auto { … }` (the rule
///   is spec-valid syntax) and the exclusion lives here, on the cascade
///   side; see
///   <https://www.w3.org/TR/css-page-3/#page-selectors>.
/// - Zero-or-more pseudo-pages: **all** must match (compound = AND).
fn match_page_entry(
    entry: &PageSelectorEntry,
    query: &PageContextQuery,
) -> Option<PageSpecificity> {
    if let Some(ident) = &entry.ident {
        // Reserved-keyword exclusion: `auto` (ASCII case-insensitive) is
        // spec-valid but must never match — CSS Paged Media L3
        // §"Page selectors", `#page-selectors`. Applied *before* the
        // name compare so no `query.page_name` value (including a
        // literal `"auto"` custom-ident from an author `page` property)
        // can bypass the rule.
        if ident.0.eq_ignore_ascii_case("auto") {
            return None;
        }
        match &query.page_name {
            Some(name) if ident == name => {}
            _ => return None,
        }
    }
    for pseudo in &entry.pseudos {
        let ok = match pseudo {
            PagePseudo::First => query.is_first,
            PagePseudo::Left => query.is_left,
            PagePseudo::Right => query.is_right,
            PagePseudo::Blank => query.is_blank,
        };
        if !ok {
            return None;
        }
    }
    let f = if entry.ident.is_some() { 1 } else { 0 };
    let g = entry
        .pseudos
        .iter()
        .filter(|p| matches!(p, PagePseudo::First | PagePseudo::Blank))
        .count() as u32;
    let h = entry
        .pseudos
        .iter()
        .filter(|p| matches!(p, PagePseudo::Left | PagePseudo::Right))
        .count() as u32;
    Some(PageSpecificity { f, g, h })
}

/// Winner tie-break: `candidate` beats `existing` when its
/// `(rank, specificity, source_order)` triple is `>=` the existing's.
///
/// Sibling arm to `cascade::beats`. The `>=` is intentional and mirrors the
/// style-rule tie-break: within a single `@page` rule, later declarations of
/// the same property beat earlier ones (CSS Cascading L4 §6.1
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of Appearance");
/// across rules, `source_order` is monotonically increasing so `>` and `>=`
/// coincide.
fn page_beats(
    candidate: &(u8, PageSpecificity, u32, PropertyValue),
    existing: &(u8, PageSpecificity, u32, PropertyValue),
) -> bool {
    (candidate.0, candidate.1, candidate.2) >= (existing.0, existing.1, existing.2)
}

#[cfg(test)]
mod tests {
    //! Verification tests for `@page` cascade order (raikiri-spike-m4.1).
    //!
    //! Spec anchors, verified by planner + implementer (WebFetch):
    //! - CSS Paged Media L3 §"Cascading and page context" —
    //!   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
    //! - CSS Cascading L4 §"Cascade Origin" —
    //!   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
    //!
    //! Test naming mirrors `cascade::tests` (sibling convention 37n).
    //!
    //! **Note on task description Verification 2**: the parent bd task's prose
    //! says "Author !important > UA !important > Author normal > UA normal",
    //! which contradicts CSS Cascading L4 §"Cascade Origin" (Important order:
    //! Author < User < UA — UA-important wins). The parenthetical
    //! (`UA_imp=3` > `Author_imp=2`) is spec-correct and matches `cascade_rank`
    //! and the existing style-rule test
    //! `cascade::tests::important_ua_beats_important_author_display`.
    //! Implementation follows the spec; this test asserts UA `!important` wins.

    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::property::{
        BoxSizing, ContentComponent, CssColor, Direction, DisplayValue, FontWeightValue, Length,
        LengthOrAuto, LineHeight, PositionValue, TextAlign,
    };
    use crate::resolve::{ComputedLength, ComputedLineHeight};
    use std::sync::Arc;

    const RED: CssColor = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    const BLUE: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    const GREEN: CssColor = CssColor {
        r: 0,
        g: 128,
        b: 0,
        a: 255,
    };

    fn color_of(result: &PageCascadeResult) -> Option<CssColor> {
        match result.declarations().get(&PropertyKey::Color) {
            Some(PropertyValue::Color(c)) => Some(*c),
            _ => None,
        }
    }

    #[test]
    fn cascade_page_empty_rule_tree_returns_empty() {
        let tree = RuleTree::empty();
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert!(result.declarations().is_empty());
    }

    #[test]
    fn cascade_page_default_selector_matches_every_page() {
        // `@page { color: red }` has an empty prelude → matches every page.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_named_rule_does_not_apply_to_unnamed_page() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page my-cover { color: red }", Origin::Author);
        let query = PageContextQuery {
            page_name: None,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert!(color_of(&result).is_none());
    }

    // ── Verification 1: origin cascade (Author > UA for normal) ─────────────
    // Spec: CSS Cascading L4 §"Cascade Origin" — Normal author declarations
    // beat normal user-agent declarations.

    #[test]
    fn cascade_page_author_normal_beats_ua_normal() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red }", Origin::UserAgent);
        tree.add_stylesheet("@page { color: blue }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_author_normal_beats_ua_normal_regardless_of_stylesheet_add_order() {
        // Author added first, UA second — origin rank (not source order) still
        // makes Author win because UA-normal rank (0) < Author-normal rank (1).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: blue }", Origin::Author);
        tree.add_stylesheet("@page { color: red }", Origin::UserAgent);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(BLUE));
    }

    // ── Verification 2: !important reversal (UA-important > Author-important) ─
    // Spec: CSS Cascading L4 §"Cascade Origin". See module docstring for the
    // task-prose divergence note.

    #[test]
    fn cascade_page_important_ua_beats_important_author() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red !important }", Origin::UserAgent);
        tree.add_stylesheet("@page { color: blue !important }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_important_author_beats_normal_ua_and_author() {
        // rank(Author, important) = 2 > rank(Author, normal) = 1 > rank(UA, normal) = 0
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: green }", Origin::UserAgent);
        tree.add_stylesheet("@page { color: blue }", Origin::Author);
        tree.add_stylesheet("@page { color: red !important }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(RED));
    }

    // ── Verification 3: pseudo-page specificity (:first > :left) ─────────────
    // Spec: `@page :first` = (0,1,0), `@page :left` = (0,0,1). (0,1,0) > (0,0,1).

    #[test]
    fn cascade_page_first_beats_left_by_specificity() {
        // Query is both :first AND :left (first page happens to also be a
        // left/verso page). Both rules match; :first must win.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page :left { color: red } @page :first { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            is_first: true,
            is_left: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_first_beats_left_regardless_of_source_order() {
        // Reverse source order — specificity (rank tier 2) still dominates
        // source order (rank tier 3).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page :first { color: blue } @page :left { color: red }",
            Origin::Author,
        );
        let query = PageContextQuery {
            is_first: true,
            is_left: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    // ── Verification 4: named ident selectivity (named > unnamed) ────────────
    // Spec: `@page named` = (1,0,0), `@page` = (0,0,0). (1,0,0) > (0,0,0).

    #[test]
    fn cascade_page_named_ident_beats_unnamed() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } @page cover { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("cover")),
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_named_ident_case_sensitive() {
        // Cover (rule) vs cover (query) — <custom-ident> is case-sensitive
        // per CSS Values L4 §4.2, so the rule must NOT match. Falls back
        // to the unnamed rule.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } @page Cover { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("cover")),
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_named_ident_auto_never_matches_reserved_keyword() {
        // CSS Paged Media L3 §"Page selectors" (`#page-selectors`):
        //   "A page type name of auto (ASCII case-insensitive) does not
        //    make the rule invalid, but must never match."
        // Even with `query.page_name = Some("auto")` — the strongest form
        // of the rule where a naive byte-equality compare would match —
        // the `@page auto` rule must be excluded from the cascade, so the
        // unnamed fallback (red) wins over the named-auto rule (blue).
        // This is the "rule was excluded" proof shape.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } @page auto { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("auto")),
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_named_ident_auto_case_insensitive_reserved_keyword() {
        // Same spec sentence, ASCII case-insensitive half: `Auto` and
        // `AUTO` are equally reserved and must never match. Every named
        // rule below is excluded, so the unnamed fallback (red) wins
        // regardless of the query's own casing.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } \
             @page Auto { color: blue } \
             @page AUTO { color: green }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("Auto")),
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(RED));
    }

    // ── Verification 5: named-page cascade produces named-page declarations ──
    // Task Verification 5 target: `(page_name=Some("landscape_a3"), page_index=0,
    // is_first=true)` — the winning declarations must come from the named-page
    // rule. Property proxy: `color` (M1.4 supported). `size` is an
    // `@page` descriptor and is still not wired through the declaration parser
    // (see `ruletree::tests::page_body_unsupported_property_drops_declaration`);
    // `margin` *is* supported (raikiri-spike-0vv.5) but `color` keeps this test
    // focused on selector specificity rather than on length resolution, which
    // the phase 3 tests cover. The named-page
    // rule wins because its specificity `(1, 1, 0)` beats every non-named
    // alternative under the L3 §"Cascading and page context" tuple.

    #[test]
    fn cascade_page_named_page_first_page_declarations_win_over_alternatives() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } \
             @page :first { color: green } \
             @page landscape_a3:first { color: blue } \
             @page other-page { color: red }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("landscape_a3")),
            is_first: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    // ── Additional coverage: OR over entries, AND over pseudos, source order ─

    #[test]
    fn cascade_page_comma_list_or_semantics() {
        // `@page :first, :left` — comma-separated list is OR. Only :left
        // matches this query, but the rule still contributes.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :first, :left { color: blue }", Origin::Author);
        let query = PageContextQuery {
            is_left: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_compound_pseudo_and_semantics_requires_all_match() {
        // `@page :first:left` — compound AND. Query has :first only.
        // Rule does NOT match — falls back to nothing (no rule applies).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :first:left { color: blue }", Origin::Author);
        let query = PageContextQuery {
            is_first: true,
            is_left: false,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert!(color_of(&result).is_none());
    }

    #[test]
    fn cascade_page_compound_pseudo_matches_when_all_conditions_true() {
        // Same rule, now query has both is_first + is_left.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :first:left { color: blue }", Origin::Author);
        let query = PageContextQuery {
            is_first: true,
            is_left: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_source_order_tiebreak_later_wins() {
        // Two rules of equal (rank, specificity) — later source_order wins
        // per CSS Cascading L4 §6.1
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> "Order of
        // Appearance", sibling of style-rule
        // `cascade::tests::source_order_tiebreak_later_wins`.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page :first { color: red } @page :first { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            is_first: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_later_duplicate_in_same_rule_wins() {
        // Sibling of `cascade::tests::later_duplicate_in_same_rule_wins`:
        // within a single rule, later declarations of the same property win.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red; color: blue }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_specificity_examples_from_spec() {
        // Directly encode the spec's own examples:
        //   @page { }        → (0,0,0)
        //   @page :left { }  → (0,0,1)
        //   @page :first { } → (0,1,0)
        //   @page artsy { }  → (1,0,0)
        // Winner order: artsy > :first > :left > default when all match.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } \
             @page :left { color: red } \
             @page :first { color: red } \
             @page artsy { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("artsy")),
            is_first: true,
            is_left: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_blank_pseudo_contributes_to_g_component() {
        // `:blank` counts toward `g` (same tier as `:first`) per L3 spec.
        // `@page :blank` = (0,1,0) > `@page :left` = (0,0,1).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page :left { color: red } @page :blank { color: blue }",
            Origin::Author,
        );
        let query = PageContextQuery {
            is_left: true,
            is_blank: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_right_pseudo_contributes_to_h_component() {
        // `:right` = (0,0,1) — same tier as `:left`. Source order tiebreak.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :right { color: red }", Origin::Author);
        let query = PageContextQuery {
            is_right: true,
            ..Default::default()
        };
        let result = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_specificity_ordering_derived_ord() {
        // Direct assertion on the `PageSpecificity` `Ord` derivation ensures
        // the derive-order (f, g, h) matches the spec's tuple ordering.
        let default_spec = PageSpecificity { f: 0, g: 0, h: 0 };
        let left = PageSpecificity { f: 0, g: 0, h: 1 };
        let first = PageSpecificity { f: 0, g: 1, h: 0 };
        let named = PageSpecificity { f: 1, g: 0, h: 0 };
        assert!(default_spec < left);
        assert!(left < first);
        assert!(first < named);
    }

    #[test]
    fn cascade_page_multiple_properties_all_win_independently() {
        // Regression: winner selection is per-property. Two rules setting
        // different properties both contribute.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } @page { font-weight: 700 }",
            Origin::Author,
        );
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(RED));
        assert_eq!(
            result.declarations().get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)))
        );
    }

    // ── page context inheritance: relative font-weight resolution ───────────
    //
    // CSS Paged Media 3 §6 "Page Properties"
    // <https://www.w3.org/TR/css-page-3/#page-properties>:
    //   "As with elements in the document, both the page context and the margin
    //    context have a computed value for every property … page-margin boxes
    //    inherit from the page context. The page context inherits from the root
    //    element. However, since the previous revision of CSS Paged Media Level
    //    3 did not specify this point, an implementation that sets inherited
    //    properties in the page context to their initial values (as for the root
    //    element) is also conformant …"
    //
    // Table rows come from CSS Fonts 4 §2.2.1 "Relative Weights"
    // <https://www.w3.org/TR/css-fonts-4/#relative-weights>; the table itself is
    // pinned exhaustively by `cascade::tests::
    // font_weight_bolder_lighter_table_all_six_rows`. What these tests pin is the
    // **wiring**: that `cascade_page` resolves against its `PageInheritance`
    // argument at all, and which weight it uses as the inherited value
    // (raikiri-spike-ygl0 — before
    // this, `FontWeightValue::Bolder` parked unresolved in the public
    // `declarations` map).

    /// Direct, single-site pin that `PageInheritance::FromRoot` and
    /// `PageInheritance::LegacyInitialValues` are a real discrimination, not
    /// two names for the same behaviour. The per-property tests elsewhere in
    /// this module each show one side of this split across two *separate*
    /// tests (e.g. `cascade_page_font_weight_bolder_resolves_against_root_computed_weight`
    /// vs. `cascade_page_font_weight_relative_with_legacy_initial_values_uses_initial_400`);
    /// this asserts both sides against the same rule tree in one place,
    /// following the "non-vacuous control" convention used elsewhere in this
    /// crate (see `specified_layer_residue_detector_is_not_vacuous`).
    #[test]
    fn cascade_page_from_root_and_legacy_initial_values_diverge_for_relative_font_weight() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { font-weight: bolder }", Origin::Author);
        let root = root_with_weight(700.0);

        let from_root = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        let legacy = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );

        assert_eq!(
            from_root.declarations().get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
            "`FromRoot(700)` must resolve `bolder` against the supplied root weight"
        );
        assert_eq!(
            legacy.declarations().get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))),
            "`LegacyInitialValues` must resolve `bolder` against the initial \
             weight (400), not the root's 700"
        );
    }

    /// Root [`ComputedValues`] with `font-weight: w`, everything else initial.
    fn root_with_weight(w: f32) -> ComputedValues {
        ComputedValues {
            font_weight: w,
            ..ComputedValues::initial()
        }
    }

    /// Cascade `@page { font-weight: <decl> }` with `inheritance` as the page
    /// context's inheritance parent and return the resolved absolute weight
    /// from `declarations`. `PageInheritance::LegacyInitialValues` selects the
    /// L3 legacy exception (resolution against the initial values — see
    /// [`PageInheritance`]).
    ///
    /// Panics unless the winner is an already-resolved `Absolute` — a relative
    /// keyword surviving into the public map is exactly the ygl0 regression.
    fn page_font_weight(decl: &str, inheritance: PageInheritance<'_>) -> f32 {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&format!("@page {{ font-weight: {decl} }}"), Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), inheritance);
        match result.declarations().get(&PropertyKey::FontWeight) {
            Some(PropertyValue::FontWeight(FontWeightValue::Absolute(w))) => *w,
            Some(other) => panic!(
                "raikiri-spike-ygl0 regression: an unresolved font-weight value \
                 reached the public PageCascadeResult.declarations — expected \
                 FontWeight(Absolute(_)), got {other:?}"
            ),
            None => panic!(
                "no font-weight winner in PageCascadeResult.declarations for \
                 `@page {{ font-weight: {decl} }}` — the declaration failed to \
                 parse or the cascade dropped it (this is *not* the ygl0 \
                 resolution path)"
            ),
        }
    }

    #[test]
    fn cascade_page_font_weight_bolder_resolves_against_root_computed_weight() {
        // All six `bolder` rows of the CSS Fonts 4 §2.2.1 table, selected by the
        // *root element's* computed weight (the page context's inheritance
        // parent per CSS Page 3 §6).
        let bolder = |root_w| {
            page_font_weight(
                "bolder",
                PageInheritance::FromRoot(&root_with_weight(root_w)),
            )
        };
        assert_eq!(bolder(50.0), 400.0, "w < 100 row");
        assert_eq!(bolder(100.0), 400.0, "100 <= w < 350 row");
        assert_eq!(bolder(400.0), 700.0, "350 <= w < 550 row");
        assert_eq!(bolder(700.0), 900.0, "550 <= w < 750 row");
        assert_eq!(bolder(800.0), 900.0, "750 <= w < 900 row");
        assert_eq!(
            bolder(1000.0),
            1000.0,
            "900 <= w row is no-change: must stay 1000, not clamp to 900"
        );
    }

    #[test]
    fn cascade_page_font_weight_lighter_resolves_against_root_computed_weight() {
        // Same six rows, `lighter` column.
        let lighter = |root_w| {
            page_font_weight(
                "lighter",
                PageInheritance::FromRoot(&root_with_weight(root_w)),
            )
        };
        assert_eq!(
            lighter(50.0),
            50.0,
            "w < 100 row is no-change: must stay 50, not rise to 100"
        );
        assert_eq!(lighter(100.0), 100.0, "100 <= w < 350 row");
        assert_eq!(lighter(400.0), 100.0, "350 <= w < 550 row");
        assert_eq!(lighter(700.0), 400.0, "550 <= w < 750 row");
        assert_eq!(lighter(800.0), 700.0, "750 <= w < 900 row");
        assert_eq!(lighter(1000.0), 700.0, "900 <= w row");
    }

    // ── font-size in the page context (bd raikiri-spike-zls8) ──────────────
    //
    // CSS Page 3 §6 "Page Properties"
    // <https://www.w3.org/TR/css-page-3/#page-properties> verbatim: "When used
    // on the font-size property in the page context, they are relative to the
    // font-size of the root element." `resolve_against_inherited` resolves it
    // so the public `declarations` map carries an absolute `Length::Px`.

    /// Cascade `@page { font-size: <decl> }` and return the resolved px value.
    /// `PageInheritance::LegacyInitialValues` selects the L3 legacy exception
    /// (resolution against the initial values — see [`PageInheritance`]).
    ///
    /// Panics unless the winner is an already-absolutized `Length::Px` — a
    /// font-relative unit surviving into the public map is the same class of
    /// regression as ygl0's `FontWeightValue::Bolder`.
    fn page_font_size_px(decl: &str, inheritance: PageInheritance<'_>) -> f32 {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&format!("@page {{ font-size: {decl} }}"), Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), inheritance);
        match result.declarations().get(&PropertyKey::FontSize) {
            Some(PropertyValue::FontSize(Length::Px(v))) => *v,
            Some(other) => panic!(
                "an unresolved font-size value reached the public \
                 PageCascadeResult.declarations — expected FontSize(Px(_)), \
                 got {other:?}"
            ),
            None => panic!(
                "no font-size winner in PageCascadeResult.declarations for \
                 `@page {{ font-size: {decl} }}`"
            ),
        }
    }

    /// Root [`ComputedValues`] with `font-size: px`, everything else initial.
    fn root_with_font_size(px: f32) -> ComputedValues {
        ComputedValues {
            font_size: ComputedLength(px),
            ..ComputedValues::initial()
        }
    }

    #[test]
    fn cascade_page_font_size_resolves_against_root_computed_font_size() {
        let root = root_with_font_size(20.0);
        // §6: em on font-size in the page context is relative to the root
        // element's font-size.
        assert_eq!(
            page_font_size_px("2em", PageInheritance::FromRoot(&root)),
            40.0
        );
        // CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>: rem is
        // the computed font-size of the root element — the same basis here.
        assert_eq!(
            page_font_size_px("1.5rem", PageInheritance::FromRoot(&root)),
            30.0
        );
        // Derived (not stated by §6): CSS Fonts 4 §2.5 "Percentages: refer to
        // parent element's font size" + §6 "The page context inherits from the
        // root element."
        assert_eq!(
            page_font_size_px("150%", PageInheritance::FromRoot(&root)),
            30.0
        );
        // Absolute units are context-independent.
        assert_eq!(
            page_font_size_px("18px", PageInheritance::FromRoot(&root)),
            18.0
        );
        assert_eq!(
            page_font_size_px("12pt", PageInheritance::FromRoot(&root)),
            16.0
        );
    }

    #[test]
    fn cascade_page_font_size_with_legacy_initial_values_uses_initial_16px() {
        // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
        // (initial values). §6 also
        // records the matching conformance exception for em/ex on font-size:
        // "an implementation that treats em and ex on font-size as relative to
        // the initial value is also conformant".
        assert_eq!(
            page_font_size_px("2em", PageInheritance::LegacyInitialValues),
            32.0
        );
        assert_eq!(
            page_font_size_px("2rem", PageInheritance::LegacyInitialValues),
            32.0
        );
    }

    /// `<relative-size>` (`larger` / `smaller`、raikiri-spike-4rmu) in the page
    /// context resolves against the root's computed font-size, same basis as
    /// `em` / `rem` above (CSS Page 3 §6). Reuses `page_font_size_px`, which
    /// already panics on an unresolved winner reaching `declarations` — a
    /// `FontSizeRelative` leak here is the same regression class as ygl0.
    #[test]
    fn cascade_page_font_size_relative_resolves_against_root_computed_font_size() {
        let root = root_with_font_size(20.0);
        assert_eq!(
            page_font_size_px("larger", PageInheritance::FromRoot(&root)),
            24.0
        );
        assert_eq!(
            page_font_size_px("smaller", PageInheritance::FromRoot(&root)),
            20.0 / 1.2
        );
    }

    #[test]
    fn cascade_page_font_size_relative_with_legacy_initial_values_uses_initial_16px() {
        // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
        // (initial values, 16px).
        assert_eq!(
            page_font_size_px("larger", PageInheritance::LegacyInitialValues),
            19.2
        );
    }

    // ── declarations may carry non-finite f32 (bd raikiri-spike-kj2s) ──────
    //
    // This is not guarded here — see the `# Non-finite values pass through
    // unguarded` section of
    // `PageCascadeResult::declarations`'s doc for why that is intentional
    // (sink-boundary precedent, bd raikiri-spike-2ui0) rather than an
    // oversight. This test exists to pin that the hazard is *real*, so a
    // future reader cannot dismiss the doc's claim as theoretical, and so a
    // regression that added a clamp here (which would violate the
    // precedent) has to delete this test rather than merely adjust it.

    #[test]
    fn cascade_page_font_size_can_carry_nan_from_pathological_em() {
        // Same overflow-then-multiply mechanism as the element path's 2ui0
        // reproducer (`crates/raikiri-dom/src/layout.rs`, "Reproducer A'"):
        // `1e40` overflows `f32` to `+Inf` at parse time (cssparser's f64 →
        // f32 conversion), and phase 2's `resolve_font_size`
        // (`parent_font_size.0 * v`) computes `0.0 * inf` = `NaN` per IEEE
        // 754 — root font-size 0 supplies the `0.0`.
        let root = root_with_font_size(0.0);
        let px = page_font_size_px("1e40em", PageInheritance::FromRoot(&root));
        // cov:ignore: the panic-message literal below is only executed if
        // the assertion fails, which it doesn't while this test passes.
        assert!(
            px.is_nan(),
            "expected NaN from `0.0 * inf` (root font-size 0 times an overflowed `1e40em`), got {px} — either the overflow/multiply mechanism changed (update this test and the `declarations` doc together) or a guard was added in raikiri-style (which would violate the sink-boundary precedent from bd raikiri-spike-2ui0 — see the doc's rationale before doing that)"
        );
    }

    #[test]
    fn cascade_page_font_size_can_carry_infinity_from_finite_operand_multiply() {
        // Arithmetic-only counterpart to the test above: both operands are
        // already finite `f32` values (no contested f64→f32 cast involved,
        // unlike the `1e40em` reproducer — see bd
        // raikiri-spike-9mbo). `1e30` (root font-size) and `1e20` (page
        // em multiplier) are each well within f32's finite range on their
        // own; their product, `1e50`, overflows f32 (max ~3.4e38) to
        // `+Infinity` per IEEE 754. This pins that the non-finite hazard
        // documented on `PageCascadeResult::declarations` holds
        // independently of how 9mbo's parse-time question is resolved.
        let root = root_with_font_size(1e30);
        let px = page_font_size_px("1e20em", PageInheritance::FromRoot(&root));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            px.is_infinite() && px.is_sign_positive(),
            "expected +Infinity from `1e30 * 1e20` overflowing f32, got {px}"
        );
    }

    // ── page context inheritance: relative font-weight resolution (cont'd) ──

    #[test]
    fn cascade_page_font_weight_relative_with_legacy_initial_values_uses_initial_400() {
        // `PageInheritance::LegacyInitialValues` = the L3 legacy exception
        // quoted above ("sets inherited properties in the page context to
        // their initial values").
        // `font-weight` initial is 400, so bolder(400) = 700 and
        // lighter(400) = 100.
        assert_eq!(
            page_font_weight("bolder", PageInheritance::LegacyInitialValues),
            700.0
        );
        assert_eq!(
            page_font_weight("lighter", PageInheritance::LegacyInitialValues),
            100.0
        );
    }

    #[test]
    fn cascade_page_font_weight_absolute_ignores_root_computed_weight() {
        // Absolute weights are not inherited-value dependent: the root weight
        // must not perturb them (round-trip through `resolve_against_inherited`
        // is lossless).
        let root = root_with_weight(900.0);
        assert_eq!(
            page_font_weight("250", PageInheritance::FromRoot(&root)),
            250.0
        );
        assert_eq!(
            page_font_weight("bold", PageInheritance::FromRoot(&root)),
            700.0
        );
        assert_eq!(
            page_font_weight("normal", PageInheritance::FromRoot(&root)),
            400.0
        );
    }

    #[test]
    fn cascade_page_resolution_runs_on_the_cascade_winner_only() {
        // The winner is picked *first*, then resolved once. A losing `bolder`
        // must not contribute, and a winning `bolder` must resolve against the
        // root weight — not against the weight declared by the losing rule
        // (the page context inherits from the root element, never from another
        // `@page` declaration).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { font-weight: 100 } @page { font-weight: bolder }",
            Origin::Author,
        );
        let root = root_with_weight(700.0);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
            "later `bolder` wins and resolves off root 700 → 900, not off the \
             losing declaration's 100 → 400"
        );
    }

    #[test]
    fn cascade_page_non_font_weight_winners_pass_through_resolution_unchanged() {
        // The pass-through arm of `resolve_against_inherited` must not perturb
        // properties that carry no inherited-value dependency, even when the
        // root style differs from the declared value.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red }", Origin::Author);
        let root = ComputedValues {
            color: BLUE,
            ..ComputedValues::initial()
        };
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_length_em_is_absolutized_against_page_context_font_size() {
        // Was `cascade_page_length_em_passes_through_as_specified_value` until
        // bd raikiri-spike-sshp added phase 3 (082k Phase 3). CSS Paged Media 3
        // §6 <https://www.w3.org/TR/css-page-3/#page-properties>: "Values in
        // units of em and ex are interpreted relative to the font associated
        // with their context". With no `font-size` in the page context the
        // associated font is the inherited one ("The page context inherits from
        // the root element"), i.e. the initial 16px here → 32px.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { margin-top: 2em }", Origin::Author);
        let root = root_with_weight(700.0);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                32.0
            )))),
            "`em` must be resolved against the page context's font-size"
        );
    }

    // ── 082k Phase 3 (bd raikiri-spike-sshp): phase 3 in the page context ───
    //
    // Primary sources, fetched as raw HTML (`curl -sL`) so the `data-level`
    // attributes are visible:
    // - CSS Paged Media 3 §6 "Page Properties"
    //   <https://www.w3.org/TR/css-page-3/#page-properties>
    // - CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
    //   <https://www.w3.org/TR/css-backgrounds-3/#border-width>
    // - CSS Values 4 §6.1.1 `rem` <https://www.w3.org/TR/css-values-4/#rem>

    // `root_with_font_size` (above) supplies a non-initial root font-size so
    // the assertions below cannot pass by accident off the 16px initial.

    fn padding_top_of(result: &PageCascadeResult) -> Option<Length> {
        match result.declarations().get(&PropertyKey::PaddingTop) {
            Some(PropertyValue::PaddingTop(l)) => Some(*l),
            _ => None,
        }
    }

    fn page(css: &str, root: &ComputedValues) -> PageCascadeResult {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(root),
        )
    }

    /// `@page { padding: 2em }` — the `em` basis is the page context's own
    /// font-size, which here comes from the inheritance parent (§6 "The page
    /// context inherits from the root element"). Root 20px → 40px.
    #[test]
    fn cascade_page_padding_em_uses_inherited_font_size_when_page_declares_none() {
        let root = root_with_font_size(20.0);
        let result = page("@page { padding: 2em }", &root);
        assert_eq!(padding_top_of(&result), Some(Length::Px(40.0)));
    }

    /// The sibling-declaration case that forces phase 3 to be its own pass:
    /// `font-size` is declared in the same `@page` block, so the `em` basis is
    /// the page context's *own* font-size (20px), not the root's (16px).
    #[test]
    fn cascade_page_padding_em_uses_own_font_size_over_inherited() {
        let root = ComputedValues::initial(); // 16px
        let result = page("@page { font-size: 20px; padding: 2em }", &root);
        assert_eq!(
            padding_top_of(&result),
            Some(Length::Px(40.0)),
            "`em` must use the page context's declared font-size, not the root's"
        );
    }

    /// `rem` stays relative to the **root element** even when the page context
    /// declares its own `font-size` — CSS Values 4 §6.1.1: "Equal to the
    /// computed value of the em unit on the root element." The page context is
    /// not the root element, so this is the `finalize` case, not
    /// `finalize_as_root`.
    ///
    /// `font-size: 2em` on the page context resolves against the root (§6:
    /// "When used on the font-size property in the page context, they are
    /// relative to the font-size of the root element") → 32px; `padding: 1rem`
    /// must still be 16px, **not** 32px.
    #[test]
    fn cascade_page_rem_resolves_against_root_element_not_page_context() {
        let root = ComputedValues::initial(); // 16px
        let result = page("@page { font-size: 2em; padding: 1rem }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::FontSize),
            Some(&PropertyValue::FontSize(Length::Px(32.0))),
        );
        assert_eq!(
            padding_top_of(&result),
            Some(Length::Px(16.0)),
            "`rem` is the root element's font-size, not the page context's"
        );
    }

    // ── `lh` / `rlh` in the page context (CSS Values 4 §6.1.1, bd raikiri-spike-vxha) ──

    /// `@page { line-height: 2; padding: 1.5lh }` — `1lh` uses the page
    /// context's **own** used line-height (its own font-size × the declared
    /// `<number>`), the same "own basis" story as `em`/`rem` above.
    #[test]
    fn cascade_page_padding_lh_uses_own_page_context_line_height() {
        let root = root_with_font_size(20.0); // page context's own font-size, undeclared here
        let result = page("@page { line-height: 2; padding: 1.5lh }", &root);
        // own used line-height = 2 * 20px = 40px; 1.5lh = 60px.
        assert_eq!(padding_top_of(&result), Some(Length::Px(60.0)));
    }

    /// `rlh` always refers to the **root element's** `lh`, regardless of what
    /// the page context itself declares for `line-height` — CSS Values 4
    /// §6.1.1 `rlh` + CSS Paged Media 3 §6 "The page context inherits from
    /// the root element" (the page context is not the root element, same
    /// distinction `cascade_page_rem_resolves_against_root_element_not_page_context`
    /// pins for `rem`).
    #[test]
    fn cascade_page_padding_rlh_uses_root_line_height_not_own() {
        let root = ComputedValues {
            font_size: ComputedLength(20.0),
            line_height: ComputedLineHeight::Number(3.0), // root used = 60px
            ..ComputedValues::initial()
        };
        // Page context declares its own (different) line-height — must not
        // affect `rlh`.
        let result = page("@page { line-height: 1; padding: 1rlh }", &root);
        assert_eq!(
            padding_top_of(&result),
            Some(Length::Px(60.0)),
            "rlh must use the root element's used line-height (60px), not the \
             page context's own (20px)"
        );
    }

    /// The common case: no font metrics available for `normal` — falls back
    /// to padding's own initial value `0`, same policy as the element path
    /// (`crate::resolve::resolve_length_percentage` doc). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-o9h6)
    #[test]
    fn cascade_page_padding_lh_falls_back_to_zero_when_line_height_normal() {
        let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
        let result = page("@page { padding: 1lh }", &root);
        assert_eq!(padding_top_of(&result), Some(Length::Px(0.0)));
    }

    /// roborev-refine iter 1 Finding A regression pin, `@page` path:
    /// `margin-top: 1lh` under (the initial, unresolvable) `line-height: normal`
    /// must compute to `Px(0.0)` — margin's true spec initial (CSS Box 3
    /// §3.1) — **not** `Auto` (`resolve_margin_length_or_auto`, bd
    /// raikiri-spike-vxha — element-path sibling is
    /// `margin_lh_falls_back_to_zero_not_auto_when_line_height_normal` in
    /// `crate::cascade`). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    #[test]
    fn cascade_page_margin_lh_falls_back_to_zero_not_auto_when_line_height_normal() {
        let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
        let result = page("@page { margin-top: 1lh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                0.0
            )))),
            "margin-top: 1lh under line-height: normal must be Px(0.0), not Auto"
        );
    }

    /// `@page { line-height: 1lh }` is self-referential — CSS Values 4
    /// §6.1.1, spec quote canonically documented on
    /// `crate::resolve::resolve_line_height`. The page context's "parent" // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// for this purpose is the root element (CSS Paged Media 3 §6), which
    /// is exactly what `ctx.root_line_height` already carries.
    #[test]
    fn cascade_page_line_height_self_reference_uses_root_as_parent() {
        let root = ComputedValues {
            font_size: ComputedLength(20.0),
            line_height: ComputedLineHeight::Number(2.0), // root used = 40px
            ..ComputedValues::initial()
        };
        let result = page("@page { line-height: 1lh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::LineHeight),
            Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
                40.0
            )))),
        );
    }

    /// `@page { line-height: 1rlh }` — unlike `1lh` above, `rlh` is **not**
    /// treated as self-referential (`resolve_line_height`'s `Length::Rlh`
    /// arm ignores the `self_reference_basis` argument entirely and reads
    /// `ctx.root_line_height` directly). This test happens to land on the
    /// same numeric answer as `cascade_page_line_height_self_reference_uses_root_as_parent`
    /// because the page context's "parent" *is* the root — the point is
    /// this is not a coincidence of implementation wiring but `rlh`'s own
    /// plain definition ("the lh unit on the root element").
    #[test]
    fn cascade_page_line_height_rlh_is_not_self_referential() {
        let root = ComputedValues {
            font_size: ComputedLength(20.0),
            line_height: ComputedLineHeight::Number(2.0), // root used = 40px
            ..ComputedValues::initial()
        };
        let result = page("@page { line-height: 1rlh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::LineHeight),
            Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
                40.0
            )))),
        );
    }

    // ── `font-size: 1lh` / `1rlh` in the page context (bd raikiri-spike-yh3w) ──
    //
    // §6 does not mention `lh`/`rlh` explicitly (only `em`/`ex`, verified against
    // the primary source at implementation time) — this is the same kind of
    // *derivation* the `%`/`rem` cases above already rely on: "the page context
    // inherits from the root element" (§6) + CSS Values 4 §6.1.1's self-reference
    // clause for font-* properties. It mirrors `page_context_line_height_basis`'s
    // treatment of `line-height` itself in this same context (`self_reference_parent
    // = ctx.root_line_height` there) — for the page context specifically, "parent"
    // and "root" are the same node (`inherited`), so `lh` and `rlh` on `font-size`
    // coincide here (same note as `cascade_page_line_height_rlh_is_not_self_referential`).

    /// `@page { font-size: 1.5lh }` is self-referential (same clause as
    /// `cascade_page_line_height_self_reference_uses_root_as_parent` above) —
    /// its basis is the root element's used line-height, **not** any
    /// `line-height` the same `@page` block declares (that would be the page
    /// context's *own* line-height, which is irrelevant here — `font-size` and
    /// `line-height` self-reference against the same "parent" independently).
    #[test]
    fn cascade_page_font_size_lh_self_reference_uses_root_as_parent() {
        let root = ComputedValues {
            font_size: ComputedLength(20.0),
            line_height: ComputedLineHeight::Number(2.0), // root used = 40px
            ..ComputedValues::initial()
        };
        // Page context declares a different own line-height — must not leak in.
        let result = page("@page { line-height: 1; font-size: 1.5lh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::FontSize),
            Some(&PropertyValue::FontSize(Length::Px(60.0))), // 1.5 * 40
        );
    }

    /// `@page { font-size: 1.5rlh }` lands on the same answer as `1lh` above —
    /// not a coincidence of implementation wiring, but because the page
    /// context's "parent" *is* the root element (CSS Paged Media 3 §6),
    /// exactly like `cascade_page_line_height_rlh_is_not_self_referential`.
    #[test]
    fn cascade_page_font_size_rlh_matches_lh_because_parent_is_root() {
        let root = ComputedValues {
            font_size: ComputedLength(20.0),
            line_height: ComputedLineHeight::Number(2.0), // root used = 40px
            ..ComputedValues::initial()
        };
        let result = page("@page { line-height: 1; font-size: 1.5rlh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::FontSize),
            Some(&PropertyValue::FontSize(Length::Px(60.0))), // 1.5 * 40
        );
    }

    /// The common case: root's `line-height: normal` (initial, no font
    /// metrics) — `font-size: 1lh` falls back to `font-size`'s own spec
    /// initial (`medium` = 16px), same "no real font metrics in the style
    /// layer" wall as the element path
    /// (`font_size_lh_falls_back_to_initial_when_parent_line_height_is_normal`
    /// in `crate::cascade`). Deliberately **not** `root_with_font_size`'s // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// 20px — the fallback is `font-size`'s spec initial, unconditionally,
    /// not whatever font-size the root happens to declare.
    #[test]
    fn cascade_page_font_size_lh_falls_back_to_initial_when_root_line_height_normal() {
        let root = root_with_font_size(20.0); // line-height stays `normal` (initial)
        let result = page("@page { font-size: 1lh }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::FontSize),
            Some(&PropertyValue::FontSize(Length::Px(INITIAL_FONT_SIZE_PX))),
        );
    }

    /// `page_context_line_height_basis`'s undeclared-`line-height` branch
    /// inherits `ComputedLineHeight::Number` from the root and multiplies
    /// it by the **page context's own** font-size — ordinary CSS inheritance
    /// semantics for the unitless multiplier (CSS Inline 3 §5.1), not a
    /// page-context special case. Root font-size (16px, unused for this)
    /// deliberately differs from the page context's declared font-size
    /// (40px) so a bug that used the root's font-size instead would be
    /// caught.
    #[test]
    fn cascade_page_padding_lh_uses_page_context_own_font_size_for_inherited_number() {
        let root = ComputedValues {
            line_height: ComputedLineHeight::Number(1.5), // inherited by the page context
            ..ComputedValues::initial()                   // root font-size stays 16px
        };
        let result = page("@page { font-size: 40px; padding: 1lh }", &root);
        assert_eq!(
            padding_top_of(&result),
            Some(Length::Px(60.0)), // 1.5 * 40 (page's own), not 1.5 * 16 (root's)
        );
    }

    /// `<percentage>` on `padding` is **not** absolutized — §6: "Percentage
    /// values on the margin and padding properties are relative to the
    /// dimensions of the containing block", i.e. a used-value input. The
    /// computed value is the percentage itself (CSS Values 4 §5.5.1), same as
    /// the element path (`resolve_length_percentage`).
    #[test]
    fn cascade_page_padding_percentage_stays_a_percentage() {
        let root = root_with_font_size(20.0);
        let result = page("@page { padding: 25%; margin-top: 10%; width: 50% }", &root);
        assert_eq!(padding_top_of(&result), Some(Length::Percent(25.0)));
        // `<length-percentage> | auto` takes a separate resolve path from the
        // `<length-percentage>` one above — both must pass the percentage
        // through.
        assert_eq!(
            result.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(
                Length::Percent(10.0)
            ))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::Width),
            Some(&PropertyValue::Width(LengthOrAuto::Length(
                Length::Percent(50.0)
            ))),
        );
    }

    /// `pt` is an absolute unit but not the canonical one — phase 3 normalises
    /// it to `px` (CSS Values 4 §6.2: 1pt = 1/72in, 1px = 1/96in → 12pt = 16px).
    #[test]
    fn cascade_page_absolute_units_are_normalised_to_px() {
        let root = ComputedValues::initial();
        let result = page("@page { padding: 12pt }", &root);
        assert_eq!(padding_top_of(&result), Some(Length::Px(16.0)));
    }

    /// The (b) case of bd raikiri-spike-sshp. CSS Backgrounds 3 §3.3 propdef
    /// verbatim: "Computed value: absolute length, snapped as a border width;
    /// zero if the border style is `none` or `hidden`" — a **computed**-layer
    /// requirement, so the declared `5px` must not reach `declarations`.
    #[test]
    fn cascade_page_border_width_is_gated_by_border_style_none() {
        let root = ComputedValues::initial();
        let result = page(
            "@page { border-top-width: 5px; border-top-style: none }",
            &root,
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
        );
    }

    /// `hidden` gates identically to `none` (same §3.3 clause).
    #[test]
    fn cascade_page_border_width_is_gated_by_border_style_hidden() {
        let root = ComputedValues::initial();
        let result = page(
            "@page { border-top-width: 5px; border-top-style: hidden }",
            &root,
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
        );
    }

    /// An **undeclared** `border-*-style` is its initial value `none` — §6
    /// gives the page context "a computed value for every property" and
    /// `border-style` is not inherited. So a lone `border-top-width` gates to
    /// zero, exactly as `ComputedValues::initial().border.top.width` does on
    /// the element path.
    #[test]
    fn cascade_page_border_width_alone_gates_to_zero_via_initial_style() {
        let root = ComputedValues::initial();
        let result = page("@page { border-top-width: 5px }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
        );
        assert_eq!(
            crate::computed::ComputedValues::initial().border.top.width,
            ComputedLength::ZERO,
            "element path agrees — the gate is one rule, not two",
        );
    }

    /// A visible style lets the width through, absolutized against the page
    /// context's font-size (`0.5em` of 20px = 10px). Guards against a gate that
    /// zeroes everything.
    #[test]
    fn cascade_page_border_width_with_visible_style_is_absolutized() {
        let root = ComputedValues::initial();
        let result = page(
            "@page { font-size: 20px; border-top-width: 0.5em; border-top-style: solid }",
            &root,
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(10.0))),
        );
    }

    /// The gate is per side, and all four sides are exercised — this is the
    /// only test that reaches the `BorderRightWidth` / `BorderBottomWidth` arms
    /// of `absolutize_in_page_context` and the `right` / `bottom` writes in
    /// `page_context_border_styles`.
    ///
    /// **What it does and does not catch**: the gate is binary, so of the 6
    /// possible side pairings this catches the 4 that straddle it
    /// (gated ↔ ungated); swapping the two gated sides (top ↔ bottom) or the
    /// two ungated ones is observable only through the distinct widths, which
    /// is why `6px` and `8px` differ. A mix-up *within* the gated pair stays
    /// invisible here — `page_context_border_styles` guards that structurally
    /// by naming the side in each match arm rather than looking it up by key.
    #[test]
    fn cascade_page_border_width_gate_is_per_side() {
        let root = ComputedValues::initial();
        let result = page(
            "@page { border-top-width: 5px; border-top-style: none; \
             border-right-width: 6px; border-right-style: solid; \
             border-bottom-width: 7px; border-bottom-style: none; \
             border-left-width: 8px; border-left-style: solid }",
            &root,
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(0.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderRightWidth),
            Some(&PropertyValue::BorderRightWidth(Length::Px(6.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderBottomWidth),
            Some(&PropertyValue::BorderBottomWidth(Length::Px(0.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderLeftWidth),
            Some(&PropertyValue::BorderLeftWidth(Length::Px(8.0))),
        );
    }

    /// `line-height: <percentage>` is absolutized against the page context's
    /// own font-size — CSS Inline 3 §5.1
    /// <https://www.w3.org/TR/css-inline-3/#propdef-line-height> "Percentages:
    /// computed relative to 1em". 150% of 20px = 30px.
    #[test]
    fn cascade_page_line_height_percentage_is_absolutized() {
        let root = ComputedValues::initial();
        let result = page("@page { font-size: 20px; line-height: 150% }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::LineHeight),
            Some(&PropertyValue::LineHeight(LineHeight::Length(Length::Px(
                30.0
            )))),
        );
    }

    /// `line-height: <number>` survives phase 3 as a number — the distinction
    /// is load-bearing in the computed layer (§5.1: the number is inherited and
    /// multiplied by each context's own font-size). `normal` likewise stays a
    /// keyword (resolved at the used-value layer).
    #[test]
    fn cascade_page_line_height_number_and_normal_stay_keywords() {
        let root = ComputedValues::initial();
        assert_eq!(
            page("@page { line-height: 1.5 }", &root)
                .declarations()
                .get(&PropertyKey::LineHeight),
            Some(&PropertyValue::LineHeight(LineHeight::Number(1.5))),
        );
        assert_eq!(
            page("@page { line-height: normal }", &root)
                .declarations()
                .get(&PropertyKey::LineHeight),
            Some(&PropertyValue::LineHeight(LineHeight::Normal)),
        );
    }

    /// `margin` / `width` / `height` take `<length-percentage> | auto`; `auto`
    /// is a keyword in the computed layer and must survive phase 3.
    #[test]
    fn cascade_page_auto_survives_phase_3() {
        let root = ComputedValues::initial();
        let result = page("@page { margin-top: auto; width: auto }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Auto)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::Width),
            Some(&PropertyValue::Width(LengthOrAuto::Auto)),
        );
    }

    /// Every remaining box property goes through phase 3, not just `padding` —
    /// enumerated so a missing arm in `absolutize_in_page_context` cannot hide
    /// behind the `padding` tests.
    #[test]
    fn cascade_page_all_box_properties_are_absolutized() {
        let root = ComputedValues::initial(); // 16px
        let result = page(
            "@page { padding-right: 1em; margin-bottom: 1em; width: 1em; height: 1em }",
            &root,
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::PaddingRight),
            Some(&PropertyValue::PaddingRight(Length::Px(16.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::MarginBottom),
            Some(&PropertyValue::MarginBottom(LengthOrAuto::Length(
                Length::Px(16.0)
            ))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::Width),
            Some(&PropertyValue::Width(LengthOrAuto::Length(Length::Px(
                16.0
            )))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::Height),
            Some(&PropertyValue::Height(LengthOrAuto::Length(Length::Px(
                16.0
            )))),
        );
    }

    /// The `PageInheritance::LegacyInitialValues` path also runs phase 3,
    /// against the initial values. Guards against the absolutization being
    /// wired only into the `FromRoot` branch.
    #[test]
    fn cascade_page_phase_3_runs_on_the_legacy_initial_values_path() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { padding: 2em }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(padding_top_of(&result), Some(Length::Px(32.0)));
    }

    /// Phase 3 must not re-absolutize `font-size` — phase 2 already resolved it
    /// against the *inheritance parent*, and a second pass would use the page
    /// context's own (already-resolved) value as the basis. Root 20px, `2em`
    /// → 40px; a double application would give 80px.
    #[test]
    fn cascade_page_font_size_is_not_absolutized_twice() {
        let root = root_with_font_size(20.0);
        let result = page("@page { font-size: 2em }", &root);
        assert_eq!(
            result.declarations().get(&PropertyKey::FontSize),
            Some(&PropertyValue::FontSize(Length::Px(40.0))),
        );
    }

    /// Direct unit test of the total fallback in `page_context_font_size`: with
    /// no `font-size` winner the basis is the inheritance parent's computed
    /// font-size (§6 "The page context inherits from the root element").
    #[test]
    fn page_context_font_size_falls_back_to_inherited() {
        let root = root_with_font_size(24.0);
        assert_eq!(
            page_context_font_size(&HashMap::new(), &root),
            ComputedLength(24.0),
        );
    }

    /// Direct unit test of the "undeclared style = initial `none`" rule in
    /// `page_context_border_styles`, on all four sides.
    #[test]
    fn page_context_border_styles_default_to_initial_none() {
        let styles = page_context_border_styles(&HashMap::new());
        assert_eq!(styles.top, BorderStyle::None);
        assert_eq!(styles.right, BorderStyle::None);
        assert_eq!(styles.bottom, BorderStyle::None);
        assert_eq!(styles.left, BorderStyle::None);
    }

    /// Direct exercise of the three **shorthand fall-through arms** of
    /// `absolutize_in_page_context`. They are unreachable through `cascade_page`:
    /// `crate::rule::expand_shorthand_into` runs both at the parse exit // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// (`parse_declaration_block`) and at the `@page` cascade entry (the
    /// candidate loop in `cascade_page`, bd raikiri-spike-3svx), so the
    /// post-parse mutation path through the `pub` field
    /// `PageRule::declarations` is covered too — that is what the
    /// `post_parse_page_*` tests in this module pin.
    ///
    /// They are therefore driven directly here, for the same reason
    /// `cascade::tests::apply_value_direct_margin_shorthand_fall_through` exists
    /// (behaviour pinned instead of `unreachable!` — the crate keeps the
    /// cascade panic-free, and the exhaustive expansion `match` does not
    /// enforce everything; see its doc, bd raikiri-spike-ez7b).
    ///
    /// Since bd raikiri-spike-7m33 `absolutize_in_page_context` takes a
    /// `ResolvedAgainstInherited`, whose constructor is private outside
    /// `crate::cascade` — `ResolvedAgainstInherited::for_test` is the // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
    /// `#[cfg(test)]`-only escape hatch that lets this test keep driving the
    /// function directly with a hand-picked payload (see that type's doc,
    /// "test 用の裏口").
    #[test]
    fn absolutize_in_page_context_shorthand_fall_throughs() {
        let fs = ComputedLength(20.0);
        let ctx = ResolveContext::new(ComputedLength(16.0));
        let styles = Sides::all(BorderStyle::None);

        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::Padding(Sides::all(Length::Em(
                    2.0
                )))),
                fs,
                None,
                &ctx,
                styles,
            ),
            PropertyValue::Padding(Sides::all(Length::Px(40.0))),
        );
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::Margin(Sides::all(
                    LengthOrAuto::Length(Length::Rem(2.0))
                ))),
                fs,
                None,
                &ctx,
                styles,
            ),
            PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(32.0)))),
        );
        // Each side of the `border` shorthand gates on the style it carries
        // itself, not on `border_styles` (which describes the longhands).
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::Border(Sides::all(Border {
                    width: Length::Em(1.0),
                    style: BorderStyle::Solid,
                    color: BorderColor::CurrentColor,
                }))),
                fs,
                None,
                &ctx,
                styles,
            ),
            PropertyValue::Border(Sides::all(Border {
                width: Length::Px(20.0),
                style: BorderStyle::Solid,
                color: BorderColor::CurrentColor,
            })),
        );
    }

    /// Direct exercise of the `FontSizeRelative` "safety net" arm of
    /// `absolutize_in_page_context` — structurally unreachable through
    /// `cascade_page` (step 3/phase 2 always converges `FontSizeRelative` to
    /// `FontSize(Length::Px(_))` first, see the arm's own doc), but the
    /// `pub(crate)` function can still be driven directly with an unresolved
    /// value, same as `absolutize_in_page_context_shorthand_fall_throughs`
    /// above — via `ResolvedAgainstInherited::for_test`, the `#[cfg(test)]`
    /// escape hatch bd raikiri-spike-7m33 added once the function's parameter
    /// stopped being a raw `PropertyValue` (see that type's doc, "test 用の
    /// 裏口"). Pins: no panic, and the result shape matches what phase 2
    /// (`resolve_against_inherited`'s `FontSizeRelative` arm) would have
    /// produced — `FontSize(Length::Px(_))`.
    #[test]
    fn absolutize_in_page_context_font_size_relative_safety_net() {
        use crate::property::RelativeFontSize;

        let fs = ComputedLength(20.0);
        let ctx = ResolveContext::new(ComputedLength(16.0));
        let styles = Sides::all(BorderStyle::None);

        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::FontSizeRelative(
                    RelativeFontSize::Larger
                )),
                fs,
                None,
                &ctx,
                styles,
            ),
            PropertyValue::FontSize(Length::Px(24.0)),
        );
        assert_eq!(
            absolutize_in_page_context(
                ResolvedAgainstInherited::for_test(PropertyValue::FontSizeRelative(
                    RelativeFontSize::Smaller
                )),
                fs,
                None,
                &ctx,
                styles,
            ),
            PropertyValue::FontSize(Length::Px(20.0 / 1.2)),
        );
    }

    // ── 「`declarations` は computed 値」契約の機械的 pin ────────────────────
    //
    // bd raikiri-spike-awjx。契約の canonical な記述は
    // `PageCascadeResult::declarations` の doc にあり、本節はそれを**散文では
    // なく実行可能な形で**押さえる。散文だけで保っていた間に drift が 2 回
    // 起きている: (1) bd raikiri-spike-ygl0 で対応表が実装と乖離し
    // bd raikiri-spike-zls8 が in-band caveat で patch、(2) bd raikiri-spike-sshp
    // の diff 内で pass-through arm の数え上げが「21」と書かれた (実測 22)。
    //
    // 本節が壊れる条件:
    //
    // - `PropertyValue` に variant を足す → `specified_layer_residue` の網羅
    //   match が **compile error**。raikiri-spike-a754 以前は、分類を書いた
    //   後に `page_corpus` / `PROPERTY_VALUE_VARIANTS` (手で持つ数)
    //   の更新を強制するものが無かった — 両者が 40 で整合したまま新 variant
    //   が corpus 外に残る case は**全 test green になる** (bd
    //   raikiri-spike-awjx §8.2 debt lens が `PropertyValue::Orphan` を足して
    //   実測、raikiri-spike-a754 で当時の crate 状態 (58343de) に対し
    //   再実証: 6 箇所の網羅 match [`property::PropertyValue::key`,
    //   `rule::expand_shorthand_into`, `absolutize_in_page_context`,
    //   `specified_layer_residue`, `cascade::resolve_against_inherited`,
    //   `cascade::apply_value`] を素直に分類しただけで 674 passed / 0
    //   failed、新 variant は corpus 側 3 本の pin のどれにも通らなかった —
    //   awjx 時点の記録は 5 site だったが、`rule::expand_shorthand_into` が
    //   その後の landing で網羅 match になっており今は 6 site)。
    //   raikiri-spike-a754 は `PROPERTY_VALUE_VARIANTS` を廃止し、
    //   `page_corpus` を `sample_for` (`PropertyKey` に対する網羅 match、
    //   `property_key_samples!` マクロ生成) 駆動に変えた — 新しい
    //   `PropertyKey` variant を伴う通常の property 追加は、上と同じ手順
    //   (新 variant + 6 site の素直な分類) を fix 後の code に対して
    //   再実行して確認済み: `sample_for` の compile error が発生し、
    //   `property_key_samples!` 自体を書き換えない限り、それを直す方法は
    //   呼び出しへの 1 行追加 (`ALL_PROPERTY_KEYS` と `sample_for` を同時に
    //   拡張する) だけなので、「compile error は直したが corpus は更新し
    //   忘れた」という中間状態が自然には起きない — この経路では
    //   674 passed / 0 failed が **2 failed**
    //   (`phase_3_variant_classification_matches_the_documented_counts`
    //   / `specified_layer_residue_detector_is_not_vacuous` — どちらも
    //   `PHASE_3_PASS_THROUGH_VARIANTS` 系の別定数側が動かないことで発火する、
    //   詳細は該当 test の doc) に変わることを確認した。
    //
    //   **guard は当初 one-way だった** (bd raikiri-spike-a754 が
    //   bd raikiri-spike-c0z9 として追跡開始) — 新 variant が既存の
    //   `PropertyKey` を再利用する場合 (`FontSizeRelative` と同型) は
    //   `PropertyKey` の variant 集合が増えないため `sample_for` も compile
    //   error にならず、`key_sharing_extras` への追加は手作業のままだった。
    //   raikiri-spike-c0z9 起票時点の記録は「真の automatic closure には
    //   `PropertyValue`/`PropertyKey` 自体への macro/derive が要り、それは
    //   walls.md 壁 5 (umbrella re-export) の再判定を要する out-of-scope な
    //   変更」だった — この結論は「型の variant を安全に列挙する手段が
    //   stable Rust に無い (`mem::variant_count` は unstable、外部 derive
    //   crate は cleanroom 方針外)」という前提に基づいていたが、
    //   **reflection による列挙**と**網羅 match による forcing**を区別して
    //   いなかった。bd raikiri-spike-c0z9 の実装は後者を選んだ:
    //   `property_value_variant_registry!` (`key_sharing_extras` の直後) が
    //   `PropertyKey` ではなく `PropertyValue` **自身**に対して網羅的な
    //   match を生成する — `property_key_samples!` が `PropertyKey` に
    //   対してやっていることをそのまま一般化しただけで、`PropertyValue`/
    //   `PropertyKey` の定義自体には触れない (`#[non_exhaustive]` は
    //   downstream crate にのみ効くため、定義と同じ crate 内のここでの
    //   網羅性には影響しない — wall/umbrella crossing ではない)。
    //
    //   ⚠️ **これで閉じるのは「compile error になるかどうか」までであり
    //   「corpus への反映を忘れないこと」自体を compile error にはしない**
    //   — 網羅 match は「型に variant が増えたこと」しか検出できず、増えた
    //   variant を `sample_for` / `key_sharing_extras` 側へ反映し忘れる
    //   ことまでは compile-time には防げない。その反映漏れは
    //   `page_corpus_covers_every_registered_property_value_variant` (test、
    //   `property_value_variant_registry!` の直後) が runtime で検出する —
    //   `PROPERTY_VALUE_VARIANT_COUNT` (登録側、網羅 match 経由で正しく
    //   増える) と `page_corpus().len()` (corpus 側、反映漏れがあれば
    //   増えない) の不一致が test failure として現れる。
    //
    //   ⚠️ 本節がこの一連の経緯・機構・残存ギャップの canonical な記述
    //   (raikiri-spike-a754、raikiri-spike-c0z9) — `property_key_samples!` /
    //   `sample_for` / `property_value_variant_registry!` /
    //   `specified_layer_residue` / 下の corpus 整合性 test の doc は
    //   ここへの pointer のみを持ち、繰り返さない。
    // - `absolutize_in_page_context` の arm 分類を動かす →
    //   `phase_3_variant_classification_matches_the_documented_counts` が落ちる。
    // - phase 2 / phase 3 を素通りする値が `declarations` に届くようになる →
    //   `page_declarations_carry_no_specified_layer_residue` /
    //   `cascade_page_output_carries_no_specified_layer_residue` が落ちる。
    //
    // raikiri-spike-l3wg で実際に踏んだ改修: `text-align: match-parent` が
    // 解決可能になったことで上記 2 test の期待値は「例外 1 つ」から「例外 0」
    // (空 vec) に変わった (旧 test 名
    // `page_declarations_carry_exactly_one_specified_layer_residue` は
    // `page_declarations_carry_no_specified_layer_residue` に改名)。
    // `specified_layer_residue` の `TextAlign::MatchParent` arm 自体は
    // **削除しなかった** — `resolve_against_inherited` を経由し損ねる将来の
    // regression (bd raikiri-spike-7m33 gap (b) 相当) に対する tripwire として
    // 残してある。同じ「今後別の inherited-value-dependent keyword を足す」
    // ケースへの一般化: 新しい `PropertyValue` variant / payload が **phase 2
    // で解決される**ようになったら、本節に残る手で持つ数
    // (`PHASE_3_PASS_THROUGH_VARIANTS` / `raw_corpus_residue_variants` の
    // `+ 3` 項の内訳コメント — raikiri-spike-a754 でこの 2 つは対象外、
    // 別途判断が要ることが確定している) と `page_corpus` の worst-case
    // payload、および `page_declarations_carry_no_specified_layer_residue` /
    // `cascade_page_output_carries_no_specified_layer_residue` の期待値を
    // 同時に見直すこと。

    /// phase 3 (`absolutize_in_page_context`) が**素通しする** variant 数。
    ///
    /// 22 → 23 (raikiri-spike-l3wg、`Direction` は phase 3 で変換する length を
    /// 持たないため pass-through 側に加わる — `TextAlign` 自身は元々こちら側)。
    ///
    /// raikiri-spike-a754 の対象外 — 本定数と下の `raw_corpus_residue_variants`
    /// の `+ 3` 項は「phase 3 の分類自体」という別種の hand-maintained な事実
    /// であり、bd raikiri-spike-a754 が明示的に別途判断としている
    /// (origin bd raikiri-spike-awjx §8.2 quality lens F1 の「残り 2 つ」)。
    const PHASE_3_PASS_THROUGH_VARIANTS: usize = 23;

    /// phase 3 が**変換する** variant 数。内訳は line-height 1 / padding
    /// (longhand 4 + shorthand 1) / margin (longhand 4 + shorthand 1) /
    /// border-width (longhand 4 + `border` shorthand 1) / width + height 2 /
    /// font-size: larger/smaller 1 (`FontSizeRelative` — raikiri-spike-4rmu、
    /// `absolutize_in_page_context` の structurally-unreachable な safety-net
    /// arm。到達しないが「素通し」ではなく実際に変換する形の arm なので
    /// `PHASE_3_PASS_THROUGH_VARIANTS` 側には数えない)。
    ///
    /// raikiri-spike-a754 以前は `PROPERTY_VALUE_VARIANTS -
    /// PHASE_3_PASS_THROUGH_VARIANTS` という `const` 式だった。
    /// `PROPERTY_VALUE_VARIANTS` を廃止した (`page_corpus` の doc 参照) ので
    /// `page_corpus().len()` (`Vec` を allocate するため const 文脈で呼べない)
    /// を使う `fn` に変えてある。**この値の基準は `page_corpus().len()` —
    /// corpus が `PropertyValue` の全 variant を実際に覆っている間だけ
    /// 「変換する variant 数」を表す。** corpus の完全性自体は
    /// raikiri-spike-a754 時点では独立には検査されていなかった (旧 `const`
    /// 式と数値的に同じ結果を返すことと「意味が同じ」ことは別の主張
    /// だった) — bd raikiri-spike-c0z9 (`page_corpus` 手前の section
    /// comment 参照) がこのギャップを埋め、`property_value_variant_registry!`
    /// (`PropertyValue` 自身の variant 集合に対する網羅 match、`page_corpus`
    /// 直後) と `page_corpus_covers_every_registered_property_value_variant`
    /// (test、同じ並び) 経由で corpus 完全性の独立検査を復活させた。「基準
    /// として妥当」は再び test で担保されている。
    fn phase_3_transformed_variants() -> usize {
        page_corpus().len() - PHASE_3_PASS_THROUGH_VARIANTS
    }

    /// phase 2 / phase 3 を**通す前**の corpus が持つ specified 層残滓の数 =
    /// `phase_3_transformed_variants()` + phase 2 が解決する `font-size` /
    /// `font-weight` / `text-align: match-parent` の 3。
    ///
    /// 数自体 (3) は raikiri-spike-l3wg 前後で**変わらない** — 変わったのは
    /// 3 番目の意味: 以前は「phase 2 でも phase 3 でも解決しない孤立した残滓」
    /// (`text-align: match-parent`) だったが、今は他 2 つ (`font-size` /
    /// `font-weight`) と同じ「phase 2 が解決する」側に合流した。この値は
    /// **raw corpus** (どちらの phase も通していない) に対する残滓数を数えて
    /// いるので、resolve 先が phase 2 か「resolve 不能」かに関わらず raw 値
    /// そのものが `specified_layer_residue` に引っかかる限り数は同じになる。
    ///
    /// `FontSizeRelative` (`larger`/`smaller`) もこの「phase 2 が解決する」
    /// 3 種と同じ扱いに**見える**が、`+3` には数えない — phase 3
    /// (`absolutize_in_page_context`) 側で `FontSize` / `FontWeight` /
    /// `TextAlign::MatchParent` は「pass-through」bucket
    /// (`PHASE_3_PASS_THROUGH_VARIANTS`) に居るのに対し、`FontSizeRelative` は
    /// 独自の (structurally unreachable な) transform arm を持ち
    /// `phase_3_transformed_variants()` 側に既に数えられているため
    /// (二重計上を避ける、上記関数の doc 参照)。
    fn raw_corpus_residue_variants() -> usize {
        phase_3_transformed_variants() + 3
    }

    /// `sample_for` / `ALL_PROPERTY_KEYS` を **1 つの token 列**から生成する
    /// (raikiri-spike-a754)。`key => value` の対を 1 度書けば
    /// `ALL_PROPERTY_KEYS` (列挙) と `sample_for` (網羅 match) の**両方**に
    /// そのまま展開される。
    ///
    /// 本 macro が何を置き換え、何を塞ぎ何を塞がないかの canonical な記述は
    /// `page_corpus` 手前の section comment (「`declarations` は computed 値」
    /// 契約の機械的 pin 節) にある — 繰り返さない。
    macro_rules! property_key_samples {
        ($($key:ident => $value:expr),+ $(,)?) => {
            /// `PropertyValue::key()` を経由して 1:1 対応する `PropertyKey`
            /// 全件、`property_key_samples!` 呼び出しでの記述順
            /// (= `PropertyKey` 自身の宣言順、`property.rs`)。key を複数
            /// `PropertyValue` variant で共有するもの (`FontSize` /
            /// `FontSizeRelative`) はここには 1 度しか現れない —
            /// 共有側は `key_sharing_extras` が別途持つ。
            const ALL_PROPERTY_KEYS: &[PropertyKey] = &[$(PropertyKey::$key),+];

            /// 与えられた `PropertyKey` に対する **specified 層の worst
            /// case** `PropertyValue` サンプルを 1 つ返す。
            ///
            /// **wildcard arm を置かない** (`property_key_samples!` の
            /// 展開そのものが持たない) — `PropertyKey` に variant を足すと
            /// ここで **compile error** になる。この compile error が何を
            /// 強制し (通常の新 property 追加)、何を強制しないか
            /// (`FontSizeRelative` 型の key 共有 variant、bd
            /// raikiri-spike-c0z9) の canonical な記述は `page_corpus` 手前の
            /// section comment にある。
            fn sample_for(key: PropertyKey) -> PropertyValue {
                match key {
                    $(PropertyKey::$key => $value,)+
                }
            }
        };
    }

    property_key_samples! {
        Color => PropertyValue::Color(RED),
        BackgroundColor => PropertyValue::BackgroundColor(BLUE),
        FontFamily => PropertyValue::FontFamily(Arc::new(vec![Atom::from("serif")])),
        FontSize => PropertyValue::FontSize(Length::Em(2.0)),
        FontWeight => PropertyValue::FontWeight(FontWeightValue::Bolder),
        LineHeight => PropertyValue::LineHeight(LineHeight::Length(Length::Em(2.0))),
        Display => PropertyValue::Display(DisplayValue::Block),
        CounterReset => PropertyValue::CounterReset(Arc::new(vec![("c".into(), 0)])),
        CounterIncrement => PropertyValue::CounterIncrement(Arc::new(vec![("c".into(), 1)])),
        CounterSet => PropertyValue::CounterSet(Arc::new(vec![("c".into(), 2)])),
        Content =>
            PropertyValue::Content(Arc::new(vec![ContentComponent::Literal("x".into())])),
        StringSet => PropertyValue::StringSet(Arc::new(vec![(
            "s".into(),
            vec![ContentComponent::Literal("x".into())],
        )])),
        Position => PropertyValue::Position(PositionValue::Static),
        TextAlign => PropertyValue::TextAlign(TextAlign::MatchParent),
        PaddingTop => PropertyValue::PaddingTop(Length::Em(2.0)),
        PaddingRight => PropertyValue::PaddingRight(Length::Em(2.0)),
        PaddingBottom => PropertyValue::PaddingBottom(Length::Em(2.0)),
        PaddingLeft => PropertyValue::PaddingLeft(Length::Em(2.0)),
        Padding => PropertyValue::Padding(Sides::all(Length::Em(2.0))),
        MarginTop => PropertyValue::MarginTop(LengthOrAuto::Length(Length::Rem(2.0))),
        MarginRight => PropertyValue::MarginRight(LengthOrAuto::Length(Length::Rem(2.0))),
        MarginBottom => PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Rem(2.0))),
        MarginLeft => PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Rem(2.0))),
        Margin => PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Rem(2.0)))),
        BorderTopWidth => PropertyValue::BorderTopWidth(Length::Pt(12.0)),
        BorderRightWidth => PropertyValue::BorderRightWidth(Length::Pt(12.0)),
        BorderBottomWidth => PropertyValue::BorderBottomWidth(Length::Pt(12.0)),
        BorderLeftWidth => PropertyValue::BorderLeftWidth(Length::Pt(12.0)),
        BorderTopStyle => PropertyValue::BorderTopStyle(BorderStyle::Solid),
        BorderRightStyle => PropertyValue::BorderRightStyle(BorderStyle::Solid),
        BorderBottomStyle => PropertyValue::BorderBottomStyle(BorderStyle::Solid),
        BorderLeftStyle => PropertyValue::BorderLeftStyle(BorderStyle::Solid),
        BorderTopColor => PropertyValue::BorderTopColor(BorderColor::CurrentColor),
        BorderRightColor => PropertyValue::BorderRightColor(BorderColor::CurrentColor),
        BorderBottomColor => PropertyValue::BorderBottomColor(BorderColor::CurrentColor),
        BorderLeftColor => PropertyValue::BorderLeftColor(BorderColor::CurrentColor),
        Border => PropertyValue::Border(Sides::all(Border {
            width: Length::Em(1.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor,
        })),
        Width => PropertyValue::Width(LengthOrAuto::Length(Length::Em(3.0))),
        Height => PropertyValue::Height(LengthOrAuto::Length(Length::Em(4.0))),
        BoxSizing => PropertyValue::BoxSizing(BoxSizing::BorderBox),
        // No specified/computed distinction for `direction` (computed
        // value = specified value) — any value is "worst case".
        Direction => PropertyValue::Direction(Direction::Rtl),
    }

    /// `sample_for` の 1:1 `PropertyKey -> PropertyValue` マッピングに
    /// **乗らない** `PropertyValue` variant — 他の variant と `PropertyKey`
    /// を意図的に共有するもの。今日時点でこれに該当するのは
    /// [`PropertyValue::FontSizeRelative`] (`PropertyKey::FontSize` を
    /// `PropertyValue::FontSize` と共有 — cascade winner selection のための
    /// 設計、同 variant の doc 参照) だけ。
    ///
    /// `page_corpus` へは**この関数の戻り値をそのまま追加**する — 「+1」の
    /// ような長さの算術に畳まない。2 つ目の key 共有 variant が現れたら
    /// ここに `vec!` の要素をもう 1 つ足すだけで済み、この comment を
    /// 読み解いて magic number を計算し直す必要が無い。
    ///
    /// 沿革: raikiri-spike-4rmu 以前は空 (共有 pattern 自体が無かった)、
    /// 4rmu で `FontSizeRelative` により 1 要素になって以来変わっていない。
    ///
    /// 2 つ目の key 共有 variant を足す義務は、以前は comment 頼みだった
    /// (compile error による forcing が無かった — bd raikiri-spike-c0z9)。
    /// 今は `property_value_variant_registry!` (下) が `PropertyValue` 自身に
    /// 対して網羅的な match を生成しており、新 variant を足すとまずそちらが
    /// compile error になる。その状態で本関数への追加を忘れても
    /// `page_corpus_covers_every_registered_property_value_variant` (test、
    /// 下) が red になるので、ここへの追加漏れは最終的に検出される。
    fn key_sharing_extras() -> Vec<PropertyValue> {
        use crate::property::RelativeFontSize;
        vec![PropertyValue::FontSizeRelative(RelativeFontSize::Larger)]
    }

    /// 全 `PropertyValue` variant を **specified 層の worst case** payload で
    /// 1 つずつ並べたもの — `sample_for` (`ALL_PROPERTY_KEYS` を経由) と
    /// `key_sharing_extras` から生成する (raikiri-spike-a754)。並び順は
    /// `PropertyKey` の宣言順 + 末尾に key 共有 variant。本 module のどの
    /// test も corpus の順序には依存しない (`HashSet` / `filter` / 走査で
    /// 完結する) ので、`PropertyValue` 自身の宣言順 (旧来の順序) との違いは
    /// 挙動に影響しない。
    ///
    /// worst case = 「phase 2 / phase 3 を通さなければ specified 層の残滓が
    /// 残る」値: length は `Em` / `Rem` / `Pt` (`Px` / `Percent` は既に computed
    /// 層なので使わない)、`font-weight` は `bolder`、`text-align` は
    /// `match-parent`、`font-size` の relative variant は `larger`
    /// (raikiri-spike-4rmu)。個々の選定根拠は `sample_for` / `key_sharing_extras`
    /// の呼び出し箇所を参照。
    ///
    /// この関数**自体**の完全性 (「`PropertyValue` の全 variant を実際に
    /// 覆っているか」) は `ALL_PROPERTY_KEYS` / `sample_for` の網羅性からは
    /// 出てこない (`sample_for` は `PropertyKey` に対して網羅的であり、
    /// `PropertyValue` に対してではない — bd raikiri-spike-c0z9)。その完全性は
    /// `property_value_variant_registry!` + `page_corpus_covers_every_registered_property_value_variant`
    /// (共に下) が別途保証する。
    fn page_corpus() -> Vec<PropertyValue> {
        let mut corpus: Vec<PropertyValue> =
            ALL_PROPERTY_KEYS.iter().copied().map(sample_for).collect();
        corpus.extend(key_sharing_extras());
        corpus
    }

    /// `PropertyValue` **自身**に対して網羅的な match を 1 つの token 列から
    /// 生成する (`property_key_samples!` の姉妹 macro、bd raikiri-spike-c0z9)。
    ///
    /// `property_key_samples!` は `PropertyKey` に対して網羅的なので、新しい
    /// `PropertyKey` を伴う通常の property 追加は forced だが、**既存の**
    /// `PropertyKey` を再利用する新 variant (`FontSizeRelative` が
    /// `PropertyKey::FontSize` を再利用するのと同型) は `PropertyKey` の
    /// variant 集合を増やさないため、その網羅 match は compile error に
    /// ならない (`page_corpus` 手前の section comment の「一方向性」節、
    /// raikiri-spike-a754 が残した bd raikiri-spike-c0z9 として追跡していた
    /// ギャップ)。
    ///
    /// 本 macro はこの穴を埋める — `PropertyValue` 自身の variant 集合に
    /// 対して網羅的なので、key を再利用する variant も含め **どんな新
    /// variant でも** compile error になる。`stable Rust` に variant を
    /// 安全に列挙する手段 (`mem::variant_count` は unstable、外部 derive
    /// crate は cleanroom 方針外) が無いという前提は変わっていないが、
    /// 「型に対する reflection」ではなく「型に対する網羅 match」で同じ
    /// forcing を得られる — これは `property_key_samples!` が `PropertyKey`
    /// に対して既にやっていることを `PropertyValue` に一般化しただけであり、
    /// `PropertyValue`/`PropertyKey` の定義自体には一切触れない
    /// (`#[non_exhaustive]` は downstream crate にのみ効くため、定義側と
    /// 同じ crate 内の本 match には影響しない)。
    ///
    /// 生成するもの:
    /// - `PROPERTY_VALUE_VARIANT_COUNT`: token 列の要素数 = 現在の
    ///   `PropertyValue` variant 総数。
    /// - `property_value_variant_name`: 上記の網羅 match。戻り値
    ///   (variant 名の文字列) 自体に意味は無い — exhaustiveness を
    ///   compile-time に強制することだけが目的。
    ///
    /// この 2 つを組み合わせても、`key_sharing_extras()` / `sample_for` への
    /// 追加漏れそのものを**この macro だけでは**検出しない — 網羅 match は
    /// 「型に新しい variant が増えた」ことだけを compile error にする。
    /// 「増えた variant を corpus (`page_corpus`) 側へ反映し忘れた」ことは
    /// `page_corpus_covers_every_registered_property_value_variant` (test、
    /// 下) が runtime で検出する: `PROPERTY_VALUE_VARIANT_COUNT` は compile
    /// error 経由で正しく増えるが `page_corpus().len()` は反映漏れがあれば
    /// 増えないので、両者の不一致が test failure として現れる。
    macro_rules! property_value_variant_registry {
        ($($variant:ident),+ $(,)?) => {
            const PROPERTY_VALUE_VARIANT_COUNT: usize = [$(stringify!($variant)),+].len();

            fn property_value_variant_name(value: &PropertyValue) -> &'static str {
                match value {
                    $(PropertyValue::$variant(_) => stringify!($variant),)+
                }
            }
        };
    }

    // `property.rs` の `PropertyValue` 宣言順と同じ順に列挙 (機械的な追従を
    // 楽にするための慣習 — 順序自体に意味は無い、`property_key_samples!` の
    // 呼び出しと同様)。
    property_value_variant_registry! {
        Color,
        BackgroundColor,
        FontFamily,
        FontSize,
        FontSizeRelative,
        FontWeight,
        LineHeight,
        Display,
        CounterReset,
        CounterIncrement,
        CounterSet,
        Content,
        StringSet,
        Position,
        TextAlign,
        PaddingTop,
        PaddingRight,
        PaddingBottom,
        PaddingLeft,
        Padding,
        MarginTop,
        MarginRight,
        MarginBottom,
        MarginLeft,
        Margin,
        BorderTopWidth,
        BorderRightWidth,
        BorderBottomWidth,
        BorderLeftWidth,
        BorderTopStyle,
        BorderRightStyle,
        BorderBottomStyle,
        BorderLeftStyle,
        BorderTopColor,
        BorderRightColor,
        BorderBottomColor,
        BorderLeftColor,
        Border,
        Width,
        Height,
        BoxSizing,
        Direction,
    }

    /// `page_corpus()` が `property_value_variant_registry!` に登録された
    /// **全ての** `PropertyValue` variant を実際に覆っていること —
    /// bd raikiri-spike-c0z9 の一方向性 gap のクローズ。
    ///
    /// 新しい variant が `property_value_variant_registry!` の呼び出しに
    /// 追加されないままだと `property_value_variant_name` の網羅 match が
    /// compile error になる (この test 以前の問題)。本 test はその一歩先 —
    /// **compile は通ったが corpus への反映を忘れた**中間状態 (`sample_for`
    /// への新 `PropertyKey` arm 追加、または `key_sharing_extras()` への
    /// 要素追加のどちらかを忘れた場合) を runtime で検出する。
    #[test]
    fn page_corpus_covers_every_registered_property_value_variant() {
        let corpus = page_corpus();
        // `property_value_variant_name` を実際の corpus 値に対して呼ぶ ---
        // この match が持つ exhaustiveness 自体は型レベルで compile-time に
        // 強制されている (呼び出し有無に関わらず) ので、ここでの呼び出しは
        // 主に「dead code にしない」ための実利用と、match 本体が実際の
        // payload shape に対して panic しないことの smoke check。
        for value in &corpus {
            property_value_variant_name(value);
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            corpus.len(),
            PROPERTY_VALUE_VARIANT_COUNT,
            "page_corpus() has {} entries but property_value_variant_registry! \
             lists {} PropertyValue variants -- a variant was added to the \
             registry without a matching sample_for (new PropertyKey) or \
             key_sharing_extras() (reused PropertyKey) entry, or vice versa. \
             See bd raikiri-spike-c0z9.",
            corpus.len(),
            PROPERTY_VALUE_VARIANT_COUNT,
        );
    }

    /// `value` が **specified 層でしか意味を持たない表現**を残しているか。
    ///
    /// `PageCascadeResult::declarations` の doc が宣言する「これは computed 値だ」
    /// を検査可能にしたもの。`Some(_)` は「その値は phase 2 / phase 3 を素通り
    /// した」を意味する。
    ///
    /// **wildcard arm を置かない** — `PropertyValue` に variant を足すとここで
    /// compile error になる。この compile error と `page_corpus`
    /// (`sample_for`) 側の更新がどう連動する (しない) かの canonical な
    /// 記述は `page_corpus` 手前の section comment (raikiri-spike-a754) に
    /// ある。
    ///
    /// # 網羅 match が及ぶ payload 型は 5 つだけ
    ///
    /// `Length` / `LengthOrAuto` / `LineHeight` / `FontWeightValue` /
    /// `TextAlign`。この 5 型については
    /// `crate::cascade::resolve_against_inherited` の doc が「この guard が // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (non-#[test] mod-level helper doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-csmj; demoted from an already-linked span by bd raikiri-spike-gq7x)
    /// 守らない範囲」として挙げる **既存 variant への payload 追加**
    /// (bd raikiri-spike-7m33 の gap (a)) もここで compile error になる。
    ///
    /// **及ばない**もの: `BorderStyle` / `BorderColor` / `DisplayValue` /
    /// `PositionValue` / `BoxSizing` / `ContentComponent` などは `(_)` で捨てて
    /// いる。また `PropertyValue::Border` は struct pattern ではなく field
    /// access (`b.width`) で読むので、[`Border`] に length を運ぶ field を
    /// 追加しても compile error にならない。
    fn specified_layer_residue(value: &PropertyValue) -> Option<&'static str> {
        /// box property (`padding` / `margin` / `width` / `height` /
        /// `border-*-width`) の `<length-percentage>`。
        ///
        /// `%` は CSS Values 4 §5.5.1
        /// <https://www.w3.org/TR/css-values-4/#combine-percentages> の既定
        /// ("the computed value of a percentage is the specified percentage")
        /// どおり computed 層に残る。`Pt` を残滓とする根拠は「absolute で
        /// ない」ではない — CSS Values 4 §6.2
        /// <https://www.w3.org/TR/css-values-4/#absolute-lengths> は `pt` も
        /// absolute length に数える。根拠は同 § の "px is their canonical
        /// unit" 側であり、computed 層の運搬 shape を `Px` に正規化する
        /// raikiri の invariant である。
        fn length(l: Length) -> Option<&'static str> {
            match l {
                Length::Px(_) | Length::Percent(_) => None,
                Length::Em(_) => Some("Length::Em"),
                Length::Rem(_) => Some("Length::Rem"),
                Length::Pt(_) => Some("Length::Pt"),
                // bd raikiri-spike-2x8 — additional font-relative / absolute
                // units。`Em` / `Rem` / `Pt` と同じ理由で残滓 (絶対化前は
                // computed 層に存在しない specified-only 表現)。
                Length::Ex(_) => Some("Length::Ex"),
                Length::Rex(_) => Some("Length::Rex"),
                Length::Ch(_) => Some("Length::Ch"),
                Length::Rch(_) => Some("Length::Rch"),
                Length::Ic(_) => Some("Length::Ic"),
                Length::Ric(_) => Some("Length::Ric"),
                Length::Cm(_) => Some("Length::Cm"),
                Length::Mm(_) => Some("Length::Mm"),
                Length::Q(_) => Some("Length::Q"),
                Length::In(_) => Some("Length::In"),
                Length::Pc(_) => Some("Length::Pc"),
                // bd raikiri-spike-vxha — same reasoning as the `Em`/`Rem`/`Pt`
                // arms above: pre-absolutization these units don't exist in
                // the computed layer.
                Length::Lh(_) => Some("Length::Lh"),
                Length::Rlh(_) => Some("Length::Rlh"),
            }
        }
        /// `%` が computed 層に**残らない** position 用 — `font-size` と
        /// `line-height`。
        ///
        /// CSS Values 4 §5.5.1 の既定文が `font-size` を明示的な例外として
        /// 名指ししている ("such as in font-size, which computes its
        /// `<percentage>` values to `<length>`)。`line-height` は CSS Inline 3
        /// §5.1 <https://www.w3.org/TR/css-inline-3/#line-height-property> が
        /// "Percentages: computed relative to 1em" と規定する。実装側は
        /// `crate::resolve` の `resolve_font_size` / `resolve_line_height` が // doc-pointer-lint:ignore: opt-out-3, fn-body-local item doc (nested inside `specified_layer_residue`, itself inside #[cfg(test)] mod tests) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-hrau)
        /// 両方とも `%` を絶対化しており、本 helper 以前の検出器はそれを
        /// computed 層と誤分類していた。
        fn length_absolute_only(l: Length, what: &'static str) -> Option<&'static str> {
            match l {
                Length::Percent(_) => Some(what),
                other => length(other),
            }
        }
        fn length_or_auto(l: LengthOrAuto) -> Option<&'static str> {
            match l {
                LengthOrAuto::Auto => None,
                LengthOrAuto::Length(l) => length(l),
            }
        }
        fn line_height(lh: LineHeight) -> Option<&'static str> {
            match lh {
                // CSS Inline 3 §5.1: `normal` / `<number>` は computed 値のまま。
                LineHeight::Normal | LineHeight::Number(_) => None,
                LineHeight::Length(l) => length_absolute_only(l, "line-height: <percentage>"),
            }
        }
        fn font_weight(fw: FontWeightValue) -> Option<&'static str> {
            match fw {
                FontWeightValue::Absolute(_) => None,
                FontWeightValue::Bolder => Some("font-weight: bolder"),
                FontWeightValue::Lighter => Some("font-weight: lighter"),
            }
        }
        fn text_align(ta: TextAlign) -> Option<&'static str> {
            match ta {
                TextAlign::Start
                | TextAlign::End
                | TextAlign::Left
                | TextAlign::Right
                | TextAlign::Center
                | TextAlign::Justify
                | TextAlign::JustifyAll => None,
                // raikiri-spike-l3wg 以降、`resolve_against_inherited` の
                // phase 2 が必ず解決するため、この arm に**到達すること自体が
                // bug** (かつての「唯一の文書化された例外」ではない —
                // `PageCascadeResult::declarations` の doc も参照)。`Some`
                // のまま残してあるのは意図的な tripwire: 新しい entry point が
                // phase 2 を経由し損ねた場合 (bd raikiri-spike-7m33 gap (b) 相当)
                // に本検出器が拾えるようにするため。raw corpus
                // (`specified_layer_residue_detector_is_not_vacuous`) はまさに
                // この「未解決の raw 値」を検査しているので、`None` に変えると
                // その negative control が意味を失う。
                TextAlign::MatchParent => Some("text-align: match-parent"),
            }
        }
        fn sides<T: Copy>(
            s: Sides<T>,
            f: impl Fn(T) -> Option<&'static str>,
        ) -> Option<&'static str> {
            [s.top, s.right, s.bottom, s.left].into_iter().find_map(f)
        }

        match value {
            PropertyValue::FontWeight(fw) => font_weight(*fw),
            PropertyValue::TextAlign(ta) => text_align(*ta),
            PropertyValue::LineHeight(lh) => line_height(*lh),
            // `font-size` だけは `%` も残滓 (§5.5.1 の明示的例外)。
            PropertyValue::FontSize(l) => length_absolute_only(*l, "font-size: <percentage>"),
            // `larger` / `smaller` — `font_weight` の `Bolder`/`Lighter` と同型
            // (raikiri-spike-4rmu): 常に未解決の残滓。`page_corpus` の worst-case
            // payload としても使う。
            PropertyValue::FontSizeRelative(_) => Some("font-size: larger/smaller"),
            PropertyValue::PaddingTop(l)
            | PropertyValue::PaddingRight(l)
            | PropertyValue::PaddingBottom(l)
            | PropertyValue::PaddingLeft(l)
            | PropertyValue::BorderTopWidth(l)
            | PropertyValue::BorderRightWidth(l)
            | PropertyValue::BorderBottomWidth(l)
            | PropertyValue::BorderLeftWidth(l) => length(*l),
            PropertyValue::MarginTop(l)
            | PropertyValue::MarginRight(l)
            | PropertyValue::MarginBottom(l)
            | PropertyValue::MarginLeft(l)
            | PropertyValue::Width(l)
            | PropertyValue::Height(l) => length_or_auto(*l),
            PropertyValue::Padding(s) => sides(*s, length),
            PropertyValue::Margin(s) => sides(*s, length_or_auto),
            PropertyValue::Border(s) => sides(*s, |b: Border| length(b.width)),
            // 層に依存しない payload — keyword / color / ident list / counter。
            PropertyValue::Color(_)
            | PropertyValue::BackgroundColor(_)
            | PropertyValue::FontFamily(_)
            | PropertyValue::Display(_)
            | PropertyValue::CounterReset(_)
            | PropertyValue::CounterIncrement(_)
            | PropertyValue::CounterSet(_)
            | PropertyValue::Content(_)
            | PropertyValue::StringSet(_)
            | PropertyValue::Position(_)
            | PropertyValue::Direction(_)
            | PropertyValue::BorderTopStyle(_)
            | PropertyValue::BorderRightStyle(_)
            | PropertyValue::BorderBottomStyle(_)
            | PropertyValue::BorderLeftStyle(_)
            | PropertyValue::BorderTopColor(_)
            | PropertyValue::BorderRightColor(_)
            | PropertyValue::BorderBottomColor(_)
            | PropertyValue::BorderLeftColor(_)
            | PropertyValue::BoxSizing(_) => None,
        }
    }

    /// `%` が computed 層に残らない 2 つの position を検出器が取りこぼさないこと。
    ///
    /// `page_corpus` は `PropertyKey` あたり payload を 1 つしか持てない
    /// (`sample_for` が 1 key → 1 value の関数のため) ので、この 2 payload は
    /// corpus ではなく検出器を直接叩く。corpus 側の `Em` payload は据え置き
    /// なので raw residue の数え上げにも影響しない。
    #[test]
    fn percentage_is_specified_layer_residue_on_font_size_and_line_height() {
        assert_eq!(
            specified_layer_residue(&PropertyValue::FontSize(Length::Percent(150.0))),
            Some("font-size: <percentage>"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::LineHeight(LineHeight::Length(
                Length::Percent(150.0)
            ))),
            Some("line-height: <percentage>"),
        );
        // 対照 — box property では `%` が computed 値 (CSS Values 4 §5.5.1 既定)。
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Percent(50.0))),
            None,
        );
    }

    /// bd raikiri-spike-2x8 で追加した font-relative / absolute unit も
    /// `Em` / `Rem` / `Pt` と同じく「絶対化前は specified 層の残滓」として
    /// 検出される (`length()` inner helper の網羅 match — 新 variant 追加は
    /// compile error で強制されるが、各 arm の到達は compile では保証されない
    /// ため個別に exercise する)。
    #[test]
    fn additional_length_units_are_specified_layer_residue() {
        // straight-line asserts (no loop + lazy custom message) so every
        // comparison is unconditionally exercised under patch coverage —
        // a `assert_eq!(.., "{l:?}")` message argument is only evaluated on
        // failure, which would leave that formatting code permanently
        // uncovered by an all-passing loop.
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Ex(1.0))),
            Some("Length::Ex"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Rex(1.0))),
            Some("Length::Rex"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Ch(1.0))),
            Some("Length::Ch"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Rch(1.0))),
            Some("Length::Rch"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Ic(1.0))),
            Some("Length::Ic"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Ric(1.0))),
            Some("Length::Ric"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Cm(1.0))),
            Some("Length::Cm"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Mm(1.0))),
            Some("Length::Mm"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Q(1.0))),
            Some("Length::Q"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::In(1.0))),
            Some("Length::In"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Pc(1.0))),
            Some("Length::Pc"),
        );
        // bd raikiri-spike-vxha.
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Lh(1.0))),
            Some("Length::Lh"),
        );
        assert_eq!(
            specified_layer_residue(&PropertyValue::PaddingTop(Length::Rlh(1.0))),
            Some("Length::Rlh"),
        );
    }

    /// `page_corpus` に重複 variant が無く、`sample_for` の各 arm が自分の
    /// key と一致する `PropertyValue` を返すこと (raikiri-spike-a754)。
    ///
    /// 以前の本 test は手で持つ `PROPERTY_VALUE_VARIANTS` (単なる数) と
    /// `corpus.len()` を比較していたが、両者は互いにしか照合されておらず
    /// (bd raikiri-spike-awjx §8.2 debt lens M4 が `PropertyValue::Orphan` で
    /// 実証)、新 variant が両方同じ数のまま corpus 外に残るケースを検出
    /// できなかった。raikiri-spike-a754 で `PROPERTY_VALUE_VARIANTS` を廃止し
    /// `page_corpus` を `sample_for` 駆動に変えたので、その旧チェックは
    /// **常に真になる同語反復** (`corpus.len()` は `ALL_PROPERTY_KEYS.len() +
    /// key_sharing_extras().len()` の定義から出てくる) になり、削除した。
    ///
    /// 本 test が「corpus 完全性」自体は保証しない件の canonical な記述は
    /// `page_corpus` 手前の section comment にある。本 test 自身が見ている
    /// のは 2 つの**内部整合性**だけ:
    ///
    /// - discriminant の重複が無いこと (`std::mem::discriminant` — payload の
    ///   trait bound に依存せず variant のみを区別する)。`PropertyValue::key()`
    ///   はもう使えない — raikiri-spike-4rmu で `FontSizeRelative` /
    ///   `FontSize` が意図的に `PropertyKey::FontSize` を共有し単射性が崩れた
    ///   ため。
    /// - `sample_for(key).key() == key` — arm の中身が自分の key と食い違って
    ///   いないこと (コピペミス class の検出。`property_key_samples!` マクロ
    ///   はこの一貫性まで保証しない — マクロは token 列を lhs/rhs にそのまま
    ///   展開するだけで、rhs の式が lhs の `PropertyKey` に対応する variant を
    ///   実際に construct しているかは見ていない)。
    #[test]
    fn page_corpus_has_no_duplicate_or_mismatched_samples() {
        let corpus = page_corpus();
        let discriminants: std::collections::HashSet<std::mem::Discriminant<PropertyValue>> =
            corpus.iter().map(std::mem::discriminant).collect();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            discriminants.len(),
            corpus.len(),
            "page_corpus に同じ variant が 2 度現れている (discriminant が重複)",
        );
        for key in ALL_PROPERTY_KEYS.iter().copied() {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                sample_for(key).key(),
                key,
                "sample_for({key:?}) が別の PropertyKey の value を返している",
            );
        }
    }

    /// `FontSize` / `FontSizeRelative` が意図的に同じ `PropertyKey` を共有する
    /// こと自体の direct pin (`page_corpus_has_no_duplicate_or_mismatched_samples`
    /// の doc が説明する単射性崩れの根拠)。property.rs 側の
    /// `font_size_relative_shares_property_key_with_font_size` と同じ主張を
    /// page 経路の corpus に対して確認する — corpus の 2 entry が同じ key を
    /// 持つことは bug ではなく仕様であると明示する。
    #[test]
    fn page_corpus_font_size_and_font_size_relative_share_one_key() {
        let corpus = page_corpus();
        let font_size_key_count = corpus
            .iter()
            .filter(|v| v.key() == PropertyKey::FontSize)
            .count();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            font_size_key_count, 2,
            "FontSize と FontSizeRelative の 2 entry が PropertyKey::FontSize を共有するはず",
        );
    }

    /// phase 3 の「素通し」分類が doc の数え上げと一致すること。
    ///
    /// bd raikiri-spike-sshp が doc に書いた「21」が実測 22 だった drift の
    /// 再発 pin。分類 (どの arm にどの variant を置くか) を動かすと落ちる。
    ///
    /// ⚠️ **射程**: 判定は `out == value` なので、pin しているのは arm の所属
    /// ではなく「`page_corpus` の payload に対する挙動」である。corpus が
    /// payload を variant あたり 1 つしか持たない以上、`border_styles` を
    /// `Sides::all(Solid)` に固定した本 test は **style gate 自体を pin しない**
    /// (gate は `cascade_page_border_width_*` の 4 本が持つ)。同様に `%` /
    /// `auto` / `line-height: <number>` の挙動も本 test の射程外で、それぞれ
    /// 専用の acceptance test がある。
    ///
    /// `page_corpus()` は phase 2 を通していない raw payload を含む
    /// (`FontWeight::Bolder` / `TextAlign::MatchParent` 等) — 本 test はそれを
    /// **意図的に** phase 3 へ直接投入して分類する。bd raikiri-spike-7m33 で
    /// `absolutize_in_page_context` の引数が `ResolvedAgainstInherited` に
    /// なったため、`ResolvedAgainstInherited::for_test` (`#[cfg(test)]` 限定)
    /// を経由してこの raw payload を包む。
    ///
    /// 「変換」側の数 (`phase_3_transformed_variants()`) は独立には assert
    /// しない (raikiri-spike-a754 debt lens) — `unchanged + 変換された数 ==
    /// page_corpus().len()` の恒等式と `phase_3_transformed_variants() ==
    /// page_corpus().len() - PHASE_3_PASS_THROUGH_VARIANTS` の定義から、下の
    /// `unchanged` の assert が通った時点で自動的に真になる算術的同語反復
    /// になるため。`phase_3_transformed_variants()` 自体は
    /// `raw_corpus_residue_variants()` から引き続き参照される。
    #[test]
    fn phase_3_variant_classification_matches_the_documented_counts() {
        let font_size = ComputedLength(20.0);
        let ctx = ResolveContext::new(ComputedLength(16.0));
        // `Solid` にしておかないと border-*-width が style gate で `0px` に
        // 潰れ、「変換された」判定が gate 由来か絶対化由来か区別できない。
        let styles = Sides::all(BorderStyle::Solid);

        let unchanged = page_corpus()
            .into_iter()
            .filter(|value| {
                absolutize_in_page_context(
                    ResolvedAgainstInherited::for_test(value.clone()),
                    font_size,
                    None,
                    &ctx,
                    styles,
                ) == *value
            })
            .count();
        assert_eq!(
            unchanged, PHASE_3_PASS_THROUGH_VARIANTS,
            "phase 3 の pass-through arm が覆う variant 数が doc とずれた",
        );
    }

    /// **本節の中心 pin** — phase 2 → phase 3 を通した後、`declarations` に
    /// 届く値に specified 層残滓は**一切残らない**。
    ///
    /// `PageCascadeResult::declarations` の doc が consumer に宣言している
    /// 「These are computed values, with no documented exception」そのもの。
    /// raikiri-spike-l3wg 以前は `text-align: match-parent` が唯一の例外
    /// だった (旧 test 名
    /// `page_declarations_carry_exactly_one_specified_layer_residue`) — 本
    /// task がそれを解消したので期待値を空 `vec![]` に変えた。例外が復活したら
    /// ここで落ち、doc を直させる。
    #[test]
    fn page_declarations_carry_no_specified_layer_residue() {
        let root = root_with_font_size(16.0);
        let font_size = ComputedLength(20.0);
        let ctx = ResolveContext::new(root.font_size);
        let styles = Sides::all(BorderStyle::Solid);

        let residues: Vec<(PropertyKey, &'static str)> = page_corpus()
            .into_iter()
            .map(|v| resolve_against_inherited(v, &root, &ctx))
            .map(|v| absolutize_in_page_context(v, font_size, None, &ctx, styles))
            .filter_map(|v| specified_layer_residue(&v).map(|r| (v.key(), r)))
            .collect();

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            residues,
            Vec::<(PropertyKey, &'static str)>::new(),
            "PageCascadeResult::declarations に specified 層残滓が残ってはならない (raikiri-spike-l3wg 以降、documented exception は無い)",
        );
    }

    /// 同じ規則を **`cascade_page` の出力そのもの** に対して確かめる。
    ///
    /// 上の test は phase 2 ∘ phase 3 を直接合成しているので、`cascade_page`
    /// が phase 3 を呼ばなくなっても落ちない。契約が書かれているのは
    /// `PageCascadeResult::declarations` = `cascade_page` の戻り値なので、
    /// 主語を合わせた pin をもう 1 本置く。bd raikiri-spike-7m33 で
    /// `resolve_against_inherited` → `absolutize_in_page_context` の合成順序
    /// 自体は `ResolvedAgainstInherited` 型で強制されるようになったが、
    /// 「本関数群を一切呼ばない新しい entry point」までは型で数え上げられない
    /// (7m33 は narrow しただけで close していない) ので、既存経路
    /// (`cascade_page`) の wiring だけでも押さえておく。
    #[test]
    fn cascade_page_output_carries_no_specified_layer_residue() {
        let root = root_with_font_size(16.0);
        let result = page(
            "@page { font-size: 2em; font-weight: bolder; line-height: 1.5em; \
             padding: 2em; margin: 3rem; border: 12pt solid red; \
             width: 4em; height: 5em; text-align: match-parent; direction: rtl }",
            &root,
        );
        // vacuity guard — stylesheet が黙って落ちていないこと。
        //
        // exact count で持つ。`>= N` 形だと `border` shorthand の 12 longhand が
        // 丸ごと落ちる parse regression が起きても残りで閾値を超えてしまい、
        // かつ落ちた分は residue も 0 なので本 pin が素通りする。
        //
        // 27 = font-size / font-weight / line-height / text-align / width /
        // height / direction の 7 + padding 4 + margin 4 + `border` shorthand
        // の展開 12 (4 side × width / style / color)。`@page` の shorthand 展開が
        // 変わったらここが先に落ちる — 失敗時の意味: corpus stylesheet が
        // 期待通り parse / 展開されていない。
        //
        // 診断文言は (custom message ではなく) この comment 側に置く:
        // `assert_eq!` の custom message 引数は assertion 失敗時のみ評価され
        // る cold path なので、test が pass する限り gate §8.1.1 patch
        // coverage 上 uncovered 扱いになる (bd raikiri-spike-sxd7 と同型)。
        assert_eq!(result.declarations().len(), 27);

        // `declarations` は HashMap-random 順なので sort して比較する。
        let mut residues: Vec<String> = result
            .declarations()
            .values()
            .filter_map(|v| specified_layer_residue(v).map(|r| format!("{:?}: {r}", v.key())))
            .collect();
        residues.sort();
        // raikiri-spike-l3wg 以前はここに `TextAlign: text-align: match-parent`
        // が 1 件残っていた (関数名が予告していた「no residue」と実際の
        // assertion が食い違っていた quirk) — 今は名前どおり空になる。
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            residues,
            Vec::<String>::new(),
            "cascade_page の出力に specified 層残滓が居る",
        );
    }

    /// `page_declarations_carry_no_specified_layer_residue` と
    /// `cascade_page_output_carries_no_specified_layer_residue` が vacuous で
    /// ないこと (negative control) — 検出器は phase 2 / phase 3 を通していない
    /// **raw** 値に対しては実際に発火する (`text-align: match-parent` を含む —
    /// raikiri-spike-l3wg 以降も raw corpus はまだ resolve 前なので、この
    /// negative control 自体は変わらない)。
    #[test]
    fn specified_layer_residue_detector_is_not_vacuous() {
        let raw = page_corpus()
            .iter()
            .filter(|v| specified_layer_residue(v).is_some())
            .count();
        assert_eq!(
            raw,
            raw_corpus_residue_variants(),
            "corpus の worst-case payload が specified 層残滓として検出されない \
             — 検出器か corpus のどちらかが骨抜きになっている",
        );
    }

    /// Phase 3 must leave every computed-equivalent value untouched — the
    /// pass-through arm covers `PHASE_3_PASS_THROUGH_VARIANTS` of
    /// `page_corpus`'s entries (the rest are transformed) and a wrong
    /// classification there would corrupt a value rather than merely leave it
    /// unresolved. That count is only a stand-in for "of the `PropertyValue`
    /// variants" while `page_corpus` stays complete — completeness is no
    /// longer independently checked (see the section comment above
    /// `page_corpus`; tracked at bd raikiri-spike-c0z9). The counts
    /// themselves are pinned by
    /// `phase_3_variant_classification_matches_the_documented_counts`; this
    /// test drives the same rule end-to-end through `cascade_page`.
    #[test]
    fn cascade_page_computed_equivalent_values_pass_phase_3_unchanged() {
        let root = root_with_weight(700.0);
        let result = page(
            "@page { color: red; font-weight: bolder; display: block; \
             box-sizing: border-box; border-top-color: red; text-align: center; \
             direction: rtl }",
            &root,
        );
        assert_eq!(color_of(&result), Some(RED));
        assert_eq!(
            result.declarations().get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BoxSizing),
            Some(&PropertyValue::BoxSizing(
                crate::property::BoxSizing::BorderBox
            )),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Center)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::Direction),
            Some(&PropertyValue::Direction(Direction::Rtl)),
        );
    }

    /// `@page { text-align: match-parent }` now resolves against the root
    /// element's computed `text-align` + `direction` (raikiri-spike-l3wg) —
    /// this used to be the crate's one documented specified-layer exception
    /// (`cascade_page_text_align_match_parent_passes_through_as_specified_value`,
    /// asserting `TextAlign::MatchParent` survived unresolved). CSS Text 3
    /// §6.1 `#valdef-text-align-match-parent` verbatim: "an inherited value
    /// of start or end is interpreted against the parent's direction value".
    #[test]
    fn cascade_page_text_align_match_parent_resolves_against_root_direction() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);

        // Default root (`ComputedValues::initial()`): text-align = start,
        // direction = ltr → left.
        let ltr_root = root_with_weight(700.0);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&ltr_root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Left)),
        );

        // Root with `direction: rtl` (text-align still start) → right.
        let rtl_root = ComputedValues {
            direction: Direction::Rtl,
            ..ComputedValues::initial()
        };
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&rtl_root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Right)),
        );
    }

    /// The trap the doc warns about: unlike the element path's root element
    /// (CSS Text 3 §6.1's "computes to start"), the page context's
    /// `PageInheritance::LegacyInitialValues` L3 legacy exception is **not**
    /// a "no parent" case — it substitutes `ComputedValues::initial()` as an
    /// ordinary inheritance parent (text-align = start, direction = ltr) and
    /// goes through the same parent-direction table, landing on `left` rather
    /// than being short-circuited to `start`.
    #[test]
    fn cascade_page_text_align_match_parent_legacy_initial_values_not_start_shortcut() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Left)),
            "the L3 legacy-exception initial-values parent must still go through the parent-direction table, not the root-element \"computes to start\" shortcut"
        );
    }

    /// Page-path analogue of `cascade::tests::
    /// text_align_match_parent_uses_parent_direction_not_own_direction_winner`
    /// (that test's doc calls itself "the end-to-end pin for the whole
    /// `direction` + `text-align: match-parent` design" — this is the same
    /// pin for the *second, independent* resolution site,
    /// `resolve_against_inherited`'s `TextAlign` arm, which the element-path
    /// test cannot exercise).
    ///
    /// `@page` here declares its own (conflicting) `direction: rtl` on the
    /// page context itself. Per CSS Text 3 §6.1
    /// `#valdef-text-align-match-parent` ("interpreted against **the
    /// parent's** direction value"), resolution must use the *root element's*
    /// `ltr`, not the page context's own `rtl`. Without this test, a
    /// regression that made `resolve_against_inherited` read the page
    /// context's own `direction` winner instead of `inherited.direction`
    /// would flip `Left` → `Right` here while every other test in this
    /// module — including the residue-count pins — stayed green (none of
    /// them cross own-direction with match-parent on the page path).
    #[test]
    fn cascade_page_text_align_match_parent_ignores_page_context_own_direction() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { direction: rtl; text-align: match-parent }",
            Origin::Author,
        );
        // Root: text-align = start, direction = ltr (defaults).
        let root = ComputedValues::initial();
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Left)),
        );
        // The page context's own `direction: rtl` winner is unaffected — it
        // is a separate property, unrelated to the match-parent resolution.
        assert_eq!(
            result.declarations().get(&PropertyKey::Direction),
            Some(&PropertyValue::Direction(Direction::Rtl)),
        );
    }

    #[test]
    fn cascade_page_text_align_match_parent_copies_non_start_end_root_value() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);
        let root = ComputedValues {
            text_align: TextAlign::Center,
            ..ComputedValues::initial()
        };
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::FromRoot(&root),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::Center)),
        );
    }

    #[test]
    fn cascade_page_query_default_matches_only_default_rule() {
        // Default `PageContextQuery::default()` = unnamed + all pseudos false.
        // Named / pseudo rules must not match; only `@page { … }` applies.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: green } \
             @page :first { color: red } \
             @page cover { color: blue }",
            Origin::Author,
        );
        let result = cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        );
        assert_eq!(color_of(&result), Some(GREEN));
    }

    #[test]
    fn cascade_page_result_default_is_empty() {
        // `PageCascadeResult::default()` is the empty result (no rules
        // matched) — used by consumers that need a placeholder.
        let r = PageCascadeResult::default();
        assert!(r.declarations().is_empty());
    }

    #[test]
    fn cascade_page_result_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<PageCascadeResult>();
    }

    // ── post-parse shorthand injection into `@page` (bd raikiri-spike-3svx) ──
    //
    // `add_stylesheet` の**後**に declaration を shorthand variant へ書き戻す
    // post-parse mutation 経路。`crate::rule::parse_declaration_block` の
    // parse-time 展開はこの経路を守らない。element 経路の同形 gap を塞いだのが
    // bd raikiri-spike-nqkj (`crate::cascade` の `collect_cascaded`)、`@page`
    // 経路 = `cascade_page` を塞いだのが bd raikiri-spike-3svx。
    //
    // `RuleTree.page_rules` / `PageRule.declarations` は bd raikiri-spike-qzn3
    // の後も `pub` field である (element 経路と違い可視性では閉じない)。何が
    // 閉じていて何が開いているかは `crate::rule::expand_shorthand_into` doc の
    // 「2 と 3 で閉じている範囲が違う」節が canonical — ここには再掲しない
    // (再掲は既に 2 度 drift した — bd raikiri-spike-awjx)。
    //
    // 守るべき spec は 2 条:
    //
    // - CSS Cascading L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand>
    //   "A shorthand property sets all of its longhand sub-properties, exactly
    //   as if expanded in place."
    // - CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
    //   "Order of Appearance: … the last declaration in document order wins."
    //
    // ⚠️ **失敗の形は element 経路と異なる** (silent drop になる理由は
    // `crate::rule::expand_shorthand_into` doc の「2 と 3 で破れ方が違う」節)。
    // 展開しないと shorthand winner が `PropertyKey::Margin` 等の独立 key に
    // park するため、各 family test は「shorthand key が結果に存在しないこと」を
    // 直接 assert する — element 経路には書けなかった強い guard である。
    //
    // 各 test の comment にある「展開の有無を区別する / しない」の判定は、
    // `cascade_page` の展開 hunk を revert した状態での実測に基づく
    // (bd raikiri-spike-3svx gate §8.2、`crate::cascade` の `post_parse_*` 群が
    // 採ったのと同じ hunk-revert 法)。

    /// nqkj の element 経路 helper (`cascade::tests::
    /// cascade_with_post_parse_injection`) の `@page` 版。
    ///
    /// `css` を `RuleTree::add_stylesheet` で parse したあと、
    /// `page_rules[0].declarations[idx].value` を `injected` に差し替え、
    /// `cascade_page` を通した結果を返す。
    ///
    /// この手つきは **3svx が報告した当時の Consumer 経路**そのもの。qzn3 で
    /// `Declaration::value` が `pub(crate)` になったので、もう crate 外からは
    /// 書けない。
    ///
    /// `PageRule` を直接 literal 構築しないのが要点 — `#[non_exhaustive]` は
    /// crate 内構築を妨げないので、literal だと **報告された経路とは別の経路**を
    /// test してしまう。
    ///
    /// ⚠️ 差し替えるのは `.value` **だけ**なので、`css` の `idx` 番目の
    /// declaration は 2 役を持つ:
    ///
    /// 1. **値は捨てられる** — 単に「その位置に declaration を 1 個作る」ための
    ///    placeholder (以降の assert に一切現れない `99px` を置いている)。
    /// 2. **`important` flag は生き残り、injected shorthand に載る** — 展開時に
    ///    各 longhand へ copy される。これを利用して「注入 shorthand は normal」
    ///    を作っているのが
    ///    `post_parse_page_important_longhand_survives_later_normal_shorthand`。
    ///
    /// `PageInheritance::LegacyInitialValues` = L3 legacy exception (initial values に解決)。
    /// 本節の fixture は全て `px` なので phase 2 / phase 3 は恒等写像になる。
    fn page_with_post_parse_injection(
        css: &str,
        idx: usize,
        injected: PropertyValue,
    ) -> PageCascadeResult {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        tree.page_rules[0].declarations[idx].value = injected;
        cascade_page(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
        )
    }

    /// 4 side が全て互いに異なり、かつ initial (0) とも異なる margin fixture。
    /// `Sides::all` だと「1 side しか展開していない」実装と「4 side 展開した」
    /// 実装が区別できず、0 を使うと initial と区別できない
    /// (`cascade::tests::distinct_margin_sides` と同じ意図)。
    fn distinct_page_margin_sides() -> Sides<LengthOrAuto> {
        Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        }
    }

    /// `key` 位置に入っている margin longhand の px 値。4 variant を or-pattern で
    /// 畳んでいるのは、explicit な `PropertyValue` assert だと 4 side × 4 test で
    /// 16 site に膨らむため。key ↔ variant が食い違う形の bug は本 helper では
    /// 検出できないが、`cascade_page` の key は `PropertyValue::key()` 由来であり
    /// (winner selection の `let key = value.key();`)、その対応は
    /// `crate::property` の `margin_longhand_keys_map_correctly` が pin 済み。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (non-#[test] mod-level helper doc) — rustdoc-blind, confirmed via わざと壊して確かめる (bd raikiri-spike-csmj; demoted from an already-linked span by bd raikiri-spike-gq7x)
    /// border 側に同型 helper を置かず explicit assert にしてあるのは、3
    /// sub-property family ぶんの helper が要るのに対し assert が 12 個で済むため。
    fn margin_px(result: &PageCascadeResult, key: PropertyKey) -> Option<f32> {
        match result.declarations().get(&key) {
            Some(
                PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(v)))
                | PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(v)))
                | PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(v)))
                | PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(v))),
            ) => Some(*v),
            _ => None,
        }
    }

    /// `margin_px` の padding 版 (or-pattern の是非は同 doc を参照)。
    fn padding_px(result: &PageCascadeResult, key: PropertyKey) -> Option<f32> {
        match result.declarations().get(&key) {
            Some(
                PropertyValue::PaddingTop(Length::Px(v))
                | PropertyValue::PaddingRight(Length::Px(v))
                | PropertyValue::PaddingBottom(Length::Px(v))
                | PropertyValue::PaddingLeft(Length::Px(v)),
            ) => Some(*v),
            _ => None,
        }
    }

    #[test]
    fn post_parse_page_margin_shorthand_before_longhand_lets_longhand_win() {
        // 注入後の declaration 列 (= `margin: 1px 2px 3px 4px; margin-top: 10px`):
        //   `[0]` Margin(1,2,3,4)   ← 注入
        //   `[1]` MarginTop(10px)
        // spec §3 + §6.1 → top=10 (後方 longhand)、right/bottom/left=2/3/4。
        //
        // **展開の有無を区別する**: 展開しない実装では `PropertyKey::Margin` が
        // 独立 winner として park し、`MarginRight` 等は結果に現れない
        // (top だけが 10px で残る silent drop)。
        let result = page_with_post_parse_injection(
            "@page { margin-left: 99px; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_page_margin_sides()),
        );
        assert_eq!(
            margin_px(&result, PropertyKey::MarginTop),
            Some(10.0),
            "後方 longhand が order of appearance で勝つこと (§6.1)"
        );
        // 残り 3 side は shorthand 由来の per-side 値。全て assert するのは
        // 「shorthand を単に落とす」実装と「top arm だけ展開する」実装を弾くため。
        assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
        assert!(
            !result.declarations().contains_key(&PropertyKey::Margin),
            "shorthand key が独立 slot に park してはならない (silent drop の直接 pin)"
        );
    }

    #[test]
    fn post_parse_page_margin_shorthand_after_longhand_lets_shorthand_win() {
        // 鏡像方向 (`margin-top: 10px; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px)
        //   `[1]` Margin(1,2,3,4)   ← 注入
        // spec §6.1 → 全 side が shorthand 由来 = 1/2/3/4。
        //
        // **展開の有無を区別する** — element 経路の同名 test
        // (`post_parse_margin_shorthand_after_longhand_lets_shorthand_win`) は
        // `PropertyKey` 宣言順のせいで偶然 pass する弱い guard だったが、
        // `@page` では shorthand が独立 key に park するので top は 10px のまま
        // 残り (spec は 1px)、区別できる。
        let result = page_with_post_parse_injection(
            "@page { margin-top: 10px; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_page_margin_sides()),
        );
        assert_eq!(margin_px(&result, PropertyKey::MarginTop), Some(1.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
        assert!(!result.declarations().contains_key(&PropertyKey::Margin));
    }

    #[test]
    fn post_parse_page_padding_shorthand_before_longhand_lets_longhand_win() {
        // margin と同じ形を padding family でも pin (展開 arm が family ごとに
        // 独立に書かれているため)。1/2/3/4px の意図は
        // `distinct_page_margin_sides` doc と同じ。**展開の有無を区別する**。
        let result = page_with_post_parse_injection(
            "@page { padding-left: 99px; padding-top: 10px }",
            0,
            PropertyValue::Padding(Sides {
                top: Length::Px(1.0),
                right: Length::Px(2.0),
                bottom: Length::Px(3.0),
                left: Length::Px(4.0),
            }),
        );
        assert_eq!(padding_px(&result, PropertyKey::PaddingTop), Some(10.0));
        assert_eq!(padding_px(&result, PropertyKey::PaddingRight), Some(2.0));
        assert_eq!(padding_px(&result, PropertyKey::PaddingBottom), Some(3.0));
        assert_eq!(padding_px(&result, PropertyKey::PaddingLeft), Some(4.0));
        assert!(!result.declarations().contains_key(&PropertyKey::Padding));
    }

    #[test]
    fn post_parse_page_border_shorthand_before_longhand_lets_longhand_win() {
        // border は 4 side × 3 sub-property = 12 longhand に展開される。
        // width / style を per-side で全て違う値にして、12 arm が sink 経由でも
        // 落ちていないことを pin する。
        //
        // **展開の有無を区別する**、しかも二重に: 展開しないと (a) 12 longhand
        // が結果に現れず、(b) `page_context_border_styles` が style longhand を
        // 1 つも見つけられないので initial `none` と判定し、残った
        // `border-top-width: 10px` すら §3.3 の style gate で 0px に潰れる。
        let result = page_with_post_parse_injection(
            "@page { border-left-width: 99px; border-top-width: 10px }",
            0,
            PropertyValue::Border(Sides {
                top: Border {
                    width: Length::Px(1.0),
                    style: BorderStyle::Solid,
                    color: BorderColor::Resolved(RED),
                },
                right: Border {
                    width: Length::Px(2.0),
                    style: BorderStyle::Dashed,
                    color: BorderColor::Resolved(BLUE),
                },
                bottom: Border {
                    width: Length::Px(3.0),
                    style: BorderStyle::Dotted,
                    color: BorderColor::CurrentColor,
                },
                left: Border {
                    width: Length::Px(4.0),
                    style: BorderStyle::Double,
                    color: BorderColor::Resolved(RED),
                },
            }),
        );
        // top.width だけ後方 longhand が勝つ。
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopWidth),
            Some(&PropertyValue::BorderTopWidth(Length::Px(10.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderRightWidth),
            Some(&PropertyValue::BorderRightWidth(Length::Px(2.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderBottomWidth),
            Some(&PropertyValue::BorderBottomWidth(Length::Px(3.0))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderLeftWidth),
            Some(&PropertyValue::BorderLeftWidth(Length::Px(4.0))),
        );
        // style / color は shorthand 由来のまま per-side に残る。
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopStyle),
            Some(&PropertyValue::BorderTopStyle(BorderStyle::Solid)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderRightStyle),
            Some(&PropertyValue::BorderRightStyle(BorderStyle::Dashed)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderBottomStyle),
            Some(&PropertyValue::BorderBottomStyle(BorderStyle::Dotted)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderLeftStyle),
            Some(&PropertyValue::BorderLeftStyle(BorderStyle::Double)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderTopColor),
            Some(&PropertyValue::BorderTopColor(BorderColor::Resolved(RED))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderRightColor),
            Some(&PropertyValue::BorderRightColor(BorderColor::Resolved(
                BLUE
            ))),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderBottomColor),
            Some(&PropertyValue::BorderBottomColor(BorderColor::CurrentColor)),
        );
        assert_eq!(
            result.declarations().get(&PropertyKey::BorderLeftColor),
            Some(&PropertyValue::BorderLeftColor(BorderColor::Resolved(RED))),
        );
        assert!(!result.declarations().contains_key(&PropertyKey::Border));
    }

    #[test]
    fn post_parse_page_shorthand_injection_propagates_important() {
        // 展開時の `!important` copy (spec §3 verbatim: "Declaring a shorthand
        // property to be !important is equivalent to declaring all of its
        // sub-properties to be !important.") が `@page` 入口の展開でも保たれる
        // こと。注入した shorthand は placeholder の `!important` を継承するので
        // 後方の normal longhand には**負けない**。
        //
        // **展開の有無を区別する** (element 経路の同名 test は区別しなかった) —
        // 展開しないと `MarginTop` は 10px のまま残り、1/2/3/4 のどれも現れない。
        let result = page_with_post_parse_injection(
            "@page { margin-left: 99px !important; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_page_margin_sides()),
        );
        assert_eq!(
            margin_px(&result, PropertyKey::MarginTop),
            Some(1.0),
            "important shorthand 由来の MarginTop が normal longhand に勝つこと"
        );
        assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
        assert!(!result.declarations().contains_key(&PropertyKey::Margin));
    }

    #[test]
    fn post_parse_page_important_longhand_survives_later_normal_shorthand() {
        // 注入後の declaration 列
        // (= `margin-top: 10px !important; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px) !important
        //   `[1]` Margin(1,2,3,4)  normal   ← 注入 (`important` は false のまま)
        //
        // CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
        // の cascade sort は Origin and Importance を Order of Appearance
        // **より上位**に置く。したがって後方の normal shorthand は前方の
        // important longhand に勝てない → top=10。残り 3 side は shorthand 由来
        // = 2/3/4。
        //
        // **展開の有無を区別する** — ただし区別しているのは下の 3 side assert と
        // `contains_key` assert であり、headline の `MarginTop == 10.0` (§6.1) は
        // 展開しない実装でも pass する (`MarginTop(10px) !important` がそのまま
        // 独立 winner として残るため)。3 side assert を「冗長」として削らないこと。
        let result = page_with_post_parse_injection(
            "@page { margin-top: 10px !important; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_page_margin_sides()),
        );
        assert_eq!(
            margin_px(&result, PropertyKey::MarginTop),
            Some(10.0),
            "important longhand が後方の normal shorthand 由来 longhand に勝つこと \
             (§6.1 Origin and Importance)"
        );
        assert_eq!(margin_px(&result, PropertyKey::MarginRight), Some(2.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginBottom), Some(3.0));
        assert_eq!(margin_px(&result, PropertyKey::MarginLeft), Some(4.0));
        assert!(!result.declarations().contains_key(&PropertyKey::Margin));
    }

    #[test]
    fn cascade_page_deterministic_across_10_runs() {
        // Sibling of `cascade::tests::cascade_with_ua_deterministic_across_10_runs`.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@page { color: red } \
             @page :first { color: blue } \
             @page cover:first { color: green }",
            Origin::Author,
        );
        let query = PageContextQuery {
            page_name: Some(Atom::from("cover")),
            is_first: true,
            ..Default::default()
        };
        let baseline = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
        for _ in 0..9 {
            let run = cascade_page(&tree, &query, PageInheritance::LegacyInitialValues);
            assert_eq!(
                run.declarations().get(&PropertyKey::Color),
                baseline.declarations().get(&PropertyKey::Color),
            );
        }
    }
}
