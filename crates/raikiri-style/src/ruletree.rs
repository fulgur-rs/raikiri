//! Unified rule tree: an index shared by the cascade and future GCPM resolution.
//! Populates style_rules. Also stores @page at-rules in
//! [`RuleTree::page_rules`] instead of silently skipping them (cascade support is not implemented).
//! The generic at-rule parse view stores rules without dedicated semantics
//! (@supports / @import, etc.) as raw data in [`RuleTree::opaque_at_rules`].
//! It also retains `@media` in this raw view while expanding qualified rules
//! for supported media types into a dedicated cascade view. `@counter-style`,
//! which also has a dedicated view, is recorded here for raw/source-order inspection;
//! `@page` remains in the existing page view.

use cssparser::{Parser, ParserInput, SourceLocation, StyleSheetParser, Token};
use selectors::parser::{ParseRelative, Parser as SelectorParser, Selector, SelectorList};
use std::collections::HashMap;

use crate::consumer::ConsumerPropertyRegistration;
use crate::counter_style::{CounterStyleRegistry, CounterStyleRule, parse_counter_style_rules};
use crate::font_face::{FontFaceRegistry, FontFaceRule, parse_font_face_rules};
use crate::media::{MediaCondition, MediaContext, MediaRule, parse_media_prelude};
use crate::page::{
    PageBlockBody, PageRule, PageSelector, parse_page_declaration_block, parse_page_prelude,
};
use crate::property::{CssColor, PropertyValue, parse_value};
use crate::rule::{Declaration, StyleRule, parse_declaration_block_with_consumer_properties};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use crate::{Atom, PseudoClass, PseudoElem, RaikiriSelectorImpl, RaikiriSelectorParser};

/// Flatten the subset of cascade layers that the rule parser can evaluate.
///
/// The generic rule parser stores unsupported at-rules as opaque records, which
/// would otherwise hide `@page` and ordinary style rules nested in `@layer`.
/// Flattening here keeps the existing parser and rule-tree representation while
/// ordering normal layers from lower to higher precedence.  Unlayered rules are
/// appended last, as required by the normal cascade.  This is intentionally a
/// small source-level pass; nested blocks, strings, and comments are skipped
/// while locating only top-level layer blocks.
/// Remove supported `@supports` blocks before the generic stylesheet parser.
///
/// The rule tree intentionally retains unknown at-rules as opaque records, but a
/// supported condition must expose its qualified rules to the normal cascade.
/// This small source pass handles declaration conditions and `not`/`and`/`or`
/// combinations while preserving unsupported blocks verbatim for inspection.
fn expand_supports(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut plain_start = 0;
    let mut cursor = 0;
    while let Some(start) = find_top_level_at_rule(source, cursor, "supports") {
        let Some((delimiter, delimiter_index)) = find_layer_delimiter(source, start + 9) else {
            break;
        };
        if delimiter != b'{' {
            break;
        }
        let Some(close) = matching_brace(source, delimiter_index) else {
            break;
        };
        output.push_str(&source[plain_start..start]);
        let condition = &source[start + 9..delimiter_index];
        let body = &source[delimiter_index + 1..close];
        if supports_condition(condition) && !body.trim_start().starts_with('@') {
            // Keep the opaque record for inspection, and prepend its qualified
            // rules as ordinary stylesheet input for the cascade.
            output.push_str(&expand_supports(body));
            output.push_str(&source[start..=close]);
        } else {
            output.push_str(&source[start..=close]);
        }
        cursor = close + 1;
        plain_start = cursor;
    }
    output.push_str(&source[plain_start..]);
    output
}

/// Expand qualified CSS nesting into the flat selector rules understood by the
/// rule tree. The parser intentionally keeps this pass source-based: it handles
/// nested qualified rules, preserves at-rules, and leaves declaration values,
/// strings, comments, and function arguments opaque to the brace scanner.
fn expand_css_nesting(source: &str) -> Result<String, ()> {
    expand_css_nesting_at_depth(source, &mut NestingBudget::new(), 0)
}

// One stylesheet shares these limits across siblings, wrappers, and every
// recursive expansion. Charge scans and copies before allocating their output.
const MAX_NESTING_WORK_BYTES: usize = 64 * 1024 * 1024;
const MAX_NESTING_ITEMS: usize = 4096;
const MAX_NESTING_DEPTH: usize = 512;

struct NestingBudget {
    bytes: usize,
    items: usize,
}

impl NestingBudget {
    fn new() -> Self {
        Self {
            bytes: MAX_NESTING_WORK_BYTES,
            items: MAX_NESTING_ITEMS,
        }
    }

    fn consume_bytes(&mut self, bytes: usize) -> Result<(), ()> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(())?;
        Ok(())
    }

    fn consume_items(&mut self, items: usize) -> Result<(), ()> {
        self.items = self.items.checked_sub(items).ok_or(())?;
        Ok(())
    }

    fn append(&mut self, output: &mut String, value: &str) -> Result<(), ()> {
        self.consume_bytes(value.len())?;
        output.push_str(value);
        Ok(())
    }
}

fn expand_css_nesting_at_depth(
    source: &str,
    budget: &mut NestingBudget,
    depth: usize,
) -> Result<String, ()> {
    if depth > MAX_NESTING_DEPTH {
        return Err(());
    }
    budget.consume_bytes(source.len())?;
    let mut output = String::new();
    let mut cursor = 0;
    while let Some((kind, index)) = next_css_top_level_construct(source, cursor) {
        match kind {
            b';' => {
                budget.append(&mut output, &source[cursor..=index])?;
                cursor = index + 1;
            }
            b'{' => {
                let Some(close) = matching_brace(source, index) else {
                    budget.append(&mut output, &source[cursor..])?;
                    return Ok(output);
                };
                let prelude = &source[cursor..index];
                let body = &source[index + 1..close];
                if prelude.trim_start().starts_with('@') {
                    budget.append(&mut output, prelude)?;
                    budget.append(&mut output, "{")?;
                    let expanded = expand_css_nesting_at_depth(body, budget, depth + 1)?;
                    budget.append(&mut output, &expanded)?;
                    budget.append(&mut output, "}")?;
                } else {
                    let expanded = flatten_css_style_rule(prelude, body, budget, depth)?;
                    budget.append(&mut output, &expanded)?;
                }
                cursor = close + 1;
            }
            _ => unreachable!("next_css_top_level_construct only returns ; or {{"), // cov:ignore: helper returns only semicolon or block-opener tags.
        }
    }
    budget.append(&mut output, &source[cursor..])?;
    Ok(output)
}

/// Return the next top-level declaration terminator or block opener.
fn next_css_top_level_construct(source: &str, from: usize) -> Option<(u8, usize)> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut paren_depth = 0_u32;
    let mut bracket_depth = 0_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'(' => paren_depth = paren_depth.saturating_add(1),
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth = bracket_depth.saturating_add(1),
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b';' | b'{' if paren_depth == 0 && bracket_depth == 0 => {
                return Some((byte, index));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn flatten_css_style_rule(
    prelude: &str,
    body: &str,
    budget: &mut NestingBudget,
    depth: usize,
) -> Result<String, ()> {
    if depth > MAX_NESTING_DEPTH {
        return Err(());
    }
    budget.consume_bytes(body.len())?;
    let unchanged = |budget: &mut NestingBudget| {
        let mut unchanged = String::new();
        budget.append(&mut unchanged, prelude)?;
        budget.append(&mut unchanged, "{")?;
        budget.append(&mut unchanged, body)?;
        budget.append(&mut unchanged, "}")?;
        Ok(unchanged)
    };
    let Some(segments) = split_css_nested_body(body, budget)? else {
        return unchanged(budget);
    };
    let selector = prelude.trim();
    // cov:ignore: callers only pass qualified-rule preludes; at-rules are routed elsewhere.
    if selector.is_empty() || selector.starts_with('@') {
        return unchanged(budget); // cov:ignore: see above.
    }

    let leading_len = prelude.len() - prelude.trim_start().len();
    let mut output = String::new();
    budget.append(&mut output, &prelude[..leading_len])?;
    for segment in segments {
        match segment {
            // Declarations keep their source position relative to nested
            // rules: CSS Nesting wraps declarations that follow a nested rule
            // in a nested declarations rule matching the parent's elements
            // with the parent's specificity, i.e. a repeat of the parent rule.
            NestedSegment::Declarations(declarations) => {
                if declarations.trim().is_empty() {
                    continue;
                }
                budget.append(&mut output, selector)?;
                budget.append(&mut output, "{")?;
                budget.append(&mut output, &declarations)?;
                budget.append(&mut output, "}\n")?;
            }
            NestedSegment::Rule(nested_selector, nested_body) => {
                let combined = combine_nested_selectors(selector, &nested_selector, budget)?;
                let expanded = flatten_css_style_rule(&combined, &nested_body, budget, depth + 1)?;
                budget.append(&mut output, &expanded)?;
                budget.append(&mut output, "\n")?;
            }
        }
    }
    Ok(output)
}

/// A rule body split in source order.
enum NestedSegment {
    /// A run of declarations between nested rules.
    Declarations(String),
    /// A nested qualified rule: its selector and its body.
    Rule(String, String),
}

/// Split a rule body into declaration runs and nested qualified-rule blocks,
/// in source order. Returns `None` when the body has no nested rules or
/// contains a nested construct outside this pass's qualified-rule subset.
fn split_css_nested_body(
    body: &str,
    budget: &mut NestingBudget,
) -> Result<Option<Vec<NestedSegment>>, ()> {
    let mut segments = Vec::new();
    let mut has_nested_rule = false;
    let mut segment_start = 0;
    let mut cursor = 0;
    while let Some((kind, index)) = next_css_top_level_construct(body, cursor) {
        match kind {
            b';' => cursor = index + 1,
            b'{' => {
                let Some(close) = matching_brace(body, index) else {
                    return Ok(None);
                };
                let nested_selector = body[cursor..index].trim();
                // cov:ignore: at-rules and empty nested preludes remain opaque to this qualified-rule pass.
                if nested_selector.is_empty() || nested_selector.starts_with('@') {
                    return Ok(None);
                }
                budget.consume_items(2)?;
                segments.push(NestedSegment::Declarations(
                    body[segment_start..cursor].to_owned(),
                ));
                segments.push(NestedSegment::Rule(
                    nested_selector.to_owned(),
                    body[index + 1..close].to_owned(),
                ));
                has_nested_rule = true;
                cursor = close + 1;
                segment_start = cursor;
            }
            _ => unreachable!("next_css_top_level_construct only returns ; or {{"), // cov:ignore: helper returns only semicolon or block-opener tags.
        }
    }
    if !has_nested_rule {
        return Ok(None);
    }
    budget.consume_items(1)?;
    segments.push(NestedSegment::Declarations(
        body[segment_start..].to_owned(),
    ));
    Ok(Some(segments))
}

/// Resolve a nested selector list against its parent selector list.
///
/// CSS Nesting gives `&` the meaning of `:is(<parent list>)`, which the
/// selector matcher does not support. Each `&` is therefore expanded
/// independently over the parent list, so `.a, .b { & + & {} }` covers all
/// four pairings. Matching is equivalent; specificity differs from `:is()`
/// only when the parent list mixes selectors of different specificity. A
/// nested selector without `&` is relative to the parent as a descendant.
fn combine_nested_selectors(
    parent: &str,
    nested: &str,
    budget: &mut NestingBudget,
) -> Result<String, ()> {
    let parents = split_top_level_selector_list(parent, budget)?;
    let mut combined = Vec::new();
    for nested in split_top_level_selector_list(nested, budget)? {
        let pieces = split_on_nesting_selector(&nested, budget)?;
        if pieces.len() == 1 {
            budget.consume_items(parents.len())?;
            for parent in &parents {
                budget.consume_bytes(
                    parent
                        .len()
                        .checked_add(nested.len())
                        .and_then(|len| len.checked_add(1))
                        .ok_or(())?,
                )?;
                combined.push(format!("{parent} {nested}"));
            }
            continue;
        }
        budget.consume_items(1)?;
        budget.consume_bytes(pieces[0].len())?;
        let mut expansions = vec![pieces[0].to_owned()];
        for piece in &pieces[1..] {
            let count = expansions.len().checked_mul(parents.len()).ok_or(())?;
            budget.consume_items(count)?;
            let mut next = Vec::new();
            for prefix in &expansions {
                for parent in &parents {
                    let length = prefix
                        .len()
                        .checked_add(parent.len())
                        .and_then(|len| len.checked_add(piece.len()))
                        .ok_or(())?;
                    budget.consume_bytes(length)?;
                    next.push(format!("{prefix}{parent}{piece}"));
                }
            }
            expansions = next;
        }
        combined.extend(expansions);
    }
    let separators = combined.len().saturating_sub(1).checked_mul(2).ok_or(())?;
    let length = combined
        .iter()
        .try_fold(separators, |len, value| len.checked_add(value.len()))
        .ok_or(())?;
    budget.consume_bytes(length)?;
    Ok(combined.join(", "))
}

/// Split `selector` around each nesting selector `&`, skipping `&` inside
/// quoted strings and escaped `\&`.
fn split_on_nesting_selector<'a>(
    selector: &'a str,
    budget: &mut NestingBudget,
) -> Result<Vec<&'a str>, ()> {
    budget.consume_bytes(selector.len())?;
    let bytes = selector.as_bytes();
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' {
            index = index.saturating_add(2);
            continue;
        }
        match quote {
            Some(delimiter) if byte == delimiter => quote = None,
            Some(_) => {}
            None if byte == b'\'' || byte == b'"' => quote = Some(byte),
            None if byte == b'&' => {
                budget.consume_items(1)?;
                pieces.push(&selector[start..index]);
                start = index + 1;
            }
            None => {}
        }
        index += 1;
    }
    budget.consume_items(1)?;
    pieces.push(&selector[start.min(selector.len())..]);
    Ok(pieces)
}

fn split_top_level_selector_list(
    value: &str,
    budget: &mut NestingBudget,
) -> Result<Vec<String>, ()> {
    budget.consume_bytes(value.len())?;
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut paren_depth = 0_u32;
    let mut bracket_depth = 0_u32;
    let bytes = value.as_bytes();
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else {
            match byte {
                b'(' => paren_depth = paren_depth.saturating_add(1),
                b')' => paren_depth = paren_depth.saturating_sub(1),
                b'[' => bracket_depth = bracket_depth.saturating_add(1),
                b']' => bracket_depth = bracket_depth.saturating_sub(1),
                b',' if paren_depth == 0 && bracket_depth == 0 => {
                    let item = value[start..index].trim();
                    if !item.is_empty() {
                        budget.consume_items(1)?;
                        budget.consume_bytes(item.len())?;
                        result.push(item.to_owned());
                    }
                    start = index + 1;
                }
                _ => {}
            }
        }
        index += 1;
    }
    let item = value[start..].trim();
    if !item.is_empty() {
        budget.consume_items(1)?;
        budget.consume_bytes(item.len())?;
        result.push(item.to_owned());
    }
    Ok(result)
}

fn supports_condition(raw: &str) -> bool {
    let condition = raw.trim();
    if let Some(rest) = condition.strip_prefix("not ") {
        return !supports_condition(rest);
    }
    if let Some((left, right)) = split_supports_operator(condition, " or ") {
        return supports_condition(left) || supports_condition(right);
    }
    if let Some((left, right)) = split_supports_operator(condition, " and ") {
        return supports_condition(left) && supports_condition(right);
    }
    let condition = condition
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
        .unwrap_or(condition);
    let Some((name, value)) = condition.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return false;
    }
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|input| {
            parse_value(name, input).ok_or_else(|| input.new_custom_error::<_, ()>(()))
        })
        .is_ok()
}

fn split_supports_operator<'a>(value: &'a str, operator: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0_u32;
    let mut index = 0;
    while index + operator.len() <= value.len() {
        match value.as_bytes()[index] {
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && value.as_bytes()[index..].starts_with(operator.as_bytes()) {
            return Some((&value[..index], &value[index + operator.len()..]));
        }
        index += 1;
    }
    None
}

fn find_top_level_at_rule(source: &str, from: usize, name: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => depth = depth.saturating_sub(1),
            b'@' if depth == 0
                && bytes[index + 1..].len() >= name.len()
                && bytes[index + 1..]
                    .get(..name.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes()))
                && bytes
                    .get(index + 1 + name.len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'-') =>
            {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn expand_cascade_layers(source: &str) -> Vec<(u32, String)> {
    let mut cursor = 0;
    let mut plain_start = 0;
    let mut unlayered = String::with_capacity(source.len());
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut declared_order = Vec::new();
    let mut found_layer = false;

    while let Some(start) = find_top_level_layer(source, cursor) {
        unlayered.push_str(&source[plain_start..start]);
        let prelude_start = start + "@layer".len();
        let Some((delimiter, delimiter_index)) = find_layer_delimiter(source, prelude_start) else {
            unlayered.push_str(&source[start..]);
            break;
        };
        let prelude = source[prelude_start..delimiter_index].trim();
        match delimiter {
            b';' => {
                for name in prelude
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    if !declared_order.iter().any(|existing| existing == name) {
                        declared_order.push(name.to_owned());
                    }
                }
                cursor = delimiter_index + 1;
                plain_start = cursor;
                found_layer = true;
            }
            b'{' => {
                let Some(close) = matching_brace(source, delimiter_index) else {
                    unlayered.push_str(&source[start..]);
                    break;
                };
                if !prelude.is_empty() {
                    blocks.push((
                        prelude.to_owned(),
                        source[delimiter_index + 1..close].to_owned(),
                    ));
                    found_layer = true;
                    cursor = close + 1;
                    plain_start = cursor;
                } else {
                    unlayered.push_str(&source[start..=close]);
                    cursor = close + 1;
                    plain_start = cursor;
                }
            }
            _ => {
                unlayered.push_str(&source[start..]);
                break;
            }
        }
    }

    if !found_layer {
        return vec![(u32::MAX, source.to_owned())];
    }
    unlayered.push_str(&source[plain_start..]);

    let mut order = declared_order;
    for (name, _) in &blocks {
        if !order.iter().any(|existing| existing == name) {
            order.push(name.clone());
        }
    }
    let mut chunks = Vec::with_capacity(order.len() + 1);
    for (layer_order, name) in order.into_iter().enumerate() {
        let mut body = String::new();
        for (block_name, block_body) in &blocks {
            if block_name == &name {
                body.push_str(block_body);
                body.push('\n');
            }
        }
        chunks.push((layer_order as u32, body));
    }
    chunks.push((u32::MAX, unlayered));
    chunks
}

fn find_top_level_layer(source: &str, from: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => depth = depth.saturating_sub(1),
            b'@' if depth == 0
                && bytes
                    .get(index..index + "@layer".len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(b"@layer"))
                && source
                    .as_bytes()
                    .get(index + "@layer".len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'-') =>
            {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn find_layer_delimiter(source: &str, from: usize) -> Option<(u8, usize)> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'{' || byte == b';' {
            return Some((byte, index));
        }
        index += 1;
    }
    None
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = open + 1;
    let mut depth = 1_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'{' {
            depth = depth.saturating_add(1);
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

/// Cascade origin (CSS Cascading L4 §6.2).
///
/// [`Origin::AuthorPresentationalHint`] corresponds to CSS Cascading L5 §6.5 "Precedence
/// of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>), which defines the "author
/// presentational hint origin" as an independent origin between user and author.
/// See the [`crate::cascade::cascade_rank`] docs for its position
/// among the origin ranks.
///
/// [`Origin::User`] corresponds to CSS Cascading L4 §6.2 at
/// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>, which defines the "user
/// origin". Within raikiri-style the variant itself is fully functional:
/// it participates in the origin and importance ordering of
/// [`crate::cascade::cascade_rank`].
/// Consumer-supplied `extra_stylesheets` are routed here through three crates:
/// the `StylesheetKind::User` variant in raikiri-traits, retagging in
/// raikiri-html, and extension of `stylesheet_kind_to_origin` in the umbrella.
/// Thus `extra_stylesheets` actually reach [`Origin::User`].
///
/// The raikiri umbrella crate maps `StylesheetKind` (the DOM-level tag)
/// as part of cascade orchestration.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    UserAgent,
    /// CSS Cascading L4 §6.2 "user origin"
    /// (<https://www.w3.org/TR/css-cascade-4/#cascading-origins>). Consumer-supplied
    /// CSS through `ParseOptions::extra_stylesheets` is routed here
    /// (via the `StylesheetKind::User` retagging in raikiri-html).
    User,
    /// CSS Cascading L5 §6.5 "author presentational hint origin"
    /// (<https://drafts.csswg.org/css-cascade-5/#preshint>) — HTML
    /// presentational hints (e.g., `<img width>`/`<img height>`),
    /// between the user and author origins.
    AuthorPresentationalHint,
    Author,
    /// Sampled Web Animations declarations, above every normal origin and
    /// below every important origin (CSS Cascading L5 §6.1).
    Animation,
}

/// A syntactically valid at-rule body retained for a consumer that does not
/// yet implement the at-rule's semantics.
///
/// The contents exclude the outer braces. `Statement` represents an at-rule
/// terminated by `;` (or by the end of the stylesheet, which CSS syntax also
/// permits). `Block` retains the raw component-value text inside `{ ... }`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtRuleBody {
    /// The at-rule had no block body.
    Statement,
    /// The raw contents of the at-rule's curly-bracket block.
    Block(String),
}

impl AtRuleBody {
    /// Return the block contents, or `None` for a statement at-rule.
    pub fn as_block(&self) -> Option<&str> {
        match self {
            Self::Statement => None,
            Self::Block(body) => Some(body),
        }
    }
}

/// A raw qualified rule found inside an opaque at-rule block.
///
/// The prelude and body are intentionally not interpreted. A later semantic
/// pass can parse them according to that at-rule's grammar, while an
/// inspector can still walk the nested rule list without reparsing bytes.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualifiedRuleRecord {
    /// Raw component-value text before the nested rule's `{`.
    pub prelude: String,
    /// Raw component-value text inside the nested rule's braces.
    pub body: String,
    /// Zero-based order among sibling nested rules.
    pub source_order: u32,
    /// The stylesheet origin inherited from the containing at-rule.
    pub origin: Origin,
}

/// A rule node retained inside an opaque at-rule block.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleNode {
    /// A qualified rule retained without selector/property interpretation.
    Qualified(QualifiedRuleRecord),
    /// A nested at-rule retained recursively.
    AtRule(Box<AtRuleRecord>),
}

/// A valid at-rule that is retained as opaque data.
///
/// This is deliberately an owned representation. The parser input belongs to
/// the caller of [`RuleTree::add_stylesheet`], so retaining `&str` slices here
/// would make the rule tree borrow the stylesheet source. `prelude` includes
/// the source text between the at-rule name and its terminator; comments and
/// whitespace are retained in that text. For a block at-rule, `children` is a
/// best-effort recursive view of nested rules; `body` remains authoritative
/// for declaration lists and arbitrary component-value content.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtRuleRecord {
    /// The at-rule name without the leading `@`.
    pub name: String,
    /// The raw prelude, including source whitespace/comments after `name`.
    pub prelude: String,
    /// The statement or raw block body.
    pub body: AtRuleBody,
    /// Nested rule nodes in source order, if this is a block at-rule.
    pub children: Vec<RuleNode>,
    /// Zero-based cross-kind source order for this top-level at-rule.
    /// Nested records use sibling-local order instead.
    pub source_order: u32,
    /// The stylesheet origin supplied to [`RuleTree::add_stylesheet`].
    pub origin: Origin,
}

/// The kind of a retained top-level rule in [`RuleTree::rules`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CssRuleKind {
    /// Index into [`RuleTree::style_rules`].
    Style { index: usize },
    /// Index into [`RuleTree::page_rules`].
    Page { index: usize },
    /// Index into [`RuleTree::opaque_at_rules`].
    AtRule { index: usize },
}

/// A source-order entry for a retained top-level stylesheet rule.
///
/// The compatibility views keep their historical independent source-order
/// counters. This ordered view supplies the cross-kind order needed when a
/// later semantic pass expands an at-rule into ordinary style rules.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssRule {
    /// Zero-based order across all retained top-level style, page, and opaque
    /// at-rules in the tree.
    pub source_order: u32,
    /// Cascade origin for this rule.
    pub origin: Origin,
    /// Which compatibility view contains the rule's parsed payload.
    pub kind: CssRuleKind,
}

impl AtRuleRecord {
    /// Return a best-effort CSS serialization of this retained at-rule.
    ///
    /// The name and component-value text are retained, while the outer syntax
    /// is reconstructed. This is intended for inspection and forwarding, not
    /// as a byte-for-byte source-map representation.
    pub fn to_css(&self) -> String {
        match &self.body {
            AtRuleBody::Statement => format!("@{}{};", self.name, self.prelude),
            AtRuleBody::Block(body) => format!("@{}{}{{{body}}}", self.name, self.prelude),
        }
    }

    /// Return nested rules in source order.
    pub fn children(&self) -> &[RuleNode] {
        &self.children
    }
}

/// Unified rule tree: an index consumed by the cascade and future GCPM resolution.
///
/// Populates `style_rules`. Adds @page at-rules to `page_rules` (parsed
/// but not yet applied in the cascade). `counter_styles`
/// ([`CounterStyleRegistry`]) registers `@counter-style` at-rules and provides
/// a snapshot to the cascade for downstream marker/generated-content painting.
/// `opaque_at_rules` retains at-rule raw data in a generic parse
/// view. Qualified rules under `@media` conditions `all` / `print` /
/// `screen` are expanded into an internal view in addition to the compatibility
/// `style_rules` view; they are evaluated before the cascade. Future fields
/// (font_face_rules / supports_rules / import_rules) can be added later without
/// breaking consumers thanks to `#[non_exhaustive]`.
#[non_exhaustive]
pub struct RuleTree {
    /// Qualified style rules (`selectors { declarations }`), kept in source order.
    pub(crate) style_rules: Vec<StyleRule>,
    /// Named custom-highlight background colors, in stylesheet source order.
    custom_highlight_styles: HashMap<String, CssColor>,
    /// `@page` at-rules. `source_order` starts at zero independently of `style_rules`.
    /// [`crate::page::cascade_page`] applies the cascade; per-page `PageBox`
    /// derivation and margin-box slot layout are not implemented.
    ///
    /// Unlike `style_rules`, page declarations are exposed for page-context
    /// consumers.
    pub page_rules: Vec<PageRule>,
    /// Registry mapping `@counter-style` names to rules.
    /// Same-name rules follow CSS Counter Styles Level 3's standard cascade
    /// order and are stored as complete rule values.
    pub(crate) counter_styles: CounterStyleRegistry,
    /// Registry mapping `@font-face` family names to rules.
    ///
    /// Each call to [`RuleTree::add_stylesheet`], regardless of origin, separately
    /// runs [`crate::font_face::parse_font_face_rules`] over the same text again,
    /// then passes each resulting [`crate::font_face::FontFaceRule`]
    /// with the call's `origin` to
    /// [`FontFaceRegistry::insert_with_origin`]. The registry itself tracks origins
    /// and resolves same-name rule precedence (see the resolution table in the
    /// [`FontFaceRegistry`] type docs): it directly implements standard cascade
    /// precedence (origin first, then source order within an origin) by comparing
    /// ranks based on [`crate::cascade::cascade_rank`].
    /// The caller (`add_stylesheet`) need not filter by origin.
    /// The registry performs that resolution directly.
    ///
    /// This intentionally uses a separate pass from the `style_rules` parser (see
    /// "RuleTree integration" in the [`crate::font_face`] module docs; the same design as [`crate::counter_style`]).
    pub(crate) font_faces: FontFaceRegistry,
    /// Every `@counter-style` and `@font-face` registration in insertion order,
    /// including those from stylesheets with a media condition, so the
    /// registries can be rebuilt for one media context.
    counter_style_log: Vec<Registration<CounterStyleRule>>,
    font_face_log: Vec<Registration<FontFaceRule>>,
    /// Generic records for at-rules that do not use the `@page` compatibility
    /// view.
    ///
    /// This includes valid statement and block at-rules such as `@media`,
    /// `@supports`, `@import`, and `@counter-style`. The records are
    /// intentionally separate from `style_rules` and `page_rules`: retaining
    /// an at-rule must not make its declarations execute accidentally.
    pub(crate) opaque_at_rules: Vec<AtRuleRecord>,
    /// Executable qualified rules parsed from `@media`, kept apart from the
    /// historical direct-rule compatibility view.
    pub(crate) media_rules: Vec<MediaRule>,
    /// Next source order shared by direct and media-qualified style rules.
    next_style_order: u32,
    /// Cross-kind source-order index. The individual compatibility views keep
    /// their historical counters; this vector records their retained order.
    pub(crate) rules: Vec<CssRule>,
    /// Remaining budget for overlapping bodies retained by nested opaque rules.
    opaque_body_budget: usize,
    /// Consumer-owned CSS property registrations used while parsing declarations.
    /// Empty in the compatibility/default path.
    consumer_properties: Vec<ConsumerPropertyRegistration>,
}

impl RuleTree {
    /// Read-only accessor for qualified style rules (in source order).
    ///
    /// For **qualified style rules**, [`RuleTree::add_stylesheet`] is now the only
    /// write path. `page_rules` remains `pub`; see that field's docs.
    ///
    ///
    /// # Inaccessibility of the `style_rules` field itself
    ///
    /// The `style_rules` field is `pub(crate)`: external crates can reach only
    /// this accessor. `RuleTree` is `#[non_exhaustive]` and does not derive
    /// `Clone`, so external crates cannot construct a struct literal or use
    /// functional update, nor can they clone an owned value. The following
    /// compile-fail check verifies that the field name itself is private:
    /// making it `pub` would make the example compile.
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.style_rules;
    /// ```
    pub fn style_rules(&self) -> &[StyleRule] {
        &self.style_rules
    }

    /// Return the winning background color for each parsed `::highlight(name)` rule.
    pub fn custom_highlight_styles(&self) -> &HashMap<String, CssColor> {
        &self.custom_highlight_styles
    }

    /// Read-only accessor for the `@counter-style` registry.
    ///
    /// Each call to [`RuleTree::add_stylesheet`] populates this regardless of origin.
    /// The `CounterStyleRegistry` itself resolves same-name rules by origin
    /// (see the field docs). An empty `RuleTree` ([`RuleTree::empty`]) has
    /// [`CounterStyleRegistry::is_empty`] equal to `true`. `raikiri-paint` handles
    /// generating counters (resolving `counter()`/`counters()`) from the cascade
    /// result. Wiring this registry into
    /// [`crate::counter_style::resolve_custom_counter`] for markers/generated content
    /// is a consumer-side boundary.
    ///
    /// # Inaccessibility of the `counter_styles` field itself
    ///
    /// As with `style_rules`/[`RuleTree::style_rules`], this is a compile-fail
    /// check: the `counter_styles` field is `pub(crate)`, and external crates
    /// can reach only this accessor. The following checks the privacy of the field
    /// name itself: making it `pub` would make the example compile.
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.counter_styles;
    /// ```
    pub fn counter_styles(&self) -> &CounterStyleRegistry {
        &self.counter_styles
    }

    /// Read-only accessor for the `@font-face` registry.
    ///
    /// Each call to [`RuleTree::add_stylesheet`] populates this regardless of origin.
    /// The `FontFaceRegistry` itself resolves same-name rules by origin
    /// (see the field docs). An empty `RuleTree` ([`RuleTree::empty`]) has
    /// [`FontFaceRegistry::is_empty`] equal to `true`. Fetching `src: url(...)` and
    /// matching faces are the responsibility of consumers reading this registry.
    ///
    /// # Inaccessibility of the `font_faces` field itself
    ///
    /// As with `style_rules`/[`RuleTree::style_rules`], this is a compile-fail
    /// check: the `font_faces` field is `pub(crate)`, and external crates
    /// can reach only this accessor. The following checks the privacy of the field
    /// name itself: making it `pub` would make the example compile.
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.font_faces;
    /// ```
    pub fn font_faces(&self) -> &FontFaceRegistry {
        &self.font_faces
    }

    /// The `@font-face` registry for one media context.
    ///
    /// Unlike [`RuleTree::font_faces`], which holds only rules from
    /// stylesheets without a media condition, this also includes rules from
    /// stylesheets added through [`RuleTree::add_stylesheet_with_media`] whose
    /// media query list matches `context`.
    pub fn font_faces_for(&self, context: &MediaContext) -> FontFaceRegistry {
        replay(
            &self.font_face_log,
            &self.font_faces,
            context,
            FontFaceRegistry::new,
            |registry, rule, origin| registry.insert_with_origin(rule, origin),
        )
    }

    /// The `@counter-style` registry for one media context; see
    /// [`RuleTree::font_faces_for`].
    pub fn counter_styles_for(&self, context: &MediaContext) -> CounterStyleRegistry {
        replay(
            &self.counter_style_log,
            &self.counter_styles,
            context,
            CounterStyleRegistry::new,
            |registry, rule, origin| registry.insert_with_origin(rule, origin),
        )
    }

    /// Generic retained records for at-rules outside the `@page`
    /// compatibility view.
    ///
    /// The returned records retain their cross-kind source order in the
    /// corresponding [`CssRule`] entries. The raw records are observational;
    /// supported `@media` descendants are separately evaluated by the cascade
    /// and are not inserted into this compatibility view.
    pub fn opaque_at_rules(&self) -> &[AtRuleRecord] {
        &self.opaque_at_rules
    }

    /// Alias for [`RuleTree::opaque_at_rules`] for callers that use the CSSOM
    /// term "at-rules" for the retained opaque records.
    pub fn at_rules(&self) -> &[AtRuleRecord] {
        self.opaque_at_rules()
    }

    /// Retained top-level rules in one cross-kind source-order sequence.
    ///
    /// Use the `index` in [`CssRuleKind`] to access the parsed payload through
    /// [`RuleTree::style_rules`], [`RuleTree::page_rules`], or
    /// [`RuleTree::opaque_at_rules`]. Unsupported selectors are absent from
    /// this sequence because they do not produce a retained rule.
    pub fn rules(&self) -> &[CssRule] {
        &self.rules
    }

    /// An empty RuleTree (zero rules).
    pub fn empty() -> Self {
        Self {
            style_rules: Vec::new(),
            custom_highlight_styles: HashMap::new(),
            page_rules: Vec::new(),
            counter_styles: CounterStyleRegistry::new(),
            font_faces: FontFaceRegistry::new(),
            counter_style_log: Vec::new(),
            font_face_log: Vec::new(),
            opaque_at_rules: Vec::new(),
            media_rules: Vec::new(),
            next_style_order: 0,
            rules: Vec::new(),
            opaque_body_budget: MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES,
            consumer_properties: Vec::new(),
        }
    }

    /// Empty rule tree configured to retain the supplied consumer-owned
    /// property names while parsing qualified rules and inline declarations.
    pub fn empty_with_consumer_properties(
        consumer_properties: &[ConsumerPropertyRegistration],
    ) -> Self {
        let mut tree = Self::empty();
        tree.consumer_properties = consumer_properties.to_vec();
        tree
    }

    /// Registrations used by this tree's declaration parser.
    pub fn consumer_property_registrations(&self) -> &[ConsumerPropertyRegistration] {
        &self.consumer_properties
    }

    /// Parse a stylesheet string and append its rules.
    ///
    /// - `source_order` starts from the existing rule count and increments by call order
    ///   (`style_rules` and `page_rules` use separate counters; see the docs for
    ///   [`PageRule::source_order`]).
    /// - `origin` propagates to both style and `@page` rules. Origin-based cascade
    ///   ranking (with reversed ordering for `!important`) is not implemented yet.
    ///   Because `PageRule` retains the origin, the cascade will not need to re-index
    ///   rules when the ranking is wired up. CSS Cascading L4 §"cascade-origin"
    ///   (<https://www.w3.org/TR/css-cascade-4/#cascade-origin>)
    /// - Invalid selectors and unsupported properties inherit the existing
    ///   silent-drop behavior.
    /// - Syntactically valid at-rules other than `@page` are retained in
    ///   [`RuleTree::opaque_at_rules`]. Declarations in unsupported at-rules
    ///   are not applied by the cascade. Supported `@media` descendants are
    ///   additionally parsed into the cascade's media-qualified view.
    /// - For every `@counter-style` at-rule, regardless of `origin`,
    ///   [`crate::counter_style::parse_counter_style_rules`] independently parses
    ///   the same `source` a second time, then passes each resulting rule and
    ///   the call's `origin` to [`CounterStyleRegistry::insert_with_origin`].
    ///
    ///   CSS Counter Styles L3 §3 says same-name `@counter-style` winners are chosen
    ///   "according to standard cascade rules" (origin first; UA always loses to
    ///   other origins). [`CounterStyleRegistry`] itself tracks per-origin entries
    ///   and resolves precedence (see its type docs' resolution table). Thus the
    ///   `add_stylesheet` caller need not filter by origin; it merely propagates it.
    ///   As a result, a standalone `Origin::UserAgent` `@counter-style` with no
    ///   same-name collision from other origins appears in `counter_styles`, whereas
    ///   the former Author-only gate used to drop it unconditionally.
    ///
    /// CSS nesting preprocessing is bounded by cumulative work bytes, generated
    /// items, and recursion depth. A stylesheet that exceeds a preprocessing
    /// limit is discarded before any of its rules are added to this tree.
    ///
    pub fn add_stylesheet(&mut self, source: &str, origin: Origin) {
        self.add_conditional_stylesheet(source, origin, None);
    }

    /// Parse a stylesheet that applies only where its media query list matches,
    /// such as the `media` attribute of `<link rel=stylesheet>` or `<style>`.
    ///
    /// `None`, an empty string, or whitespace means the stylesheet always
    /// applies (HTML Standard: an omitted or empty `media` attribute is `all`).
    /// Otherwise every rule in the stylesheet, including `@page`,
    /// `@font-face`, and `@counter-style`, is guarded by the list, as if the
    /// whole stylesheet were nested in `@media`. A list that matches in no
    /// context adds nothing.
    ///
    /// Style rules from a guarded stylesheet live in the media-guarded view,
    /// like rules nested in `@media`, so they are absent from
    /// [`RuleTree::style_rules`] and [`RuleTree::rules`]. Use
    /// [`RuleTree::font_faces_for`] and [`RuleTree::counter_styles_for`] to
    /// read the registries for one media context.
    pub fn add_stylesheet_with_media(&mut self, source: &str, origin: Origin, media: Option<&str>) {
        let condition = match media.map(str::trim) {
            None | Some("") => None,
            Some(media) => match parse_media_prelude(media) {
                Some(condition) => Some(condition),
                None => return,
            },
        };
        self.add_conditional_stylesheet(source, origin, condition.as_ref());
    }

    fn add_conditional_stylesheet(
        &mut self,
        source: &str,
        origin: Origin,
        condition: Option<&MediaCondition>,
    ) {
        let Ok(source) = expand_css_nesting(&expand_supports(source)) else {
            return;
        };
        for (layer_order, chunk) in expand_cascade_layers(&source) {
            self.add_stylesheet_chunk(&chunk, origin, layer_order, condition);
        }
    }

    fn add_stylesheet_chunk(
        &mut self,
        source: &str,
        origin: Origin,
        layer_order: u32,
        condition: Option<&MediaCondition>,
    ) {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut namespaces = NamespaceMap::new();
        let mut rule_parser = StyleRuleParser {
            source,
            opaque_body_budget: &mut self.opaque_body_budget,
            namespaces: &mut namespaces,
            consumer_properties: &self.consumer_properties,
        };
        let mut style_order = self.next_style_order;
        let mut page_order = self.page_rules.len() as u32;
        let mut rule_order = self.rules.len() as u32;
        if let Some(prelude) = leading_charset_prelude(source) {
            let index = self.opaque_at_rules.len();
            self.opaque_at_rules.push(AtRuleRecord {
                name: "charset".to_owned(),
                prelude,
                body: AtRuleBody::Statement,
                children: Vec::new(),
                source_order: rule_order,
                origin,
            });
            self.rules.push(CssRule {
                source_order: rule_order,
                origin,
                kind: CssRuleKind::AtRule { index },
            });
            rule_order = rule_order.wrapping_add(1);
        }
        for rule in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
            match rule {
                ParsedRule::Style(selectors, declarations) => {
                    // Skip selectors containing unsupported components (pseudo-classes, etc.;
                    // see the `is_supported_selector_list` docs).
                    // Descendant/child and next-sibling/general-sibling combinators
                    // are accepted (see `is_supported_selector_list` for details).
                    // as specified in those docs.
                    if !is_supported_selector_list(&selectors) {
                        continue;
                    }
                    let rule = StyleRule {
                        selectors,
                        declarations,
                        source_order: style_order,
                        origin,
                    };
                    style_order = style_order.wrapping_add(1);
                    if let Some(condition) = condition {
                        self.media_rules.push(MediaRule {
                            rule,
                            condition: condition.clone(),
                        });
                        continue;
                    }
                    let index = self.style_rules.len();
                    self.style_rules.push(rule);
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::Style { index },
                    });
                    rule_order = rule_order.wrapping_add(1);
                }
                ParsedRule::CustomHighlight { name, color } => {
                    if let Some(color) = color {
                        self.custom_highlight_styles.insert(name, color);
                    }
                }
                ParsedRule::Page(selector, body) => {
                    let PageBlockBody {
                        declarations,
                        size_declarations,
                        marks_declarations,
                        bleed_declarations,
                        margin_box_rules,
                    } = body;
                    let index = self.page_rules.len();
                    self.page_rules.push(PageRule {
                        selector,
                        declarations,
                        size_declarations,
                        marks_declarations,
                        bleed_declarations,
                        margin_box_rules,
                        source_order: page_order,
                        layer_order,
                        origin,
                        media_condition: condition.cloned(),
                    });
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::Page { index },
                    });
                    page_order = page_order.wrapping_add(1);
                    rule_order = rule_order.wrapping_add(1);
                }
                ParsedRule::OpaqueAtRule(mut record) => {
                    record.source_order = rule_order;
                    set_at_rule_origin(&mut record, origin);
                    if record.name.eq_ignore_ascii_case("media") {
                        collect_media_style_rules(
                            &record,
                            condition,
                            &mut self.media_rules,
                            &mut style_order,
                            &mut self.page_rules,
                            &mut page_order,
                            layer_order,
                            &self.consumer_properties,
                        );
                    }
                    let index = self.opaque_at_rules.len();
                    self.opaque_at_rules.push(record);
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::AtRule { index },
                    });
                    rule_order = rule_order.wrapping_add(1);
                }
            }
        }
        for rule in parse_counter_style_rules(source) {
            if condition.is_none() {
                self.counter_styles.insert_with_origin(rule.clone(), origin);
            }
            self.counter_style_log
                .push(Registration::new(rule, origin, condition));
        }
        for rule in parse_font_face_rules(source) {
            if condition.is_none() {
                self.font_faces.insert_with_origin(rule.clone(), origin);
            }
            self.font_face_log
                .push(Registration::new(rule, origin, condition));
        }
        self.next_style_order = style_order;
    }
}

/// One registry insertion, kept so a registry can be rebuilt for a media
/// context.
struct Registration<R> {
    rule: R,
    origin: Origin,
    condition: Option<MediaCondition>,
}

impl<R> Registration<R> {
    fn new(rule: R, origin: Origin, condition: Option<&MediaCondition>) -> Self {
        Self {
            rule,
            origin,
            condition: condition.cloned(),
        }
    }
}

/// Rebuild a registry from `log`, keeping the registrations whose condition
/// matches `context`. Replaying in insertion order preserves the registry's
/// same-name precedence. When no registration is conditional, the
/// unconditional registry is already the answer.
fn replay<R: Clone, Registry: Clone>(
    log: &[Registration<R>],
    unconditional: &Registry,
    context: &MediaContext,
    new: fn() -> Registry,
    insert: fn(&mut Registry, R, Origin),
) -> Registry {
    if log
        .iter()
        .all(|registration| registration.condition.is_none())
    {
        return unconditional.clone();
    }
    let mut registry = new();
    for registration in log {
        if registration
            .condition
            .as_ref()
            .is_none_or(|condition| condition.matches(context))
        {
            insert(
                &mut registry,
                registration.rule.clone(),
                registration.origin,
            );
        }
    }
    registry
}

/// Walk the DOM via DFS and gather text from all `<style>` elements as Author
/// stylesheets. Excludes UA CSS: `raikiri-html::parse` already injects it via
/// Document.add_stylesheet, and the umbrella passes `Document.stylesheets()`
/// to RuleTree.
pub fn build_rule_tree<D: StyleDom>(dom: &D) -> RuleTree {
    let mut tree = RuleTree::empty();
    walk_and_collect(dom, dom.root_id(), &mut |source| {
        tree.add_stylesheet(source, Origin::Author);
    });
    tree
}

/// Walk the DOM via DFS from the root and pass each `<style>` element's text to a callback.
///
/// Excludes UA CSS: `raikiri-html::parse` injects it via `Document::add_stylesheet`,
/// and the umbrella consumes it separately through `Document::stylesheets()`.
/// This walker gathers text only from DOM `<style>` elements.
///
/// Call order follows document order under `walk_and_collect`'s iterative DFS.
/// Shares `walk_and_collect`'s protection against stack overflow.
pub fn walk_style_elements<D: StyleDom, F: FnMut(&str)>(dom: &D, mut on_style_text: F) {
    walk_and_collect(dom, dom.root_id(), &mut on_style_text);
}

/// DOM walk implementation. Iterative DFS with an explicit `Vec` stack avoids
/// stack overflow on deep nesting. Children are reverse-pushed and then
/// popped LIFO (see the stack push below), preserving the same sibling visitation
/// order as naive recursion. Document order matters for correctness, not only
/// compatibility: [`RuleTree::add_stylesheet`] monotonically numbers `source_order`
/// in call order, and `build_rule_tree` calls it from this walk's callback.
fn walk_and_collect<D: StyleDom, F: FnMut(&str)>(dom: &D, id: StyleNodeId, on_style_text: &mut F) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // Skip `<template>` descendants and future inert subtrees consistently.
            // In a real Document (populated through the sink), the `is_in_document()` bit
            // is the primary skip path.
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let tag = elem.tag_name();
                // NOTE: The normal path (a Document populated through the sink)
                // is already handled by the `is_in_document()` gate above. This arm
                // is a safety net for calls from Node implementations such as TestDoc,
                // whose default `is_in_document()` returns true: it prevents `<style>` inside
                // templates from reaching the cascade. The production "single aggregation point"
                // contract uses the sink check as primary; retain this explicit second defense.
                //
                // Check the namespace too, so this applies only to HTML
                // `<template>` (SVG has no defined `<template>` element, but raw parsing
                // may yield local="template"). Matches the sink's `namespace.is_none()` check.
                //
                if tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none() {
                    continue;
                }
                if tag.eq_ignore_ascii_case("style") {
                    // Concatenate child Text nodes.
                    let mut concat = String::new();
                    for child_id in dom.child_ids(id) {
                        if let Some(child) = dom.node(child_id)
                            && let Some(t) = child.text_content()
                        {
                            concat.push_str(t);
                        }
                    }
                    if !concat.is_empty() {
                        on_style_text(&concat);
                    }
                }
            }
            // Push children of every kind onto the stack. Because the stack is LIFO,
            // reverse-push to preserve document order. Extend `stack` directly from the
            // `child_ids` iterator, then reverse only the appended tail slice in place;
            // do not allocate a throwaway intermediate `Vec` each time (as in cascade.rs).
            // `stack` capacity still grows with the same amortized pattern as
            // `for .. { stack.push(..) }`; this avoids only the temporary `Vec`
            // allocated for this iteration.
            //
            // Why document order matters for correctness here: as noted above,
            // `RuleTree::add_stylesheet` assigns monotonically increasing
            // source_order by call order. `build_rule_tree` calls it for each `<style>`
            // element in visit order from this walk. Thus visitation order determines
            // source_order and the cascade tie-break.
            // The two similar locations in cascade.rs (`collect_cascaded` /
            // `resolve_inheritance`) only need to preserve behavior independent of
            // visitation order, in contrast.
            // Flat sibling order alone does not distinguish depth-first from
            // breadth-first traversal; mixed depths make the document-order
            // requirement observable.
            let start = stack.len();
            stack.extend(dom.child_ids(id));
            stack[start..].reverse();
        }
    }
}

fn parse_css_ident(bytes: &[u8], source: &str, position: usize) -> Option<(usize, String)> {
    let mut i = position;
    let mut name = String::new();
    while i < bytes.len() {
        if is_css_ident_byte(bytes[i]) {
            name.push(bytes[i] as char);
            i += 1;
            continue;
        }
        if bytes[i] != b'\\' {
            break;
        }
        i += 1;
        let first = *bytes.get(i)?;
        if first.is_ascii_hexdigit() {
            let mut value = 0u32;
            let mut digits = 0;
            while digits < 6 {
                let Some(byte) = bytes.get(i) else { break };
                let Some(digit) = (*byte as char).to_digit(16) else {
                    break;
                };
                value = value * 16 + digit;
                digits += 1;
                i += 1;
            }
            if bytes.get(i).is_some_and(|byte| byte.is_ascii_whitespace()) {
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if let Some(ch) = char::from_u32(value) {
                name.push(ch);
            }
        } else if let Some(ch) = source[i..].chars().next() {
            name.push(ch);
            i += ch.len_utf8();
        } else {
            return None;
        }
    }
    (!name.is_empty()).then_some((i, name))
}

fn is_css_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn leading_charset_prelude(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut start = 0;
    loop {
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        if source.get(start..)?.starts_with("/*") {
            let end = source.get(start + 2..)?.find("*/")?;
            start += end + 4;
            continue;
        }
        if source.get(start..)?.starts_with("<!--") {
            start += 4;
            continue;
        }
        if source.get(start..)?.starts_with("-->") {
            start += 3;
            continue;
        }
        break;
    }

    let name_start = start.checked_add(1)?;
    if bytes.get(start) != Some(&b'@') {
        return None;
    }
    let (after_name, name) = parse_css_ident(bytes, source, name_start)?;
    if !name.eq_ignore_ascii_case("charset") {
        return None;
    }
    let next = source.get(after_name..)?.chars().next();
    if next.is_some_and(|ch| {
        !ch.is_ascii() || ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '\\')
    }) {
        return None;
    }

    let mut i = after_name;
    let mut stack = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = source.get(i + 2..)?.find("*/")?;
                i += end + 4;
            }
            b'\\' => {
                i += 1;
                i += source.get(i..)?.chars().next()?.len_utf8();
            }
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => {
                            i += 1;
                            i += source.get(i..)?.chars().next()?.len_utf8();
                        }
                        byte if byte == quote => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'(' | b'[' => {
                stack.push(bytes[i]);
                i += 1;
            }
            b')' | b']' => {
                let expected = if bytes[i] == b')' { b'(' } else { b'[' };
                if stack.pop() != Some(expected) {
                    return None;
                }
                i += 1;
            }
            b'{' | b'}' if stack.is_empty() => return None,
            b'{' | b'}' => i += 1,
            b';' if stack.is_empty() => {
                let prelude = source.get(after_name..i)?.to_owned();
                if css_component_values_are_balanced(&prelude) {
                    return Some(prelude);
                }
                return None;
            }
            _ => i += 1,
        }
    }
    None
}

/// Consume the rest of `input` as raw component values and return their text.
///
/// `source` is the string `input` was created from; block-closing checks read
/// it by byte position. A tokenizer error token or a block that is not closed
/// by its own delimiter anywhere in the values rejects them. The walk enters
/// nested blocks directly, so the values are tokenized once.
fn consume_raw_component_values<'i, 't>(
    input: &mut Parser<'i, 't>,
    source: &str,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    let start = input.position();
    if !parse_component_values(input, source, None, 0) {
        return Err(input.new_custom_error(()));
    }
    Ok(input.slice(start..input.position()).to_owned())
}

/// Validate one component-value list with cssparser's own tokenization.
///
/// The recursive walk is deliberately bounded. Once the bound is reached,
/// cssparser still consumes the remaining nested block iteratively, while the
/// caller only relies on this pass to keep malformed input out of the
/// executable views. The raw data itself remains available for inspection.
fn css_component_values_are_balanced(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_component_values(&mut parser, source, None, 0)
}

fn parse_component_values<'i, 't>(
    parser: &mut Parser<'i, 't>,
    source: &str,
    expected_close: Option<u8>,
    depth: usize,
) -> bool {
    loop {
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token,
            Err(_) => {
                return expected_close.is_none()
                    || source
                        .get(parser.position().byte_index()..)
                        .and_then(|suffix| suffix.as_bytes().first())
                        == expected_close.as_ref();
            }
        };

        let closing_delimiter = match token {
            Token::Function(_) | Token::ParenthesisBlock => Some(b')'),
            Token::SquareBracketBlock => Some(b']'),
            Token::CurlyBracketBlock => Some(b'}'),
            _ => None,
        };
        if token.is_parse_error() {
            return false;
        }
        let Some(closing_delimiter) = closing_delimiter else {
            continue;
        };

        if depth >= MAX_OPAQUE_RULE_NESTING_DEPTH {
            let mut ended_at_delimiter = false;
            let result =
                parser.parse_nested_block(|nested| -> Result<(), cssparser::ParseError<'i, ()>> {
                    loop {
                        match nested.next_including_whitespace_and_comments() {
                            Ok(token) => {
                                if token.is_parse_error() {
                                    return Err(nested.new_custom_error(()));
                                }
                            }
                            Err(_) => {
                                ended_at_delimiter = source
                                    .get(nested.position().byte_index()..)
                                    .and_then(|suffix| suffix.as_bytes().first())
                                    == Some(&closing_delimiter);
                                return Ok(());
                            }
                        }
                    }
                });
            if result.is_err() || !ended_at_delimiter {
                return false;
            }
            continue;
        }

        let result =
            parser.parse_nested_block(|nested| -> Result<(), cssparser::ParseError<'i, ()>> {
                if parse_component_values(nested, source, Some(closing_delimiter), depth + 1) {
                    Ok(())
                } else {
                    Err(nested.new_custom_error(()))
                }
            });
        if result.is_err() {
            return false;
        }
    }
}

/// A raw rule list used to expose nested structure without assigning semantics
/// to an unknown at-rule body.
enum NestedParsedRule {
    AtRule(AtRuleRecord),
    Qualified(QualifiedRuleRecord),
}

const MAX_OPAQUE_RULE_NESTING_DEPTH: usize = 128;
// Descendant bodies overlap their ancestors, so cap their aggregate owned copies.
const MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES: usize = 8 * 1024 * 1024;

fn nested_block_has_closing_brace(source: &str, input: &Parser<'_, '_>) -> bool {
    let position = input.position().byte_index();
    let Some(mut suffix) = source.get(position..) else {
        return false;
    };
    loop {
        suffix = suffix.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if let Some(rest) = suffix.strip_prefix("/*") {
            let Some(end) = rest.find("*/") else {
                return false;
            };
            suffix = &rest[end + 2..];
        } else {
            return suffix.starts_with('}');
        }
    }
}

struct RawRuleParser<'s, 'b> {
    depth: usize,
    source: &'s str,
    remaining_body_bytes: &'b mut usize,
}

impl<'i, 's, 'b> cssparser::AtRuleParser<'i> for RawRuleParser<'s, 'b> {
    type Prelude = (String, String);
    type AtRule = NestedParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        Ok((
            name.to_string(),
            consume_raw_component_values(input, self.source)?,
        ))
    }

    fn rule_without_block(
        &mut self,
        (name, prelude): Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        Ok(NestedParsedRule::AtRule(AtRuleRecord {
            name,
            prelude,
            body: AtRuleBody::Statement,
            children: Vec::new(),
            source_order: 0,
            origin: Origin::Author,
        }))
    }

    fn parse_block<'t>(
        &mut self,
        (name, prelude): Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        let body = consume_raw_component_values(input, self.source)?;
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        if body.len() > *self.remaining_body_bytes {
            return Err(input.new_custom_error(()));
        }
        *self.remaining_body_bytes -= body.len();
        let children = if self.depth < MAX_OPAQUE_RULE_NESTING_DEPTH {
            parse_nested_rule_nodes_at_depth(&body, self.depth + 1, self.remaining_body_bytes)
        } else {
            Vec::new()
        };
        Ok(NestedParsedRule::AtRule(AtRuleRecord {
            name,
            prelude,
            body: AtRuleBody::Block(body),
            children,
            source_order: 0,
            origin: Origin::Author,
        }))
    }
}

impl<'i, 's, 'b> cssparser::QualifiedRuleParser<'i> for RawRuleParser<'s, 'b> {
    type Prelude = String;
    type QualifiedRule = NestedParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        consume_raw_component_values(input, self.source)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let body = consume_raw_component_values(input, self.source)?;
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        if body.len() > *self.remaining_body_bytes {
            return Err(input.new_custom_error(()));
        }
        *self.remaining_body_bytes -= body.len();
        Ok(NestedParsedRule::Qualified(QualifiedRuleRecord {
            prelude,
            body,
            source_order: 0,
            origin: Origin::Author,
        }))
    }
}

fn parse_nested_rule_nodes(source: &str, remaining_body_bytes: &mut usize) -> Vec<RuleNode> {
    parse_nested_rule_nodes_at_depth(source, 0, remaining_body_bytes)
}

fn parse_nested_rule_nodes_at_depth(
    source: &str,
    depth: usize,
    remaining_body_bytes: &mut usize,
) -> Vec<RuleNode> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut rule_parser = RawRuleParser {
        depth,
        source,
        remaining_body_bytes,
    };
    let mut nodes = Vec::new();
    for (source_order, rule) in StyleSheetParser::new(&mut parser, &mut rule_parser)
        .flatten()
        .enumerate()
    {
        let node = match rule {
            NestedParsedRule::AtRule(mut record) => {
                record.source_order = source_order as u32;
                RuleNode::AtRule(Box::new(record))
            }
            NestedParsedRule::Qualified(mut record) => {
                record.source_order = source_order as u32;
                RuleNode::Qualified(record)
            }
        };
        nodes.push(node);
    }
    nodes
}

fn set_at_rule_origin(record: &mut AtRuleRecord, origin: Origin) {
    record.origin = origin;
    for node in &mut record.children {
        match node {
            RuleNode::Qualified(qualified) => qualified.origin = origin,
            RuleNode::AtRule(nested) => set_at_rule_origin(nested, origin),
        }
    }
}

fn parse_media_style_rule(
    record: &QualifiedRuleRecord,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Option<StyleRule> {
    let mut input = ParserInput::new(&record.prelude);
    let mut parser = Parser::new(&mut input);
    let selectors = parser
        .parse_entirely(|input| {
            SelectorList::parse(&RaikiriSelectorParser, input, ParseRelative::No)
        })
        .ok()?;
    if !is_supported_selector_list(&selectors) {
        return None;
    }

    let mut input = ParserInput::new(&record.body);
    let mut parser = Parser::new(&mut input);
    Some(StyleRule {
        selectors,
        declarations: parse_declaration_block_with_consumer_properties(
            &mut parser,
            consumer_properties,
        ),
        source_order: 0,
        origin: record.origin,
    })
}

// Style and page outputs share one walk to preserve source order.
#[allow(clippy::too_many_arguments)]
fn collect_media_style_rules(
    record: &AtRuleRecord,
    parent_condition: Option<&MediaCondition>,
    out: &mut Vec<MediaRule>,
    style_order: &mut u32,
    page_out: &mut Vec<PageRule>,
    page_order: &mut u32,
    layer_order: u32,
    consumer_properties: &[ConsumerPropertyRegistration],
) {
    let Some(local_condition) = parse_media_prelude(&record.prelude) else {
        return;
    };
    let condition = match parent_condition {
        Some(parent) => parent.intersect(&local_condition),
        None => local_condition,
    };
    if condition.is_empty() {
        return;
    }

    for child in &record.children {
        match child {
            RuleNode::Qualified(qualified) => {
                let Some(mut rule) = parse_media_style_rule(qualified, consumer_properties) else {
                    continue;
                };
                rule.source_order = *style_order;
                *style_order = style_order.wrapping_add(1);
                out.push(MediaRule {
                    rule,
                    condition: condition.clone(),
                });
            }
            RuleNode::AtRule(nested) if nested.name.eq_ignore_ascii_case("media") => {
                collect_media_style_rules(
                    nested,
                    Some(&condition),
                    out,
                    style_order,
                    page_out,
                    page_order,
                    layer_order,
                    consumer_properties,
                );
            }
            RuleNode::AtRule(nested) if nested.name.eq_ignore_ascii_case("page") => {
                let Some(page_rule) =
                    parse_media_page_rule(nested, condition.clone(), *page_order, layer_order)
                else {
                    continue;
                };
                *page_order = page_order.wrapping_add(1);
                page_out.push(page_rule);
            }
            // An unknown wrapper may have a completely different grammar. Do
            // not accidentally execute its descendants as ordinary CSS rules.
            RuleNode::AtRule(_) => {}
        }
    }
}

/// Parse a `@page` rule nested in `@media` into a media-guarded [`PageRule`].
///
/// The nested record comes from the opaque `@media` child view, so its prelude
/// and body are raw strings. The prelude is reparsed with
/// [`parse_page_prelude`] and the body with [`parse_page_declaration_block`],
/// the same parsers the top-level `@page` path uses. An unparsable prelude or
/// a statement body without a block drops the rule, matching the top-level
/// silent-drop policy for invalid `@page` rules.
fn parse_media_page_rule(
    record: &AtRuleRecord,
    condition: MediaCondition,
    source_order: u32,
    layer_order: u32,
) -> Option<PageRule> {
    let AtRuleBody::Block(body) = &record.body else {
        return None;
    };
    let mut prelude_input = ParserInput::new(&record.prelude);
    let mut prelude_parser = Parser::new(&mut prelude_input);
    let selector = prelude_parser
        .parse_entirely(|input| parse_page_prelude(input))
        .ok()?;
    let mut body_input = ParserInput::new(body);
    let mut body_parser = Parser::new(&mut body_input);
    let PageBlockBody {
        declarations,
        size_declarations,
        marks_declarations,
        bleed_declarations,
        margin_box_rules,
    } = parse_page_declaration_block(&mut body_parser);
    Some(PageRule {
        selector,
        declarations,
        size_declarations,
        marks_declarations,
        bleed_declarations,
        margin_box_rules,
        source_order,
        layer_order,
        origin: record.origin,
        media_condition: Some(condition),
    })
}

/// Intermediate at-rule prelude emitted by [`StyleRuleParser`].
///
/// `@page` keeps its structured selector for the existing page compatibility
/// view. Every other syntactically valid at-rule is retained with its raw
/// component-value prelude so a later semantic pass can reinterpret it.
enum ParsedAtRulePrelude {
    Page(PageSelector),
    Namespace {
        prefix: Option<Atom>,
        uri: Atom,
        prelude: String,
    },
    Opaque {
        name: String,
        prelude: String,
    },
}

/// Top-level parsed rule shape emitted by [`StyleRuleParser`].
///
/// cssparser requires `AtRuleParser::AtRule` and `QualifiedRuleParser::QualifiedRule`
/// to be the same type, so both feed into this enum
/// (due to the `Item = R` constraint on `StyleSheetParser::next`).
enum ParsedRule {
    Style(SelectorList<RaikiriSelectorImpl>, Vec<Declaration>),
    CustomHighlight {
        name: String,
        color: Option<CssColor>,
    },
    Page(PageSelector, PageBlockBody),
    OpaqueAtRule(AtRuleRecord),
}

enum QualifiedPrelude {
    Style(SelectorList<RaikiriSelectorImpl>),
    CustomHighlight(String),
}

/// `StyleSheetParser` implementation. Accepts qualified rules and `@page`;
/// retains other at-rules as opaque records without applying their semantics.
type NamespaceMap = HashMap<Atom, Atom>;

struct NamespacedSelectorParser<'a> {
    namespaces: &'a NamespaceMap,
}

impl<'i, 'a> SelectorParser<'i> for NamespacedSelectorParser<'a> {
    type Impl = RaikiriSelectorImpl;
    type Error = selectors::parser::SelectorParseErrorKind<'i>;

    fn parse_nth_child_of(&self) -> bool {
        true
    }

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_has(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: cssparser::CowRcStr<'i>,
    ) -> Result<PseudoClass, cssparser::ParseError<'i, Self::Error>> {
        RaikiriSelectorParser.parse_non_ts_pseudo_class(location, name)
    }

    fn parse_non_ts_functional_pseudo_class<'t>(
        &self,
        name: cssparser::CowRcStr<'i>,
        parser: &mut Parser<'i, 't>,
        after_part: bool,
    ) -> Result<PseudoClass, cssparser::ParseError<'i, Self::Error>> {
        RaikiriSelectorParser.parse_non_ts_functional_pseudo_class(name, parser, after_part)
    }

    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: cssparser::CowRcStr<'i>,
    ) -> Result<PseudoElem, cssparser::ParseError<'i, Self::Error>> {
        RaikiriSelectorParser.parse_pseudo_element(location, name)
    }

    fn default_namespace(&self) -> Option<Atom> {
        self.namespaces.get(&Atom::from("")).cloned()
    }

    fn namespace_for_prefix(&self, prefix: &Atom) -> Option<Atom> {
        self.namespaces.get(prefix).cloned()
    }
}

struct StyleRuleParser<'s, 'b> {
    source: &'s str,
    opaque_body_budget: &'b mut usize,
    namespaces: &'b mut NamespaceMap,
    consumer_properties: &'b [ConsumerPropertyRegistration],
}

impl<'i, 's, 'b> cssparser::AtRuleParser<'i> for StyleRuleParser<'s, 'b> {
    type Prelude = ParsedAtRulePrelude;
    type AtRule = ParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("page") {
            return parse_page_prelude(input).map(ParsedAtRulePrelude::Page);
        }
        if name.eq_ignore_ascii_case("namespace") {
            let prefix = input
                .try_parse(|input| {
                    input
                        .expect_ident()
                        .map(|prefix| prefix.as_ref().to_owned())
                })
                .ok()
                .map(|prefix| Atom::from(prefix.as_str()));
            let uri = input
                .expect_url()
                .map(|uri| Atom::from(uri.as_ref()))
                .map_err(|_| input.new_custom_error(()))?;
            let prelude = match &prefix {
                Some(prefix) => format!(" {} url({})", prefix.0.as_str(), uri.0.as_str()),
                None => format!(" url({})", uri.0.as_str()),
            };
            return Ok(ParsedAtRulePrelude::Namespace {
                prefix,
                uri,
                prelude,
            });
        }

        // cssparser gives this closure a parser delimited at `;`, `{`, or the
        // end of the current rule list. Consume component values rather than
        // rejecting the at-rule, retaining comments and whitespace from the
        // original source through `slice`.
        Ok(ParsedAtRulePrelude::Opaque {
            name: name.to_string(),
            prelude: consume_raw_component_values(input, self.source)?,
        })
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        match prelude {
            ParsedAtRulePrelude::Page(_) => Err(()),
            ParsedAtRulePrelude::Namespace {
                prefix,
                uri,
                prelude,
            } => {
                self.namespaces
                    .insert(prefix.unwrap_or_else(|| Atom::from("")), uri);
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name: "namespace".to_owned(),
                    prelude,
                    body: AtRuleBody::Statement,
                    children: Vec::new(),
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
            ParsedAtRulePrelude::Opaque { name, prelude } => {
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name,
                    prelude,
                    body: AtRuleBody::Statement,
                    children: Vec::new(),
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        match prelude {
            ParsedAtRulePrelude::Page(selector) => {
                // `@page` body = declaration list + `size` / `marks` / `bleed`
                // descriptors + nested margin-box at-rules — parsed by a
                // dedicated `crate::page::parse_page_declaration_block` (not
                // the generic `parse_declaration_block` qualified rules use).
                // Unsupported properties/descriptors retain the existing
                // silent-drop behavior.
                let body = parse_page_declaration_block(input);
                if !nested_block_has_closing_brace(self.source, input) {
                    return Err(input.new_custom_error(()));
                }
                Ok(ParsedRule::Page(selector, body))
            }
            ParsedAtRulePrelude::Namespace { prelude, .. } => {
                let body = consume_raw_component_values(input, self.source)?;
                if !nested_block_has_closing_brace(self.source, input) {
                    return Err(input.new_custom_error(()));
                }
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name: "namespace".to_owned(),
                    prelude,
                    body: AtRuleBody::Block(body),
                    children: Vec::new(),
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
            ParsedAtRulePrelude::Opaque { name, prelude } => {
                let body = consume_raw_component_values(input, self.source)?;
                if !nested_block_has_closing_brace(self.source, input) {
                    return Err(input.new_custom_error(()));
                }
                let children = parse_nested_rule_nodes(&body, self.opaque_body_budget);
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name,
                    prelude,
                    body: AtRuleBody::Block(body),
                    children,
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
        }
    }
}

impl<'i, 's, 'b> cssparser::QualifiedRuleParser<'i> for StyleRuleParser<'s, 'b> {
    type Prelude = QualifiedPrelude;
    type QualifiedRule = ParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        if let Ok(name) = input.try_parse(parse_custom_highlight_prelude) {
            return Ok(QualifiedPrelude::CustomHighlight(name));
        }
        SelectorList::parse(
            &NamespacedSelectorParser {
                namespaces: self.namespaces,
            },
            input,
            ParseRelative::No,
        )
        .map(QualifiedPrelude::Style)
        .map_err(|_| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let declarations =
            parse_declaration_block_with_consumer_properties(input, self.consumer_properties);
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        match prelude {
            QualifiedPrelude::Style(selectors) => Ok(ParsedRule::Style(selectors, declarations)),
            QualifiedPrelude::CustomHighlight(name) => {
                let color =
                    declarations
                        .iter()
                        .rev()
                        .find_map(|declaration| match declaration.value() {
                            PropertyValue::BackgroundColor(color) => Some(*color),
                            _ => None,
                        });
                Ok(ParsedRule::CustomHighlight { name, color })
            }
        }
    }
}

fn parse_custom_highlight_prelude<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    input.expect_colon()?;
    input.expect_colon()?;
    input.expect_function_matching("highlight")?;
    let name = input.parse_nested_block(|input| {
        let name = input.expect_ident_cloned()?.to_string();
        input.expect_exhausted()?;
        Ok(name)
    })?;
    input.expect_exhausted()?;
    Ok(name)
}

/// Determine whether every selector in the SelectorList contains only currently
/// supported components. Accepts type / universal / class / id / null-namespace
/// attribute selectors (`[foo]` existence and all `[foo=bar]` value forms),
/// descendant (space) / child (`>`) and next-sibling (`+`) / general-sibling
/// (`~`) combinators, and
/// `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
/// (`:lang()` / `:dir()`) as well. But `PseudoClass::Hover` /
/// `PseudoClass::Active` (`:hover` / `:active`), which share the same
/// `Component` variant, remain unsupported.
/// `Component::Negation` (`:not()`), `Component::Is` (`:is()`),
/// `Component::Where` (`:where()`) and `Component::Has` (`:has()`) recursively
/// check their selector lists / relative selector lists using the same gate.
/// Forgiving invalid branches in `:is()`/`:where()` are accepted as
/// `Component::Invalid`, but the matcher ignores those branches.
/// Any other pseudo-class or attribute selector form returns false, dropping
/// the entire rule.
/// The "other attribute selector forms" are the two patterns grouped under
/// `Component::AttributeOther`, but they are not symmetric (confirmed from the
/// actual parse branches in selectors crate v0.39.0 `parser.rs`): namespaced
/// selectors (`[ns|foo]`) always become `AttributeOther`, for both existence
/// and value checks. Local names that are not ASCII-lowercase (e.g., `[Data-Foo]`)
/// without a namespace become `AttributeOther` **only for value checks**.
/// Existence checks with no namespace remain
/// `Component::AttributeInNoNamespaceExists` and are accepted. The asymmetry
/// comes from value checks: the selector's `local_name` is guaranteed to be
/// ASCII-lowercase at parse time (otherwise it becomes `AttributeOther`).
/// This guarantee suffices; the element's namespace does not affect which
/// lookup key to use for the attribute *name*. Resolving the case sensitivity
/// of the **value string** is separate: it happens at match time in
/// `cascade.rs::resolve_case_sensitivity`. By contrast, existence checks
/// have no value to compare, and retain the local name's original case
/// in the selector. Thus the element's namespace determines whether to use
/// `local_name` or `local_name_lower` as the attribute-name lookup key
/// (see the corresponding arm in `cascade.rs::compound_matches`).
///
/// Originally this function accepted only type/universal selectors (formerly
/// `is_type_or_universal_only`). It was later extended to class/id/attribute selectors,
/// `Component::Combinator(Combinator::Descendant | Combinator::Child)`,
/// `Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling)`,
/// `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
/// (`:lang()`/`:dir()`), `Component::Root` / `Component::Empty` /
/// `Component::Nth(_)` (`:root` / `:empty` /
/// `:first-child`/`:last-child`/`:only-child`/`:nth-child()`/`:nth-last-child()`
/// / `:first-of-type`/`:last-of-type`/`:only-of-type`/`:nth-of-type()`/
/// `:nth-last-of-type()`), `Component::PseudoElement(PseudoElem::Before |
/// PseudoElem::After)` and their bridge,
/// `Component::Combinator(Combinator::PseudoElement)` (`::before`/`::after`)
/// were added incrementally, making the type/universal-only name no longer
/// accurate, so the function was renamed. `PseudoElem::Marker` (`::marker`)
/// and `PseudoElem::FirstLine` (`::first-line`) joined the same
/// `Component::PseudoElement` arm later, under the same bridge. Other combinators
/// (`Combinator::SlotAssignment` / `Combinator::Part`) and `:hover`/`:active`
/// remain out of scope (see the `cascade.rs::match_combinator_chain` docs).
/// The two combinators' pseudo syntax (`::slotted()`/`::part()`) cannot occur:
/// `RaikiriSelectorParser` does not override `parse_slotted`/`parse_part`,
/// leaving their default `false`, so this crate's
/// `parse_selector_list` rejects them before this gate.
///
/// `::before`/`::after` are never passed as isolated `Component::PseudoElement`
/// components to the cascade's `compound_matches`/`match_combinator_chain`
/// (see the exception under "Invariant" below). For direct matches against
/// real elements, `cascade.rs::compound_matches` retains its `_ => false`
/// safety net, rejecting a compound containing only `Component::PseudoElement`.
/// This keeps `.foo::before { .. }` from applying directly to the real
/// `.foo` element. A separate matcher,
/// `cascade.rs::selector_matches_pseudo_element`, handles matching the
/// `::before`/`::after` pseudo-elements themselves.
///
/// **All four combinators may mix freely**: a complex selector may combine
/// ancestor (`>`/space) and sibling (`+`/`~`) combinators in any order,
/// any number of times. Both `.x > .y ~ .z` and `.x ~ .y > .z` are accepted.
/// This follows from mutual recursion between `cascade.rs`'s
/// `match_combinator_chain` and `match_from_element`: sibling transitions
/// preserve `ancestors` (siblings share a parent), so ancestor combinators
/// can follow. An ancestor transition correctly truncates the target's own
/// ancestor chain, making `.last()` point to the target's parent, so sibling
/// combinators can also follow it. Neither composition direction needs
/// additional state (see the "resolving the parent" note in
/// the `match_combinator_chain` docs).
///
/// `:root`/`:empty`/`:first-child`, etc., are parsed directly into their
/// dedicated `Component` variants by the `selectors` crate's own
/// `parse_simple_pseudo_class`/`parse_functional_pseudo_class` (verified by
/// reading the published parse branches in selectors 0.39.0 `parser.rs`, not Stylo).
/// They do not go through
/// `RaikiriSelectorParser::parse_non_ts_pseudo_class`/
/// `parse_non_ts_functional_pseudo_class` into `Component::NonTSPseudoClass`:
/// only non-tree-structural pseudo-classes such as `:hover`/`:active`/`:lang()`/
/// `:dir()` use that path. In contrast, `:nth-child(An+B of
/// S)` / `:nth-last-child(An+B of S)` (Selectors Level 4 §13.3.1/§13.3.2)
/// are parsed into `Component::NthOf` by the `selectors` crate
/// because `RaikiriSelectorParser::parse_nth_child_of()` enables that syntax.
/// `is_supported_selector` recursively inspects the selector list stored in
/// `Component::NthOf`; if it contains only supported components, as an ordinary
/// selector would, the rule enters the tree. The cascade considers only matching
/// siblings in its one-based `An+B` sequence; `:nth-last-child()` counts backward
/// through the same filtered list. Thus `of S` is not silently widened to the
/// default sequence of all element siblings, and unsupported components still
/// cause the entire rule to drop. Nested `Component::Nth` / `Component::NthOf`
/// inside `S` are intentionally unsupported. Selectors L4 grammar permits a
/// complex-real-selector-list in `S` (Selectors L4 §13.3.1,
/// <https://www.w3.org/TR/selectors-4/#the-nth-child-pseudo> /
/// §13.3.2 <https://www.w3.org/TR/selectors-4/#the-nth-last-child-pseudo>)
/// but this is a known bounded-support restriction deliberately imposed here.
/// The cascade matcher evaluates `S` for each sibling; allowing nested nth
/// components would amplify sibling scanning recursively. Thus
/// `is_supported_selector` accepts nth components only in the outer selector
/// and rejects nested nth while checking `S`, before any scan. Flat `S`
/// containing ordinary type/class/id/attribute selectors and supported
/// combinators remains accepted.
///
/// spec: CSS Selectors Level 4 — class selector
/// <https://www.w3.org/TR/selectors-4/#class-html>, ID selector
/// <https://www.w3.org/TR/selectors-4/#id-selectors>, attribute selector
/// <https://www.w3.org/TR/selectors-4/#attribute-selectors>, descendant
/// combinator <https://www.w3.org/TR/selectors-4/#descendant-combinators>,
/// child combinator <https://www.w3.org/TR/selectors-4/#child-combinators>,
/// next-sibling combinator
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>,
/// general-sibling combinator
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>,
/// `:lang()` <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>, `:dir()`
/// <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>.
///
/// # Invariant with `cascade.rs::compound_matches` / `match_combinator_chain`
///
/// Every `Component` variant accepted here **must** have a matching arm on the
/// `cascade.rs` side (`compound_matches` for simple selector components and
/// `match_combinator_chain` for combinators). Otherwise, a rule could silently
/// enter the rule tree yet never match in the cascade because it falls through
/// the `_ => false` safety net. The reverse pointer from the
/// `compound_matches` docs to this function already exists in `cascade.rs`.
/// These are independent enumerations; a shared helper was deferred as
/// premature abstraction. Instead, the docs state the invariant explicitly.
/// Changes to combinators or pseudo-classes require editing both this gate and
/// the matching arms in `cascade.rs`, so preserve this relationship on each
/// change.
///
/// **Exception (`Component::PseudoElement` / `Component::Combinator(Combinator::
/// PseudoElement)`)**: only these two are exempt from the invariant.
/// Instead of adding matching arms to `compound_matches`/`match_combinator_chain`,
/// the separate `cascade.rs::selector_matches_pseudo_element` matcher handles
/// them completely. For direct matches against real elements,
/// `compound_matches`'s `_ => false` safety net still rejects isolated
/// `Component::PseudoElement` safely (see the `::before`/`::after` section above
/// and the docs of `selector_matches_pseudo_element` itself).
fn is_supported_selector_list(list: &SelectorList<RaikiriSelectorImpl>) -> bool {
    list.slice()
        .iter()
        .all(|selector| is_supported_selector(selector, true))
}

/// As the name suggests, `allow_nth` gates `Component::Nth`/`NthOf`.
/// It also indicates whether this call checks the **outer** (top-level candidate)
/// selector or the **inner** `S` of `:nth-child(An+B of S)`.
/// The same flag gates `Component::PseudoElement`/`Combinator::PseudoElement`
/// rather than adding a second parameter: CSS Selectors Level 4 also forbids
/// pseudo-elements inside the `S` of `:nth-child(of S)`.
/// The `selectors` crate already enforces this **in its grammar**.
/// Empirically, `p:nth-child(2 of .x::before)` yields `InvalidState` from
/// `selectors` v0.39.0, so a `SelectorList` containing
/// `Component::PseudoElement` cannot be constructed as `S` (see the `lib.rs` test
/// named
/// `parse_pseudo_element_rejected_inside_nth_child_of_selector_list`).
/// Calls with `allow_nth == false` still reject it recursively
/// as defense in depth: we add a safety net even when upstream should
/// prevent the component.
fn is_supported_selector(selector: &Selector<RaikiriSelectorImpl>, allow_nth: bool) -> bool {
    is_supported_selector_with_relative_anchor(selector, allow_nth, false, false)
}

/// Same support check as [`is_supported_selector`], with an explicit allowance
/// for the anchor component that `selectors` inserts into a `:has()` relative
/// selector. The anchor is not a selector that can appear in ordinary CSS
/// input; it is an internal marker and must stay rejected outside that one
/// parser-produced context. `allow_invalid` is restricted to the selector
/// arguments of forgiving `:is()`/`:where()` lists, where `selectors` stores a
/// syntactically invalid branch as `Component::Invalid` for the matcher to
/// ignore.
fn is_supported_selector_with_relative_anchor(
    selector: &Selector<RaikiriSelectorImpl>,
    allow_nth: bool,
    allow_relative_anchor: bool,
    allow_invalid: bool,
) -> bool {
    use selectors::parser::{Combinator, Component};

    selector
        .iter_raw_match_order()
        .all(|component| match component {
            Component::LocalName(_)
            | Component::ExplicitUniversalType
            | Component::ExplicitAnyNamespace
            | Component::ExplicitNoNamespace
            | Component::DefaultNamespace(_)
            | Component::Namespace(_, _)
            | Component::ID(_)
            | Component::Class(_)
            | Component::AttributeInNoNamespaceExists { .. }
            | Component::AttributeInNoNamespace { .. }
            | Component::Combinator(
                Combinator::Descendant
                | Combinator::Child
                | Combinator::NextSibling
                | Combinator::LaterSibling,
            )
            | Component::Root
            | Component::Empty => true,
            Component::Negation(selectors) => selectors.slice().iter().all(|selector| {
                // `:not()` is non-forgiving. A nested `:is()`/`:where()` still
                // applies its own forgiving handling when this recursion sees
                // that component.
                is_supported_selector_with_relative_anchor(selector, allow_nth, false, false)
            }),
            Component::Is(selectors) | Component::Where(selectors) => {
                selectors.slice().iter().all(|selector| {
                    // A nested logical selector matches an ordinary element;
                    // only the outer relative selector owns the `:has()`
                    // anchor marker. Invalid branches are legal here and are
                    // ignored by `selector_slice_matches_with_anchor`.
                    is_supported_selector_with_relative_anchor(selector, allow_nth, false, true)
                })
            }
            // cov:ignore: nested :has() is rejected by `selectors`; this is a future-parser safety net
            Component::Has(_) if allow_relative_anchor => false,
            Component::Has(relative_selectors) => relative_selectors.iter().all(|relative| {
                is_supported_selector_with_relative_anchor(
                    &relative.selector,
                    allow_nth,
                    true,
                    false,
                )
            }),
            Component::Nth(_) => allow_nth,
            Component::NthOf(data) => {
                allow_nth
                    && data.selectors().iter().all(|selector| {
                        is_supported_selector_with_relative_anchor(selector, false, false, false)
                    })
            }
            Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_)) => true,
            Component::Combinator(Combinator::PseudoElement) => allow_nth,
            Component::PseudoElement(
                PseudoElem::Before | PseudoElem::After | PseudoElem::Marker | PseudoElem::FirstLine,
            ) => allow_nth,
            Component::RelativeSelectorAnchor => allow_relative_anchor,
            Component::Invalid(_) => allow_invalid,
            _ => false,
        })
}

#[cfg(test)]
mod tests;
