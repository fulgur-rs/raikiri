//! One-sheet native parse memo. It never owns a JavaScript or GC object.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use super::{
    CssRuleInfo, SheetKey, StylesheetVersion, is_page_prelude, margin_rule_body_range,
    nested_at_rule_ranges, parse_css_rules,
};

/// Keeps one source and its metadata until the next sheet/source access,
/// explicit invalidation, or runtime drop. Storage is O(source + rules + margins).
/// Alternating sheets can evict each other; old native text cannot root scripts.
#[derive(Default)]
pub(crate) struct RuleMemo {
    current: Option<(SheetKey, Rc<ParsedSheet>, Option<StylesheetVersion>)>,
}

impl RuleMemo {
    pub(super) fn get(&mut self, sheet: SheetKey, source: Rc<str>) -> Rc<ParsedSheet> {
        if let Some((key, parsed, _)) = &self.current
            && *key == sheet
            && Rc::ptr_eq(&parsed.text, &source)
        {
            return Rc::clone(parsed);
        }
        let parsed = Rc::new(ParsedSheet::new(source));
        self.current = Some((sheet, Rc::clone(&parsed), None));
        parsed
    }

    pub(super) fn native(
        &self,
        sheet: SheetKey,
        version: &StylesheetVersion,
    ) -> Option<Rc<ParsedSheet>> {
        self.current
            .as_ref()
            .filter(|(key, _, verified)| *key == sheet && verified.as_ref() == Some(version))
            .map(|(_, parsed, _)| Rc::clone(parsed))
    }

    pub(super) fn store_native(
        &mut self,
        sheet: SheetKey,
        source: Rc<str>,
        version: StylesheetVersion,
    ) -> Rc<ParsedSheet> {
        let parsed = self.get(sheet, source);
        self.current = Some((sheet, Rc::clone(&parsed), Some(version)));
        parsed
    }

    pub(super) fn clear_for(&mut self, sheet: SheetKey) {
        if self.current.as_ref().is_some_and(|(key, ..)| *key == sheet) {
            self.current = None;
        }
    }
}

pub(super) struct ParsedSheet {
    pub(super) text: Rc<str>,
    pub(super) rules: Vec<CssRuleInfo>,
    pages: Vec<bool>,
    margins: RefCell<HashMap<usize, Rc<[Range<usize>]>>>,
}

impl ParsedSheet {
    fn new(text: Rc<str>) -> Self {
        let rules = parse_css_rules(&text);
        let pages = rules
            .iter()
            .map(|rule| is_page_prelude(&rule.prelude))
            .collect();
        Self {
            text,
            rules,
            pages,
            margins: RefCell::default(),
        }
    }

    pub(super) fn is_page(&self, rule: usize) -> bool {
        self.pages.get(rule).copied().unwrap_or(false)
    }

    pub(super) fn rule_body(&self, rule: usize) -> Option<String> {
        let rule = self.rules.get(rule)?;
        Some(
            rule.body
                .as_ref()
                .map_or_else(String::new, |range| self.text[range.clone()].to_owned()),
        )
    }

    /// Absolute source ranges, always paired with this snapshot's text.
    pub(super) fn margins(&self, rule: usize) -> Rc<[Range<usize>]> {
        if let Some(ranges) = self.margins.borrow().get(&rule) {
            return Rc::clone(ranges);
        }
        let ranges: Rc<[Range<usize>]> = self
            .rules
            .get(rule)
            .filter(|_| self.is_page(rule))
            .and_then(|rule| rule.body.as_ref())
            .map(|body| {
                nested_at_rule_ranges(&self.text[body.clone()])
                    .into_iter()
                    .map(|range| body.start + range.start..body.start + range.end)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into();
        self.margins.borrow_mut().insert(rule, Rc::clone(&ranges));
        ranges
    }

    pub(super) fn margin_body(&self, rule: usize, margin: usize) -> Option<String> {
        let ranges = self.margins(rule);
        let range = ranges.get(margin)?;
        let text = &self.text[range.clone()];
        Some(text[margin_rule_body_range(text)?].to_owned())
    }
}

#[cfg(test)]
mod tests;
