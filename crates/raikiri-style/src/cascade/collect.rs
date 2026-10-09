use crate::layer::LayerPosition;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{DefaultHasher, Hash, Hasher};

use cssparser::{Parser, ParserInput};
use selectors::parser::Selector;

use crate::PseudoElem;
use crate::RaikiriSelectorImpl;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::property::PropertyKey;
use crate::rule::{Declaration, parse_declaration_block_with_consumer_properties};

use super::candidate::{
    CandidateSink, ElementCandidates, ElementInput, Precedence, SharedDeclarations, pseudo_slot,
    push_rule_candidates, too_many,
};
use crate::ruletree::{Origin, RuleTree};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleQuirksMode};

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

/// Parsed `style`-attribute blocks for one cascade, keyed by source text.
///
/// Generated documents often repeat the same inline style on many elements;
/// storing each repeated string's declarations avoids re-tokenizing it per
/// element, and every element after the first with that source refers to
/// the stored block's declarations instead of copying them.
/// A source is only stored the **second** time it is seen: the first sight
/// records just its hash, so a document whose inline styles are all
/// different pays one hash and one small map entry per element instead of
/// an extra copy of every string. Entries are keyed by
/// hash but always confirm the full source text before reuse, so a hash
/// collision falls back to a fresh parse. Consumer property registrations
/// are fixed for the rule tree the cascade runs against, so the parse result
/// is a function of the source text alone.
#[derive(Default)]
struct DeclarationBlockCache {
    entries: HashMap<u64, DeclarationBlockEntry>,
    /// The stored blocks, at the positions cached handles name.
    blocks: Vec<Box<[Declaration]>>,
}

enum DeclarationBlockEntry {
    SeenOnce,
    /// The stored source text and the position of its block in `blocks`.
    Parsed(Box<str>, u32),
}

/// The declarations of one `style` attribute.
#[derive(Debug, PartialEq)]
enum DeclarationBlock {
    /// Parsed for this element alone: the first sight of its source, or a
    /// source whose hash collides with a stored one.
    Fresh(Vec<Declaration>),
    /// The stored block at this position.
    Cached(u32),
}

impl DeclarationBlockCache {
    /// The declarations of `source`. Stores the block the second time a
    /// source is seen, failing when its position or a declaration's position
    /// in it does not fit in a `u32` handle.
    fn declarations(
        &mut self,
        source: &str,
        rule_tree: &RuleTree,
    ) -> Result<DeclarationBlock, CascadeError> {
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
                Ok(DeclarationBlock::Fresh(parse()))
            }
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                if matches!(entry, DeclarationBlockEntry::SeenOnce) {
                    let declarations = parse();
                    let block = u32::try_from(self.blocks.len())
                        .map_err(|_| too_many("stored style attributes"))?;
                    u32::try_from(declarations.len())
                        .map_err(|_| too_many("declarations in one style attribute"))?;
                    self.blocks.push(declarations.into_boxed_slice());
                    *entry = DeclarationBlockEntry::Parsed(source.into(), block);
                }
                Ok(match entry {
                    DeclarationBlockEntry::Parsed(cached, block) if **cached == *source => {
                        DeclarationBlock::Cached(*block)
                    }
                    // A different source with the same hash: never reuse it.
                    _ => DeclarationBlock::Fresh(parse()),
                })
            }
        }
    }

    /// Adds a candidate for every declaration of `block`, with the precedence
    /// `precedence` gives it: a stored block's declarations are referred to,
    /// a fresh one's become the element's own.
    fn push(
        &self,
        sink: &mut CandidateSink<'_>,
        block: DeclarationBlock,
        precedence: impl Fn(&Declaration) -> Precedence,
    ) -> Result<(), CascadeError> {
        match block {
            DeclarationBlock::Fresh(declarations) => {
                for decl in declarations {
                    let precedence = precedence(&decl);
                    sink.push_local(Cow::Owned(decl), precedence)?;
                }
            }
            DeclarationBlock::Cached(block) => sink.push_cached(&self.blocks, block, precedence),
        }
        Ok(())
    }
}

/// Scratch slot for [`pick_winners`]: the provisional winner for one key.
///
/// Crucially, [`idx`](Self::idx) is an **index**, not a
/// [`PropertyValue`](crate::property::PropertyValue):
/// - The slot is `Copy` with no `Drop`, so [`Option::take`] resets it without
///   dropping or reallocating the entire buffer.
/// - Losing candidates are never cloned. Previously each candidate's
///   `value.clone()` was discarded if it lost; now only the winner is cloned
///   once for [`super::inherit::apply_value`].
///
/// Like [`Precedence`], this type uses named fields: [`beats`] compares
/// precedence fields **in order**, and
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
    /// Position in the candidates passed to [`pick_winners`].
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

/// Collects the cascade input of one element at a time, for one cascade.
///
/// The cascade walks the document once, in pre-order, and asks the collector
/// for each element's input just before resolving it. Collection only reads
/// the DOM and the rule tree, so collecting in the walk produces exactly what
/// a separate pass in the same order would: selector matching, hints, quirks
/// and the inline style cache all see the elements in document order.
pub(crate) struct Collector<'a, 'r, D: StyleDom> {
    dom: &'a D,
    rule_tree: &'r RuleTree,
    quirks_mode: StyleQuirksMode,
    match_ctx: MatchContext<'a, D>,
    /// The active style rules of this cascade, bucketed by the selectors'
    /// subject compounds.
    index: RuleIndex<'r>,
    block_cache: DeclarationBlockCache,
    cell_padding_cache: HashMap<StyleNodeId, Option<u32>>,
    /// Scratch list of the rules the index could not rule out for an element.
    candidate_rules: Vec<u32>,
}

impl<'a, 'r, D: StyleDom> Collector<'a, 'r, D> {
    /// A collector for the rules of `rule_tree` active under `media_context`.
    /// `match_caches` memoizes DOM-derived facts for every element of the
    /// cascade.
    ///
    /// # Errors
    ///
    /// Fails when the rule index cannot number the active rules (see
    /// [`RuleIndex::new`]).
    pub(crate) fn new(
        dom: &'a D,
        rule_tree: &'r RuleTree,
        media_context: &MediaContext,
        match_caches: &'a MatchCaches,
    ) -> Result<Self, CascadeError> {
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
        let match_ctx = MatchContext::new(dom, quirks_mode, None, false, match_caches);
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
        let index = RuleIndex::new(style_rules, &layers)?;
        Ok(Self {
            dom,
            rule_tree,
            quirks_mode,
            match_ctx,
            index,
            block_cache: DeclarationBlockCache::default(),
            cell_padding_cache: HashMap::new(),
            candidate_rules: Vec::new(),
        })
    }

    /// The declarations the collected inputs' rule and cached handles refer
    /// to.
    pub(crate) fn shared(&self) -> SharedDeclarations<'_> {
        SharedDeclarations::new(self.index.rules(), &self.block_cache.blocks)
    }

    /// Whether some selector of an active rule targets `pseudo`.
    pub(crate) fn targets(&self, pseudo: PseudoElem) -> bool {
        self.index.targets(pseudo)
    }

    /// Collects the input of element `id` into the empty `input`.
    ///
    /// `ancestor_path` holds `id`'s element ancestors, root-most first and its
    /// parent last, and `ancestor_filter` their hashes. Candidates are pushed
    /// in the order that decides ties: presentational hints and quirks,
    /// matching stylesheet rules in source order, then inline style and
    /// animation.
    ///
    /// # Errors
    ///
    /// Fails when the element has more own declarations than a local handle
    /// can number, or when the declaration block cache cannot number a stored
    /// `style` attribute or its declarations.
    pub(crate) fn collect<E: StyleElement>(
        &mut self,
        id: StyleNodeId,
        elem: &E,
        ancestor_path: &[StyleNodeId],
        ancestor_filter: &AncestorFilter,
        input: &mut ElementInput,
    ) -> Result<(), CascadeError> {
        let dom = self.dom;
        let (mut sink, pseudo_inputs) = input.parts();
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
        push_img_dimension_hints(elem, &mut sink)?;
        push_table_attribute_hints(
            dom,
            elem,
            ancestor_path,
            &mut self.cell_padding_cache,
            &mut sink,
        )?; // cov:ignore: the error branch needs a u32 handle overflow
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
            super::svg_hints::push_dimension_hints(elem, &mut sink)?;
        }
        if elem.namespace_uri() == Some(SVG_NAMESPACE) {
            if !is_svg_root {
                super::svg_hints::push_font_size_hint(elem, &mut sink)?;
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
                    sink.push_hint(value)?;
                }
            }
        }
        // HTML LS §15.3.9 margin-collapsing quirks (quirks-mode
        // margin-block zeroing) — `ancestor_path` here is still
        // `id`'s ancestor chain *without* `id` itself (the walk pushes it
        // only after this element's candidates are collected), so
        // `ancestor_path.last()` is exactly `id`'s real DOM parent. See
        // `push_margin_collapsing_quirk_declarations` doc for the
        // full rule set and design rationale.
        push_margin_collapsing_quirk_declarations(
            dom,
            id,
            elem,
            ancestor_path,
            self.quirks_mode,
            &mut sink,
        )?; // cov:ignore: the error branch needs a u32 handle overflow
        // stylesheet rule matching, restricted to the rules the
        // index could not rule out (still in source order)
        self.index
            .candidate_rules(elem, ancestor_filter, &mut self.candidate_rules);
        for &rule_idx in &self.candidate_rules {
            let indexed = self.index.rule(rule_idx);
            let rule = indexed.rule;
            if indexed.has_element_selector
                && let Some(spec) = match_complex_selector_list(
                    &rule.selectors,
                    self.match_ctx,
                    elem,
                    id,
                    ancestor_path,
                )
            {
                // The declarations were expanded when the rule was
                // parsed (`crate::rule::expand_shorthand_into`), and the
                // rule tree has no path that changes a rule after
                // parsing.
                sink.push_rule(self.index.rules(), rule_idx, |d| {
                    Precedence::new(
                        rule.origin,
                        d.important,
                        spec,
                        rule.source_order,
                        indexed.layer,
                    )
                });
            }
            if !indexed.has_pseudo_selector {
                continue;
            }
            // Pseudo-elements — independent pass over the same rule's selector
            // list (a rule's comma-separated list can target the real element
            // via one selector and a pseudo-element via another, e.g.
            // `a, a::before {..}`, so this isn't mutually exclusive with the
            // match above). `selector_matches_pseudo_element` fast-returns
            // `None` via `Selector::pseudo_element()`'s `O(1)` flag check for
            // the (overwhelmingly common) selector that doesn't target a
            // pseudo-element at all, so this second list walk stays cheap for
            // documents with no pseudo-element rules.
            for selector in rule.selectors.slice() {
                let Some(pseudo) = selector_matches_pseudo_element(
                    self.match_ctx,
                    selector,
                    elem,
                    id,
                    ancestor_path,
                ) else {
                    continue;
                };
                let spec = specificity_of(selector);
                // cov:ignore: selector_matches_pseudo_element excludes boxless native pseudos
                let Some(slot) = pseudo_slot(pseudo) else {
                    continue;
                };
                push_rule_candidates(
                    pseudo_inputs[slot].lists(),
                    self.index.rules(),
                    rule_idx,
                    |d| {
                        Precedence::new(
                            rule.origin,
                            d.important,
                            spec,
                            rule.source_order,
                            indexed.layer,
                        )
                    },
                );
            }
        }
        // inline style
        if let Some(source) = elem.inline_style_source() {
            let block = self.block_cache.declarations(source, self.rule_tree)?;
            self.block_cache.push(&mut sink, block, |decl| {
                Precedence::new(
                    Origin::Author,
                    decl.important,
                    INLINE_SPECIFICITY,
                    INLINE_SOURCE_ORDER,
                    LayerPosition {
                        attached: true,
                        ..LayerPosition::default()
                    },
                )
            })?;
        }
        if let Some(source) = elem.animation_style_source() {
            let block = self.block_cache.declarations(source, self.rule_tree)?;
            self.block_cache.push(&mut sink, block, |_| {
                Precedence::new(
                    Origin::Animation,
                    // Animated values are never important.
                    false,
                    INLINE_SPECIFICITY,
                    INLINE_SOURCE_ORDER,
                    LayerPosition::default(),
                )
            })?;
        }
        Ok(())
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
/// [`super::inherit::walk_from`] owns and reuses the buffer.
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
pub(crate) fn pick_winners(
    candidates: ElementCandidates<'_>,
    winners: &mut Vec<Option<RankedDecl>>,
) {
    debug_assert!(
        winners.iter().all(Option::is_none),
        "pick_winners は空の scratch buffer を要求する — \
         前 node の winner slot が生き残っている (drain の unwind 等)"
    );

    // Only the candidate records are read here; a declaration is looked up
    // only for the winners, when they are applied.
    let decls = candidates.decls();
    let mut has_rollback = false;
    for (idx, candidate) in decls.iter().enumerate() {
        if candidate.is_all_revert_layer() {
            has_rollback = true;
            continue;
        }
        // Use the fieldless enum discriminant directly as the slot index.
        // `resize` handles new variants; no fixed upper bound is needed.
        let slot = candidate.key() as usize;
        if winners.len() <= slot {
            winners.resize(slot + 1, None);
        }
        let ranked = candidate.precedence().ranked(idx);
        if winners[slot].is_none_or(|existing| beats(ranked, existing)) {
            winners[slot] = Some(ranked);
        }
    }
    if has_rollback {
        for winner in winners.iter_mut() {
            let Some(existing) = *winner else {
                continue;
            };
            let key = decls[existing.idx].key();
            if matches!(key, PropertyKey::Direction | PropertyKey::UnicodeBidi) {
                continue;
            }
            let selected = super::rollback::select_layered_winner(decls, |idx, candidate| {
                if candidate.key() != key && !candidate.is_all_revert_layer() {
                    return None;
                }
                Some(candidate.precedence().layered(idx, candidate.rollback()))
            });
            *winner = selected.map(|idx| decls[idx].precedence().ranked(idx));
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
