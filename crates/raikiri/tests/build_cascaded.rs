//! raikiri umbrella integration tests (raikiri-spike-m1.23)。
//!
//! Consumer が `use raikiri::…;` のみで parse → build_cascaded → display 判定を
//! 完結できることを verify する。

use raikiri::{
    DisplayValue, Dom, Element, Node, NodeId, NodeKind, ParseOptions, build_cascaded, parse,
};

fn parse_html(source: &str) -> raikiri::UncascadedDocument {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    parse(source.as_bytes(), &opts).expect("parse")
}

/// DOM を root から DFS walk して最初に見つかった tag 一致の Element の NodeId を返す。
fn find_by_tag<D: Dom>(dom: &D, tag: &str) -> Option<NodeId> {
    fn walk<D: Dom>(dom: &D, id: NodeId, tag: &str) -> Option<NodeId> {
        if let Some(node) = dom.node(id)
            && node.kind() == NodeKind::Element
            && let Some(elem) = node.as_element()
            && elem.tag_name().eq_ignore_ascii_case(tag)
        {
            return Some(id);
        }
        for child_id in dom.child_ids(id) {
            if let Some(found) = walk(dom, child_id, tag) {
                return Some(found);
            }
        }
        None
    }
    walk(dom, dom.root_id(), tag)
}

#[test]
fn p_without_author_style_is_display_block_via_ua_css() {
    let doc = parse_html("<html><body><p>Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<p> should inherit display: block from bundled UA CSS via build_cascaded",
    );
}

#[test]
fn author_inline_style_overrides_ua_display_block() {
    // NB: m1.4 cascade は class/id selector を drop するので inline style を使う
    let doc = parse_html("<html><body><p style=\"display:inline\">Hi</p></body></html>");
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "author inline style (Normal Author) should override UA (Normal UA) per cascade rank",
    );
}

#[test]
fn dom_style_element_author_rule_overrides_ua() {
    // 明示的な <style> Author rule が UA を上回ることを verify。
    // m1.4 cascade は type selector のみサポートするため p{...} を使う。
    let html = "<html><head><style>p { display: inline }</style></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Inline,
        "DOM <style> Author rule should override UA (both via umbrella wiring)",
    );
}

#[test]
fn extra_stylesheets_author_rule_overrides_ua_via_umbrella() {
    // Consumer が opts.extra_stylesheets 経由で渡した CSS が Author として
    // build_cascaded 経路に到達することを verify (parse 時 Document.stylesheets
    // に Author として push される)。
    let extra: &[&str] = &["p { display: inline }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(display, DisplayValue::Inline);
}
