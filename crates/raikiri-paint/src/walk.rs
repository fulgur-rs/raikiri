//! DOM walker — Document arena を DFS で walk し PaintScene に emit する。
//!
//! `paint_document` は iterative
//! `Vec<(node_id, parent_abs_x, parent_abs_y, parent_font_size, shift_y)>`
//! stack で walk する (cascade / find_body の pattern と一貫、深 DOM で
//! stack overflow 回避)。kind 分岐は loop 内で inline に行い、Element は
//! children を push、Text は draw_text_node を call、display:none は
//! subtree ごと skip する。`parent_font_size` / `shift_y` は
//! `vertical_align_shift_px` doc 参照。
//!
//! 将来 inline formatting context を実装する時は、Element 分岐内の children
//! push を "self の inline layout を walk する" に置き換え、Text 分岐は
//! unreachable 化する予定 (現状の text_layout 選択は暫定的な妥協のため)。
//!
//! **この inline formatting context の不在は `vertical_align_shift_px` の
//! shift 適用でも未解決のまま残る** — `display: inline` の要素も現状は
//! taffy 上で他の block 要素と同じ独立した行として積み上がる
//! (`bridge_display` が `DisplayValue::Inline` を `taffy::Display::Block` に
//! 写す)。したがって `<p>H<sub>2</sub>O</p>` の "H" / "2" / "O" は現状でも
//! 3 行に分かれたまま描画される — `vertical_align_shift_px` が加える
//! offset は「その独立した行の中で `2` をわずかに動かす」だけであり、
//! `2` を `H`/`O` と同じ行に呼び戻すものではない。この shift は taffy が
//! box 位置を確定させた**後**、paint 時にのみ加算される (taffy 自体は
//! `vertical_align` を一切見ない) ため box-model 計算には一切参加せず、
//! どこにもクリップされない — 例えば page 最上部近くの `vertical-align:
//! super` は margin 領域へはみ出して描画されうる。
//!
//! find_body は raikiri-dom::layout::find_body と重複するが、5 行の helper
//! を crate 境界越境で pub 化するよりも paint 側で持つ方が clean。

use anyrender::PaintScene;
use kurbo::Rect;
use peniko::{Color, Fill};
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_style::property::{CssColor, DisplayValue, VerticalAlign};
use raikiri_traits::{NodeKind, PageBox};

use crate::text;

/// Canvas 背景 fill site。現状は cascade に background-color が無く実質
/// no-op で、future-proof pin として存在。将来 CSS Backgrounds L3 §2.11.2
/// "canvas propagation" (html の background-color を取得、TRANSPARENT なら
/// body に fallback、Some なら PageBox 全域を fill) を実装する予定。
pub(crate) fn paint_canvas_background(
    scene: &mut impl PaintScene,
    _document: &Document,
    _cascade: &CascadeResult,
    page_box: PageBox,
) {
    // Minimal canvas background: fill page white (UA default) so transparent areas are not mismatched.
    // Full CSS Backgrounds §2.11 canvas propagation (html/body) is future work; this ensures page is white.
    let color = peniko::Color::from_rgba8(255, 255, 255, 255);
    let rect = kurbo::Rect::new(0.0, 0.0, page_box.width as f64, page_box.height as f64);
    scene.fill(
        peniko::Fill::NonZero,
        kurbo::Affine::IDENTITY,
        color,
        None,
        &rect,
    );
}

/// Document arena を body から iterative DFS で walk する。fragment (no `<body>`)
/// case は silent return (layout_single_page が Err を返すので paint
/// 呼び出し前に検出済のはず、defensive)。
///
/// Stack frame = `(node_id, parent_abs_x, parent_abs_y, parent_font_size,
/// shift_y)`。children は `.rev()` で push し、pop 時に document order で
/// 処理する。Element の場合は `is_display_none` を先に判定し true なら
/// subtree ごと skip (旧来の size == 0 判定は overflow: visible な
/// legitimate zero-size 要素も silent drop するため誤りだったための対応)。
///
/// `parent_font_size` はこの stack frame の node の**親**の used font-size
/// (px)。`shift_y` はこの node に至るまでの祖先全体が積んだ
/// `vertical-align` shift の累計 (px、down 方向が正)。両方とも
/// `vertical_align_shift_px` の入力・出力に対応する — 詳細はその doc 参照。
///
/// 将来 element background-color / border / box-shadow を Element arm 内で
/// 描画する予定 (site だけ確保)。
pub(crate) fn paint_document(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
) {
    let Some(body_id) = find_body(document) else {
        return;
    };

    // body 自身の親 (`<html>`) の font-size は stack と独立した traversal
    // (`find_body`) でしか到達できないため、self-referential に body 自身の
    // font-size を代わりに使う。body の UA default display は block なので
    // `vertical_align_shift_px` の inline-level gate が常にこの値を無視する
    // — body に `display: inline` を override するような病的な入力でない
    // 限り、この fallback の精度は実質無関係。
    let body_font_size = cascade.computed[body_id].font_size.px();
    let mut stack: Vec<(usize, f32, f32, f32, f32)> =
        vec![(body_id, 0.0, 0.0, body_font_size, 0.0)];
    while let Some((node_id, parent_abs_x, parent_abs_y, parent_font_size, shift_y)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        // template 子孫 + 将来の inert subtree を統一 skip。
        // UA CSS の display:none rule 有無に依存しない、明示的な gate。
        if !node.is_in_document() {
            continue;
        }
        // HTML の hidden elements (metadata / raw-text content / ruby
        // parenthesis fallback) は subtree ごと描画対象外。
        // 現在の対象 tag 集合は `Node::is_non_rendered_html_element` の
        // match arms を single source of truth とする。
        // UA CSS `display: none` は author / user CSS で override 可能なため
        // cascade-independent な defense-in-depth gate として paint 側で
        // fail-close する (HTML LS §15.3.1 "Hidden elements"、
        // https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements
        // 準拠、namespace check で SVG / MathML の同名 element は除外)。
        // <template> は is_in_document 側と二重 gate。
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
                let cv = &cascade.computed[node_id];
                let own_shift =
                    vertical_align_shift_px(cv.vertical_align, cv.display, parent_font_size);
                let child_shift_y = shift_y + own_shift;
                // Paint element background (CSS Backgrounds 3 §2.2). Shift applies to the box itself
                // per CSS 2.1 §10.8.1, so use `child_shift_y` not `abs_y`.
                paint_element_background(
                    scene,
                    layout.size.width,
                    layout.size.height,
                    abs_x,
                    abs_y + child_shift_y,
                    cv.background_color,
                    cv.background_clip,
                    &cv.border,
                    &cv.padding,
                );
                let child_font_size = cv.font_size.px();
                // children を reverse push すると pop 時に document order で処理される。
                for &child in node.children.iter().rev() {
                    stack.push((child, abs_x, abs_y, child_font_size, child_shift_y));
                }
            }
            NodeKind::Text => {
                let layout = node.unrounded_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                text::draw_text_node(scene, node, cascade, node_id, abs_x, abs_y + shift_y);
            }
            NodeKind::Document => {
                // paint_document が body から start するので通常来ない。
                // Document node は children を持ちうる (未 attach <html>) が
                // 現状は扱わない。defensive: subtree を skip。
            }
            _ => {
                // NodeKind is #[non_exhaustive]: `Comment` / `ProcessingInstruction` /
                // `DocumentFragment` はここに落ちる (paint 対象外)。実際には
                // mark_in_document_flags が Comment/PI の IS_IN_DOCUMENT bit を
                // clear しているため、この walker 到達前段の is_in_document()
                // gate で先に filter されることが expected — defense-in-depth の
                // 第 2 gate として本 arm を保持 (kind gate と is_in_document gate
                // の両方が failing した場合でも subtree ごと skip)。将来 CDATA /
                // DocumentType 等が追加された場合も同じ扱い。
            }
        }
    }
}

/// `vertical-align` が inline-level box の位置へ寄与する pixel offset。
/// 正の戻り値 = 下方向 (`draw_text_node` の `abs_y` と同じ、Y が下に伸びる
/// 座標系)。
///
/// # 実装範囲
///
/// [`VerticalAlign::Sub`] / [`VerticalAlign::Super`] のみ shift を計算する。
/// `top` / `text-top` / `middle` / `bottom` / `text-bottom` は
/// raikiri-style の parser がそもそも受理しない (silent drop —
/// [`raikiri_style::property::VerticalAlign`] doc の "Scope carving" 節)
/// ためこの関数に届かない。`_` arm はこの関数を total にするための
/// defensive default であり (`VerticalAlign` は `#[non_exhaustive]`)、
/// [`VerticalAlign::Baseline`] も同じ 0 shift になる (CSS 2.1 §10.8.1
/// verbatim: "Align the baseline of the box with the baseline of the
/// parent box" — 追加の shift なし)。
///
/// CSS 2.1 §10.8.1 "Applies to: inline-level and 'table-cell' elements"
/// <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
/// (`table-cell` はこの crate 未実装) — block-level box の
/// `vertical-align: sub` は shift に寄与しない。
///
/// # Shift 量
///
/// CSS 2.1 §10.8.1 自体は `sub`/`super` の offset を "the proper position
/// for subscripts/superscripts" とだけ述べ、量を implementation-defined の
/// ままにする。CSS Inline Layout Module Level 3 §4.2.3 "Post-Alignment
/// Shift: the baseline-shift longhand"
/// <https://www.w3.org/TR/css-inline-3/#baseline-shift-property> が
/// 具体的な UA-default fallback を与える (font metrics 参照はそちらが
/// 優先だが本関数では未実装 — font table を一切読まない):
///
/// - `sub`, spec verbatim: "Lower by the offset appropriate for
///   subscripts of the parent's box. The UA may use the parent's font
///   metrics to find this offset; otherwise it defaults to dropping by
///   one fifth of the parent's used font-size."
/// - `super`, spec verbatim: "Raise by the offset appropriate for
///   superscripts of the parent's box. The UA may use the parent's font
///   metrics to find this offset; otherwise it defaults to raising by one
///   third of the parent's used font-size."
///
/// `vertical-align` (CSS 2.1) と `baseline-shift` (CSS Inline 3) は別
/// property である — [`raikiri_style::property::VerticalAlign`] doc が
/// 説明する通り、本 crate は keyword grammar の primary source として CSS
/// 2.1 を採り続ける。ここで CSS Inline 3 を引くのは、CSS 2.1 が定義しない
/// shift **量**についてのみ、CSS Inline 3 の同じ `sub`/`super` keyword に
/// 対する UA-default fallback 記述を借りるためである。
///
/// `parent_font_size_px` は **box 自身の親の** used font-size でなければ
/// ならない (box 自身の font-size ではない — `sub`/`super` content は通常
/// 既に author/UA の `font-size: smaller` で縮小済みで、上記 spec 文の
/// "the parent's used font-size" はその縮小前の値を指す)。
///
/// # Nested `vertical-align` の合成 (未検証の近似)
///
/// この関数自体は 1 box 分の shift だけを返す。呼び出し側
/// ([`paint_document`]) は祖先ごとの shift を単純加算で累積する
/// (`shift_y` stack frame) — real な inline formatting context 下では
/// 各 box は直接の親の baseline に対して shift し、それが line box
/// 構築を通じて連鎖することの素朴な近似であり、どの primary source にも
/// 明記された規則ではない。
///
/// # Box-model への非参加 (未実装)
///
/// この戻り値は taffy が box 位置を確定させた後、paint 時にのみ加算される
/// — taffy 自身は `vertical_align` を見ないため、shift された結果が
/// どこにもクリップされない。CSS 2.1 / CSS Inline 3 とも shift 後の位置を
/// box-model 計算 (line box の高さ等) に参加させる前提だが、ここでは
/// 参加しない — 極端な shift 量が page box の外へはみ出して描画されうる。
#[allow(clippy::too_many_arguments)]
fn paint_element_background(
    scene: &mut impl PaintScene,
    width: f32,
    height: f32,
    abs_x: f32,
    abs_y: f32,
    bg: CssColor,
    clip: raikiri_style::property::VisualBox,
    border: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedBorder>,
    padding: &raikiri_style::property::Sides<raikiri_style::resolve::ComputedLengthPercentage>,
) {
    if bg.a == 0 || width <= 0.0 || height <= 0.0 {
        return;
    }
    // background-clip: text — clip to text glyphs (CSS Backgrounds 4 §2.6).
    // Requires glyph path clipping which is not yet implemented; treat as no opaque rect.
    // This intentionally leaves coverage gap for text-clip tests (tracked separately).
    if matches!(clip, raikiri_style::property::VisualBox::Text) {
        return;
    }
    // Compute inset rect for background-clip per CSS Backgrounds 3 §2.7:
    // - border-box / border-area: border box (full rect)
    // - padding-box: padding box (inset by border widths)
    // - content-box: content box (inset by border + padding)
    // Width/height is border-box size from taffy layout.
    let (mut x0, mut y0, mut x1, mut y1) = (
        abs_x as f64,
        abs_y as f64,
        (abs_x + width) as f64,
        (abs_y + height) as f64,
    );
    // Helper to get padding px: for Px use directly, for Percent approximate as percent of width
    fn padding_px(v: raikiri_style::resolve::ComputedLengthPercentage, reference: f32) -> f32 {
        match v {
            raikiri_style::resolve::ComputedLengthPercentage::Px(px) => px,
            raikiri_style::resolve::ComputedLengthPercentage::Percent(p) => reference * p / 100.0,
        }
    }
    match clip {
        raikiri_style::property::VisualBox::PaddingBox => {
            let bw = border;
            x0 += bw.left.width().px() as f64;
            y0 += bw.top.width().px() as f64;
            x1 -= bw.right.width().px() as f64;
            y1 -= bw.bottom.width().px() as f64;
        }
        raikiri_style::property::VisualBox::ContentBox => {
            let bw = border;
            // border inset
            let bl = bw.left.width().px();
            let bt = bw.top.width().px();
            let br = bw.right.width().px();
            let bb = bw.bottom.width().px();
            // padding inset (handle Px/Percent)
            let pl = padding_px(padding.left, width);
            let pt = padding_px(padding.top, width);
            let pr = padding_px(padding.right, width);
            let pb = padding_px(padding.bottom, width);
            x0 += (bl + pl) as f64;
            y0 += (bt + pt) as f64;
            x1 -= (br + pr) as f64;
            y1 -= (bb + pb) as f64;
        }
        // BorderBox, BorderArea: no inset
        _ => {}
    }
    // Guard against negative or inverted rect after inset (e.g. border larger than box)
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let color = Color::from_rgba8(bg.r, bg.g, bg.b, bg.a);
    let rect = Rect::new(x0, y0, x1, y1);
    scene.fill(Fill::NonZero, kurbo::Affine::IDENTITY, color, None, &rect);
}

fn vertical_align_shift_px(
    va: VerticalAlign,
    display: DisplayValue,
    parent_font_size_px: f32,
) -> f32 {
    if !matches!(display, DisplayValue::Inline | DisplayValue::InlineBlock) {
        return 0.0;
    }
    match va {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        // `VerticalAlign` は `#[non_exhaustive]` — この関数を total に
        // するための defensive default で、今日は `Baseline` だけがここへ
        // 落ちる (0 shift、spec 通り)。将来 `VerticalAlign` に新しい
        // keyword が加われば、raikiri-style 側で明示的に shift 計算が
        // 実装されるまでこの arm がその keyword を黙って 0 shift にする
        // — `raikiri_style::property::VerticalAlign` doc の "Scope
        // carving" 節が parse 層で戒めている「実装が追いつくまで受理し
        // ない」規律を、この consumption 側では compile time に強制でき
        // ない。新しい variant を追加する際は、まずここを明示的な match
        // arm にすること。
        _ => 0.0,
    }
}

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// 深 DOM で stack overflow を回避)。fragment parse (no `<body>`) では `None`。
///
/// `!is_in_document()` の subtree (`<template>` descendants など) を skip
/// する。inert subtree 内の hypothetical `<body>` を選ばないため。paint 側の
/// find_body と layout 側の
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
