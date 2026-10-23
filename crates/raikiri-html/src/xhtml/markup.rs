//! Source details that the XML tree does not retain.

use html5ever::interface::{Attribute, QualName};
use html5ever::tendril::StrTendril;
use markup5ever::ns;

use super::XhtmlParseError;

pub(super) fn reject_internal_dtd(source: &str) -> Result<(), XhtmlParseError> {
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        let rest = &source[start..];
        let delimited = [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")]
            .into_iter()
            .find(|(prefix, _)| rest.starts_with(prefix));
        if let Some((prefix, end)) = delimited {
            cursor = rest[prefix.len()..]
                .find(end)
                .map_or(source.len(), |offset| {
                    start + prefix.len() + offset + end.len()
                });
            continue;
        }
        let doctype = rest.starts_with("<!DOCTYPE");
        let mut quote = None;
        cursor = source.len();
        for (offset, byte) in rest.bytes().enumerate() {
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else {
                match byte {
                    b'\'' | b'"' => quote = Some(byte),
                    b'[' if doctype => return Err(XhtmlParseError::InternalDtd),
                    b'>' => {
                        cursor = start + offset + 1;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

pub(super) fn namespace_attributes(node: roxmltree::Node<'_, '_>, source: &str) -> Vec<Attribute> {
    // Only inspect this validated start tag; namespaces() also includes inherited
    // bindings and cannot distinguish an explicit redundant declaration.
    let tag = &source[node.range().start + 1..];
    let name_end = tag
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(tag.len());
    let mut rest = &tag[name_end..];
    let mut attributes = Vec::new();
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if rest.starts_with(['/', '>']) || rest.is_empty() {
            break;
        }
        let name_end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=')
            .expect("validated XML attribute name");
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        rest = rest
            .strip_prefix('=')
            .expect("validated XML attribute equals")
            .trim_start_matches(|c: char| c.is_ascii_whitespace());
        let quote = rest.as_bytes()[0] as char;
        rest = &rest[1..];
        let end = rest.find(quote).expect("validated XML attribute quote");
        rest = &rest[end + 1..];
        let prefix = if name == "xmlns" {
            None
        } else if let Some(prefix) = name.strip_prefix("xmlns:") {
            Some(prefix)
        } else {
            continue;
        };
        let qualified = QualName::new(
            prefix.map(|_| "xmlns".into()),
            ns!(xmlns),
            prefix.unwrap_or("xmlns").into(),
        );
        attributes.push(Attribute {
            name: qualified,
            value: StrTendril::from_slice(node.lookup_namespace_uri(prefix).unwrap_or_default()),
        });
    }
    attributes
}
