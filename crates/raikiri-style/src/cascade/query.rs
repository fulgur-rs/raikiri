//! Public selector matching entry point for DOM query APIs
//! (`querySelector`, `matches`, `closest`).
//!
//! Reuses the cascade's own complex-selector matcher so query results agree
//! with how the same selector matches during style resolution.

use selectors::parser::SelectorList;

use super::selector_match::match_complex_selector_list;
use crate::{RaikiriSelectorImpl, StyleDom, StyleNode, StyleNodeId, parse_selector_list};

/// A parsed selector list ready to test elements of a [`StyleDom`].
#[derive(Debug, Clone)]
pub struct SelectorQuery {
    list: SelectorList<RaikiriSelectorImpl>,
}

impl SelectorQuery {
    /// Parse a selector list (CSS Selectors L4 `<selector-list>`).
    ///
    /// Returns an error for an empty or syntactically invalid list, which DOM
    /// callers report as a `SyntaxError` `DOMException`.
    pub fn parse(source: &str) -> Result<Self, String> {
        parse_selector_list(source).map(|list| Self { list })
    }

    /// Whether the element `elem_id` matches any selector in the list.
    ///
    /// `ancestors` holds `elem_id`'s ancestor **element** ids only, root
    /// side first and ending with `elem_id`'s parent element. The Document
    /// node (and any other non-element ancestor) is excluded — CSS
    /// Selectors L4 descendant/child combinators are defined over elements
    /// (e.g. <https://www.w3.org/TR/selectors-4/#descendant-combinators>),
    /// so a non-element node is never itself a combinator ancestor — which
    /// means `ancestors` is empty when `elem_id` is the document element.
    /// This is the same shape [`super::collect::collect_cascaded`]'s DFS
    /// builds internally, and what `:root` (`ancestors.is_empty()`) relies
    /// on. Non-element `elem_id`s never match.
    pub fn matches<D: StyleDom>(
        &self,
        dom: &D,
        elem_id: StyleNodeId,
        ancestors: &[StyleNodeId],
    ) -> bool {
        let Some(node) = dom.node(elem_id) else {
            return false;
        };
        let Some(elem) = node.as_element() else {
            return false;
        };
        match_complex_selector_list(
            &self.list,
            dom,
            &elem,
            elem_id,
            ancestors,
            dom.quirks_mode(),
        )
        .is_some()
    }
}

#[cfg(test)]
mod tests;
