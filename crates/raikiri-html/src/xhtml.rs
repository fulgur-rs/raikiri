//! XML parsing into the same DOM and stylesheet pipeline as HTML.

use std::io::Read;

use html5ever::interface::{Attribute, ElementFlags, NodeOrText, QualName, TreeSink};
use html5ever::tendril::StrTendril;
use markup5ever::Namespace;
use raikiri_traits::{LimitKind, ParseError, RenderLimits};

use crate::{ParseOptions, RaikiriTreeSink, UncascadedDocument};

mod markup;

/// A terminal XHTML input, budget, or XML syntax error.
#[non_exhaustive]
#[derive(Debug)]
pub enum XhtmlParseError {
    /// Input reading, UTF-8 decoding, or a parser budget failed.
    Input(ParseError),
    /// The document is not well-formed XML or uses an unsupported entity.
    Xml(roxmltree::Error),
    /// Internal DTD subsets are unsupported to prevent unbounded entity expansion.
    InternalDtd,
}

impl std::fmt::Display for XhtmlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(error) => error.fmt(f),
            Self::Xml(error) => write!(f, "XHTML syntax error: {error}"),
            Self::InternalDtd => write!(f, "XHTML internal DTD subsets are unsupported"),
        }
    }
}

impl std::error::Error for XhtmlParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Xml(error) => Some(error),
            Self::InternalDtd => None,
        }
    }
}

impl From<ParseError> for XhtmlParseError {
    fn from(error: ParseError) -> Self {
        Self::Input(error)
    }
}

/// Parse UTF-8 XHTML as XML, preserving self-closing elements, namespaces,
/// case, and CDATA text, and collecting stylesheets as [`crate::parse()`] does.
///
/// Applies the default input-byte and DOM-node budgets to both the intermediate
/// XML tree and the resulting DOM. XML syntax errors are terminal; this never
/// falls back to HTML parsing. External DTDs and entities are not fetched.
/// Internal DTD subsets are rejected before parsing to bound entity expansion.
///
/// # Errors
/// Returns an error for invalid UTF-8, malformed XML, unsupported DTDs or
/// entities, input I/O failures, or exceeded parser budgets.
pub fn parse_xhtml(
    input: impl Read,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, XhtmlParseError> {
    parse_with_limits(input, options, &RenderLimits::default())
}

fn parse_with_limits(
    input: impl Read,
    options: &ParseOptions<'_>,
    limits: &RenderLimits,
) -> Result<UncascadedDocument, XhtmlParseError> {
    let mut bytes = Vec::new();
    input
        .take(
            limits
                .max_input_bytes
                .map_or(u64::MAX, |cap| cap.saturating_add(1)),
        )
        .read_to_end(&mut bytes)
        .map_err(ParseError::Io)?;
    if let Some(limit) = limits.max_input_bytes
        && bytes.len() as u64 > limit
    {
        return Err(ParseError::LimitExceeded {
            kind: LimitKind::InputBytes,
            limit,
            actual: bytes.len() as u64,
        }
        .into());
    }
    let source = std::str::from_utf8(&bytes).map_err(|error| ParseError::Encoding {
        label: "utf-8".into(),
        reason: error.to_string(),
    })?;
    markup::reject_internal_dtd(source)?;
    let nodes_limit = limits
        .max_dom_nodes
        .unwrap_or(u64::from(u32::MAX))
        .min(u64::from(u32::MAX)) as u32;
    let xml = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            // The preflight rejects internal subsets. No external resolver is installed.
            allow_dtd: true,
            nodes_limit,
            ..Default::default()
        },
    )
    .map_err(|error| match error {
        roxmltree::Error::NodesLimitReached => XhtmlParseError::Input(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit: u64::from(nodes_limit),
            actual: u64::from(nodes_limit) + 1,
        }),
        error => XhtmlParseError::Xml(error),
    })?;
    let document = build_dom(&xml, limits)?;
    crate::parse::finish_document(document, options).map_err(Into::into)
}

fn build_dom(
    xml: &roxmltree::Document<'_>,
    limits: &RenderLimits,
) -> Result<UncascadedDocument, XhtmlParseError> {
    let source = xml.input_text();
    let sink = RaikiriTreeSink::with_limits(limits.max_parse_warnings, limits.max_dom_nodes);
    let signal = sink.dom_node_limit_signal();
    let mut stack = vec![(xml.root(), sink.get_document())];
    while let Some((node, parent)) = stack.pop() {
        let handle = match node.node_type() {
            roxmltree::NodeType::Root => parent,
            roxmltree::NodeType::Element => {
                let tag = node.tag_name();
                let raw_name = source[node.range().start + 1..]
                    .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .next()
                    .unwrap_or_default();
                let mut attributes: Vec<_> = node
                    .attributes()
                    .map(|attr| Attribute {
                        name: qualified_name(
                            attr.namespace(),
                            attr.name(),
                            &source[attr.range_qname()],
                        ),
                        value: StrTendril::from_slice(attr.value()),
                    })
                    .collect();
                attributes.extend(markup::namespace_attributes(node, source));
                let handle = sink.create_element(
                    qualified_name(tag.namespace(), tag.name(), raw_name),
                    attributes,
                    ElementFlags::default(),
                );
                sink.append(&parent, NodeOrText::AppendNode(handle));
                handle
            }
            roxmltree::NodeType::Text => {
                sink.append(
                    &parent,
                    NodeOrText::AppendText(StrTendril::from_slice(node.text().unwrap_or_default())),
                );
                parent
            }
            roxmltree::NodeType::Comment => {
                let handle =
                    sink.create_comment(StrTendril::from_slice(node.text().unwrap_or_default()));
                sink.append(&parent, NodeOrText::AppendNode(handle));
                handle
            }
            roxmltree::NodeType::PI => {
                let pi = node.pi().expect("processing instruction node");
                let handle = sink.create_pi(
                    StrTendril::from_slice(pi.target),
                    StrTendril::from_slice(pi.value.unwrap_or_default()),
                );
                sink.append(&parent, NodeOrText::AppendNode(handle));
                handle
            }
        };
        if let Some((limit, actual)) = sink.dom_node_limit_error() {
            return Err(ParseError::LimitExceeded {
                kind: LimitKind::DomNodes,
                limit,
                actual,
            }
            .into());
        }
        stack.extend(node.children().rev().map(|child| (child, handle)));
    }
    let document = sink.finish();
    if let Some(actual) = signal.get() {
        return Err(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit: limits.max_dom_nodes.unwrap_or(actual.saturating_sub(1)),
            actual,
        }
        .into());
    }
    Ok(document)
}

fn qualified_name(namespace: Option<&str>, local: &str, raw: &str) -> QualName {
    QualName::new(
        raw.split_once(':').map(|(prefix, _)| prefix.into()),
        Namespace::from(namespace.unwrap_or_default()),
        local.into(),
    )
}

#[cfg(test)]
mod tests;
