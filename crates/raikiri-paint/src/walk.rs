//! DOM walker — Document arena を DFS で walk し PaintScene に emit する。
//!
//! `paint_document` は iterative Vec<(node_id, parent_abs_x, parent_abs_y)>
//! stack で walk する (roborev job 223 finding 対応、cascade / m1.6 find_body
//! の pattern と一貫、深 DOM で stack overflow 回避)。kind 分岐は loop 内で
//! inline に行い、Element は children を push、Text は draw_text_node を call、
//! display:none は subtree ごと skip する。
//!
//! M3 で inline formatting context を実装する時は、Element 分岐内の children
//! push を "self の inline layout を walk する" に置き換え、Text 分岐は
//! unreachable 化する予定 (m1.6 の text_layout 選択が M1 限定妥協のため)。
//!
//! find_body は raikiri-dom::layout::find_body と重複するが、5 行の helper
//! を crate 境界越境で pub 化するよりも paint 側で持つ方が clean。

use anyrender::PaintScene;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeKind, PageBox};

use crate::text;

/// Canvas 背景 fill site。M1.4 では cascade に background-color 無しで実質 no-op、
/// future-proof pin として存在。M4 で CSS Backgrounds L3 §2.11.2 "canvas
/// propagation" (html の background-color を取得、TRANSPARENT なら body に
/// fallback、Some なら PageBox 全域を fill) を実装する。
pub(crate) fn paint_canvas_background(
    _scene: &mut impl PaintScene,
    _document: &Document,
    _cascade: &CascadeResult,
    _page_box: PageBox,
) {
    // M1.4: no-op site。M4 で発火。
}

/// Document arena を body から iterative DFS で walk する。fragment (no `<body>`)
/// case は silent return (m1.6 layout_single_page が Err を返すので paint
/// 呼び出し前に検出済のはず、defensive)。
///
/// Stack frame = `(node_id, parent_abs_x, parent_abs_y)`。children は
/// `.rev()` で push し、pop 時に document order で処理する。Element の場合は
/// `is_display_none` を先に判定し true なら subtree ごと skip (roborev job 223
/// finding 対応: 従来の size == 0 判定は overflow: visible な legitimate zero-
/// size 要素も silent drop するため誤り)。
///
/// M4 で element background-color / border / box-shadow を Element arm 内で
/// 描画する予定 (site だけ確保)。
pub(crate) fn paint_document(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
) {
    let Some(body_id) = find_body(document) else {
        return;
    };

    let mut stack: Vec<(usize, f32, f32)> = vec![(body_id, 0.0, 0.0)];
    while let Some((node_id, parent_abs_x, parent_abs_y)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        // raikiri-spike-37c: template 子孫 + 将来の inert subtree を統一 skip。
        // UA CSS の display:none rule 有無に依存しない、明示的な gate。
        if !node.is_in_document() {
            continue;
        }
        // raikiri-spike-d9y.5 (SEC MED, Codex finding): HTML の
        // metadata / raw-text content elements (<head>/<title>/<meta>/<link>/
        // <base>/<noscript>/<script>/<style>/<template>) は subtree ごと
        // 描画対象外。UA CSS `display: none` は author / user CSS で
        // override 可能なため cascade-independent な defense-in-depth gate と
        // して paint 側で fail-close する (HTML LS §15.4.1 "Elements that are
        // not rendered" / CSS 2.1 App.D 準拠、namespace check で SVG /
        // MathML の同名 element は除外)。<template> は is_in_document 側と
        // 二重 gate。
        if node.is_non_rendered_html_element() {
            continue;
        }
        match node.kind() {
            NodeKind::Element => {
                if node.is_display_none() {
                    continue;
                }
                let layout = node.unrounded_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                // paint_element_background(scene, node, abs_x, abs_y, cascade) — M4 でここに挿入
                // children を reverse push すると pop 時に document order で処理される。
                for &child in node.children.iter().rev() {
                    stack.push((child, abs_x, abs_y));
                }
            }
            NodeKind::Text => {
                let layout = node.unrounded_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                text::draw_text_node(scene, node, cascade, node_id, abs_x, abs_y);
            }
            NodeKind::Document => {
                // paint_document が body から start するので通常来ない。
                // Document node は children を持ちうる (未 attach <html>) が
                // M1.7 では扱わない。defensive: subtree を skip。
            }
            _ => {
                // NodeKind is #[non_exhaustive] (M4+ で Comment/CDATA 等が追加され得る)。
                // M1.7 では未知 kind は subtree ごと skip。
            }
        }
    }
}

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// 深 DOM で stack overflow を回避)。fragment parse (no `<body>`) では `None`。
///
/// raikiri-spike-37c roborev job 295 M3 finding: `!is_in_document()` の
/// subtree (`<template>` descendants など) を skip する。inert subtree 内の
/// hypothetical `<body>` を選ばないため。paint 側の find_body と layout 側の
/// find_body は独立実装 (crate 境界越境コスト回避)、同じ contract を持つ。
fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}
