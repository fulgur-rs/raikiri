//! A reusable seed for independent trees of unconditional style rules.

use super::{CssRule, OpaqueBudget, Origin, ParseBudget, RuleTree, StyleRule};

/// Parsed, unconditional style rules that can seed a fresh [`RuleTree`].
///
/// The seed has the default parsing limits and no consumer-owned properties.
/// It owns no document state. Each [`Self::to_rule_tree`] call produces an
/// independent tree, preserving the parser's actual budget consumption,
/// including work spent on rules it discarded.
#[doc(hidden)]
pub struct PreparedStylesheet {
    style_rules: Vec<StyleRule>,
    rules: Vec<CssRule>,
    next_style_order: u32,
    budget: ParseBudget,
    opaque_budget: OpaqueBudget,
}

impl PreparedStylesheet {
    /// Parse `source` under the default limits without consumer properties.
    ///
    /// Returns `None` when a limit is exceeded or the result contains state
    /// beyond plain style rules, such as layers, media or other at-rules,
    /// descriptors, or highlights. The caller can then use normal parsing.
    /// The caller owns the seed and decides its lifetime; this constructor
    /// does not install a process-wide stylesheet cache.
    pub fn new(source: &str, origin: Origin) -> Option<Self> {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, origin);
        if tree.budget.exceeded()
            || !tree.page_rules.is_empty()
            || !tree.media_rules.is_empty()
            || !tree.opaque_at_rules.is_empty()
            || !tree.layers.is_empty()
            || !tree.highlight_log.is_empty()
            || !tree.counter_style_log.is_empty()
            || !tree.font_face_log.is_empty()
            || !tree.counter_styles.is_empty()
            || !tree.font_faces.is_empty()
            || tree.style_rules.iter().any(|rule| rule.layer.is_some())
        {
            return None;
        }
        Some(Self {
            style_rules: tree.style_rules,
            rules: tree.rules,
            next_style_order: tree.next_style_order,
            budget: tree.budget,
            opaque_budget: tree.opaque_budget,
        })
    }

    /// Create an independent tree, ready for subsequent stylesheets.
    ///
    /// The tree starts with default limits and no consumer properties.
    /// Applying tighter limits afterwards has the same semantics as
    /// [`RuleTree::with_limits`] after parsing, not parsing within those
    /// limits: use the normal parser when limits must apply from the start.
    pub fn to_rule_tree(&self) -> RuleTree {
        RuleTree {
            style_rules: self
                .style_rules
                .iter()
                .map(|rule| StyleRule {
                    selectors: rule.selectors.clone(),
                    declarations: rule.declarations.clone(),
                    source_order: rule.source_order,
                    origin: rule.origin,
                    layer: rule.layer,
                })
                .collect(),
            rules: self.rules.clone(),
            next_style_order: self.next_style_order,
            budget: self.budget.clone(),
            opaque_budget: self.opaque_budget.clone(),
            ..RuleTree::empty()
        }
    }
}

#[cfg(test)]
mod tests;
