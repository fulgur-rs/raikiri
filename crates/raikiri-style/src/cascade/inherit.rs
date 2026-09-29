use std::collections::HashMap;
use std::sync::Arc;

use crate::PseudoElem;
use crate::computed::{
    ComputedValues, CustomPropertyEnvironment, RunningTemplate, empty_custom_properties,
};
use crate::property::longhand_value_pat;
use crate::property::{
    Border, BorderColor, BorderRadius, BorderStyle, CssWideKeyword, FontWeightValue,
    GridAutoFlowValue, GridLineValue, GridTemplateAreasValue, Length, LengthOrAuto,
    LetterSpacingValue, PositionValue, PropertyValue, RelativeFontSize, Sides, TextIndentLength,
    WordSpacingValue, WritingMode, initial_grid_auto_track_list,
    resolve_text_align_internal_center, resolve_text_align_match_parent,
};
use crate::resolve::{
    ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ResolveContext,
    used_line_height_length,
};
use crate::rule::{
    expand_background, expand_border, expand_border_color, expand_border_css_wide,
    expand_border_right, expand_border_right_css_wide, expand_border_style, expand_border_width,
    expand_flex, expand_flex_flow, expand_font, expand_gap, expand_grid_column, expand_grid_row,
    expand_margin, expand_margin_block, expand_margin_inline, expand_outline, expand_overflow,
    expand_padding, expand_padding_block, expand_padding_inline, expand_place_content,
    expand_place_items, expand_place_self, expand_text_decoration,
};
use crate::ruletree::Origin;
use crate::specified::{INITIAL_BORDER, SpecifiedValues};
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind};

use super::collect::{CascadedArena, CascadedDecl, RankedDecl, cascade_rank, pick_winners};
use super::custom_property::{resolve_custom_properties, resolve_deferred_value};

type InheritanceStackEntry = (
    StyleNodeId,
    Option<StyleNodeId>,
    Option<ResolveContext>,
    Arc<CustomPropertyEnvironment>,
);

/// Top-down inheritance walk. Children need their parent's computed values, so
/// each stack entry stores the parent's node ID and retrieves its already-written
/// values when visited. We save the parent's result before pushing its children
/// and never overwrite it during the tree walk. The ID lets us retrieve the value
/// again even if the output Vec grows. The first entry has no parent ID (`None`)
/// and uses `parent_computed` instead. This avoids cloning [`ComputedValues`] for
/// every child.
///
/// # Threading the `rem` context (design document §6.3)
///
/// The third field of each stack entry, `Option<ResolveContext>`, indicates
/// **whether this node has an element ancestor**:
///
/// - `None` — there is no element ancestor. If this node is an element, it is
///   the **root element** and goes through [`SpecifiedValues::finalize_as_root`].
///   That function's docs quote the parent-metrics clause of CSS Values 4 §6.1.1
///   (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim.
///   They explain why `rem` uses different bases for `font-size` (initial 16px)
///   and box properties (the element's own font-size). **This prevents the
///   incorrect self-reference in `html { font-size: 2rem }` that would result
///   from giving the entire tree one context.**
/// - `Some(ctx)` — there is an element ancestor. The computed font-size of the
///   highest such ancestor (= root element) is `ctx.root_font_size`; its `lh`
///   value (the `rlh` basis, absolutized by `used_line_height_length`, or `None`
///   when `normal` cannot be resolved) is `ctx.root_line_height`.
///
/// [`StyleDom::root_id`] identifies the Document node, not the root element
/// (see the Contract section of [`crate::style_dom`]). Non-element nodes such as
/// Document / Comment / Text therefore pass `None` through to their children.
///
/// A well-formed HTML document has one root element, but `StyleDom` does not
/// enforce this. In a synthetic DOM with several elements directly under the
/// Document, **each acts as a root element** (providing the `rem` basis for its
/// own subtree). This deliberately follows the §6.1.1 rule that elements
/// without an element parent use initial values.
///
/// `pub(crate)` permits intra-doc links from other modules; making this private
/// fails the documentation gate (repository rule 3).
#[allow(clippy::too_many_arguments)] // the traversal writes several independent cascade outputs
pub(crate) fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &CascadedArena,
    out: &mut Vec<ComputedValues>,
    non_ua_margin_sides: &mut Vec<Sides<bool>>,
    authored_writing_modes: &mut Vec<Option<WritingMode>>,
    page_values: &mut [crate::property::PageValue],
    pseudo_out: &mut HashMap<(StyleNodeId, PseudoElem), ComputedValues>,
) {
    let mut stack: Vec<InheritanceStackEntry> = vec![(id, None, None, empty_custom_properties())];
    // Scratch buffer for `apply_winners`, allocated **outside** the walk loop
    // and reused for every node. Two per-node `HashMap`s previously accounted
    // for 3,667 allocations / 3.0 MB at n=1000 nodes, or 56.7% of all cascade
    // heap traffic. The buffer grows to the maximum `PropertyKey` index over
    // the first few nodes, then incurs no allocations. `apply_winners` owns its
    // fill and drain; the walk loop only lends it a reusable container.
    let mut winners: Vec<Option<RankedDecl>> = Vec::new();
    while let Some((id, parent_id, root_ctx, parent_custom_properties)) = stack.pop() {
        // Skip the entire subtree when is_in_document()==false.
        //
        // Previously resize + write + children push ran unconditionally to
        // bring the computed length up to node_count(). Now `cascade()`
        // pre-allocates `computed` to `dom.node_count()` and fills it with
        // initial(), so unvisited slots naturally remain initial(). Thus:
        //   - detached / template descendants retain initial() instead of
        //     inheriting from inherit_from(parent) (nodes under
        //     `<template style="color:red">` do not inherit red)
        //   - the walk skips template subtrees (a performance improvement)
        //
        // Also skip unknown NodeIds (dom.node returns None). Leaving initial()
        // intact is safer than the old code's inherit_from before writing.
        let Some(node) = dom.node(id).filter(|n| n.is_in_document()) else {
            continue;
        };
        let is_element = node.kind() == StyleNodeKind::Element;
        let parent_computed = parent_id.map_or(parent_computed, |parent| &out[parent.0 as usize]);

        let local_custom_properties = cascaded
            .custom_candidates(id)
            .map(|candidates| resolve_custom_properties(&parent_custom_properties, candidates));
        let custom_properties = local_custom_properties
            .clone()
            .unwrap_or_else(|| parent_custom_properties.clone());
        let local_custom_properties =
            local_custom_properties.unwrap_or_else(empty_custom_properties);

        // Phase 1: apply this node's cascade winners to the initial state of
        // the inheritance walk (inherited fields copied from the parent's
        // computed values; non-inherited fields initialized). The target is a
        // staging representation, so winner application order does not matter.
        let mut specified = SpecifiedValues::inherit_from(parent_computed);
        let mut node_non_ua_margin = Sides::all(false);
        if authored_writing_modes.len() <= id.0 as usize {
            authored_writing_modes.resize(id.0 as usize + 1, None);
        }
        if let Some(candidates) = cascaded.candidates(id) {
            apply_winners(
                candidates,
                &mut winners,
                &mut specified,
                parent_computed,
                &custom_properties,
                Some(&mut page_values[id.0 as usize]),
                Some(&mut node_non_ua_margin),
                Some(&mut authored_writing_modes[id.0 as usize]),
            );
        }

        // Phases 2 + 3: absolutize. A root element (no element ancestor) uses
        // different `rem` bases in these phases, so it needs a dedicated entry
        // point (see the spec quote in `SpecifiedValues::finalize_as_root` docs).
        let mut computed = match &root_ctx {
            Some(ctx) => specified.finalize(parent_computed, ctx),
            None => {
                // `finalize_as_root` fixes the phase-2 basis to the initial
                // value (§6.1.1: "if the element has no parent"). This is
                // correct because **`root_ctx == None` means this node has no
                // element parent**. Check this caller-side invariant:
                //
                // - `cascade()` always starts at `dom.root_id()` (= Document
                //   node) and passes `ComputedValues::initial()` there.
                // - `collect_cascaded` creates winners only for Elements, so
                //   the Document node retains its initial computed values.
                // - Only the Document itself and its direct children still
                //   have `root_ctx == None` (`Some` follows the first element).
                //
                // Any future entry point that calls `resolve_inheritance` partway
                // through a subtree (e.g. incremental restyle) must therefore
                // supply `root_ctx` itself. This assertion catches an omission
                // in debug builds.
                debug_assert_eq!(
                    parent_computed.font_size,
                    ComputedLength(crate::computed::INITIAL_FONT_SIZE_PX),
                    "root_ctx == None は element 親が居ないことを意味するので、\
                     親の computed font-size は initial でなければならない \
                     (subtree の途中から walk を開始していないか?)"
                );
                specified.finalize_as_root()
            }
        };
        computed.custom_properties = custom_properties.clone();
        computed.local_custom_properties = local_custom_properties;

        // The rem/rlh context passed to children. Only after phases 2 + 2.5
        // for the root element are `root_font_size` / `root_line_height` known,
        // so this is the first point where the context becomes `Some`.
        // `used_line_height_length` uses the same derivation as
        // `crate::specified::SpecifiedValues::finalize_as_root` uses to build
        // its own `ctx`; `rlh_on_root_element_matches_child_root_line_height_basis`
        // checks that the two agree.
        let child_ctx = match root_ctx {
            Some(ctx) => Some(ctx),
            None if is_element => Some(ResolveContext::with_root_line_height(
                computed.font_size,
                used_line_height_length(computed.line_height, computed.font_size),
            )),
            None => None,
        };

        // `::before`/`::after` — CSS Pseudo-Elements Module Level 4 §4
        // <https://drafts.csswg.org/css-pseudo-4/#treelike> (tree-abiding
        // pseudo-elements, of which `::before`/`::after` are a subcase):
        // "They inherit any inheritable properties from their originating
        // element; non-inheritable properties take their initial values as
        // usual." This is computed inline here, right where `id`'s own real
        // children would be, rather than in a separate pass after this
        // whole walk finishes, for one specific reason: `child_ctx` (the
        // `rem`/`rlh` basis `id`'s real children get) is *not* a
        // document-wide constant — a document can have multiple top-level
        // elements directly under the `Document` node, each independently
        // becoming its own `root_ctx == None` root with its own `rem` basis
        // (see `resolve_inheritance`'s `root_ctx` doc above). A pseudo's
        // `rem`/`rlh` basis must match whichever one its own real
        // originating element actually resolved against, and that value
        // only exists as this loop's *local* `child_ctx` at this exact
        // point — reconstructing it after the fact would require redoing
        // this same top-level-root bookkeeping in a second pass. Using
        // `computed` (not `parent_computed`) as the inherited-from base and
        // `child_ctx` (not `root_ctx`) as the absolutization context both
        // follow directly from that §4 inheritance framing — a pseudo is
        // one more entry alongside `id`'s real children in every sense this
        // cascade cares about, just one this function itself resolves
        // instead of pushing onto `stack` (there is no `StyleNodeId` for a
        // pseudo-element to push). Whether a box is actually generated from
        // the result — §4.1 <https://drafts.csswg.org/css-pseudo-4/#generated-content>'s
        // separate, `content`-conditioned rule, "When their computed
        // 'content' value is not 'none', these pseudo-elements generate
        // boxes as if they were immediate children of their originating
        // element" — is a downstream (`raikiri-dom`) decision this crate
        // does not make; see `CascadeResult::pseudo`'s doc.
        if is_element {
            for pseudo in [PseudoElem::Before, PseudoElem::After, PseudoElem::Marker] {
                let candidates = cascaded.pseudo_candidates(id, pseudo);
                let custom_candidates = cascaded.pseudo_custom_candidates(id, pseudo);
                if candidates.is_none() && custom_candidates.is_none() {
                    continue;
                }
                let pseudo_local_custom_properties = custom_candidates
                    .map(|candidates| resolve_custom_properties(&custom_properties, candidates));
                let pseudo_custom_properties = pseudo_local_custom_properties
                    .clone()
                    .unwrap_or_else(|| custom_properties.clone());
                let pseudo_local_custom_properties =
                    pseudo_local_custom_properties.unwrap_or_else(empty_custom_properties);

                let mut pseudo_specified = SpecifiedValues::inherit_from(&computed);
                if let Some(candidates) = candidates {
                    apply_winners(
                        candidates,
                        &mut winners,
                        &mut pseudo_specified,
                        &computed,
                        &pseudo_custom_properties,
                        None,
                        None,
                        None,
                    );
                }

                let ctx = child_ctx.expect(
                    "a generated pseudo-element candidate only exists for a \
                     StyleNodeKind::Element (is_element == true here, since \
                     only elements are ever matched by a selector — \
                     `collect_cascaded`'s pseudo-element pass runs inside \
                     the same `if let Some(elem) = node.as_element()` guard \
                     as its real-element pass), and `child_ctx` is `Some` \
                     for every element by this point in the match above",
                );
                let mut pseudo_computed = pseudo_specified.finalize(&computed, &ctx);
                pseudo_computed.custom_properties = pseudo_custom_properties;
                pseudo_computed.local_custom_properties = pseudo_local_custom_properties;
                pseudo_out.insert((id, pseudo), pseudo_computed);
            }
        }

        // Resize out to id+1 before writing by index. Pre-allocation in
        // `cascade()` normally makes out.len() == node_count(), so this resize
        // is a no-op. Keep it as a safety net if a Dom implementation
        // underreports node_count().
        let idx = id.0 as usize;
        if out.len() <= idx {
            out.resize(idx + 1, ComputedValues::initial());
        }
        out[idx] = computed;
        if non_ua_margin_sides.len() <= idx {
            non_ua_margin_sides.resize(idx + 1, Sides::all(false));
        }
        non_ua_margin_sides[idx] = node_non_ua_margin;

        // Push children onto the stack, looking up their already-written
        // parent's computed value by ID. The stack is LIFO, so reverse the
        // children to visit them in document order. Extend `stack` directly
        // from the `child_ids` iterator, then reverse only the newly appended
        // suffix in place; no throwaway intermediate `Vec` is needed.
        // `child_ctx` is `Copy` (via `ResolveContext`) and can be reused inside
        // the closure.
        //
        // Why preserve document order? `resolve_inheritance` itself does not
        // rely on sibling visitation order: each node depends only on its
        // parent's saved computed values and child_ctx, and the `winners`
        // scratch buffer is fully drained before and after each node. We keep
        // document order to match behavior **exactly** before the refactor.
        // It also preserves the leak-detection direction documented by
        // `winner_does_not_leak_into_next_sibling`: earlier `<p>` then later
        // `<span>` in document order.
        let start = stack.len();
        stack.extend(
            dom.child_ids(id)
                .map(|child_id| (child_id, Some(id), child_ctx, custom_properties.clone())),
        );
        stack[start..].reverse();
    }
}

/// Choose the cascade winners for one node and apply them to the staging
/// representation (**phase 1**).
///
/// The caller allocates `winners` outside the walk loop as a scratch buffer.
/// This function pairs filling it ([`pick_winners`]) with draining it, leaving
/// every slot at `None` on return.
///
/// # Application order
///
/// Iterate slots by ascending index, i.e. [`PropertyKey`] **declaration order**.
/// [`Option::take`] restores each slot to `None` during this walk, also resetting
/// the buffer for the next node.
///
/// Shorthand keys must not reach this phase. CSS Cascading Level 4 §3
/// requires shorthand declarations to behave as if expanded in place, and
/// §6.1 determines the winner by order of appearance. Parsed entry points
/// expand shorthands before collecting candidates; the exhaustive expansion
/// match requires an explicit decision for new property variants.
///
/// # The unchecked `candidates[winner.idx]` index
///
/// `winner.idx` is in bounds because the immediately preceding [`pick_winners`]
/// produced it from **the same `candidates`**. Filling and draining occur next
/// to each other in this function body; no path swaps out `candidates` between
/// them.
///
/// The index is valid only because [`pick_winners`] produced it from the
/// same candidate slice immediately before this drain. A stale slot could
/// otherwise select another declaration, so the winner buffer is reset for
/// every node.
///
/// # Origin of `candidates` (after conversion to a flat arena)
///
/// The sole caller, [`resolve_inheritance`], obtains `candidates` only through
/// [`CascadedArena::candidates`]. That method always returns a slice containing
/// **exactly this node's range** (private fields prevent alternate slicing).
/// Thus `winner.idx` cannot point to **another node's declaration** through an
/// accidental slice in global index space. The only remaining risk described
/// above is a leaked slot that was not drained.
///
/// [`PropertyKey`]: crate::property::PropertyKey
// The winner application already groups several optional cascade side channels;
// the inherited computed values add one more required input for `inherit`
// resolution without changing that staging boundary.
/// Find the origin-rollback winner for `revert` / `revert-layer` on one border longhand.
///
/// CSS Cascading 4 §7.3.4 "The revert keyword" rolls back to the previous origin:
/// an Author `revert` uses the best User (or UA if no User) winner for the same
/// [`PropertyKey`], ignoring all Author declarations for that property. CSS Cascading 5
/// §6.5 carves out presentational hints for `revert` only ("it is considered part of
/// the author origin", not for `revert-layer`): an Author `revert` therefore also
/// ignores [`Origin::AuthorPresentationalHint`] candidates. This crate stores no style
/// layers for element rules, so `revert-layer` falls back to the same origin rollback
/// (see [`CssWideKeyword`]).
///
/// `winner_rank` / `winner_origin` come from the `revert` declaration that won
/// [`pick_winners`]. Among `candidates` with the same `key`, consider only those with
/// strictly lower [`cascade_rank`] (any lower origin tier, or the same origin tier at
/// lower importance when `!important` is involved) and, for `revert` from Author,
/// exclude both Author and presentational-hint origins per the carve-out above.
/// Pick the best among the survivors with the same ordering [`pick_winners`] uses
/// (rank, specificity, source order). Candidates that are themselves `revert` /
/// `revert-layer` markers are skipped to avoid recursion; when no survivor exists,
/// the caller falls back to the initial value (see [`INITIAL_BORDER`]).
///
/// Returns the rollback [`PropertyValue`] still in specified form (possibly
/// [`PropertyValue::Deferred`], which the caller resolves). Returns `None` when no
/// lower-origin winner exists.
fn find_border_rollback(
    candidates: &[CascadedDecl],
    key: crate::property::PropertyKey,
    winner_rank: u8,
    winner_origin: Origin,
    keyword: CssWideKeyword,
) -> Option<PropertyValue> {
    use crate::cascade::collect::RankedDecl;
    let is_revert = matches!(keyword, CssWideKeyword::Revert);
    let mut best: Option<(RankedDecl, PropertyValue)> = None;
    for (idx, (value, important, origin, spec, order)) in candidates.iter().enumerate() {
        if value.key() != key {
            continue;
        }
        // Skip other revert markers to avoid recursion.
        if matches!(
            value,
            PropertyValue::BorderTopWidthCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderRightWidthCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderBottomWidthCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderLeftWidthCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderTopStyleCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderRightStyleCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderBottomStyleCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderLeftStyleCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderTopColorCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderRightColorCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderBottomColorCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            ) | PropertyValue::BorderLeftColorCssWide(
                CssWideKeyword::Revert | CssWideKeyword::RevertLayer
            )
        ) {
            continue;
        }
        let rank = cascade_rank(*origin, *important);
        if rank >= winner_rank {
            continue;
        }
        // `revert` carve-out: Author rollback also ignores presentational hints.
        if is_revert
            && matches!(
                winner_origin,
                Origin::Author | Origin::AuthorPresentationalHint
            )
            && matches!(origin, Origin::Author | Origin::AuthorPresentationalHint)
        {
            continue;
        }
        let candidate = RankedDecl {
            rank,
            specificity: *spec,
            source_order: *order,
            idx,
        };
        let better = match &best {
            None => true,
            Some((existing, _)) => {
                use crate::cascade::collect::beats as beats_fn;
                beats_fn(candidate, *existing)
            }
        };
        if better {
            best = Some((candidate, value.clone()));
        }
    }
    best.map(|(_, v)| v)
}

/// Resolve one border longhand CSS-wide marker to its concrete specified value.
///
/// `inherited` supplies the parent computed border for `Inherit`. `INITIAL_BORDER`
/// supplies `Initial` and, because all `border-*` are non-inherited, `Unset`.
/// `Revert` / `RevertLayer` use [`find_border_rollback`]; when no lower-origin winner
/// exists they fall back to [`INITIAL_BORDER`]. A rollback winner that is
/// [`PropertyValue::Deferred`] is resolved through `custom_properties`; a rollback
/// winner that is itself a CSS-wide marker (only `Inherit` / `Initial` / `Unset`
/// can survive [`find_border_rollback`]'s revert skip) is resolved recursively one
/// level without further rollback.
fn resolve_border_css_wide(
    keyword: CssWideKeyword,
    key: crate::property::PropertyKey,
    inherited: &ComputedValues,
    candidates: &[CascadedDecl],
    winner_rank: u8,
    winner_origin: Origin,
    custom_properties: &CustomPropertyEnvironment,
) -> Option<PropertyValue> {
    let parent_side = |side: &crate::resolve::ComputedBorder| -> Border {
        Border {
            width: inherited_border_width(side.width()),
            style: side.style(),
            color: side.color,
        }
    };
    let parent_border = || -> Border {
        // Dispatch by key to the matching side; unreachable keys fall back to initial
        // (defensive: every border longhand key maps to one side below).
        match key {
            crate::property::PropertyKey::BorderTopWidth
            | crate::property::PropertyKey::BorderTopStyle
            | crate::property::PropertyKey::BorderTopColor => parent_side(&inherited.border.top),
            crate::property::PropertyKey::BorderRightWidth
            | crate::property::PropertyKey::BorderRightStyle
            | crate::property::PropertyKey::BorderRightColor => {
                parent_side(&inherited.border.right)
            }
            crate::property::PropertyKey::BorderBottomWidth
            | crate::property::PropertyKey::BorderBottomStyle
            | crate::property::PropertyKey::BorderBottomColor => {
                parent_side(&inherited.border.bottom)
            }
            _ => parent_side(&inherited.border.left),
        }
    };
    let initial_border = || -> Border { INITIAL_BORDER };
    let pick_field = |border: Border| -> PropertyValue {
        match key {
            crate::property::PropertyKey::BorderTopWidth => {
                PropertyValue::BorderTopWidth(border.width)
            }
            crate::property::PropertyKey::BorderRightWidth => {
                PropertyValue::BorderRightWidth(border.width)
            }
            crate::property::PropertyKey::BorderBottomWidth => {
                PropertyValue::BorderBottomWidth(border.width)
            }
            crate::property::PropertyKey::BorderLeftWidth => {
                PropertyValue::BorderLeftWidth(border.width)
            }
            crate::property::PropertyKey::BorderTopStyle => {
                PropertyValue::BorderTopStyle(border.style)
            }
            crate::property::PropertyKey::BorderRightStyle => {
                PropertyValue::BorderRightStyle(border.style)
            }
            crate::property::PropertyKey::BorderBottomStyle => {
                PropertyValue::BorderBottomStyle(border.style)
            }
            crate::property::PropertyKey::BorderLeftStyle => {
                PropertyValue::BorderLeftStyle(border.style)
            }
            crate::property::PropertyKey::BorderTopColor => {
                PropertyValue::BorderTopColor(border.color)
            }
            crate::property::PropertyKey::BorderRightColor => {
                PropertyValue::BorderRightColor(border.color)
            }
            crate::property::PropertyKey::BorderBottomColor => {
                PropertyValue::BorderBottomColor(border.color)
            }
            _ => PropertyValue::BorderLeftColor(border.color),
        }
    };
    match keyword {
        CssWideKeyword::Inherit => Some(pick_field(parent_border())),
        CssWideKeyword::Initial | CssWideKeyword::Unset => Some(pick_field(initial_border())),
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
            let rollback =
                find_border_rollback(candidates, key, winner_rank, winner_origin, keyword)?;
            // Resolve one level: Deferred needs custom-property substitution;
            // a surviving Inherit/Initial/Unset marker resolves without further rollback.
            match rollback {
                PropertyValue::Deferred(deferred) => {
                    let resolved = resolve_deferred_value(&deferred, custom_properties)?;
                    // A deferred var() may itself substitute to a CSS-wide keyword
                    // (e.g. `--x: inherit`). Resolve that single level directly
                    // against parent/initial to avoid rollback recursion.
                    match resolved {
                        PropertyValue::BorderTopWidthCssWide(kw)
                        | PropertyValue::BorderRightWidthCssWide(kw)
                        | PropertyValue::BorderBottomWidthCssWide(kw)
                        | PropertyValue::BorderLeftWidthCssWide(kw)
                        | PropertyValue::BorderTopStyleCssWide(kw)
                        | PropertyValue::BorderRightStyleCssWide(kw)
                        | PropertyValue::BorderBottomStyleCssWide(kw)
                        | PropertyValue::BorderLeftStyleCssWide(kw)
                        | PropertyValue::BorderTopColorCssWide(kw)
                        | PropertyValue::BorderRightColorCssWide(kw)
                        | PropertyValue::BorderBottomColorCssWide(kw)
                        | PropertyValue::BorderLeftColorCssWide(kw) => match kw {
                            CssWideKeyword::Inherit => Some(pick_field(parent_border())),
                            CssWideKeyword::Initial | CssWideKeyword::Unset => {
                                Some(pick_field(initial_border()))
                            }
                            CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
                                Some(pick_field(initial_border()))
                            }
                        },
                        v => Some(v),
                    }
                }
                PropertyValue::BorderTopWidthCssWide(kw)
                | PropertyValue::BorderRightWidthCssWide(kw)
                | PropertyValue::BorderBottomWidthCssWide(kw)
                | PropertyValue::BorderLeftWidthCssWide(kw)
                | PropertyValue::BorderTopStyleCssWide(kw)
                | PropertyValue::BorderRightStyleCssWide(kw)
                | PropertyValue::BorderBottomStyleCssWide(kw)
                | PropertyValue::BorderLeftStyleCssWide(kw)
                | PropertyValue::BorderTopColorCssWide(kw)
                | PropertyValue::BorderRightColorCssWide(kw)
                | PropertyValue::BorderBottomColorCssWide(kw)
                | PropertyValue::BorderLeftColorCssWide(kw) => match kw {
                    CssWideKeyword::Inherit => Some(pick_field(parent_border())),
                    CssWideKeyword::Initial | CssWideKeyword::Unset => {
                        Some(pick_field(initial_border()))
                    }
                    // Unreachable via find_border_rollback's skip, defensive fallback.
                    CssWideKeyword::Revert | CssWideKeyword::RevertLayer => {
                        Some(pick_field(initial_border()))
                    }
                },
                v => Some(v),
            }
            .or(Some(pick_field(initial_border())))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_winners(
    candidates: &[CascadedDecl],
    winners: &mut Vec<Option<RankedDecl>>,
    specified: &mut SpecifiedValues,
    inherited: &ComputedValues,
    custom_properties: &CustomPropertyEnvironment,
    mut page_value: Option<&mut crate::property::PageValue>,
    mut non_ua_margin_sides: Option<&mut Sides<bool>>,
    mut authored_writing_mode: Option<&mut Option<WritingMode>>,
) {
    pick_winners(candidates, winners);
    for slot in winners.iter_mut() {
        if let Some(winner) = slot.take() {
            let value = &candidates[winner.idx].0;
            if let Some(sides) = non_ua_margin_sides.as_deref_mut() {
                let non_ua = candidates[winner.idx].2 != Origin::UserAgent;
                match value.key() {
                    crate::property::PropertyKey::MarginTop => sides.top = non_ua,
                    crate::property::PropertyKey::MarginRight => sides.right = non_ua,
                    crate::property::PropertyKey::MarginBottom => sides.bottom = non_ua,
                    crate::property::PropertyKey::MarginLeft => sides.left = non_ua,
                    crate::property::PropertyKey::Margin
                    | crate::property::PropertyKey::MarginInline
                    | crate::property::PropertyKey::MarginBlock => {
                        sides.top = non_ua;
                        sides.right = non_ua;
                        sides.bottom = non_ua;
                        sides.left = non_ua;
                    }
                    _ => {}
                }
            }
            let winner_rank = winner.rank;
            let winner_origin = candidates[winner.idx].2;
            let winner_key = value.key();
            let value = match value {
                PropertyValue::BorderRadiusInherit => Some(PropertyValue::BorderRadius(
                    inherited_border_radius_value(inherited),
                )),
                PropertyValue::Deferred(deferred) => {
                    let resolved = resolve_deferred_value(deferred, custom_properties);
                    match resolved {
                        None => None,
                        Some(
                            PropertyValue::BorderTopWidthCssWide(kw)
                            | PropertyValue::BorderRightWidthCssWide(kw)
                            | PropertyValue::BorderBottomWidthCssWide(kw)
                            | PropertyValue::BorderLeftWidthCssWide(kw)
                            | PropertyValue::BorderTopStyleCssWide(kw)
                            | PropertyValue::BorderRightStyleCssWide(kw)
                            | PropertyValue::BorderBottomStyleCssWide(kw)
                            | PropertyValue::BorderLeftStyleCssWide(kw)
                            | PropertyValue::BorderTopColorCssWide(kw)
                            | PropertyValue::BorderRightColorCssWide(kw)
                            | PropertyValue::BorderBottomColorCssWide(kw)
                            | PropertyValue::BorderLeftColorCssWide(kw),
                        ) => resolve_border_css_wide(
                            kw,
                            winner_key,
                            inherited,
                            candidates,
                            winner_rank,
                            winner_origin,
                            custom_properties,
                        ),
                        Some(v) => Some(v),
                    }
                }
                PropertyValue::BorderTopWidthCssWide(kw)
                | PropertyValue::BorderRightWidthCssWide(kw)
                | PropertyValue::BorderBottomWidthCssWide(kw)
                | PropertyValue::BorderLeftWidthCssWide(kw)
                | PropertyValue::BorderTopStyleCssWide(kw)
                | PropertyValue::BorderRightStyleCssWide(kw)
                | PropertyValue::BorderBottomStyleCssWide(kw)
                | PropertyValue::BorderLeftStyleCssWide(kw)
                | PropertyValue::BorderTopColorCssWide(kw)
                | PropertyValue::BorderRightColorCssWide(kw)
                | PropertyValue::BorderBottomColorCssWide(kw)
                | PropertyValue::BorderLeftColorCssWide(kw) => resolve_border_css_wide(
                    *kw,
                    winner_key,
                    inherited,
                    candidates,
                    winner_rank,
                    winner_origin,
                    custom_properties,
                ),
                _ => Some(value.clone()),
            };
            if let Some(value) = value {
                if let PropertyValue::WritingMode(mode) = &value
                    && let Some(slot) = authored_writing_mode.as_deref_mut()
                {
                    *slot = Some(*mode);
                }
                if let crate::property::PropertyValue::Page(page) = &value
                    && let Some(page_slot) = page_value.as_deref_mut()
                {
                    *page_slot = page.clone();
                }
                apply_value(value, specified);
            }
        }
    }
}

/// Resolve a specified `font-weight` to a computed absolute weight.
///
/// CSS Fonts 4 §2.2.1 "Relative Weights"
/// <https://www.w3.org/TR/css-fonts-4/#relative-weights> supplies the
/// bolder / lighter table below, reproduced **exactly in six rows**
/// (`inherited` corresponds to the table's `w`):
///
/// | inherited value | `bolder` | `lighter` |
/// |---|---|---|
/// | `w < 100`       | 400 | `w` (no change) |
/// | `100 <= w < 350`| 400 | 100 |
/// | `350 <= w < 550`| 700 | 100 |
/// | `550 <= w < 750`| 900 | 400 |
/// | `750 <= w < 900`| 900 | 700 |
/// | `900 <= w`      | `w` (no change) | 700 |
///
/// # Why arithmetic expressions are incorrect
///
/// Approximations such as `min(w + 300, 900)` / `max(w - 300, 100)` omit the
/// two "no change" rows at the ends of the table. Authors can now specify
/// the full `<number [1,1000]>` range, making those rows reachable:
///
/// - Parent `font-weight: 1000` + child `bolder` → **1000** (no change in the
///   `900 <= w` row). `min(1300, 900)` would incorrectly return 900.
/// - Parent `font-weight: 50` + child `lighter` → **50** (no change in the
///   `w < 100` row). `max(-250, 100)` would incorrectly return 100.
///
/// The boundaries are half-open intervals (e.g. `350 <= w < 550`). The match
/// arms run top to bottom, with **only an upper bound** (`w < N`) in each
/// guard; the previous arm's failure supplies the lower bound. Thus **arm
/// order must match spec row order exactly**. Do not reorder these arms.
///
/// `pub(crate)` permits intra-doc links from other modules; making this
/// private fails the documentation gate (repository rule 3).
///
/// `inherited` and the result are `f32` (formerly `u16`). All table boundaries
/// (100 / 350 / 550 / 750 / 900) are integers, but `inherited` retains
/// fractional weights (e.g. `349.5`). Comparing without rounding selects the
/// precise row prescribed by spec §2.2.1. The old `u16` implementation rounded
/// `349.5` to `350` while parsing and incorrectly selected the `350 <= w < 550`
/// row (details in the `parse_font_weight` docs of [`crate::property`]).
///
/// # Non-finite `inherited` (`NaN` / `±Inf`) — not guarded here
///
/// The `u16` type structurally excluded non-finite values. With `f32`, the
/// caller must validate finiteness rather than relying on the type. The usual
/// cascade path always supplies finite values because `parse_font_weight` in
/// [`crate::property`] checks the `[1, 1000]` range. But every field of
/// `ComputedValues` is `pub`, and [`crate::page::cascade_page`] accepts a
/// caller-provided [`crate::page::PageInheritance`]`::FromRoot` as its inherited
/// root. A direct construction that bypasses the cascade
/// (`ComputedValues { font_weight: f32::NAN, .. }`) can theoretically reach here.
///
/// In both match arms, `<` against NaN is always false. Their different
/// catch-all arms make their behavior asymmetric: `Bolder` falls through to
/// `w => w` (the `900 <= w` no-change row), propagating `NaN` / `+Inf`;
/// `-Inf` matches the first `w < 100.0` guard and resolves to 400.0.
/// `Lighter` falls through to `_ => 700.0`, mapping `NaN` / `+Inf` to **700.0**;
/// `-Inf` matches the first guard and propagates unchanged.
///
/// The resolver leaves non-finite and out-of-range values to the sink
/// boundary, where the value is normalized before it reaches layout.
pub(crate) fn resolve_relative_weight(specified: FontWeightValue, inherited: f32) -> f32 {
    match specified {
        FontWeightValue::Absolute(w) => w,
        FontWeightValue::Bolder => match inherited {
            w if w < 100.0 => 400.0,
            w if w < 350.0 => 400.0,
            w if w < 550.0 => 700.0,
            w if w < 750.0 => 900.0,
            w if w < 900.0 => 900.0,
            // `900 <= w`: no change (return inherited values above 900, such as 1000)
            w => w,
        },
        FontWeightValue::Lighter => match inherited {
            // `w < 100`: no change (return inherited values below 100, such as 50)
            w if w < 100.0 => w,
            w if w < 350.0 => 100.0,
            w if w < 550.0 => 100.0,
            w if w < 750.0 => 400.0,
            w if w < 900.0 => 700.0,
            _ => 700.0,
        },
    }
}

/// Resolve `<relative-size>` (`larger` / `smaller`) for `font-size` against
/// the parent's computed font-size. This is the font-size counterpart of
/// [`resolve_relative_weight`] (same pattern as bolder/lighter).
///
/// CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop> verbatim:
///
/// > A `<relative-size>` keyword is interpreted relative to the computed
/// > font-size of the parent element and possibly the table of font sizes.
/// > \[…\] If the parent element has a keyword font size in the absolute size
/// > keyword mapping table, larger may compute the font size to the next
/// > entry in the table, and smaller may compute the font size to the
/// > previous entry in the table. \[…\] Instead of using next and previous
/// > items in the previous keyword table, User agents may instead use a
/// > simple ratio to increase or decrease the font size relative to the
/// > parent element. The specific ratio is unspecified, but should be around
/// > 1.2–1.5.
///
/// # Why the next/previous table-entry branch is not implemented
///
/// The spec offers both branches with "may" (both are permitted, neither is
/// required). Raikiri implements **only** the simple-ratio branch. The parser
/// (`parse_font_size_keyword`) already resolves `<absolute-size>` keywords to
/// `Length::Px` relative to `medium` at parse time and discards the keyword
/// variant. The inherited computed font-size cannot reveal whether the parent
/// specified a keyword, so we cannot safely check the table branch's premise
/// ("if the parent element has a keyword font size in the ... table").
///
/// # Why ratio = 1.2
///
/// The spec quote's "should be around 1.2–1.5" gives the lower end of the
/// allowed range. **This 1.2 has a different basis from the note in §2.5.1**
/// (which says CSS2's 1.2 scaling factor between adjacent indices was too
/// small at small sizes). Do not cite that note as the basis: it concerns the
/// table branch, while this function uses the simple-ratio branch.
pub(crate) fn resolve_relative_font_size(keyword: RelativeFontSize, inherited_px: f32) -> f32 {
    const RATIO: f32 = 1.2;
    match keyword {
        RelativeFontSize::Larger => inherited_px * RATIO,
        RelativeFontSize::Smaller => inherited_px / RATIO,
    }
}

/// Resolve a specified value against inherited computed values and return a
/// computed-equivalent value **still represented as `PropertyValue`**, wrapped
/// in [`ResolvedAgainstInherited`].
///
/// # Why this is separate from [`apply_value`]
///
/// [`apply_value`] writes results directly into fields of [`SpecifiedValues`];
/// callers needing a `PropertyValue` result cannot reuse it. The result of
/// [`crate::page::cascade_page`] exposes
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations)
/// publicly as a `HashMap<PropertyKey, PropertyValue>`, so values must pass
/// through this function before insertion. The CSS Fonts 4 §2.2.1 relative-
/// weight table exists only once in `resolve_relative_weight`; this function
/// and [`apply_value`] both call it rather than duplicating the table.
///
/// # Why there is no wildcard arm (contract)
///
/// The pass-through side lists every variant instead of using `_ => value`.
/// This is an intentional compile-time guard: **if a new property needs
/// inherited-value resolution, a wildcard would silently pass its unresolved
/// value into the public result**.
/// The pass-through side is exhaustive rather than using a wildcard. Adding a
/// property variant therefore requires an explicit decision about whether it
/// depends on inherited values. This compile-time guard does not detect new
/// entry points or new payload semantics inside an existing variant.
///
/// # Pass-through here does not mean "resolved" (phase 3 is still required)
///
/// This function performs only resolutions possible using **inherited computed
/// values alone**: **phase 2**. Values passed through still include specified
/// [`Length`] `Em` / `Rem` / `Pt` in box properties
/// ([`padding`](PropertyValue::PaddingTop) /
/// [`margin`](PropertyValue::MarginTop) / [`width`](PropertyValue::Width) /
/// [`height`](PropertyValue::Height) / `border-*-width`) and `line-height`.
/// CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> says, "Values in units of
/// em and ex are interpreted relative to the font associated with their
/// context". Thus `Em` uses the page context's own font, whose size **may come
/// from a sibling declaration in the same cascade** and cannot be determined
/// from `inherited` alone. `border-*-width` style gating (CSS Backgrounds 3 §3.3)
/// likewise requires a sibling `border-*-style` declaration.
///
/// **The caller must perform that resolution.** The sole caller,
/// [`crate::page::cascade_page`], immediately runs **phase 3**
/// (`absolutize_in_page_context` in [`crate::page`]) to absolutize against the
/// page context's font-size and apply style gating. Values are therefore
/// computed by the time they enter
/// [`PageCascadeResult::declarations`](crate::page::PageCascadeResult::declarations).
/// **Do not write a new caller that exposes this function's result directly.**
///
/// The element counterpart is [`apply_value`] → [`SpecifiedValues::finalize`]:
/// phase 2 determines font-size, then phase 3 absolutizes remaining values
/// against the element's own font-size. Both phase-3 paths share the same
/// functions in [`crate::resolve`], so each spec rule (`em` / `rem` bases,
/// unchanged percentages, border style gating) has one implementation.
///
/// # This function resolves `TextAlign::MatchParent`
///
/// [`TextAlign::MatchParent`](crate::property::TextAlign::MatchParent) can be
/// resolved using `inherited` alone: this covers the actual-parent case of
/// CSS Text 3 §6.1 `#valdef-text-align-match-parent`, not the root element's
/// "computes to start" case (see below). The `TextAlign` arm passes
/// `inherited.text_align` and `inherited.direction` to
/// [`crate::property::resolve_text_align_match_parent`]. Raikiri previously
/// could not implement this because it lacked computed `direction`.
///
/// ⚠️ **Trap**: "The page context inherits from the root element" in CSS Paged
/// Media 3 §6 **does not** mean the page context has no parent. `inherited`
/// always denotes the actual parent (or initial values under the L3 legacy
/// exception). The "Computes to start when specified on the root element"
/// exception quoted in the [`TextAlign`](crate::property::TextAlign) docs is
/// **not handled here**. A page context is not itself the root element, so it
/// never receives that exception. [`SpecifiedValues::finalize_as_root`] handles
/// this exception on the element path.
///
/// The public contract is [`crate::PageCascadeResult::declarations`].
///
/// `Percent` is not "unresolved": the computed value of a box property
/// retains its percentage. CSS Paged Media 3 §6 says "Percentage values on the
/// margin and padding properties are relative to the dimensions of the
/// containing block"; that is an input to the used-value stage (the quote is
/// on the canonical side). This matches the element path's
/// [`crate::resolve::resolve_length_percentage`].
///
/// # Why the result is [`ResolvedAgainstInherited`], not a bare [`PropertyValue`]
///
/// This type narrows the gap described in §2 of "What this guard does not
/// protect" above (a new entry point that does not call this function).
/// See the [`ResolvedAgainstInherited`] docs for details.
///
/// # Caller contract for `ctx`
///
/// Pass `ctx.root_line_height` derived from `inherited` via
/// (`used_line_height_length(inherited.line_height, inherited.font_size)`).
/// The `FontSize` arm's `lh`/`rlh` resolution (see below) requires that match.
/// The sole caller, [`crate::page::cascade_page`], computes the value once and
/// reuses it here and in phase 3 (`absolutize_in_page_context` in
/// [`crate::page`]). Because `inherited` is unchanged throughout the function,
/// computing it twice would yield the same result (see the caller's docs).
pub(crate) fn inherited_border_radius(value: ComputedLengthPercentage) -> Length {
    match value {
        ComputedLengthPercentage::Px(px) => Length::Px(px),
        ComputedLengthPercentage::Percent(percent) => Length::Percent(percent),
    }
}

/// Resolve one border width longhand CSS-wide marker for the page path
/// ([`resolve_against_inherited`]), which has no rollback candidates.
///
/// `is_top` / `is_right` / `is_bottom` select the side (all false = left).
/// `Inherit` lifts the parent computed side's gated width (see
/// [`inherited_border_width`]); `Initial` / `Unset` use [`INITIAL_BORDER`];
/// `Revert` / `RevertLayer` fall back to [`INITIAL_BORDER`] here because this
/// function has no candidate list — the page winner-selection in
/// [`crate::page::cascade_page`] replaces revert winners with their rollback
/// before calling this function, so reaching this arm via the normal page path
/// means no lower-origin winner existed.
fn resolve_border_page_longhand(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> Length {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => inherited_border_width(side.width()),
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.width,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.width,
    }
}

/// Page-path companion of [`resolve_border_page_longhand`] for `border-*-style`.
fn resolve_border_page_style(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> BorderStyle {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => side.style(),
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.style,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.style,
    }
}

/// Page-path companion for `border-*-color`.
fn resolve_border_page_color(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> BorderColor {
    let side = if is_top {
        &inherited.border.top
    } else if is_right {
        &inherited.border.right
    } else if is_bottom {
        &inherited.border.bottom
    } else {
        &inherited.border.left
    };
    match kw {
        CssWideKeyword::Inherit => side.color,
        CssWideKeyword::Initial | CssWideKeyword::Unset => INITIAL_BORDER.color,
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => INITIAL_BORDER.color,
    }
}

/// Page-path single-side resolver for `border-right: <css-wide>` (and the per-side
/// helper for [`resolve_border_page_sides`]).
fn resolve_border_page_side(
    kw: CssWideKeyword,
    inherited: &ComputedValues,
    is_top: bool,
    is_right: bool,
    is_bottom: bool,
) -> Border {
    Border {
        width: resolve_border_page_longhand(kw, inherited, is_top, is_right, is_bottom),
        style: resolve_border_page_style(kw, inherited, is_top, is_right, is_bottom),
        color: resolve_border_page_color(kw, inherited, is_top, is_right, is_bottom),
    }
}

/// Page-path resolver for `border: <css-wide>` — all four sides.
fn resolve_border_page_sides(kw: CssWideKeyword, inherited: &ComputedValues) -> Sides<Border> {
    Sides {
        top: resolve_border_page_side(kw, inherited, true, false, false),
        right: resolve_border_page_side(kw, inherited, false, true, false),
        bottom: resolve_border_page_side(kw, inherited, false, false, true),
        left: resolve_border_page_side(kw, inherited, false, false, false),
    }
}

fn inherited_border_radius_value(inherited: &ComputedValues) -> BorderRadius {
    BorderRadius {
        top_left: inherited_border_radius(inherited.border_radius.top_left),
        top_right: inherited_border_radius(inherited.border_radius.top_right),
        bottom_right: inherited_border_radius(inherited.border_radius.bottom_right),
        bottom_left: inherited_border_radius(inherited.border_radius.bottom_left),
    }
}

/// Lift a parent computed border width into specified form for `inherit`.
///
/// CSS Cascading 4 §7.3 takes the parent's computed value. For `border-*-width`
/// the computed value is already gated to zero when the parent's style is `none`
/// or `hidden` (see [`crate::resolve::resolve_border`]); inheriting that gated
/// zero is spec-correct. `ComputedLength` absolutizes to px (see [`crate::resolve`]),
/// so representing it as [`Length::Px`] is lossless.
fn inherited_border_width(computed: ComputedLength) -> Length {
    Length::Px(computed.0)
}

fn inherited_margin_length(value: ComputedLengthPercentageOrAuto) -> LengthOrAuto {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => LengthOrAuto::Length(Length::Px(px)),
        ComputedLengthPercentageOrAuto::Percent(percent) => {
            LengthOrAuto::Length(Length::Percent(percent))
        }
        ComputedLengthPercentageOrAuto::Calc(calc) => LengthOrAuto::Calc(calc),
        ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
    }
}

pub(crate) fn resolve_against_inherited(
    value: PropertyValue,
    inherited: &ComputedValues,
    ctx: &ResolveContext,
) -> ResolvedAgainstInherited {
    ResolvedAgainstInherited(match value {
        PropertyValue::BorderRadiusInherit => {
            PropertyValue::BorderRadius(inherited_border_radius_value(inherited))
        },
        PropertyValue::MarginTopInherit => {
            PropertyValue::MarginTop(inherited_margin_length(inherited.margin.top))
        }
        PropertyValue::MarginRightInherit => {
            PropertyValue::MarginRight(inherited_margin_length(inherited.margin.right))
        }
        PropertyValue::MarginBottomInherit => {
            PropertyValue::MarginBottom(inherited_margin_length(inherited.margin.bottom))
        }
        PropertyValue::MarginLeftInherit => {
            PropertyValue::MarginLeft(inherited_margin_length(inherited.margin.left))
        }
        PropertyValue::MarginInherit => PropertyValue::Margin(Sides {
            top: inherited_margin_length(inherited.margin.top),
            right: inherited_margin_length(inherited.margin.right),
            bottom: inherited_margin_length(inherited.margin.bottom),
            left: inherited_margin_length(inherited.margin.left),
        }),
        PropertyValue::BorderTopWidthCssWide(kw) => {
            PropertyValue::BorderTopWidth(resolve_border_page_longhand(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightWidthCssWide(kw) => {
            PropertyValue::BorderRightWidth(resolve_border_page_longhand(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomWidthCssWide(kw) => {
            PropertyValue::BorderBottomWidth(resolve_border_page_longhand(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftWidthCssWide(kw) => {
            PropertyValue::BorderLeftWidth(resolve_border_page_longhand(kw, inherited, false, false, false))
        }
        PropertyValue::BorderTopStyleCssWide(kw) => {
            PropertyValue::BorderTopStyle(resolve_border_page_style(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightStyleCssWide(kw) => {
            PropertyValue::BorderRightStyle(resolve_border_page_style(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomStyleCssWide(kw) => {
            PropertyValue::BorderBottomStyle(resolve_border_page_style(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftStyleCssWide(kw) => {
            PropertyValue::BorderLeftStyle(resolve_border_page_style(kw, inherited, false, false, false))
        }
        PropertyValue::BorderTopColorCssWide(kw) => {
            PropertyValue::BorderTopColor(resolve_border_page_color(kw, inherited, true, false, false))
        }
        PropertyValue::BorderRightColorCssWide(kw) => {
            PropertyValue::BorderRightColor(resolve_border_page_color(kw, inherited, false, true, false))
        }
        PropertyValue::BorderBottomColorCssWide(kw) => {
            PropertyValue::BorderBottomColor(resolve_border_page_color(kw, inherited, false, false, true))
        }
        PropertyValue::BorderLeftColorCssWide(kw) => {
            PropertyValue::BorderLeftColor(resolve_border_page_color(kw, inherited, false, false, false))
        }
        PropertyValue::BorderCssWide(kw) => {
            PropertyValue::Border(resolve_border_page_sides(kw, inherited))
        }
        PropertyValue::BorderRightCssWide(kw) => {
            PropertyValue::BorderRight(resolve_border_page_side(kw, inherited, false, true, false))
        }
        // CSS Fonts 4 §2.2.1 "Relative Weights"
        // <https://www.w3.org/TR/css-fonts-4/#relative-weights>: resolve `bolder` /
        // `lighter` against the inherited computed weight. Convert to
        // `Absolute` here so no relative keyword remains in the result.
        // The `Absolute(f32)` → `f32` → `Absolute(f32)` round-trip loses nothing.
        PropertyValue::FontWeight(fw) => PropertyValue::FontWeight(FontWeightValue::Absolute(
            resolve_relative_weight(fw, inherited.font_weight),
        )),
        // `font-size` needs only the inherited computed font-size. Delegate to
        // `crate::resolve::resolve_font_size` and wrap the result in `Length::Px`
        // to make it computed-equivalent (as the `FontWeight` arm returns
        // `Absolute(f32)`).
        //
        // Why `inherited.font_size` is the right basis:
        //
        // - `em`: CSS Paged Media 3 §6 "Page Properties"
        //   <https://www.w3.org/TR/css-page-3/#page-properties> verbatim —
        //   "When used on the font-size property in the page context, they are
        //   relative to the font-size of the root element."
        //   `cascade_page` receives the root element's `ComputedValues` as
        //   `inherited` (or initial values for
        //   `PageInheritance::LegacyInitialValues`). This is the §6 basis.
        // - `rem`: CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>
        //   "Equal to the computed value of the em unit on the root element." —
        //   again, `inherited.font_size`.
        // - `%`: **§6 does not specify `%` for page-context `font-size`.**
        //   CSS Fonts 4 §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>
        //   says "Percentages: refer to parent element's font size"; together
        //   with §6's "The page context inherits from the root element.", this
        //   implies the root element's font-size. This is an **inference**, not
        //   an explicit statement in §6.
        // - `px` / `pt`: absolute units, independent of context.
        // - `lh` / `rlh`: §6 does not specify `lh`/`rlh` either. By the same
        //   **inference** as for `%`, "the page context inherits from the root
        //   element" plus CSS Values 4 §6.1.1's self-reference clause makes
        //   the root element's used line-height the page context's `lh`
        //   self-reference basis. For this page context, "root" and "parent"
        //   are the same node (= `inherited`), so `rlh` has the same basis.
        //   The docs of `crate::page::page_context_line_height_basis` make
        //   this argument for `line-height` itself and note that the two bases
        //   coincide **only for this page context**. This lets us pass
        //   `ctx.root_line_height` as the `lh` self-reference basis too (as
        //   guaranteed by the caller contract for `ctx` above).
        //
        // This arm handles only `font-size`: `em` in box properties (`padding` /
        // `margin` / `width` / `height` / `border-*-width`) needs the page
        // context's own font-size, which may come from a sibling declaration
        // in this cascade and is not determined by `inherited` alone. The
        // caller (`cascade_page`) handles these in phase 3 after this function
        // (see "Pass-through here does not mean resolved" above).
        // **Always returning `Length::Px` here is essential**: phase 3 reads
        // it back as the page context's font-size (the `em` basis) via
        // `crate::page::page_context_font_size`. The next `FontSizeRelative`
        // arm also produces `PropertyValue::FontSize(Length::Px(_))`.
        PropertyValue::FontSize(len) => PropertyValue::FontSize(Length::Px(
            crate::resolve::resolve_font_size(len, inherited.font_size, ctx.root_line_height, ctx)
                .px(),
        )),
        // CSS Text 3 §6.1 `#valdef-text-align-match-parent`.
        // `inherited` is the page context's inheritance parent (the root
        // element, or initial values under the L3 legacy exception). Resolve
        // it as an **actual parent**; the page context is not itself the root
        // element and never receives its "computes to start" exception (see
        // the trap in the docs above). Other keywords are no-ops (see docs).
        PropertyValue::TextAlign(t) => PropertyValue::TextAlign(
            resolve_text_align_internal_center(
                resolve_text_align_match_parent(t, inherited.text_align, inherited.direction),
                inherited.text_align,
            ),
        ),
        // CSS Fonts 4 §2.5 `<relative-size>` (`larger` / `smaller`): like
        // `bolder` / `lighter`, resolve against the inherited computed
        // font-size. Convert to the `FontSize` variant. As the "resolution
        // timing" section of the `PropertyValue::FontSizeRelative` docs says,
        // this variant exists only temporarily for cascade winners, never in
        // the public result (`crate::page::PageCascadeResult::declarations`).
        PropertyValue::FontSizeRelative(rel) => PropertyValue::FontSize(Length::Px(
            resolve_relative_font_size(rel, inherited.font_size.px()),
        )),
        // Pass through properties not resolved here. The "Pass-through here
        // does not mean resolved (phase 3 is still required)" section above
        // explains how the caller's phase 3 absolutizes them. Do not use `_`.
        //
        // `Direction` belongs here: computed value = specified value (no
        // relative resolution; see `crate::property::Direction` docs), just
        // like `Color` / `FontFamily`.
        v @ (PropertyValue::Color(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::LineHeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::TextAutospace(_)
        | PropertyValue::WordSpaceTransform(_)
        | PropertyValue::TextSpacingTrim(_)
        | PropertyValue::Direction(_)
        | PropertyValue::TextIndent(_)
        | PropertyValue::PaddingTop(_)
        | PropertyValue::PaddingRight(_)
        | PropertyValue::PaddingBottom(_)
        | PropertyValue::PaddingLeft(_)
        | PropertyValue::Padding(_)
        // `padding-inline`/`padding-block` shorthand — same "nothing for
        // phase 2 to resolve" shape as `Padding`/`Margin` above (the
        // `<length-percentage>` font-relative absolutization happens later,
        // in phase 3).
        | PropertyValue::PaddingInline(_)
        | PropertyValue::PaddingBlock(_)
        | PropertyValue::MarginTop(_)
        | PropertyValue::MarginRight(_)
        | PropertyValue::MarginBottom(_)
        | PropertyValue::MarginLeft(_)
        | PropertyValue::Margin(_)
        | PropertyValue::MarginInline(_)
        | PropertyValue::MarginBlock(_)
        | PropertyValue::BorderTopWidth(_)
        | PropertyValue::BorderRightWidth(_)
        | PropertyValue::BorderBottomWidth(_)
        | PropertyValue::BorderLeftWidth(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::Border(_)
        | PropertyValue::BorderRight(_)
        // `border-style` / `border-width` / `border-color` shorthands —
        // same "nothing for phase 2 to resolve" shape as `Border` above
        // (non-inherited keywords/lengths; structurally unreachable here
        // since rule.rs expands them first).
        | PropertyValue::BorderStyle(_)
        | PropertyValue::BorderWidth(_)
        | PropertyValue::BorderColor(_)
        | PropertyValue::Width(_)
        | PropertyValue::Height(_)
        | PropertyValue::MaxWidth(_)
        | PropertyValue::MaxHeight(_)
        | PropertyValue::MinWidth(_)
        | PropertyValue::MinHeight(_)
        | PropertyValue::MinBlockSize(_)
        | PropertyValue::Top(_)
        | PropertyValue::Right(_)
        | PropertyValue::Bottom(_)
        | PropertyValue::Left(_)
        | PropertyValue::BoxSizing(_)
        // `overflow-x`/`overflow-y`/`overflow` join this arm — CSS Overflow 3
        // §3.1's cross-axis coupling (`resolve_overflow`) depends only on the
        // *other axis of the same node*, never on the inheritance parent, so
        // there is nothing for this function (phase 2) to resolve. It is
        // applied in phase 3 instead (`crate::page::absolutize_in_page_context`,
        // mirroring the element path's `SpecifiedValues::absolutize_with`).
        | PropertyValue::OverflowX(_)
        | PropertyValue::OverflowY(_)
        | PropertyValue::Overflow(_)
        // `writing-mode` joins this arm for the same reason `overflow-x`/
        // `overflow-y`/`overflow` do — the renderer-facing `HorizontalTb`
        // fallback (`resolve_writing_mode`, CSS Writing Modes 4 §3.2) depends
        // only on its own specified value, never on the inheritance parent.
        // The raw CSSOM computed keyword is retained separately in the element
        // computed values; this page/layout path still applies the fallback in
        // phase 3 (`crate::page::absolutize_in_page_context`).
        | PropertyValue::WritingMode(_)
        | PropertyValue::RubyPosition(_)
        // `text-decoration-line`/`-style`/`-color` (and the `text-decoration`
        // shorthand, structurally unreachable here per
        // `crate::rule::expand_shorthand_into`) carry no length. The inherited
        // `text-decoration-skip-ink`/`-skip-spaces` keywords also need no
        // phase-2 resolution because none of these values has a relative part.
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextEmphasisStyle(_)
        | PropertyValue::TextEmphasisColor(_)
        | PropertyValue::TextEmphasis(_)
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::TextDecoration(_)
        // `text-decoration-thickness` / `text-decoration-inset` carry a
        // `<length-percentage>` that needs the *declaring node's own*
        // font-size (phase 3), not the inheritance parent's — same shape
        // as `Padding`/`Margin`/`Width` above, nothing for phase 2 to
        // resolve here. (`TextDecoration` shorthand itself joins the group
        // above since expansion removes it before this function runs.)
        | PropertyValue::TextDecorationThickness(_)
        | PropertyValue::TextDecorationInset(_)
        | PropertyValue::TextUnderlineOffset(_)
        // `vertical-align`'s 6 keywords (`baseline`/`sub`/`super`/`middle`/
        // `text-top`/`text-bottom`) describe a shift *relative to the
        // parent's font metrics*, but that relation is a used-value/layout
        // concern (raikiri-paint scope, `VerticalAlign` doc's "baseline
        // shift calculation is raikiri-paint's scope" note) — CSS 2.1 §10.8.1's
        // computed value for these keywords is still the bare specified
        // keyword, so there is nothing for this function (phase 2,
        // inheritance-parent-relative resolution) to resolve. The
        // `<length>` variant needs *own-node* font-size resolution (`em`/
        // `rem`), which is phase 3's job
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_vertical_align` call, same split `FlexBasis` below
        // uses) — not this function's, since it depends on the declaring
        // node's own winners rather than the inheritance parent.
        | PropertyValue::VerticalAlign(_)
        // `font-style` carries no length at this crate's scope
        // (`normal`/`italic`/`oblique` implemented, `oblique`'s `<angle>`
        // argument is not — `FontStyle` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve.
        | PropertyValue::FontStyle(_)
        | PropertyValue::FontKerning(_)
        | PropertyValue::FontOpticalSizing(_)
        | PropertyValue::FontVariantEmoji(_)
        | PropertyValue::FontLanguageOverride(_)
        | PropertyValue::FontVariantLigatures(_)
        | PropertyValue::FontSynthesis(_)
        | PropertyValue::FontVariantPosition(_)
        | PropertyValue::FontPalette(_)
        | PropertyValue::FontVariantNumeric(_)
        | PropertyValue::FontVariantEastAsian(_)
        | PropertyValue::FontVariationSettings(_)
        // `text-transform` carries no length (`TextTransform` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::TextTransform(_)
        // `visibility` carries no length (`Visibility` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::Visibility(_)
        // `z-index` carries no length (`ZIndexValue` doc) and does not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::ZIndex(_)
        // `word-break` (CSS Text 3 §5.1) carries no length (`WordBreak`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WordBreak(_)
        // `overflow-wrap`/`word-wrap` (CSS Text 3 §5.4) carries no length
        // (`OverflowWrap` doc) either — same as `WordBreak` above.
        | PropertyValue::OverflowWrap(_)
        // `letter-spacing` / `word-spacing` carry a `<length>` that needs
        // the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `Padding`/`Margin`/`Width`/
        // `Height` above, nothing for phase 2 to resolve here.
        | PropertyValue::LetterSpacing(_)
        | PropertyValue::WordSpacing(_)
        // `tab-size`'s `<length>` alternative needs the same *declaring
        // node's own* font-size basis (phase 3) as `LetterSpacing`/
        // `WordSpacing` above; its `<number>` alternative carries no length
        // at all. Either way, nothing for phase 2 to resolve here.
        | PropertyValue::TabSize(_)
        // `break-before`/`break-after` (CSS Fragmentation Module Level 3
        // §3.1, legacy shorthand `page-break-before`/`page-break-after`
        // included) carry no length (`BreakBetween` doc) and do not
        // depend on the inheritance parent — nothing for phase 2 to
        // resolve.
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        // `break-inside` (CSS Fragmentation Module Level 3 §3.2, legacy
        // shorthand `page-break-inside` included) carries no length
        // either (`BreakInside` doc) — same as `BreakBefore`/`BreakAfter`
        // above.
        | PropertyValue::BreakInside(_)
        // `float` (CSS2 §9.5.1) carries no length and does not depend on
        // the inheritance parent — nothing for phase 2 to resolve. The
        // §9.7 `display` coupling this value drives is a same-node
        // dependency, resolved in phase 3
        // (`crate::specified::SpecifiedValues::absolutize_with`'s
        // `resolve_display_for_float` call), not here.
        | PropertyValue::Float(_)
        // `clear` (CSS2 §9.5.2) carries no length either — same as
        // `Float` above.
        | PropertyValue::Clear(_)
        // `white-space` (CSS Text 3 §3) carries no length (`WhiteSpace`
        // doc) and does not depend on the inheritance parent — nothing for
        // phase 2 to resolve.
        | PropertyValue::WhiteSpace(_)
        // `white-space-collapse` is a keyword-only value; nothing for phase 2
        // to resolve.
        | PropertyValue::WhiteSpaceCollapse(_)
        // `text-wrap` and `text-wrap-style` carry keyword values only; nothing
        // for phase 2 to resolve.
        | PropertyValue::TextWrap(_)
        | PropertyValue::TextWrapStyle(_)
        | PropertyValue::TextWrapShorthand(_)
        | PropertyValue::TextSpacingShorthand(_)
        // `flex-*` / alignment / `row-gap`/`column-gap` (and their
        // shorthands) — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Width`/`Height` above for the length-bearing
        // ones (`FlexBasis`/`RowGap`/`ColumnGap`/`Flex`/`Gap`; phase 3
        // (`absolutize_in_page_context`) does the declaring node's own
        // font-size resolution), and no length payload at all for the rest
        // (`FlexDirection`/`FlexWrap`/`FlexGrow`/`FlexShrink`/
        // `JustifyContent`/`AlignContent`/`AlignItems`/`AlignSelf`/
        // `PlaceContent`).
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        | PropertyValue::FlexBasis(_)
        | PropertyValue::Flex(_)
        | PropertyValue::FlexFlow(_)
        | PropertyValue::Order(_)
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        | PropertyValue::RowGap(_)
        | PropertyValue::ColumnGap(_)
        | PropertyValue::Gap(_)
        | PropertyValue::PlaceContent(_)
        // `hyphens` (CSS Text 3 §5.3) carries no length (`Hyphens` doc) and
        // does not depend on the inheritance parent — nothing for phase 2
        // to resolve.
        | PropertyValue::Hyphens(_)
        // `hyphenate-character` carries only `auto` or a decoded string; it
        // has no relative length for phase 2 to resolve.
        | PropertyValue::HyphenateCharacter(_)
        // `hyphenate-limit-chars` is a three-component integer/`auto` value;
        // no relative length or inheritance-parent resolution is needed.
        | PropertyValue::HyphenateLimitChars(_)
        // `font-variant-caps` (CSS Fonts Module Level 3 §6.6) carries no
        // length (`FontVariantCaps` doc) and does not depend on the
        // inheritance parent — nothing for phase 2 to resolve, same shape
        // as `FontStyle` above.
        | PropertyValue::FontVariantCaps(_)
        // `quotes` (CSS Content 3 §2.4.1) carries no length and does not
        // depend on the inheritance parent (computed value = specified
        // value, `ComputedValues::quotes` doc) — nothing for phase 2 to
        // resolve.
        | PropertyValue::Quotes(_)
        // `text-shadow`'s per-item lengths (offset-x/offset-y/blur-radius)
        // need the *declaring node's own* font-size (phase 3), not the
        // inheritance parent's — same shape as `LetterSpacing`/`WordSpacing`
        // above, nothing for phase 2 to resolve here. The `<color>`
        // component carries no length either (`TextShadowColor` doc).
        | PropertyValue::TextShadow(_)
        // F7/F8/F9 lengths need the declaring node/page context and therefore
        // remain for phase 3.
        | PropertyValue::BorderRadius(_)
        | PropertyValue::BorderRadiusTopLeft(_)
        | PropertyValue::BorderRadiusTopRight(_)
        | PropertyValue::BorderRadiusBottomRight(_)
        | PropertyValue::BorderRadiusBottomLeft(_)
        | PropertyValue::BoxShadow(_)
        | PropertyValue::Outline(_)
        | PropertyValue::OutlineWidth(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::OutlineOffset(_)
        // CSS Grid Layout Module Level 1 grid-template-columns/-rows/-areas
        // (§7.2/§7.3) + grid-auto-columns/-rows/-flow (§7.6/§7.7) +
        // grid-row-start/-end/grid-column-start/-end (+ their `grid-row`/
        // `grid-column` shorthands, §8.3/§8.4) — same "nothing for phase 2
        // to resolve" shape as `FlexBasis`/`RowGap` above for the
        // length-bearing track-sizing ones (`GridTemplateColumns`/
        // `GridTemplateRows`/`GridAutoColumns`/`GridAutoRows`; phase 3 does
        // the declaring node's own font-size resolution over the track
        // list), and no length payload at all for the rest
        // (`GridTemplateAreas`/`GridAutoFlow`/`GridRowStart`/`GridRowEnd`/
        // `GridColumnStart`/`GridColumnEnd`/`GridRow`/`GridColumn`).
        | PropertyValue::GridTemplateColumns(_)
        | PropertyValue::GridTemplateRows(_)
        | PropertyValue::GridTemplateAreas(_)
        | PropertyValue::GridAutoColumns(_)
        | PropertyValue::GridAutoRows(_)
        | PropertyValue::GridAutoFlow(_)
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        // justify-items (CSS Box Alignment Module Level 3 §7.1) /
        // justify-self (§6.1) / place-items (§7.3) / place-self (§6.3) —
        // same "no length payload" shape as `AlignItems`/`AlignSelf` above.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        // `orphans`/`widows` (CSS Fragmentation Module Level 3 §3.3) carry a
        // bare positive `<integer>`, not a length — nothing for phase 2 to
        // resolve, same shape as `FlexGrow`/`FlexShrink` above.
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        // background-repeat / background-attachment / background-clip /
        // background-origin / background-size / background-position (CSS
        // Backgrounds and Borders 3 §2.4-§2.9) — nothing for phase 2 to
        // resolve against the inheritance parent (all 6 are non-inherited,
        // `background-color`'s sibling shape); `background-size`/
        // `background-position`'s `<length-percentage>` absolutization is
        // phase 3's job (`crate::specified::SpecifiedValues::absolutize_with`
        // on the element path, `crate::page`'s `absolutize_in_page_context`
        // on the page path), same as `Padding`/`Width` above.
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)
        | PropertyValue::BackgroundSize(_)
        | PropertyValue::BackgroundPosition(_)
        // background-image (CSS Backgrounds and Borders 3 §2.3) — `None` /
        // `Url(String)` are opaque to font-size/line-height resolution, same
        // as its 4 keyword-only siblings just above. `Gradient(..)` *does*
        // carry `<length-percentage>`/`<angle>` payloads (CSS Images 4 §3),
        // but this crate deliberately never resolves them against the
        // inheritance parent (or at all, pre-paint) — resolving a gradient's
        // lengths needs the gradient box's own dimensions, an input phase 2
        // doesn't have, so the whole variant is left as specified-layer data
        // all the way into `ComputedValues` (`BackgroundImage` doc's scope
        // note). Nothing for phase 2 to do here either way.
        | PropertyValue::BackgroundImage(_)
        // `background` shorthand — same "nothing for phase 2 to resolve"
        // shape as `Padding`/`Margin`/`Border`/`Flex`/`Gap` above: its
        // `<length-percentage>` components (`position`/`size`) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Background(_)
        // `font` shorthand — same "nothing for phase 2 to resolve" shape as
        // `Padding`/`Margin`/`Border`/`Flex`/`Gap`/`Background` above: its
        // length-bearing components (`size` as <length-percentage>,
        // `line-height` as <number>/<length-percentage>) need the
        // *declaring node's own* font-size (phase 3), not the inheritance
        // parent's, and it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::Font(_)
        // mix-blend-mode (CSS Compositing and Blending Level 1 §3.4.1) —
        // non-inherited, bare keyword payload with no phase-2 dependency,
        // same "nothing for phase 2 to resolve" shape as `BackgroundRepeat`
        // above.
        | PropertyValue::MixBlendMode(_)
        // mask-image (CSS Masking Level 1 §7.1) / clip-path (§5.1) —
        // non-inherited, `<url>`/`<gradient>`/`<geometry-box>` payloads
        // that don't depend on the inheritance parent — nothing for phase
        // 2 to resolve here, same as `BackgroundImage` above (neither
        // crate absolutizes their embedded lengths at all — see
        // `MaskImage`/`ClipPath` doc's scope notes).
        | PropertyValue::MaskImage(_)
        | PropertyValue::ClipPath(_)
        // transform (CSS Transforms Level 1 §4) / filter (CSS Filter
        // Effects Level 1 §5) — non-inherited, embedded `Length`/`Angle`/
        // `f32` payloads that don't depend on the inheritance parent —
        // nothing for phase 2 to resolve here, same as `MaskImage`/
        // `ClipPath` above (neither absolutizes at all, see
        // `TransformFunction`/`FilterFunction` doc's scope notes).
        | PropertyValue::Transform(_)
        | PropertyValue::TransformOrigin(..)
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        // `border-spacing` (CSS Tables 3 §6.1): absolutizing `<length>{1,2}`
        // needs the declaring node's own font-size, so it belongs in phase 3
        // (like the "nothing for phase 2" `Padding`/`Margin` arms).
        // `caption-side` (§7) holds a bare keyword and has no phase-2
        // dependency (like `BorderCollapse`).
        | PropertyValue::BorderSpacing(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(_)
        | PropertyValue::Columns(_)
        // Table-declared longhands (`properties!` in property/decl.rs) have
        // no phase-2 dependency: a parent-dependent value would need its own
        // arm here.
        | longhand_value_pat!()) => v,
    })
}

/// Wrapper that **encodes in its type** passage through phase 2
/// ([`resolve_against_inherited`]). Its tuple field is private to the
/// `cascade` module: another module cannot construct this value without
/// calling [`resolve_against_inherited`].
///
/// Phase 3's `absolutize_in_page_context` in [`crate::page`] requires this
/// type. Thus any caller that reuses phase 3 on the page path must pass through
/// [`resolve_against_inherited`], regardless of its module. Even `page` itself
/// is foreign to the tuple field, since the type is defined in `cascade`:
/// there is no special access between these two modules.
///
/// # narrowed, not closed
///
/// A new path inside this module (`cascade.rs`) could access the tuple field
/// directly. A margin-box cascade that implements its own absolutization
/// instead of reusing phase 3 also bypasses this type. That remaining inability
/// to enumerate all paths is tracked separately in the margin-box cascade's
/// acceptance criteria. See §2 of "What this guard does not protect" in the
/// [`resolve_against_inherited`] docs.
///
/// This wrapper marks values that have passed phase 2 before phase 3.
/// The direct constructor exists only in test builds; production code uses
/// [`resolve_against_inherited`].
#[derive(Debug)]
pub(crate) struct ResolvedAgainstInherited(PropertyValue);

impl ResolvedAgainstInherited {
    /// Extract a value that has passed phase 2, taking ownership.
    pub(crate) fn into_property_value(self) -> PropertyValue {
        self.0
    }

    /// Inspect a value that has passed phase 2 without taking ownership.
    /// `page_context_font_size` / `page_context_border_styles` in [`crate::page`]
    /// use this to read `font-size` / `border-*-style` before phase 3.
    pub(crate) fn as_property_value(&self) -> &PropertyValue {
        &self.0
    }

    /// Test-only constructor for phase-3 inputs.
    #[cfg(test)]
    pub(crate) fn for_test(value: PropertyValue) -> Self {
        Self(value)
    }
}

/// Write one cascade winner to the staging representation
/// ([`SpecifiedValues`]) (**phase 1**).
///
/// Keep length-bearing properties in specified form and let
/// [`SpecifiedValues::finalize`] absolutize them: the font-size used for `em`
/// cannot be known until all of this node's winners have been applied.
/// Winners can be applied in any order, so resolutions depending on other
/// properties (such as `text-align: match-parent`) also do not happen here.
///
/// Exceptions are `bolder` / `lighter` in `font-weight` and `larger` /
/// `smaller` in `font-size`. [`SpecifiedValues::inherit_from`] has already
/// seeded `target` with the parent's computed values. These arms read the
/// inherited values from `target` before replacing them with absolute ones.
///
/// Shorthand arms are normally unreachable because
/// [`crate::rule::expand_shorthand_into`] expands them to longhands before the
/// cascade. If invoked directly, they delegate to the same per-family
/// expanders in `crate::rule` (e.g. [`crate::rule::expand_border`]).
///
/// `pub(crate)` permits intra-doc links from other modules.
pub(crate) fn apply_value(value: PropertyValue, target: &mut SpecifiedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        PropertyValue::BackgroundColor(c) => target.background_color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        PropertyValue::FontSizeRelative(rel) => {
            // Always the `Length::Px` parent seed: `FontSize` shares this
            // key's single winner slot, so it has not overwritten the seed.
            let inherited_px = target.font_size.payload();
            target.font_size = Length::Px(resolve_relative_font_size(rel, inherited_px));
        }
        PropertyValue::FontWeight(fw) => {
            let inherited = target.font_weight;
            target.font_weight = resolve_relative_weight(fw, inherited);
        }
        PropertyValue::LineHeight(lh) => target.line_height = lh,
        PropertyValue::Display(d) => target.display = d,
        PropertyValue::ListStyleType(v) => target.list_style_type = v,
        PropertyValue::ListStyleImage(v) => target.list_style_image = v,
        PropertyValue::ListStylePosition(v) => target.list_style_position = v,
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterResetInherit => {
            target.counter_reset = crate::property::empty_counter_entries();
        }
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
        PropertyValue::Content(v) => target.content = v,
        PropertyValue::StringSet(v) => target.string_set = v,
        PropertyValue::Position(pv) => match pv {
            PositionValue::Static => {
                target.position = PositionValue::Static;
            }
            PositionValue::Sticky => {
                target.position = PositionValue::Sticky;
            }
            PositionValue::Relative => {
                target.position = PositionValue::Relative;
            }
            PositionValue::Absolute => {
                target.position = PositionValue::Absolute;
            }
            PositionValue::Fixed => {
                target.position = PositionValue::Fixed;
            }
            PositionValue::Running(name) => {
                target.running_templates.push(RunningTemplate { name });
            }
        },
        PropertyValue::TextAlign(t) => target.text_align = t,
        PropertyValue::HangingPunctuation(v) => target.hanging_punctuation = v,
        // CSS Text 4: inherited keyword/flag set, simple by-value assignment.
        PropertyValue::TextAutospace(v) => target.text_autospace = v,
        PropertyValue::WordSpaceTransform(v) => target.word_space_transform = v,
        PropertyValue::TextSpacingTrim(v) => target.text_spacing_trim = v,
        PropertyValue::TextSpacingShorthand(v) => {
            target.text_spacing_trim = v.trim;
            target.text_autospace = v.autospace;
        }
        PropertyValue::TextJustify(v) => target.text_justify = v,
        PropertyValue::TextAlignLast(v) => target.text_align_last = v,
        PropertyValue::TextIndent(v) => {
            target.text_indent = v.length;
            target.text_indent_ch_factor = match v.length {
                TextIndentLength::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
            target.text_indent_ch_font = None;
            target.text_indent_ch_inherited = false;
            target.text_indent_hanging = v.hanging;
            target.text_indent_each_line = v.each_line;
        }
        PropertyValue::Direction(d) => target.direction = d,
        PropertyValue::PaddingTop(v) => target.padding.top = v,
        PropertyValue::PaddingRight(v) => target.padding.right = v,
        PropertyValue::PaddingBottom(v) => target.padding.bottom = v,
        PropertyValue::PaddingLeft(v) => target.padding.left = v,
        PropertyValue::Padding(sides) => expand_padding(sides, |v| apply_value(v, target)),
        PropertyValue::PaddingInline(pair) => {
            expand_padding_inline(pair, |v| apply_value(v, target))
        }
        PropertyValue::PaddingBlock(pair) => expand_padding_block(pair, |v| apply_value(v, target)),
        // Page-cascade-only inherit markers; the element parser never produces them.
        PropertyValue::BorderRadiusInherit
        | PropertyValue::MarginTopInherit
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeftInherit
        | PropertyValue::MarginInherit => {}
        PropertyValue::MarginTop(v) => target.margin.top = v,
        PropertyValue::MarginRight(v) => target.margin.right = v,
        PropertyValue::MarginBottom(v) => target.margin.bottom = v,
        PropertyValue::MarginLeft(v) => target.margin.left = v,
        PropertyValue::Margin(sides) => expand_margin(sides, |v| apply_value(v, target)),
        PropertyValue::MarginInline(pair) => expand_margin_inline(pair, |v| apply_value(v, target)),
        PropertyValue::MarginBlock(pair) => expand_margin_block(pair, |v| apply_value(v, target)),
        PropertyValue::BorderTopWidth(v) => target.border.top.width = v,
        PropertyValue::BorderRightWidth(v) => target.border.right.width = v,
        PropertyValue::BorderBottomWidth(v) => target.border.bottom.width = v,
        PropertyValue::BorderLeftWidth(v) => target.border.left.width = v,
        PropertyValue::BorderTopStyle(v) => target.border.top.style = v,
        PropertyValue::BorderRightStyle(v) => target.border.right.style = v,
        PropertyValue::BorderBottomStyle(v) => target.border.bottom.style = v,
        PropertyValue::BorderLeftStyle(v) => target.border.left.style = v,
        PropertyValue::BorderTopColor(v) => target.border.top.color = v,
        PropertyValue::BorderRightColor(v) => target.border.right.color = v,
        PropertyValue::BorderBottomColor(v) => target.border.bottom.color = v,
        PropertyValue::BorderLeftColor(v) => target.border.left.color = v,
        // Border longhand CSS-wide markers are resolved in `apply_winners` before
        // reaching here (they need the parent computed value or rollback candidates).
        // Keep them panic-free for direct callers that bypass that phase.
        PropertyValue::BorderTopWidthCssWide(_)
        | PropertyValue::BorderRightWidthCssWide(_)
        | PropertyValue::BorderBottomWidthCssWide(_)
        | PropertyValue::BorderLeftWidthCssWide(_)
        | PropertyValue::BorderTopStyleCssWide(_)
        | PropertyValue::BorderRightStyleCssWide(_)
        | PropertyValue::BorderBottomStyleCssWide(_)
        | PropertyValue::BorderLeftStyleCssWide(_)
        | PropertyValue::BorderTopColorCssWide(_)
        | PropertyValue::BorderRightColorCssWide(_)
        | PropertyValue::BorderBottomColorCssWide(_)
        | PropertyValue::BorderLeftColorCssWide(_) => {}
        PropertyValue::Border(sides) => expand_border(sides, |v| apply_value(v, target)),
        PropertyValue::BorderRight(border) => {
            expand_border_right(border, |v| apply_value(v, target))
        }
        PropertyValue::BorderCssWide(kw) => expand_border_css_wide(kw, |v| apply_value(v, target)),
        PropertyValue::BorderRightCssWide(kw) => {
            expand_border_right_css_wide(kw, |v| apply_value(v, target))
        }
        PropertyValue::Width(v) => target.width = v,
        PropertyValue::Height(v) => target.height = v,
        PropertyValue::MaxWidth(v) => target.max_width = v,
        PropertyValue::MaxHeight(v) => target.max_height = v,
        PropertyValue::MinWidth(v) => target.min_width = v,
        PropertyValue::MinHeight(v) => target.min_height = v,
        PropertyValue::MinBlockSize(v) => target.min_block_size = Some(v),
        PropertyValue::Top(v) => target.top = v,
        PropertyValue::Right(v) => target.right = v,
        PropertyValue::Bottom(v) => target.bottom = v,
        PropertyValue::Left(v) => target.left = v,
        PropertyValue::BoxSizing(bs) => target.box_sizing = bs,
        PropertyValue::OverflowX(v) => target.overflow.x = v,
        PropertyValue::OverflowY(v) => target.overflow.y = v,
        PropertyValue::Overflow(pair) => expand_overflow(pair, |v| apply_value(v, target)),
        PropertyValue::TextDecorationLine(v) => target.text_decoration_line = v,
        PropertyValue::TextDecorationStyle(v) => target.text_decoration_style = v,
        PropertyValue::TextDecorationColor(v) => target.text_decoration_color = v,
        PropertyValue::TextDecorationThickness(v) => target.text_decoration_thickness = v,
        PropertyValue::TextDecorationSkipInk(v) => target.text_decoration_skip_ink = v,
        PropertyValue::TextDecorationSkipSpaces(v) => target.text_decoration_skip_spaces = v,
        PropertyValue::TextDecorationInset(v) => target.text_decoration_inset = v,
        PropertyValue::TextUnderlineOffset(v) => target.text_underline_offset = v,
        PropertyValue::TextUnderlinePosition(v) => target.text_underline_position = v,
        PropertyValue::TextEmphasisPosition(v) => target.text_emphasis_position = v,
        PropertyValue::TextEmphasisStyle(v) => target.text_emphasis_style = v,
        PropertyValue::TextEmphasisColor(v) => target.text_emphasis_color = v,
        PropertyValue::TextEmphasis(shorthand) => {
            crate::rule::expand_text_emphasis(&shorthand, |value| apply_value(value, target))
        }
        PropertyValue::TextDecoration(shorthand) => {
            expand_text_decoration(shorthand, |v| apply_value(v, target))
        }
        PropertyValue::VerticalAlign(va) => target.vertical_align = va,
        PropertyValue::FontStyle(fs) => target.font_style = fs,
        PropertyValue::FontKerning(value) => target.font_kerning = value,
        PropertyValue::FontOpticalSizing(value) => target.font_optical_sizing = value,
        PropertyValue::FontVariantEmoji(value) => target.font_variant_emoji = value,
        PropertyValue::FontLanguageOverride(value) => target.font_language_override = value,
        PropertyValue::FontVariantLigatures(value) => target.font_variant_ligatures = value,
        PropertyValue::FontSynthesis(value) => target.font_synthesis = value,
        PropertyValue::FontVariantPosition(value) => target.font_variant_position = value,
        PropertyValue::FontPalette(value) => target.font_palette = value,
        PropertyValue::FontVariantNumeric(value) => target.font_variant_numeric = value,
        PropertyValue::FontVariantEastAsian(value) => target.font_variant_east_asian = value,
        PropertyValue::FontVariationSettings(value) => target.font_variation_settings = value,
        PropertyValue::TextTransform(tt) => target.text_transform = tt,
        PropertyValue::TextCombineUpright(value) => target.text_combine_upright = value,
        PropertyValue::TextOrientation(value) => target.text_orientation = value,
        PropertyValue::UnicodeBidi(value) => target.unicode_bidi = value,
        PropertyValue::Visibility(v) => target.visibility = v,
        PropertyValue::ZIndex(z) => target.z_index = z,
        PropertyValue::WordBreak(wb) => target.word_break = wb,
        PropertyValue::LineBreak(lb) => target.line_break = lb,
        PropertyValue::OverflowWrap(ow) => target.overflow_wrap = ow,
        PropertyValue::LetterSpacing(ls) => {
            target.letter_spacing = ls;
            target.letter_spacing_ch_factor = match ls {
                LetterSpacingValue::Length(Length::Ch(factor)) if factor.is_finite() => {
                    Some(factor)
                }
                _ => None,
            };
            target.letter_spacing_ch_font = None;
        }
        PropertyValue::WordSpacing(ws) => {
            target.word_spacing = ws;
            target.word_spacing_ch_factor = match ws {
                WordSpacingValue::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
                _ => None,
            };
            target.word_spacing_ch_font = None;
        }
        PropertyValue::TabSize(ts) => target.tab_size = ts,
        PropertyValue::BreakBefore(bb) => target.break_before = bb,
        PropertyValue::BreakAfter(bb) => target.break_after = bb,
        PropertyValue::BreakInside(bi) => target.break_inside = bi,
        PropertyValue::Float(f) => target.float = f,
        PropertyValue::Clear(c) => target.clear = c,
        PropertyValue::WhiteSpace(ws) => target.white_space = ws,
        PropertyValue::WhiteSpaceCollapse(value) => target.white_space_collapse = value,
        PropertyValue::TextWrap(v) => target.text_wrap = v,
        PropertyValue::TextWrapStyle(v) => target.text_wrap_style = v,
        PropertyValue::TextWrapShorthand(v) => {
            target.text_wrap = v.mode;
            target.text_wrap_style = v.style;
        }
        PropertyValue::Hyphens(h) => target.hyphens = h,
        PropertyValue::HyphenateCharacter(value) => target.hyphenate_character = value,
        PropertyValue::HyphenateLimitChars(value) => target.hyphenate_limit_chars = value,
        PropertyValue::FlexDirection(fd) => target.flex_direction = fd,
        PropertyValue::FlexWrap(fw) => target.flex_wrap = fw,
        PropertyValue::FlexGrow(g) => target.flex_grow = g,
        PropertyValue::FlexShrink(s) => target.flex_shrink = s,
        PropertyValue::FlexBasis(fb) => target.flex_basis = fb,
        PropertyValue::Flex(f) => expand_flex(f, |v| apply_value(v, target)),
        PropertyValue::FlexFlow(f) => expand_flex_flow(f, |v| apply_value(v, target)),
        PropertyValue::Order(o) => target.order = o,
        PropertyValue::JustifyContent(jc) => target.justify_content = jc,
        PropertyValue::AlignContent(ac) => target.align_content = ac,
        PropertyValue::AlignItems(ai) => target.align_items = ai,
        PropertyValue::AlignSelf(as_) => target.align_self = as_,
        PropertyValue::RowGap(rg) => target.row_gap = rg,
        PropertyValue::ColumnGap(cg) => target.column_gap = cg,
        PropertyValue::Gap(g) => expand_gap(g, |v| apply_value(v, target)),
        PropertyValue::PlaceContent(p) => expand_place_content(p, |v| apply_value(v, target)),
        PropertyValue::FontVariantCaps(fvc) => target.font_variant_caps = fvc,
        PropertyValue::Quotes(v) => {
            target.quotes_auto = false;
            target.quotes = v;
        }
        PropertyValue::TextShadow(shadows) => target.text_shadow = shadows,
        PropertyValue::BorderRadius(v) => target.border_radius = v,
        PropertyValue::BorderRadiusTopLeft(v) => target.border_radius.top_left = v,
        PropertyValue::BorderRadiusTopRight(v) => target.border_radius.top_right = v,
        PropertyValue::BorderRadiusBottomRight(v) => target.border_radius.bottom_right = v,
        PropertyValue::BorderRadiusBottomLeft(v) => target.border_radius.bottom_left = v,
        PropertyValue::BoxShadow(shadows) => target.box_shadow = shadows,
        PropertyValue::Outline(outline) => expand_outline(outline, |v| apply_value(v, target)),
        PropertyValue::OutlineWidth(v) => target.outline.width = v,
        PropertyValue::OutlineStyle(v) => target.outline.style = v,
        PropertyValue::OutlineColor(v) => target.outline.color = v,
        PropertyValue::OutlineOffset(v) => target.outline_offset = v,
        PropertyValue::GridArea(area) => {
            target.grid_row_start = area.row_start;
            target.grid_column_start = area.column_start;
            target.grid_row_end = area.row_end;
            target.grid_column_end = area.column_end;
        }
        PropertyValue::Grid(shorthand) => {
            target.grid_template_rows = shorthand.rows;
            target.grid_template_columns = shorthand.columns;
            target.grid_template_areas = GridTemplateAreasValue::None;
            target.grid_auto_columns = initial_grid_auto_track_list();
            target.grid_auto_rows = initial_grid_auto_track_list();
            target.grid_auto_flow = GridAutoFlowValue::Row;
            target.grid_row_start = GridLineValue::Auto;
            target.grid_row_end = GridLineValue::Auto;
            target.grid_column_start = GridLineValue::Auto;
            target.grid_column_end = GridLineValue::Auto;
        }
        PropertyValue::GridTemplateColumns(v) => target.grid_template_columns = v,
        PropertyValue::GridTemplateRows(v) => target.grid_template_rows = v,
        PropertyValue::GridTemplateAreas(v) => target.grid_template_areas = v,
        PropertyValue::GridAutoColumns(v) => target.grid_auto_columns = v,
        PropertyValue::GridAutoRows(v) => target.grid_auto_rows = v,
        PropertyValue::GridAutoFlow(v) => target.grid_auto_flow = v,
        PropertyValue::GridRowStart(v) => target.grid_row_start = v,
        PropertyValue::GridRowEnd(v) => target.grid_row_end = v,
        PropertyValue::GridColumnStart(v) => target.grid_column_start = v,
        PropertyValue::GridColumnEnd(v) => target.grid_column_end = v,
        PropertyValue::GridRow(shorthand) => {
            expand_grid_row(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::GridColumn(shorthand) => {
            expand_grid_column(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::JustifyItems(v) => target.justify_items = v,
        PropertyValue::JustifySelf(v) => target.justify_self = v,
        PropertyValue::PlaceItems(p) => expand_place_items(p, |v| apply_value(v, target)),
        PropertyValue::PlaceSelf(p) => expand_place_self(p, |v| apply_value(v, target)),
        PropertyValue::Orphans(n) => target.orphans = n,
        PropertyValue::Widows(n) => target.widows = n,
        PropertyValue::WritingMode(v) => target.writing_mode = v,
        PropertyValue::RubyPosition(v) => target.ruby_position = v,
        PropertyValue::BackgroundRepeat(v) => target.background_repeat = v,
        PropertyValue::BackgroundAttachment(v) => target.background_attachment = v,
        PropertyValue::BackgroundClip(v) => target.background_clip = v,
        PropertyValue::BackgroundOrigin(v) => target.background_origin = v,
        PropertyValue::BackgroundSize(v) => target.background_size = v,
        PropertyValue::BackgroundPosition(v) => target.background_position = v,
        PropertyValue::BackgroundImage(v) => target.background_image = v,
        PropertyValue::Background(shorthand) => {
            expand_background(&shorthand, |v| apply_value(v, target))
        }
        PropertyValue::BorderStyle(sides) => expand_border_style(sides, |v| apply_value(v, target)),
        PropertyValue::BorderWidth(sides) => expand_border_width(sides, |v| apply_value(v, target)),
        PropertyValue::BorderColor(sides) => expand_border_color(sides, |v| apply_value(v, target)),
        PropertyValue::Font(shorthand) => expand_font(&shorthand, |v| apply_value(v, target)),
        PropertyValue::MixBlendMode(v) => target.mix_blend_mode = v,
        PropertyValue::MaskImage(v) => target.mask_image = v,
        PropertyValue::ClipPath(v) => target.clip_path = v,
        PropertyValue::Transform(v) => target.transform = v,
        PropertyValue::TransformOrigin(position, z) => {
            target.transform_origin = position;
            target.transform_origin_z = z;
        }
        PropertyValue::Filter(v) => target.filter = v,
        PropertyValue::TableLayout(v) => target.table_layout = v,
        PropertyValue::BorderCollapse(v) => target.border_collapse = v,
        PropertyValue::BorderSpacing(v) => target.border_spacing = v,
        PropertyValue::CaptionSide(v) => target.caption_side = v,
        // Parsed but not yet staged for elements.
        PropertyValue::TextAlignAll(_) => {}
        PropertyValue::Page(value) => target.page = value,
        PropertyValue::ColumnCount(value) => target.column_count = value,
        PropertyValue::ColumnWidth(value) => target.column_width = value,
        // cov:ignore: direct unexpanded shorthand callers are defensive-only.
        PropertyValue::Columns(value) => {
            target.column_count = value.count;
            target.column_width = value.width;
        }
        // Resolved before ordinary winners reach this function.
        PropertyValue::CustomProperty(_) | PropertyValue::Deferred(_) => {}
        v @ longhand_value_pat!() => target.longhands.apply(v),
    }
}

// cov:ignore: pure test-code relocation (no logic changed). patch-coverage's git-diff-based line classifier treats every moved
// line as newly added, and cargo-llvm-cov does not record hits for
// multi-line string-literal continuation lines inside assert!/panic!
// messages even though the containing statement executes in a
// passing test. Verified against every flagged line in this move:
// all are string-literal fragments or trivial format-arg
// expressions inside already-passing tests, none of them
// production code.
#[cfg(test)]
mod tests;
