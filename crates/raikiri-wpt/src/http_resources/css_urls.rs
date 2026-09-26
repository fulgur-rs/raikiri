use cssparser::{ParseError, Parser, ParserInput, Token};
use raikiri::Url;

#[derive(Debug)]
struct Replacement {
    start: usize,
    end: usize,
    value: String,
}

pub(super) fn absolutize_stylesheet_urls(source: &str, base_url: &Url) -> String {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut replacements = Vec::new();
    let _ = collect_urls(&mut parser, &mut replacements);
    if replacements.is_empty() {
        return source.to_owned();
    }

    replacements.sort_by_key(|replacement| replacement.start);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for replacement in replacements {
        if replacement.start < cursor {
            continue;
        }
        let Some(absolute) = Url::parse(&replacement.value)
            .ok()
            .or_else(|| base_url.join(&replacement.value).ok())
        else {
            continue;
        };
        output.push_str(&source[cursor..replacement.start]);
        output.push_str("url(\"");
        output.push_str(&absolute.as_str().replace('\\', "\\\\").replace('"', "\\\""));
        output.push_str("\")");
        cursor = replacement.end;
    }
    output.push_str(&source[cursor..]);
    output
}

fn collect_urls<'i, 't>(
    parser: &mut Parser<'i, 't>,
    replacements: &mut Vec<Replacement>,
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
            }),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let value = parser.parse_nested_block(|nested| {
                    let value = nested.expect_string_cloned()?;
                    nested.expect_exhausted()?;
                    Ok::<_, ParseError<'i, ()>>(value.to_string())
                });
                if let Ok(value) = value {
                    replacements.push(Replacement {
                        start,
                        end: parser.position().byte_index(),
                        value,
                    });
                }
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                parser.parse_nested_block(|nested| collect_urls(nested, replacements))?;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
