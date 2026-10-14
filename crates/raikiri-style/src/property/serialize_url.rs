//! CSSOM "serialize a URL" for every `<url>` in a specified value.
//!
//! CSSOM writes a `<url>` as `url(`, the URL as a CSS string, and `)`, so
//! `url(a.png)` and `url('a.png')` both read back as `url("a.png")`. This
//! applies at any nesting depth (`image-set(url(a.png) 1x)`) and for every
//! property that takes a URL.
//! <https://drafts.csswg.org/cssom/#serialize-a-url>

use std::ops::Range;

use cssparser::{Parser, ParserInput, Token};

use super::css_component_values_are_bounded;
use super::serialize_content::push_url;
use super::serialize_position::contains_substitution;

/// `raw_value` with each `url()` rewritten as CSSOM serializes a URL, or
/// `None` when nothing changed.
///
/// A value with a substitution function (`var()`, `env()`, `attr()`) keeps
/// the text it was written with, since its URLs are only known after
/// substitution. So does a `url()` with modifiers after its string, and a
/// value this tokenizer bounds check rejects.
pub fn serialize_urls(raw_value: &str) -> Option<String> {
    // Most values have no URL; skip tokenizing them. A backslash may spell
    // `url` with an escape (`u\72l(a.png)`), so it is tokenized too.
    if !raw_value.contains('\\')
        && !raw_value
            .as_bytes()
            .windows(3)
            .any(|window| window.eq_ignore_ascii_case(b"url"))
    {
        return None;
    }
    if !css_component_values_are_bounded(raw_value)
        || contains_substitution(&mut Parser::new(&mut ParserInput::new(raw_value)))
    {
        return None;
    }
    let mut input = ParserInput::new(raw_value);
    let mut replacements = Vec::new();
    collect_urls(&mut Parser::new(&mut input), &mut replacements);
    apply_replacements(raw_value, replacements)
}

/// `source` with each range replaced by its text, trimmed, or `None` when
/// every replacement equals the text it covers. The ranges must be in
/// source order and must not overlap.
pub(super) fn apply_replacements(
    source: &str,
    replacements: Vec<(Range<usize>, String)>,
) -> Option<String> {
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
    changed.then(|| out.trim().to_owned())
}

/// Pushes the source range and canonical text of each `url()` in `block`,
/// at any nesting depth.
fn collect_urls(block: &mut Parser<'_, '_>, replacements: &mut Vec<(Range<usize>, String)>) {
    loop {
        block.skip_whitespace();
        let start = block.position().byte_index();
        let Ok(token) = block.next() else { return };
        let is_url = match token {
            Token::UnquotedUrl(url) => {
                let canonical = canonical_url(url);
                replacements.push((start..block.position().byte_index(), canonical));
                continue;
            }
            Token::Function(name) => name.eq_ignore_ascii_case("url"),
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => false,
            _ => continue,
        };
        let quoted = block.parse_nested_block(|inner| {
            if !is_url {
                collect_urls(inner, replacements);
                return Ok::<_, cssparser::ParseError<'_, ()>>(None);
            }
            // `url("...")` with one string and nothing else; a URL with
            // modifiers is kept as written.
            let url = inner
                .expect_string()
                .ok()
                .map(|url| url.as_ref().to_owned());
            let lone = inner.is_exhausted();
            while inner.next().is_ok() {}
            Ok(url.filter(|_| lone))
        });
        if let Ok(Some(url)) = quoted {
            replacements.push((start..block.position().byte_index(), canonical_url(&url)));
        }
    }
}

fn canonical_url(url: &str) -> String {
    let mut out = String::new();
    push_url(&mut out, url);
    out
}

#[cfg(test)]
mod tests;
