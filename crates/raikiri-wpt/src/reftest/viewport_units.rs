//! Resolve the viewport-relative lengths of CSS text to pixels.

use std::ops::{ControlFlow, Range};

/// The viewport a dimension resolves against.
#[derive(Clone, Copy)]
struct Basis {
    width: f32,
    height: f32,
    vertical: bool,
}

type Edits = Vec<(Range<usize>, String)>;

// Bounds recursion on pathological nesting; deeper blocks stay as written.
const MAX_DEPTH: usize = 256;

/// Expand CSS dimensions without interpreting quoted strings as HTML attributes.
pub(super) fn expand_css_viewport_units_with_media_basis(
    input: &str,
    width: f32,
    height: f32,
    media_width: f32,
    media_height: f32,
) -> String {
    expand_css_viewport_units_with_axes(input, width, height, media_width, media_height, false)
}

/// Resolve the viewport-relative lengths of `input`, which is CSS text,
/// against a `width` by `height` viewport. `@media` and `@import` preludes
/// resolve against the `media_width` by `media_height` viewport instead.
///
/// `vertical` is set when the root has a vertical writing mode, which swaps
/// the axes that `vi` and `vb` resolve against.
pub(super) fn expand_css_viewport_units_with_axes(
    input: &str,
    width: f32,
    height: f32,
    media_width: f32,
    media_height: f32,
    vertical: bool,
) -> String {
    let basis = Basis {
        width,
        height,
        vertical,
    };
    let media = Basis {
        width: media_width,
        height: media_height,
        vertical: false,
    };
    let mut edits = Vec::new();
    let mut source = cssparser::ParserInput::new(input);
    expand_items(
        &mut cssparser::Parser::new(&mut source),
        basis,
        media,
        &mut edits,
        0,
    );
    let mut result = String::with_capacity(input.len());
    let mut copied = 0;
    for (range, replacement) in edits {
        result.push_str(&input[copied..range.start]);
        result.push_str(&replacement);
        copied = range.end;
    }
    result.push_str(&input[copied..]);
    result
}

/// The viewport extent that one hundred `unit`s span.
fn unit_extent(unit: &str, basis: Basis) -> Option<f32> {
    let Basis {
        width,
        height,
        vertical,
    } = basis;
    let extent = match unit {
        "vw" | "svw" | "lvw" | "dvw" => width,
        "vh" | "svh" | "lvh" | "dvh" => height,
        "vmin" | "svmin" | "lvmin" | "dvmin" => width.min(height),
        "vmax" | "svmax" | "lvmax" | "dvmax" => width.max(height),
        "vi" | "svi" | "lvi" | "dvi" => {
            if vertical {
                height
            } else {
                width
            }
        }
        "vb" | "svb" | "lvb" | "dvb" => {
            if vertical {
                width
            } else {
                height
            }
        }
        _ => return None,
    };
    Some(extent)
}

/// Expand the dimension `token`, which ended at the parser position,
/// and the dimensions inside it when it opens a block. URL contents are
/// opaque.
fn expand_token<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    token: &cssparser::Token<'i>,
    start: usize,
    basis: Basis,
    edits: &mut Edits,
    depth: usize,
) {
    match token {
        cssparser::Token::Dimension { value, unit, .. } => {
            // Multiply before dividing, as the cascade does, so `100vw`
            // is the viewport width exactly. Scaling by a precomputed
            // `width / 100` rounds twice: 100 * (420 / 100) is 419.99997.
            if let Some(px) =
                unit_extent(&unit.to_ascii_lowercase(), basis).map(|extent| extent * value / 100.0)
                && px.is_finite()
            {
                let end = input.position().byte_index();
                edits.push((start..end, format!("{px:.6}px")));
            }
        }
        cssparser::Token::Function(name) if name.eq_ignore_ascii_case("url") => {}
        cssparser::Token::Function(_)
        | cssparser::Token::ParenthesisBlock
        | cssparser::Token::SquareBracketBlock
        | cssparser::Token::CurlyBracketBlock
            if depth < MAX_DEPTH =>
        {
            let _ = input.parse_nested_block(|input| {
                loop {
                    let start = input.position().byte_index();
                    let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
                        break;
                    };
                    expand_token(input, &token, start, basis, edits, depth + 1);
                }
                Ok::<_, cssparser::ParseError<'i, ()>>(())
            });
        }
        _ => {}
    }
}

/// Expand the dimensions of a list of rules and declarations, the
/// contents of a stylesheet, a declaration block or a style attribute,
/// leaving qualified rule preludes (selectors) as written.
///
/// `@media` and `@import` preludes resolve against the media basis.
fn expand_items<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    basis: Basis,
    media: Basis,
    edits: &mut Edits,
    depth: usize,
) {
    while expand_item(input, basis, media, edits, depth).is_continue() {}
}

/// Expand the next item of the list. The result is `Break` once the input
/// has ended and `Continue` otherwise, also when the next token was skipped
/// as whitespace or a comment.
fn expand_item<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    basis: Basis,
    media: Basis,
    edits: &mut Edits,
    depth: usize,
) -> ControlFlow<()> {
    let start = input.position().byte_index();
    let token = match input.next_including_whitespace_and_comments() {
        Ok(cssparser::Token::WhiteSpace(_) | cssparser::Token::Comment(_)) => {
            return ControlFlow::Continue(());
        }
        Ok(token) => token.clone(),
        Err(_) => return ControlFlow::Break(()),
    };
    if let cssparser::Token::AtKeyword(name) = &token {
        return expand_at_rule(input, name, basis, media, edits, depth);
    }
    expand_declaration_or_rule(input, token, start, basis, media, edits, depth)
}

/// Expand an at-rule whose name was just read: its prelude, and its block
/// when it has one.
fn expand_at_rule<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    name: &str,
    basis: Basis,
    media: Basis,
    edits: &mut Edits,
    depth: usize,
) -> ControlFlow<()> {
    let prelude = if name.eq_ignore_ascii_case("media") || name.eq_ignore_ascii_case("import") {
        media
    } else {
        basis
    };
    loop {
        let start = input.position().byte_index();
        match input.next_including_whitespace_and_comments().cloned() {
            Err(_) => return ControlFlow::Break(()),
            Ok(cssparser::Token::Semicolon) => return ControlFlow::Continue(()),
            Ok(cssparser::Token::CurlyBracketBlock) => {
                nested_items(input, basis, media, edits, depth);
                return ControlFlow::Continue(());
            }
            Ok(token) => expand_token(input, &token, start, prelude, edits, depth),
        }
    }
}

/// Expand a declaration or a qualified rule, which starts with `first`
/// at `start`.
fn expand_declaration_or_rule<'i>(
    input: &mut cssparser::Parser<'i, '_>,
    first: cssparser::Token<'i>,
    start: usize,
    basis: Basis,
    media: Basis,
    edits: &mut Edits,
    depth: usize,
) -> ControlFlow<()> {
    // A custom property value may hold `{}` blocks; any other item
    // that reaches a `{}` block is a qualified rule, whose prelude is a
    // selector rather than a value.
    let custom_property = matches!(&first, cssparser::Token::Ident(name) if name.starts_with("--"));
    let mut pending = Vec::new();
    let mut token = Some((start, first));
    loop {
        let (start, current) = match token.take() {
            Some(token) => token,
            None => {
                let start = input.position().byte_index();
                match input.next_including_whitespace_and_comments() {
                    Ok(token) => (start, token.clone()),
                    Err(_) => {
                        edits.append(&mut pending);
                        return ControlFlow::Break(());
                    }
                }
            }
        };
        match current {
            cssparser::Token::Semicolon => {
                edits.append(&mut pending);
                return ControlFlow::Continue(());
            }
            cssparser::Token::CurlyBracketBlock if !custom_property => {
                nested_items(input, basis, media, edits, depth);
                return ControlFlow::Continue(());
            }
            current => expand_token(input, &current, start, basis, &mut pending, depth),
        }
    }
}

fn nested_items(
    input: &mut cssparser::Parser<'_, '_>,
    basis: Basis,
    media: Basis,
    edits: &mut Edits,
    depth: usize,
) {
    if depth < MAX_DEPTH {
        let _ = input.parse_nested_block(|input| {
            expand_items(input, basis, media, edits, depth + 1);
            Ok::<_, cssparser::ParseError<'_, ()>>(())
        });
    }
}
