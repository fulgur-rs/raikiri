//! Style-rule bodies, including nested style rules and supported groups.

use std::cell::Cell;

use super::*;
use crate::rule::{ParsedDeclaration, expand_shorthand_into, parse_declaration_value};
use crate::selector_depth::check_selector_token_depth;
use cssparser::{DeclarationParser, RuleBodyItemParser, RuleBodyParser, ToCss};

/// What the rules made from a style rule's selectors count as against the
/// rule tree's limits (see [`RuleTreeLimits::max_selectors`]).
#[derive(Clone, Copy)]
pub(super) struct SelectorCount {
    /// What the rule's prelude counted, which stands for the first rule
    /// made from the selectors.
    prelude: usize,
    /// The selectors' weighted size, when the prelude measured it.
    weight: Option<usize>,
}

impl SelectorCount {
    /// What a top-level rule's prelude counts: one for each selector.
    pub(super) fn top_level(selectors: &SelectorList<RaikiriSelectorImpl>) -> Self {
        Self {
            prelude: selectors.slice().len(),
            weight: None,
        }
    }
}

/// Preserve each declaration run's selectors and position among child rules.
/// Direct declarations keep the parent's pseudo-elements and per-branch
/// specificity, as required by CSS Nesting 1's nested declarations rule.
#[allow(clippy::too_many_arguments)]
pub(super) fn parse_style_body(
    input: &mut Parser<'_, '_>,
    selectors: SelectorList<RaikiriSelectorImpl>,
    count: SelectorCount,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
    budget: &mut ParseBudget,
) -> Vec<GroupItem> {
    parse_body(
        input,
        &BodyParent::new(ParentRule::Style(selectors, count)),
        source,
        depth,
        namespaces,
        supports_context,
        budget,
    )
}

pub(super) fn parse_highlight_body(
    input: &mut Parser<'_, '_>,
    name: String,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
    budget: &mut ParseBudget,
) -> Vec<GroupItem> {
    parse_body(
        input,
        &BodyParent::new(ParentRule::Highlight(name.into())),
        source,
        depth,
        namespaces,
        supports_context,
        budget,
    )
}

/// The rule a body's rules are made from. The groups nested in the body make
/// their rules from it too, and share what is built from it.
struct BodyParent {
    rule: ParentRule,
    /// Whether the count the rule's prelude made still stands for the next
    /// rule made from it.
    prelude_count: Cell<bool>,
    /// The weighted size of the rule's selectors, measured on first use
    /// when the prelude did not measure it.
    weight: OnceCell<usize>,
    /// The parent list placed in the rules nested in the body, built on
    /// first use.
    nesting: OnceCell<NestingParent>,
}

enum ParentRule {
    /// A style rule's selectors, and what the rules made from them count as.
    Style(SelectorList<RaikiriSelectorImpl>, SelectorCount),
    Highlight(Arc<str>),
}

/// The parent list placed in nested rules.
struct NestingParent {
    selectors: SelectorList<RaikiriSelectorImpl>,
    /// Its weighted size (see [`selector_list_cost`]), or more than the work
    /// a selector list may cost.
    cost: usize,
}

impl NestingParent {
    /// What the list counts as at each `&`: its weighted size, and at least
    /// one.
    fn weight(&self) -> usize {
        self.cost.max(1)
    }
}

impl BodyParent {
    fn new(rule: ParentRule) -> Self {
        Self {
            rule,
            prelude_count: Cell::new(true),
            weight: OnceCell::new(),
            nesting: OnceCell::new(),
        }
    }

    /// The selectors the next rule made from this body counts: none for the
    /// first, which the rule's prelude counted. Each later rule holds the
    /// whole list again with little text of its own, so it counts as the
    /// list's weighted size when that is more; a list too large to measure
    /// passes any limit.
    fn next_rule_selectors(&self) -> usize {
        let ParentRule::Style(selectors, count) = &self.rule else {
            return 0;
        };
        if self.prelude_count.replace(false) {
            return 0;
        }
        let weight = self.weight.get_or_init(|| {
            count
                .weight
                .or_else(|| selector_list_cost(selectors, None))
                .unwrap_or(usize::MAX)
        });
        count.prelude.max(*weight)
    }

    /// The parent list placed in the rules nested in this body, built once
    /// for the body and every group nested in it.
    fn nesting(&self) -> &NestingParent {
        self.nesting.get_or_init(|| {
            let selectors = nesting_parent_of(&self.rule);
            let cost = selector_list_cost(&selectors, None).unwrap_or(MAX_SELECTOR_WORK + 1);
            NestingParent { selectors, cost }
        })
    }
}

/// The parent list placed in the rules nested in a body: the body's
/// selectors without their pseudo-element branches, which are invalid in the
/// implicit `:is()` parent and contribute neither matches nor specificity to
/// a child selector.
fn nesting_parent_of(parent: &ParentRule) -> SelectorList<RaikiriSelectorImpl> {
    let ParentRule::Style(parent_selectors, _) = parent else {
        // An unrepresentable highlight parent cannot match, but independent
        // branches of a forgiving child selector remain valid.
        return SelectorList::from_iter(std::iter::empty());
    };
    if parent_selectors
        .slice()
        .iter()
        .all(|s| !s.has_pseudo_element())
    {
        parent_selectors.clone()
    } else {
        let parents: Vec<_> = parent_selectors
            .slice()
            .iter()
            .filter(|s| !s.has_pseudo_element())
            .cloned()
            .collect();
        SelectorList::from_iter(parents.into_iter())
    }
}

/// Parses one body into its rules: a rule for each run of declarations, and
/// the nested rules between them. Every declaration is counted as soon as it
/// is expanded, and every rule before its contents are parsed, so a body past
/// a limit stops there.
fn parse_body(
    input: &mut Parser<'_, '_>,
    parent: &BodyParent,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
    budget: &mut ParseBudget,
) -> Vec<GroupItem> {
    // Every body makes at least one rule, so with no rule left it is not
    // parsed at all.
    if depth > MAX_OPAQUE_RULE_NESTING_DEPTH || !budget.rule_fits() {
        return Vec::new();
    }
    let mut parser = StyleBodyParser {
        parent,
        source,
        depth,
        namespaces,
        supports_context,
        budget,
    };
    let mut declarations = Vec::new();
    // Whether the current run of declarations has been counted as a rule.
    let mut run_counted = false;
    let mut items = Vec::new();
    let mut body = RuleBodyParser::new(input, &mut parser);
    while let Some(item) = body.next() {
        match item {
            Ok(BodyItem::Declaration(declaration)) => {
                // A run is counted when it starts, so that a run past the rule
                // limit is not parsed on.
                if !run_counted {
                    if !body.parser.budget.style_rule(parent.next_rule_selectors()) {
                        return items;
                    }
                    run_counted = true;
                }
                let before = declarations.len();
                expand_shorthand_into(&declaration, |d| declarations.push(d));
                body.parser.budget.declarations(declarations.len() - before);
            }
            Ok(BodyItem::Rule(rule)) => {
                if !declarations.is_empty() {
                    items.push(body_item(parent, std::mem::take(&mut declarations)));
                }
                run_counted = false;
                items.push(rule);
            }
            Err(_) => {}
        }
        if body.parser.budget.exceeded() {
            return items;
        }
    }
    // A body without declarations or nested rules is one empty rule.
    if !declarations.is_empty()
        || (items.is_empty()
            && (run_counted || parser.budget.style_rule(parent.next_rule_selectors())))
    {
        items.push(body_item(parent, declarations));
    }
    items
}

fn body_item(parent: &BodyParent, declarations: Vec<Declaration>) -> GroupItem {
    match &parent.rule {
        ParentRule::Style(selectors, _) => GroupItem::Style(StyleRule {
            selectors: selectors.clone(),
            declarations,
            source_order: 0,
            origin: Origin::Author,
            layer: None,
        }),
        ParentRule::Highlight(name) => GroupItem::CustomHighlight {
            name: name.clone(),
            color: custom_highlight_color(&declarations),
        },
    }
}

enum BodyItem {
    Declaration(ParsedDeclaration),
    Rule(GroupItem),
}

struct StyleBodyParser<'a> {
    parent: &'a BodyParent,
    source: &'a str,
    depth: usize,
    namespaces: &'a NamespaceMap,
    supports_context: &'a SupportsContext<'a>,
    budget: &'a mut ParseBudget,
}

impl<'i> DeclarationParser<'i> for StyleBodyParser<'_> {
    type Declaration = BodyItem;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &cssparser::ParserState,
    ) -> Result<BodyItem, cssparser::ParseError<'i, ()>> {
        parse_declaration_value(name, input, self.supports_context.consumer_properties)
            .map(BodyItem::Declaration)
    }
}

impl<'i> cssparser::AtRuleParser<'i> for StyleBodyParser<'_> {
    type Prelude = GroupCondition;
    type AtRule = BodyItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, ()>> {
        if let Some((_, condition)) = parse_group_prelude(
            &name,
            input,
            self.source,
            self.supports_context,
            self.budget,
        )? {
            return Ok(condition);
        }
        if name.eq_ignore_ascii_case("layer") {
            return parse_layer_names(input, self.budget).map(GroupCondition::Layer);
        }
        Err(input.new_custom_error(()))
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        // The prelude counted the layers a statement declares.
        match prelude {
            GroupCondition::Layer(names) if !names.is_empty() => {
                Ok(BodyItem::Rule(GroupItem::LayerStatement(names)))
            }
            _ => Err(()),
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, ()>> {
        if matches!(&prelude, GroupCondition::Layer(names) if names.len() > 1) {
            return Err(input.new_custom_error(()));
        }
        // A nested group is a rule of its own. A named layer block's layers
        // were counted with its prelude, and an anonymous one declares a
        // layer of its own.
        let rules = match &prelude {
            GroupCondition::Layer(names) => usize::from(names.is_empty()),
            _ => 1,
        };
        if !self.budget.rules(rules) {
            return Err(input.new_custom_error(()));
        }
        let items = parse_body(
            input,
            self.parent,
            self.source,
            self.depth + 1,
            self.namespaces,
            self.supports_context,
            self.budget,
        );
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        Ok(BodyItem::Rule(GroupItem::Group(prelude, items)))
    }
}

impl<'i> cssparser::QualifiedRuleParser<'i> for StyleBodyParser<'_> {
    /// The rule's selectors with the parent placed in them, and what the
    /// rules made from them count as.
    type Prelude = (SelectorList<RaikiriSelectorImpl>, SelectorCount);
    type QualifiedRule = BodyItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, ()>> {
        let start = input.state();
        let count = check_selector_token_depth(input, 0)?;
        // A prelude without a block, such as a declaration cssparser retries
        // as a rule, is not parsed, and the parent is not measured for it.
        // Nor is a rule when no rule is left.
        if !prelude_opens_block(self.source, input) || !self.budget.rule_fits() {
            return Err(input.new_custom_error(()));
        }
        // Nor is a list that would pass the selector limit: each of its
        // selectors holds the parent at least once.
        let body = self.parent;
        let parent = body.nesting();
        let least = count.saturating_mul(parent.weight());
        if !self.budget.selectors_fit(least) {
            return Err(input.new_custom_error(()));
        }
        input.reset(&start);
        let selectors = SelectorList::parse(
            &NamespacedSelectorParser::for_nesting(self.namespaces),
            input,
            ParseRelative::ForNesting,
        )
        .map_err(|_| input.new_custom_error(()))?;
        let Some(weight) = selector_list_cost(&selectors, Some(parent.cost)) else {
            return Err(input.new_custom_error(()));
        };
        // The selectors are counted before the parent is placed in them, so
        // that placing it is counted whatever the rule's body holds.
        let count = SelectorCount {
            prelude: nested_selector_count(&selectors, parent),
            weight: Some(weight),
        };
        if !self.budget.selectors(count.prelude) {
            return Err(input.new_custom_error(()));
        }
        let selectors = selectors.replace_parent_selector(&parent.selectors);
        if selectors.slice().iter().any(|selector| {
            selector_needs_revalidation(selector, false, false, &mut HashMap::new())
        }) {
            // Replacement does not revalidate the parent's new :has()/nth context.
            // Reparse only this bounded selector list through the public parser,
            // which drops contextually invalid forgiving branches and recomputes
            // specificity. Ordinary nesting keeps its shared selector graph.
            let source =
                selector_css_for_revalidation(&selectors, &mut self.budget.selector_revalidation)
                    .map_err(|_| input.new_custom_error(()))?;
            let mut source_input = ParserInput::new(&source);
            let mut parser = Parser::new(&mut source_input);
            check_selector_token_depth(&mut parser, 0).map_err(|_| input.new_custom_error(()))?;
            let source = reject_recursive_nth(&source, &mut self.budget.selector_revalidation)
                .map_err(|_| input.new_custom_error(()))?;
            let mut source_input = ParserInput::new(&source);
            let reparsed = SelectorList::parse(
                &NamespacedSelectorParser::new(self.namespaces),
                &mut Parser::new(&mut source_input),
                ParseRelative::No,
            )
            .map_err(|_| input.new_custom_error(()))?;
            return Ok((reparsed, count));
        }
        Ok((selectors, count))
    }

    fn parse_block<'t>(
        &mut self,
        (selectors, count): Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, ()>> {
        let items = parse_style_body(
            input,
            selectors,
            count,
            self.source,
            self.depth + 1,
            self.namespaces,
            self.supports_context,
            self.budget,
        );
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        Ok(BodyItem::Rule(GroupItem::Sequence(items)))
    }
}

/// What a nested rule's prelude counts its `selectors` as, before `parent`
/// is placed in them. A selector holds the parent at every `&`, or once at
/// the `&` the parser gives a selector without one, and counts as the
/// parent's weight at each. Placing the parent in a logical list that holds
/// `&` measures the parent's selectors again for every selector of the list
/// (of a `:has()` list, for every one that holds `&`), so each of those
/// counts as many as the parent has selectors.
fn nested_selector_count(
    selectors: &SelectorList<RaikiriSelectorImpl>,
    parent: &NestingParent,
) -> usize {
    let mut uses = ParentUses::default();
    for selector in selectors.slice() {
        uses.add_selector(selector);
    }
    uses.references
        .saturating_mul(parent.weight())
        .saturating_add(uses.measured.saturating_mul(parent.selectors.slice().len()))
}

/// How often placing a parent in selectors uses it.
#[derive(Default)]
struct ParentUses {
    /// The `&` it is placed at.
    references: usize,
    /// The selectors of logical lists it is placed in.
    measured: usize,
}

impl ParentUses {
    fn add_selector(&mut self, selector: &Selector<RaikiriSelectorImpl>) {
        use selectors::parser::Component;
        for component in selector.iter_raw_match_order() {
            match component {
                Component::ParentSelector => self.references = self.references.saturating_add(1),
                Component::Is(list) | Component::Where(list) | Component::Negation(list) => {
                    self.add_list(list.slice().iter())
                }
                Component::NthOf(data) => self.add_list(data.selectors().iter()),
                // Only the relative selectors that hold `&` have the parent
                // placed in them; the others are kept as they are.
                Component::Has(list) => {
                    for child in list
                        .iter()
                        .filter(|child| child.selector.has_parent_selector())
                    {
                        self.add_placement(&child.selector);
                    }
                }
                _ => {}
            }
        }
    }

    /// The parent is placed in each selector of a list once one of them
    /// holds `&`.
    fn add_list<'a>(
        &mut self,
        list: impl Iterator<Item = &'a Selector<RaikiriSelectorImpl>> + Clone,
    ) {
        if !list.clone().any(|selector| selector.has_parent_selector()) {
            return;
        }
        for selector in list {
            self.add_placement(selector);
        }
    }

    /// Placing the parent in `selector` measures the parent once, and then
    /// uses it as `selector` does.
    fn add_placement(&mut self, selector: &Selector<RaikiriSelectorImpl>) {
        self.measured = self.measured.saturating_add(1);
        if selector.has_parent_selector() {
            self.add_selector(selector);
        }
    }
}

impl<'i> RuleBodyItemParser<'i, BodyItem, ()> for StyleBodyParser<'_> {
    fn parse_qualified(&self) -> bool {
        true
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

// Rare contextual reparsing must not expand compact parent references into an
// unbounded string. The tree shares this budget across rules and stylesheets.
pub(super) const MAX_SELECTOR_REVALIDATION_BYTES: usize = 64 * 1024 * 1024;

fn selector_css_for_revalidation(
    selectors: &SelectorList<RaikiriSelectorImpl>,
    remaining: &mut usize,
) -> Result<String, std::fmt::Error> {
    let mut output = SelectorOutput {
        text: String::new(),
        remaining,
    };
    selectors.to_css(&mut output)?;
    Ok(output.text)
}

struct SelectorOutput<'a> {
    text: String,
    remaining: &'a mut usize,
}

impl std::fmt::Write for SelectorOutput<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let Some(remaining) = self.remaining.checked_sub(text.len()) else {
            return Err(std::fmt::Error);
        };
        self.text.push_str(text);
        *self.remaining = remaining;
        Ok(())
    }
}

// A parent can introduce contextual components absent when its child was parsed.
fn selector_needs_revalidation(
    selector: &Selector<RaikiriSelectorImpl>,
    inside_has: bool,
    inside_nth: bool,
    memo: &mut HashMap<(selectors::parser::SelectorKey, bool, bool), bool>,
) -> bool {
    use selectors::parser::{Component, SelectorKey};
    let key = (SelectorKey::new(selector), inside_has, inside_nth);
    if let Some(&nested) = memo.get(&key) {
        return nested;
    }
    let nested = selector
        .iter_raw_match_order()
        .any(|component| match component {
            Component::Has(list) => {
                inside_has
                    || list.iter().any(|child| {
                        selector_needs_revalidation(&child.selector, true, inside_nth, memo)
                    })
            }
            Component::Is(list) | Component::Where(list) | Component::Negation(list) => list
                .slice()
                .iter()
                .any(|child| selector_needs_revalidation(child, inside_has, inside_nth, memo)),
            Component::Nth(_) => inside_nth,
            Component::NthOf(data) => {
                inside_nth
                    || data
                        .selectors()
                        .iter()
                        .any(|child| selector_needs_revalidation(child, inside_has, true, memo))
            }
            _ => false,
        });
    memo.insert(key, nested);
    nested
}

// Recursive nth is valid grammar but intentionally unsupported by the matcher.
// Make only those pseudo-classes unknown to the public parser, so its forgiving
// lists discard their branches and its non-forgiving lists still reject them.
fn reject_recursive_nth<'a>(
    source: &'a str,
    remaining: &mut usize,
) -> Result<std::borrow::Cow<'a, str>, ()> {
    fn replacements(
        input: &mut Parser<'_, '_>,
        mut inside_nth: bool,
        nth_arguments: bool,
        edits: &mut Vec<std::ops::Range<usize>>,
    ) -> Result<(), ()> {
        let mut after_colon = false;
        loop {
            let start = input.position().byte_index();
            let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
                break;
            };
            match token {
                Token::Function(name) => {
                    let indexed = after_colon
                        && matches!(
                            name.as_ref(),
                            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type"
                        );
                    if indexed && inside_nth {
                        input
                            .parse_nested_block(|nested| {
                                while nested.next_including_whitespace_and_comments().is_ok() {}
                                Ok::<_, cssparser::ParseError<'_, ()>>(())
                            })
                            .map_err(|_| ())?;
                        edits.push(start..input.position().byte_index());
                    } else {
                        input
                            .parse_nested_block(|nested| {
                                replacements(nested, inside_nth, indexed, edits)
                                    .map_err(|()| nested.new_custom_error::<(), ()>(()))
                            })
                            .map_err(|_| ())?;
                    }
                    after_colon = false;
                }
                Token::Ident(name) => {
                    if nth_arguments && name == "of" {
                        inside_nth = true;
                    }
                    if inside_nth
                        && after_colon
                        && matches!(
                            name.as_ref(),
                            "first-child"
                                | "last-child"
                                | "only-child"
                                | "first-of-type"
                                | "last-of-type"
                                | "only-of-type"
                        )
                    {
                        edits.push(start..input.position().byte_index());
                    }
                    after_colon = false;
                }
                Token::Colon => after_colon = true,
                _ => after_colon = false,
            }
        }
        Ok(())
    }
    let mut input = ParserInput::new(source);
    let mut edits = Vec::new();
    replacements(&mut Parser::new(&mut input), false, false, &mut edits)?;
    if edits.is_empty() {
        return Ok(std::borrow::Cow::Borrowed(source));
    }
    let mut output = SelectorOutput {
        text: String::new(),
        remaining,
    };
    let mut start = 0;
    for edit in edits {
        std::fmt::Write::write_str(&mut output, &source[start..edit.start]).map_err(|_| ())?;
        std::fmt::Write::write_str(&mut output, "raikiri-unsupported-nth").map_err(|_| ())?;
        start = edit.end;
    }
    std::fmt::Write::write_str(&mut output, &source[start..]).map_err(|_| ())?;
    Ok(std::borrow::Cow::Owned(output.text))
}

// Compact parent references avoid string products, but repeated references
// still multiply the work of a recursive matcher. Check the logical component
// count before replacement, so even a compact selector graph stays bounded.
const MAX_SELECTOR_WORK: usize = 32 * 1024;

fn selector_list_cost(
    list: &SelectorList<RaikiriSelectorImpl>,
    parent_cost: Option<usize>,
) -> Option<usize> {
    let mut memo = HashMap::new();
    let mut cost = 0usize;
    for selector in list.slice() {
        let own = selector_cost(selector, parent_cost, 0, &mut memo)?;
        cost = cost.checked_add(own)?;
        if let Some(parent) = parent_cost
            && !selector.has_parent_selector()
        {
            cost = cost.checked_add(parent)?.checked_add(2)?;
        }
        if cost > MAX_SELECTOR_WORK {
            return None;
        }
    }
    Some(cost)
}

fn selector_cost(
    selector: &Selector<RaikiriSelectorImpl>,
    parent_cost: Option<usize>,
    depth: usize,
    memo: &mut HashMap<selectors::parser::SelectorKey, usize>,
) -> Option<usize> {
    use selectors::parser::{Component, SelectorKey};
    if depth > MAX_OPAQUE_RULE_NESTING_DEPTH {
        return None;
    }
    let key = SelectorKey::new(selector);
    if let Some(&cost) = memo.get(&key) {
        return Some(cost);
    }
    let mut cost = 0usize;
    for component in selector.iter_raw_match_order() {
        cost = cost.checked_add(1)?;
        match component {
            Component::ParentSelector => {
                cost = cost.checked_add(parent_cost.unwrap_or(0))?.checked_add(1)?;
            }
            Component::Is(list) | Component::Where(list) | Component::Negation(list) => {
                for child in list.slice() {
                    cost = cost.checked_add(selector_cost(child, parent_cost, depth + 1, memo)?)?;
                }
            }
            Component::NthOf(data) => {
                for child in data.selectors() {
                    cost = cost.checked_add(selector_cost(child, parent_cost, depth + 1, memo)?)?;
                }
            }
            Component::Has(list) => {
                for child in list.iter() {
                    cost = cost.checked_add(selector_cost(
                        &child.selector,
                        parent_cost,
                        depth + 1,
                        memo,
                    )?)?;
                }
            }
            _ => {}
        }
        if cost > MAX_SELECTOR_WORK {
            return None;
        }
    }
    memo.insert(key, cost);
    Some(cost)
}

#[cfg(test)]
mod tests;
