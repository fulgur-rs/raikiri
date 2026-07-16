//! DOM walker — Document arena を DFS で walk し PaintScene に emit する。
//!
//! `paint_document` / `paint_element` / `paint_node` の 3 関数分離は
//! separation of concerns — paint_element = element 描画責任 (背景/border/
//! children walk、M4 で肉付け)、paint_node = kind dispatch のみ。M3 で inline
//! formatting context を実装する時に paint_element を変えずに paint_node の
//! Text 分岐だけ unreachable 化できる。
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

/// Document arena を body から DFS walk する。fragment (no `<body>`) case は
/// silent return (m1.6 layout_single_page が Err を返すので paint 呼び出し
/// 前に検出済のはず、defensive)。
pub(crate) fn paint_document(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
) {
    let Some(body_id) = find_body(document) else {
        return;
    };
    paint_element(scene, document, cascade, body_id, 0.0, 0.0);
}

/// Element node の描画。M1.7 では background site を no-op で置く + children
/// を document order で walk。M4 で element background-color / border /
/// box-shadow の描画をここに追加。zero-size subtree は skip
/// (display:none 相当、taffy が LayoutOutput::HIDDEN で size=0 を出す)。
pub(crate) fn paint_element(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    parent_abs_x: f32,
    parent_abs_y: f32,
) {
    let Some(node) = document.get_node(node_id) else {
        return;
    };
    let layout = node.unrounded_layout;
    if layout.size.width <= 0.0 || layout.size.height <= 0.0 {
        return;
    }
    let abs_x = parent_abs_x + layout.location.x;
    let abs_y = parent_abs_y + layout.location.y;
    // paint_element_background(scene, node, abs_x, abs_y, cascade) — M4 でここに挿入
    for &child in &node.children {
        paint_node(scene, document, cascade, child, abs_x, abs_y);
    }
}

/// Kind dispatch。M1.7 は Element / Text の 2 分岐、Document は通常通らない。
/// M3 で AnonymousBlock (inline wrapper) 等が加わる、M3 以降で inline formatting
/// context を実装する時は Text 分岐が unreachable 化する予定。
pub(crate) fn paint_node(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    parent_abs_x: f32,
    parent_abs_y: f32,
) {
    let Some(node) = document.get_node(node_id) else {
        return;
    };
    match node.kind {
        NodeKind::Element => {
            paint_element(scene, document, cascade, node_id, parent_abs_x, parent_abs_y)
        }
        NodeKind::Text => {
            let layout = node.unrounded_layout;
            let abs_x = parent_abs_x + layout.location.x;
            let abs_y = parent_abs_y + layout.location.y;
            text::draw_text_node(scene, node, cascade, node_id, abs_x, abs_y);
        }
        NodeKind::Document => {
            // paint_document が body から start するので通常来ない。
            // Document node は children を持ちうる (未 attach <html>) が M1.7 では扱わない。
        }
        _ => {
            // NodeKind is #[non_exhaustive] (M4+ で Comment/CDATA 等が追加され得る)。
            // M1.7 では未知 kind は no-op。
        }
    }
}

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// 深 DOM で stack overflow を回避)。fragment parse (no `<body>`) では `None`。
fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if node.kind == NodeKind::Element && node.tag_name.as_deref() == Some("body") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}
