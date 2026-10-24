//! Deterministic inputs for stage and page-output benchmarks.

// Each bench binary uses a different subset of these shared fixtures.
#![allow(dead_code)]

use std::fmt::Write as _;

use raikiri::{FontCollectionBuilder, HtmlDocument, RenderResources, parse_html_with_resources};

/// Resources with only the bundled Ahem font, independent of installed fonts.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
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
// cov:ignore: bench-only item; workspace tests do not instrument this target.
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
    /// A flex container that is the containing block of many absolutely
    /// positioned children.
    FlexOof,
    /// A grid container that is the containing block of many absolutely
    /// positioned children.
    GridOof,
    /// Balanced columns.
    Multicol,
}

// cov:ignore: bench-only item; workspace tests do not instrument this target.
impl Shape {
    /// How many text-bearing items `document(self, items)` holds. The out-of-flow
    /// shapes hold `items` boxes without text, next to a tenth as many items with
    /// text.
    pub fn text_items(self, items: usize) -> usize {
        match self {
            Self::FlexOof | Self::GridOof => oof_in_flow_items(items),
            _ => items,
        }
    }

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
            Self::FlexOof => "flex_oof",
            Self::GridOof => "grid_oof",
            Self::Multicol => "multicol",
        }
    }
}

/// The children of an out-of-flow shape that stay in flow, so that the
/// container has content of its own next to the positioned boxes.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn oof_in_flow_items(items: usize) -> usize {
    (items / 10).max(1)
}

/// Build `items` paragraphs, items or table rows inside one formatting context.
///
/// The out-of-flow shapes hold `items` children: the first
/// [`oof_in_flow_items`] stay in flow and the others are absolutely positioned
/// boxes (every tenth one fixed), mixing boxes placed by insets, boxes placed by
/// their static position and, in the grid, boxes placed against a grid area.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
pub fn document(shape: Shape, items: usize) -> String {
    let css = match shape {
        Shape::Text | Shape::Inline => "",
        Shape::TableFixed => "table { table-layout: fixed }",
        Shape::TableAuto | Shape::TableSpans => "table { table-layout: auto }",
        Shape::Flex => {
            ".container { display: flex; flex-wrap: wrap; gap: 4px } .item { width: 100px }"
        }
        Shape::Grid => ".container { display: grid; grid-template-columns: 1fr 2fr 1fr; gap: 4px }",
        Shape::FlexOof => {
            ".container { display: flex; position: relative; flex-wrap: wrap; gap: 4px; height: 400px } \
             .item { width: 100px } \
             .oof { position: absolute; width: 20px; height: 10px } \
             .a { left: 10px; top: 5px } .b { right: 3px; bottom: 3px } \
             .c { align-self: center; justify-self: end } \
             .f { position: fixed; left: 1px; top: 1px }"
        }
        Shape::GridOof => {
            ".container { display: grid; position: relative; grid-template-columns: 1fr 2fr 1fr; gap: 4px; height: 400px } \
             .oof { position: absolute; width: 20px; height: 10px } \
             .a { grid-column: 2 / 3; grid-row: 1 / 3; left: 2px; top: 2px } \
             .b { grid-column: 3; right: 3px; bottom: 3px } \
             .c { align-self: end; justify-self: center } \
             .f { position: fixed; left: 1px; top: 1px }"
        }
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
                Shape::FlexOof | Shape::GridOof => {
                    if item < oof_in_flow_items(items) {
                        write!(html, "<div class=item>Item {item}</div>").unwrap();
                    } else {
                        let placement = if item % 10 == 9 {
                            "f"
                        } else {
                            ["a", "b", "c"][item % 3]
                        };
                        write!(html, "<div class=\"oof {placement}\"></div>").unwrap();
                    }
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
// cov:ignore: bench-only item; workspace tests do not instrument this target.
pub fn pages(count: usize) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><style>\
         @page { size: 640px 480px; margin: 20px }\
         body { margin: 0; font-family: Ahem; font-size: 10px; line-height: 12px }\
         section { break-after: page } section.last-page { break-after: auto }\
         p { margin: 0 }</style></head><body>",
    );
    for page in 0..count {
        // A class marks the last page without a forward-dependent selector
        // that would hold every streaming page until EOF.
        let class = if page + 1 == count {
            " class=last-page"
        } else {
            ""
        };
        write!(
            html,
            "<section{class}><p>Page {page}: alpha beta gamma.</p></section>"
        )
        .unwrap();
    }
    html.push_str("</body></html>");
    html
}

/// Parse and cascade once, outside stage measurements.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
pub fn parse(html: &str, resources: &RenderResources<'_>) -> HtmlDocument {
    parse_html_with_resources(html.as_bytes(), resources).expect("the fixture parses and cascades")
}

/// Fresh DOM and cascade with fonts installed for the direct layout entry point.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
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
// cov:ignore: bench-only item; workspace tests do not instrument this target.
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
        Shape::Flex | Shape::FlexOof => DisplayValue::Flex,
        Shape::Grid | Shape::GridOof => DisplayValue::Grid,
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
// cov:ignore: bench-only item; workspace tests do not instrument this target.
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
// cov:ignore: bench-only item; workspace tests do not instrument this target.
pub fn check_layout(dom: &raikiri::Document, expected_items: usize) {
    let mut text_nodes = 0;
    for id in 0..dom.node_count() {
        let node = dom.get_node(id).expect("arena node");
        if matches!(node.tag_name(), Some("p" | "td" | "div")) {
            assert!(node.layout().size.width > 0.0, "element {id} has no width");
            assert!(
                node.layout().size.height > 0.0,
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

/// Verify the laid-out document of `shape`: every text node has lines and every
/// visible element has geometry and, for the out-of-flow shapes, every
/// positioned box was placed.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
pub fn check_document(dom: &raikiri::Document, shape: Shape, items: usize) {
    check_layout(dom, shape.text_items(items));
    if matches!(shape, Shape::FlexOof | Shape::GridOof) {
        check_oof_layout(dom, items - shape.text_items(items));
    }
}

/// Verify every out-of-flow box was laid out against its containing block: it
/// has the size its style gives it, and every kind of placement (`a` insets
/// from the top left, `b` insets from the bottom right, `c` the static
/// position, `f` fixed) moved its boxes away from the origin.
// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn check_oof_layout(dom: &raikiri::Document, expected: usize) {
    const KINDS: [&str; 4] = ["a", "b", "c", "f"];
    let mut found = [0usize; 4];
    let mut placed = [0usize; 4];
    for id in 0..dom.node_count() {
        let node = dom.get_node(id).expect("arena node");
        let Some(class) = node.attribute("class") else {
            continue;
        };
        let mut names = class.split_whitespace();
        if names.next() != Some("oof") {
            continue;
        }
        let placement = names.next();
        let kind = KINDS
            .iter()
            .position(|kind| Some(*kind) == placement)
            .expect("an out-of-flow element names its placement");
        found[kind] += 1;
        let layout = node.layout();
        assert_eq!(
            (layout.size.width, layout.size.height),
            (20.0, 10.0),
            "out-of-flow element {id} was not sized by its style"
        );
        if layout.location.x != 0.0 || layout.location.y != 0.0 {
            placed[kind] += 1;
        }
    }
    assert_eq!(
        found.iter().sum::<usize>(),
        expected,
        "out-of-flow elements in the document"
    );
    for (kind, name) in KINDS.iter().enumerate() {
        assert!(found[kind] > 0, "no out-of-flow element placed as {name}");
        assert_eq!(
            placed[kind], found[kind],
            "out-of-flow elements placed as {name} left at the origin"
        );
    }
}
