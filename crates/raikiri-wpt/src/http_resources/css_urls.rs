use cssparser::{ParseError, Parser, ParserInput, Token};
use raikiri::Url;

#[derive(Debug)]
struct Replacement {
    start: usize,
    end: usize,
    value: String,
    /// Whether `start..end` covers a whole `url()` (rewritten as
    /// `url("…")`) rather than a single string token (rewritten as `"…"`).
    whole_url: bool,
}

/// `source` with every relative URL resolved against `base_url`.
///
/// `url()`, `src()` and the bare `<string>` URLs of `image-set()` are
/// rewritten. Fragment-only references (`url(#id)`) name an element of the
/// document that uses the stylesheet, and empty URLs name no resource, so
/// both are left as written.
pub(super) fn absolutize_stylesheet_urls(source: &str, base_url: &Url) -> String {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut replacements = Vec::new();
    let _ = collect_urls(&mut parser, &mut replacements, false);
    if replacements.is_empty() {
        return source.to_owned();
    }

    replacements.sort_by_key(|replacement| replacement.start);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for replacement in replacements {
        // An empty URL names no resource, so it is left as written too.
        if replacement.start < cursor
            || replacement.value.is_empty()
            || replacement.value.starts_with('#')
        {
            continue;
        }
        let Some(absolute) = Url::parse(&replacement.value)
            .ok()
            .or_else(|| base_url.join(&replacement.value).ok())
        else {
            continue;
        };
        let mut string = String::new();
        if cssparser::serialize_string(absolute.as_str(), &mut string).is_err() {
            continue;
        }
        output.push_str(&source[cursor..replacement.start]);
        if replacement.whole_url {
            output.push_str("url(");
            output.push_str(&string);
            output.push(')');
        } else {
            output.push_str(&string);
        }
        cursor = replacement.end;
    }
    output.push_str(&source[cursor..]);
    output
}

/// Collect the URLs in `parser`.
///
/// `in_image_set` is true directly inside `image-set()` or
/// `-webkit-image-set()`, whose bare `<string>` arguments are URLs
/// (CSS Images 4 §2.2).
fn collect_urls<'i, 't>(
    parser: &mut Parser<'i, 't>,
    replacements: &mut Vec<Replacement>,
    in_image_set: bool,
) -> Result<(), ParseError<'i, ()>> {
    loop {
        let start = parser.position().byte_index();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => return Ok(()),
        };
        match token {
            Token::UnquotedUrl(value) => replacements.push(Replacement {
                start,
                end: parser.position().byte_index(),
                value: value.to_string(),
                whole_url: true,
            }),
            Token::QuotedString(value) if in_image_set => replacements.push(Replacement {
                start,
                end: parser.position().byte_index(),
                value: value.to_string(),
                whole_url: false,
            }),
            Token::Function(name)
                if name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("src") =>
            {
                if let Ok(mut replacement) = parser.parse_nested_block(url_argument) {
                    // A modifier-free `url()` is rewritten whole; otherwise
                    // only the string changes so the function name and any
                    // `<url-modifier>`s stay as written.
                    if name.eq_ignore_ascii_case("url") && replacement.whole_url {
                        replacement.start = start;
                        replacement.end = parser.position().byte_index();
                    } else {
                        replacement.whole_url = false;
                    }
                    replacements.push(replacement);
                }
            }
            Token::Function(name) => {
                let image_set = name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set");
                parser
                    .parse_nested_block(|nested| collect_urls(nested, replacements, image_set))?;
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                parser.parse_nested_block(|nested| collect_urls(nested, replacements, false))?;
            }
            _ => {}
        }
    }
}

/// Parse `<string> <url-modifier>*`, the arguments of `url()` and `src()`
/// (CSS Values 4 §4.5), where a `<url-modifier>` is an `<ident>` or a
/// function. The result spans the string token and has `whole_url` set
/// when no modifier follows it.
fn url_argument<'i>(nested: &mut Parser<'i, '_>) -> Result<Replacement, ParseError<'i, ()>> {
    nested.skip_whitespace();
    let start = nested.position().byte_index();
    let value = nested.expect_string_cloned()?.to_string();
    let end = nested.position().byte_index();
    let mut has_modifiers = false;
    while !nested.is_exhausted() {
        has_modifiers = true;
        match nested.next()? {
            Token::Ident(_) => {}
            Token::Function(_) => nested.parse_nested_block(|modifier| {
                while modifier.next().is_ok() {}
                Ok(())
            })?,
            token => {
                let token = token.clone();
                return Err(nested.new_unexpected_token_error(token));
            }
        }
    }
    Ok(Replacement {
        start,
        end,
        value,
        whole_url: !has_modifiers,
    })
}

#[cfg(test)]
mod tests;
