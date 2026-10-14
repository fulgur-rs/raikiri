//! Deterministic inputs for stage and page-output benchmarks.

// Each bench binary uses a different subset of these shared fixtures.
#![allow(dead_code)]

use std::fmt::Write as _;

use raikiri::{FontCollectionBuilder, HtmlDocument, RenderResources, parse_html_with_resources};

/// Resources with only the bundled Ahem font, independent of installed fonts.
pub fn resources() -> RenderResources<'static> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("the bundled font builds");
    RenderResources::new().fonts(fonts)
}

/// Formatting contexts whose costs should be tracked independently.
#[derive(Clone, Copy)]
pub enum Shape {
    /// Paragraph construction, shaping and line breaking.
    Text,
    /// Multiple text nodes per paragraph.
    Inline,
    /// Fixed table layout with collapsed borders.
    TableFixed,
    /// Intrinsic table sizing with collapsed borders.
    TableAuto,
    /// Row and column spans in a collapsed-border table.
    TableSpans,
    /// Wrapping flex items.
    Flex,
    /// Grid track sizing.
    Grid,
    /// Balanced columns.
    Multicol,
}

impl Shape {
    /// Stable benchmark identifier.
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Inline => "inline",
            Self::TableFixed => "table_fixed",
            Self::TableAuto => "table_auto",
            Self::TableSpans => "table_spans",
            Self::Flex => "flex",
            Self::Grid => "grid",
            Self::Multicol => "multicol",
        }
    }
}

/// Build `items` paragraphs, items or table rows inside one formatting context.
pub fn document(shape: Shape, items: usize) -> String {
    let css = match shape {
        Shape::Text | Shape::Inline => "",
        Shape::TableFixed => "table { table-layout: fixed }",
        Shape::TableAuto | Shape::TableSpans => "table { table-layout: auto }",
        Shape::Flex => {
            ".container { display: flex; flex-wrap: wrap; gap: 4px } .item { width: 100px }"
        }
        Shape::Grid => ".container { display: grid; grid-template-columns: 1fr 2fr 1fr; gap: 4px }",
        Shape::Multicol => ".container { column-count: 3; column-gap: 16px; column-fill: balance }",
    };
    let mut html = format!(
        "<!doctype html><html><head><style>\
         @page {{ size: 640px 480px; margin: 20px }}\
         body {{ margin: 0; font-family: Ahem; font-size: 10px; line-height: 12px }}\
         p {{ margin: 0 0 4px }}\
         table {{ width: 100%; border-collapse: collapse }}\
         td {{ border: 1px solid #999; padding: 2px }}\
         {css}</style></head><body>"
    );
    if matches!(
        shape,
        Shape::TableFixed | Shape::TableAuto | Shape::TableSpans
    ) {
        html.push_str("<table><tbody>");
        for row in 0..items {
            if matches!(shape, Shape::TableSpans) {
                if row % 2 == 0 {
                    write!(
                        html,
                        "<tr><td rowspan=2>R{row}</td><td colspan=2>C{row}</td><td>X</td></tr>"
                    )
                    .unwrap();
                } else {
                    html.push_str("<tr><td>A</td><td>B</td><td>C</td></tr>");
                }
            } else {
                write!(
                    html,
                    "<tr><td>R{row}</td><td>description</td><td>123</td><td>4567</td></tr>"
                )
                .unwrap();
            }
        }
        html.push_str("</tbody></table>");
    } else {
        html.push_str("<div class=container>");
        for item in 0..items {
            match shape {
                Shape::Inline => {
                    write!(html, "<p>Item {item} ").unwrap();
                    for _ in 0..8 {
                        html.push_str("<span>alpha </span><em>beta </em>");
                    }
                    html.push_str("end.</p>");
                }
                Shape::Flex | Shape::Grid => {
                    write!(html, "<div class=item>Item {item}</div>").unwrap();
                }
                _ => {
                    write!(html, "<p>Item {item}: alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu.</p>").unwrap();
                }
            }
        }
        html.push_str("</div>");
    }
    html.push_str("</body></html>");
    html
}

/// Forced page breaks with one short paragraph per section.
pub fn pages(count: usize) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><style>\
         @page { size: 640px 480px; margin: 20px }\
         body { margin: 0; font-family: Ahem; font-size: 10px; line-height: 12px }\
         section { break-after: page } section:last-child { break-after: auto }\
         p { margin: 0 }</style></head><body>",
    );
    for page in 0..count {
        write!(
            html,
            "<section><p>Page {page}: alpha beta gamma.</p></section>"
        )
        .unwrap();
    }
    html.push_str("</body></html>");
    html
}

/// Parse and cascade once, outside stage measurements.
pub fn parse(html: &str, resources: &RenderResources<'_>) -> HtmlDocument {
    parse_html_with_resources(html.as_bytes(), resources).expect("the fixture parses and cascades")
}

/// Fresh DOM and cascade with fonts installed for the direct layout entry point.
pub fn single_page_input(
    html: &str,
    resources: &RenderResources<'_>,
) -> (raikiri::UncascadedDocument, raikiri::CascadeResult) {
    let (mut document, cascade) = parse(html, resources).into_parts();
    // Parsing does not install resource fonts; paged layout normally does this.
    document
        .dom
        .set_font_collection(resources.inline_engine_fonts());
    assert!(document.dom.has_font_collection());
    (document, cascade)
}

/// Confirm the formatting context was selected rather than falling back to blocks.
pub fn check_shape(dom: &raikiri::Document, cascade: &raikiri::CascadeResult, shape: Shape) {
    use raikiri_style::property::{ColumnCountValue, DisplayValue, TableLayoutValue};
    let table = matches!(
        shape,
        Shape::TableFixed | Shape::TableAuto | Shape::TableSpans
    );
    let id = (0..dom.node_count())
        .find(|&id| {
            let node = dom.get_node(id).unwrap();
            if table {
                node.tag_name() == Some("table")
            } else {
                node.attribute("class") == Some("container")
            }
        })
        .expect("formatting context root");
    let computed = &cascade.computed[id];
    let display = match shape {
        Shape::Flex => DisplayValue::Flex,
        Shape::Grid => DisplayValue::Grid,
        Shape::TableFixed | Shape::TableAuto | Shape::TableSpans => DisplayValue::Table,
        _ => DisplayValue::Block,
    };
    assert_eq!(computed.display, display, "formatting context");
    if table {
        assert_eq!(
            computed.table_layout,
            if matches!(shape, Shape::TableFixed) {
                TableLayoutValue::Fixed
            } else {
                TableLayoutValue::Auto
            }
        );
    }
    if matches!(shape, Shape::Multicol) {
        assert_eq!(computed.column_count, ColumnCountValue::Count(3));
    }
}

/// A multiset of visible ASCII characters, ignoring collapsed whitespace.
pub fn text_signature(dom: &raikiri::Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    for id in 0..dom.node_count() {
        let node = dom.get_node(id).expect("arena node");
        if dom
            .parent_of(id)
            .and_then(|parent| dom.get_node(parent))
            .and_then(|parent| parent.tag_name())
            != Some("style")
            && let Some(text) = node.text_content()
        {
            bytes.extend(text.bytes().filter(|byte| !byte.is_ascii_whitespace()));
        }
    }
    bytes.sort_unstable();
    bytes
}

/// Verify every text node has lines and every visible element has geometry.
pub fn check_layout(dom: &raikiri::Document, expected_items: usize) {
    let mut text_nodes = 0;
    for id in 0..dom.node_count() {
        let node = dom.get_node(id).expect("arena node");
        if matches!(node.tag_name(), Some("p" | "td" | "div")) {
            assert!(
                node.unrounded_layout.size.width > 0.0,
                "element {id} has no width"
            );
            assert!(
                node.unrounded_layout.size.height > 0.0,
                "element {id} has no height"
            );
        }
        if node.text_content().is_some()
            && dom
                .parent_of(id)
                .and_then(|parent| dom.get_node(parent))
                .and_then(|parent| parent.tag_name())
                != Some("style")
        {
            assert!(
                dom.ifc_text_lines(id).is_some(),
                "text node {id} has no lines"
            );
            text_nodes += 1;
        }
    }
    assert!(
        text_nodes >= expected_items,
        "layout lost text: {text_nodes} < {expected_items}"
    );
}
