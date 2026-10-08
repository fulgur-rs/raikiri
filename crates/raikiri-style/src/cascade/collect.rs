use crate::layer::LayerPosition;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use cssparser::{Parser, ParserInput};
use selectors::parser::Selector;

use crate::PseudoElem;
use crate::RaikiriSelectorImpl;
use crate::media::MediaContext;
use crate::property::{CustomProperty, PropertyValue};
use crate::rule::{Declaration, parse_declaration_block_with_consumer_properties};
use crate::ruletree::{Origin, RuleTree};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

use super::html_quirks::{push_img_dimension_hints, push_margin_collapsing_quirk_declarations};
use super::rule_index::{AncestorFilter, RuleIndex};
use super::selector_match::{
    MatchCaches, MatchContext, match_complex_selector_list, selector_matches_pseudo_element,
};
use super::table_hints::push_table_attribute_hints;

/// A 32-bit specificity from selectors, totally ordered as `u32`.
pub(crate) type Specificity = u32;

/// Inline-style specificity. CSS Cascading L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>, Specificity paragraph:
/// "declarations that do not belong to a style rule (such as the contents of a
/// style attribute) are considered to have a specificity higher than any
/// selector." (`(1, 0, 0, 0)` is older CSS 2.1 §6.4.3 notation, not L4.)
/// The selectors crate packs 32 bits as `id << 20 | class << 10 | element`.
/// `1 << 30` is above the selector-specificity range used by this adapter,
/// satisfying the CSS ordering rule.
pub(crate) const INLINE_SPECIFICITY: Specificity = 1 << 30;
/// Inline-style source_order: after all stylesheet rules (last occurrence).
pub(crate) const INLINE_SOURCE_ORDER: u32 = u32::MAX;

/// Specificity of HTML presentational hints (`<img width>` / `<img height>`,
/// [`push_img_dimension_hints`]). HTML LS §15.2 "The CSS user agent style
/// sheet and presentational hints"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#presentational-hints>)
/// calls these "author-level **zero-specificity** presentational hints".
/// The `0` follows that wording directly. CSS Cascading L5 §6.5
/// <https://drafts.csswg.org/css-cascade-5/#preshint> defines origin choices
/// but does not itself use "specificity"; do not conflate the two sources.
/// For the origin choice, see the "Cascade origin" section of the
/// [`push_img_dimension_hints`] docs.
pub(crate) const PRESENTATIONAL_HINT_SPECIFICITY: Specificity = 0;
/// Source order of the same hint. `0` can numerically tie with the first
/// actual stylesheet rule ([`crate::ruletree::RuleTree::add_stylesheet`]
/// assigns `source_order = 0` to the first rule in an empty `RuleTree`).
/// Since the dedicated `cascade_rank` tier was added for
/// [`Origin::AuthorPresentationalHint`], that tie cannot affect the cascade:
/// origin rank is compared before specificity and source_order. See the
/// "Cascade origin" section of the [`push_img_dimension_hints`] docs.
/// The value `0` was chosen for simplicity in the same numeric range as
/// stylesheet source_order, not as a value guaranteed to be collision-free.
pub(crate) const PRESENTATIONAL_HINT_SOURCE_ORDER: u32 = 0;

/// Margin-collapsing quirks zeroing declaration
/// ([`push_margin_collapsing_quirk_declarations`]) specificity. Pushed with
/// [`Origin::UserAgent`], so it never needs to out-rank a real
/// [`Origin::Author`] declaration on specificity — [`cascade_rank`] already
/// guarantees any `Origin::Author` declaration outranks any
/// `Origin::UserAgent` one regardless of specificity. This constant only
/// has to out-rank *other* `Origin::UserAgent` declarations for the same
/// property on the same node — most notably minimal.css's `blockquote,
/// figure, listing, p, plaintext, pre, xmp { margin-top: 1em;
/// margin-bottom: 1em; }` rule.
///
/// Reuses [`INLINE_SPECIFICITY`]'s bound rather than defining a second
/// selector-specificity threshold.
pub(crate) const MARGIN_COLLAPSING_QUIRK_SPECIFICITY: Specificity = INLINE_SPECIFICITY;
/// [`MARGIN_COLLAPSING_QUIRK_SPECIFICITY`]'s companion source_order. `0` —
/// same reasoning as [`PRESENTATIONAL_HINT_SOURCE_ORDER`]: the
/// dedicated `cascade_rank` tier already decides every comparison that
/// matters (against other `Origin::UserAgent` declarations, `beats`'s
/// specificity comparison is what actually separates this from
/// minimal.css's rule, and no two of *this* function's own pushes ever
/// compete against each other for the same property on the same node), so
/// no value other than "the same low end of the range real stylesheet
/// source_order uses" is needed here.
pub(crate) const MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER: u32 = 0;

/// Every pseudo-element the cascade collects candidates for.
pub(crate) const CASCADED_PSEUDO_ELEMENTS: [PseudoElem; 5] = [
    PseudoElem::Before,
    PseudoElem::After,
    PseudoElem::Marker,
    PseudoElem::FirstLine,
    PseudoElem::FirstLetter,
];

/// One candidate declaration: `(value, important, origin, specificity, source_order, layer)`.
/// `collect_cascaded` populates it; `pick_winners` ranks and selects winners.
/// Alias the tuple (including `Origin`) to avoid clippy::type_complexity.
pub(crate) type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32, LayerPosition);

/// A custom-property candidate. Unlike ordinary declarations, custom
/// properties are keyed by their case-sensitive name rather than by a fixed
/// `PropertyKey` slot.
pub(crate) type CustomCascadedDecl = (
    CustomProperty,
    bool,
    Origin,
    Specificity,
    u32,
    LayerPosition,
);

/// Output of [`collect_cascaded`]: all nodes' candidates in one flat `Vec`,
/// indexed by per-node [`Range`] values.
///
/// The flat arena keeps candidate storage contiguous and records one range per
/// node. This avoids per-node candidate containers while preserving document
/// order.
///
/// # Why wrap this in a struct instead of `(Vec<_>, HashMap<_, Range<usize>>)`?
///
/// The `winner.idx` from [`pick_winners`] indexes **the supplied slice**. A
/// wrong slice can silently select a declaration for another node without
/// going out of bounds. The flat arena adds precisely this risk: passing all
/// `decls` or an incomplete slice such as `decls[range.start..]` to
/// [`super::inherit::apply_winners`] instead of using
/// [`candidates`](Self::candidates). Make production callers use only
/// [`candidates`](Self::candidates), which supplies exactly this node's range.
/// The `decls`/`ranges` fields are `pub(crate)` so the owning cascade
/// module can maintain the non-overlapping range invariant; implementation
/// code reads candidates through the typed accessors.
///
/// `pub(crate)` follows [`super::inherit::resolve_inheritance`], which is
/// also `pub(crate)` for an intra-doc link from another module. Other modules
/// are not meant to construct or manipulate this arena: [`super::cascade`]
/// constructs it, and [`collect_cascaded`] populates it in this module.
pub(crate) struct CascadedArena {
    /// Flat storage for all nodes' candidates in document visit order.
    pub(crate) decls: Vec<CascadedDecl>,
    /// Per-node ranges in `decls`. A node with no candidates has no entry,
    /// matching the old `if !per_node.is_empty() { out.insert(..) }` contract.
    pub(crate) ranges: HashMap<StyleNodeId, Range<usize>>,
    /// All custom-property candidates in document visit order.
    custom_decls: Vec<CustomCascadedDecl>,
    /// Per-node ranges into `custom_decls`.
    custom_ranges: HashMap<StyleNodeId, Range<usize>>,
    /// Flat arena like `decls`/`ranges`, but keyed by the originating
    /// element's `StyleNodeId` plus its `::before` or `::after` pseudo-element.
    /// One element may have independent candidates for both. Empty `(id,
    /// pseudo)` pairs have no entry, following the same contract.
    pseudo_decls: Vec<CascadedDecl>,
    /// Per-`(id, pseudo)` ranges in `pseudo_decls`.
    pseudo_ranges: HashMap<(StyleNodeId, PseudoElem), Range<usize>>,
    /// Custom-property counterpart of `pseudo_decls`.
    pseudo_custom_decls: Vec<CustomCascadedDecl>,
    /// Per-`(id, pseudo)` ranges in `pseudo_custom_decls`.
    pseudo_custom_ranges: HashMap<(StyleNodeId, PseudoElem), Range<usize>>,
}

impl CascadedArena {
    pub(crate) fn new() -> Self {
        Self {
            decls: Vec::new(),
            ranges: HashMap::new(),
            custom_decls: Vec::new(),
            custom_ranges: HashMap::new(),
            pseudo_decls: Vec::new(),
            pseudo_ranges: HashMap::new(),
            pseudo_custom_decls: Vec::new(),
            pseudo_custom_ranges: HashMap::new(),
        }
    }

    /// Candidate list for `id`: a slice **only for this node**, suitable for
    /// passing directly to [`pick_winners`].
    ///
    /// The slice is always `&self.decls[range]`, where `range` is exactly the
    /// range filled by `collect_cascaded` for `id`. This struct exposes no
    /// path for callers to assemble the full slice or shift `range.start`.
    pub(crate) fn candidates(&self, id: StyleNodeId) -> Option<&[CascadedDecl]> {
        self.ranges.get(&id).map(|range| &self.decls[range.clone()])
    }

    /// Every node that has candidates, with its candidate slice, in no
    /// particular order.
    pub(crate) fn all_candidates(&self) -> impl Iterator<Item = (StyleNodeId, &[CascadedDecl])> {
        self.ranges
            .iter()
            .map(|(&id, range)| (id, &self.decls[range.clone()]))
    }

    pub(crate) fn custom_candidates(&self, id: StyleNodeId) -> Option<&[CustomCascadedDecl]> {
        self.custom_ranges
            .get(&id)
            .map(|range| &self.custom_decls[range.clone()])
    }

    /// Candidates for `(id, pseudo)`: the `::before`/`::after` counterpart
    /// of [`candidates`](Self::candidates).
    pub(crate) fn pseudo_candidates(
        &self,
        id: StyleNodeId,
        pseudo: PseudoElem,
    ) -> Option<&[CascadedDecl]> {
        self.pseudo_ranges
            .get(&(id, pseudo))
            .map(|range| &self.pseudo_decls[range.clone()])
    }

    pub(crate) fn pseudo_custom_candidates(
        &self,
        id: StyleNodeId,
        pseudo: PseudoElem,
    ) -> Option<&[CustomCascadedDecl]> {
        self.pseudo_custom_ranges
            .get(&(id, pseudo))
            .map(|range| &self.pseudo_custom_decls[range.clone()])
    }

    /// Whether `a` and `b` carry exactly the same cascade input: equal
    /// ordinary, custom-property, and per-pseudo-element candidate lists,
    /// compared value by value including origin, importance, specificity,
    /// and source order.
    ///
    /// Everything node-specific that the cascade knows about an element —
    /// matched rules, inline style, presentational hints, quirks
    /// declarations — reaches the inheritance walk only through these lists.
    /// Two nodes that agree here and share a parent therefore resolve to the
    /// same computed values; see [`super::inherit::resolve_inheritance`].
    pub(crate) fn same_cascade_input(&self, a: StyleNodeId, b: StyleNodeId) -> bool {
        self.candidates(a) == self.candidates(b)
            && self.custom_candidates(a) == self.custom_candidates(b)
            && CASCADED_PSEUDO_ELEMENTS.iter().all(|&pseudo| {
                self.pseudo_candidates(a, pseudo) == self.pseudo_candidates(b, pseudo)
                    && self.pseudo_custom_candidates(a, pseudo)
                        == self.pseudo_custom_candidates(b, pseudo)
            })
    }
}

/// Parsed `style`-attribute blocks for one cascade, keyed by source text.
///
/// Generated documents often repeat the same inline style on many elements;
/// parsing each distinct string once avoids re-tokenizing it per element.
/// A source is only stored the **second** time it is seen: the first sight
/// records just its hash, so a document whose inline styles are all
/// different pays one hash and one small map entry per element instead of
/// an extra copy of every string and declaration list. Entries are keyed by
/// hash but always confirm the full source text before reuse, so a hash
/// collision falls back to a fresh parse. Consumer property registrations
/// are fixed for the rule tree the cascade runs against, so the parse result
/// is a function of the source text alone.
#[derive(Default)]
struct DeclarationBlockCache {
    entries: HashMap<u64, DeclarationBlockEntry>,
}

enum DeclarationBlockEntry {
    SeenOnce,
    Parsed(Box<str>, Vec<Declaration>),
}

impl DeclarationBlockCache {
    fn declarations(&mut self, source: &str, rule_tree: &RuleTree) -> Cow<'_, [Declaration]> {
        let parse = || {
            let mut input = ParserInput::new(source);
            let mut parser = Parser::new(&mut input);
            parse_declaration_block_with_consumer_properties(
                &mut parser,
                rule_tree.consumer_property_registrations(),
            )
        };
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        match self.entries.entry(hasher.finish()) {
            Entry::Vacant(vacant) => {
                vacant.insert(DeclarationBlockEntry::SeenOnce);
                Cow::Owned(parse())
            }
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                if matches!(entry, DeclarationBlockEntry::SeenOnce) {
                    *entry = DeclarationBlockEntry::Parsed(source.into(), parse());
                }
                match entry {
                    DeclarationBlockEntry::Parsed(cached, declarations) if **cached == *source => {
                        Cow::Borrowed(declarations.as_slice())
                    }
                    // A different source with the same hash: never reuse it.
                    _ => Cow::Owned(parse()),
                }
            }
        }
    }
}

/// Calls `push` with each `(value, important)` pair, moving out of a freshly
/// parsed block and cloning out of a cached one.
fn for_each_declaration(
    declarations: Cow<'_, [Declaration]>,
    mut push: impl FnMut(PropertyValue, bool),
) {
    match declarations {
        Cow::Owned(owned) => owned.into_iter().for_each(|d| push(d.value, d.important)),
        Cow::Borrowed(borrowed) => borrowed
            .iter()
            .for_each(|d| push(d.value.clone(), d.important)),
    }
}

/// Pushes one declaration into `decls`/`custom_decls` (routing on
/// `PropertyValue::CustomProperty`, same split every candidate list in this
/// module uses). Takes the destination `Vec`s directly rather than a whole
/// [`CascadedArena`] so the same function serves both the real-element path
/// (`&mut out.decls, &mut out.custom_decls`, [`collect_cascaded`]) and the
/// `::before`/`::after` path (a per-element scratch buffer pair,
/// [`collect_cascaded`]'s pseudo-element section) without duplicating this
/// match.
fn push_cascaded_decl(
    (decls, custom_decls): (&mut Vec<CascadedDecl>, &mut Vec<CustomCascadedDecl>),
    value: PropertyValue,
    important: bool,
    origin: Origin,
    specificity: Specificity,
    source_order: u32,
    layer: LayerPosition,
) {
    match value {
        PropertyValue::CustomProperty(custom) => {
            custom_decls.push((custom, important, origin, specificity, source_order, layer))
        }
        value => decls.push((value, important, origin, specificity, source_order, layer)),
    }
}

/// Scratch slot for [`pick_winners`]: the provisional winner for one key.
///
/// Crucially, [`idx`](Self::idx) is an **index**, not a [`PropertyValue`]:
/// - The slot is `Copy` with no `Drop`, so [`Option::take`] resets it without
///   dropping or reallocating the entire buffer.
/// - Losing candidates are never cloned. Previously each candidate's
///   `value.clone()` was discarded if it lost; now only the winner is cloned
///   once for [`super::inherit::apply_value`].
///
/// The sibling [`CascadedDecl`] remains a tuple alias because it is always
/// destructured into named bindings, never accessed positionally. This type
/// uses named fields because [`beats`] compares precedence fields **in order**:
/// [`specificity`](Self::specificity) and [`source_order`](Self::source_order)
/// are both `u32`; swapping tuple positions `.1` and `.2` would compile and
/// silently change cascade winners.
#[derive(Clone, Copy)]
pub(crate) struct RankedDecl {
    /// Origin plus `!important` precedence from [`cascade_rank`].
    pub(crate) rank: u8,
    /// Element-attached declarations first, then the importance-adjusted layer rank.
    pub(crate) layer_priority: (bool, u32),
    /// Selector specificity (inline style uses [`INLINE_SPECIFICITY`]).
    pub(crate) specificity: Specificity,
    /// Source order in the stylesheet (inline style uses [`INLINE_SOURCE_ORDER`]).
    pub(crate) source_order: u32,
    /// Position in the `candidates` slice passed to [`pick_winners`].
    pub(crate) idx: usize,
}

/// Precedence rank from cascade origin and `!important`.
/// [`Origin::AuthorPresentationalHint`], [`Origin::User`], and
/// [`Origin::Animation`] were added after the initial UA/Author implementation.
/// Each insertion re-derives the origin ordering rather than merely shifting
/// rank numbers.
///
/// Higher ranks win. This represents Origin and Importance in CSS Cascading
/// L4 §6.1 "Cascade Sorting Order"
/// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>. See §6.2
/// <https://www.w3.org/TR/css-cascade-4/#cascading-origins> for origins and
/// §6.3 <https://www.w3.org/TR/css-cascade-4/#importance> for the `!important`
/// reversal.
///
/// # UA / User / Author: three origins (verbatim spec order)
///
/// §6.1 "Cascade Sorting Order" lists origins from strongest to weakest.
/// The relevant entries follow; transitions are not yet implemented in this
/// crate:
/// - "Important user agent declarations"
/// - "Important user declarations"
/// - "Important author declarations"
/// - "Animations"
/// - "Normal author declarations"
/// - "Normal user declarations"
/// - "Normal user agent declarations"
///
/// "Declarations from origins earlier in this list win over declarations
/// from later origins". Reversing that strongest-to-weakest list to ascending
/// rank gives `Normal UA < Normal User < Normal Author < Animation
/// < Important Author < Important User < Important UA`. This ordering for the
/// three origins,
/// including Important User's position (`Important Author < Important User
/// < Important UA`), comes **directly** from the spec, not a derivation.
///
/// # Inserting [`Origin::AuthorPresentationalHint`] (CSS Cascading L5 §6.5)
///
/// [`Origin::AuthorPresentationalHint`] is the "author presentational hint
/// origin" defined by CSS Cascading L5 §6.5 "Precedence of Non-CSS
/// Presentational Hints" (<https://drafts.csswg.org/css-cascade-5/#preshint>).
/// L5 §6.1 still lists the same eight entries without explicitly inserting
/// this origin; §6.5 describes its placement separately. These are
/// **relevant excerpts** from §6.5, not a verbatim copy of its entire text:
/// - "All document language-based styling must be translated to
///   corresponding CSS rules and enter the cascade as rules in **either
///   the UA-origin or** a special-purpose author presentational hint
///   origin between the regular user origin and the author origin" —
///   "treated as an independent origin"
/// - "Presentational hints entering the cascade as author presentational
///   hint origin rules can be overridden by author-origin styles, but not
///   by non-important user-origin styles"
/// - "**A document language may define whether** such a presentational
///   hint enters the cascade as UA-origin or author-origin; if so, the UA
///   must behave accordingly. For example, SVG maps its presentation
///   attributes into the author origin."
/// - "however for the purpose of the revert keyword (but not for the
///   revert-layer keyword) it is considered part of the author origin"
///
/// **The host language chooses the tier.** The third quote above lets the
/// host language choose UA or author origin explicitly (SVG chooses author).
/// HTML Living Standard §15.2 only calls its hints "author-level
/// zero-specificity presentational hints". It never uses the CSS-Cascade-5
/// terms "UA-origin", "author-origin", or "author presentational hint origin",
/// and thus does not explicitly select one of those tiers. Assigning `<img>`
/// width/height hints to the independent [`Origin::AuthorPresentationalHint`]
/// tier (see [`push_img_dimension_hints`]) is this crate's interpretation of
/// "author-level" and the quoted spec text: a defensible judgment call,
/// **not a placement directly specified by the standard**.
///
/// The §6.5 quotes directly place hints only at the **Normal tier**:
/// "between the regular user origin and the author origin", i.e. between
/// Normal User and Normal Author. They do not explicitly place
/// `AuthorPresentationalHint` at the Important tier. Host-language-generated
/// hints (HTML LS §15.2) are always non-important, so the spec has no reason
/// to define an important-hint tier. Derive this otherwise absent position
/// from independent-origin status (quoted above) and the §6.1
/// **reversal-equivalence**: for UA/User/Author, ascending Important order
/// (`Author < User < UA`) is exactly the reverse of ascending Normal order
/// (`UA < User < Author`). This is a direct observation of the §6.1 list,
/// not an analogy. Extending the same reversal to a fourth origin is the
/// smallest extension without additional assumptions: the hint stays
/// between User and Author in Normal order, and between Important Author
/// and Important User in reversed Important order. This position is
/// **derived**, not verbatim, from the spec's own reversal mechanism.
///
/// # Rank table for all declaration sources (re-derived)
///
/// Combining these two sections orders the four CSS origins at each tier;
/// animations occupy their own precedence band between normal and important
/// declarations:
/// - Normal: UA < User < AuthorPresentationalHint < Author (insert the
///   spec-defined hint position between User and Author into the
///   spec-defined UA/User/Author ordering).
/// - Important: Author < AuthorPresentationalHint < User < UA (insert the
///   derived hint position between Important Author and Important User into
///   the reversed, spec-defined Author/User/UA ordering).
///
/// Assign ranks with 0 weakest and 8 strongest:
///
/// | origin                     | Normal | Important |
/// |-----------------------------|--------|-----------|
/// | `UserAgent`                 | 0      | 8         |
/// | `User`                      | 1      | 7         |
/// | `AuthorPresentationalHint`  | 2      | 6         |
/// | `Author`                    | 3      | 5         |
/// | `Animation`                 | 4      | 4         |
///
/// Check: `min(Important) = 5 > Animation = 4 > max(Normal) = 3`, so any
/// important declaration beats every animation declaration, and animations
/// beat every normal declaration (§6.1). For UA/User/Author,
/// ascending Normal (`0,1,3`) and descending Important (`8,7,5`) follow the
/// spec directly. The inserted hint ranks (`2`/`5`) retain the same relative
/// position between User and Author in both orderings. Animation declarations
/// are collected as non-important; the total rank function maps either flag
/// to the same animation band.
///
/// [`push_img_dimension_hints`] always pushes `important = false`, so its
/// `(AuthorPresentationalHint, true)` arm is currently unreachable. The two
/// User arms (`(User, false)` / `(User, true)`) originally had no production
/// caller. Consumer-provided `extra_stylesheets` now route to [`Origin::User`],
/// making both arms reachable (see the [`Origin::User`] docs). Because
/// [`crate::page::cascade_page`] also uses [`Origin`], keep these as total
/// function arms rather than `unreachable!()`. This was the right choice
/// even before a producer existed.
///
/// The `revert` keyword carve-out (the fourth quote: "it is considered part
/// of the author origin"; not `revert-layer`) is applied when border longhands
/// roll back their winners in [`super::inherit`].
///
/// The `@page` cascade shares this origin ordering, so expose this function
/// as `pub(crate)` for reuse by [`crate::page::cascade_page`].
pub(crate) fn cascade_rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::User, false) => 1,
        (Origin::AuthorPresentationalHint, false) => 2,
        (Origin::Author, false) => 3,
        (Origin::Animation, false | true) => 4,
        (Origin::Author, true) => 5,
        (Origin::AuthorPresentationalHint, true) => 6,
        (Origin::User, true) => 7,
        (Origin::UserAgent, true) => 8,
    }
}

/// `collect_cascaded` traverses nodes with DFS. Before descendant/child
/// combinators were supported, per-node work was independent of other nodes
/// and visit order did not matter. This is **no longer true**: selector
/// matching uses the local `ancestor_path` (IDs of elements already visited).
/// Correctness requires pre-order traversal with ancestors before descendants;
/// otherwise ancestor lookups for combinators fail. The traversal remains
/// iterative with an explicit `Vec` stack to avoid overflow, but each entry
/// is `(StyleNodeId, usize)` rather than just `StyleNodeId`: the second value
/// is the length to which the ancestor path must be truncated before the node.
/// See the implementation comments below.
///
/// # Writing to the flat arena
///
/// Candidates for one node are appended **contiguously** to `out.decls`:
/// matching stylesheet rules in source order, then inline style. Once both
/// complete, `start..out.decls.len()` becomes the node's range. No other
/// append occurs before the next node. Thus indices in the slice returned
/// by [`CascadedArena::candidates`] remain indices into **that node's own**
/// `candidates` for [`pick_winners`]/[`super::inherit::apply_winners`].
/// Passing indices in the global arena instead would break this contract.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "default-context entry point named by the surrounding cascade \
                  documentation; production dispatch uses \
                  collect_cascaded_with_media_context"
    )
)]
pub(crate) fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
) {
    collect_cascaded_with_media_context(dom, id, rule_tree, out, &MediaContext::default());
}

pub(crate) fn collect_cascaded_with_media_context<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut CascadedArena,
    media_context: &MediaContext,
) {
    // Document-wide constant — read once rather than
    // per (node, rule) pair inside the loop below.
    let layers = rule_tree.layer_order(media_context);
    let quirks_mode = dom.quirks_mode();
    // Sibling positions, languages and directionality are pure functions of
    // the DOM, which stays immutably borrowed for this whole walk, so one
    // set of match caches serves every element. A stylesheet cascade has no
    // scoping element (`:scope` falls back to `:root` semantics, matched by
    // `is_supported_selector`'s existing rejection of any selector
    // containing `:scope` before it ever reaches the rule tree), and it
    // skips inert candidates (`allow_detached = false`).
    let match_caches = MatchCaches::default();
    let match_ctx = MatchContext::new(dom, quirks_mode, None, false, &match_caches);
    // The media context is fixed for this cascade invocation. Evaluate each
    // condition once, keeping inactive rules out of every element's scan.
    let mut style_rules = rule_tree.style_rules.iter().collect::<Vec<_>>();
    style_rules.extend(
        rule_tree
            .media_rules
            .iter()
            .filter(|media| media.condition.matches(media_context))
            .map(|media| &media.rule),
    );
    style_rules.sort_unstable_by_key(|rule| rule.source_order);
    // Bucket the active rules once per cascade so each element only runs the
    // full matcher against rules that can possibly match it. See the
    // `rule_index` module docs for why the filtering never drops a match.
    let rule_index = RuleIndex::new(style_rules);
    let mut candidate_rules: Vec<u32> = Vec::new();
    let mut block_cache = DeclarationBlockCache::default();
    let mut cell_padding_cache = HashMap::new();

    // Stack entries pair a node id with the `ancestor_path` length it should
    // be truncated to *before* that node is processed.
    // `stack` itself interleaves the pending work of
    // multiple subtrees in one flat `Vec` (sibling branches, cousins, ...),
    // so a plain push/pop can't recover "the current node's actual ancestor
    // chain" by itself — truncating `ancestor_path` to the depth recorded
    // when each entry was pushed undoes whatever a since-fully-processed
    // sibling subtree appended, reconstructing exactly the root..parent
    // chain for whichever node is popped next. Standard technique for
    // recovering DFS ancestor paths from a single explicit stack; it stays
    // O(1) amortized (`Vec::truncate` just shrinks `len`, no deallocation)
    // and needs no `HashMap`/parent-pointer side table.
    let mut stack: Vec<(StyleNodeId, usize)> = vec![(id, 0)];
    // Ancestor **element** ids, root-most first / immediate-parent last
    // (`ancestor_path.last()` = current node's parent). Only `Element`-kind
    // nodes are ever pushed — `Document`/`Text`/etc. can never be matched by
    // a compound selector, so they must not count as a combinator ancestor
    // either (CSS Selectors L4 descendant/child combinators are defined in
    // terms of element ancestry, e.g.
    // <https://www.w3.org/TR/selectors-4/#descendant-combinators> "an
    // element B that is an arbitrary descendant of some ancestor element
    // A" — verbatim (see `match_combinator_chain`'s "Spec provenance note"
    // for how this text was confirmed), both sides are elements).
    let mut ancestor_path: Vec<StyleNodeId> = Vec::new();
    // Bloom filter over the same ancestors, truncated and pushed in lockstep
    // with `ancestor_path`.
    let mut ancestor_filter = AncestorFilter::new();
    // `::before`/`::after` candidate scratch buffers — declared outside the
    // walk loop and drained (via `Vec::append`, see the flush site below) at
    // the end of each element's processing, so they're always empty when a
    // new element starts. Reused across the whole document walk rather than
    // allocated fresh per element, same rationale as `resolve_inheritance`'s
    // `winners` buffer.
    let mut pseudo_before_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_after_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_marker_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_first_line_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_first_letter_decls: Vec<CascadedDecl> = Vec::new();
    let mut pseudo_before_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_after_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_marker_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_first_line_custom: Vec<CustomCascadedDecl> = Vec::new();
    let mut pseudo_first_letter_custom: Vec<CustomCascadedDecl> = Vec::new();
    while let Some((id, depth)) = stack.pop() {
        ancestor_path.truncate(depth);
        ancestor_filter.truncate(depth);
        if let Some(node) = dom.node(id) {
            // Skip descendants of <template> and future inert subtrees alike.
            // Silent bug fix: rule matching previously ran inside templates,
            // wasting arena space (formerly per-node Vec<CascadedDecl>).
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let start = out.decls.len();
                let custom_start = out.custom_decls.len();
                // HTML presentational hints (later retagged to
                // `Origin::AuthorPresentationalHint`, distinct from plain
                // `Origin::Author`). This push is kept ahead of
                // stylesheet-rule matching / inline style below for
                // historical/document-order reasons, but it is no longer a
                // *correctness* requirement: since the hint has its own
                // `cascade_rank` tier (strictly between `UserAgent` and
                // `Author`, see `push_img_dimension_hints` doc's "Cascade
                // origin" section), rank alone decides against any real
                // Author-origin declaration regardless of specificity,
                // source_order, or push order — no tie can occur (that was
                // only possible earlier, when hint and real Author
                // declarations shared the same `Origin::Author` rank).
                // Re-verified after the 4th `Origin::User` tier was inserted:
                // the hint's `cascade_rank` value moved (see `cascade_rank`'s
                // rank table) but stayed strictly between `Origin::User` and
                // `Origin::Author` — never equal to the real `Author` rank in
                // either the Normal or the Important half of the table — so
                // this reasoning still holds unchanged; no test pins the push
                // order itself (nothing here is order-*dependent* left to
                // pin), but
                // `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
                // continues to check the outcome this comment claims.
                push_img_dimension_hints(&elem, &mut out.decls);
                push_table_attribute_hints(
                    dom,
                    &elem,
                    &ancestor_path,
                    &mut cell_padding_cache,
                    &mut out.decls,
                );
                const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
                let is_svg_root = elem.tag_name() == "svg"
                    && elem.namespace_uri() == Some(SVG_NAMESPACE)
                    && !ancestor_path.iter().any(|ancestor_id| {
                        dom.node(*ancestor_id).is_some_and(|ancestor| {
                            ancestor.as_element().is_some_and(|ancestor| {
                                ancestor.tag_name() == "svg"
                                    && ancestor.namespace_uri() == Some(SVG_NAMESPACE)
                            })
                        })
                    });
                if is_svg_root {
                    super::svg_hints::push_dimension_hints(&elem, &mut out.decls);
                }
                if elem.namespace_uri() == Some(SVG_NAMESPACE) {
                    if !is_svg_root {
                        super::svg_hints::push_font_size_hint(&elem, &mut out.decls);
                    }
                    for (attribute, property, expected_key) in [
                        ("display", "display", crate::property::PropertyKey::Display),
                        ("color", "color", crate::property::PropertyKey::Color),
                        (
                            "font-family",
                            "font-family",
                            crate::property::PropertyKey::FontFamily,
                        ),
                        (
                            "font-weight",
                            "font-weight",
                            crate::property::PropertyKey::FontWeight,
                        ),
                        (
                            "font-style",
                            "font-style",
                            crate::property::PropertyKey::FontStyle,
                        ),
                        (
                            "visibility",
                            "visibility",
                            crate::property::PropertyKey::Visibility,
                        ),
                        ("opacity", "opacity", crate::property::PropertyKey::Opacity),
                        (
                            "background-color",
                            "background-color",
                            crate::property::PropertyKey::BackgroundColor,
                        ),
                    ] {
                        let Some(raw_value) = elem.attr(attribute) else {
                            continue;
                        };
                        let mut input = ParserInput::new(raw_value);
                        let mut parser = Parser::new(&mut input);
                        if let Some(value) = crate::property::parse_value(property, &mut parser)
                            && value.key() == expected_key
                            && parser.expect_exhausted().is_ok()
                        {
                            push_cascaded_decl(
                                (&mut out.decls, &mut out.custom_decls),
                                value,
                                false,
                                Origin::AuthorPresentationalHint,
                                PRESENTATIONAL_HINT_SPECIFICITY,
                                PRESENTATIONAL_HINT_SOURCE_ORDER,
                                LayerPosition::default(),
                            );
                        }
                    }
                }
                // HTML LS §15.3.9 margin-collapsing quirks (quirks-mode
                // margin-block zeroing) — `ancestor_path` here is still
                // `id`'s ancestor chain *without* `id` itself (that push
                // happens further below, after this element's candidates
                // are collected), so `ancestor_path.last()` is exactly
                // `id`'s real DOM parent. See
                // `push_margin_collapsing_quirk_declarations` doc for the
                // full rule set and design rationale.
                push_margin_collapsing_quirk_declarations(
                    dom,
                    id,
                    &elem,
                    &ancestor_path,
                    quirks_mode,
                    &mut out.decls,
                );
                // stylesheet rule matching, restricted to the rules the
                // index could not rule out (still in source order)
                rule_index.candidate_rules(&elem, &ancestor_filter, &mut candidate_rules);
                for &rule_idx in &candidate_rules {
                    let indexed = rule_index.rule(rule_idx);
                    let rule = indexed.rule;
                    if indexed.has_element_selector
                        && let Some(spec) = match_complex_selector_list(
                            &rule.selectors,
                            match_ctx,
                            &elem,
                            id,
                            &ancestor_path,
                        )
                    {
                        // Longhands were expanded when the index was built
                        // for this cascade. Expanding only during parsing
                        // would not cover post-parse mutation of `RuleTree`;
                        // see the `crate::rule::expand_shorthand_into` docs.
                        for d in &indexed.declarations {
                            push_cascaded_decl(
                                (&mut out.decls, &mut out.custom_decls),
                                d.value.clone(),
                                d.important,
                                rule.origin,
                                spec,
                                rule.source_order,
                                LayerPosition {
                                    attached: false,
                                    rank: layers.rank(rule.layer, rule.origin),
                                },
                            );
                        }
                    }
                    if !indexed.has_pseudo_selector {
                        continue;
                    }
                    // `::before`/`::after` — independent pass over the same
                    // rule's selector list (a rule's comma-separated list can
                    // target the real element via one selector and a
                    // pseudo-element via another, e.g. `a, a::before {..}`,
                    // so this isn't mutually exclusive with the match above).
                    // `selector_matches_pseudo_element` fast-returns `None`
                    // via `Selector::pseudo_element()`'s `O(1)` flag check
                    // for the (overwhelmingly common) selector that doesn't
                    // target a pseudo-element at all, so this second list
                    // walk stays cheap for documents with no `::before`/
                    // `::after` rules.
                    for selector in rule.selectors.slice() {
                        let Some(pseudo) = selector_matches_pseudo_element(
                            match_ctx,
                            selector,
                            &elem,
                            id,
                            &ancestor_path,
                        ) else {
                            continue;
                        };
                        let spec = specificity_of(selector);
                        let (buf, custom_buf) = match pseudo {
                            PseudoElem::Before => {
                                (&mut pseudo_before_decls, &mut pseudo_before_custom)
                            }
                            PseudoElem::After => {
                                (&mut pseudo_after_decls, &mut pseudo_after_custom)
                            }
                            PseudoElem::Marker => {
                                (&mut pseudo_marker_decls, &mut pseudo_marker_custom)
                            }
                            PseudoElem::FirstLine => {
                                (&mut pseudo_first_line_decls, &mut pseudo_first_line_custom)
                            }
                            PseudoElem::FirstLetter => (
                                &mut pseudo_first_letter_decls,
                                &mut pseudo_first_letter_custom,
                            ),
                            // cov:ignore: selector_matches_pseudo_element excludes boxless native pseudos
                            PseudoElem::Backdrop | PseudoElem::FileSelectorButton => continue,
                        };
                        for d in &indexed.declarations {
                            push_cascaded_decl(
                                (buf, custom_buf),
                                d.value.clone(),
                                d.important,
                                rule.origin,
                                spec,
                                rule.source_order,
                                LayerPosition {
                                    attached: false,
                                    rank: layers.rank(rule.layer, rule.origin),
                                },
                            );
                        }
                    }
                }
                // inline style
                if let Some(source) = elem.inline_style_source() {
                    for_each_declaration(
                        block_cache.declarations(source, rule_tree),
                        |value, important| {
                            push_cascaded_decl(
                                (&mut out.decls, &mut out.custom_decls),
                                value,
                                important,
                                Origin::Author,
                                INLINE_SPECIFICITY,
                                INLINE_SOURCE_ORDER,
                                LayerPosition {
                                    attached: true,
                                    ..LayerPosition::default()
                                },
                            );
                        },
                    );
                }
                if let Some(source) = elem.animation_style_source() {
                    for_each_declaration(
                        block_cache.declarations(source, rule_tree),
                        |value, _| {
                            push_cascaded_decl(
                                (&mut out.decls, &mut out.custom_decls),
                                value,
                                false,
                                Origin::Animation,
                                INLINE_SPECIFICITY,
                                INLINE_SOURCE_ORDER,
                                LayerPosition::default(),
                            );
                        },
                    );
                }
                let end = out.decls.len();
                if end > start {
                    out.ranges.insert(id, start..end);
                }
                let custom_end = out.custom_decls.len();
                if custom_end > custom_start {
                    out.custom_ranges.insert(id, custom_start..custom_end);
                }
                // Flush this element's `::before`/`::after` scratch buffers
                // into the shared pseudo arena — `Vec::append` moves (no
                // clone) and leaves the scratch buffer empty, ready for the
                // next element that has a pseudo match to reuse without a
                // fresh allocation (same "declare outside the loop, drain in
                // place" idiom `resolve_inheritance`'s `winners` buffer
                // uses). Only elements with an actual match ever touch these
                // buffers, so they stay empty (a cheap `is_empty` `Vec`, no
                // allocation) for the common no-`::before`/`::after` case.
                for (pseudo, buf, custom_buf) in [
                    (
                        PseudoElem::Before,
                        &mut pseudo_before_decls,
                        &mut pseudo_before_custom,
                    ),
                    (
                        PseudoElem::After,
                        &mut pseudo_after_decls,
                        &mut pseudo_after_custom,
                    ),
                    (
                        PseudoElem::Marker,
                        &mut pseudo_marker_decls,
                        &mut pseudo_marker_custom,
                    ),
                    (
                        PseudoElem::FirstLine,
                        &mut pseudo_first_line_decls,
                        &mut pseudo_first_line_custom,
                    ),
                    (
                        PseudoElem::FirstLetter,
                        &mut pseudo_first_letter_decls,
                        &mut pseudo_first_letter_custom,
                    ),
                ] {
                    let pseudo_start = out.pseudo_decls.len();
                    out.pseudo_decls.append(buf);
                    if out.pseudo_decls.len() > pseudo_start {
                        out.pseudo_ranges
                            .insert((id, pseudo), pseudo_start..out.pseudo_decls.len());
                    }
                    let pseudo_custom_start = out.pseudo_custom_decls.len();
                    out.pseudo_custom_decls.append(custom_buf);
                    if out.pseudo_custom_decls.len() > pseudo_custom_start {
                        out.pseudo_custom_ranges.insert(
                            (id, pseudo),
                            pseudo_custom_start..out.pseudo_custom_decls.len(),
                        );
                    }
                }
                // This element becomes an ancestor for its own children
                // (pushed just below with `ancestor_path.len()` as their
                // truncation depth).
                ancestor_path.push(id);
                ancestor_filter.push(&elem);
            }
            // The stack is LIFO, so reverse children to visit them in document
            // order. Extend `stack` directly from `child_ids`, then reverse
            // only the newly appended tail in place. This removes one
            // disposable intermediate `Vec`; `stack` retains the same
            // amortized capacity-growth pattern as `for .. { stack.push(..) }`.
            //
            // Document-order pre-order is now **necessary** for correctness.
            // Descendant/child combinator matching reads `ancestor_path`, which
            // reflects DFS visit order. Visiting a child before its parent
            // leaves the parent absent and falsely rejects the match. Before
            // combinators were supported, visit order only preserved behavior;
            // that older rationale no longer applies.
            let child_depth = ancestor_path.len();
            let start = stack.len();
            stack.extend(dom.child_ids(id).map(|child| (child, child_depth)));
            stack[start..].reverse();
        } // cov:ignore: fallthrough-vs-continue region split inside a loop body; every test with an in-document element already exercises this closing brace, but cargo-llvm-cov does not attribute the hit to this line.
    }
}
pub(crate) fn specificity_of(selector: &Selector<RaikiriSelectorImpl>) -> Specificity {
    // Selector::specificity in selectors returns a packed 32-bit integer.
    selector.specificity()
}
/// Pick the winning declaration for each property key using specificity,
/// `!important`, and source order.
///
/// Write results to `best` rather than returning them. `best` is a
/// **direct-address table** indexed by the [`PropertyKey`] discriminant:
/// `best[k as usize]` stores the winner for `k` (an index into `candidates`).
///
/// # Why not return a `HashMap`?
///
/// [`PropertyKey`] is a payload-free one-byte enum with about 40 variants:
/// **already dense small integers** that do not need hashing and buckets.
/// The old implementation allocated two per-node `HashMap`s (working data and
/// return value), accounting for 3,667 allocations / 3.0 MB across 1,000
/// nodes: 56.7% of total cascade heap traffic. With slots, only the first
/// few nodes grow the buffer to its largest index; later nodes reuse it with
/// zero allocations.
///
/// The only caller is [`super::inherit::apply_winners`]. Its walk loop in
/// [`super::inherit::resolve_inheritance`] owns and reuses the buffer.
///
/// # Call contract
///
/// - **entry**: all `winners` slots are `None` (checked by debug_assert).
/// - **exit**: only slots for encountered keys are `Some`.
///
/// Normally [`super::inherit::apply_winners`] fills and drains the slots as
/// a pair. Keep the inexpensive debug_assert because the drain loop can stop
/// on unwind (for example, a panic in `apply_value`), leaving a slot alive.
/// The next node could then apply **another node's declaration**.
///
/// [`PropertyKey`]: crate::property::PropertyKey
pub(crate) fn pick_winners(candidates: &[CascadedDecl], winners: &mut Vec<Option<RankedDecl>>) {
    debug_assert!(
        winners.iter().all(Option::is_none),
        "pick_winners は空の scratch buffer を要求する — \
         前 node の winner slot が生き残っている (drain の unwind 等)"
    );

    let mut has_rollback = false;
    for (idx, (value, important, origin, spec, order, layer)) in candidates.iter().enumerate() {
        if matches!(value, PropertyValue::AllRevertLayer) {
            has_rollback = true;
            continue;
        }
        // Use the fieldless enum discriminant directly as the slot index.
        // `resize` handles new variants; no fixed upper bound is needed.
        let slot = value.key() as usize;
        if winners.len() <= slot {
            winners.resize(slot + 1, None);
        }
        let candidate = RankedDecl {
            rank: cascade_rank(*origin, *important),
            layer_priority: layer.priority(*important),
            specificity: *spec,
            source_order: *order,
            idx,
        };
        if winners[slot].is_none_or(|existing| beats(candidate, existing)) {
            winners[slot] = Some(candidate);
        }
    }
    if has_rollback {
        for winner in winners.iter_mut() {
            let Some(existing) = *winner else {
                continue;
            };
            let key = candidates[existing.idx].0.key();
            if matches!(
                key,
                crate::property::PropertyKey::Direction | crate::property::PropertyKey::UnicodeBidi
            ) {
                continue;
            }
            let selected = super::rollback::select_layered_winner(
                candidates,
                |idx, (value, important, origin, spec, order, layer)| {
                    let rollback = matches!(value, PropertyValue::AllRevertLayer);
                    if value.key() != key && !rollback {
                        return None;
                    }
                    Some((
                        (
                            cascade_rank(*origin, *important),
                            layer.priority(*important),
                            *spec,
                            *order,
                            idx,
                        ),
                        *origin,
                        *layer,
                        *important,
                        super::rollback::rollback_kind(value),
                    ))
                },
            );
            *winner = selected.map(|idx| {
                let (_, important, origin, spec, order, layer) = &candidates[idx];
                RankedDecl {
                    rank: cascade_rank(*origin, *important),
                    layer_priority: layer.priority(*important),
                    specificity: *spec,
                    source_order: *order,
                    idx,
                }
            });
        }
    }
}

pub(crate) fn beats(candidate: RankedDecl, existing: RankedDecl) -> bool {
    // Compare origin/importance, element attachment and layer, specificity, then source order.
    // - Higher rank wins (see `cascade_rank` for exact ordering and values:
    //   originally UA/Author, now UA/User/AuthorPresentationalHint/Author).
    // - At equal rank, element-attached declarations and stronger layers win.
    // - At equal origin and layer priority, higher specificity wins.
    // - At equal rank and specificity, later source_order wins.
    // Deliberately use `>=` so a later duplicate in the same rule wins,
    // per CSS Cascading L4 §6.1 "Order of Appearance"
    // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>: "the last
    // declaration in document order wins". Cross-rule source_order differs,
    // so `>=` is safe there too.
    //
    // Do **not** include `idx` in the comparison key. It increases with
    // candidate iteration order, so it would not change results, but would
    // obscure the explicit tie-break rule: later wins at equal rank/spec/order.
    (
        candidate.rank,
        candidate.layer_priority,
        candidate.specificity,
        candidate.source_order,
    ) >= (
        existing.rank,
        existing.layer_priority,
        existing.specificity,
        existing.source_order,
    )
}

#[cfg(test)]
mod tests;
