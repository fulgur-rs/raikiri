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
    /// `ancestors` lists the element's ancestors from the root side first,
    /// ending with its parent. Non-element ids never match.
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
