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
/// The URLs of `src()` and the bare `<string>` URLs of `image-set()` resolve
/// the same way. Absolute URLs, fragment-only references (`url(#id)`, which
/// name an element of the document) and empty URLs are left as written, as
/// is the `@namespace` prelude, whose URL is a name rather than a resource.
pub(crate) fn absolutize_urls<'s>(source: &'s str, base: &Url) -> Cow<'s, str> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut edits = Vec::new();
    collect(&mut parser, base, &mut edits, false);
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

/// Collect the edits for the URLs in `parser`.
///
/// `in_image_set` is true directly inside `image-set()` or
/// `-webkit-image-set()`, whose bare `<string>` arguments are URLs
/// (CSS Images 4 §2.2).
fn collect(
    parser: &mut Parser<'_, '_>,
    base: &Url,
    edits: &mut Vec<(usize, usize, String)>,
    in_image_set: bool,
) {
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
                    nested(parser, base, edits, false);
                }
            }
            _ if in_namespace => skip_block(parser, &token),
            Token::UnquotedUrl(value) => {
                let end = parser.position().byte_index();
                push_url(edits, start, end, &value, base);
            }
            Token::QuotedString(value) if in_image_set => {
                let end = parser.position().byte_index();
                push_string(edits, start, end, &value, base);
            }
            Token::Function(name)
                if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
            {
                let Ok(argument) = parser.parse_nested_block(url_argument) else {
                    continue;
                };
                if name.eq_ignore_ascii_case("url") && !argument.has_modifiers {
                    let end = parser.position().byte_index();
                    push_url(edits, start, end, &argument.value, base);
                } else {
                    // Rewrite only the string so the function name and any
                    // `<url-modifier>`s stay as written.
                    push_string(edits, argument.start, argument.end, &argument.value, base);
                }
            }
            Token::Function(name) => {
                let image_set = name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set");
                nested(parser, base, edits, image_set);
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                nested(parser, base, edits, false)
            }
            _ => {}
        }
    }
}

/// The `<string>` argument of a `url()` or `src()` function.
struct UrlArgument {
    value: String,
    /// Byte range of the string token in the stylesheet source.
    start: usize,
    end: usize,
    has_modifiers: bool,
}

/// Parse `<string> <url-modifier>*`, the arguments of `url()` and `src()`
/// (CSS Values 4 §4.5), where a `<url-modifier>` is an `<ident>` or a
/// function.
fn url_argument<'i>(input: &mut Parser<'i, '_>) -> Result<UrlArgument, ParseError<'i, ()>> {
    input.skip_whitespace();
    let start = input.position().byte_index();
    let value = input.expect_string()?.as_ref().to_owned();
    let end = input.position().byte_index();
    let mut has_modifiers = false;
    while !input.is_exhausted() {
        has_modifiers = true;
        match input.next()? {
            Token::Ident(_) => {}
            Token::Function(_) => input.parse_nested_block(|modifier| {
                while modifier.next().is_ok() {}
                Ok(())
            })?,
            token => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token));
            }
        }
    }
    Ok(UrlArgument {
        value,
        start,
        end,
        has_modifiers,
    })
}

fn nested(
    parser: &mut Parser<'_, '_>,
    base: &Url,
    edits: &mut Vec<(usize, usize, String)>,
    in_image_set: bool,
) {
    let _ = parser.parse_nested_block(|input| {
        collect(input, base, edits, in_image_set);
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

/// Replace `start..end` with `url("<absolute>")`.
fn push_url(
    edits: &mut Vec<(usize, usize, String)>,
    start: usize,
    end: usize,
    value: &str,
    base: &Url,
) {
    if let Some(string) = absolute_string(value, base) {
        edits.push((start, end, format!("url({string})")));
    }
}

/// Replace the string token at `start..end` with `"<absolute>"`.
fn push_string(
    edits: &mut Vec<(usize, usize, String)>,
    start: usize,
    end: usize,
    value: &str,
    base: &Url,
) {
    if let Some(string) = absolute_string(value, base) {
        edits.push((start, end, string));
    }
}

/// `value` resolved against `base` and serialized as a CSS string, or
/// `None` when it is kept as written.
fn absolute_string(value: &str, base: &Url) -> Option<String> {
    // The URL parser strips leading and trailing C0 controls and spaces.
    let value = value.trim_matches(|c: char| c <= ' ');
    if value.is_empty() || value.starts_with('#') || Url::parse(value).is_ok() {
        return None;
    }
    let absolute = base.join(value).ok()?;
    let mut string = String::new();
    cssparser::serialize_string(absolute.as_str(), &mut string).ok()?;
    Some(string)
}

#[cfg(test)]
mod tests;
