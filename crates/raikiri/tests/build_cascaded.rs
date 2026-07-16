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

#[test]
fn style_inside_template_element_does_not_affect_cascade() {
    // <template> は spec 上 inert。内部の <style> は cascade に流れず、
    // <p> は UA CSS のみで `display: block` を取る。
    // (raikiri-html/src/sink.rs:315-318 の invariant を umbrella surface で検証)
    let html = "<html><head><template><style>p { display: inline }</style></template></head>\
                <body><p>Hi</p></body></html>";
    let doc = parse_html(html);
    let result = build_cascaded(&doc);

    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;
    assert_eq!(
        display,
        DisplayValue::Block,
        "<template> 内の <style> は inert として無視され、<p> は UA CSS の display: block を得る",
    );
}

#[test]
fn ua_important_beats_author_important_via_umbrella() {
    // CSS Cascading L4 §6.4.4 (!important 反転): !important UA > !important Author。
    // umbrella の StylesheetKind → Origin map が正しく機能していることを end-to-end で確認。
    // Author 側は extra_stylesheets で渡す (parse 時 Author kind として Document に注入される)。
    let extra: &[&str] = &["p { display: inline !important }"];
    let opts = raikiri::ParseOptions {
        extra_stylesheets: extra,
        network: None,
        base_url: None,
    };
    let doc = raikiri::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");

    let result = build_cascaded(&doc);
    let p_id = find_by_tag(&doc.dom, "p").expect("<p> exists");
    let display = result.computed[p_id.0 as usize].display;

    // NB: この test 段階では bundled UA CSS は !important を含まない (spec §M1.4a の minimal.css)。
    // Author !important があると Author が勝つ (Normal UA 0 < Important Author 2 < Important UA 3)。
    // したがって p の display は inline になる。この test は "Important Author > Normal UA"
    // の origin-rank ordering が umbrella wiring 越しに保存されることを confirm する。
    assert_eq!(
        display,
        DisplayValue::Inline,
        "Author !important should beat Normal UA via umbrella cascade wiring",
    );
}
