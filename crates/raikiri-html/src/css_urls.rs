//! Resolution of relative `url()` values against their stylesheet's URL.
//!
//! CSS Values 4 §4.5.1 resolves a relative URL in a stylesheet against the
//! stylesheet's own URL, so `url(bg.png)` in `/css/print.css` names
//! `/css/bg.png`. Computed values keep the authored text, and resource
//! loading later resolves what is still relative against the document base
//! URL. Rewriting the relative values of each stylesheet part to absolute
//! URLs when the part is created keeps that per-stylesheet base.

use std::borrow::Cow;

use cssparser::{ParseError, Parser, ParserInput, Token};
use url::Url;

/// `source` with every relative `url()` resolved against `base`.
///
/// Absolute URLs, fragment-only references (`url(#id)`, which name an
/// element of the document) and empty URLs are left as written, as is the
/// `@namespace` prelude, whose URL is a name rather than a resource.
pub(crate) fn absolutize_urls<'s>(source: &'s str, base: &Url) -> Cow<'s, str> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut edits = Vec::new();
    collect(&mut parser, base, &mut edits);
    if edits.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for (start, end, replacement) in edits {
        out.push_str(&source[cursor..start]);
        out.push_str(&replacement);
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    Cow::Owned(out)
}

fn collect(parser: &mut Parser<'_, '_>, base: &Url, edits: &mut Vec<(usize, usize, String)>) {
    let mut in_namespace = false;
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token.clone() {
            Token::AtKeyword(name) => in_namespace = name.eq_ignore_ascii_case("namespace"),
            Token::Semicolon | Token::CurlyBracketBlock if in_namespace => {
                in_namespace = false;
                if matches!(token, Token::CurlyBracketBlock) {
                    nested(parser, base, edits);
                }
            }
            _ if in_namespace => skip_block(parser, &token),
            Token::UnquotedUrl(value) => {
                let end = parser.position().byte_index();
                push(edits, start, end, &value, base);
            }
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let value = parser.parse_nested_block(|input| {
                    let value = input.expect_string()?.as_ref().to_owned();
                    Ok::<_, ParseError<'_, ()>>(value)
                });
                if let Ok(value) = value {
                    let end = parser.position().byte_index();
                    push(edits, start, end, &value, base);
                }
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => nested(parser, base, edits),
            _ => {}
        }
    }
}

fn nested(parser: &mut Parser<'_, '_>, base: &Url, edits: &mut Vec<(usize, usize, String)>) {
    let _ = parser.parse_nested_block(|input| {
        collect(input, base, edits);
        Ok::<_, ParseError<'_, ()>>(())
    });
}

/// Consume the contents of a block token without rewriting them.
fn skip_block(parser: &mut Parser<'_, '_>, token: &Token<'_>) {
    if matches!(
        token,
        Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock
    ) {
        let _ = parser.parse_nested_block(|input| {
            while input.next_including_whitespace_and_comments().is_ok() {}
            Ok::<_, ParseError<'_, ()>>(())
        });
    }
}

fn push(
    edits: &mut Vec<(usize, usize, String)>,
    start: usize,
    end: usize,
    value: &str,
    base: &Url,
) {
    if value.is_empty() || value.starts_with('#') || Url::parse(value).is_ok() {
        return;
    }
    let Ok(absolute) = base.join(value) else {
        return;
    };
    let mut replacement = String::from("url(");
    if cssparser::serialize_string(absolute.as_str(), &mut replacement).is_err() {
        return;
    }
    replacement.push(')');
    edits.push((start, end, replacement));
}

#[cfg(test)]
mod tests;
