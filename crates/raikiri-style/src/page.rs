//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3.
//!
//! # Status
//!
//! The rule tree stores parsed `@page` rules and [`cascade_page`] resolves the
//! winning declarations for a given page context (origin / specificity /
//! source-order cascade + resolution against the page context's inheritance
//! parent — see that function's doc). Per-page `PageBox` derivation and
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
use crate::cascade::{cascade_rank, resolve_against_inherited};
use crate::computed::ComputedValues;
use crate::property::{PropertyKey, PropertyValue};
use crate::rule::Declaration;
use crate::ruletree::{Origin, RuleTree};

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
// [`crate::cascade::collect_cascaded`] + [`crate::cascade::pick_winners`]:
// per-candidate `(value, important, origin, specificity, source_order)` tuple,
// group by [`PropertyKey`], pick winner by `(rank, specificity, source_order)`
// where higher tuples beat lower. `rank` reuses [`cascade_rank`] verbatim —
// `@page` rules and style rules share the same origin ordering (spec §6.2).
// The only diverging element is the specificity type: [`PageSpecificity`] is a
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
/// declared. Unsupported properties (e.g. `size`, `margin`, `marks` — M1.4
/// property parser silently drops these; see
/// `ruletree::tests::page_body_unsupported_property_drops_declaration`) are
/// absent. Downstream page-layout code is expected to translate this bag into
/// its page-box model and future `@page`-descriptor fields when the M4
/// descriptor property parser lands; raikiri-style remains a leaf crate.
///
/// Iteration order over `declarations` is `HashMap`-random; consumers that
/// need a deterministic order should sort or look up by [`PropertyKey`]
/// (Tests here look up by key rather than iterating).
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageCascadeResult {
    /// Winning `(key, value)` per property, after the resolution pass described
    /// below.
    ///
    /// `font-weight`'s `bolder` / `lighter` relative keywords are resolved
    /// against the inherited weight here, so **no relative font-weight**
    /// sentinel reaches the consumer. The inheritance parent is the
    /// `root_style` argument of [`cascade_page`]; CSS Paged Media 3 §6 "Page
    /// Properties" (<https://www.w3.org/TR/css-page-3/#page-properties>) states
    /// that "both the page context and the margin context have a computed value
    /// for every property" and that "The page context inherits from the root
    /// element" (raikiri-spike-ygl0).
    ///
    /// **Values other than `font-weight` are still specified values.**
    /// Resolution that needs more than the inheritance parent's computed values
    /// is out of scope for this pass and the value passes through unchanged.
    /// Known cases:
    ///
    /// - [`Length::Em`](crate::property::Length::Em) / `Rem` / `Percent` on the
    ///   box properties — `em` is relative to "the font associated with their
    ///   context" (§6 above) and the page context's own font-size can come from
    ///   a sibling `@page` declaration, so `root_style` alone does not determine
    ///   it; `Percent` needs the containing block. Tracked: bd
    ///   raikiri-spike-082k.
    /// - [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent) —
    ///   CSS Text 3 §6.1
    ///   (<https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent>)
    ///   computes it to the parent's computed `text-align` interpreted against
    ///   the parent's `direction`; raikiri models no `direction` property yet
    ///   (see the (b) milestone-subset carve-out on
    ///   [`TextAlign`](crate::property::TextAlign)).
    ///
    /// See `cascade::resolve_against_inherited` for the exact contract.
    ///
    /// **This map is not a full computed-value bag.** It contains only the
    /// properties that some matching `@page` rule actually *declared*;
    /// properties whose value would come purely from inheritance or from the
    /// initial value are **absent**. The spec sentence quoted above ("a computed
    /// value for every property") describes the page context as a whole, not
    /// this map — materialising the complete bag is the downstream page-layout
    /// consumer's job, and it starts from `root_style` plus these declarations.
    pub declarations: HashMap<PropertyKey, PropertyValue>,
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
/// 3. Resolve each winner against the page context's inheritance parent
///    (`root_style`) through the crate-internal `resolve_against_inherited` —
///    the sibling of `cascade::apply_value` that keeps the `PropertyValue`
///    shape. Today this resolves `font-weight: bolder` / `lighter` only (CSS
///    Fonts 4 §2.2.1 "Relative Weights"); every other value passes through
///    unchanged and therefore reaches the resolved declaration map as a
///    *specified* value — that field's doc enumerates which ones are still
///    unresolved.
///
/// # The inheritance parent (`root_style`)
///
/// CSS Paged Media 3 §6 "Page Properties"
/// (<https://www.w3.org/TR/css-page-3/#page-properties>) states: "As with
/// elements in the document, both the page context and the margin context have
/// a computed value for every property … The normal rules for CSS properties
/// apply with the following exceptions: page-margin boxes inherit from the page
/// context. The page context inherits from the root element."
///
/// `root_style` is therefore the root element's [`ComputedValues`]:
///
/// ```ignore
/// let cascaded = cascade(dom, rule_tree)?;
/// let result = cascade_page(rule_tree, &query,
///                           Some(&cascaded.computed[dom.root_id().0 as usize]));
/// ```
///
/// `None` selects the *legacy exception* the very same paragraph grants: "since
/// the previous revision of CSS Paged Media Level 3 did not specify this point,
/// an implementation that sets inherited properties in the page context to
/// their initial values (as for the root element) is also conformant to CSS
/// Paged Media Level 3." `None` is thus resolved against
/// [`ComputedValues::initial()`] — for `font-weight` that is `400`. The spec
/// states that exception will be removed in Level 4, so **prefer `Some`** —
/// passing `None` where a root style is available silently resolves `bolder` /
/// `lighter` against 400 instead of the root's weight.
///
/// # Ties on equal `(rank, specificity, source_order)`
///
/// The spec text at the anchor says @page cascade follows normal cascade
/// tie-breaking. Within a single rule, later declarations of the same
/// property win per CSS Cascading §"Order of appearance"; the `>=` in the
/// crate-internal `page_beats` mirrors the style-rule sibling's tie-break
/// (`cascade::beats` uses `>=` on the same triple).
///
/// # Example
///
/// ```ignore
/// use raikiri_style::{Origin, PageContextQuery, RuleTree, cascade_page};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page :first { color: red }", Origin::Author);
/// let query = PageContextQuery { is_first: true, ..Default::default() };
/// // Real callers pass `Some(root_style)` — see "The inheritance parent" above
/// // for the copy-pasteable form. `None` appears here only because this
/// // snippet has no document to cascade, and it selects the L3 legacy
/// // exception (resolution against the initial values).
/// let result = cascade_page(&tree, &query, None);
/// // result.declarations contains one entry: PropertyKey::Color -> red
/// ```
pub fn cascade_page(
    rule_tree: &RuleTree,
    query: &PageContextQuery,
    root_style: Option<&ComputedValues>,
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
                candidates.push((
                    decl.value.clone(),
                    decl.important,
                    rule.origin,
                    spec,
                    rule.source_order,
                ));
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
    // Step 3: resolve winners against the page context's inheritance parent.
    // `root_style == None` falls back to the initial values, which the L3 legacy
    // exception in CSS Paged Media 3 §6 "Page Properties" permits explicitly
    // (see the `root_style` section of this function's doc). Shared static: the
    // fallback is immutable and `initial()` costs a heap allocation, which the
    // `None` path would otherwise take on every call (`property.rs` の
    // `empty_counter_entries` と同じ前例)。
    static INITIAL_PAGE_PARENT: LazyLock<ComputedValues> = LazyLock::new(ComputedValues::initial);
    let inherited = root_style.unwrap_or(&INITIAL_PAGE_PARENT);
    PageCascadeResult {
        declarations: best
            .into_iter()
            .map(|(k, (_, _, _, v))| (k, resolve_against_inherited(v, inherited)))
            .collect(),
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
/// the same property beat earlier ones (CSS Cascading §"Order of appearance");
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
    use crate::property::{CssColor, FontWeightValue, Length, LengthOrAuto, TextAlign};

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
        match result.declarations.get(&PropertyKey::Color) {
            Some(PropertyValue::Color(c)) => Some(*c),
            _ => None,
        }
    }

    #[test]
    fn cascade_page_empty_rule_tree_returns_empty() {
        let tree = RuleTree::empty();
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
        assert!(result.declarations.is_empty());
    }

    #[test]
    fn cascade_page_default_selector_matches_every_page() {
        // `@page { color: red }` has an empty prelude → matches every page.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red }", Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_author_normal_beats_ua_normal_regardless_of_stylesheet_add_order() {
        // Author added first, UA second — origin rank (not source order) still
        // makes Author win because UA-normal rank (0) < Author-normal rank (1).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: blue }", Origin::Author);
        tree.add_stylesheet("@page { color: red }", Origin::UserAgent);
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
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
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_important_author_beats_normal_ua_and_author() {
        // rank(Author, important) = 2 > rank(Author, normal) = 1 > rank(UA, normal) = 0
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: green }", Origin::UserAgent);
        tree.add_stylesheet("@page { color: blue }", Origin::Author);
        tree.add_stylesheet("@page { color: red !important }", Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
        assert_eq!(color_of(&result), Some(RED));
    }

    // ── Verification 5: named-page cascade produces named-page declarations ──
    // Task Verification 5 target: `(page_name=Some("landscape_a3"), page_index=0,
    // is_first=true)` — the winning declarations must come from the named-page
    // rule. Property proxy: `color` (M1.4 supported). `size` / `margin` are
    // M4-descriptor scope not yet wired through the declaration parser (see
    // `ruletree::tests::page_body_unsupported_property_drops_declaration`), so
    // this test uses `color` to prove the shape end-to-end. The named-page
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_source_order_tiebreak_later_wins() {
        // Two rules of equal (rank, specificity) — later source_order wins
        // per CSS Cascading §"Order of appearance", sibling of style-rule
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
        let result = cascade_page(&tree, &query, None);
        assert_eq!(color_of(&result), Some(BLUE));
    }

    #[test]
    fn cascade_page_later_duplicate_in_same_rule_wins() {
        // Sibling of `cascade::tests::later_duplicate_in_same_rule_wins`:
        // within a single rule, later declarations of the same property win.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red; color: blue }", Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &query, None);
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
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
        assert_eq!(color_of(&result), Some(RED));
        assert_eq!(
            result.declarations.get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(700)))
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
    // **wiring**: that `cascade_page` resolves against `root_style` at all, and
    // which weight it uses as the inherited value (raikiri-spike-ygl0 — before
    // this, `FontWeightValue::Bolder` parked unresolved in the public
    // `declarations` map).

    /// Root [`ComputedValues`] with `font-weight: w`, everything else initial.
    fn root_with_weight(w: u16) -> ComputedValues {
        ComputedValues {
            font_weight: w,
            ..ComputedValues::initial()
        }
    }

    /// Cascade `@page { font-weight: <decl> }` with `root` as the page context's
    /// inheritance parent and return the resolved absolute weight from
    /// `declarations`. `root: None` selects the L3 legacy exception (resolution
    /// against the initial values — see [`cascade_page`]).
    ///
    /// Panics unless the winner is an already-resolved `Absolute` — a relative
    /// keyword surviving into the public map is exactly the ygl0 regression.
    fn page_font_weight(decl: &str, root: Option<&ComputedValues>) -> u16 {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&format!("@page {{ font-weight: {decl} }}"), Origin::Author);
        let result = cascade_page(&tree, &PageContextQuery::default(), root);
        match result.declarations.get(&PropertyKey::FontWeight) {
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
        let bolder = |root_w| page_font_weight("bolder", Some(&root_with_weight(root_w)));
        assert_eq!(bolder(50), 400, "w < 100 row");
        assert_eq!(bolder(100), 400, "100 <= w < 350 row");
        assert_eq!(bolder(400), 700, "350 <= w < 550 row");
        assert_eq!(bolder(700), 900, "550 <= w < 750 row");
        assert_eq!(bolder(800), 900, "750 <= w < 900 row");
        assert_eq!(
            bolder(1000),
            1000,
            "900 <= w row is no-change: must stay 1000, not clamp to 900"
        );
    }

    #[test]
    fn cascade_page_font_weight_lighter_resolves_against_root_computed_weight() {
        // Same six rows, `lighter` column.
        let lighter = |root_w| page_font_weight("lighter", Some(&root_with_weight(root_w)));
        assert_eq!(
            lighter(50),
            50,
            "w < 100 row is no-change: must stay 50, not rise to 100"
        );
        assert_eq!(lighter(100), 100, "100 <= w < 350 row");
        assert_eq!(lighter(400), 100, "350 <= w < 550 row");
        assert_eq!(lighter(700), 400, "550 <= w < 750 row");
        assert_eq!(lighter(800), 700, "750 <= w < 900 row");
        assert_eq!(lighter(1000), 700, "900 <= w row");
    }

    #[test]
    fn cascade_page_font_weight_relative_without_root_style_uses_initial_400() {
        // `root_style: None` = the L3 legacy exception quoted above ("sets
        // inherited properties in the page context to their initial values").
        // `font-weight` initial is 400, so bolder(400) = 700 and
        // lighter(400) = 100.
        assert_eq!(page_font_weight("bolder", None), 700);
        assert_eq!(page_font_weight("lighter", None), 100);
    }

    #[test]
    fn cascade_page_font_weight_absolute_ignores_root_computed_weight() {
        // Absolute weights are not inherited-value dependent: the root weight
        // must not perturb them (round-trip through `resolve_against_inherited`
        // is lossless).
        let root = root_with_weight(900);
        assert_eq!(page_font_weight("250", Some(&root)), 250);
        assert_eq!(page_font_weight("bold", Some(&root)), 700);
        assert_eq!(page_font_weight("normal", Some(&root)), 400);
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
        let root = root_with_weight(700);
        let result = cascade_page(&tree, &PageContextQuery::default(), Some(&root));
        assert_eq!(
            result.declarations.get(&PropertyKey::FontWeight),
            Some(&PropertyValue::FontWeight(FontWeightValue::Absolute(900))),
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
        let result = cascade_page(&tree, &PageContextQuery::default(), Some(&root));
        assert_eq!(color_of(&result), Some(RED));
    }

    #[test]
    fn cascade_page_length_em_passes_through_as_specified_value() {
        // Contract pin for the pass-through arm on a value that is *not*
        // computed-equivalent. CSS Paged Media 3 §6
        // <https://www.w3.org/TR/css-page-3/#page-properties>: "Values in units
        // of em and ex are interpreted relative to the font associated with
        // their context" — so `2em` is not a computed value, but resolving it
        // needs the page context's *own* font-size (possibly declared by a
        // sibling `@page` declaration), not just the inheritance parent. It
        // therefore stays specified here and the downstream page-layout
        // consumer resolves it (bd raikiri-spike-082k). This test exists so the
        // public doc claim on `PageCascadeResult::declarations` cannot drift.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { margin-top: 2em }", Origin::Author);
        let root = root_with_weight(700);
        let result = cascade_page(&tree, &PageContextQuery::default(), Some(&root));
        assert_eq!(
            result.declarations.get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Em(
                2.0
            )))),
            "pass-through must be documented as specified-value, not claimed resolved"
        );
    }

    #[test]
    fn cascade_page_text_align_match_parent_passes_through_as_specified_value() {
        // Second pass-through pin, on the other value the public doc calls out.
        // CSS Text 3 §6.1
        // <https://www.w3.org/TR/css-text-3/#valdef-text-align-match-parent>
        // computes `match-parent` to the parent's computed `text-align`
        // interpreted against the parent's `direction`; raikiri has no
        // `direction` in the computed layer, so the keyword ships as-is.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { text-align: match-parent }", Origin::Author);
        let root = root_with_weight(700);
        let result = cascade_page(&tree, &PageContextQuery::default(), Some(&root));
        assert_eq!(
            result.declarations.get(&PropertyKey::TextAlign),
            Some(&PropertyValue::TextAlign(TextAlign::MatchParent)),
            "match-parent is documented as an unresolved pass-through, not resolved"
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
        let result = cascade_page(&tree, &PageContextQuery::default(), None);
        assert_eq!(color_of(&result), Some(GREEN));
    }

    #[test]
    fn cascade_page_result_default_is_empty() {
        // `PageCascadeResult::default()` is the empty result (no rules
        // matched) — used by consumers that need a placeholder.
        let r = PageCascadeResult::default();
        assert!(r.declarations.is_empty());
    }

    #[test]
    fn cascade_page_result_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<PageCascadeResult>();
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
        let baseline = cascade_page(&tree, &query, None);
        for _ in 0..9 {
            let run = cascade_page(&tree, &query, None);
            assert_eq!(
                run.declarations.get(&PropertyKey::Color),
                baseline.declarations.get(&PropertyKey::Color),
            );
        }
    }
}
