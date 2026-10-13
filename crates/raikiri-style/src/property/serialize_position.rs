//! Canonical serialization of `<position>` values from their source text.
//!
//! The parsed [`CssPosition`](super::CssPosition) keeps only edge offsets,
//! so it cannot tell `left` from `0%` or `center` from `50%`. CSSOM keeps
//! those keywords in a specified value and only normalizes their order
//! (CSS Values 4 §"Serializing `<position>`"): the horizontal component comes
//! first, and a one-component value gains the implied `center`.

use cssparser::{Parser, ParserInput, Token};

use super::css_component_values_are_bounded;

/// Serializes `raw_value` for the `<position>`-valued properties
/// (`background-position`, `object-position`) and for `background-image`,
/// whose radial gradients' `at <position>` is canonicalized and whose
/// `url()`s serialize as `url("...")` (CSSOM "serialize a URL"). Returns `None` for other
/// properties or for text this canonicalizer does not recognize, in which
/// case callers keep the source text.
pub fn serialize_position_value(name: &str, raw_value: &str) -> Option<String> {
    if !css_component_values_are_bounded(raw_value) {
        return None;
    }
    match name {
        "background-position" => canonical_position_list(raw_value),
        "object-position" => canonical_position(raw_value),
        "background-image" => canonical_image_list(raw_value),
        _ => None,
    }
}

/// One `<position>` component: an optional edge keyword and its optional
/// offset, both as source text.
struct Component<'a> {
    keyword: Option<String>,
    offset: Option<&'a str>,
}

impl Component<'_> {
    fn is_horizontal_keyword(&self) -> bool {
        matches!(self.keyword.as_deref(), Some("left" | "right"))
    }

    fn is_vertical_keyword(&self) -> bool {
        matches!(self.keyword.as_deref(), Some("top" | "bottom"))
    }

    fn text(&self) -> String {
        match (&self.keyword, self.offset) {
            (Some(keyword), Some(offset)) => format!("{keyword} {offset}"),
            (Some(keyword), None) => keyword.clone(),
            (None, Some(offset)) => offset.to_owned(),
            (None, None) => String::new(),
        }
    }
}

fn is_substitution_function(name: &str) -> bool {
    ["var", "env", "attr"]
        .iter()
        .any(|function| name.eq_ignore_ascii_case(function))
}

/// Whether `block` uses a substitution function at any nesting depth, as
/// in `calc(var(--y))`. Consumes the rest of `block`.
fn contains_substitution(block: &mut Parser<'_, '_>) -> bool {
    let mut found = false;
    while let Ok(token) = block.next() {
        let nested = match token {
            Token::Function(name) if is_substitution_function(name) => {
                found = true;
                true
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => true,
            _ => false,
        };
        if nested {
            found |= block
                .parse_nested_block(|inner| {
                    Ok::<_, cssparser::ParseError<'_, ()>>(contains_substitution(inner))
                })
                .unwrap_or(false);
        }
    }
    found
}

/// The canonical text of a comma-separated list of `<position>` values,
/// one per background layer, or `None` when any item is not one.
fn canonical_position_list(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let items = parser
        .parse_comma_separated(|item| {
            let start = item.position();
            while item.next().is_ok() {}
            Ok::<_, cssparser::ParseError<'_, ()>>(item.slice_from(start).trim().to_owned())
        })
        .ok()?;
    let items = items
        .iter()
        .map(|item| canonical_position(item))
        .collect::<Option<Vec<_>>>()?;
    Some(items.join(", "))
}

/// The canonical text of a whole `<position>` value, or `None` when
/// `source` is not one.
fn canonical_position(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut values = Vec::new();
    while !parser.is_exhausted() {
        let start = parser.position();
        let keyword = match parser.next().ok()? {
            Token::Ident(ident) => Some(ident.to_ascii_lowercase()),
            Token::Dimension { .. } | Token::Percentage { .. } | Token::Number { .. } => None,
            // A substitution function may stand for any number of
            // components, so no axis can be inferred before substitution.
            Token::Function(name) if is_substitution_function(name) => return None,
            Token::Function(_) => {
                let substituted = parser
                    .parse_nested_block(|block| {
                        Ok::<_, cssparser::ParseError<'_, ()>>(contains_substitution(block))
                    })
                    .ok()?;
                if substituted {
                    return None;
                }
                None
            }
            _ => return None,
        };
        values.push((keyword, parser.slice_from(start).trim()));
    }
    let components = group_components(values)?;
    let (horizontal, vertical) = match components.as_slice() {
        [only] if only.is_vertical_keyword() => (center(), Some(only)),
        [only] => (only.text(), None),
        [first, second] => {
            // An edge keyword names its axis; `center` and bare offsets take
            // whichever axis the other component leaves.
            if first.is_vertical_keyword() || second.is_horizontal_keyword() {
                (second.text(), Some(first))
            } else {
                (first.text(), Some(second))
            }
        }
        _ => return None,
    };
    let vertical = vertical.map_or_else(center, Component::text);
    Some(format!("{horizontal} {vertical}"))
}

fn center() -> String {
    "center".to_owned()
}

/// Pairs each edge keyword with the offset that follows it. `center` never
/// takes an offset.
fn group_components(values: Vec<(Option<String>, &str)>) -> Option<Vec<Component<'_>>> {
    let mut components: Vec<Component<'_>> = Vec::new();
    for (keyword, text) in values {
        match keyword {
            Some(keyword) => {
                if !matches!(
                    keyword.as_str(),
                    "left" | "right" | "top" | "bottom" | "center"
                ) {
                    return None;
                }
                components.push(Component {
                    keyword: Some(keyword),
                    offset: None,
                });
            }
            None => match components.last_mut() {
                Some(last)
                    if last.offset.is_none()
                        && last.keyword.as_deref().is_some_and(|k| k != "center") =>
                {
                    last.offset = Some(text);
                }
                _ => components.push(Component {
                    keyword: None,
                    offset: Some(text),
                }),
            },
        }
    }
    // A bare offset after a keyword-only component is the two-value form
    // (`left 10px`), not a keyword offset, when only two values were given.
    if components.len() == 1
        && let Some(only) = components.first()
        && only.keyword.is_some()
        && only.offset.is_some()
    {
        let offset = only.offset;
        let keyword = only.keyword.clone();
        components = vec![
            Component {
                keyword,
                offset: None,
            },
            Component {
                keyword: None,
                offset,
            },
        ];
    }
    Some(components)
}

/// `source` with the `at <position>` of each top-level radial gradient
/// canonicalized and each top-level `url()` written as a quoted string, or
/// `None` when nothing changed.
fn canonical_image_list(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut replacements = Vec::new();
    loop {
        parser.skip_whitespace();
        let start = parser.position().byte_index();
        let Ok(token) = parser.next() else { break };
        if let Token::UnquotedUrl(url) = token {
            let canonical = serialized_url(url);
            replacements.push((start..parser.position().byte_index(), canonical));
            continue;
        }
        let is_radial = matches!(
            token,
            Token::Function(name)
                if name.eq_ignore_ascii_case("radial-gradient")
                    || name.eq_ignore_ascii_case("repeating-radial-gradient")
        );
        let is_url = matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("url"));
        if !matches!(token, Token::Function(_)) {
            continue;
        }
        let quoted = parser.parse_nested_block(|block| {
            if is_radial && let Some(found) = gradient_position(source, block) {
                replacements.push(found);
            }
            let quoted = match block.next() {
                Ok(Token::QuotedString(url)) if is_url => Some(url.as_ref().to_owned()),
                _ => None,
            };
            let only_token = block.is_exhausted();
            while block.next().is_ok() {}
            Ok::<_, cssparser::ParseError<'_, ()>>(quoted.filter(|_| only_token))
        });
        if let Ok(Some(url)) = quoted {
            let end = parser.position().byte_index();
            replacements.push((start..end, serialized_url(&url)));
        }
    }
    let mut out = String::with_capacity(source.len());
    let mut copied = 0;
    let mut changed = false;
    for (range, canonical) in replacements {
        changed |= source[range.clone()] != canonical;
        out.push_str(&source[copied..range.start]);
        out.push_str(&canonical);
        copied = range.end;
    }
    out.push_str(&source[copied..]);
    changed.then_some(out)
}

/// `url` as CSSOM serializes a URL: `url(` and the URL as a CSS string.
fn serialized_url(url: &str) -> String {
    let mut out = String::from("url(");
    // Writing to a String cannot fail.
    let _ = cssparser::serialize_string(url, &mut out);
    out.push(')');
    out
}

/// The source range of the position after `at` in a radial gradient's
/// first argument, with its canonical text (keeping one leading space).
fn gradient_position(
    source: &str,
    block: &mut Parser<'_, '_>,
) -> Option<(std::ops::Range<usize>, String)> {
    let mut range: Option<std::ops::Range<usize>> = None;
    loop {
        match block.next() {
            Ok(Token::Comma) | Err(_) => break,
            Ok(Token::Ident(ident)) if range.is_none() && ident.eq_ignore_ascii_case("at") => {
                let start = block.position().byte_index();
                range = Some(start..start);
            }
            Ok(token) => {
                if matches!(token, Token::Function(_) | Token::ParenthesisBlock) {
                    let _ = block.parse_nested_block(|nested| {
                        while nested.next().is_ok() {}
                        Ok::<_, cssparser::ParseError<'_, ()>>(())
                    });
                }
                if let Some(range) = range.as_mut() {
                    range.end = block.position().byte_index();
                }
            }
        }
    }
    let range = range.filter(|range| range.end > range.start)?;
    let canonical = canonical_position(source[range.clone()].trim())?;
    Some((range, format!(" {canonical}")))
}

#[cfg(test)]
mod tests;
