//! CSS token boundaries shared by SVG stylesheet consumers.

use cssparser::{
    AtRuleParser, ParseError, Parser, ParserInput, ParserState, QualifiedRuleParser,
    StyleSheetParser,
};

/// A qualified rule whose selector and body still borrow the original CSS.
#[derive(Debug)]
pub(crate) struct RawRule<'a> {
    pub(crate) selectors: &'a str,
    pub(crate) body: &'a str,
}

struct RuleParser;

impl<'i> AtRuleParser<'i> for RuleParser {
    type Prelude = ();
    type AtRule = RawRule<'i>;
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for RuleParser {
    type Prelude = &'i str;
    type QualifiedRule = RawRule<'i>;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, ()>> {
        Ok(remaining(input))
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, ()>> {
        Ok(RawRule {
            selectors,
            body: remaining(input),
        })
    }
}

fn remaining<'i>(input: &mut Parser<'i, '_>) -> &'i str {
    let start = input.position();
    while input.next_including_whitespace_and_comments().is_ok() {}
    input.slice_from(start)
}

/// Visit qualified rules without allocating selector or declaration arrays.
/// Unsupported at-rules remain outside the SVG backend's execution scope.
pub(crate) fn visit_rules<'a>(css: &'a str, mut visit: impl FnMut(RawRule<'a>)) {
    let mut input = ParserInput::new(css);
    let mut input = Parser::new(&mut input);
    for rule in StyleSheetParser::new(&mut input, &mut RuleParser).flatten() {
        visit(rule);
    }
}

/// Read one stylesheet with CSS declaration recovery.
pub(crate) fn parse(css: &str) -> simplecss::StyleSheet<'_> {
    let mut sheet = simplecss::StyleSheet::new();
    parse_more(&mut sheet, css);
    sheet
}

/// Bound recovered selector and declaration allocations before parsing them.
pub(crate) fn estimate(css: &str) -> crate::InitialStylesheetResourceEstimate {
    let mut estimate = crate::InitialStylesheetResourceEstimate::default();
    visit_rules(css, |rule| {
        let mut count = 1usize;
        let mut input = ParserInput::new(rule.selectors);
        let mut input = Parser::new(&mut input);
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            if matches!(token, cssparser::Token::Comma) {
                count = count.saturating_add(1);
            }
        }
        let mut declarations = 0usize;
        visit_declarations(rule.body, |_| declarations = declarations.saturating_add(1));
        // Saturation remains an upper bound: preflight rejects estimates
        // above its finite limits before allocating the corresponding arrays.
        let ast = count
            .saturating_mul(std::mem::size_of::<simplecss::Rule<'static>>())
            .saturating_mul(2)
            .saturating_add(
                rule.selectors
                    .len()
                    .saturating_mul(crate::MAX_INITIAL_PARSE_SELECTOR_AST_BYTES_PER_SOURCE_BYTE),
            );
        let storage = count
            .saturating_add(1)
            .saturating_mul(declarations.max(4))
            .saturating_mul(std::mem::size_of::<simplecss::Declaration<'static>>())
            .saturating_mul(crate::INITIAL_PARSE_DECLARATION_CAPACITY_FACTOR);
        estimate.selector_count = estimate.selector_count.saturating_add(count);
        estimate.selector_ast_bytes = estimate.selector_ast_bytes.saturating_add(ast);
        estimate.declaration_storage_bytes =
            estimate.declaration_storage_bytes.saturating_add(storage);
    });
    estimate
}

/// Append recovered rules, retaining the backend's selector language and
/// stable specificity ordering.
pub(crate) fn parse_more<'a>(sheet: &mut simplecss::StyleSheet<'a>, css: &'a str) {
    visit_rules(css, |rule| {
        let declarations = declarations(rule.body);
        if declarations.is_empty() {
            return;
        }
        for text in crate::split_css_selector_list(rule.selectors) {
            if let Some(selector) =
                simplecss::Selector::parse(text.trim_matches(|c: char| c.is_ascii_whitespace()))
            {
                sheet.rules.push(simplecss::Rule {
                    selector,
                    declarations: declarations.clone(),
                });
            } else {
                break;
            }
        }
    });
    sheet
        .rules
        .sort_by_cached_key(|rule| rule.selector.specificity());
}

/// The declarations of a block, as a browser splits them.
///
/// `simplecss` ends a function at the first closing parenthesis, so it loses
/// every declaration after a nested one, as in `x:calc(min(1px, 2px));y:3px`,
/// and it reads `!important` in lowercase only. Here a function, a
/// parenthesis, a bracket and a brace are blocks that nest, a string and a
/// comment are single tokens, and `!important` may be spelled in any case with
/// white space or comments inside. A declaration without a name, a colon or a
/// value is skipped up to the next semicolon, which is how a browser recovers.
pub(crate) fn declarations(style: &str) -> Vec<simplecss::Declaration<'_>> {
    let mut result = Vec::new();
    visit_declarations(style, |declaration| result.push(declaration));
    result
}

fn visit_declarations<'a>(style: &'a str, mut visit: impl FnMut(simplecss::Declaration<'a>)) {
    use cssparser::Token;

    let mut input = cssparser::ParserInput::new(style);
    let mut parser = cssparser::Parser::new(&mut input);
    loop {
        parser.skip_whitespace();
        let name_start = parser.position();
        match parser.next_including_whitespace_and_comments() {
            Err(_) => return,
            Ok(Token::Semicolon) => continue,
            Ok(Token::Ident(_)) => {}
            Ok(_) => {
                skip_declaration(&mut parser);
                continue;
            }
        }
        let name = parser.slice_from(name_start);
        match parser.next() {
            Ok(Token::Colon) => {}
            Ok(Token::Semicolon) => continue,
            Err(_) => return,
            Ok(_) => {
                skip_declaration(&mut parser);
                continue;
            }
        }
        parser.skip_whitespace();
        let value_start = parser.position();
        // Where the value ends: after its last token that is not white space or
        // a comment, and before the `!important` that closes it if it has one.
        let mut end = value_start;
        let mut cut = None;
        // Where the value ended before a `!` that is the last token.
        let mut bang = None;
        loop {
            let piece = match parser.next_including_whitespace_and_comments() {
                Ok(Token::Semicolon) | Err(_) => break,
                Ok(Token::WhiteSpace(_) | Token::Comment(_)) => continue,
                Ok(Token::Delim('!')) => Piece::Bang,
                Ok(Token::Ident(word)) if word.eq_ignore_ascii_case("important") => {
                    Piece::Important
                }
                Ok(
                    Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock,
                ) => Piece::Block,
                Ok(_) => Piece::Other,
            };
            if matches!(piece, Piece::Block) {
                skip_block(&mut parser);
            }
            match piece {
                Piece::Bang => {
                    bang = Some(end);
                    cut = None;
                }
                Piece::Important if bang.is_some() => cut = bang.take(),
                _ => {
                    bang = None;
                    cut = None;
                }
            }
            end = parser.position();
        }
        let important = cut.is_some();
        let value = parser.slice(value_start..cut.unwrap_or(end));
        if !value.is_empty() {
            visit(simplecss::Declaration {
                name,
                value,
                important,
            });
        }
    }
}

/// The kinds of token that a value is told apart by.
enum Piece {
    /// The `!` that may start `!important`.
    Bang,
    /// The word `important`, in any case.
    Important,
    /// A function, a parenthesis, a bracket or a brace, which holds tokens of
    /// its own up to its closing mark.
    Block,
    Other,
}

/// Moves past the block that the token just read opened: to its closing mark,
/// or to the end of the text.
fn skip_block(parser: &mut cssparser::Parser<'_, '_>) {
    let _ = parser.parse_nested_block(|inner| {
        while inner.next_including_whitespace_and_comments().is_ok() {}
        Ok::<(), cssparser::ParseError<'_, ()>>(())
    });
}

/// Moves past the rest of a declaration that is not one: to the semicolon that
/// ends it, or to the end. A block stands for a single token, so a semicolon in
/// one does not end the declaration.
fn skip_declaration(parser: &mut cssparser::Parser<'_, '_>) {
    while !matches!(
        parser.next_including_whitespace_and_comments(),
        Ok(cssparser::Token::Semicolon) | Err(_)
    ) {}
}

#[cfg(test)]
mod tests;

/// A private backend copy whose declaration boundaries SimpleCSS can read.
/// Geometry has already consumed the complete cascade before this runs.
/// The document's retained/exported source is never replaced by this copy.
pub(crate) fn backend_source(source: &str) -> Result<std::borrow::Cow<'_, str>, crate::SvgError> {
    if !source.contains("<style") && !source.contains(":style") {
        return Ok(std::borrow::Cow::Borrowed(source));
    }
    let xml = roxmltree::Document::parse(source)
        .map_err(|error| crate::SvgError::InvalidDocument(error.to_string()))?;
    let mut budget = crate::SelectorFreezeBudget::new();
    budget.bytes(source.len())?;
    let mut edits = Vec::new();
    for (node, range) in xml
        .descendants()
        .filter(|node| node.has_tag_name("style") && crate::svg_style_has_css_type(*node))
        .filter_map(|node| {
            crate::xml_element_content_range(source, node).map(|range| (node, range))
        })
    {
        let Some(text) = node.text() else { continue };
        // Ordinary stylesheets need no copy or declaration-rule expansion.
        // Both reads remain within the initial preflight allocation bounds.
        let legacy = simplecss::StyleSheet::parse(text);
        let recovered = parse(text);
        if legacy.rules.len() == recovered.rules.len()
            && legacy.rules.iter().zip(&recovered.rules).all(|(old, new)| {
                old.declarations == new.declarations
                    && old.selector.to_string() == new.selector.to_string()
            })
        {
            continue;
        }
        let mut output = String::new();
        let mut failure = None;
        visit_rules(text, |rule| {
            if failure.is_some() {
                return;
            }
            let declarations = declarations(rule.body);
            // Keep the selector acceptance boundary identical to parse_more.
            // An unsupported list entry does not expose later entries.
            for selector in crate::split_css_selector_list(rule.selectors) {
                let selector = selector.trim_matches(|c: char| c.is_ascii_whitespace());
                if simplecss::Selector::parse(selector).is_none() {
                    break;
                }
                for declaration in declarations.iter().copied() {
                    let result =
                        backend_declaration(selector, declaration, &mut output, &mut budget);
                    if let Err(error) = result {
                        failure = Some(error);
                        return;
                    }
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        budget.bytes(crate::xml_attribute_escape_allocation_bytes(&output)?)?;
        edits.push((range, crate::escape_xml_text(&output)));
    }
    if edits.is_empty() {
        Ok(std::borrow::Cow::Borrowed(source))
    } else {
        let source = crate::apply_selector_edits(source, edits, &mut budget)?;
        let xml = roxmltree::Document::parse(&source)
            .map_err(|error| crate::SvgError::InvalidDocument(error.to_string()))?; // cov:ignore: Defensive error mapping; edits replace only validated XML text with escaped CSS.
        // Each recovered declaration becomes a backend rule. Recheck its
        // selector storage and matching work against the expanded document.
        crate::preflight_initial_svg_selectors(&xml)?;
        Ok(std::borrow::Cow::Owned(source))
    }
}

fn backend_declaration(
    selectors: &str,
    declaration: simplecss::Declaration<'_>,
    output: &mut String,
    budget: &mut crate::SelectorFreezeBudget,
) -> Result<(), crate::SvgError> {
    let length = selectors
        .len()
        .saturating_add(declaration.name.len())
        .saturating_add(declaration.value.len())
        .saturating_add(40);
    budget.bytes(length.saturating_mul(4))?;
    let priority = if declaration.important {
        "!important"
    } else {
        ""
    };
    let text = format!("{}:{}{}", declaration.name, declaration.value, priority);
    let mut tokens = simplecss::DeclarationTokenizer::from(text.as_str());
    if tokens.next() != Some(declaration) || tokens.next().is_some() {
        return Ok(());
    }
    // Check recovery against an independent following rule as well: malformed
    // strings and braces must never consume or expose neighboring rules.
    let probe = format!("*{{{text};}} *{{raikiri-guard:ok}}");
    let checked = simplecss::StyleSheet::parse(&probe);
    if checked.rules.len() != 2
        || checked.rules[0].declarations != [declaration]
        || checked.rules[1].declarations
            != [simplecss::Declaration {
                name: "raikiri-guard",
                value: "ok",
                important: false,
            }]
    {
        return Ok(()); // cov:ignore: Defensive guard; exact tokenizer equality currently implies an isolated single-declaration rule.
    }
    output.push_str(selectors);
    output.push('{');
    output.push_str(&text);
    output.push_str(";}\n");
    Ok(())
}
