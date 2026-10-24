//! The `all` shorthand with `initial`, `inherit` or `unset`.
//!
//! `all` sets every property except `direction` and `unicode-bidi` to the
//! keyword (CSS Cascade 4 section 3.1), and each of those properties then
//! takes the keyword like a declaration of its own, competing with the other
//! declarations of the property by the ordinary cascade order. Rule parsing
//! cannot expand it into that many declarations without every `all` in a
//! stylesheet costing a few hundred of them, so it stays one candidate and the
//! cascade spreads it over the properties for the elements that have it
//! ([`ElementCandidates::with_all_spread`]).
//!
//! The spread declarations are the ones that `name: keyword` parses to for each
//! property name, so a property takes the keyword exactly as it does when
//! written alone. They are built once per keyword and shared.
//!
//! `revert` and `revert-layer` are not spread: one candidate rolls every slot
//! back (`Candidate::is_all_rollback`).
//!
//! [`ElementCandidates::with_all_spread`]: super::candidate::ElementCandidates::with_all_spread

use std::collections::HashMap;
use std::sync::LazyLock;

use cssparser::{Parser, ParserInput};

use crate::property::{
    CssWideKeyword, PropertyKey, property_key_for_name, supported_property_names,
};
use crate::rule::{Declaration, parse_declaration_block_within};

/// The spread declarations for `initial`, `inherit` and `unset`.
static SPREAD: LazyLock<[Vec<Declaration>; 3]> = LazyLock::new(|| {
    [
        build(CssWideKeyword::Initial),
        build(CssWideKeyword::Inherit),
        build(CssWideKeyword::Unset),
    ]
});

/// The declarations `all: keyword` stands for, one for each property it
/// resets that accepts the keyword; empty for the keywords that roll back.
pub(crate) fn declarations(keyword: CssWideKeyword) -> &'static [Declaration] {
    match keyword {
        CssWideKeyword::Initial => &SPREAD[0],
        CssWideKeyword::Inherit => &SPREAD[1],
        CssWideKeyword::Unset => &SPREAD[2],
        CssWideKeyword::Revert | CssWideKeyword::RevertLayer => &[],
    }
}

/// Whether `key` takes part in a spread. The flow-relative sizes stay out:
/// the cascade keeps each apart from the physical size it maps to, and applies
/// it after that one, so a keyword spread over `inline-size` would override a
/// `width` that outranks the `all`.
const fn is_spread(key: PropertyKey) -> bool {
    key.is_reset_by_all()
        && !matches!(
            key,
            PropertyKey::All
                | PropertyKey::Custom
                | PropertyKey::InlineSize
                | PropertyKey::BlockSize
                | PropertyKey::MinInlineSize
                | PropertyKey::MinBlockSize
                | PropertyKey::MaxInlineSize
                | PropertyKey::MaxBlockSize
        )
}

/// What `keyword` gives each supported property name, as the declarations a
/// rule would hold for `name: keyword`, shorthands expanded and each key kept
/// once, as its own name's declaration where that has one. The flow-relative
/// sides of margins, paddings and offsets stay out: the physical declarations
/// cover their slots.
fn build(keyword: CssWideKeyword) -> Vec<Declaration> {
    let mut at = HashMap::new();
    let mut spread = Vec::new();
    for name in supported_property_names() {
        let own_key = property_key_for_name(name);
        let css = format!("{name}: {}", keyword.as_css_str());
        let mut input = ParserInput::new(&css);
        let mut parser = Parser::new(&mut input);
        // No block expands to more than `usize::MAX` declarations.
        let declarations =
            parse_declaration_block_within(&mut parser, &[], usize::MAX).unwrap_or_default();
        for declaration in declarations {
            if declaration.logical.is_some() || !is_spread(declaration.key) {
                continue;
            }
            match at.get(&declaration.key) {
                None => {
                    at.insert(declaration.key, spread.len());
                    spread.push(declaration);
                }
                // A shorthand sorts before its longhands; the longhand's own
                // declaration replaces the one the shorthand expanded to.
                Some(&index) if own_key == Some(declaration.key) => spread[index] = declaration,
                Some(_) => {}
            }
        }
    }
    spread
}

#[cfg(test)]
mod tests;
