//! Identifier case conversions.

/// Splits a `CamelCase` identifier into lowercase words joined by `sep`:
/// `ScaleDown` becomes `scale-down` with `sep = '-'` and `scale_down` with
/// `sep = '_'`.
///
/// A new word starts at an uppercase letter that follows a lowercase letter
/// or a digit, and at the last uppercase letter of an acronym that is
/// followed by a lowercase letter, so `HTMLFoo` becomes `html_foo` and
/// `FooHTML` becomes `foo_html`. Digits stay with the preceding word
/// (`Pre2` becomes `pre2`).
pub(crate) fn split_camel(ident: &str, sep: char) -> String {
    let chars: Vec<char> = ident.chars().collect();
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|c| c.is_ascii_lowercase());
            let boundary = prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_is_lower);
            if boundary && !out.ends_with(sep) {
                out.push(sep);
            }
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_camel;

    #[test]
    fn splits_camel_case() {
        assert_eq!(split_camel("ScaleDown", '-'), "scale-down");
        assert_eq!(split_camel("Auto", '-'), "auto");
        assert_eq!(split_camel("EmptyCells", '_'), "empty_cells");
        assert_eq!(split_camel("Pre2", '-'), "pre2");
    }

    #[test]
    fn keeps_acronyms_together() {
        assert_eq!(split_camel("HTMLFoo", '_'), "html_foo");
        assert_eq!(split_camel("FooHTML", '_'), "foo_html");
        assert_eq!(split_camel("FooHTMLBar", '-'), "foo-html-bar");
        assert_eq!(split_camel("URL", '_'), "url");
        assert_eq!(split_camel("Webkit2DText", '_'), "webkit2_d_text");
        assert_eq!(split_camel("Foo_Bar", '_'), "foo_bar");
    }
}
