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
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontWeight, Layout, LayoutContext,
    StyleProperty,
};
use raikiri_style::CascadeResult;
use raikiri_style::ComputedValues;
use raikiri_style::property::{
    Border, BorderStyle, BoxSizing as StyleBoxSizing, DisplayValue, Length, LengthOrAuto,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AvailableSpace, BoxSizing as TaffyBoxSizing, Dimension, Display, LengthPercentage,
    LengthPercentageAuto, NodeId as TaffyNodeId, Rect, Size, compute_root_layout,
};

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// deep DOM で stack overflow を回避)。fragment parse (no `<body>`) では
/// `None`、caller が `LayoutError::Internal` に昇格させる。
///
/// raikiri-spike-37c roborev job 295 M3 finding: `!is_in_document()` の subtree
/// (`<template>` descendants など) を skip する。inert subtree 内に `<body>`
/// tag があってもそれを本物の body として選ばないため — 例えば
/// `<template><body>ghost</body></template>` の後に real `<body>` が来る HTML
/// で ghost body を選んでしまうと後続の layout / paint が inert subtree に対して
/// 実行されてしまう。
pub(crate) fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(node_idx) = stack.pop() {
        let node = &doc.nodes[node_idx];
        if !node.is_in_document() {
            // inert subtree — 本 subtree の中に body があっても選ばない。
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(node_idx);
        }
        // children を reverse push すると document order で pop される
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// `<body>` の taffy::Style.size を PageBox の width / height (CSS px) に強制する。
///
/// CSS Paged Media の initial containing block = @page size。M1 は @page 非対応
/// のため body.style.size に直接注入する妥協。M4 で @page cascade + per-page
/// PageBox を導入時に `<html>` root style に site を昇格予定。
pub(crate) fn apply_page_box_to_body(doc: &mut Document, body_id: usize, page_box: PageBox) {
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(page_box.width),
        height: Dimension::length(page_box.height),
    };
}

/// ComputedValues → taffy::Style bridge の dispatch site。
///
/// Sprint 18 (raikiri-spike-j5rz) で per-element for loop 内 inline mapping から
/// per-field `bridge_*` helper へ dispatch する pattern に refactor
/// (advisor #4 first-merged refactor scaffold)。Wave 2+ で bridge_padding /
/// bridge_size / bridge_border / bridge_box_sizing を helper add + dispatch 1 行
/// 追加 のみで conflict 密度最小化。
///
/// 現時点で active な bridge:
/// - [`bridge_display`] — [`DisplayValue`] → [`taffy::Display`] (Sprint 13
///   w2s で initial landing)
/// - [`bridge_margin`] — [`Sides<LengthOrAuto>`] → [`taffy::Rect<LengthPercentageAuto>`]
///   (raikiri-spike-j5rz)
/// - [`bridge_padding`] — [`Sides<Length>`] → [`taffy::Rect<LengthPercentage>`]
///   (raikiri-spike-jbu0)
/// - [`bridge_size`] — [`LengthOrAuto`] `cv.width` → [`taffy::Style::size`]`.width`
///   のみ (raikiri-spike-ggig Wave 2 scaffold)。`.height` は raikiri-spike-01up
///   (Wave 3) が同 helper を拡張して書き込むため本 landing では touch しない。
/// - [`bridge_border`] — [`Sides<Border>`] → [`taffy::Rect<LengthPercentage>`]
///   with border-style gating (raikiri-spike-q0uc Wave 2, advisor #2 CSS Backgrounds 3 §5.2)
/// - [`bridge_box_sizing`] — [`raikiri_style::BoxSizing`] → [`taffy::BoxSizing`]
///   (raikiri-spike-o11x Wave 2, CSS Sizing 3 §7)
pub(crate) fn apply_computed_to_style(doc: &mut Document, cascade: &CascadeResult) {
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let style = &mut doc.nodes[idx].style;
        bridge_display(style, cv);
        bridge_margin(style, cv);
        bridge_padding(style, cv);
        bridge_size(style, cv);
        bridge_border(style, cv);
        bridge_box_sizing(style, cv);
    }
}

/// [`DisplayValue`] → [`taffy::Display`] mapping。
///
/// raikiri-spike-0vv.4 (Sprint 12) で追加された [`DisplayValue`] を
/// [`taffy::Display`] に mapping する。taffy 0.x は Block / Flex / Grid /
/// None のみ (`Inline` / `InlineBlock` 独立 variant なし) のため:
/// - `Inline` → `Block` (initial は Block、text-only は leaf で render)
/// - `InlineBlock` → `Block` (block child + inline-level flow parent の
///   separate 扱い、精密化は follow-up)
/// - `None` → `None`
/// - catch-all arm → `Block` (`non_exhaustive` forward-compat)
fn bridge_display(style: &mut taffy::Style, cv: &ComputedValues) {
    style.display = match cv.display {
        DisplayValue::Block => Display::Block,
        DisplayValue::Inline => Display::Block,
        DisplayValue::InlineBlock => Display::Block,
        DisplayValue::None => Display::None,
        _ => {
            // non_exhaustive catch-all — unknown future variant goes to Block
            Display::Block
        }
    };
}

/// [`ComputedValues::margin`] (`Sides<LengthOrAuto>`) → [`taffy::Style::margin`]
/// (`Rect<LengthPercentageAuto>`) bridge。
///
/// CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> の
/// physical margin 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。
///
/// Length policy は [`length_or_auto_to_taffy_lpa`] を参照。
///
/// (raikiri-spike-j5rz Sprint 18 Wave 1)
fn bridge_margin(style: &mut taffy::Style, cv: &ComputedValues) {
    let m = cv.margin;
    style.margin = Rect {
        top: length_or_auto_to_taffy_lpa(m.top),
        right: length_or_auto_to_taffy_lpa(m.right),
        bottom: length_or_auto_to_taffy_lpa(m.bottom),
        left: length_or_auto_to_taffy_lpa(m.left),
    };
}

/// [`ComputedValues::padding`] (`Sides<Length>`) → [`taffy::Style::padding`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// CSS Box 3 §6.1 <https://www.w3.org/TR/css-box-3/#padding-physical> の
/// physical padding 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。margin と
/// の差は value type: padding は `<length-percentage [0,∞]>` (auto なし、
/// non-negative は raikiri-style parse-time enforce) のため
/// [`length_to_taffy_length_percentage`] を使う。
///
/// Length policy は [`length_to_taffy_length_percentage`] を参照。
///
/// (raikiri-spike-jbu0 Sprint 18 Wave 2)
fn bridge_padding(style: &mut taffy::Style, cv: &ComputedValues) {
    let p = cv.padding;
    style.padding = Rect {
        top: length_to_taffy_length_percentage(p.top),
        right: length_to_taffy_length_percentage(p.right),
        bottom: length_to_taffy_length_percentage(p.bottom),
        left: length_to_taffy_length_percentage(p.left),
    };
}

/// [`ComputedValues::border`] (`Sides<Border>`) → [`taffy::Style::border`]
/// (`Rect<LengthPercentage>`) bridge、CSS Backgrounds 3 §5.2 の
/// **style-gating** (used border-width policy) を適用する。
///
/// CSS Backgrounds 3 §5.2 <https://www.w3.org/TR/css-backgrounds-3/#border-style>:
/// > `none` — No border. Color and width are ignored (i.e., the border has
/// > width 0, unless the border is an image, see 'border-image-width').
///
/// `hidden` は §5.2 で "Same as `none`, except in terms of border conflict
/// resolution for table elements." — used border-width も 0。
///
/// したがって border-style が `None` / `Hidden` の側は specified border-width
/// を無視して used border-width = 0 として taffy に渡す ([`used_border_width`]
/// helper)。この gating を怠ると spec 違反 (`5px none red` の 5px が layout に
/// 影響してしまう)。
///
/// 4-side は **field 名 mapping** で write (positional constructor は使わない —
/// [`bridge_margin`] と同じ `Sides` vs `Rect` field 順不一致の silent transpose
/// 防止)。
///
/// # taffy scope の非対応
///
/// - `border-color` / `border-style` 自体は taffy が track しない (taffy は
///   border-width のみ)。色 / 線 pattern は paint scope が別途 [`ComputedValues::border`]
///   から consume する将来 task。
/// - `border-image` / `border-radius` は Sprint 18 スコープ外。
///
/// # Length policy
///
/// [`length_to_taffy_length_percentage`] を reuse (Unified Length policy)。
/// taffy `border` は `LengthPercentage` (no auto、CSS Backgrounds 3 §5.1
/// grammar `<length [0,∞]>` に percentage は含まれないが taffy 型は
/// LengthPercentage が最小共通型)。
///
/// (raikiri-spike-q0uc Sprint 18 Wave 2)
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues) {
    let b = cv.border;
    style.border = Rect {
        top: length_to_taffy_length_percentage(used_border_width(&b.top)),
        right: length_to_taffy_length_percentage(used_border_width(&b.right)),
        bottom: length_to_taffy_length_percentage(used_border_width(&b.bottom)),
        left: length_to_taffy_length_percentage(used_border_width(&b.left)),
    };
}

/// CSS Backgrounds 3 §5.2 "used border-width" policy — style が `None` /
/// `Hidden` なら width を 0 として扱う。
///
/// `matches!` + `#[non_exhaustive]` 対応: 未知未来 variant は else 枝に落ちて
/// specified width を透過 (spec 上「visible なんらかの style」が追加された時
/// にも fail-safe に width が生きる)。
fn used_border_width(b: &Border) -> Length {
    if matches!(b.style, BorderStyle::None | BorderStyle::Hidden) {
        Length::Px(0.0)
    } else {
        b.width
    }
}

/// [`ComputedValues::width`] (`LengthOrAuto`) → [`taffy::Style::size`]`.width`
/// (`Dimension`) bridge (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
///
/// **Wave 2 scaffold (raikiri-spike-ggig): width component のみ書き込む** —
/// `style.size.height` は Wave 3 (raikiri-spike-01up) が本 helper を拡張して
/// 書き込むまで touch しない。partial write (fields を個別 assign) にすること
/// で height 側の default (`Dimension::auto()`) を残しつつ、両 waves 完了後は
/// `style.size = Size { width, height }` の struct literal に refactor 可能。
///
/// Length policy は [`length_or_auto_to_taffy_dimension`] を参照。
///
/// # PageBox 妥協 (M1)
///
/// `<body>` element の `style.size` は本 bridge の後、[`apply_page_box_to_body`]
/// で PageBox の値に上書きされる (layout.rs Step 1 → Step 4)。したがって
/// `<body style="width: 100px">` の author width は本 helper で一度 taffy に
/// write されるが、Step 4 で PageBox width に clobber される — M1 期間中の
/// 意図された挙動 (M4 で @page cascade + per-page PageBox に refactor 予定)。
/// regression pin は tests `apply_page_box_clobbers_body_width_from_bridge` を
/// 参照。
fn bridge_size(style: &mut taffy::Style, cv: &ComputedValues) {
    // Partial write: width のみ。height は Task D (raikiri-spike-01up) で追記。
    // struct literal (`style.size = Size {...}`) を使わず field assign する
    // ことで、Wave 3 マージ前でも default height を破壊しない。
    style.size.width = length_or_auto_to_taffy_dimension(cv.width);
}

/// [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`] bridge。
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`、spec initial `content-box`。**enum 1:1 mapping**
/// (Length policy に不参加、Sprint 18 Wave 2 の最小 helper)。
///
/// # Initial-value 補正 note
///
/// - raikiri-style initial = `BoxSizing::ContentBox` (CSS Sizing 3 §3.3 spec 準拠)
/// - taffy default = `taffy::BoxSizing::BorderBox` (taffy 0.12 の `#[default]`)
///
/// 両者の初期値は spec と食い違うが、cascade は unspecified 時に必ず
/// [`ComputedValues::initial`] 経由で `ContentBox` を seed するため、本 bridge が
/// 走った後の `style.box_sizing` は常に spec 初期値 (`ContentBox`) になる。
/// つまり本 helper の副作用として "taffy default の spec 違反" を補正する。
///
/// # non_exhaustive catch-all
///
/// raikiri-style の [`BoxSizing`] は `#[non_exhaustive]` (37n sibling pattern
/// for forward-compat)。未知 variant は spec initial (`ContentBox`) に
/// fail-quiet — spec-violation を silent に伸ばさないよう "最も安全な既定"
/// にする方針 ([`bridge_display`] catch-all → `Block` と同じ趣旨)。
///
/// taffy 側 (`taffy::BoxSizing`) は `#[non_exhaustive]` **ではない** ため、
/// mapping 出力 arm は `ContentBox` / `BorderBox` の 2 個で網羅済。
///
/// (raikiri-spike-o11x Sprint 18 Wave 2)
///
/// [`BoxSizing`]: raikiri_style::property::BoxSizing
fn bridge_box_sizing(style: &mut taffy::Style, cv: &ComputedValues) {
    style.box_sizing = match cv.box_sizing {
        StyleBoxSizing::ContentBox => TaffyBoxSizing::ContentBox,
        StyleBoxSizing::BorderBox => TaffyBoxSizing::BorderBox,
        // non_exhaustive catch-all — 未知 variant は spec initial (ContentBox)
        // に fail-quiet (silent spec-violation 拡大を避ける)。
        _ => TaffyBoxSizing::ContentBox,
    };
}

/// [`Length`] → [`taffy::LengthPercentage`] bridge (padding / border 用)。
///
/// **taffy 空間 = CSS px** (raikiri-traits/src/page.rs:98 authoritative、
/// PageBox width / height は CSS px、`1 CSS px = 1/96 in`)。
///
/// Unified Length policy (Sprint 18 全 6 bridge task で verbatim 共有):
/// - `Length::Px(v)` → `length(v)` (identity)
/// - `Length::Pt(v)` → `length(v * 4.0 / 3.0)` — CSS Values 4 §6.2 で
///   `1pt = 1/72 in`、CSS で `1in = 96 px` なので `1 pt = 96/72 px = 4/3 px`。
/// - `Length::Percent(p)` → `percent(p / 100.0)` — CSS spec の authored 0-100 を
///   taffy fraction 0.0-1.0 に。
/// - `Length::Em(_)` / `Length::Rem(_)` → defensive `length(0.0)` — cascade
///   em→px resolution 未実装 (Sprint 18 スコープ外、drain 時に spinout 候補)。
///   TODO: font-size context を cascade で resolve 済にして em/rem を実 px 値へ。
/// - `_` (non_exhaustive catch-all) → `length(0.0)` (forward-compat)
///
/// Wave 2 の `bridge_padding` (raikiri-spike-jbu0) / `bridge_border`
/// (raikiri-spike-q0uc) から consume される。
fn length_to_taffy_length_percentage(len: Length) -> LengthPercentage {
    match len {
        Length::Px(v) => LengthPercentage::length(v),
        Length::Pt(v) => LengthPercentage::length(v * 4.0 / 3.0),
        Length::Percent(p) => LengthPercentage::percent(p / 100.0),
        // TODO(raikiri-spike-0vv.17 相当): cascade で em/rem を px に resolve、
        // ここでは defensive 0.0 で fail-quiet (Sprint 18 スコープ外)。
        Length::Em(_) | Length::Rem(_) => LengthPercentage::length(0.0),
        // non_exhaustive catch-all — unknown future variant は 0.0 で fail-quiet。
        _ => LengthPercentage::length(0.0),
    }
}

/// [`LengthOrAuto`] → [`taffy::Dimension`] bridge (width / height 用)。
///
/// Length policy は [`length_to_taffy_length_percentage`] と同じ。
/// `LengthOrAuto::Auto` → `Dimension::auto()`。
///
/// Wave 2 の [`bridge_size`] (raikiri-spike-ggig、width) から consume される。
/// Wave 3 (raikiri-spike-01up) が height 側でも同 helper を reuse する。
fn length_or_auto_to_taffy_dimension(loa: LengthOrAuto) -> Dimension {
    match loa {
        LengthOrAuto::Auto => Dimension::auto(),
        LengthOrAuto::Length(len) => match len {
            Length::Px(v) => Dimension::length(v),
            Length::Pt(v) => Dimension::length(v * 4.0 / 3.0),
            Length::Percent(p) => Dimension::percent(p / 100.0),
            // TODO(raikiri-spike-0vv.17 相当): cascade で em/rem を px に resolve。
            Length::Em(_) | Length::Rem(_) => Dimension::length(0.0),
            _ => Dimension::length(0.0),
        },
        // non_exhaustive catch-all — 未知 variant は auto に fail-quiet
        // (Sizing spec の initial default が auto なので、safe fallback)。
        _ => Dimension::auto(),
    }
}

/// [`LengthOrAuto`] → [`taffy::LengthPercentageAuto`] bridge (margin 用)。
///
/// Length policy は [`length_to_taffy_length_percentage`] と同じ。
/// `LengthOrAuto::Auto` → `LengthPercentageAuto::auto()` (CSS Box 3 §3.1
/// "margin auto = distribute available space" を taffy に委譲)。
fn length_or_auto_to_taffy_lpa(loa: LengthOrAuto) -> LengthPercentageAuto {
    match loa {
        LengthOrAuto::Auto => LengthPercentageAuto::auto(),
        LengthOrAuto::Length(len) => match len {
            Length::Px(v) => LengthPercentageAuto::length(v),
            Length::Pt(v) => LengthPercentageAuto::length(v * 4.0 / 3.0),
            Length::Percent(p) => LengthPercentageAuto::percent(p / 100.0),
            // TODO(raikiri-spike-0vv.17 相当): cascade で em/rem を px に resolve、
            // ここでは defensive 0.0 で fail-quiet (Sprint 18 スコープ外)。
            Length::Em(_) | Length::Rem(_) => LengthPercentageAuto::length(0.0),
            // non_exhaustive catch-all — unknown future variant は 0.0 で fail-quiet。
            _ => LengthPercentageAuto::length(0.0),
        },
        // non_exhaustive catch-all — 未知 variant は auto に fail-quiet
        // (margin の initial value は 0 だが、Auto に落とすことで taffy が
        // property-specific resolution を行う余地を残す)。
        _ => LengthPercentageAuto::auto(),
    }
}

/// 全 Text node を parley で pre-shape、結果を `Node.text_layout` に格納する。
///
/// 呼び出し側 (`layout_single_page`) は事前に全 `Node.text_layout = None` に
/// clear 済であることを前提とする (re-entrance safety)。
///
/// Font stack / size / weight は `cascade.computed[idx]` (親から inherit 済) を消費。
/// `max_advance` は行折り返し境界で、通常 `page_box.width`。
///
/// # Errors
/// - `LayoutError::Internal` — parley shape が想定外の状態で失敗した場合
///   (M1 ASCII 前提では発生想定なし、defensive)
pub(crate) fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) -> Result<(), LayoutError> {
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        // raikiri-spike-37c roborev job 294 M3 finding: template subtree /
        // detached な text は paint も layout tree (taffy) からも filter される。
        // 無駄な parley shape + intrinsic size 計算を避けるため、bit gate で
        // 早期 skip する。paint / cascade の gate と一貫。
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let text: String = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(t) if !t.text_content.is_empty() => {
                t.text_content.as_str().to_string()
            }
            _ => continue,
        };
        // cascade は Text node 位置にも ComputedValues を populate する
        // (親から inherit)。M1.4 test `text_node_inherits_from_element_parent`
        // で確認済。
        let cv = &cascade.computed[idx];

        // font-family: Vec<Atom> → parley::FontFamily。Atom は SmolStr newtype
        // なので as_str() で &str に落として parley に食わせる。
        //
        // API tuning: brief pseudo-code は `parley::FontStack` を想定していたが
        // parley 0.10 実 API には FontStack 型が存在せず、代わりに
        // `parley::style::FontFamily` (re-export元は `parlance` crate) を使う。
        // `FontFamily::from(&str)` は CSS 形式の family list をそのまま source
        // string として保持する `FontFamily::Source` variant を返す。
        let family_str: String = cv
            .font_family
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let font_family = FontFamily::from(family_str.as_str());

        // API tuning: `Length` is `#[non_exhaustive]` (raikiri-style may add
        // non-Px variants in a later milestone), so this match requires a
        // wildcard arm even though M1.4 scope only produces `Length::Px`.
        // Defensive: surface as `LayoutError::Internal` rather than panic.
        let font_size_px = match cv.font_size {
            Length::Px(v) => v,
            _ => {
                return Err(LayoutError::Internal {
                    message: format!(
                        "preshape_text: unsupported Length variant for font-size at node {idx}"
                    ),
                });
            }
        };

        let mut builder = layout_cx.ranged_builder(fonts, &text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(font_family));
        builder.push_default(StyleProperty::FontSize(font_size_px));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(
            cv.font_weight as f32,
        )));
        let mut layout: Layout<()> = builder.build(&text);
        layout.break_all_lines(Some(max_advance));
        // API tuning: brief pseudo-code は `align(Some(max_advance), Alignment::Start,
        // AlignmentOptions::default())` (3 引数) を想定していたが、parley 0.10 実 API の
        // `Layout::align` は 2 引数 (`alignment`, `options`) のみ。max_advance は
        // 直前の `break_all_lines(Some(max_advance))` で既に確定済のため、align 側では
        // 再指定不要 (内部的に break 時の width を使う)。
        layout.align(Alignment::Start, AlignmentOptions::default());

        if let Some(t) = doc.nodes[idx].data.as_text_mut() {
            t.text_layout = Some(layout);
        }
    }
    Ok(())
}

/// 単一 A4 (or 指定 PageBox) ページに Document を layout する。
///
/// # 変更 (in-place)
/// - Node.text_layout を全 `None` にクリア (re-entrance safety)
/// - `apply_computed_to_style` で computed → taffy::Style bridge (M1.4 no-op)
/// - `preshape_text` で全 Text node を parley shape、Node.text_layout に格納
/// - `apply_page_box_to_body` で body.style.size = length(PageBox)
/// - `compute_root_layout` で taffy 計算、Node.unrounded_layout に書き込む
///
/// # Errors
/// - `LayoutError::Internal` — `<body>` element が見つからない (fragment
///   parse は M1 非対応) / parley shape が失敗 / taffy internal
///
/// # Non-goals in M1.6
/// - 同じ Document で複数回呼ぶことは safe (text_layout を毎回 clear) だが、
///   incremental (差分だけ再走) は M2+ で追加
/// - Consumer からの PageBox 上書きは M4 per-page PageBox で対応
/// - Fragment parse (no `<body>`) support は M2+
/// # API 互換性 (raikiri-spike-e93, spec §6.3)
///
/// この signature は M1.14 の 3-arg `(document, cascade, page_box)` から
/// 4-arg `(document, cascade, page_box, font_ctx)` に **意図的に breaking
/// change** された (spec §6.3 の choice β)。α (dual API: 既存 3-arg +
/// 新規 `_with_fonts`) との trade-off の末、raikiri-dom 内 caller が全て
/// in-repo (12 箇所 = production 1 + test 11) であり、内部 DI の explicit
/// 化と signature 統一の方が長期保守で優れると判断した。詳細:
/// - spec `docs/superpowers/specs/2026-07-18-raikiri-spike-e93-wpt-font-pin-design.md`
///   §6.3 (β 選択の理由), §6.4 (12 caller の内訳)
/// - roborev finding e93 round 3 M3 で reflag、user 再確認済 (plan-mandated)
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    mut font_ctx: FontContext,
) -> Result<(), LayoutError> {
    // raikiri-spike-37c roborev job 295 M1 finding: observation-side entry で
    // membership を sync する — `mark_in_document_flags` は flags_dirty=false
    // なら idempotent no-op なので、既に sink.finish() 経由で sync 済の場合は
    // 事実上 free。post-parse mutation (`Document::append_*` 等) の後で cascade
    // を skip して直接 layout する consumer に対する safety net。
    //
    // Contract note: cascade は `&D: Dom` を取り mutation 不可なので、cascade
    // 呼び出し側で sync せざるを得ない (parse.finish() 経由でしか自動 sync
    // されない)。layout はここで sync することで少なくとも layout/paint 段に
    // stale bit を持ち込まないことを保証する。
    document.mark_in_document_flags();

    // Step 0: text_layout re-entrance clear
    for node in document.nodes.iter_mut() {
        if let Some(t) = node.data.as_text_mut() {
            t.text_layout = None;
        }
    }

    // Step 1: ComputedValues → taffy::Style bridge (M1.4 no-op site)
    apply_computed_to_style(document, cascade);

    // Step 2: pre-shape all text with parley
    // font_ctx は呼び出し側が構築 (system font 経路なら FontContext::new()、
    // VRT なら raikiri_dom::fonts::build_wpt_font_ctx で pin 済)
    let mut layout_cx = LayoutContext::<()>::new();
    preshape_text(
        document,
        cascade,
        &mut font_ctx,
        &mut layout_cx,
        page_box.width,
    )?;

    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported in M1)".to_string(),
    })?;

    // Step 4: body.style.size を PageBox に強制セット
    apply_page_box_to_body(document, body_id, page_box);

    // Step 5: taffy compute
    compute_root_layout(
        document,
        TaffyNodeId::from(body_id),
        taffy::Size {
            width: AvailableSpace::Definite(page_box.width),
            height: AvailableSpace::Definite(page_box.height),
        },
    );

    Ok(())
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
        assert_eq!(size.width, Dimension::length(793.7008));
        assert_eq!(size.height, Dimension::length(1122.5197));
    }

    #[test]
    fn preshape_text_populates_text_layout_for_text_nodes() {
        use parley::{FontContext, LayoutContext};
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
        let text = doc.append_text(p, "Hi");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        let mut fonts = FontContext::new();
        let mut layout_cx = LayoutContext::<()>::new();
        preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width)
            .expect("preshape Ok");

        assert!(
            doc.nodes[text].text_layout().is_some(),
            "text node's text_layout must be populated"
        );
        let layout = doc.nodes[text].text_layout().unwrap();
        assert!(layout.width() > 0.0, "text 'Hi' must have non-zero width");
        assert!(
            layout.height() > 0.0,
            "text 'Hi' must have non-zero line height"
        );

        // Element / Document は None のまま
        assert!(
            doc.nodes[html].text_layout().is_none(),
            "html element is not text"
        );
        assert!(
            doc.nodes[body].text_layout().is_none(),
            "body element is not text"
        );
        assert!(
            doc.nodes[p].text_layout().is_none(),
            "p element is not text"
        );
        assert!(
            doc.nodes[0].text_layout().is_none(),
            "document root is not text"
        );
    }

    #[test]
    fn preshape_text_respects_computed_font_size() {
        use parley::{FontContext, LayoutContext};
        use raikiri_style::{build_rule_tree, cascade};

        fn shape_text_height_at_font_size(px: &str) -> f32 {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let inline = format!("font-size:{}", px);
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline.as_str()));
            let text = doc.append_text(p, "Hi");
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).unwrap();
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width).unwrap();
            doc.nodes[text].text_layout().unwrap().height()
        }

        let small = shape_text_height_at_font_size("8px");
        let large = shape_text_height_at_font_size("32px");
        assert!(
            large > small,
            "font-size:32px must produce taller text than 8px (cascade→shape inheritance regression pin, got small={small}, large={large})"
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_display_to_taffy() {
        // raikiri-spike-w2s: display bridge active — DisplayValue → taffy::Display
        // mapping が正しく行われていることを確認する regression pin。
        //
        // raikiri-spike-j5rz (Sprint 18): bridge_margin が dispatch に加わったが
        // margin unspecified の element では initial `Sides::all(Length::Px(0.0))`
        // が cascade で入る → taffy `LengthPercentageAuto::length(0.0)` に translate、
        // これは `taffy::Style::default().margin` (all `Length(0.0)`) と一致するため
        // 既存 assertion は無変更で通ることを確認する pin にもなる。
        //
        // raikiri-spike-jbu0 (Sprint 18 Wave 2): bridge_padding も dispatch に
        // 加わったが同様に padding unspecified の element では initial
        // `Sides::all(Length::Px(0.0))` → taffy `LengthPercentage::length(0.0)`
        // が入り、`taffy::Style::default().padding` と一致するため padding assertion
        // も無変更で通る pin。
        //
        // raikiri-spike-ggig (Sprint 18 Wave 2): bridge_size (width) が dispatch に
        // 加わったが width unspecified の element は initial `LengthOrAuto::Auto`
        // → `Dimension::auto()` に translate、これは `taffy::Style::default().size`
        // (`Size::auto()`) の width と一致 (height は Wave 3 まで default 保持)。
        // 既存 `size == default_style.size` 相当 assertion は変化なく通る。
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:none"));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        apply_computed_to_style(&mut doc, &cr);
        assert_eq!(doc.nodes[body].style.display, Display::None);

        let default_style = <taffy::Style as Default>::default();
        assert_eq!(doc.nodes[body].style.size, default_style.size);
        assert_eq!(doc.nodes[body].style.margin, default_style.margin);
        assert_eq!(doc.nodes[body].style.padding, default_style.padding);
    }

    #[test]
    fn apply_computed_to_style_bridges_margin_to_taffy() {
        // raikiri-spike-j5rz (Sprint 18 dom-4 Wave 1): bridge_margin が
        // Sides<LengthOrAuto> を taffy::Rect<LengthPercentageAuto> に translate
        // することを確認する regression pin。Unified Length policy の 4 分岐
        // (Px / Auto / Percent / Pt) をそれぞれ 1 case で covering。
        //
        // Test 戦略: 各 case は独立 fixture で cascade → apply_computed_to_style
        // → body.style.margin を assert。inline style 経由なので raikiri-style
        // の parse_margin_shorthand + longhand path も同時に regression pin。
        use raikiri_style::{build_rule_tree, cascade};
        use taffy::{LengthPercentageAuto, Rect};

        fn margin_for(inline: &str) -> Rect<LengthPercentageAuto> {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[body].style.margin
        }

        // Case 1: shorthand `margin: 10px 20px 30px 40px` (top/right/bottom/left)
        //   → Rect { top: 10, right: 20, bottom: 30, left: 40 } (all Px identity)。
        //   Sides.top,right,bottom,left → Rect.top,right,bottom,left の field-name
        //   mapping を pin (positional silent transpose を防ぐ)。
        assert_eq!(
            margin_for("margin: 10px 20px 30px 40px"),
            Rect {
                top: LengthPercentageAuto::length(10.0),
                right: LengthPercentageAuto::length(20.0),
                bottom: LengthPercentageAuto::length(30.0),
                left: LengthPercentageAuto::length(40.0),
            }
        );

        // Case 2: shorthand `margin: auto` → 4 side 全て auto()。
        assert_eq!(
            margin_for("margin: auto"),
            Rect {
                top: LengthPercentageAuto::auto(),
                right: LengthPercentageAuto::auto(),
                bottom: LengthPercentageAuto::auto(),
                left: LengthPercentageAuto::auto(),
            }
        );

        // Case 3: longhand `margin-left: 50%` → left = percent(0.5)、他 3 side は
        //   initial (0.0 px)。CSS spec の authored 0-100 → taffy fraction 0.0-1.0
        //   の div-by-100 policy を pin。
        assert_eq!(
            margin_for("margin-left: 50%"),
            Rect {
                top: LengthPercentageAuto::length(0.0),
                right: LengthPercentageAuto::length(0.0),
                bottom: LengthPercentageAuto::length(0.0),
                left: LengthPercentageAuto::percent(0.5),
            }
        );

        // Case 4: longhand `margin-top: 10pt` → top = length(10 * 4/3) = length(13.333...)。
        //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
        //   f32 bit-identical assert のため右辺を expression のまま書く
        //   (`13.333` literal は round-trip で drift する)。
        assert_eq!(
            margin_for("margin-top: 10pt"),
            Rect {
                top: LengthPercentageAuto::length(10.0 * 4.0 / 3.0),
                right: LengthPercentageAuto::length(0.0),
                bottom: LengthPercentageAuto::length(0.0),
                left: LengthPercentageAuto::length(0.0),
            }
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_padding_to_taffy() {
        // raikiri-spike-jbu0 (Sprint 18 dom-4 Wave 2): bridge_padding が
        // Sides<Length> を taffy::Rect<LengthPercentage> に translate することを
        // 確認する regression pin。padding は margin と違い `auto` を持たない
        // (<length-percentage [0,∞]>) ため 3 分岐 (Px / Percent / Pt) を各 1 case
        // で covering。
        //
        // Test 戦略: 各 case は独立 fixture で cascade → apply_computed_to_style
        // → body.style.padding を assert。inline style 経由なので raikiri-style
        // の parse_padding_shorthand + longhand path も同時に regression pin。
        use raikiri_style::{build_rule_tree, cascade};

        fn padding_for(inline: &str) -> Rect<LengthPercentage> {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[body].style.padding
        }

        // Case 1: shorthand `padding: 5px 10px 15px 20px` (top/right/bottom/left)
        //   → Rect { top: 5, right: 10, bottom: 15, left: 20 } (all Px identity)。
        //   Sides.top,right,bottom,left → Rect.top,right,bottom,left の field-name
        //   mapping を pin (positional silent transpose を防ぐ — Sides の field 順は
        //   top,right,bottom,left、Rect の field 順は left,right,top,bottom で異なる)。
        assert_eq!(
            padding_for("padding: 5px 10px 15px 20px"),
            Rect {
                top: LengthPercentage::length(5.0),
                right: LengthPercentage::length(10.0),
                bottom: LengthPercentage::length(15.0),
                left: LengthPercentage::length(20.0),
            }
        );

        // Case 2: longhand `padding-left: 5%` → left = percent(0.05)、他 3 side は
        //   initial (0.0 px)。CSS spec の authored 0-100 → taffy fraction 0.0-1.0
        //   の div-by-100 policy を pin。
        assert_eq!(
            padding_for("padding-left: 5%"),
            Rect {
                top: LengthPercentage::length(0.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::percent(0.05),
            }
        );

        // Case 3: longhand `padding-top: 3pt` → top = length(3 * 4/3) = length(4.0)。
        //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
        //   f32 bit-identical assert のため右辺を expression のまま書く
        //   (`4.0` literal は 3*4/3 と bit-identical だが policy 明示のため式のまま)。
        assert_eq!(
            padding_for("padding-top: 3pt"),
            Rect {
                top: LengthPercentage::length(3.0 * 4.0 / 3.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_width_to_taffy() {
        // raikiri-spike-ggig (Sprint 18 dom-4 Wave 2): bridge_size (width component)
        // が cv.width: LengthOrAuto を taffy::Style::size.width: Dimension に
        // translate することを pin する。Unified Length policy の 4 分岐
        // (Px / Auto / Percent / Pt) をそれぞれ 1 case で covering。
        //
        // Test 戦略: fixture は **非 body element** (この場合 `<p>`) を使う —
        // `<body>` は後段 `apply_page_box_to_body` で clobber されるため本 bridge
        // の効果は observable でない (別 test `apply_page_box_clobbers_body_width_from_bridge`
        // で clobber 挙動を pin)。inline style 経由なので raikiri-style の
        // parse_width path + LengthOrAuto encoding も同時に regression pin。
        //
        // scaffold contract: height は Wave 3 (raikiri-spike-01up) が書くため
        // 本 test では size.height を assert しない (default 保持は field assign
        // 実装で自然に守られるが、Wave 3 との conflict 面積を最小化するため
        // width のみに absert 対象を絞る)。
        use raikiri_style::{build_rule_tree, cascade};

        fn width_for(inline: &str) -> Dimension {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            // 非 body element (p) に inline を載せる。apply_page_box_to_body は
            // body だけを触るため、p の style.size は bridge 実行後そのまま観測可能。
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[p].style.size.width
        }

        // Case 1: `width: 100px` → Dimension::length(100.0) (Px identity)。
        assert_eq!(width_for("width: 100px"), Dimension::length(100.0));

        // Case 2: `width: auto` → Dimension::auto() (LengthOrAuto::Auto arm)。
        assert_eq!(width_for("width: auto"), Dimension::auto());

        // Case 3: `width: 50%` → Dimension::percent(0.5)。CSS spec の authored
        //   0-100 → taffy fraction 0.0-1.0 の div-by-100 policy を pin。
        assert_eq!(width_for("width: 50%"), Dimension::percent(0.5));

        // Case 4: `width: 20pt` → Dimension::length(20 * 4/3) = length(26.666...)。
        //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
        //   f32 bit-identical assert のため右辺を expression で書く。
        assert_eq!(
            width_for("width: 20pt"),
            Dimension::length(20.0 * 4.0 / 3.0)
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_border_to_taffy() {
        // raikiri-spike-q0uc (Sprint 18 dom-4 Wave 2): bridge_border が
        // Sides<Border> を taffy::Rect<LengthPercentage> に translate、CSS
        // Backgrounds 3 §5.2 の "used border-width" style-gating を適用する
        // ことを確認する regression pin。
        //
        // Advisor #2 spec correctness gate: border-style が None / Hidden の
        // 場合、specified border-width にかかわらず used border-width = 0 で
        // なければならない (§5.2 "The used values of the corresponding
        // border-*-width become 0.")。gating を怠ると specified 5px が taffy
        // に leak して layout に影響 → spec 違反。
        //
        // Test 戦略: `border: <w> <s> <c>` 4-side shorthand と longhand の
        // 両方を使い、shorthand 展開 → per-side cascade → bridge_border の
        // pipeline を end-to-end で pin する (raikiri-spike-e51 codebase note:
        // 単一 side shorthand `border-top: ...` は現時点で parser 未対応、
        // computed.rs 228-229 参照)。
        use raikiri_style::{build_rule_tree, cascade};
        use taffy::{LengthPercentage, Rect};

        fn border_for(inline: &str) -> Rect<LengthPercentage> {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), Some(inline));
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[body].style.border
        }

        // Case 1 (positive path): `border: 5px solid red` shorthand → 4 side
        //   全て width=5、style=solid で cascade。gating off (solid ≠ None/Hidden)
        //   なので 4 side 全て length(5.0) になる。Rect.top/right/bottom/left ↔
        //   Sides.top/right/bottom/left の field-name mapping pin。
        assert_eq!(
            border_for("border: 5px solid red"),
            Rect {
                top: LengthPercentage::length(5.0),
                right: LengthPercentage::length(5.0),
                bottom: LengthPercentage::length(5.0),
                left: LengthPercentage::length(5.0),
            }
        );

        // Case 2 (spec correctness — advisor #2): `border: 5px none red` shorthand
        //   → 4 side 全て width=5, style=None で cascade。§5.2 style-gating で
        //   used width = 0 → 4 side 全て length(0.0)。gating が壊れると 5.0 が
        //   leak するので、この case が canary。
        assert_eq!(
            border_for("border: 5px none red"),
            Rect {
                top: LengthPercentage::length(0.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );

        // Case 3 (spec correctness — advisor #2): `border: 5px hidden red`
        //   shorthand → §5.2 で hidden は "Same as none, except in terms of
        //   border conflict resolution for table elements." — used width = 0。
        assert_eq!(
            border_for("border: 5px hidden red"),
            Rect {
                top: LengthPercentage::length(0.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );

        // Case 4 (pt unit conversion): `border-top-width: 3pt` + solid → top
        //   only、他 3 side は initial (width=medium=3px, style=None) → gating
        //   で length(0.0)。top は 3pt × 4/3 = 4.0 px (CSS Values 4 §6.2、
        //   1pt = 96/72 px = 4/3 px)。f32 bit-identical のため右辺は式のまま。
        assert_eq!(
            border_for("border-top-width: 3pt; border-top-style: solid"),
            Rect {
                top: LengthPercentage::length(3.0 * 4.0 / 3.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );

        // Case 5 (medium keyword): `border-top-width: medium` + solid → top =
        //   3.0 px (§5.1 UA-defined recommendation の thin=1/medium=3/thick=5、
        //   property.rs `parse_border_width_side` 参照)。他 3 side は Case 4
        //   同様 gating で 0。medium keyword が Length::Px(3.0) にパースされる
        //   ことを end-to-end で pin。
        assert_eq!(
            border_for("border-top-width: medium; border-top-style: solid"),
            Rect {
                top: LengthPercentage::length(3.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_box_sizing_to_taffy() {
        // raikiri-spike-o11x (Sprint 18 dom-4 Wave 2): bridge_box_sizing が
        // raikiri_style::BoxSizing → taffy::BoxSizing の enum 1:1 mapping を
        // 実施することを確認する regression pin。
        //
        // 3 case:
        //   #1 border-box (specified)   → taffy::BoxSizing::BorderBox
        //   #2 content-box (specified)  → taffy::BoxSizing::ContentBox
        //   #3 unspecified (cascade default = raikiri-style initial = ContentBox)
        //      → taffy::BoxSizing::ContentBox
        //
        // 特筆: taffy 0.12 default は BorderBox (spec 違反)、raikiri-style initial
        // は ContentBox (CSS Sizing 3 §3.3 準拠)。#3 は cascade が initial 経由で
        // ContentBox を seed し、bridge がそれを taffy に伝播することで、taffy default
        // の spec 違反を副作用的に補正することを pin する。
        use raikiri_style::{build_rule_tree, cascade};

        fn box_sizing_for(inline: Option<&str>) -> TaffyBoxSizing {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), inline);
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[body].style.box_sizing
        }

        // Case 1: border-box → taffy::BoxSizing::BorderBox
        assert_eq!(
            box_sizing_for(Some("box-sizing: border-box")),
            TaffyBoxSizing::BorderBox
        );

        // Case 2: content-box (explicit) → taffy::BoxSizing::ContentBox
        assert_eq!(
            box_sizing_for(Some("box-sizing: content-box")),
            TaffyBoxSizing::ContentBox
        );

        // Case 3: unspecified → raikiri-style initial (ContentBox) → taffy ContentBox
        // (taffy default の BorderBox を上書き、spec 補正 pin)
        assert_eq!(box_sizing_for(None), TaffyBoxSizing::ContentBox);
    }

    #[test]
    fn apply_page_box_clobbers_body_width_from_bridge() {
        // raikiri-spike-ggig (Sprint 18 dom-4 Wave 2、advisor #3): M1 PageBox
        // 妥協の regression pin — `<body style="width: 100px">` に対して
        //   Step 1 (`apply_computed_to_style`) → bridge_size が body.style.size.width
        //       を length(100.0) に write
        //   Step 4 (`apply_page_box_to_body`) → PageBox.width で clobber
        // の順で走ると、最終 body.style.size.width は PageBox.width (author 値
        // ではない) になる。M4 で @page per-page PageBox に refactor するまで
        // この clobber 挙動を意図的に保つ (M1 妥協) — silent regression 検出用。
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("width: 100px"));
        let rules = raikiri_style::build_rule_tree(&doc);
        let cr = raikiri_style::cascade(&doc, &rules).expect("cascade Ok");

        // Step 1: bridge 実行後、body.style.size.width は author 値 100px。
        apply_computed_to_style(&mut doc, &cr);
        assert_eq!(
            doc.nodes[body].style.size.width,
            Dimension::length(100.0),
            "bridge_size must first write author width (100px) to body.style.size.width"
        );

        // Step 4: PageBox clobber 後、author 値は消えて PageBox.width が入る。
        apply_page_box_to_body(&mut doc, body, PageBox::A4);
        assert_eq!(
            doc.nodes[body].style.size.width,
            Dimension::length(PageBox::A4.width),
            "apply_page_box_to_body must clobber author width with PageBox.width (M1 妥協)"
        );
        // author 値と PageBox 値は不一致 (clobber が実際に起きていることを pin)。
        assert_ne!(
            doc.nodes[body].style.size.width,
            Dimension::length(100.0),
            "post-clobber body.style.size.width must NOT equal author 100px"
        );
    }

    // ── layout_single_page driver (Task 7) ──────────────────────

    fn hello_world_doc() -> (Document, raikiri_style::CascadeResult) {
        // <html><head></head><body><p style="color:red">Hi</p></body></html>
        // 相当 (parser の代わりに手動構築、raikiri-html 統合は m1.7+ で umbrella が担当)
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
        let _text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        (doc, cr)
    }

    #[test]
    fn layout_single_page_hello_world_produces_body_at_page_width() {
        use raikiri_traits::PageBox;
        let (mut doc, cr) = hello_world_doc();
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
        // body の layout size.width が A4 幅 (793.7008) と一致
        let body_id = find_body(&doc).expect("body exists");
        let body_size = doc.nodes[body_id].unrounded_layout.size;
        assert!(
            (body_size.width - 793.7008).abs() < 0.5,
            "body width should be A4.width (793.7008), got {}",
            body_size.width
        );
        assert!(
            body_size.height > 0.0,
            "body height should be non-zero from block layout of <p>Hi</p>, got {}",
            body_size.height
        );
    }

    #[test]
    fn layout_single_page_without_body_returns_error() {
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::{LayoutError, PageBox};

        // <p> 直接 attach (fragment 相当)
        let mut doc = Document::new();
        let _p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).unwrap();

        match layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()) {
            Err(LayoutError::Internal { message }) => {
                assert!(
                    message.contains("body"),
                    "error message should mention <body>, got '{}'",
                    message
                );
            }
            other => panic!("expected LayoutError::Internal, got {:?}", other),
        }
    }

    #[test]
    fn layout_single_page_can_be_called_multiple_times() {
        use raikiri_traits::PageBox;
        let (mut doc, cr) = hello_world_doc();
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("first call Ok");
        let body_id = find_body(&doc).expect("body exists");
        let first_size = doc.nodes[body_id].unrounded_layout.size;

        // 2 回目呼び出し — text_layout の re-entrance clear と layout の再走が
        // 同じ結果を返すことを pin (将来 incremental optimization が silent
        // regression を起こしても検出できる)
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("second call Ok");
        let second_size = doc.nodes[body_id].unrounded_layout.size;

        assert!((first_size.width - second_size.width).abs() < 0.001);
        assert!((first_size.height - second_size.height).abs() < 0.001);
    }

    #[test]
    fn layout_single_page_bridges_display_none() {
        // raikiri-spike-w2s: layout_single_page 経由で display bridge が active
        // であることを確認 — body に display:none を指定すると taffy::Style.display
        // が Display::None になる。
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), Some("display:none"));
        let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
        let _text = doc.append_text(p, "Hi");
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
        assert_eq!(doc.nodes[body].style.display, Display::None);
    }

    #[test]
    fn layout_single_page_deterministic_across_10_runs() {
        // M1 acceptance: 10 回連続実行で byte-identical。
        // 同一マシン上の determinism を pin (cross-machine は m1.13 で font
        // pinning に置き換わる)。
        use raikiri_traits::PageBox;

        fn one_run() -> Vec<taffy::Layout> {
            let (mut doc, cr) = hello_world_doc();
            layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
            doc.nodes.iter().map(|n| n.unrounded_layout).collect()
        }

        let baseline = one_run();
        for i in 1..10 {
            let run = one_run();
            assert_eq!(
                baseline.len(),
                run.len(),
                "run {i}: layout node count changed"
            );
            for (j, (b, r)) in baseline.iter().zip(run.iter()).enumerate() {
                // taffy::Layout の全 field を byte-identical で比較。
                // 浮動小数点の subnormal / NaN drift があると here が最も先に
                // 反応する (design doc §12.8 の NonFiniteFloat 検討の pin 相当)
                assert_eq!(
                    b.size.width, r.size.width,
                    "run {i} node {j}: size.width differs (baseline={} run={})",
                    b.size.width, r.size.width
                );
                assert_eq!(b.size.height, r.size.height);
                assert_eq!(b.location.x, r.location.x);
                assert_eq!(b.location.y, r.location.y);
            }
        }
    }

    #[test]
    #[ignore] // 明示的に cargo test -- --ignored で実行
    fn font_context_new_cost_is_reasonable() {
        let start = std::time::Instant::now();
        for _ in 0..10 {
            let _ = parley::FontContext::new();
        }
        let elapsed = start.elapsed();
        // 10 回 total で 5 秒未満なら M1.6 の per-call new() は許容
        // (10 連ラン determinism test が timeout しないため)
        assert!(
            elapsed.as_secs() < 5,
            "FontContext::new() too slow: 10x = {:?}",
            elapsed
        );
    }
}
