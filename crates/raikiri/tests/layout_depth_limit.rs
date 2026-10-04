//! Public parse-plus-layout regressions for bounded, stack-safe layout depth.

use raikiri::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageBox, PageDefaults, layout, parse_html,
};

fn nested_html(layout_depth: usize, display: &str, named_leaf: bool) -> String {
    let mut html = String::from("<html><body>");
    for depth in 1..layout_depth {
        let page = if named_leaf && depth == layout_depth - 1 {
            ";page:named"
        } else {
            ""
        };
        if display == "flex" {
            html.push_str(&format!("<div style=\"display:flex{page}\">"));
        } else if !page.is_empty() {
            html.push_str("<div style=\"page:named\">");
        } else {
            html.push_str("<div>");
        }
    }
    html.push('x');
    for _ in 1..layout_depth {
        html.push_str("</div>");
    }
    html.push_str("</body></html>");
    html
}

fn public_layout(layout_depth: usize, display: &str) -> Result<LayoutStatus, raikiri::RenderError> {
    public_layout_with_named_leaf(layout_depth, display, false)
}

fn public_layout_with_named_leaf(
    layout_depth: usize,
    display: &str,
    named_leaf: bool,
) -> Result<LayoutStatus, raikiri::RenderError> {
    let html = nested_html(layout_depth, display, named_leaf);
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = parse_html(html.as_bytes(), &options).expect("deep document parses");
    let mut defaults = PageDefaults::default();
    defaults.page_box = PageBox::new();
    layout(
        &document,
        defaults,
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
}

#[test]
fn public_layout_accepts_block_tree_at_depth_256() {
    assert!(
        matches!(public_layout(256, "block"), Ok(LayoutStatus::Completed(_))),
        "a block tree at the supported depth must complete layout"
    );
}

#[test]
fn public_layout_accepts_flex_tree_at_depth_256() {
    assert!(
        matches!(public_layout(256, "flex"), Ok(LayoutStatus::Completed(_))),
        "a flex tree at the supported depth must complete layout"
    );
}

#[test]
fn public_layout_accepts_named_flex_tree_at_depth_256() {
    assert!(matches!(
        public_layout_with_named_leaf(256, "flex", true),
        Ok(LayoutStatus::Completed(_))
    ));
}

#[test]
fn public_layout_rejects_block_tree_at_depth_257_with_typed_error() {
    assert!(matches!(
        public_layout(257, "block"),
        Err(raikiri::RenderError::Layout(
            raikiri::LayoutError::TreeDepthLimitExceeded {
                limit: 256,
                actual: 257,
            }
        ))
    ));
}

#[test]
fn public_layout_rejects_flex_tree_at_depth_257_with_typed_error() {
    assert!(matches!(
        public_layout(257, "flex"),
        Err(raikiri::RenderError::Layout(
            raikiri::LayoutError::TreeDepthLimitExceeded {
                limit: 256,
                actual: 257,
            }
        ))
    ));
}
