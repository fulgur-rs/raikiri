//! Feature-query parsing and evaluation (CSS Conditional Rules 3 §6).

use cssparser::{Delimiter, Parser, ParserInput, Token};
use selectors::parser::{Component, ParseRelative, Selector, SelectorList};

use crate::condition::Condition;
use crate::grammar::{PResult, first_of, keyword, parens, zero_or_more};
use crate::property::parse_value;
use crate::ruletree::is_supported_selector_list;
use crate::{RaikiriSelectorImpl, RaikiriSelectorParser};

type SupportsCondition = Condition<bool>;

// Bound all nested blocks before any grammar or value parser recurses.
const MAX_SUPPORTS_NESTING_DEPTH: usize = 128;

/// Evaluate a complete feature query; invalid syntax never matches.
pub(crate) fn supports_condition(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let start = parser.state();
    if validate_component_values(&mut parser, 0).is_err() {
        return false;
    }
    parser.reset(&start);
    parser
        .parse_entirely(parse_supports_condition)
        .is_ok_and(|condition| condition.eval(&|value| Some(*value)) == Some(true))
}

/// Reject tokenizer errors before forgiving parsers can consume them.
fn validate_component_values<'i>(input: &mut Parser<'i, '_>, depth: usize) -> PResult<'i, ()> {
    while !input.is_exhausted() {
        let token = input.next()?;
        if token.is_parse_error() {
            return Err(input.new_custom_error(()));
        }
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            if depth >= MAX_SUPPORTS_NESTING_DEPTH {
                return Err(input.new_custom_error(()));
            }
            input.parse_nested_block(|input| validate_component_values(input, depth + 1))?;
        }
    }
    Ok(())
}

/// `<supports-condition> = not <supports-in-parens>
///                      | <supports-in-parens> [ and <supports-in-parens> ]*
///                      | <supports-in-parens> [ or <supports-in-parens> ]*`
fn parse_supports_condition<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    first_of(
        input,
        &[parse_supports_not, |input| {
            let head = parse_supports_in_parens(input)?;
            let and = zero_or_more(input, parse_supports_and);
            if !and.is_empty() {
                return Ok(Condition::all(head, and));
            }
            Ok(Condition::any(head, zero_or_more(input, parse_supports_or)))
        }],
    )
}

/// Negation of one parenthesised condition or feature.
fn parse_supports_not<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    keyword(input, "not")?;
    Ok(Condition::Not(Box::new(parse_supports_in_parens(input)?)))
}

/// One conjunction operand, including its operator.
fn parse_supports_and<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    keyword(input, "and")?;
    parse_supports_in_parens(input)
}

/// One disjunction operand, including its operator.
fn parse_supports_or<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    keyword(input, "or")?;
    parse_supports_in_parens(input)
}

/// A parenthesised condition, a feature, or a forward-compatible block.
fn parse_supports_in_parens<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    first_of(
        input,
        &[
            parse_supports_feature,
            |input| parens(input, parse_supports_condition),
            |input| parse_general_enclosed(input),
        ],
    )
}

/// A declaration test or a `selector()` test (CSS Conditional Rules 4 §2).
fn parse_supports_feature<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    first_of(
        input,
        &[
            |input| parens(input, parse_supports_declaration).map(Condition::Leaf),
            |input| parse_supports_selector(input).map(Condition::Leaf),
        ],
    )
}

/// Parse one supported declaration, including an optional importance marker.
fn parse_supports_declaration<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, bool> {
    let name = input.expect_ident_cloned()?;
    input.expect_colon()?;
    input.parse_until_before(Delimiter::Bang, |input| {
        parse_value(&name, input).ok_or_else(|| input.new_custom_error(()))?;
        Ok(())
    })?;
    let _ = input.try_parse(cssparser::parse_important);
    input.expect_exhausted()?;
    Ok(true)
}

/// Test one complex selector, rejecting unsupported forgiving branches too.
fn parse_supports_selector<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, bool> {
    input.expect_function_matching("selector")?;
    input.parse_nested_block(|input| {
        let selectors = SelectorList::parse(&RaikiriSelectorParser, input, ParseRelative::No)
            .map_err(|_| input.new_custom_error(()))?;
        Ok(selectors.slice().len() == 1
            && is_supported_selector_list(&selectors)
            && !selectors.slice().iter().any(has_invalid_selector_component))
    })
}

/// `<general-enclosed>` is false rather than unknown for feature queries.
fn parse_general_enclosed<'i>(input: &mut Parser<'i, '_>) -> PResult<'i, SupportsCondition> {
    let location = input.current_source_location();
    match input.next()? {
        Token::Function(_) | Token::ParenthesisBlock => {}
        _ => return Err(location.new_custom_error(())),
    }
    input.parse_nested_block(|input| {
        input.expect_no_error_token()?;
        Ok(Condition::Leaf(false))
    })
}

fn has_invalid_selector_component(selector: &Selector<RaikiriSelectorImpl>) -> bool {
    selector
        .iter_raw_match_order()
        .any(|component| match component {
            Component::Invalid(_) => true,
            Component::Is(list) | Component::Where(list) | Component::Negation(list) => {
                list.slice().iter().any(has_invalid_selector_component)
            }
            Component::NthOf(data) => data.selectors().iter().any(has_invalid_selector_component),
            Component::Has(relative) => relative
                .iter()
                .any(|relative| has_invalid_selector_component(&relative.selector)),
            _ => false,
        })
}
