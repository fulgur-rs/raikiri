//! Shared preflight for every recursive public selector-parser entry point.

use cssparser::{Parser, Token};

// The upstream selector parser recurses per block. Bound tokens before calling
// it, including blocks in forgiving branches that it might otherwise discard.
const MAX_SELECTOR_TOKEN_DEPTH: usize = 32;

/// Checks the token nesting of a selector list, returning how many selectors
/// its top level holds at most: one more than its top-level commas.
pub(crate) fn check_selector_token_depth<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<usize, cssparser::ParseError<'i, ()>> {
    let mut selectors = 1;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::Comma => selectors += 1,
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                if depth >= MAX_SELECTOR_TOKEN_DEPTH {
                    return Err(input.new_custom_error(()));
                }
                input.parse_nested_block(|nested| check_selector_token_depth(nested, depth + 1))?;
            }
            _ => {}
        }
    }
    Ok(selectors)
}
