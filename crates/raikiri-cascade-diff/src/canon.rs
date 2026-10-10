//! Canonical form of `Debug` output.
//!
//! The derived `Debug` output of a `HashMap` or `HashSet` follows the
//! iteration order of the collection, and that order depends on the
//! per-process `RandomState`. Two processes that compute equal cascade results
//! can therefore print different text. Sorting the entries of every map and
//! set literal makes the text a function of the value alone, so outputs of two
//! builds can be compared as strings.

/// Returns `debug` with the entries of every map and set literal sorted, and
/// without every struct field, at any depth, whose value is a literal of the
/// struct named `dropped`.
///
/// A brace group preceded by an identifier is a struct or enum-variant literal
/// (`Name { field: value }`) and keeps its field order. Any other brace group
/// is the output of a map or set (`{key: value, ...}` or `{item, ...}`), and its
/// top-level entries are sorted as strings. Nested groups are canonicalized
/// before the group that contains them, so the result does not depend on the
/// order of entries at any depth. String and character literals are copied
/// verbatim, so braces and commas inside them are not structure.
pub(crate) fn canonicalize(debug: &str, dropped: Option<&str>) -> String {
    let chars: Vec<char> = debug.chars().collect();
    let mut pos = 0;
    // With no closing bracket to stop at, the top level consumes the whole
    // input; an unbalanced closing bracket is copied like any other character.
    canonicalize_until(&chars, &mut pos, None, dropped)
}

/// Splits the canonical text of a struct literal into its top-level fields.
///
/// Returns `None` when `text` is not of the form `Name { a: x, b: y }`. Used to
/// print one field per line so a textual diff points at the field that
/// changed.
pub(crate) fn struct_fields(text: &str) -> Option<(&str, Vec<&str>)> {
    let open = text.find(" { ")?;
    let name = &text[..open];
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let inner = text[open + 3..].strip_suffix(" }")?;
    Some((name, split_top_level(inner)))
}

fn canonicalize_until(
    chars: &[char],
    pos: &mut usize,
    close: Option<char>,
    dropped: Option<&str>,
) -> String {
    let mut out = String::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        if Some(c) == close {
            return out;
        }
        match c {
            '"' | '\'' => copy_quoted(chars, pos, &mut out),
            '(' | '[' => {
                let end = if c == '(' { ')' } else { ']' };
                *pos += 1;
                let inner = canonicalize_until(chars, pos, Some(end), dropped);
                out.push(c);
                out.push_str(&inner);
                if *pos < chars.len() {
                    out.push(end);
                    *pos += 1;
                }
            }
            '{' => {
                let is_struct = out
                    .trim_end()
                    .chars()
                    .next_back()
                    .is_some_and(|prev| prev.is_alphanumeric() || prev == '_');
                *pos += 1;
                let inner = canonicalize_until(chars, pos, Some('}'), dropped);
                out.push('{');
                if is_struct {
                    match dropped {
                        Some(name) => out.push_str(&without_fields_of(&inner, name)),
                        None => out.push_str(&inner),
                    }
                } else {
                    let mut entries = split_top_level(&inner);
                    entries.sort_unstable();
                    out.push_str(&entries.join(", "));
                }
                if *pos < chars.len() {
                    out.push('}');
                    *pos += 1;
                }
            }
            _ => {
                out.push(c);
                *pos += 1;
            }
        }
    }
    out
}

/// The canonical inner text of a struct literal (` a: x, b: y `) without the
/// fields whose value is a `name` literal.
fn without_fields_of(inner: &str, name: &str) -> String {
    let literal = format!("{name} {{");
    let fields: Vec<&str> = split_top_level(inner.trim())
        .into_iter()
        .filter(|field| {
            !field
                .split_once(": ")
                .is_some_and(|(_, value)| value.starts_with(&literal))
        })
        .collect();
    if fields.is_empty() {
        String::new()
    } else {
        format!(" {} ", fields.join(", "))
    }
}

/// Copies one string or character literal, including its escapes.
fn copy_quoted(chars: &[char], pos: &mut usize, out: &mut String) {
    let quote = chars[*pos];
    out.push(quote);
    *pos += 1;
    while *pos < chars.len() {
        let c = chars[*pos];
        out.push(c);
        *pos += 1;
        if c == '\\' {
            if *pos < chars.len() {
                out.push(chars[*pos]);
                *pos += 1;
            }
        } else if c == quote {
            return;
        }
    }
}

/// Splits already-canonical text on the `", "` separators that are not nested
/// inside brackets or literals.
fn split_top_level(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b',' if depth == 0 && bytes.get(i + 1) == Some(&b' ') => {
                    entries.push(&text[start..i]);
                    start = i + 2;
                    i += 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    if start < text.len() {
        entries.push(&text[start..]);
    }
    entries
}

#[cfg(test)]
mod tests;
