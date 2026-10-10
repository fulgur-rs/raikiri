//! Limits on what the parsers of a rule tree count, and the budget they
//! spend.

use cssparser::Parser;

use crate::cascade::Counter;
use crate::error::{CascadeError, CascadeLimitKind};
use crate::rule::{Declaration, parse_page_context_declaration_block};

/// Limits on what the parsers of one rule tree count, summed over every
/// stylesheet added to it.
///
/// Each limit counts something that depends only on the stylesheets, so a
/// stylesheet that passes one fails the same way every time. The parsers
/// count items as they parse them, including some the tree then drops (a
/// rule whose selectors are unsupported, a group whose condition can never
/// hold), so the counts bound the work of parsing as well as what the tree
/// keeps. They stop at the first item past a limit, so a huge stylesheet is
/// never retained in full: the tree keeps the top-level rules before the
/// one that passed the limit, drops that rule whole, adds nothing more, and
/// records the limit, and every element cascade of the tree then fails with
/// that [`CascadeError::LimitExceeded`] (see
/// [`super::RuleTree::limit_exceeded`]). `None` lifts a limit.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleTreeLimits {
    /// The most rules counted: style rules, where each run of declarations
    /// around nested rules counts as one, and custom highlight rules;
    /// at-rules, including group rules nested in other rules and the margin
    /// boxes of `@page` rules; and the layers `@layer` declares, where each
    /// segment of a dotted name declares one, as does an anonymous layer
    /// block. A nested `@layer` counts only its layers.
    ///
    /// Defaults to `Some(2^19)`.
    pub max_rules: Option<u64>,
    /// The most selectors counted: those of style rules, summed over their
    /// selector lists, and the entries of `@page` selector lists. The
    /// cascade indexes every style rule selector and may test it against
    /// every element. A nested rule's selectors each hold the list they nest
    /// in, so each counts as that list's weighted size: one for each of its
    /// simple selectors and combinators, those in the branches of its
    /// logical lists (`:is()`, `:where()`, `:not()`, `:has()`, `of`)
    /// included. A selector list that would pass the limit is not parsed.
    ///
    /// Defaults to `Some(2^20)`.
    pub max_selectors: Option<u64>,
    /// The most declarations counted, after shorthands expand into their
    /// longhands; each `@page`, `@font-face` and `@counter-style` descriptor
    /// counts as one.
    ///
    /// Defaults to `Some(2^21)`.
    pub max_declarations: Option<u64>,
}

impl Default for RuleTreeLimits {
    fn default() -> Self {
        Self {
            max_rules: Some(1 << 19),
            max_selectors: Some(1 << 20),
            max_declarations: Some(1 << 21),
        }
    }
}

/// What the parsers of one rule tree may still spend, and the first limit
/// its stylesheets passed. Once a limit is passed every later charge fails,
/// so nothing more is retained.
pub(crate) struct ParseBudget {
    /// Serialized selector bytes contextual revalidation may still spend.
    pub(crate) selector_revalidation: usize,
    rules: Counter,
    selectors: Counter,
    declarations: Counter,
    /// The first limit passed: its kind, the limit and the count reached.
    exceeded: Option<(CascadeLimitKind, u64, u64)>,
}

impl ParseBudget {
    pub(crate) fn new(limits: &RuleTreeLimits, selector_revalidation: usize) -> Self {
        Self {
            selector_revalidation,
            rules: Counter::new(limits.max_rules, CascadeLimitKind::StyleRules),
            selectors: Counter::new(limits.max_selectors, CascadeLimitKind::StyleSelectors),
            declarations: Counter::new(
                limits.max_declarations,
                CascadeLimitKind::StyleDeclarations,
            ),
            exceeded: None,
        }
    }

    /// The same budget under `limits`, with what was counted so far counted
    /// again against them.
    pub(crate) fn with_limits(&self, limits: &RuleTreeLimits) -> Self {
        let mut budget = Self::new(limits, self.selector_revalidation);
        budget.exceeded = self.exceeded;
        budget.rules_n(self.rules.count());
        budget.selectors_n(self.selectors.count());
        budget.declarations_n(self.declarations.count());
        budget
    }

    /// Whether a limit has been passed.
    pub(crate) fn exceeded(&self) -> bool {
        self.exceeded.is_some()
    }

    /// The error for the first limit passed.
    pub(crate) fn error(&self) -> Option<CascadeError> {
        self.exceeded
            .map(|(kind, limit, actual)| CascadeError::LimitExceeded {
                kind,
                limit,
                actual,
            })
    }

    /// What has been counted: rules, selectors and declarations.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> [u64; 3] {
        [
            self.rules.count(),
            self.selectors.count(),
            self.declarations.count(),
        ]
    }

    /// Accounts for one more style rule whose list holds `selectors`
    /// selectors, or `false` when that passes a limit.
    pub(crate) fn style_rule(&mut self, selectors: usize) -> bool {
        self.rules_n(1) && self.selectors_n(selectors as u64)
    }

    /// Accounts for `count` more selectors outside style rules, or `false`
    /// when that passes the limit.
    pub(crate) fn selectors(&mut self, count: usize) -> bool {
        self.selectors_n(count as u64)
    }

    /// Whether one more rule fits under the limit, counting nothing; when it
    /// does not, the limit is passed as if it had been counted. A style rule
    /// body, which always makes at least one rule, is checked this way
    /// before it is parsed.
    pub(crate) fn rule_fits(&mut self) -> bool {
        if self.exceeded.is_none() && !self.rules.fits(1) {
            self.exceeded = Some(self.rules.exceeding(1));
        }
        self.exceeded.is_none()
    }

    /// Whether `count` more selectors fit under the limit, counting
    /// nothing; when they do not, the limit is passed as if they had been
    /// counted. A selector list is checked this way before it is parsed.
    /// Without a limit any count fits, however large.
    pub(crate) fn selectors_fit(&mut self, count: usize) -> bool {
        let count = count as u64;
        if self.exceeded.is_none() && !self.selectors.fits(count) {
            self.exceeded = Some(self.selectors.exceeding(count));
        }
        self.exceeded.is_none()
    }

    /// Accounts for `count` more rules, or `false` when that passes the
    /// limit.
    pub(crate) fn rules(&mut self, count: usize) -> bool {
        self.rules_n(count as u64)
    }

    /// Accounts for `count` more declarations, or `false` when that passes
    /// the limit.
    pub(crate) fn declarations(&mut self, count: usize) -> bool {
        self.declarations_n(count as u64)
    }

    /// Parses a declaration block in the page context (a margin-box body)
    /// and accounts for its declarations, or `None` when they pass the
    /// limit: parsing stops at the first declaration past it.
    pub(crate) fn page_context_declaration_block(
        &mut self,
        input: &mut Parser<'_, '_>,
    ) -> Option<Vec<Declaration>> {
        let room = usize::try_from(self.declarations.left()).unwrap_or(usize::MAX);
        match parse_page_context_declaration_block(input, room) {
            Ok(block) => self.declarations(block.len()).then_some(block),
            Err(count) => {
                self.declarations(count);
                None
            }
        }
    }

    fn rules_n(&mut self, count: u64) -> bool {
        Self::charge(&mut self.exceeded, &mut self.rules, count)
    }

    fn selectors_n(&mut self, count: u64) -> bool {
        Self::charge(&mut self.exceeded, &mut self.selectors, count)
    }

    fn declarations_n(&mut self, count: u64) -> bool {
        Self::charge(&mut self.exceeded, &mut self.declarations, count)
    }

    fn charge(
        exceeded: &mut Option<(CascadeLimitKind, u64, u64)>,
        counter: &mut Counter,
        count: u64,
    ) -> bool {
        if exceeded.is_some() {
            return false;
        }
        if counter.add(count).is_ok() {
            return true;
        }
        // A failed add counts nothing, so this is the count it stopped at.
        *exceeded = Some(counter.exceeding(count));
        false
    }
}

#[cfg(test)]
mod tests;
