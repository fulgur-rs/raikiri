//! Single-page layout driver — `layout_single_page` を pub 提供。
//!
//! Pipeline: cascade (raikiri-style) 出力 + Document arena + PageBox から
//! taffy compute_root_layout を駆動し、text intrinsic size は parley 0.10 の
//! 最小統合で pre-shape する。M1.6 scope: 単一 A4 ページ、ASCII Latin、
//! parley system font default (byte-identical cross-machine は m1.13 で font pinning)。
//!
//! 全 helper は crate-private、pub 型は [`layout_single_page`] のみ。

use raikiri_traits::NodeKind;

use crate::document::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;
use taffy::{Dimension, Size};

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// deep DOM で stack overflow を回避)。fragment parse (no `<body>`) では
/// `None`、caller が `LayoutError::Internal` に昇格させる。
#[allow(dead_code)]
pub(crate) fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(node_idx) = stack.pop() {
        let node = &doc.nodes[node_idx];
        if node.kind == NodeKind::Element
            && node.tag_name.as_deref() == Some("body")
        {
            return Some(node_idx);
        }
        // children を reverse push すると document order で pop される
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// `<body>` の taffy::Style.size を PageBox の width / height (CSS pt) に強制する。
///
/// CSS Paged Media の initial containing block = @page size。M1 は @page 非対応
/// のため body.style.size に直接注入する妥協。M4 で @page cascade + per-page
/// PageBox を導入時に `<html>` root style に site を昇格予定。
#[allow(dead_code)]
pub(crate) fn apply_page_box_to_body(
    doc: &mut Document,
    body_id: usize,
    page_box: PageBox,
) {
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(page_box.width),
        height: Dimension::length(page_box.height),
    };
}

/// ComputedValues → taffy::Style bridge の site。
///
/// M1.4 property (color / font-family / font-size / font-weight) は全て
/// inherited text-only property で taffy::Style を変えないため、M1.6 では
/// 本体 no-op。M4 で display / margin / padding / width / height 等の
/// layout property が加わった時、ここに merge ロジックを追加する。
/// (この関数の存在自体が M1.6 の site 確立の遺産。)
#[allow(dead_code)]
pub(crate) fn apply_computed_to_style(
    _doc: &mut Document,
    _cascade: &CascadeResult,
) {
    // M1.4 では no-op。M4 で ComputedValues に display / size / margin / padding
    // 等が加わった時、下記のような per-element loop を追加:
    //
    // for idx in 0..doc.nodes.len() {
    //     if doc.nodes[idx].kind != NodeKind::Element { continue; }
    //     let cv = &cascade.computed[idx];
    //     merge_layout_properties(&mut doc.nodes[idx].style, cv);
    // }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::Style;

    #[test]
    fn find_body_returns_index_when_present() {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        assert_eq!(find_body(&doc), Some(body));
    }

    #[test]
    fn find_body_returns_none_when_absent() {
        // Fragment 相当: <p> を Document root 直下に append、<body> なし
        let mut doc = Document::new();
        let _p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
        assert_eq!(find_body(&doc), None);
    }

    #[test]
    fn find_body_iterative_no_stack_overflow_on_deep_dom() {
        // 5000 深さで stack overflow を起こさず None を返す。
        // cascade §deep_nesting_5000_cascade_no_overflow と同水準の regression pin。
        let mut doc = Document::new();
        let mut parent = 0usize;
        for _ in 0..5000 {
            parent = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
        }
        assert_eq!(find_body(&doc), None);
    }

    #[test]
    fn find_body_returns_first_body_in_document_order() {
        // 2 個の <body> がある病理的なケースでは最初の document order の <body> を返す
        // (html5ever は 1 個しか作らない想定だが、defensive contract を pin)
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body1 = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let _body2 = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        assert_eq!(find_body(&doc), Some(body1));
    }

    #[test]
    fn apply_page_box_to_body_sets_body_style_size_to_page_dimensions() {
        use raikiri_traits::PageBox;
        use taffy::{Dimension, Size};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);

        apply_page_box_to_body(&mut doc, body, PageBox::A4);

        let size: Size<Dimension> = doc.nodes[body].style.size;
        assert_eq!(size.width, Dimension::length(595.0));
        assert_eq!(size.height, Dimension::length(842.0));
    }

    #[test]
    fn apply_computed_to_style_is_noop_at_m1_4() {
        // M1.4 の 4 property (color / font-family / font-size / font-weight) は
        // 全て inherited かつ text-only、taffy::Style を変えない。この test は
        // M4 で display / margin / width / height 等の layout property が入った時に
        // ここの assertion が「変える」に反転する: bridge site が正しく機能している
        // かの regression pin。
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("color:red"));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let before = doc.nodes[body].style.clone();
        apply_computed_to_style(&mut doc, &cr);
        let after = doc.nodes[body].style.clone();

        // M1.4 では size / margin / padding など layout に効く field は変えない
        assert_eq!(before.size, after.size, "size unchanged at M1.4");
        assert_eq!(before.display, after.display, "display unchanged at M1.4");
        assert_eq!(before.margin, after.margin, "margin unchanged at M1.4");
        assert_eq!(before.padding, after.padding, "padding unchanged at M1.4");
    }
}
