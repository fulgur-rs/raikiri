//! Shared preflight for every recursive public selector-parser entry point.

use cssparser::{Parser, Token};

// The upstream selector parser recurses per block. Bound tokens before calling
// it, including blocks in forgiving branches that it might otherwise discard.
const MAX_SELECTOR_TOKEN_DEPTH: usize = 32;

pub(crate) fn check_selector_token_depth<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<(), cssparser::ParseError<'i, ()>> {
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            if depth >= MAX_SELECTOR_TOKEN_DEPTH {
                return Err(input.new_custom_error(()));
            }
            input.parse_nested_block(|nested| check_selector_token_depth(nested, depth + 1))?;
        }
    }
    Ok(())
}
