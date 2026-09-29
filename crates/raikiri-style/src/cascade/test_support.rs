//! `#[cfg(test)]`-only helpers shared by 3 or more of `cascade`'s submodule
//! test modules. A helper used by only one submodule's tests lives directly
//! in that submodule's own `mod tests` instead — this module exists purely
//! to avoid duplicating a helper body across multiple files.

use crate::computed::ComputedValues;
use crate::property::CssColor;
use crate::ruletree::{Origin, build_rule_tree};
use crate::test_dom::TestDoc;

use super::cascade;

pub(crate) fn cascade_doc(css: &str, tag: &str, inline: Option<&str>) -> ComputedValues {
    let mut doc = TestDoc::new();
    if !css.is_empty() {
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, css);
    }
    let e = doc.push_element(0, tag, inline);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[e].clone()
}

pub(crate) const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};

pub(crate) const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

pub(crate) fn cascade_with_ua(
    ua_css: &str,
    author_css: &str,
    target_tag: &str,
    inline: Option<&str>,
) -> ComputedValues {
    // Build the UA and author rules plus inline styles, then run the cascade.
    let mut doc = TestDoc::new();
    // Have build_rule_tree read the author's <style> from the DOM.
    if !author_css.is_empty() {
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, author_css);
    }
    let e = doc.push_element(0, target_tag, inline);

    // build_rule_tree gathers Author rules; add_stylesheet adds UA rules.
    let mut tree = build_rule_tree(&doc);
    // Rather than inserting UA CSS first, add it after the existing Author rules.
    // Origin rank preserves cascade precedence because rank beats source_order.
    // Currently, add_stylesheet assigns source_order by call order, so Author
    // rules come first (lower source_order) and UA rules last (higher source_order).
    // Ranking always places normal Origin::UserAgent below normal Origin::Author
    // (see `cascade_rank` for the exact values). UA rules therefore cannot
    // override Author rules, regardless of source_order.
    if !ua_css.is_empty() {
        tree.add_stylesheet(ua_css, Origin::UserAgent);
    }
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[e].clone()
}
