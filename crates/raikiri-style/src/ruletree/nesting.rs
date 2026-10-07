//! Style-rule bodies, including nested style rules and supported groups.

use super::*;
use crate::rule::{expand_shorthand_into, parse_declaration_value};
use cssparser::{DeclarationParser, RuleBodyItemParser, RuleBodyParser};

/// Preserve each declaration run's selectors and position among child rules.
/// Direct declarations keep the parent's pseudo-elements and per-branch
/// specificity, as required by CSS Nesting 1's nested declarations rule.
pub(super) fn parse_style_body(
    input: &mut Parser<'_, '_>,
    selectors: SelectorList<RaikiriSelectorImpl>,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
) -> Vec<GroupItem> {
    parse_body(
        input,
        BodyParent::Style(selectors),
        source,
        depth,
        namespaces,
        supports_context,
    )
}

pub(super) fn parse_highlight_body(
    input: &mut Parser<'_, '_>,
    name: String,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
) -> Vec<GroupItem> {
    parse_body(
        input,
        BodyParent::Highlight(name),
        source,
        depth,
        namespaces,
        supports_context,
    )
}

#[derive(Clone)]
enum BodyParent {
    Style(SelectorList<RaikiriSelectorImpl>),
    Highlight(String),
}

fn parse_body(
    input: &mut Parser<'_, '_>,
    parent: BodyParent,
    source: &str,
    depth: usize,
    namespaces: &NamespaceMap,
    supports_context: &SupportsContext<'_>,
) -> Vec<GroupItem> {
    if depth > MAX_OPAQUE_RULE_NESTING_DEPTH {
        return Vec::new();
    }
    let mut parser = StyleBodyParser {
        parent: &parent,
        source,
        depth,
        namespaces,
        supports_context,
        nesting_parent: None,
        parent_cost: None,
    };
    let mut declarations = Vec::new();
    let mut items = Vec::new();
    for item in RuleBodyParser::new(input, &mut parser).flatten() {
        match item {
            BodyItem::Declaration(declaration) => {
                expand_shorthand_into(&declaration, |d| declarations.push(d));
            }
            BodyItem::Rule(rule) => {
                if !declarations.is_empty() {
                    items.push(body_item(&parent, std::mem::take(&mut declarations)));
                }
                items.push(rule);
            }
        }
    }
    if !declarations.is_empty() || items.is_empty() {
        items.push(body_item(&parent, declarations));
    }
    items
}

fn body_item(parent: &BodyParent, declarations: Vec<Declaration>) -> GroupItem {
    match parent {
        BodyParent::Style(selectors) => GroupItem::Style(StyleRule {
            selectors: selectors.clone(),
            declarations,
            source_order: 0,
            origin: Origin::Author,
            layer: None,
        }),
        BodyParent::Highlight(name) => GroupItem::CustomHighlight {
            name: name.clone(),
            color: custom_highlight_color(&declarations),
        },
    }
}

enum BodyItem {
    Declaration(Declaration),
    Rule(GroupItem),
}

struct StyleBodyParser<'a> {
    parent: &'a BodyParent,
    source: &'a str,
    depth: usize,
    namespaces: &'a NamespaceMap,
    supports_context: &'a SupportsContext<'a>,
    nesting_parent: Option<SelectorList<RaikiriSelectorImpl>>,
    parent_cost: Option<usize>,
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
        if let Some((_, condition)) =
            parse_group_prelude(&name, input, self.source, self.supports_context)?
        {
            return Ok(condition);
        }
        if name.eq_ignore_ascii_case("layer") {
            return parse_layer_names(input).map(GroupCondition::Layer);
        }
        Err(input.new_custom_error(()))
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
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
        let items = parse_body(
            input,
            self.parent.clone(),
            self.source,
            self.depth + 1,
            self.namespaces,
            self.supports_context,
        );
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        Ok(BodyItem::Rule(GroupItem::Group(prelude, items)))
    }
}

impl<'i> cssparser::QualifiedRuleParser<'i> for StyleBodyParser<'_> {
    type Prelude = SelectorList<RaikiriSelectorImpl>;
    type QualifiedRule = BodyItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, ()>> {
        let BodyParent::Style(parent_selectors) = self.parent else {
            // Parent selectors cannot represent the custom highlight pseudo-element.
            return Err(input.new_custom_error(()));
        };
        let start = input.state();
        check_selector_token_depth(input, 0)?;
        input.reset(&start);
        let selectors = SelectorList::parse(
            &NamespacedSelectorParser::for_nesting(self.namespaces),
            input,
            ParseRelative::ForNesting,
        )
        .map_err(|_| input.new_custom_error(()))?;
        // Pseudo-element branches are invalid in the implicit :is() parent:
        // they contribute neither matches nor specificity to a child selector.
        let nesting_parent = self.nesting_parent.get_or_insert_with(|| {
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
        });
        let parent_cost = *self.parent_cost.get_or_insert_with(|| {
            selector_list_cost(nesting_parent, None).unwrap_or(MAX_SELECTOR_WORK + 1)
        });
        if selector_list_cost(&selectors, Some(parent_cost)).is_none() {
            return Err(input.new_custom_error(()));
        }
        Ok(selectors.replace_parent_selector(nesting_parent))
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, ()>> {
        let items = parse_style_body(
            input,
            selectors,
            self.source,
            self.depth + 1,
            self.namespaces,
            self.supports_context,
        );
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        Ok(BodyItem::Rule(GroupItem::Sequence(items)))
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

// Check token depth before invoking the recursive public selector parser.
// The rule-body depth limit cannot bound selector functions in one prelude.
const MAX_SELECTOR_TOKEN_DEPTH: usize = 32;

fn check_selector_token_depth<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            if depth >= MAX_SELECTOR_TOKEN_DEPTH {
                return Err(input.new_custom_error(()));
            }
            input.parse_nested_block(|nested| check_selector_token_depth(nested, depth + 1))?;
        }
    }
    Ok(())
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
