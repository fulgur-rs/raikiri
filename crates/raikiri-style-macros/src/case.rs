//! Identifier case conversions.

/// Splits a `CamelCase` identifier at its uppercase letters and joins the
/// lowercased words with `sep`: `ScaleDown` becomes `scale-down` with
/// `sep = '-'` and `scale_down` with `sep = '_'`.
pub(crate) fn split_camel(ident: &str, sep: char) -> String {
    let mut out = String::with_capacity(ident.len() + 4);
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push(sep);
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
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
}
