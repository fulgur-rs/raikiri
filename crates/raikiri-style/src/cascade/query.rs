//! Public selector matching entry point for DOM query APIs
//! (`querySelector`, `matches`, `closest`).
//!
//! Reuses the cascade's own complex-selector matcher so query results agree
//! with how the same selector matches during style resolution.

use std::cell::RefCell;
use std::rc::Rc;

use super::selector_match::SelectorWork;
use crate::CascadeError;

use selectors::parser::{Component, Selector, SelectorList};

use super::selector_match::{MatchCaches, MatchContext, match_complex_selector_list};
use crate::{RaikiriSelectorImpl, StyleDom, StyleNode, StyleNodeId, parse_selector_list};

/// A parsed selector list ready to test elements of a [`StyleDom`].
///
/// The compatibility bool methods are unlimited. Use a bounded matcher
/// and a shared work budget for fallible cumulative work limits.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct SelectorQuery {
    list: SelectorList<RaikiriSelectorImpl>,
}

/// Cumulative logical selector work, independent of any DOM or query cache.
///
/// Share one budget across candidates, queries and immutable DOM borrows.
/// `None` disables the limit while retaining counts. A failed charge keeps
/// its first error permanently: unused remaining work cannot be spent after
/// failure, and rejected bulk work is never reported as executed.
/// The units match cascade selector work. Callers may additionally charge
/// their outer traversal before inspecting or enqueueing nodes. String bytes,
/// selector parsing and arbitrary DOM implementations are outside this bound.
pub struct SelectorWorkBudget {
    work: Rc<SelectorWork>,
    failed: RefCell<Option<CascadeError>>,
}

impl SelectorWorkBudget {
    /// Starts a budget. Its lifetime, rather than a matcher or DOM lifetime,
    /// determines when cumulative accounting starts again.
    pub fn new(limit: Option<u64>) -> Self {
        Self {
            work: Rc::new(SelectorWork::new(limit)),
            failed: RefCell::new(None),
        }
    }

    /// Logical units accepted so far, saturating at the integer maximum.
    /// Rejected bulk charges add nothing.
    pub fn count(&self) -> u64 {
        self.work.count()
    }

    /// Charges outer traversal work before its inspection or allocation.
    /// Returns the first terminal work error after any failed charge.
    pub fn charge(&self, units: u64) -> Result<(), CascadeError> {
        self.check()?;
        self.remember(self.work.charge(units))
    }

    fn check(&self) -> Result<(), CascadeError> {
        match self.failed.borrow().as_ref() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn remember<T>(&self, result: Result<T, CascadeError>) -> Result<T, CascadeError> {
        result.map_err(|error| self.failed.borrow_mut().get_or_insert(error).clone())
    }
}

/// Fallible matcher with per-query immutable DOM caches and a shared budget.
///
/// It has no bool matching method. Every call checks terminal failure before
/// reading the DOM, including after an internal bulk reservation failed.
pub struct BoundedSelectorMatcher<'a, D: StyleDom> {
    matcher: SelectorMatcher<'a, D>,
    budget: &'a SelectorWorkBudget,
}

impl<D: StyleDom> BoundedSelectorMatcher<'_, D> {
    /// Matches a subject using its real element ancestor chain. The subject
    /// visit costs one unit before its node is read; inner matching uses the
    /// same cumulative budget. Failure never becomes a successful no-match.
    pub fn try_matches(
        &self,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
    ) -> Result<bool, CascadeError> {
        self.budget.charge(1)?;
        self.budget
            .remember(self.matcher.try_matches(elem_id, ancestors))
    }
}

impl SelectorQuery {
    /// Parse a selector list (CSS Selectors L4 `<selector-list>`).
    ///
    /// Returns an error for an empty or syntactically invalid list, which DOM
    /// callers report as a `SyntaxError` `DOMException`.
    /// Token blocks are limited to 32 nested levels before recursive parsing.
    /// Also rejects syntactically valid nth selectors nested inside an
    /// `:nth-child(An+B of S)` or `:nth-last-child(An+B of S)` filter.
    /// This intentional bounded-support restriction prevents recursively
    /// amplified sibling scans, as in stylesheet registration.
    /// Decision: `raikiri-spike-58do6`.
    pub fn parse(source: &str) -> Result<Self, String> {
        let list = parse_selector_list(source)?;
        if !list
            .slice()
            .iter()
            .all(|selector| has_bounded_nth_filters(selector, true))
        {
            return Err("nested nth selectors inside an nth-of filter are unsupported".into());
        }
        Ok(Self { list })
    }

    /// Whether the element `elem_id` matches any selector in the list, with
    /// no `:scope` element bound (`:scope` falls back to `:root`
    /// semantics — see [`Self::matches_scoped`]).
    ///
    /// `ancestors` holds `elem_id`'s ancestor **element** ids only, root
    /// side first and ending with `elem_id`'s parent element. The Document
    /// node (and any other non-element ancestor) is excluded — CSS
    /// Selectors L4 descendant/child combinators are defined over elements
    /// (e.g. <https://www.w3.org/TR/selectors-4/#descendant-combinators>),
    /// so a non-element node is never itself a combinator ancestor — which
    /// means `ancestors` is empty when `elem_id` is the document element.
    /// This is the same shape [`super::inherit::walk_from`]'s DFS
    /// builds internally, and what `:root` (`ancestors.is_empty()`) relies
    /// on. Non-element `elem_id`s never match.
    pub fn matches<D: StyleDom>(
        &self,
        dom: &D,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
    ) -> bool {
        self.matches_scoped(dom, elem_id, ancestors, None)
    }

    /// [`Self::matches`], additionally binding CSS Selectors L4 `:scope`
    /// (§14.3.3 <https://www.w3.org/TR/selectors-4/#the-scope-pseudo>) to
    /// `scope`: a `:scope` component in the selector list matches iff
    /// `elem_id == scope`. `scope: None` is [`Self::matches`]'s own
    /// behavior — `:scope` then falls back to `:root` semantics
    /// (`ancestors.is_empty()`), the same fallback the `selectors` crate's
    /// own (unused-by-this-matcher) reference matcher applies when it has
    /// no bound scope element.
    ///
    /// DOM query callers bind `scope` to the scoping root of a
    /// element-scoped query (`Element.querySelector`/`querySelectorAll`/
    /// `matches`/`closest`, DOM §4.2.6, §4.9); a document- or
    /// fragment-scoped query has no such element, so it calls this the
    /// same as [`Self::matches`] (`scope: None`).
    /// This single-candidate entry point scans filtered sibling positions
    /// directly; [`Self::matcher`] retains ranks for repeated candidates.
    pub fn matches_scoped<D: StyleDom>(
        &self,
        dom: &D,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
        scope: Option<StyleNodeId>,
    ) -> bool {
        self.matcher_with_caches(dom, scope, MatchCaches::for_single_match())
            .matches(elem_id, ancestors)
    }

    /// A matcher that tests many elements of `dom` against this selector
    /// list with `:scope` bound to `scope` (see [`Self::matches_scoped`]).
    ///
    /// Use it for a whole tree traversal such as `querySelectorAll`: it
    /// keeps the DOM facts the matcher derives (sibling positions for
    /// `:nth-child()` and `+`, effective language, directionality) across
    /// every element it tests, where calling [`Self::matches_scoped`] per
    /// element would rebuild them each time. The matcher borrows `dom` for
    /// its whole lifetime, so the DOM cannot change while it is in use.
    pub fn matcher<'a, D: StyleDom>(
        &'a self,
        dom: &'a D,
        scope: Option<StyleNodeId>,
    ) -> SelectorMatcher<'a, D> {
        self.matcher_with_caches(dom, scope, MatchCaches::default())
    }

    /// A fallible matcher borrowing one immutable DOM, with a budget shared
    /// across all subjects and later queries. Caches end with this matcher;
    /// consuming or dropping it never refunds the budget.
    pub fn bounded_matcher<'a, D: StyleDom>(
        &'a self,
        dom: &'a D,
        scope: Option<StyleNodeId>,
        budget: &'a SelectorWorkBudget,
    ) -> BoundedSelectorMatcher<'a, D> {
        self.bounded_with_caches(dom, scope, budget, false)
    }

    /// Fallible one-subject matching. The caller supplies a cumulative budget;
    /// it is not reset by this call. Single-subject filtered nth matching scans
    /// directly instead of building a reusable rank cache.
    pub fn try_matches_scoped<D: StyleDom>(
        &self,
        dom: &D,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
        scope: Option<StyleNodeId>,
        budget: &SelectorWorkBudget,
    ) -> Result<bool, CascadeError> {
        self.bounded_with_caches(dom, scope, budget, true)
            .try_matches(elem_id, ancestors)
    }

    fn bounded_with_caches<'a, D: StyleDom>(
        &'a self,
        dom: &'a D,
        scope: Option<StyleNodeId>,
        budget: &'a SelectorWorkBudget,
        single: bool,
    ) -> BoundedSelectorMatcher<'a, D> {
        BoundedSelectorMatcher {
            matcher: self.matcher_with_caches(
                dom,
                scope,
                MatchCaches::with_query_work(Rc::clone(&budget.work), single),
            ),
            budget,
        }
    }

    fn matcher_with_caches<'a, D: StyleDom>(
        &'a self,
        dom: &'a D,
        scope: Option<StyleNodeId>,
        caches: MatchCaches,
    ) -> SelectorMatcher<'a, D> {
        SelectorMatcher {
            query: self,
            dom,
            scope,
            caches,
        }
    }
}

/// A [`SelectorQuery`] bound to one DOM and one `:scope` element, created by
/// [`SelectorQuery::matcher`].
#[doc(hidden)]
pub struct SelectorMatcher<'a, D: StyleDom> {
    query: &'a SelectorQuery,
    dom: &'a D,
    scope: Option<StyleNodeId>,
    /// Shared by every [`Self::matches`] call. Sound because `dom` stays
    /// borrowed, and therefore unchanged, for the matcher's lifetime.
    caches: MatchCaches,
}

impl<D: StyleDom> SelectorMatcher<'_, D> {
    /// Whether `elem_id` matches any selector in the list. `ancestors`
    /// follows the same contract as [`SelectorQuery::matches`]: `elem_id`'s
    /// ancestor element ids, root side first, ending with its parent.
    pub fn matches(&self, elem_id: StyleNodeId, ancestors: &[StyleNodeId]) -> bool {
        self.try_matches(elem_id, ancestors)
            .expect("query matching has no selector work limit")
    }

    fn try_matches(
        &self,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
    ) -> Result<bool, CascadeError> {
        let Some(node) = self.dom.node(elem_id) else {
            return Ok(false);
        };
        let Some(elem) = node.as_element() else {
            return Ok(false);
        };
        // DOM query entry points allow detached candidates (same-tree gating);
        // the stylesheet cascade passes `false` to keep inert/template-contents
        // filtering (see `is_candidate_element`).
        let ctx = MatchContext::new(
            self.dom,
            self.dom.quirks_mode(),
            self.scope,
            true,
            &self.caches,
        );
        match_complex_selector_list(&self.query.list, ctx, &elem, elem_id, ancestors)
            .map(|specificity| specificity.is_some())
    }

    #[cfg(test)]
    pub(crate) fn cached_sibling_parents(&self) -> usize {
        self.caches.cached_sibling_parents()
    }
}

// Follow the rule-tree's nested-nth restriction without its unrelated
// stylesheet support restrictions: DOM queries also support `:scope` and
// accept state pseudo-classes that simply do not match.
fn has_bounded_nth_filters(selector: &Selector<RaikiriSelectorImpl>, allow_nth: bool) -> bool {
    selector
        .iter_raw_match_order()
        .all(|component| match component {
            Component::Nth(_) => allow_nth,
            Component::NthOf(data) => {
                allow_nth
                    && data
                        .selectors()
                        .iter()
                        .all(|selector| has_bounded_nth_filters(selector, false))
            }
            Component::Negation(list) | Component::Is(list) | Component::Where(list) => list
                .slice()
                .iter()
                .all(|selector| has_bounded_nth_filters(selector, allow_nth)),
            Component::Has(list) => list
                .iter()
                .all(|relative| has_bounded_nth_filters(&relative.selector, allow_nth)),
            _ => true,
        })
}

#[cfg(test)]
mod tests;
