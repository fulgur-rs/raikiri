//! Single-page layout driver — `layout_single_page` を pub 提供。
//!
//! Pipeline: cascade (raikiri-style) 出力 + Document arena + PageBox から
//! taffy compute_root_layout を駆動し、text intrinsic size は parley 0.10 の
//! 最小統合で pre-shape する。現在の scope は単一 A4 ページ、ASCII Latin、
//! parley system font default (byte-identical cross-machine は将来 font pinning で対応予定)。
//!
//! 全 helper は crate-private、pub 型は [`layout_single_page`] のみ。

use raikiri_traits::NodeKind;
use rayon::prelude::*;

use crate::document::Document;
use crate::node::NodeFlags;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, StyleProperty,
};
use raikiri_style::property::{
    AlignSelfValue, BoxSizing as StyleBoxSizing, ClearValue, ContentAlignmentValue, DisplayValue,
    FlexDirectionValue, FlexWrapValue, FloatValue, FontStyle as StyleFontStyle, GridAutoFlowValue,
    GridLineValue, GridRepeatCount, GridTemplateAreasValue, SelfAlignmentValue,
};
use raikiri_style::{
    CascadeResult, ComputedFlexBasis, ComputedGridTemplateTracks, ComputedGridTrackBreadth,
    ComputedGridTrackListComponent, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedLineHeight, ComputedValues,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AlignContent as TaffyAlignContent, AlignItems as TaffyAlignItems, AvailableSpace,
    BoxSizing as TaffyBoxSizing, Clear as TaffyClear, Dimension, Display,
    FlexDirection as TaffyFlexDirection, FlexWrap as TaffyFlexWrap, Float as TaffyFloat,
    GridAutoFlow as TaffyGridAutoFlow, GridPlacement, GridTemplateArea as TaffyGridTemplateArea,
    GridTemplateComponent, GridTemplateRepetition, Layout as TaffyLayout, LengthPercentage,
    LengthPercentageAuto, Line as TaffyLine, MaxTrackSizingFunction, MinTrackSizingFunction,
    NodeId as TaffyNodeId, Point, Rect, RepetitionCount as TaffyRepetitionCount, Size,
    TrackSizingFunction, compute_root_layout, style_helpers as taffy_style_helpers,
};

pub(crate) mod table;

/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// deep DOM で stack overflow を回避)。fragment parse (no `<body>`) では
/// `None`、caller が `LayoutError::Internal` に昇格させる。
///
/// `!is_in_document()` の subtree
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
/// CSS Paged Media の initial containing block = @page size。現行実装は @page 非対応
/// のため body.style.size に直接注入する妥協。将来 @page cascade + per-page
/// PageBox を導入する際に `<html>` root style に site を昇格予定。
pub(crate) fn apply_page_box_to_body(doc: &mut Document, body_id: usize, page_box: PageBox) {
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(page_box.width),
        height: Dimension::length(page_box.height),
    };
}

/// ComputedValues → taffy::Style bridge の dispatch site。
///
/// per-element for loop 内 inline mapping から per-field `bridge_*` helper へ
/// dispatch する pattern にリファクタ済み。新しい bridge は helper 追加 +
/// dispatch 1 行追加のみで足りるよう設計している。
///
/// 現時点で active な bridge:
/// - [`bridge_display`] — [`DisplayValue`] → [`taffy::Display`]
/// - [`bridge_float`] — [`ComputedValues::float`] / [`ComputedValues::clear`] →
///   [`taffy::Style::float`] / [`taffy::Style::clear`] (CSS2 §9.5.1 / §9.5.2)
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentage>` → [`taffy::Rect<LengthPercentage>`]
/// - [`bridge_size`] — [`ComputedLengthPercentageOrAuto`] `cv.width` / `cv.height` →
///   [`taffy::Style::size`] (`Size<Dimension>`)。width / height 両 field を
///   struct literal 1 発 assign で書く。
/// - [`bridge_border`] — `Sides<ComputedBorder>` → [`taffy::Rect<LengthPercentage>`]
///   。**border-style gating は本 bridge ではなく上流の
///   `raikiri_style::resolve_border` (computed 層) が持つ** — CSS Backgrounds 3
///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>。
/// - [`bridge_box_sizing`] — [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`]
///   (CSS Sizing 3 §7)
/// - [`bridge_flex`] — `flex-direction` / `flex-wrap` / `flex-grow` /
///   `flex-shrink` / `flex-basis` → [`taffy::Style`]'s matching flex
///   container/item fields (CSS Flexible Box Layout Module Level 1)
/// - [`bridge_alignment`] — `justify-content` / `align-content` /
///   `align-items` / `align-self` / `justify-items` / `justify-self` →
///   [`taffy::Style`]'s matching `Option<AlignItems>`/`Option<AlignContent>`
///   fields (CSS Box Alignment Module Level 3)
/// - [`bridge_gap`] — `row-gap` / `column-gap` → [`taffy::Style::gap`]
///   (`Size<LengthPercentage>`, CSS Box Alignment Module Level 3 §8.1)
/// - [`bridge_grid`] — `grid-template-columns` / `grid-template-rows` /
///   `grid-template-areas` / `grid-auto-columns` / `grid-auto-rows` /
///   `grid-auto-flow` / `grid-row-start` / `grid-row-end` /
///   `grid-column-start` / `grid-column-end` → [`taffy::Style`]'s matching
///   grid container/item fields (CSS Grid Layout Module Level 1)
///
/// per-node の bridge loop の後、second pass
/// ([`establish_minimal_line_boxes`]) が bridge 済みの tree 全体を走査し、
/// 条件を満たす block container に minimal な inline formatting context を
/// 確立する — qualifying condition と scope は同関数の doc 参照。
pub(crate) fn apply_computed_to_style(doc: &mut Document, cascade: &CascadeResult) {
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        // Preserve full DisplayValue for table dispatch before taffy collapses it.
        doc.nodes[idx].display = cv.display;
        // Table engine inputs — no taffy::Style counterpart (taffy 0.12
        // has no table layout), carried Node-side like `display` above.
        doc.nodes[idx].table_layout = cv.table_layout;
        doc.nodes[idx].border_collapse = cv.border_collapse;
        let style = &mut doc.nodes[idx].style;
        bridge_display(style, cv);
        bridge_float(style, cv);
        bridge_margin(style, cv, &mut doc.layout_warnings);
        bridge_padding(style, cv, &mut doc.layout_warnings);
        bridge_size(style, cv, &mut doc.layout_warnings);
        bridge_border(style, cv, &mut doc.layout_warnings);
        bridge_box_sizing(style, cv);
        bridge_flex(style, cv, &mut doc.layout_warnings);
        bridge_alignment(style, cv);
        bridge_gap(style, cv, &mut doc.layout_warnings);
        bridge_grid(style, cv, &mut doc.layout_warnings);
    }
    establish_minimal_line_boxes(doc, cascade);
    // Mark table formatting roots for blitz-compat bit preservation.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let is_table = matches!(
            doc.nodes[idx].display,
            DisplayValue::Table | DisplayValue::InlineTable
        );
        if is_table {
            doc.nodes[idx].flags.insert(NodeFlags::IS_TABLE_ROOT);
        } else {
            doc.nodes[idx].flags.remove(NodeFlags::IS_TABLE_ROOT);
        }
    }
}

/// [`DisplayValue`] → [`taffy::Display`] mapping。
///
/// [`DisplayValue`] を
/// [`taffy::Display`] に mapping する。taffy 0.x は Block / Flex / Grid /
/// None のみ (`Inline` / `InlineBlock` 独立 variant なし) のため:
/// - `Inline` → `Block` (initial は Block、text-only は leaf で render)
/// - `InlineBlock` → `Block` (block child + inline-level flow parent の
///   separate 扱い、精密化は follow-up)
/// - `None` → `None`
/// - `Flex` → `Flex` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block flex` と等価。
///   [`crate::taffy_impl`]'s `LayoutFlexboxContainer` impl + taffy's
///   `compute_flexbox_layout` が実 layout を担う)
/// - `Grid` → `Grid` (CSS Display 3 §2.2 `<display-inside>` keyword、
///   outer-defaulting rule で `block grid` と等価。
///   [`crate::taffy_impl`]'s `LayoutGridContainer` impl + taffy's
///   `compute_grid_layout` が実 layout を担う)
/// - Table internal types (`table` / `inline-table` / `table-row-group` /
///   `table-header-group` / `table-footer-group` / `table-row` /
///   `table-column-group` / `table-column` / `table-cell` / `table-caption`)
///   → `Block` (暫定: taffy 0.12 は table layout 未対応のため block 近似。
///   TODO(table-layout): [`crate::layout::table`] の dedicated table
///   formatting context (CSS 2.1 §17.2.1 anonymous table object generation
///   含む) が landing したら専用 Display / layout へ置換)
/// - `list-item` / `contents` → `Block` (catch-all 経由、将来専用 handling
///   が入るまで block 近似)
/// - catch-all arm → `Block` (`non_exhaustive` forward-compat)
fn bridge_display(style: &mut taffy::Style, cv: &ComputedValues) {
    style.display = match cv.display {
        DisplayValue::Block => Display::Block,
        DisplayValue::Inline => Display::Block,
        DisplayValue::InlineBlock => Display::Block,
        DisplayValue::None => Display::None,
        DisplayValue::Flex => Display::Flex,
        DisplayValue::Grid => Display::Grid,
        // Table model — CSS Display 3 §2 / CSS 2.1 §17.2. Taffy 0.12 has no
        // table layout, so map to Block as temporary approximation.
        // Real table routing uses `Node::display` (preserved in
        // `apply_computed_to_style` before this collapse) rather than
        // `Style::display`.
        DisplayValue::Table => Display::Block,
        DisplayValue::InlineTable => Display::Block,
        DisplayValue::TableRowGroup => Display::Block,
        DisplayValue::TableHeaderGroup => Display::Block,
        DisplayValue::TableFooterGroup => Display::Block,
        DisplayValue::TableRow => Display::Block,
        DisplayValue::TableColumnGroup => Display::Block,
        DisplayValue::TableColumn => Display::Block,
        DisplayValue::TableCell => Display::Block,
        DisplayValue::TableCaption => Display::Block,
        _ => {
            // non_exhaustive catch-all — unknown future variant (e.g.
            // `list-item` / `contents`) goes to Block
            Display::Block
        }
    };
}

/// [`ComputedValues::float`] / [`ComputedValues::clear`] → [`taffy::Style::float`] /
/// [`taffy::Style::clear`] bridge (CSS2 §9.5.1
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-float> / §9.5.2
/// <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>)。**enum 1:1
/// mapping** (both raikiri-style's `FloatValue`/`ClearValue` and taffy's
/// `Float`/`Clear` carry exactly the spec's keyword sets, so no length
/// resolution or Length policy participation is needed, same shape as
/// [`bridge_box_sizing`]).
///
/// This only carries the keyword through; the actual float positioning,
/// shrink-to-fit width, and wraparound layout it drives is taffy's
/// `float_layout` feature (`compute_block_layout`'s `BlockFormattingContext`/
/// `FloatContext`), wired via this crate's `taffy_impl` module.
///
/// **Known taffy `block_layout` limitation**: a same-BFC child narrower than
/// its own BFC root can get float insets computed against the root's width
/// instead of its own — not yet worked around or covered by a test here.
fn bridge_float(style: &mut taffy::Style, cv: &ComputedValues) {
    style.float = match cv.float {
        FloatValue::None => TaffyFloat::None,
        FloatValue::Left => TaffyFloat::Left,
        FloatValue::Right => TaffyFloat::Right,
        // cov:ignore: unreachable while FloatValue is None|Left|Right only;
        // required for its #[non_exhaustive] contract.
        _ => TaffyFloat::None,
    };
    style.clear = match cv.clear {
        ClearValue::None => TaffyClear::None,
        ClearValue::Left => TaffyClear::Left,
        ClearValue::Right => TaffyClear::Right,
        ClearValue::Both => TaffyClear::Both,
        // cov:ignore: unreachable while ClearValue is None|Left|Right|Both
        // only; required for its #[non_exhaustive] contract.
        _ => TaffyClear::None,
    };
}

/// [`ComputedValues::margin`] (`Sides<ComputedLengthPercentageOrAuto>`) → [`taffy::Style::margin`]
/// (`Rect<LengthPercentageAuto>`) bridge。
///
/// CSS Box 3 §3.1 <https://www.w3.org/TR/css-box-3/#margin-physical> の
/// physical margin 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_length_percentage_auto`] を参照。
fn bridge_margin(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let m = cv.margin;
    style.margin = Rect {
        top: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.top, diag),
        right: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.right, diag),
        bottom: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.bottom, diag),
        left: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.left, diag),
    };
}

/// [`ComputedValues::padding`] (`Sides<ComputedLengthPercentage>`) → [`taffy::Style::padding`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// CSS Box 3 §4.1 <https://www.w3.org/TR/css-box-3/#padding-physical> の
/// physical padding 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。margin と
/// の差は value type: padding は `<length-percentage [0,∞]>` (auto なし、
/// non-negative は raikiri-style parse-time enforce) のため
/// [`computed_length_percentage_to_taffy_length_percentage`] を使う。
///
/// Length policy は [`computed_length_percentage_to_taffy_length_percentage`] を参照。
fn bridge_padding(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let p = cv.padding;
    style.padding = Rect {
        top: computed_length_percentage_to_taffy_length_percentage(p.top, "padding", diag),
        right: computed_length_percentage_to_taffy_length_percentage(p.right, "padding", diag),
        bottom: computed_length_percentage_to_taffy_length_percentage(p.bottom, "padding", diag),
        left: computed_length_percentage_to_taffy_length_percentage(p.left, "padding", diag),
    };
}

/// [`ComputedValues::border`] (`Sides<ComputedBorder>`) → [`taffy::Style::border`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// # style-gating は **上流** で済んでいる
///
/// `border-style: none` / `hidden` の側で width を 0 にする規則は、CSS
/// Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
/// の propdef table が "Computed value: absolute length, snapped as a border
/// width; **zero if the border style is `none` or `hidden`**" と規定するとおり
/// **computed 層**の要求である。したがって gate は
/// `raikiri_style::resolve_border` が持ち、[`ComputedValues::border`] に届く
/// 時点で width は既に 0 に潰れている。
///
/// 本 bridge が同じ判定を再実装してはならない (spec 規則の二重実装になり、
/// 一方だけ直す drift の温床になる)。かつてあった `used_border_width` helper は
/// この理由で削除した。end-to-end の gating pin は本 file の
/// `apply_computed_to_style_bridges_border_to_taffy` が引き続き持つ。
///
/// `@page` 経路 (`PageCascadeResult::declarations`) も同じ `resolve_border` へ
/// funnel するようになったので、border-width の gate は
/// raikiri-style 側に 1 本しか無い。本 bridge が読むのは per-node
/// [`ComputedValues`] なので直接の影響は無いが、将来 page-margin box の layout を
/// 本 bridge に通す場合も **gate を再実装せず** 上流の値を信頼すること。
///
/// ⚠️ 上記は **border-style gating に限った話**。`PageCascadeResult::declarations`
/// は非有限 `f32` (+Inf / NaN) について本段落とは別の未対応 hazard を抱えている
/// (contract は `raikiri_style::page::PageCascadeResult::declarations`
/// の doc が canonical)。将来 page-margin box の layout を本 bridge に通す際は
/// gating の再確認だけでなくそちらも再確認すること。
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
/// - `border-image` / `border-radius` はスコープ外。
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let b = cv.border;
    style.border = Rect {
        top: computed_length_to_taffy_length_percentage(b.top.width(), diag),
        right: computed_length_to_taffy_length_percentage(b.right.width(), diag),
        bottom: computed_length_to_taffy_length_percentage(b.bottom.width(), diag),
        left: computed_length_to_taffy_length_percentage(b.left.width(), diag),
    };
}

/// [`ComputedValues::width`] / [`ComputedValues::height`] (`ComputedLengthPercentageOrAuto`) →
/// [`taffy::Style::size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §3.1.1
/// "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
///
/// width 側を先に landing、続けて height 側を追記して両 preferred size 軸を
/// full-bridge にした。両 field を同時に書き込むため struct literal
/// (`style.size = Size { width, height }`) で 1 発 assign する — partial write
/// scaffold はもう不要になった。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_dimension`] を参照。
///
/// # PageBox 妥協
///
/// `<body>` element の `style.size` は本 bridge の後、[`apply_page_box_to_body`]
/// で PageBox の値 (width / height 両方) に上書きされる (layout.rs Step 1 →
/// Step 4)。したがって `<body style="width: 100px; height: 200px">` の author
/// 値は本 helper で一度 taffy に write されるが、Step 4 で PageBox 値に
/// clobber される — 現行実装で意図された挙動 (将来 @page cascade + per-page
/// PageBox に refactor 予定)。width 側 clobber の author→PageBox 上書き経路は
/// test `apply_page_box_clobbers_body_width_from_bridge` が pin する。height
/// 側は [`apply_page_box_to_body`] が `style.size = Size { width, height }` の
/// struct literal で **field を分岐なく一括代入する** ため、width と同じ
/// clobber 経路を通る (両 field は同一 statement で書かれる)。同 helper の
/// PageBox output pin は test `apply_page_box_to_body_sets_body_style_size_to_page_dimensions`
/// が担う (author→PageBox の bridge→clobber 連鎖 test は width 側で十分、
/// 冗長化を避け height 側は structural 保証に留める)。
fn bridge_size(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    // width + height 両方を同時に書くので struct literal を採用。
    // (以前の field-assign scaffold は一時形態で、
    //  もはや保つ必要がない — default 保持は cv 側で `Auto` を返せば自然に達成。)
    style.size = Size {
        width: computed_length_percentage_or_auto_to_taffy_dimension(cv.width, "width", diag),
        height: computed_length_percentage_or_auto_to_taffy_dimension(cv.height, "height", diag),
    };
}

/// [`raikiri_style::property::BoxSizing`] → [`taffy::BoxSizing`] bridge。
///
/// CSS Sizing 3 §3.3 "Box Edges for Sizing: the box-sizing property"
/// <https://www.w3.org/TR/css-sizing-3/#box-sizing>: value grammar
/// `content-box | border-box`、spec initial `content-box`。**enum 1:1 mapping**
/// (Length policy に不参加の最小 helper)。
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
/// raikiri-style の [`BoxSizing`] は `#[non_exhaustive]`
/// (forward-compat のための sibling pattern)。未知 variant は spec initial (`ContentBox`) に
/// fail-quiet — spec-violation を silent に伸ばさないよう "最も安全な既定"
/// にする方針 ([`bridge_display`] catch-all → `Block` と同じ趣旨)。
///
/// taffy 側 (`taffy::BoxSizing`) は `#[non_exhaustive]` **ではない** ため、
/// mapping 出力 arm は `ContentBox` / `BorderBox` の 2 個で網羅済。
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

/// flex container/item property → [`taffy::Style`] bridge。
///
/// CSS Flexible Box Layout Module Level 1 の 5 property を対応する taffy
/// field へ写す:
///
/// - `flex-direction` ([`FlexDirectionValue`]) → `style.flex_direction`
///   (§5.1)
/// - `flex-wrap` ([`FlexWrapValue`]) → `style.flex_wrap` (§5.2)
/// - `flex-grow` (`f32`) → `style.flex_grow` (§7.2.1、無変換で直接 copy —
///   `<number>` は spec 上絶対化を要さない、[`ComputedValues::flex_grow`]
///   doc の sink-guard 注記参照)
/// - `flex-shrink` (`f32`) → `style.flex_shrink` (§7.2.2、同上)
/// - `flex-basis` ([`ComputedFlexBasis`]) → `style.flex_basis`
///   (`taffy::Dimension`、§7.2.3)
///
/// # `flex-basis` の bridge
///
/// `Px` / `Percent` は [`bridge_size`] と同じ percent `/100.0` 変換 +
/// [`sanitize_taffy`] 非有限 guard で [`taffy::Dimension`] に載せる。
/// `MinContent` / `MaxContent` / `FitContent` は taffy 0.14 の同名
/// `Dimension` variant にそのまま写像する (flex アルゴリズムが content
/// 測定で解決する)。
///
/// - `Content` → [`taffy::Dimension::content`] (0.14 — flex-basis 専用
///   keyword、そのままの意味で写像する)。
fn bridge_flex(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.flex_direction = match cv.flex_direction {
        FlexDirectionValue::Row => TaffyFlexDirection::Row,
        FlexDirectionValue::RowReverse => TaffyFlexDirection::RowReverse,
        FlexDirectionValue::Column => TaffyFlexDirection::Column,
        FlexDirectionValue::ColumnReverse => TaffyFlexDirection::ColumnReverse,
        // cov:ignore: unreachable while FlexDirectionValue is
        // Row|RowReverse|Column|ColumnReverse only; required for its
        // #[non_exhaustive] contract (see doc above, `bridge_display`
        // catch-all と同じ趣旨).
        _ => TaffyFlexDirection::Row,
    };
    style.flex_wrap = match cv.flex_wrap {
        FlexWrapValue::NoWrap => TaffyFlexWrap::NoWrap,
        FlexWrapValue::Wrap => TaffyFlexWrap::Wrap,
        FlexWrapValue::WrapReverse => TaffyFlexWrap::WrapReverse,
        // cov:ignore: unreachable while FlexWrapValue is
        // NoWrap|Wrap|WrapReverse only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyFlexWrap::NoWrap,
    };
    // `<number>` は絶対化不要 — 無変換で直接 copy。
    style.flex_grow = cv.flex_grow;
    style.flex_shrink = cv.flex_shrink;
    // `flex-basis: content` (CSS Flexible Box Layout 1 §4.5) is distinct
    // from `auto` in principle — it always uses the item's content size as
    // the flex basis, even when `width`/`height` are also set, whereas
    // `auto` defers to `width`/`height` first and only falls back to
    // `content` maps to taffy's own `Dimension::content` (0.14 — "only
    // valid for flex-basis", exactly this property's keyword).
    // `min-content` / `max-content` / bare `fit-content` map to taffy's own
    // intrinsic `Dimension` variants (0.14), which the flex algorithm
    // resolves through content measurement.
    style.flex_basis = match cv.flex_basis {
        ComputedFlexBasis::Auto => taffy::Dimension::auto(),
        ComputedFlexBasis::Content => taffy::Dimension::content(),
        ComputedFlexBasis::MinContent => taffy::Dimension::min_content(),
        ComputedFlexBasis::MaxContent => taffy::Dimension::max_content(),
        ComputedFlexBasis::FitContent => taffy::Dimension::fit_content(),
        ComputedFlexBasis::Px(v) => taffy::Dimension::length(sanitize_taffy(v, "flex-basis", diag)),
        ComputedFlexBasis::Percent(p) => {
            taffy::Dimension::percent(sanitize_taffy(p / 100.0, "flex-basis", diag))
        }
    };
}

/// alignment property → [`taffy::Style`] bridge (CSS Box Alignment Module
/// Level 3)。
///
/// - `justify-content` ([`ContentAlignmentValue`]) → `style.justify_content`
///   (§5.1)
/// - `align-content` ([`ContentAlignmentValue`]) → `style.align_content`
///   (§5.1)
/// - `align-items` ([`SelfAlignmentValue`]) → `style.align_items` (§7.2)
/// - `align-self` ([`AlignSelfValue`]) → `style.align_self` (§6.2)
/// - `justify-items` ([`SelfAlignmentValue`]) → `style.justify_items` (§7.1)
///   — grid container 上の inline-axis 版 `align-items`、同じ keyword set
///   ([`SelfAlignmentValue`] doc 参照) を共有する。
/// - `justify-self` ([`AlignSelfValue`]) → `style.justify_self` (§6.1) —
///   grid item 上の inline-axis 版 `align-self`。taffy `GridItemStyle::justify_self`
///   の doc も `align_self` と同じ "Falls back to the parents … if not set"
///   契約を持つため、`align-self` と同じ `normal`/`auto` 特殊 handling が
///   そのまま適用できる。
///
/// taffy 側の 6 field は全て `Option<…>` — CSS の `normal` (content-*系)
/// keyword には対応する taffy keyword が無く、`None` へ写す
/// (`bridge_box_sizing` の "Initial-value 補正" 節と同型の initial-value
/// mismatch)。taffy はその後 layout mode 依存の default で埋める
/// (`GridContainerStyle::grid_align_content` 等の `unwrap_or(AlignContent::STRETCH)`
/// — grid path の fallback は `STRETCH`、flex path は
/// `compute_flexbox_layout` 内部の別 default)。`align-self`/`justify-self`
/// の `auto` も同じ形 (`Option::None` → 親の `align-items`/`justify-items`
/// に fallback、CSS Box Alignment 3 §6.2/§6.1 の spec 規定どおり)。
///
/// 明示 keyword は [`taffy::AlignItems`] / [`taffy::AlignContent`] の
/// 定数 (`AlignItems::CENTER` 等、struct constant であって enum variant
/// ではない — taffy 0.12 の safe/unsafe overflow-position 分離 shape、
/// [`ContentAlignmentValue`]/[`SelfAlignmentValue`] doc の scope carving
/// 節参照) に写す。`safety` field は常に `AlignmentSafety::Unsafe` —
/// `safe`/`unsafe` prefix 自体を本 crate が受理していないため
/// (`parse_content_alignment`/`parse_self_alignment` doc 参照)。
fn bridge_alignment(style: &mut taffy::Style, cv: &ComputedValues) {
    style.justify_content = content_alignment_to_taffy(cv.justify_content);
    style.align_content = content_alignment_to_taffy(cv.align_content);
    style.align_items = self_alignment_to_taffy(cv.align_items);
    style.align_self = self_alignment_or_auto_to_taffy(cv.align_self);
    style.justify_items = self_alignment_to_taffy(cv.justify_items);
    style.justify_self = self_alignment_or_auto_to_taffy(cv.justify_self);
}

/// [`AlignSelfValue`] → `Option<taffy::AlignItems>` mapping — shared by
/// `align-self` and `justify-self` ([`bridge_alignment`] doc の
/// `justify-self` 節参照、taffy 側 `AlignSelf` は `AlignItems` の type
/// alias)。
fn self_alignment_or_auto_to_taffy(v: AlignSelfValue) -> Option<TaffyAlignItems> {
    match v {
        AlignSelfValue::Auto => None,
        // `normal` は `auto` とは異なり、親の align-items/justify-items へ
        // fallback せず、flex/grid layout では単独で `stretch` 相当に
        // 振る舞う (CSS Box Alignment 3 §8.3 の "In flex layout, this
        // value behaves as stretch" — grid layout も同節の対象)。
        // `self_alignment_to_taffy`'s `Normal => None` mapping はここでは
        // 使えない — taffy の `None` は「コンテナの対応 property を
        // 継承する」意味 (`auto` の spec 挙動そのもの) であり、`normal` の
        // 「親の値に関わらず stretch」とは異なるため、明示的に `STRETCH`
        // へ写す。
        AlignSelfValue::Value(SelfAlignmentValue::Normal) => Some(TaffyAlignItems::STRETCH),
        AlignSelfValue::Value(v) => self_alignment_to_taffy(v),
        // cov:ignore: unreachable while AlignSelfValue is Auto|Value(_)
        // only; required for its #[non_exhaustive] contract
        // (`self_alignment_to_taffy` の catch-all と同じ判断).
        _ => None,
    }
}

/// [`ContentAlignmentValue`] → `Option<taffy::AlignContent>` mapping —
/// [`bridge_alignment`] の `justify_content` / `align_content` 両 field で
/// 共有する ([`ContentAlignmentValue`] 自体が両 property の共有 payload
/// 型であるのと同じ理由)。
fn content_alignment_to_taffy(v: ContentAlignmentValue) -> Option<TaffyAlignContent> {
    match v {
        ContentAlignmentValue::Normal => None,
        ContentAlignmentValue::Stretch => Some(TaffyAlignContent::STRETCH),
        ContentAlignmentValue::SpaceBetween => Some(TaffyAlignContent::SPACE_BETWEEN),
        ContentAlignmentValue::SpaceEvenly => Some(TaffyAlignContent::SPACE_EVENLY),
        ContentAlignmentValue::SpaceAround => Some(TaffyAlignContent::SPACE_AROUND),
        ContentAlignmentValue::Center => Some(TaffyAlignContent::CENTER),
        ContentAlignmentValue::Start => Some(TaffyAlignContent::START),
        ContentAlignmentValue::End => Some(TaffyAlignContent::END),
        ContentAlignmentValue::FlexStart => Some(TaffyAlignContent::FLEX_START),
        ContentAlignmentValue::FlexEnd => Some(TaffyAlignContent::FLEX_END),
        // cov:ignore: unreachable while ContentAlignmentValue is the 10
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// [`SelfAlignmentValue`] → `Option<taffy::AlignItems>` mapping —
/// [`bridge_alignment`] の `align_items` field と `align_self` の
/// `AlignSelfValue::Value` branch で共有する (`align-self` の非-`auto` 値は
/// `align-items` と同じ keyword set、[`AlignSelfValue`] doc 参照)。
fn self_alignment_to_taffy(v: SelfAlignmentValue) -> Option<TaffyAlignItems> {
    match v {
        SelfAlignmentValue::Normal => None,
        SelfAlignmentValue::Stretch => Some(TaffyAlignItems::STRETCH),
        SelfAlignmentValue::Center => Some(TaffyAlignItems::CENTER),
        SelfAlignmentValue::Start => Some(TaffyAlignItems::START),
        SelfAlignmentValue::End => Some(TaffyAlignItems::END),
        SelfAlignmentValue::FlexStart => Some(TaffyAlignItems::FLEX_START),
        SelfAlignmentValue::FlexEnd => Some(TaffyAlignItems::FLEX_END),
        SelfAlignmentValue::Baseline => Some(TaffyAlignItems::BASELINE),
        // cov:ignore: unreachable while SelfAlignmentValue is the 8
        // variants matched above only; required for its #[non_exhaustive]
        // contract.
        _ => None,
    }
}

/// `row-gap` / `column-gap` → [`taffy::Style::gap`] (`Size<LengthPercentage>`)
/// bridge。
///
/// CSS Box Alignment Module Level 3 §8.1 "Row and Column Gutters: the
/// row-gap and column-gap properties"
/// <https://www.w3.org/TR/css-align-3/#column-row-gap> propdef の
/// "Initial: normal" に対し、同 propdef の value 説明 (verbatim) が:
///
/// > The value `normal` represents a used value of `1em` on multi-column
/// > containers, and a used value of `0px` in all other contexts.
///
/// と規定する — flex/grid container はこの "all other contexts" に属する。
/// これは [`bridge_box_sizing`] の "Initial-value 補正" 節と同型の
/// mismatch: raikiri-style の computed 層 initial は `normal` keyword を
/// 保持する ([`ComputedLengthPercentageOrNormal::Normal`]) が、taffy 側
/// `Size<LengthPercentage>` (non-`Option`) には `normal` keyword が
/// 存在しないため、本 bridge が `normal → 0` の変換を担う。
///
/// `Px` / `Percent` は [`ComputedLengthPercentage`] に一度変換してから
/// [`computed_length_percentage_to_taffy_length_percentage`] (percent の
/// `/100.0` 変換 + [`sanitize_taffy`] 非有限 guard を含む) へ delegate する
/// — [`bridge_padding`] と同じ helper を再利用し、実装を複製しない。
fn bridge_gap(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    style.gap = Size {
        width: computed_gap_component_to_taffy(cv.column_gap, "column-gap", diag),
        height: computed_gap_component_to_taffy(cv.row_gap, "row-gap", diag),
    };
}

/// [`bridge_gap`] の per-axis helper — `taffy::Size<LengthPercentage>` の
/// `width` は inline axis (`column-gap` 相当)、`height` は block axis
/// (`row-gap` 相当) に対応する (taffy `Size` の一般 convention、
/// [`bridge_size`] の `width`/`height` mapping と同じ軸の向き)。
fn computed_gap_component_to_taffy(
    gap: ComputedLengthPercentageOrNormal,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    let lp = match gap {
        ComputedLengthPercentageOrNormal::Normal => ComputedLengthPercentage::Px(0.0),
        ComputedLengthPercentageOrNormal::Px(v) => ComputedLengthPercentage::Px(v),
        ComputedLengthPercentageOrNormal::Percent(p) => ComputedLengthPercentage::Percent(p),
    };
    computed_length_percentage_to_taffy_length_percentage(lp, site, diag)
}

/// grid container/item property → [`taffy::Style`] bridge (CSS Grid Layout
/// Module Level 1)。
///
/// - `grid-template-columns` / `grid-template-rows` → `style.grid_template_columns`
///   / `style.grid_template_rows` (`Vec<GridTemplateComponent>`、§7.2) +
///   `style.grid_template_column_names` / `style.grid_template_row_names`
///   (line names outside any `repeat()`、§7.2.2 — [`grid_template_tracks_to_taffy`]
///   doc の shape 対応参照)
/// - `grid-template-areas` → `style.grid_template_areas`
///   (`Vec<GridTemplateArea>`、§7.3)
/// - `grid-auto-columns` / `grid-auto-rows` → `style.grid_auto_columns` /
///   `style.grid_auto_rows` (§7.6)
/// - `grid-auto-flow` → `style.grid_auto_flow` (§7.7)
/// - `grid-row-start`/`grid-row-end` / `grid-column-start`/`grid-column-end`
///   → `style.grid_row` / `style.grid_column` (`Line<GridPlacement>`、§8.3)
fn bridge_grid(style: &mut taffy::Style, cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    let (columns, column_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_columns, "grid-template-columns", diag);
    style.grid_template_columns = columns;
    style.grid_template_column_names = column_names;
    let (rows, row_names) =
        grid_template_tracks_to_taffy(&cv.grid_template_rows, "grid-template-rows", diag);
    style.grid_template_rows = rows;
    style.grid_template_row_names = row_names;

    style.grid_template_areas = match &cv.grid_template_areas {
        GridTemplateAreasValue::None => None,
        GridTemplateAreasValue::Areas(areas) => Some(taffy::style::GridTemplateAreas {
            areas: areas
                .areas
                .iter()
                .map(|a| TaffyGridTemplateArea {
                    name: a.name.to_string(),
                    row_start: saturate_u16(a.row_start),
                    row_end: saturate_u16(a.row_end),
                    column_start: saturate_u16(a.column_start),
                    column_end: saturate_u16(a.column_end),
                })
                .collect(),
            // taffy clamps grid dimensions to 10,000 tracks; saturate our
            // u32 counts the same way as the area coordinates above.
            row_count: areas.row_count.min(u16::MAX as u32) as u16,
            column_count: areas.column_count.min(u16::MAX as u32) as u16,
        }),
        // cov:ignore: unreachable while GridTemplateAreasValue is
        // None|Areas(_) only; required for its #[non_exhaustive] contract
        // (see `bridge_display` catch-all doc).
        _ => None,
    };

    style.grid_auto_columns = cv
        .grid_auto_columns
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-columns", diag))
        .collect();
    style.grid_auto_rows = cv
        .grid_auto_rows
        .iter()
        .copied()
        .map(|t| grid_track_size_to_taffy(t, "grid-auto-rows", diag))
        .collect();

    style.grid_auto_flow = match cv.grid_auto_flow {
        GridAutoFlowValue::Row => TaffyGridAutoFlow::Row,
        GridAutoFlowValue::Column => TaffyGridAutoFlow::Column,
        GridAutoFlowValue::RowDense => TaffyGridAutoFlow::RowDense,
        GridAutoFlowValue::ColumnDense => TaffyGridAutoFlow::ColumnDense,
        // cov:ignore: unreachable while GridAutoFlowValue is
        // Row|Column|RowDense|ColumnDense only; required for its
        // #[non_exhaustive] contract (see `bridge_display` catch-all doc).
        _ => TaffyGridAutoFlow::Row,
    };

    style.grid_row = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_row_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_row_end),
    };
    style.grid_column = TaffyLine {
        start: grid_line_value_to_taffy_placement(&cv.grid_column_start),
        end: grid_line_value_to_taffy_placement(&cv.grid_column_end),
    };
}

/// [`ComputedGridTemplateTracks`] → taffy's `(Vec<GridTemplateComponent>,
/// Vec<Vec<String>>)` pair — [`bridge_grid`]'s `grid_template_columns`/
/// `grid_template_rows` field **and** their paired `_names` field share one
/// helper because taffy's `NamedLineResolver` consumes both in lock-step
/// (one name-list entry per top-level component, plus one trailing entry
/// after the last — see taffy 0.12's `NamedLineResolver::new`, which zips
/// `grid_template_columns()`/`grid_template_rows()` against
/// `grid_template_column_names()`/`grid_template_row_names()`). This is
/// exactly [`crate` `raikiri_style`]'s own `ComputedGridTrackList::line_names`
/// interleave convention (`line_names.len() == components.len() + 1`), so
/// the two Vecs below are built from the same source data without any
/// reindexing.
///
/// `none` maps to a pair of empty `Vec`s — taffy's own default for a
/// container with no explicit track list.
fn grid_template_tracks_to_taffy(
    tracks: &ComputedGridTemplateTracks,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> (Vec<GridTemplateComponent<String>>, Vec<Vec<String>>) {
    match tracks {
        ComputedGridTemplateTracks::None => (Vec::new(), Vec::new()),
        ComputedGridTemplateTracks::List(list) => {
            let components = list
                .components
                .iter()
                .cloned()
                .map(|component| match component {
                    ComputedGridTrackListComponent::Size(size) => {
                        GridTemplateComponent::Single(grid_track_size_to_taffy(size, site, diag))
                    }
                    ComputedGridTrackListComponent::Repeat(repeat) => {
                        GridTemplateComponent::Repeat(GridTemplateRepetition {
                            count: grid_repeat_count_to_taffy(repeat.count),
                            tracks: repeat
                                .tracks
                                .into_iter()
                                .map(|t| grid_track_size_to_taffy(t, site, diag))
                                .collect(),
                            line_names: repeat
                                .line_names
                                .into_iter()
                                .map(|names| names.iter().map(ToString::to_string).collect())
                                .collect(),
                        })
                    }
                })
                .collect();
            let line_names = list
                .line_names
                .iter()
                .map(|names| names.iter().map(ToString::to_string).collect())
                .collect();
            (components, line_names)
        }
    }
}

/// [`GridRepeatCount`] → [`taffy::RepetitionCount`] mapping. `Count`'s
/// `<integer [1,∞]>` payload is `u32` at the raikiri-style layer (CSS
/// Values 4 §4.2 places no upper bound on `<integer>`) but taffy's
/// `RepetitionCount::Count` is `u16` — silently saturating at
/// [`u16::MAX`] (65535 repetitions) rather than adding new
/// [`LayoutWarn`] machinery for an author value that large, which would
/// already make taffy's own track-sizing algorithm impractically slow
/// long before this cast is reached.
fn grid_repeat_count_to_taffy(count: GridRepeatCount) -> TaffyRepetitionCount {
    match count {
        GridRepeatCount::Count(n) => TaffyRepetitionCount::Count(saturate_u16(n)),
        GridRepeatCount::AutoFill => TaffyRepetitionCount::AutoFill,
        GridRepeatCount::AutoFit => TaffyRepetitionCount::AutoFit,
        // cov:ignore: unreachable while GridRepeatCount is
        // Count|AutoFill|AutoFit only; required for its #[non_exhaustive]
        // contract.
        _ => TaffyRepetitionCount::Count(1),
    }
}

/// [`ComputedGridTrackSize`] → [`taffy::TrackSizingFunction`]
/// (`MinMax<MinTrackSizingFunction, MaxTrackSizingFunction>`) mapping.
///
/// `FitContent` has no bare-breadth min side in taffy's model (only
/// `MaxTrackSizingFunction::fit_content_px`/`fit_content_percent` exist) —
/// the CSS spec formula `max(minimum, min(limit, max-content))` treats the
/// min side as `auto`, which is what `MinTrackSizingFunction::auto()`
/// supplies here.
fn grid_track_size_to_taffy(
    size: ComputedGridTrackSize,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> TrackSizingFunction {
    match size {
        ComputedGridTrackSize::Breadth(b) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(b, site, diag),
            max: grid_track_breadth_to_taffy_max(b, site, diag),
        },
        ComputedGridTrackSize::MinMax(min, max) => TrackSizingFunction {
            min: grid_track_breadth_to_taffy_min(min, site, diag),
            max: grid_track_breadth_to_taffy_max(max, site, diag),
        },
        ComputedGridTrackSize::FitContent(lp) => {
            let max = match lp {
                ComputedLengthPercentage::Px(v) => {
                    MaxTrackSizingFunction::fit_content_px(sanitize_taffy(v, site, diag))
                }
                ComputedLengthPercentage::Percent(p) => {
                    MaxTrackSizingFunction::fit_content_percent(sanitize_taffy(
                        p / 100.0,
                        site,
                        diag,
                    ))
                }
            };
            TrackSizingFunction {
                min: MinTrackSizingFunction::auto(),
                max,
            }
        }
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MinTrackSizingFunction`] mapping
/// (`minmax()`'s min side, or the min side of a bare `<track-breadth>`
/// widened to `MinMax { min, max }` — [`grid_track_size_to_taffy`]'s
/// `Breadth` arm). `Flex` never actually reaches this function
/// ([`ComputedGridTrackBreadth`] doc's collapse note — the min side is
/// only ever produced by `resolve_grid_inflexible_breadth`, which has no
/// `Flex` variant to produce it from); the arm below is a defensive
/// fallback to keep the match exhaustive, not a reachable case.
fn grid_track_breadth_to_taffy_min(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MinTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MinTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MinTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::MinContent => MinTrackSizingFunction::min_content(),
        B::MaxContent => MinTrackSizingFunction::max_content(),
        B::Auto => MinTrackSizingFunction::auto(),
        // cov:ignore: see this fn's doc — structurally unreachable, kept
        // only for match exhaustiveness.
        B::Flex(_) => MinTrackSizingFunction::auto(),
    }
}

/// [`ComputedGridTrackBreadth`] → [`taffy::MaxTrackSizingFunction`] mapping
/// (`minmax()`'s max side, or the max side of a bare `<track-breadth>` —
/// [`grid_track_size_to_taffy`]'s `Breadth` arm). Unlike
/// [`grid_track_breadth_to_taffy_min`], `Flex` (`fr`) is a real, reachable
/// case here — the `fr` unit is only valid on the max side of `minmax()`
/// or as a bare `<track-breadth>` (CSS Grid Layout Module Level 1 §7.2.4).
fn grid_track_breadth_to_taffy_max(
    b: ComputedGridTrackBreadth,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> MaxTrackSizingFunction {
    use ComputedGridTrackBreadth as B;
    match b {
        B::Px(v) => MaxTrackSizingFunction::length(sanitize_taffy(v, site, diag)),
        B::Percent(p) => MaxTrackSizingFunction::percent(sanitize_taffy(p / 100.0, site, diag)),
        B::Flex(f) => MaxTrackSizingFunction::fr(sanitize_taffy(f, site, diag)),
        B::MinContent => MaxTrackSizingFunction::min_content(),
        B::MaxContent => MaxTrackSizingFunction::max_content(),
        B::Auto => MaxTrackSizingFunction::auto(),
    }
}

/// [`GridLineValue`] → [`taffy::GridPlacement`] mapping (CSS Grid Layout
/// Module Level 1 §8.3). [`GridLineValue::Named`] (bare `<custom-ident>`,
/// no explicit index) maps to `NamedLine` with index `1` explicitly filled
/// in — same as [`GridLineValue`]'s own doc explains: taffy 0.12 treats
/// index `0` as an "unspecified" sentinel and normalizes it to `1`
/// internally (`NamedLineResolver::find_line_index`'s `if idx == 0 { idx =
/// 1; }`), so filling in `1` here ahead of time is behavior-identical.
///
/// `Line`/`Span`/`NamedSpan`'s `i32`/`u32` payloads are widened from parse
/// time's unbounded `<integer>` (CSS Values 4 §4.2) but taffy's
/// `GridLine`/`Span(u16)`/`NamedSpan(_, u16)` use 16-bit integers —
/// silently saturating at [`i16::MIN`]/[`i16::MAX`]/[`u16::MAX`] rather
/// than adding new [`LayoutWarn`] machinery, same rationale as
/// [`grid_repeat_count_to_taffy`].
fn grid_line_value_to_taffy_placement(v: &GridLineValue) -> GridPlacement {
    match v {
        GridLineValue::Auto => GridPlacement::Auto,
        GridLineValue::Line(n) => taffy_style_helpers::line(saturate_i16(*n)),
        GridLineValue::Named(name) => GridPlacement::NamedLine(name.to_string(), 1),
        GridLineValue::NamedLine(name, n) => {
            GridPlacement::NamedLine(name.to_string(), saturate_i16(*n))
        }
        GridLineValue::Span(n) => GridPlacement::Span(saturate_u16(*n)),
        GridLineValue::SpanNamed(name, n) => {
            GridPlacement::NamedSpan(name.to_string(), saturate_u16(*n))
        }
        // cov:ignore: unreachable while GridLineValue is the 6 variants
        // matched above only; required for its #[non_exhaustive] contract.
        _ => GridPlacement::Auto,
    }
}

/// 最小限の inline formatting context を、条件を満たす block container に
/// 確立する。`<br>` を含まない場合は単一行・non-wrapping (下記 "実現方法")
/// だが、display:none ではない `<br>` を 1 個以上含む場合は forced break
/// だけで少なくともその個数 + 1 個の (視覚上高さを持つ) line に分かれる
/// (下記 "`<br>` forced break")。先頭・末尾・連続する `<br>` はそれ自身の
/// line が高さ 0 に潰れるため、実際の見た目上の line 数はこれより少なく
/// なりうる一方、`<br>` を含む container は `flex_wrap: Wrap` も同時に
/// 有効になる副作用を持つため、それとは独立な size-based wrapping が
/// 追加の line を発生させ、より多くなることもある — 上限はない (同
/// section の doc および Non-goals 参照)。
///
/// CSS 2.1 §9.4.2 <https://www.w3.org/TR/CSS21/visuren.html#inline-formatting>:
/// "a block container either contains only block-level boxes or
/// establishes an inline formatting context and thus contains only
/// inline-level boxes."(block container は block-level box のみを含むか、
/// inline formatting context を確立して inline-level box のみを含む)。
/// ここで node が条件を満たすのは、自身の computed display
/// ([`DisplayValue`]) が [`DisplayValue::Block`] または
/// [`DisplayValue::InlineBlock`] であり (両方とも自身の content に対して
/// block container を生成する — `inline-block` は CSS Display 3 §2 上
/// "inline flow-root" (plain な `block` の "block flow" とは別の、独立
/// した formatting context を確立する概念、[`DisplayValue::InlineBlock`]
/// 自身の doc 参照) だが、本 pass の qualifying condition と扱いはこの
/// 区別をしない — どちらも「自身の content area が block container で
/// ある」という一点のみを見る)、**かつ** in-document な children が
/// **2 個以上**あり、それら
/// 全てが inline-level である場合 ([`NodeKind::Text`] の child、または
/// [`DisplayValue`] が [`DisplayValue::Inline`] / [`DisplayValue::InlineBlock`]
/// な [`NodeKind::Element`] の child。[`DisplayValue::None`] の child は
/// 無視する — count にも disqualification にも数えない)。
/// [`NodeKind::Text`] の child は無条件に count する
/// (whitespace のみを保持する text node も含む) — HTML parser は source の
/// indentation から sibling tag 間にこうした text node をよく生成し、CSS
/// 2.1 §9.2.1.1 上、text node の content は保持する文字に関わらず
/// ordinary な inline-level content である (`white-space` の collapsing は
/// rendering 時の関心事であり、formatting context の関心事ではない) ため、
/// "本物の" inline な child が 1 個だけの container でも、付随する parser
/// の whitespace により実質的にすでに 2 個以上の qualifying な children
/// を持つことが多い。inline-level な child が 1 個だけの場合は、plain な
/// block path のままにする: line 上に box が 1 個しかなければ隣に置く
/// ものが無く、taffy の block layout も 1 個だけの child を
/// `align-items: flex-start` な 1-item flex row と同じ位置に置く
/// (どちらも container を child 自身の margin box に合わせて size し、
/// cross-axis 方向の処理が不要な点も同じ) — したがってこの case には
/// 埋めるべき behavioral gap が無く、既存の block code path を乱す理由も
/// 無い。
///
/// plain な [`DisplayValue::Inline`] の node は、自身の children が
/// inline-level 2 個以上でも本条件を満たさ**ない**: `block` /
/// `inline-block` と異なり、non-replaced な `inline` box は CSS 2.1
/// §9.2.1.1 上、自身の content に対して block container を生成しない —
/// その content は `inline` box 自身と*同じ* inline formatting context
/// に流れ込む ordinary な inline-level content である (nested な
/// inline box は新しい formatting context を開始しない)。本 pass は
/// element の境界を越えた nested inline content の flatten は行わない
/// (下記 "Non-goals" 参照) ため、`inline` な node の children は
/// [`bridge_display`] が mapping した状態のまま、本 pass によって変更
/// されない。
///
/// block-level / flex / grid な in-document child を 1 個でも持つ
/// container も同様に変更しない (block-level と inline-level が混在する
/// content は、CSS 2.1 §9.2.1.1 に従い inline-level の run を囲む
/// anonymous block box の生成が必要になるが、本 minimal pass はそこまで
/// 対応しない)。
///
/// # 実現方法
///
/// taffy には inline layout mode が無いため、line box は 1 行・
/// non-wrapping な flex container として実現する: [`Display::Flex`] +
/// `flex_direction: Row` (taffy 自身の default と同じ) + `flex_wrap:
/// NoWrap` (同上) により、参加する children を左から右へ 1 行に並べる —
/// これは CSS 2.1 §9.4.2 が inline formatting context の box について
/// 述べる内容 ("laid out horizontally, one after the other, beginning at
/// the top of a containing block") そのものである。`flex_direction` /
/// `flex_wrap` を default に委ねず明示的に set しているのは、この node
/// が block container だった間は inert だった author の
/// `flex-direction` / `flex-wrap` 宣言を [`bridge_flex`] が既に copy
/// 済みの可能性があり、ここで flex container になった時点でそれが
/// inert でなくなるため — default のままだろうと期待するのではなく
/// 明示的に上書きする必要がある。
///
/// `align_items: FlexStart` (taffy 自身の default である `Stretch` では
/// なく) を set しているのは、上記の `flex_direction` / `flex_wrap` と
/// 全く同じ「もう inert ではない」理由による: `bridge_alignment` も
/// author の `align-items` 宣言をこの node に既に copy している可能性が
/// あり、block container だった間は inert だったものが、ここで flex
/// container になった時点で live になるため、同様に上書きする必要が
/// ある。`FlexStart` そのもの (`Stretch` ではなく) を選ぶ理由は、参加する
/// 各 child を最も高い sibling に合わせて stretch せず、各自の natural
/// な (measured) height のまま保つことで、CSS 2.1 §9.4.2 の "line box is
/// always tall enough for all of the boxes it contains"(line box は
/// 自身が含む全 box を収められるだけの高さを常に持つ) を、より低い box
/// を無理に伸ばさずに満たすためである。
///
/// `FlexStart` も taffy の `AlignItems::Baseline` も、ここでは本当の
/// font-baseline alignment を行わない: 本 crate の taffy leaf layout は
/// alignment の基準となる baseline を一切 report しない (`taffy_impl.rs`
/// の leaf measure closure は `Size` しか返さない) ため、`Baseline` は
/// 各 item の下端 (bottom edge) 揃えに degrade し、`FlexStart` は各
/// item の上端 (top edge) 揃えになる — どちらも近似であって、「本物の
/// baseline mode」と「fallback」の選択ではない。`FlexStart` を選んだのは、
/// 参加する children が同じ used line-height / ascent metric を共有する
/// 場合 (同じ font、同じ computed font-size、同じ line-height という
/// 一般的な case) には正しい baseline alignment と一致し、共有しない
/// 場合 (例えば `<sub>`/`<sup>` の child が smaller な computed
/// font-size を使う場合、あるいは通常の text 以外の box model) にのみ
/// そこから外れるためである。
///
/// container 自身は `justify_content: None` (taffy 自身の default、CSS
/// の `normal` 相当) へも reset する — 全く同じ「もう inert ではない」
/// 理由による: [`bridge_alignment`] が copy した author の
/// `justify-content` 宣言は、block container だった間は inert だったが、
/// ここで flex container になった時点で main-axis 方向の item 配置
/// (space-between 等) を変えてしまう。CSS 2.1 の inline formatting
/// context に "line box 上の複数 box を main-axis 方向に再配置する"
/// 意味論はそもそも存在しないため、taffy 側の default に戻すことでその
/// 意味論を無効化する。`gap` (`row-gap` / `column-gap`) も同じ理由で
/// `0` へ reset する — line box は inline-level box の間に author 指定の
/// 隙間を空ける意味論を持たない (CSS 2.1 §9.4.2 の line box は隙間なく
/// box を並べるモデル) ため、[`bridge_gap`] が copy した author 値を
/// ここで無効化する。
///
/// `align_content` も `align_items: FlexStart` (上記) と同じ理由・同じ
/// 値 (`Some(FlexStart)`) へ reset する — [`bridge_alignment`] が copy
/// した author の `align-content` 宣言、あるいは author が宣言しなければ
/// `None` (taffy はこれを flexbox path では `Stretch` 相当に default
/// する) が、ここで初めて live になる。`align-content` は cross-axis
/// 方向で複数の flex line 間に余った space を配る property であり、この
/// container が author の明示的な `height` (block container のままでも
/// 常に効く、[`bridge_size`] 参照) で自身の content より高く sizing
/// される場合、reset を怠ると default の `Stretch` がその余り space を
/// 全 line (`<br>` forced break — 下記 doc section 参照 — が確立した、
/// 高さ 0 の line を含む) に均等に配ってしまい、`<br>` の line を
/// 0 より大きくして視覚的な gap を挟んでしまう。CSS 2.1 の inline
/// formatting context に "container の余り cross space を line 間に配る"
/// 意味論はそもそも存在しない (line box は各自の natural height を保つ、
/// 上記 `align_items: FlexStart` の rationale と同じ) ため、`FlexStart`
/// へ固定してその意味論ごと無効化する。
///
/// 参加する各 child はさらに `flex_grow: 0.0` / `flex_shrink: 0.0` /
/// `flex_basis: auto` / `align_self: None` も得る ([`bridge_flex`] /
/// [`bridge_alignment`] が自身の author CSS から copy した値を、上記と
/// 同じ「もう inert ではない」理由で上書きする)。`flex_grow` /
/// `flex_shrink` を `0` にする理由: line wrapping を未実装の現状、
/// container より広い content は圧縮されずに overflow するべきものである
/// — flex の default である `flex-shrink: 1` のままだと、各 child の
/// box を自身の shaped content より狭く圧縮してしまい、text の child
/// ではすでに [`preshape_text`] が shape した glyph run と box が乖離
/// する (shaped 済みの glyph は追従して縮まないため、狭くなった box から
/// overflow し、隣の child の glyph と重なりうる — 本 pass が置き換える
/// 以前の stacked-block な描画より悪化する)。`flex_basis` を `auto` へ
/// 戻す理由も同じ box/glyph 乖離を防ぐためである — 明示的な author
/// `flex-basis` は `flex_grow` / `flex_shrink` の値に関わらず box の
/// main size を直接決めてしまうため、`flex_shrink: 0` だけでは守れない
/// (自身の content 基準の flex basis に戻すことで、box は常に自身の
/// shaped content 以上の幅を持つ)。`align_self` を `None` (= `auto`) へ
/// 戻す理由は、container 側の `align_items: FlexStart` へ一貫して
/// fallback させるためである (`auto` は親の `align-items` へ fallback
/// する契約、[`bridge_alignment`] の doc 参照)。
///
/// # `<br>` forced break
///
/// HTML LS §the-br-element
/// (<https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-br-element>)
/// の `<br>` は "a line break" を表す。HTML LS の rendering section
/// (§phrasing-content-3) は `br { display-outside: newline; }` と記すが、
/// これは CSS Display 4 の `<display-outside>` production
/// (`block | inline | run-in` のみ) に存在しない illustrative な記法で、
/// raikiri-style が消費できる CSS 宣言ではない。本 pass は代わりに `<br>`
/// を tag_name で直接判別する ([`find_body`] が `tag_name() ==
/// Some("body")` を直接比較するのと同じ pattern — `<br>` の判別に
/// [`NodeKind`] へ新しい variant を追加する必要はない、`NodeKind::Element`
/// のまま扱う)。
///
/// qualify する container の participating children に、display:none
/// ではない `<br>` が 1 個以上含まれる場合、container 自身の `flex_wrap`
/// を (taffy 自身の default である) `NoWrap` から `Wrap` へ切り替え、
/// 参加する `<br>` child 自身の `flex_basis` のみを (他の child と同じ
/// `auto` ではなく) `100%` にする。他の participating child への
/// `flex_grow: 0` / `flex_shrink: 0` の適用は変わらない。
///
/// これにより、taffy 自身の flex line-packing algorithm (CSS Flexbox 1
/// §9.2 "Line Length Determination"
/// <https://www.w3.org/TR/css-flexbox-1/#algo-line-break> — multi-line
/// (`flex-wrap` が `nowrap` ではない) container の各 line は、item を
/// 1 個ずつ足しながら、container の main size を超える最初の item の
/// 手前で確定し、その item を次の line へ持ち越す。ただしその item が
/// line 上の最初の item である場合 (line がまだ空) は、超えていても
/// そのまま現在の line に置く、という例外を持つ) が `<br>` の位置で
/// 自然に line を切る: `<br>` の hypothetical main size が container
/// 幅の 100% であるため、すでに他の item が乗っている line には決して
/// 収まらず新しい line へ move する一方、`<br>` 自身が (空の) line の
/// 先頭に来た場合は上記例外でその line にそのまま置かれる。
/// `<br>` が単独で占有するその line 自身の高さ (cross size、row 方向
/// なので `flex_basis` が支配しない軸) は
/// [`compute_leaf_layout`](taffy::compute_leaf_layout) の測定に委ねられる
/// — `<br>` は children を持たない leaf であり [`Node::text_layout`](crate::node::Node::text_layout) も
/// 返さないため、measure 結果は常に `0` になる (この module の
/// `LayoutPartialTree::compute_child_layout` — `taffy_impl.rs` 側 — の
/// leaf 分岐 doc 参照)。したがって `<br>` が占有する line は幅こそ
/// container 全幅だが高さ 0 で、直後の line が直前の line の下端に
/// 隙間なく続く。結果として `<br>` の前後にある run はそれぞれ別の
/// (見た目上の) line に分かれる一方、`<br>` 自身は視覚上何の高さも
/// 占めない — CSS 2.1 の forced line break の見た目の効果と一致する。
///
/// この機構は `<br>` を含む container にのみ `flex_wrap: Wrap` を
/// 付与する副作用も持つ: `<br>` を含まない container は従来通り
/// `flex_wrap: NoWrap` のままだが、`<br>` を含む container では
/// (`<br>` の前後どちらの run でも) 本来 Non-goal である
/// size-based wrapping が technically 可能になる — line 内の
/// inline-level item 群が container の available width を超えれば、
/// `<br>` の有無に関わらず taffy 自身が折り返してしまう。この delta は
/// `<br>` を含む container にのみ生じ、これまで pin されていた
/// no-wrap な挙動 (`establish_minimal_line_boxes_upgrades_qualifying_container_to_flex_row`
/// 等の regression test) は `<br>` を含まないケースなので影響を受けない。
///
/// # Non-goals (本 pass)
///
/// - size-based line wrapping は行わない: qualify する container **自身が
///   確立する line box** の個数は、display:none ではない `<br>` を含まない
///   限り、container の available width に関わらず常にちょうど 1 個になる
///   (`<br>` を含む場合の個数は上記 "`<br>` forced break" 参照 — それは
///   forced break であり、size に基づく wrap ではない)。ただしこれは
///   container レベルの取り扱いの話であり、participating な text child
///   自身が shape する glyph run が内部で複数行に折り返されないことまでは
///   意味しない (最後の bullet 参照 — [`preshape_text`] は本 pass とは
///   独立に、text ごとに page width 基準で soft-wrap する)。
/// - `<br>` 自身の box は測定上つねに `0×0` であり、「空の行が font の
///   line-height 相当の高さを持つ」という real browser の挙動を再現
///   しない。real browser でこの高さを与えているのは CSS 2.1 §10.8.1
///   <https://www.w3.org/TR/CSS21/visudet.html#strut> の "strut" —
///   各 line box の先頭に、その line box を確立した要素の font /
///   line-height を持つ幅 0 の仮想 inline box が置かれているものとして
///   扱う、という規定で、空行にも line-height 分の最小高さを与える。
///   本 pass はこの strut を line box (= 本 pass が作る flex line) に
///   対して合成しない — [`compute_leaf_layout`](taffy::compute_leaf_layout)
///   による `<br>` 自身の測定結果だけが line の cross size を決めるため、
///   これは [`establish_minimal_line_boxes`] が line box そのものを
///   (単一行・non-wrapping な場合を含め) 元から strut 抜きで組んでいる
///   ことの帰結であり、別々の special case ではない。これにより 2 個の
///   bullet で挙動が変わる:
///   (a) `<br>` が (自身の line 上で) 最初の participating item になる
///   場合 — line の先頭にある container、あるいは連続する `<br><br>` の
///   2 個目以降 — line-packing の "空の line はその最初の item を
///   そのまま受け入れる" 例外により `<br>` はそこに留まり、高さ 0 のまま
///   何も visible な空行を作らない。real browser が `<p><br>text</p>` の
///   先頭や `<br><br>` の間で作る空行の高さは、本 pass では再現しない。
///   (b) 末尾の `<br>` (後続の inline-level content が無い) も同様に
///   高さ 0 の line を追加するだけで、視覚上の余白は生まない。
///   両方とも、`<br>` が childless leaf で text layout を持たないために
///   measure が `0` を返すという同じ機構の帰結であり、別々の special
///   case ではない。
/// - nested な inline element を ancestor の line box へ flatten する
///   ことは行わない — qualify する各 container 自身の IFC は直接の
///   children のみを対象とし、さらに nested した inline の descendant
///   には及ばない (line box 内の `<b>` は、本 pass が存在しなかった時と
///   全く同じく、自身の children を独立に何らかの code path で
///   layout する)。
/// - [`preshape_text`] は本 pass の後 (`apply_computed_to_style` 完了後、
///   `layout_single_page` の後段) に、各 text node の glyph run を full
///   page width に対して shape・soft-wrap する。これは本 pass が最終的に
///   その text node に line box 上の sibling と並べて与える横幅とは
///   無関係であり、text の shaping を実際の available な inline space
///   と整合させることは follow-up work であり、ここでは行わない。
fn establish_minimal_line_boxes(doc: &mut Document, cascade: &CascadeResult) {
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let qualifies = qualifies_for_minimal_line_box(doc, idx, cascade);
        doc.nodes[idx]
            .flags
            .set(NodeFlags::IS_INLINE_ROOT, qualifies);
        if !qualifies {
            continue;
        }
        // display:none な child も含む — taffy はそのような child を
        // Display::None として layout tree から丸ごと除外するため、
        // flex_grow/flex_shrink を pin することに実害は無い (無駄では
        // あるが害はない)。
        let participating_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].is_in_document())
            .collect();
        // この container に display:none ではない `<br>` が 1 個でも
        // あるかどうか — "`<br>` forced break" (この関数の doc 参照) の
        // 適用条件そのもの。display:none な `<br>` は box を生成しないため
        // forced break として数えない — 数えてしまうと `<br>` を含まない
        // 見た目上の container まで `flex_wrap: Wrap` になり、size-based
        // wrapping が意図せず有効になってしまう (下記 doc 参照)。
        let has_forced_break = participating_children
            .iter()
            .any(|&c| is_forced_line_break(doc, c, cascade));
        {
            let style = &mut doc.nodes[idx].style;
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = if has_forced_break {
                TaffyFlexWrap::Wrap
            } else {
                TaffyFlexWrap::NoWrap
            };
            style.align_items = Some(TaffyAlignItems::FLEX_START);
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            style.justify_content = None;
            style.gap = Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            };
        }
        for c in participating_children {
            let is_break = is_forced_line_break(doc, c, cascade);
            let child_style = &mut doc.nodes[c].style;
            child_style.flex_grow = 0.0;
            child_style.flex_shrink = 0.0;
            child_style.flex_basis = if is_break {
                Dimension::percent(1.0)
            } else {
                Dimension::auto()
            };
            child_style.align_self = None;
        }
    }
}

/// `idx` (ある in-document な node) が [`establish_minimal_line_boxes`] の
/// "`<br>` forced break" (同関数の doc section 参照) として扱われるべき
/// かどうか。
///
/// `NodeKind::Element` かつ `tag_name() == Some("br")` かつ computed
/// `display` が [`DisplayValue::None`] ではないことを見る — tag_name の
/// 直接比較は [`find_body`] の `tag_name() == Some("body")` と同じ pattern
/// であり、`<br>` を判別するために [`NodeKind`] へ新しい variant を追加する
/// 必要はない。
fn is_forced_line_break(doc: &Document, idx: usize, cascade: &CascadeResult) -> bool {
    doc.nodes[idx].kind() == NodeKind::Element
        && doc.nodes[idx].tag_name() == Some("br")
        && cascade.computed[idx].display != DisplayValue::None
}

/// `idx` (ある [`NodeKind::Element`]) が
/// [`establish_minimal_line_boxes`] の minimal-line-box 処理の対象かどうか
/// — qualifying condition とその根拠は同関数の doc 参照。
fn qualifies_for_minimal_line_box(doc: &Document, idx: usize, cascade: &CascadeResult) -> bool {
    // ここでは意図的に pre-bridge の `DisplayValue` を読む
    // (post-`bridge_display` の `taffy::Display` ではない — この second
    // pass が走る時点で `Block` / `Inline` / `InlineBlock` は既に全て
    // `Display::Block` に collapse 済み)。plain な `Inline` の container
    // は qualify させては**ならない**、`Block` / `InlineBlock` のみ
    // (この module の doc 参照)。
    match cascade.computed[idx].display {
        DisplayValue::Block | DisplayValue::InlineBlock => {}
        _ => return false,
    }
    let mut inline_level_count = 0usize;
    for &c in &doc.nodes[idx].children {
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => inline_level_count += 1,
            NodeKind::Element => match cascade.computed[c].display {
                DisplayValue::Inline | DisplayValue::InlineBlock => inline_level_count += 1,
                DisplayValue::None => {}
                // block-level / flex / grid な child (および将来の
                // non_exhaustive な DisplayValue variant) は全て
                // disqualify する — block-level と inline-level の
                // 混在は対象外、この module の doc 参照。
                _ => return false,
            },
            // Comment / ProcessingInstruction / DocumentFragment /
            // Document はそもそも `is_in_document()` にならないはず
            // (`NodeData` の doc 参照) だが、その invariant を前提とせず
            // ここで defensive に fail closed する。
            // cov:ignore: unreachable — Comment/ProcessingInstruction/DocumentFragment/Document nodes always have IS_IN_DOCUMENT cleared (see document.rs's mark_in_document_flags invariant), filtered by the is_in_document() guard above; kept as a defensive fallback rather than relying on that invariant here.
            _ => return false,
        }
    }
    inline_level_count >= 2
}

/// Saturating `i32` → `i16` cast — [`grid_line_value_to_taffy_placement`]
/// doc's "silently saturating" rationale.
fn saturate_i16(n: i32) -> i16 {
    n.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Saturating `u32` → `u16` cast — [`grid_line_value_to_taffy_placement`] /
/// [`grid_repeat_count_to_taffy`] doc's "silently saturating" rationale.
fn saturate_u16(n: u32) -> u16 {
    n.min(u16::MAX as u32) as u16
}

// ---------------------------------------------------------------------------
// 非有限 f32 の guard
// ---------------------------------------------------------------------------

/// taffy に渡す幾何値の絶対値上限 (px、および percentage の fraction)。
///
/// # なぜ clamp が要るのか
///
/// author CSS は untrusted 入力である。`padding: 1e40px` は cssparser の
/// f64 → f32 変換で **+Inf** になり、`padding: 1e40em` は絶対化の乗算で
/// **+Inf**、`font-size: 0px` と組み合わせると `0.0 * inf` = **NaN** になる。
/// 極端な literal すら不要で、`font-size: 10em` を 38 段 nest するだけで
/// `16 * 10^38 > f32::MAX` から +Inf が出る。
///
/// これらは絶対化を cascade に入れるまで、`layout.rs` の
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` arm に**偶然**吸収されて
/// いた。網羅 match 化自体は正しいが、その arm は病的な数値も潰していた。
///
/// # spec 根拠 (§ title + anchor、`data-level` 実検証済)
///
/// CSS Values 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>) verbatim:
///
/// > The precision and supported range of numeric values in CSS is
/// > implementation-defined, and can vary based on the property or other
/// > context a value is used in. However, within the CSS specifications,
/// > infinite precision and range is assumed. When a value cannot be explicitly
/// > supported due to range/precision limitations, it must be converted to the
/// > closest value supported by the implementation, but how the implementation
/// > defines "closest" is implementation-defined as well.
///
/// すなわち (a) 上限を持つこと自体が spec 準拠、(b) **上限は property / context
/// ごとに違ってよい**、(c) 超過値は「実装がサポートする最も近い値」= 上限に
/// 変換する。§5 は `must be converted` と**命令形**で書いており値を捨てろとは
/// 言っていないので、declaration はそのまま生き残る。本 module が site ごとに
/// 別の上限を持つのは (b) の直接の適用である。
///
/// (「declaration を invalid にしない」という明示的な phrasing は §5 には
/// **無い** — それは §3.1 / §10.12 の文言なので、そちらから import しない。)
///
/// # §5 と §5.1 の切り分け
///
/// §5.1 "Range Restrictions and Range Definition Notation"
/// (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) の range 記法
/// (`<length-percentage [0,∞]>` 等) に対する違反は **parse 段で declaration を
/// drop** する話で、raikiri では `parse_padding_side` などが済ませている。
/// 本 guard が扱うのは **§5.1 の range 内だが実装 capacity 外**の値であり、
/// §5 の適用対象である。両者は別の layer なので混同しないこと。
///
/// 同 spec の §10.12 "Range Checking"
/// (<https://www.w3.org/TR/css-values-4/#calc-range>) は math function の
/// 結果について "the value resulting from a top-level calculation must be
/// clamped to the range allowed in the target context" と規定し、clamp が
/// computed / used value に対して行われるとする — 本 guard と同じ作法だが、
/// **本 guard の入力は `calc()` ではなく素の `em` 乗算なので直接の根拠には
/// ならない**。§3.1 "Range Checking"
/// (<https://www.w3.org/TR/css-values-4/#combining-range>) の同文言は
/// interpolation 専用の条項であり、こちらも本 case には適用されない。
/// 直接の根拠は上記 §5 である。
///
/// # 値の決定 (1e7 px)
///
/// spec は上限を定めないので実装裁量 (上記 (b)(c))。実装が実際に持つ帯を
/// 一次 source から取った: CSSWG issue #4552 の Tab Atkins 投稿
/// (<https://lists.w3.org/Archives/Public/public-css-archive/2019Dec/0015.html>、
/// 2019-12-02) verbatim:
///
/// > right now an s32 LayoutUnit's upper range is between 1e7px and 1e8px
/// > (exact value depends on the LU->px conversion in use)
///
/// 同投稿は units-per-px を **Firefox 60 / Chrome 64 / old-Edge 100** と述べる
/// ので `2^31 / units` は 3.58e7 / 3.36e7 / 2.15e7 px。本実装は帯の**下端**
/// `1e7` を採る (3 engine のいずれの上限より下)。
///
/// **これは normative spec text ではない** — CSSWG issue の comment であり、
/// 「実装が現に持っている桁」を示す engineering evidence として使っている。
///
/// 1e7 px は 96dpi で約 2.6 km / A4 約 8900 ページ相当なので実用上の制約に
/// ならない。f32 の上限 (3.4e38) から 31 桁の余裕があるので、taffy が内部で行う
/// **和** (width + padding + border + margin) が overflow して非有限に戻ることは
/// ない。
///
/// # 入力側 bound の射程と、出力側 guard による決着
///
/// 本定数は [`sanitize_taffy`] 経由で **px 幾何と percentage の fraction の
/// 両方**に適用されている。px 側については上の #4552 の導出がそのまま効くが、
/// **fraction 側の bound としては、値をどれだけ小さく取っても不十分である。**
///
/// percentage の containing block に対する解決は used value 層 (taffy 側) で
/// 起き、**nest するたびに再び掛かる**ので深さについて指数的に複利する。A4
/// (793.7 px) を起点にすると f32 が非有限になるまでの余裕は約 35.6 桁なので、
/// fraction の上限を `F` (> 1) としたとき最初に非有限になる深さは概ね
/// `35.6 / log10(F)` — **常に有限**である。修正前の depth sweep 実測はこの
/// model と一致する:
///
/// | decl | fraction | `35.6 / log10(F)` | 実測の最初の非有限 depth |
/// |---|---|---|---|
/// | `width: 1e9%` (本定数ちょうど) | 1e7 | 5.1 | 6 |
/// | `width: 100000%` | 1e3 | 11.9 | 12 |
/// | `width: 10000%` | 1e2 | 17.8 | 18 |
/// | `width: 1000%` | 1e1 | 35.6 | 36 |
///
/// (`padding-left` を同じ値にすると probe harness で 4 / 8 / — / 25 とより
/// 浅い。padding は `location` / `scrollable_overflow_rect` の累積にも寄与するため。)
///
/// depth 1 の直接証拠: `width: 1e9%` → `size.width = 7937008000.0`
/// (= A4 793.7008px × fraction 1e7) — 既に「長さ 1e7 px」の 3 桁上。対して
/// px 経路は健全で、全 property を `1e7px` にしても depth 45 まで有限のまま。
///
/// `F <= 1.0` (= `100%`) にすれば深さ非依存になるが、`width: 200%` のような
/// spec-valid で日常的な declaration を殺すので採れない。すなわち **fraction
/// 側の入力 bound をどう選んでもこの穴は閉じられない**。CSSWG #4552 も px の
/// 話しかしておらず (percentage の乗数については何も言っていない)、fraction
/// 専用の定数を導出する一次根拠も無い。
///
/// **決着は出力側に置いた** — [`sanitize_taffy_layout`] が taffy の
/// **resolve 後**の [`taffy::Layout`] を同じ `[-MAX, MAX]` で clamp する。
/// これは深さに依存しない。
///
/// この clamp が属する cascade stage は **actual value** である
/// (CSS Cascade 5 §4.6 "Actual Values"、
/// <https://www.w3.org/TR/css-cascade-5/#actual-value> verbatim:
/// "A used value is in principle ready to be used, but a user agent may not
/// be able to make use of the value in a given environment. For example, a
/// user agent may only be able to render borders with integer pixel widths
/// and may therefore have to approximate the used width.")。すなわち
/// **used value (= taffy が計算した値) は書き換えていない** — 環境由来の
/// 近似を適用した actual value を arena に置いているだけである。近似の作法は
/// CSS Values 4 §Range Restrictions
/// (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) の "must be
/// converted to the closest value supported by the implementation, but how
/// the implementation defines "closest" is implementation-defined as well"
/// に従う。なお #4552 の px 由来の根拠が**本来当てはまるのはこの出力側**で
/// ある — そこで近似される値は fraction ではなく px の used value だから。
///
/// 入力側 guard ([`sanitize_taffy`]) は出力側 guard 導入後も**外さないこと**:
/// ±Inf / NaN を taffy の内部演算に入れない役割が残っており (site 1-4 の
/// test がこれを pin している)、出力側 clamp は「arena に
/// 非有限が入らない」ことしか保証しない。
///
/// # 出力側 clamp が実際に効く帯 (通常 layout との境界)
///
/// 本定数は actual value の上限でもあるので、**used value が 1e7 px を超える
/// 入力では病的でなくても値が動く**。例: `width: 200%` を 14 段 nest すると
/// used width = 793.7008 × 2^14 ≒ 1.30e7 px で、actual value は 1e7 に
/// 近似される (修正前は 1.30e7 がそのまま arena に入っていた)。
///
/// これは CSS Values 4 §Range Restrictions が許す範囲だが、#4552 が挙げる
/// 3 engine の上限 (2.15e7 / 3.36e7 / 3.58e7 px) より本実装は 2.2〜3.6 倍
/// strict である点は意図的な選択として記録しておく — 同 § の "should support
/// reasonably useful ranges" は SHOULD であり、1e7 px ≒ 2.6 km / A4 8900
/// ページで充足する。「影響ゼロ」が成り立つのは `[-1e7, 1e7]` 内に収まる
/// layout に限る。
///
/// なお修正前の穴は本 guard の regression ではなかった — guard 導入前 (base) と
/// bit 一致であり、閾値超え入力では guard 有りの方が strict improvement
/// (`width: 1e40%` は base で depth 1 → guard 後 depth 6)。可用性影響も測定済で、
/// 完全な render pipeline (`raikiri::html_to_png`) は depth 1 / 3 / 4 / 6 の
/// いずれでも ~200ms で正常な PNG を出していた (hang / OOM / panic なし)。
const MAX_TAFFY_MAGNITUDE: f32 = 1e7;

/// parley に渡す `font-size` の上限 (px)。
///
/// taffy 幾何 ([`MAX_TAFFY_MAGNITUDE`]) と分けているのは CSS Values 4 §5 の
/// 「supported range は property / context ごとに違ってよい」に従うため
/// (site ごとに target context が違うので一律にしない方針)。font-size の
/// target context は parley → skrifa の glyph scaler
/// であり、幾何とは妥当域が違う。
///
/// # 値の決定 (1e6 px)
///
/// 上限の**測定値**: 依存 chain の `skrifa` は font size を 16.16 固定小数へ
/// 変換する際 `Fixed::from_bits((ppem * 64.) as i32)` を通す
/// (`skrifa-0.42.1/src/instance.rs` の `Size::fixed_linear_scale`、FreeType の
/// `FT_Set_Pixel_Size` 互換のため)。したがって `ppem * 64.0` が `i32` に
/// 収まらなくなる `i32::MAX / 64 ≈ 3.36e7` ppem で変換が saturate する
/// (Rust の `f32 as i32` は saturating cast なので UB ではないが、scale factor
/// が無意味な値になる)。
///
/// 本実装はそこから 1 桁以上下の `1e6` を採る。差分は parley が font-size に
/// 掛ける係数 (`line-height` の unitless multiplier、ascent / descent の
/// `metric / units_per_em` 比) の余裕として残す — `parley-0.10.0` の
/// `layout/data.rs` は `LineHeight::FontSizeRelative(value) * font_size` と
/// `font_size / units_per_em` を計算する。
///
/// 1e6 px の glyph は A4 高さの約 890 倍で typographic な意味を持たないので、
/// 実用上の制約にはならない。
///
/// # 本 site の harm は「値が壊れる」ではなく **hang** (実測)
///
/// 下流 sink の帰結は当初 plausible なリスクとして未 characterize のままだったが、
/// 本 guard の実装時に実測した:
/// `sanitize_finite` を恒等関数に差し替えて
/// `nonfinite_font_size_is_clamped_before_parley` を単独実行すると
/// **25 秒経っても終了しない**。すなわち非有限 font-size は parley の shaping を
/// 有界時間で終わらせない。
///
/// 対して site 1-4 の taffy 側 test は **本 test 入力では**即座に assert 失敗する
/// (値が壊れるだけ)。これは「taffy は非有限で hang しない」という一般命題では
/// ない — 測ったのは 5 本の入力だけである。taffy 内部の used value に対する
/// 挙動は下流 sink 側の characterize 課題として別途残る。
///
/// 1 element の untrusted author CSS (`<p style="font-size: 1e40px">`) で
/// 到達するので、**本 site の guard は正しさではなく可用性の要求**である。
/// 削除・迂回しないこと。
///
/// regression 検出は `nonfinite_font_size_is_clamped_before_parley` が
/// worker thread + `recv_timeout` で**有界化**してある。CI の timeout
/// (`.github/workflows/ci.yml` の job 単位 `timeout-minutes` のみで nextest 設定は
/// 無い) には頼らない — job kill は infra flake と区別できず、同一 test binary の
/// 後続 test の結果もまとめて失われるため。
const MAX_FONT_SIZE_PX: f32 = 1e6;

/// parley に渡す `line-height` の unitless multiplier
/// (`ComputedLineHeight::Number` → `parley::LineHeight::FontSizeRelative`)
/// の上限。
///
/// # 値の決定 (1e6、実測による overflow 回避)
///
/// parley は `FontSizeRelative(value) * font_size` を計算する
/// (`parley-0.10.0/src/layout/data.rs` の `push_run` 内 line height 計算)。
/// `font_size` はここに渡る時点で [`MAX_FONT_SIZE_PX`] (`1e6`) 以下に
/// clamp 済みなので、`value` 側も同じ `1e6` に抑えれば積は高々 `1e12` —
/// `f32::MAX` (`≈3.4e38`) から 26 桁以上の余裕があり、finite × finite の
/// 乗算で桁あふれして `+Inf` になることはない。
///
/// この余裕が必要な理由は実測済: `value = f32::MAX` を素通しすると
/// `f32::MAX * font_size` (`font_size` が `1.0` を超える限り) が overflow して
/// `+Inf` になり、`+Inf` な line height は [`sanitize_line_height`] の doc が
/// 挙げる hang 経路に入る。`1e6` という具体的な数値自体に他の根拠はなく、
/// 「桁あふれしないことが確認できる finite な上限」であれば足りる —
/// [`MAX_FONT_SIZE_PX`] と同じ値を採ったのは、typographic に意味のある
/// line-height multiplier (実用上せいぜい 1 桁台) から見て両方とも同程度に
/// 過大な安全域だから。
///
/// # 結合の compile-time pin
///
/// 上記の overflow 非発生の論証は「両定数が同じ `1e6`」という結合そのものに
/// 依存しており、どちらか一方だけを書き換えると崩れる。直下の
/// `const _: () = assert!(...)` は「積は高々 `1e12`」という上記 paragraph
/// 自体の関係式を compile time に固定する — `f32::MAX` 直下ではなく現在の
/// 積そのものを band として pin してあるので、積が**増える**方向にどちらか
/// の定数を変更すればビルドが落ちる (減る方向は安全域が広がるだけなので
/// 素通しする)。値だけ緩めて通すのではなく、両定数と overflow 論証を
/// 併せて見直すこと。[`MAX_FONT_SIZE_PX`] 自身の妥当域は同じ形の pin を
/// `clamp_limits_are_in_the_documented_range` (test) が別途固定している。
const MAX_LINE_HEIGHT_NUMBER: f32 = 1e6;

// `f64` で積を取るのは、両定数を `f32` へ丸めた積が `1e12` 境界の
// どちら側に丸まるかという 1 ULP 未満の差にこの検査を左右させないため
// (`f32` 同士の積が overflow しても trap せず `+Inf` に飽和するだけで、
// `+Inf <= 1e12` は正しく false と評価される。ここでの懸念は overflow
// ではなく丸め境界の精度)。
const _: () = assert!((MAX_LINE_HEIGHT_NUMBER as f64) * (MAX_FONT_SIZE_PX as f64) <= 1e12);

/// parley に渡す `font-weight` の妥当域下限。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop> の grammar は
/// `<number [1,1000]>` — target context は [`MAX_TAFFY_MAGNITUDE`] /
/// [`MAX_FONT_SIZE_PX`] とは別の sink (`parley::FontWeight`) なので、
/// 「上限は sink ごとに変える」という方針に従い spec
/// 由来の妥当域をそのまま採る — skrifa / CSSWG issue のような engineering
/// measurement を要しない、数少ない site。
const MIN_FONT_WEIGHT: f32 = 1.0;

/// parley に渡す `font-weight` の妥当域上限 (同上、CSS Fonts 4 §2.2)。
const MAX_FONT_WEIGHT: f32 = 1000.0;

/// 非有限 `font-weight` (`NaN`) の fallback 値。
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
/// `normal` keyword の computed value (`ComputedValues::initial().
/// font_weight` の値と一致、`crates/raikiri-style/src/computed.rs` 参照)。
///
/// [`sanitize_finite`] が length 系 site で NaN を `0.0` に落とすのは、
/// padding/margin の spec initial が幾何 `0` である、あるいは `0 * inf =
/// NaN` という無限精度評価が実際に `0` になるケースだから (同関数 doc
/// 参照) — font-weight にはどちらの根拠も対応しない。`0.0` は
/// `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]` の外なので、それをそのまま NaN の
/// 代わりに使うと sanitize 後の値が sink の妥当域を割ってしまう。よって
/// font-weight は独自の fallback を持つ。
const FALLBACK_FONT_WEIGHT: f32 = 400.0;

/// 非有限 f32 を `[min, max]` の有限値に落とす。
///
/// - **NaN → 0.0**。`f32::clamp` は NaN を **NaN のまま**返す (`NaN.clamp(a, b)`
///   は NaN) ので、clamp だけでは潰せない。NaN は数直線上の点ではないため
///   §5 の「closest value supported」も定義できない。
///
///   0.0 を選ぶ根拠は「spec initial だから」**ではない** — initial が幾何 `0`
///   なのは padding / margin だけで (CSS Box 3 `#propdef-padding-top` /
///   `#propdef-margin-top` とも `Initial: 0`)、`width` / `height` の initial は
///   **`auto`** (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
///   <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>、
///   `data-level="3.1.1"` 実検証済)、`border-*-width` は **`medium`**
///   (CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
///   <https://www.w3.org/TR/css-backgrounds-3/#border-width>、
///   `data-level="3.3"`、TR / ED とも `Initial: medium`) である。
///   `auto` も `medium` も幾何値ではなく**解決規則 / キーワード**なので f32 の
///   代替値として選べない。よって **全 site 一律 0.0** に倒す。§5 が "closest"
///   の定義を実装裁量とするので、この選択自体が spec 準拠である。
///
///   さらに `raikiri-style::resolve` で NaN が生じる経路 (`0px` × `1e40em` = `0.0 * inf`) に
///   限れば、**0.0 は spec 上の正解と一致する** — §5 が "within the CSS
///   specifications, infinite precision and range is assumed" と述べる以上、
///   無限精度で評価した computed value は `0 × 10^40 = 0px` である。NaN は
///   f32 の有限精度が生んだ artifact にすぎない。
///
///   傍証 (直接の根拠ではない): CSS Values 4 §10.9.1 "Infinities, NaN, and
///   Signed Zero" (<https://www.w3.org/TR/css-values-4/#calc-ieee>、
///   `data-level="10.9.1"` 実検証済。ED では §10.9.2 に採番されるが anchor は
///   同一) は math function について verbatim で
///   `NaN does not escape a top-level calculation; it's censored into a zero
///   value` / `Infinities do not escape a top-level calculation; they're clamped
///   to the minimum or maximum value allowed in the context …` と規定する (後者は原文では
///   `, as defined in § 10.12 Range Checking.` と続く — 省略を `…` で示した)。
///   **本 guard の入力は `calc()` ではないので直接の根拠にはならない**
///   (#4552 と同じく engineering evidence 扱い) が、CSS が同種の状況で採る
///   censoring 規則が NaN→zero / Inf→clamp の 2 分岐でありここでの選択と
///   一致することは、選択の妥当性を補強する。
/// - **±Inf と範囲外の有限値 → `min` / `max`**。CSS Values 4 §5 の "converted
///   to the closest value supported by the implementation" の適用。
///
/// **巨大な有限値も clamp する** (単に有限化するだけにしない) — `1e38%` は
/// bridge では有限だが、taffy 内部で containing block と掛けた時点で +Inf に
/// なり、guard を置いた意味が消える。§5 は「supported range」を実装が決めると
/// しているので、範囲外の有限値を上限に寄せるのも同じ条項の適用である。
///
/// panic しない (`LayoutError` も返さない) — **clamp して続行**する方針である。
///
/// # なぜ silent clamp ではないのか
///
/// `log` / `tracing` は workspace に依存が無い (`grep` → 0 hit) が、**それが
/// 理由ではない** — 同一 crate の `fonts.rs` に dep 追加ゼロの診断機構が既に
/// ある (`FontWarn` enum + `FontWarnObserver = Option<&mut dyn FnMut(&FontWarn)>`
/// + `emit_warn`)。
///
/// 当初この observer を本 site まで通すと公開
/// signature に波及すると判断し silent のままにしていた: `sanitize_*` は
/// `bridge_*` → `apply_computed_to_style` → `layout_single_page` の奥にあり、
/// また出力側の choke point (`sanitize_taffy_layout`) は
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout` から
/// 呼ばれる — これは `taffy` crate 側が固定した trait method signature なので
/// **観測用引数を追加できない**。
///
/// `crate::diag::emit_warn_via` 共通機構を導入し、
/// この 2 点を以下で解決した:
/// - `bridge_*` → `apply_computed_to_style` の chain は crate 内 private
///   function のみで構成されるため、`diag: &mut Vec<LayoutWarn>` を通すのは
///   crate-internal な signature 変更で完結する (pub シグネチャは無傷)。
/// - `set_unrounded_layout` は `self` (`&mut Document`) は受け取れるので、
///   observer を **closure ではなく owned buffer**
///   (`Document::layout_warnings`) として `self` に持たせることで、
///   trait signature を変えずに choke point からも push できるようにした。
///   `layout_single_page` がこの buffer をパスの最後で drain し、
///   `fonts.rs` と同じ `emit_warn_via` 経由で observer-or-eprintln に流す。
///
/// 「per-node で裸の `eprintln!` を撒いて spam する」ことは避けている —
/// [`push_layout_warn`] は実際に clamp が起きた (値が変わった) 場合のみ
/// event を積むので、通常範囲の layout は buffer に何も残らない。これは
/// `FontWarn` が「warn+skip の異常」だけを observer に渡し、処理した file
/// 全部を都度報告しないのと同じ設計原則である。
///
/// **残余リスク (部分的にのみ縮小)**:
/// 将来 absolutize 側に本物の算術 bug (例: 単位換算ミスで `1e9px`) が入ると、
/// 本 guard が 1e7 に吸収して**「それらしい layout」として描画されてしまう**
/// リスクは元々あった — NaN や破綻として可視化されない。これはかつて
/// 削除した fail-quiet arm (`Em(_) => length(0.0)`) と**同じ class の
/// 残余リスク**である。
///
/// 今は clamp が発生するたび [`LayoutWarn::NonFiniteClamped`] が
/// observer-or-eprintln 経由で外に出るが、**「解消」ではなく「silent から
/// stderr-visible への降格」**と正確に言うべきである — `layout_single_page`
/// に external observer を差し込む口は現状無い (`LayoutWarnObserver`
/// scaffolding の doc参照) ので、本 crate 内に stderr を能動的に監視する
/// consumer が無い限り、この event は誰にも読まれない。`fonts.rs` の
/// `FontWarn` も同じ状態 (observer 無しなら stderr のみ) なので同水準の
/// 可視性にはなったが、「値の病理が可視化される」と言えるのは stderr を
/// 見ている human operator がいる場合に限る。
fn sanitize_finite(
    v: f32,
    min: f32,
    max: f32,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> f32 {
    let clamped = if v.is_nan() { 0.0 } else { v.clamp(min, max) };
    // `v != clamped` は NaN 入力でも正しく true になる (NaN の比較は IEEE 754
    // で常に false 「以外」= `!=` は true) ので、NaN → 0.0 の代入も
    // out-of-range 値の clamp も同じ条件で拾える。範囲内の通常値は
    // `clamped == v` なので何も積まない (spam 回避、上の doc 参照)。
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site,
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// [`MAX_TAFFY_MAGNITUDE`] を上限とする対称 clamp (taffy 幾何用)。
///
/// 対称 (`[-MAX, MAX]`) なのは **`margin` の負値が spec-valid** だから。
/// CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties"
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>、`data-level="3.1"`
/// 実検証済) は verbatim で
///
/// > Negative values for margin properties are allowed,
/// > but there may be implementation-specific limits.
///
/// と規定する。これは本 delta で clamp する property のうち**唯一、spec が
/// 「implementation-specific limits」の存在を明示的に認めている**箇所であり、
/// 対称であることと上限があることを同時に正当化する
/// (「非負制約が無い」という不在の論証より強い)。
///
/// `padding` / `width` / `height` / `border-width` は parse 段で非負が
/// enforce されているので、対称にしても値は変わらない。
///
/// `site` は [`LayoutWarn::NonFiniteClamped`] の call-site label としてのみ
/// 使う (clamp の算術には影響しない)。
fn sanitize_taffy(v: f32, site: &'static str, diag: &mut Vec<LayoutWarn>) -> f32 {
    sanitize_finite(v, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE, site, diag)
}

/// 非有限 (`NaN` / `±Inf`) または `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]`
/// 範囲外の `font-weight` を `parley::FontWeight::new` へ渡す直前で
/// sanitize する。
///
/// # なぜここに置くか
///
/// 「非有限 / 範囲外 f32 の guard は値が実際に使われる sink 境界
/// (target context) に置く。parse-time (specified 層) にも resolve 層
/// (computed 層) にも置かない」という方針に従う。`crate::page::cascade_page`
/// (raikiri-style) の継承元 root 引数や `ComputedValues` の直接構築は
/// raikiri-style 側の resolve/computed 層であり、
/// `raikiri_style::cascade::resolve_relative_weight` も同じ層に属する —
/// この方針はそのどちらへの guard 追加も明示的に禁じる
/// ("public な computed 層 surface は sanitize しない")。
///
/// 本関数は [`preshape_text`] — `parley::FontWeight::new` を呼ぶ唯一の call
/// site — に置くことで、他の 5 site (taffy bridge
/// helper 4 本 + font-size 用 `preshape_text` 呼び出し) と同じ「sink 直前」
/// 構造に揃える (site 6)。
///
/// # NaN fallback が `0.0` ではなく [`FALLBACK_FONT_WEIGHT`] (`400.0`) な理由
///
/// [`sanitize_finite`] を直接再利用しない。理由は 2 つ:
///
/// 1. **NaN fallback がそもそも違う** — [`sanitize_finite`] は NaN を
///    無条件で `0.0` に落とすが、その根拠 (同関数 doc 参照) は font-weight
///    に対応しない。`0.0` は妥当域の外なので、そのまま使うと sanitize
///    後の値が sink の妥当域を割る。CSS Fonts 4 §2.2
///    "Font weight: the font-weight property"
///    <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
///    `normal` の computed value である `400.0` を採る方が、
///    「fallback / 上限は sink ごとに変える」という方針に忠実。
/// 2. **signature 変更の波及範囲** — `nan_fallback` 引数を足せば理屈上
///    1 関数に統合できるが、それは既存の length 系 4 call site
///    (`sanitize_taffy` 経由の 4 本 + font-size 直接呼び出し 1 本) と、
///    それらを検証する既存 test 全部に本 task の scope
///    (font-weight 1 sink) と無関係な引数を波及させる。独立した小関数として
///    持つ方が diff が scope に対して釣り合う。
///
/// # 出力側 (`resolve_relative_weight`) は変えない
///
/// `resolve_relative_weight` の非対称処理 (`Bolder`/`Lighter`/`-Inf` の
/// 扱いが異なる、同関数 doc 参照) は本関数の追加で修正しない —
/// resolve 層の挙動変更は本方針の禁止対象であり、本 sink guard は
/// 「resolve 層が何を出しても最終的に有限 + 妥当域内にする」ことだけを
/// 保証する。
fn sanitize_font_weight(v: f32, diag: &mut Vec<LayoutWarn>) -> f32 {
    let clamped = if v.is_nan() {
        FALLBACK_FONT_WEIGHT
    } else {
        v.clamp(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT)
    };
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site: "font-weight",
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// `raikiri_style::property::FontStyle` (`Normal | Italic`, `#[non_exhaustive]`)
/// → parley's `FontStyle` (re-exported from the `parlance` crate: `Normal |
/// Italic | Oblique(Option<f32>)`, CSS Fonts 4 §2.4
/// <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// Only the first two variants have a raikiri-style counterpart today —
/// `oblique` isn't parsed yet (see `raikiri_style::property::FontStyle`
/// doc's "Scope carving" section) — so parley's third variant has no source
/// value to map from. The wildcard arm exists purely for `StyleFontStyle`'s
/// `#[non_exhaustive]` forward-compat contract (a downstream match must
/// tolerate variants added to the source enum later) and is unreachable
/// with the variant set that exists today.
fn font_style_to_parley(v: StyleFontStyle) -> FontStyle {
    match v {
        StyleFontStyle::Normal => FontStyle::Normal,
        StyleFontStyle::Italic => FontStyle::Italic,
        // cov:ignore: unreachable while StyleFontStyle is Normal|Italic
        // only; required for its #[non_exhaustive] contract (see doc
        // above).
        _ => FontStyle::Normal,
    }
}

/// [`ComputedLineHeight`] (`cascade.computed[idx].line_height`,
/// [`ComputedValues`] の全 field は `pub` なので非有限になり得る) を parley の
/// 2 numeric sink (`ComputedLineHeight::Number` / `Length`) 直前で sanitize
/// する。`Normal` は数値を持たないのでそのまま素通しする。
///
/// `[0.0, MAX]` の非対称 clamp (対称でない) は CSS Inline 3 §5.1
/// "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) の grammar
/// `normal | <number [0,∞]> | <length-percentage [0,∞]>` に合わせたもの —
/// `font-size` / `font-weight` の既存 sanitize サイトと同じ理由 (負値は
/// grammar 上そもそも妥当域外)。
///
/// # `+Inf` は他の site と違う経路で **hang** する
///
/// 実測 (直接 `RangedBuilder` に `StyleProperty::LineHeight` を push し、
/// worker thread + `recv_timeout` で有界化): `parley::LineHeight::Absolute(f32::INFINITY)`
/// と `FontSizeRelative(f32::INFINITY)` はいずれも `break_all_lines` を
/// hang させる。機構は `font-size` の hang (`next_x <= max_advance` が
/// `next_x = +Inf` で恒偽になる、[`MAX_FONT_SIZE_PX`] の doc参照) とは別:
/// `parley-0.10.0/src/layout/line_break.rs` の
/// `BreakerState::add_line_height` が `running_line_height =
/// running_line_height.max(height)` を計算しており、`height` が `+Inf` だと
/// `running_line_height` も `+Inf` になって `running_line_height >
/// line_max_height` (`line_max_height` の default は `f32::MAX`) が
/// 以後ずっと真になり続ける。この `max_height_exceeded` 分岐が前進しない
/// ことで hang する。
///
/// `NaN` は hang **しない** — `f32::max` は NaN を捨てて他方の被演算子を返す
/// (IEEE 754 の total-order ではなく Rust 標準の `f32::max` 挙動) ため
/// `running_line_height` は有限のまま前進する。`font-size` の
/// `NaN`/`-Inf`/巨大 finite が hang しないのと同じ非対称構造 (site 5 の
/// `parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size`
/// 参照)。
///
/// `FontSizeRelative` はさらに `value * font_size` の乗算で桁あふれし得る
/// ([`MAX_LINE_HEIGHT_NUMBER`] の doc参照) — [`MAX_LINE_HEIGHT_NUMBER`] は
/// この overflow を避ける上限。`Absolute` はそのまま渡るだけで乗算しないため
/// `f32::MAX` 自体は overflow しない (`f32::MAX > f32::MAX` は偽) が、
/// 同じ [`MAX_FONT_SIZE_PX`] を再利用して上限とする — 「巨大 finite を防ぐ」
/// ためではなく「入力側で既に non-finite になっているケースを finite に倒す」
/// ための clamp であり、typographic に意味のある line-height (px) から見て
/// 過大という理由は font-size の値そのものと同種。
fn sanitize_line_height(v: ComputedLineHeight, diag: &mut Vec<LayoutWarn>) -> ComputedLineHeight {
    match v {
        ComputedLineHeight::Normal => ComputedLineHeight::Normal,
        ComputedLineHeight::Number(n) => ComputedLineHeight::Number(sanitize_finite(
            n,
            0.0,
            MAX_LINE_HEIGHT_NUMBER,
            "line-height (number)",
            diag,
        )),
        ComputedLineHeight::Length(len) => {
            ComputedLineHeight::Length(ComputedLength(sanitize_finite(
                len.px(),
                0.0,
                MAX_FONT_SIZE_PX,
                "line-height (length)",
                diag,
            )))
        }
    }
}

/// [`ComputedLineHeight`] (`Normal | Number | Length`) → parley's
/// [`LineHeight`] (`MetricsRelative | FontSizeRelative | Absolute`,
/// `parley-0.10.0/src/style/mod.rs`).
///
/// 呼び出し側は事前に [`sanitize_line_height`] で有限化した値を渡すこと —
/// 本関数自体は値を変換するだけで sanitize しない ([`font_style_to_parley`]
/// と同じ「mapping と sanitize は別関数」構造)。
///
/// # 値レベルの対応 (各 arm の根拠)
///
/// - [`ComputedLineHeight::Normal`] → `LineHeight::MetricsRelative(1.0)` —
///   parley 自身の `Default` (`parley-0.10.0/src/style/mod.rs` の
///   `impl Default for LineHeight`) と一致する。本関数を経由しても
///   line-height 配線前の挙動 (parley default 依存) を変えない。
/// - [`ComputedLineHeight::Number`] → `LineHeight::FontSizeRelative` —
///   parley は `FontSizeRelative(value) * font_size` を計算する
///   (`parley-0.10.0/src/layout/data.rs` の `push_run`)。ここでの
///   `font_size` は `preshape_text` が同じ `RangedBuilder` へ push する
///   **自要素の** computed font-size (`StyleProperty::FontSize`) そのもの
///   なので、CSS Inline 3 §5.1 の「unitless number は子が自分の font-size に
///   掛ける」と一致する。
/// - [`ComputedLineHeight::Length`] → `LineHeight::Absolute` — computed 層で
///   既に絶対化済みの px 値 ([`ComputedLineHeight::Length`] の doc参照) を
///   そのまま渡す。parley 側も `LineHeight::Absolute(value) => value` と
///   素通しするだけ (`data.rs`) なので、二重の解決は起きない。
///
/// [`ComputedLineHeight`] は `#[non_exhaustive]` を付けない判断がされている
/// ([`raikiri_style::resolve`] module doc参照) ため、本関数も
/// [`font_style_to_parley`] と異なり wildcard arm を持たない — 将来 variant が
/// 追加されればここでコンパイルが落ちて気づける。
fn line_height_to_parley(v: ComputedLineHeight) -> LineHeight {
    match v {
        ComputedLineHeight::Normal => LineHeight::MetricsRelative(1.0),
        ComputedLineHeight::Number(n) => LineHeight::FontSizeRelative(n),
        ComputedLineHeight::Length(len) => LineHeight::Absolute(len.px()),
    }
}

/// Structured warn event for this module's non-finite-clamp diagnostic sites
/// (`sanitize_finite` / `sanitize_taffy` / `sanitize_taffy_layout` /
/// `sanitize_font_weight`). Sibling
/// of [`crate::fonts::FontWarn`], generalized via the
/// shared [`crate::diag::emit_warn_via`] mechanism so
/// the "silent clamp" residual risk documented on [`sanitize_finite`] gets
/// the same observability `fonts.rs` already has.
///
/// Every variant is fully owned (no borrowed `Path`, unlike `FontWarn`)
/// because these clamp sites only ever see primitive `f32` values. See
/// `crate::diag`'s module doc for why this owned shape — not `FontWarn`'s
/// borrowed one — is what a shared generic `Observer<W>` type could actually
/// have supported, and why a macro was used instead so both shapes share one
/// mechanism anyway.
///
/// No `#[non_exhaustive]` (unlike `FontWarn`, which is `pub`): that attribute
/// only constrains *downstream crates*, and this enum is `pub(crate)` with no
/// external consumer to protect. Add it back if this type is ever promoted
/// to a public export.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LayoutWarn {
    /// A non-finite (NaN / +-Inf) or out-of-range `f32` was clamped to a
    /// finite in-range value before being handed to one of this module's
    /// sink boundaries: `taffy::Style` or the `Node.unrounded_layout` arena
    /// field (sites 1-5, `sanitize_finite` / `sanitize_taffy` /
    /// `sanitize_taffy_layout`),
    /// `parley::FontWeight::new` (site 6, `sanitize_font_weight`), or
    /// `StyleProperty::LineHeight`'s two numeric sub-values (sites 7-8,
    /// `sanitize_line_height`). Only
    /// emitted when clamping actually changed the
    /// value (not on every call) so ordinary in-range layouts stay silent —
    /// the "warn+skip" shape `FontWarn` uses, not a per-node trace.
    NonFiniteClamped {
        /// Call-site label (e.g. `"font-size"`, `"margin"`,
        /// `"layout.size"`) — a human-readable category, not a stable
        /// machine-parseable identifier.
        site: &'static str,
        /// Pre-clamp value (may be NaN or +-Inf).
        raw: f32,
        /// Post-clamp value actually used.
        clamped: f32,
    },
    /// `suppressed` additional [`LayoutWarn::NonFiniteClamped`] events were
    /// dropped once [`LAYOUT_WARN_CAP`] was reached during a single
    /// `layout_single_page` pass, bounding memory / `eprintln!` spam under a
    /// pathological input that clamps every field of every node (e.g. deep
    /// `width: 200%` nesting — see [`MAX_TAFFY_MAGNITUDE`]'s doc). Emitted at
    /// most once per pass, after all the real events it summarizes.
    Truncated {
        /// Count of additional `NonFiniteClamped` events dropped after the
        /// cap was reached.
        suppressed: usize,
    },
    /// One or more subtrees had a broken **parent/child geometry invariant**
    /// and were reset to a deterministic zero
    /// [`taffy::Layout`] by [`enforce_layout_invariants`]. This is a
    /// different failure class than [`LayoutWarn::NonFiniteClamped`]: that
    /// variant fires when a single `f32` field was out of range, this one
    /// fires when every individual field of the (already per-field-clamped)
    /// `Layout`s involved was in range, but the *relationship* between two
    /// or more fields — possibly on different nodes — was not (e.g. a
    /// node's own content box went negative, or a child's border box did
    /// not fit inside its parent's once one of the two had its actual value
    /// approximated by [`sanitize_taffy_layout`]). See
    /// [`enforce_layout_invariants`]'s doc for exactly which two invariants
    /// are checked and why unconditionally checking cross-node containment
    /// would be spec-incorrect.
    ///
    /// Aggregated per invariant (at most one event per invariant kind per
    /// `layout_single_page` pass, each counting every subtree it reset)
    /// rather than one event per reset subtree, so a pathological input
    /// that trips the same invariant on many nodes cannot reintroduce the
    /// per-node spam [`LAYOUT_WARN_CAP`] exists to bound.
    GeometryInvariantViolated {
        /// Which invariant was violated — `"content_box_non_negative"` or
        /// `"child_within_parent_border_box"` (see
        /// [`enforce_layout_invariants`]'s doc). Like `NonFiniteClamped`'s
        /// `site`, a human-readable category, not a stable
        /// machine-parseable identifier.
        ///
        /// The `"child_within_parent_border_box"`
        /// value can no longer actually occur — [`child_within_parent_border_box`]
        /// (the predicate) now always returns `true`, so
        /// [`enforce_layout_invariants`]'s containment branch that would
        /// produce this event is unreachable for any input. It remains
        /// listed here (and the branch remains in the code) because whether
        /// to remove the dead invariant check entirely is a separate,
        /// explicitly deferred decision. Do not treat this
        /// value's continued presence in this doc as evidence the check is
        /// still live.
        invariant: &'static str,
        /// Number of distinct subtree roots this invariant caused
        /// [`enforce_layout_invariants`] to reset in this pass (not a count
        /// of individual arena nodes touched — a reset subtree may contain
        /// further descendants that also got zeroed as part of the same
        /// reset).
        subtree_count: usize,
    },
}

impl std::fmt::Display for LayoutWarn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => write!(
                f,
                "{site}: clamped non-finite/out-of-range value {raw} to {clamped}"
            ),
            LayoutWarn::Truncated { suppressed } => write!(
                f,
                "{suppressed} additional layout clamp warning(s) suppressed (buffer cap reached)"
            ),
            LayoutWarn::GeometryInvariantViolated {
                invariant,
                subtree_count,
            } => write!(
                f,
                "{invariant}: {subtree_count} subtree(s) had a broken parent/child geometry invariant and were reset to a zero layout"
            ),
        }
    }
}

/// Observer alias for [`LayoutWarn`] — sibling of `fonts.rs`'s
/// `FontWarnObserver`. Unlike that alias, this one carries no inner lifetime
/// (`LayoutWarn` is fully owned), so it is a plain, non-higher-ranked
/// `Option<&mut dyn FnMut(&LayoutWarn)>`. Not yet reachable from any public
/// entry point — see [`Document::layout_warnings`](crate::document::Document)
/// for why (dom→paint wall: `layout_single_page`'s signature is consumed by
/// `raikiri-paint` and the `raikiri` crate, so adding a parameter — or a new
/// `_with_observer` sibling — to it is a decision for that wall, not this
/// task). This scaffolding exists so a future `_with_observer` addition only
/// has to plumb one new parameter through, rather than re-deriving the whole
/// mechanism.
type LayoutWarnObserver<'o> = Option<&'o mut dyn FnMut(&LayoutWarn)>;

/// Emit a [`LayoutWarn`] event: call the observer if `Some`, otherwise
/// `eprintln!` (matches `crate::fonts::emit_warn`'s shape exactly, via the
/// shared [`crate::diag::emit_warn_via`] macro).
fn emit_layout_warn(observer: &mut LayoutWarnObserver<'_>, event: LayoutWarn) {
    crate::diag::emit_warn_via!(observer, "[raikiri-dom::layout]", event);
}

/// Cap on buffered [`LayoutWarn::NonFiniteClamped`] events per
/// `layout_single_page` pass.
///
/// A single pathological input (e.g. deep `width: 200%` nesting hitting
/// every node, [`MAX_TAFFY_MAGNITUDE`]'s doc) can clamp every `f32` field of
/// every node in the arena, which would otherwise make both the buffer and
/// the eventual `eprintln!` replay unbounded — precisely the "per-node spam"
/// concern [`sanitize_finite`]'s doc raised about threading an observer down
/// this chain in the first place. [`push_layout_warn`] collapses anything
/// past this cap into a single running [`LayoutWarn::Truncated`] counter
/// instead of dropping it silently.
pub(crate) const LAYOUT_WARN_CAP: usize = 63;

/// Push a [`LayoutWarn`] onto `diag`, respecting [`LAYOUT_WARN_CAP`].
///
/// Once the cap is reached, further events collapse into (rather than grow)
/// a single trailing [`LayoutWarn::Truncated`] counter, so the buffer size is
/// bounded (`LAYOUT_WARN_CAP + 1`) regardless of how many clamp sites fire in
/// one pass.
fn push_layout_warn(diag: &mut Vec<LayoutWarn>, event: LayoutWarn) {
    if diag.len() < LAYOUT_WARN_CAP {
        diag.push(event);
        return;
    }
    match diag.last_mut() {
        Some(LayoutWarn::Truncated { suppressed }) => *suppressed += 1,
        _ => diag.push(LayoutWarn::Truncated { suppressed: 1 }),
    }
}

/// taffy が resolve した [`taffy::Layout`] の全 f32 field を
/// [`sanitize_taffy`] に通す **出力側** guard。
///
/// # なぜ出力側なのか (入力側の bound では閉じられない)
///
/// [`MAX_TAFFY_MAGNITUDE`] の「入力側 bound の射程」節のとおり、percentage は
/// used value 層で containing block に対して解決されるため nest ごとに複利し、
/// **1 より大きい fraction 上限はどれを選んでも有限の深さで f32 を溢れさせる**。
/// 深さは untrusted な入力 (DOM の nest) が決めるので、深さ非依存の場所 —
/// resolve の**後** — に guard を置く以外に閉じ方が無い。
///
/// # call site は 1 箇所 (choke point)
///
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
/// (`taffy_impl.rs`) — taffy が arena へ layout を書き戻す**唯一の**経路
/// (`taffy-0.12.1` の block / flexbox / grid / leaf 各 algorithm はすべて
/// この 1 メソッドを通る)。したがって「`Node.unrounded_layout` は決して
/// 非有限を含まない」は構造的な invariant であり、後付けの一括 sweep のように
/// 呼び忘れで破れることがない。
///
/// invariant の残り半分は**初期値**: `Node::new*` は
/// `Layout::with_order(0)` (`node.rs`) を置き、これは全 field 0 で有限。
/// 以降の書き込みは上記のとおり本 guard を通るので、arena が非有限 layout を
/// 持つ瞬間が存在しない。
///
/// # taffy の内部計算は変えない (used value は不変、actual value のみ近似)
///
/// `taffy::Layout` を arena から**読み戻す**のは `RoundTree::get_unrounded_layout`
/// だけで、これは `taffy::round_layout` 専用である。raikiri は `round_layout` を
/// 呼ばず `RoundTree` も実装していない (`grep -rn 'round_layout\|RoundTree'
/// crates/` → 本 doc comment 以外 0 hit、実測)。よって本 clamp は
/// **観測面だけ**を縛り、taffy 内部の
/// percentage 解決 chain (`LayoutInput::parent_size`) には影響しない。
/// すなわち「深いところで内部的に inf になった結果が clamp 済の値として
/// 見える」のであって、レイアウト計算自体を書き換えてはいない。
///
/// # 網羅的な struct literal (`..` を使わない)
///
/// 全 f32 field を明示列挙する。`..*layout` にすると taffy が将来 f32 field を
/// 増やしたときに**黙って guard の外に漏れる**が、網羅 literal なら compile
/// error になって review を強制できる。`order` は `u32` なので guard 対象外。
///
/// paint が現に読む 4 field だけに絞らないのも同じ理由 —
/// 「arena は非有限幾何を持たない」は述べられて test できる invariant だが、
/// 「paint がたまたま読む field」はそうではない。
///
/// # 保証するのは finiteness だけ (box model の包含関係は保存しない)
///
/// なお本 guard が保証するのは **finiteness だけ**で、box model の包含関係
/// (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない — field ごとに
/// 独立に clamp するので、`size.width` と `padding.{left,right}` が同時に
/// 飽和すると `size.width - padding.left - padding.right` は負になりうる。
/// 現在 `padding` / `border` / `scrollable_overflow_rect` / `scrollbar_size` を読む
/// consumer は無い (grep 実測) が、将来 paint がこれらを使うときは
/// 非負性を仮定しないこと。
///
/// `diag` collects [`LayoutWarn::NonFiniteClamped`] events for whichever
/// fields actually get clamped (site labels: `"layout.location"`,
/// `"layout.size"`, `"layout.scrollable_overflow_rect"`, `"layout.scrollbar_size"`,
/// `"layout.border"`, `"layout.padding"`, `"layout.margin"`). The sole caller
/// (`<Document as taffy::LayoutPartialTree>::set_unrounded_layout` in
/// `taffy_impl.rs`) passes `&mut self.layout_warnings` — an owned buffer on
/// `Document`, not a live observer — because that trait method's signature
/// is fixed by `taffy` and cannot receive one (see
/// `Document::layout_warnings`'s doc for why).
pub(crate) fn sanitize_taffy_layout(
    layout: &TaffyLayout,
    diag: &mut Vec<LayoutWarn>,
) -> TaffyLayout {
    fn size(s: Size<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width, site, diag),
            height: sanitize_taffy(s.height, site, diag),
        }
    }
    fn rect(r: Rect<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Rect<f32> {
        Rect {
            left: sanitize_taffy(r.left, site, diag),
            right: sanitize_taffy(r.right, site, diag),
            top: sanitize_taffy(r.top, site, diag),
            bottom: sanitize_taffy(r.bottom, site, diag),
        }
    }
    TaffyLayout {
        order: layout.order,
        location: Point {
            x: sanitize_taffy(layout.location.x, "layout.location", diag),
            y: sanitize_taffy(layout.location.y, "layout.location", diag),
        },
        size: size(layout.size, "layout.size", diag),
        scrollable_overflow_rect: rect(
            layout.scrollable_overflow_rect,
            "layout.scrollable_overflow_rect",
            diag,
        ),
        scrollbar_size: size(layout.scrollbar_size, "layout.scrollbar_size", diag),
        border: rect(layout.border, "layout.border", diag),
        padding: rect(layout.padding, "layout.padding", diag),
        margin: rect(layout.margin, "layout.margin", diag),
    }
}

/// [`sanitize_taffy_layout`] が保証する **finiteness** の一段上のレイヤー —
/// 親子 geometry の**意味的** invariant を検査し、破れている subtree を
/// 決定的な既定 geometry (ゼロ) に置き換える。
///
/// # なぜ `sanitize_taffy_layout` だけでは閉じないか
///
/// `sanitize_taffy_layout` の doc が明言する通り、
/// その guard は **field ごとに独立に** clamp するため、box model の包含
/// 関係 (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない —
/// `size.width` と `padding.{left,right}` が同時に飽和すると
/// `content_box_width()` が負になりうる。また taffy 内部の演算 chain は `LayoutOutput`
/// 経由で **clamp 前の生値** を子から親へ返す (`taffy-0.12.1` の
/// `compute/block.rs:947,973,981,1072`、`set_unrounded_layout` が呼ばれる
/// のはその**後**であり、かつ子の `Layout` を書くのは子自身ではなく
/// **親の algorithm**) ため、ある node の位置が「別の (クランプ済) node」を
/// 基準に計算されていても、各 node は「自分の field が有限」であることしか
/// 保証されない。本関数はその 2 つの隙間 — 単一 node 内の box model 包含
/// 関係、および親子間の位置関係 — を埋める。
///
/// # 検査する 2 つの invariant
///
/// 1. **content box 非負** — `Layout::content_box_width()` /
///    `content_box_height()` が両方 `>= 0.0`。**全 node に無条件で**適用する。
///    実装時に実測した (probe: `<div style="width: 10px; padding: 50px;
///    box-sizing: border-box;">` を通常経路 (`layout_single_page`) で
///    layout): taffy 自身が border box を `size.width == padding_left +
///    padding_right` (= `100.0`) まで自動的に stretch し、content box を
///    ちょうど `0.0` に floor する (`taffy-0.12.1/src/compute/mod.rs` の
///    `maybe_max(padding_border_size)` と同型の内部ロジック)。すなわち
///    **非有限 clamp が絡まない通常の CSS では content box が負になること
///    はない** — この invariant を無条件で検査しても legitimate な layout
///    を誤検出しない。
/// 2. **child の border box の原点 (`location`) が parent の border box に
///    収まる** — もともとは containment を検査する invariant として設計
///    されたが、**飽和した axis は符号を
///    問わず無条件に ok とする変更が入った** (詳細は後述の符号別の節) — つまり本 invariant は
///    もはや実際には何も検査しない (taffy の座標系は「parent border box
///    原点からの相対位置」、`taffy-0.12.1/src/tree/layout.rs` の
///    `Layout::content_box_x/y` の doc参照)。**`child.size` は見ない** —
///    本 invariant はもともと「child の **location** が parent の
///    border box 内」とだけ規定しており、child 自身の大きさは対象にしていない
///    ([`child_within_parent_border_box`] の doc「`child.size` を見ない理由」
///    節、実装時に extent (`location + size`) 版で `width: 200%` の
///    legitimate nest を誤検出することが判明した経緯を記録している)。
///
///    **以下はこの変更が入る前の設計とその根拠の記録** — 上述のとおりこの変更以降、
///    本 invariant は実際には何も検査しない無条件 accept になっている。
///    なぜ元々こちらを無条件検査にしなかったか: CSS は
///    子が親の border box をはみ出すことを普通に許す (`overflow: visible`
///    が initial 値、負 margin、固定して小さい container + 大きい content、
///    `width: 200%` のような「子が親より意図的に大きい」宣言) — raikiri は
///    今 `position: absolute` を未実装だが、それだけで十分再現する。実装時に
///    実測した (probe: parent `<div style="width: 50px; height: 50px;">` の
///    子に `<div style="width: 200px; height: 200px; margin-left:
///    -30px;">`) では parent `size=(50,50)` に対し child `location=(-30,0)`
///    — 原点自体が parent の左端 (`x=0`) より外に出ているが、これは**正しい
///    layout であって bug ではない**。無条件で検査すると legitimate な
///    layout を誤って fallback してしまうため、`child.location.x` /
///    `child.location.y` の**その axis 自身**が [`MAX_TAFFY_MAGNITUDE`] の
///    飽和境界にちょうど達している場合**だけ**、その axis を検査する
///    ([`child_within_parent_border_box`] の実装参照 — `parent.size` や
///    `child.size` が飽和しているかどうかはこの gate に関与しない)。
///    飽和が起きたということは、その field の「actual value」(近似後の値)
///    が taffy 内部の「used value」(実際の計算結果) と乖離している —
///    以前の版ではこれを根拠に「だからこそ改めて明示的に整合性を
///    検査し、破れていれば決定的な値に倒す」という設計だった。**その後
///    この設計は覆った**: 「actual value が近似
///    されている」こと自体は仕様上許容された範囲内の動作であり、収まって
///    いようといまいと本関数が reset の理由にすることはない、という結論に
///    符号を問わず統一された (詳細は後述の符号別の節)。
///    `saturated_but_contained_layout_is_not_reset` は元々「gate かつ
///    containment 違反」という conjunction を pin する目的の test だった
///    が、この変更以降は assert 自体は変わらず通る (この test の fixture が
///    たまたま「収まっている」ケースなので) ものの、conjunction の主張は
///    もう成立しない — 同 test の doc および対の regression pin
///    (`saturated_child_outside_parent_is_not_reset`、
///    「明らかに収まっていない」fixture でも reset されないことを直接示す
///    ために追加/改名) を参照。
///
///    **実装時の実測 (`width: 200%` を 45 段 nest、単一 chain)**: 当初は
///    extent (`location + size <= parent.size`) を検査していたが、この
///    legitimate な (どの深さでも「child は parent の 2 倍」という一貫した
///    関係を表す) declaration が深い段で誤って reset されることが判明した —
///    child の origin は常に `(0, 0)` のままなので、origin だけを見る現行の
///    定義では reset されない
///    (`nested_percentage_wide_child_chain_is_not_reset` が pin)。
///
///    **過去に発見された gate 不備 (修正済み)**: origin
///    だけを見るようにした直後の版は、なお gate を「`parent.size` /
///    `child.size` / `child.location` のいずれか 1 つでも飽和していれば
///    axis 区別なく両 axis を検査する」という条件にしていた。この形では
///    **child 自身の location が飽和していなくても** — 例えば同じ subtree の
///    どこか別の node の `parent.size` が (無関係な原因で) 飽和していた
///    だけで — legitimate な負 margin (`location.x` が通常範囲の負値、
///    例: `-30`) を持つ child が `>= 0.0` に落ちて誤って reset されうる、と
///    非軽微な finding として指摘された。現在の
///    axis 単位 gate (`child.location.x` / `.y` 自身が飽和している場合だけ、
///    その axis だけを検査する) はこれを構造的に閉じる — 検査対象になる
///    field は必ず「それ自身が近似された」field に限られるため、legitimate
///    な小さい負値がこの gate を通ることはない
///    (`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    が直接 pin する — 「`y` 軸だけでも reset の説明がつく」fixture では
///    新旧実装を区別できないという指摘を受けて、`y` 軸が
///    飽和かつ収まっている fixture に差し替えた経緯は同 test の doc参照。
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    はこの変更が入る前は `y` 軸の検出力が保たれていることの
///    pin だったが、この変更でその検出力自体が失われたため、現在は同 test の
///    doc が記録するとおり別の主張 (どちらの axis も reset の理由に
///    ならない) の pin になっている)。
///
///    **負方向の false positive (当初は残余リスクとして認識されていたが、
///    後に解決)**: axis 単位の gate まで閉じた上でも、飽和した axis 自身が
///    **負**の場合に固有の false positive が残っていた。当時導入されていた版の
///    再検査式 `child.location >= 0.0 && child.location <= parent.size` は、
///    `child.location` が負である間は **恒等的に false** — `>= 0.0` を満たす
///    負数は存在しないので、負方向についてこの式は「containment を
///    re-validate する」のではなく「飽和かつ負なら無条件 reset する」式と
///    数学的に同値だった。CSS Box 3 §3.1 (前掲、`margin` の負値は
///    「implementation-specific limits」の範囲で無制限に許される) の下で、
///    [`MAX_TAFFY_MAGNITUDE`] はまさにその「implementation-specific limit」
///    自身であり、そこに達したこと自体は──合法な負方向の関係が本実装の
///    上限を超えて近似され始めた、というだけで──破綻の証拠にならない。
///    これは同じ関数がすでに無条件で信頼している「飽和していない負値」
///    (`legitimate_negative_margin_overflow_is_not_reset` の `-30` や、深い
///    nest で `-30`,`-60`,…と単調に増大する中間段)と対称であり、境界
///    (`±MAX_TAFFY_MAGNITUDE` にちょうど達する瞬間) だけ扱いが不連続に
///    反転する理由が無い。
///
///    当時はここから「したがって現在の定義は符号で分岐する:
///    飽和した axis が負なら無条件に ok、正なら従来通り `<= parent.size`
///    を再検査する」という結論を導いていた。根拠は 2 つ — (a) `padding` /
///    `border` / `width` は parse 時点で非負が enforce されるため、正方向の
///    巨大な `location` を「CSS が無制限に許す」と正当化する spec 上の
///    対称な根拠が (margin とは違って) 無い、(b) 正方向の再検査は実際に
///    両方の分岐を持つ (`<=` が真になる `saturated_but_contained_layout_is_
///    not_reset`、偽になる旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout`) ので
///    緩めると既存の検出力を実際に失う——というものだった。
///
///    **この根拠 (a) は後に覆った**:
///    margin は symmetric — CSS Box 3 §3.1
///    (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
///    for margin properties are allowed, but there may be
///    implementation-specific limits") は**負値**を明示的に許容している
///    だけで、正の margin をそれより厳しく縛る根拠にはなっていない —
///    `margin-left` の grammar (`<length-percentage> | auto`) は正負どちらの
///    巨大な値も等しく spec-legal であり、[`MAX_TAFFY_MAGNITUDE`] という
///    「implementation-specific limit」に達すること自体は、先に負方向で
///    確立したのと同じ理由で、正方向でも破綻の証拠にはならない。実際
///    `margin-left: 1e9%` (100px container 内) は `location.x` を正方向に
///    飽和させ、旧実装はこれを誤って reset していた — 実測は
///    `saturated_negative_margin_percentage_child_is_not_reset` の正方向対
///    である CSS パイプライン経由の regression test を参照。
///
///    根拠 (b) (「正方向の再検査には現に検出力がある」) はこの変更でも
///    **反証されてはいない** — 反証されたのは (a) だけで、(b) の
///    「検出力を失う」という指摘自体は正しかった。その損失は
///    **承知の上で受け入れられた** — 既存 test は TaffyLayout を直接構築する
///    のみで実 CSS パイプライン経由の検出力を一度も示しておらず、一方で
///    今回の data loss (legitimate content の完全消失) は実 CSS 経由で
///    実証済みだったため。**結果として `axis_ok` は符号を問わず「飽和して
///    いれば無条件 accept」に統一され、[`child_within_parent_border_box`]
///    は常に `true` を返す** — containment を再検査する経路は
///    もう存在しない (`axis_ok` の実装、および doc「符号を問わず無条件
///    accept になった理由」節参照)。「この invariant 自体を維持すべきか」
///    は別途明示的に deferred とされた decision であり、
///    本 doc のこの時点では未解決。
///
///    この変更で挙動が反転した regression pin: 旧
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
///    `saturated_child_outside_parent_is_not_reset` に改名・反転し、旧
///    `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    は
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    に改名・反転した — 詳細はそれぞれの doc を参照。
///
///    **別案として検討し却下したもの**: 「`parent.size` 自身も同じ axis で
///    飽和していれば (符号を見ずに) re-validate をスキップする」という
///    parent 飽和ゲート案は、
///    `saturated_negative_margin_percentage_child_is_not_reset`
///    (単一の `margin-left: -1e9%` 宣言、nest 無し、parent は
///    `width: 100px` で飽和していない) を誤って reset したまま説明できない
///    ——「parent が飽和しているかどうか」ではなく「child 自身の符号」が
///    正しい判別軸である証拠として、この test を上の nested chain test と
///    独立に残している。
///
/// NaN → `0.0` (`sanitize_finite` の doc参照) は飽和境界 (`±MAX_TAFFY_
/// MAGNITUDE`) に一致しないため invariant 2 の gate をすり抜けるが、実害は
/// 無い — `location=(0,0)` `size=(0,0)` は非負サイズの任意の parent に
/// 対して常に「収まっている」ので、invariant 2 が検査対象から漏れても
/// そもそも違反として検出すべき状態にならない。padding/border が巻き込
/// まれて NaN → 0 になり content box が負に振れるケースは invariant 1 が
/// 無条件に (gate なしで) 拾う。
///
/// # 決定的 fallback: subtree をゼロ化 (`zero_layout_subtree`)
///
/// 「入力制限」「途中 saturation」「layout abort」を採らず「決定的
/// fallback」を採る設計判断は決定済み。fallback 値は検討時に挙がった 2 案
/// (「0 サイズ」「直近の有限な親サイズ」) のうち **0 サイズ**を採る —
/// [`taffy::Layout::with_order`] (`order` だけ保持、他は全 zero) は
/// (a) 自明に invariant 1 (`0 - 0 - 0 = 0 >= 0`) と invariant 2 (`(0,0)` は
/// 非負サイズの任意 parent に収まる) の両方を再帰的に満たすため、subtree
/// 全体をこの値で埋めても新たな invariant 違反を作らない (「直近の有限な
/// 親サイズへの fallback」だと、fallback 後の値がさらに invariant 2 を
/// 破らないことを別途保証する必要があり、再検査を繰り返す設計になる)、
/// (b) 検討時の記述でも「0 サイズ」を先に挙げていた、の 2 点から選んだ。
///
/// # traversal は再帰しない
///
/// この関数が対象にする入力 (深い percentage nest — [`MAX_TAFFY_MAGNITUDE`]
/// の doc 表) はまさに**深い DOM tree** なので、[`find_body`] や cascade の
/// deep-nesting 対策と同じ理由で、素朴な再帰で書くと同じ入力で stack
/// overflow の新しい経路を作ってしまう。本関数と [`zero_layout_subtree`]
/// はともに `Vec` を明示 stack として使う iterative 実装。
///
/// # taffy が実際に visit した node だけを見る
///
/// `document.nodes[idx].children` をそのまま辿らず、taffy の
/// `TraversePartialTree` 実装 (`taffy_impl.rs`) と同じ `is_in_document()`
/// filter を子の走査に適用する — `<template>` descendants など taffy が
/// そもそも layout しなかった node は `unrounded_layout` が構築時デフォルト
/// (`Layout::with_order(0)`) のまま (他 subtree の古い値が紛れ込むわけでは
/// ない) なので対象に含めても実害は無いが、taffy の traversal 契約と揃えて
/// おく方が読み手にとって驚きが無い。
///
/// # 呼び出し元
///
/// [`layout_single_page`] の Step 5 (`compute_root_layout`) 直後、Step 6
/// (warning replay) の前 — root (`<body>`) の下で taffy が実際に書いた全
/// `unrounded_layout` が揃った直後に 1 回だけ走る。`compute_root_layout` を
/// 直に呼ぶ経路 (`lib.rs` の各 unit test) はこの pass を経由しない —
/// それらのテストは `sanitize_taffy_layout` (finiteness のみ) の
/// characterization が目的であり、意味的 invariant は対象外
/// (`taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry` の
/// doc参照)。
pub(crate) fn enforce_layout_invariants(document: &mut Document, root_idx: usize) {
    let mut content_box_violations = 0usize;
    // `child_within_parent_border_box` now always
    // returns `true` (see its doc), so the `if` below that increments this
    // is unreachable for any input — `containment_violations` can never
    // exceed 0, and the `LayoutWarn::GeometryInvariantViolated { invariant:
    // "child_within_parent_border_box", .. }` warning below can never be
    // emitted. Kept (not deleted) because removing the dead branch is part
    // of the deferred "should this invariant check exist at all" follow-up.
    let mut containment_violations = 0usize;
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let layout = document.nodes[idx].unrounded_layout;
        if layout.content_box_width() < 0.0 || layout.content_box_height() < 0.0 {
            zero_layout_subtree(document, idx);
            content_box_violations += 1;
            continue;
        }
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if !document.nodes[child_idx].is_in_document() {
                continue;
            }
            let child_layout = document.nodes[child_idx].unrounded_layout;
            if !child_within_parent_border_box(&layout, &child_layout) {
                zero_layout_subtree(document, child_idx);
                containment_violations += 1;
            } else {
                stack.push(child_idx);
            }
        }
    }
    if content_box_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: content_box_violations,
            },
        );
    }
    if containment_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "child_within_parent_border_box",
                subtree_count: containment_violations,
            },
        );
    }
}

/// [`enforce_layout_invariants`] の fallback 本体 — `root_idx` を根とする
/// subtree (taffy が実際に訪問した node のみ、`is_in_document()` filter) の
/// `unrounded_layout` を [`taffy::Layout::with_order`] (`order` だけ保持し
/// 他は全 zero) で上書きする。iterative (`Vec` stack) — 対象がまさに深い
/// DOM である以上、再帰は使わない ([`enforce_layout_invariants`] の
/// 「traversal は再帰しない」節参照)。
fn zero_layout_subtree(document: &mut Document, root_idx: usize) {
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let order = document.nodes[idx].unrounded_layout.order;
        document.nodes[idx].unrounded_layout = TaffyLayout::with_order(order);
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
}

/// [`MAX_TAFFY_MAGNITUDE`] の対称 clamp 境界にちょうど乗っているかどうか。
/// `sanitize_finite` (`v.clamp(min, max)`) は範囲外の有限値をちょうど
/// `min` / `max` に丸めるので、この等価判定は「この field で実際に clamp
/// が発火した」ことの正確な proxy になる — `NaN` は `0.0` に丸まる別経路
/// なのでここには現れない ([`enforce_layout_invariants`] の doc「NaN →
/// 0.0 …」節参照)。
fn taffy_magnitude_is_saturated(v: f32) -> bool {
    v == MAX_TAFFY_MAGNITUDE || v == -MAX_TAFFY_MAGNITUDE
}

/// `child` の border box の**原点** (`location`、`child.size` は見ない) が
/// `parent` の border box (parent 座標系の原点 `(0,0)` から `parent.size`)
/// の中にあるかどうかを検査する述語として設計された。**ただし
/// 本関数は常に `true` を返す** — 飽和して
/// いない axis は元から無条件に「ok」、飽和している axis も**符号を問わず**
/// 無条件に「ok」になったため (詳細は下の「符号を問わず無条件 accept に
/// なった理由」節)、
/// containment を実際に再検査する経路はもう存在しない。「この invariant
/// check 自体を維持すべきか」は別途明示的に deferred とされた
/// decision であり、この doc の時点では未解決 (下記参照)。
///
/// # `child.size` を見ない理由
///
/// 当初は `location.x + size.width <= parent.size.width` という **extent**
/// (child の右端/下端まで含めた) containment を検査していたが、これは
/// `width: 200%` のような「子が親より意図的に大きい」legitimate な CSS を
/// 誤検出することが実装時に判明した — `width: 200%` は**どの深さでも** (飽和
/// していない浅い段も含め) 同じ「child は parent の 2 倍」という一貫した
/// 関係を表しており、破綻ではない。深い nest で個々の used value が
/// [`MAX_TAFFY_MAGNITUDE`] の帯を超えて近似され始めても、この関係自体は
/// 変わらない (`legitimate_negative_margin_overflow_is_not_reset` が pin する
/// 「小さい parent + 大きい child」も同じ class の legitimate overflow)。
/// この関数の設計もこの区別を反映しており、「child の
/// **location** が parent の border box 内」とだけ書いている — extent では
/// なく **origin** の containment を指している。この関数はその通り origin
/// だけを見る。
///
/// # gate を axis 単位・`child.location` 自身に限定する理由
///
/// 直前の版は「`parent.size.width/height` か `child.size.width/height` か
/// `child.location.x/y` のいずれか 1 つでも飽和していれば、`x`/`y` **両方**を
/// 検査する」という gate だった。この形には 2 段階の false positive があった:
///
/// 1. **field 単位**: `child.location` 自身は飽和していなくても、
///    無関係な `parent.size` (同じ subtree の別の場所の飽和が原因のことも
///    ある) や `child.size` が飽和しているだけで検査が開いてしまい、
///    legitimate な負 margin (`location.x` が通常範囲の負値、例: `-30`) を
///    `>= 0.0` で弾いて誤って reset していた。
/// 2. **axis 単位**: 1 を「`child.location` 自身が飽和していること」に
///    絞っても、`x` と `y` の**どちらか一方**が飽和していれば両方を検査する
///    形のままだと、飽和していない側の axis に legitimate な負 margin が
///    あると同じ理由で誤検出しうる。
///
/// axis 単位に絞った版はどちらも閉じていた — field 単位で
/// 「検査対象にする/しない」を区別する gate 自体は **axis 自身の
/// `child.location` が実際に飽和しているかどうか**に限っていたため、
/// legitimate な負 margin (通常範囲、飽和していない) を持つ axis が
/// 誤って巻き込まれることはなかった。飽和した field だけが「actual value
/// が taffy の生の計算結果から乖離している」ため区別対象になる、という
/// 本関数群の一貫した設計原則 (本 module doc「なぜ `sanitize_taffy_layout`
/// だけでは閉じないか」節) を axis 粒度まで徹底した形、という説明はこの
/// 時点では正確だった。**この変更以降は、飽和した axis も
/// 無条件 accept になったため、この gate は「どの axis が検査対象になるか」
/// ではなく「どの axis も検査されない」という結果に収束している** — 下の
/// 「符号を問わず無条件 accept になった理由」節参照。
///
/// `saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は当初「飽和した axis だけ検査、他 axis は無条件 ok」という
/// conjunction を直接 pin していた — 飽和している axis 自身は実際には
/// 収まっているようにし、もう一方の (飽和していない) axis に legitimate な
/// 負 margin を与えることで、「`y` 軸だけでも reset の説明がつく」fixture
/// では新旧実装を区別できないという指摘を踏まえた設計だった。この変更以降はこの test の assert 自体は
/// 変わらず通るが (fixture がたまたま「収まっている」ケースなので)、
/// 主張の中身は「どちらの axis も reset の理由にならない」に変わっている
/// (同 test の doc 参照)。旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (現
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// は当初「`y` 軸の検出力」の pin だったが、この変更でその検出力
/// 自体が失われたため reset されなくなった。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// (現 `saturated_child_outside_parent_is_not_reset`) も同様 — 飽和した
/// axis 自身が (かつては) 違反していても、この変更以降はもう検出されない。
///
/// # 符号を問わず無条件 accept になった理由 (負方向 → 正方向の順で変更)
///
/// **負方向**: axis 単位まで絞った直後の版でも
/// なお、**飽和した axis 自身が負**のケースに固有の false positive が
/// 残っていた。再検査式 `child.location >= 0.0 && child.location <=
/// parent.size` は、`child.location` が負である限り `>= 0.0` を
/// 満たしようがないので**恒等的に false** — つまり負方向についてこの式は
/// 「containment を検査する」のではなく「飽和かつ負なら無条件に reset
/// する」ことと同値だった。CSS Box 3 §3.1 (`sanitize_taffy` の doc参照、
/// margin の負値は「implementation-specific limits」の範囲で無制限に
/// 許される) の下では、[`MAX_TAFFY_MAGNITUDE`] こそがその limit そのもので
/// あり、そこに達したこと自体は合法な負方向の関係が本実装の上限を超えて
/// 近似され始めた、というだけで破綻の証拠にはならない — 同じ関数が
/// すでに無条件で信頼している「飽和していない負値」
/// (`legitimate_negative_margin_overflow_is_not_reset` の `-30`) と対称
/// であり、`±MAX_TAFFY_MAGNITUDE` の境界を跨いだ瞬間だけ扱いを不連続に
/// 反転させる理由が無い。
///
/// 当時はここで「正方向は緩めない」と結論していた。根拠は 2 つ —
/// (a) `padding` / `border` / `width` は parse 時点で非負が enforce
/// されるため、正方向の巨大な `location` を margin と同じ「CSS が無制限に
/// 許す」根拠では正当化できない、(b) 正方向の再検査は実際に pass/fail
/// 両方の分岐を持ち (`saturated_but_contained_layout_is_not_reset` が
/// pass、旧 `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// が fail)、緩めると現に存在する検出力を失う — 負方向はそもそも pass
/// する経路が存在しなかったので、失われる検出力は無い、というものだった。
///
/// **正方向**: 根拠 (a) は
/// 覆った — margin は symmetric。CSS Box 3 §3.1
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>、"Negative values
/// for margin properties are allowed, but there may be
/// implementation-specific limits") は**負値**を明示的に許容している
/// だけで、正の margin をそれより厳しく縛る spec 上の対称な根拠にはなって
/// いない。`margin-left: 1e9%` (`width: 100px` container 内) は
/// `location.x` を正方向に飽和させ、旧実装はこれを誤って reset していた —
/// `saturated_positive_margin_percentage_child_is_not_reset` が実際の
/// CSS パイプライン経由でこれを pin する (`saturated_negative_margin_
/// percentage_child_is_not_reset` の正方向対)。根拠 (b) (「検出力を失う」)
/// は反証されていない — その損失は承知の上で受け入れられた: 既存 test
/// (`saturated_but_contained_layout_is_not_reset`、旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`) は
/// TaffyLayout を直接構築するのみで実 CSS パイプライン経由の検出力を
/// 一度も示していなかった一方、今回の data loss (legitimate content の
/// 完全消失) は実 CSS 経由で実証済みだったため。
///
/// **結果**: `axis_ok` は符号を問わず「飽和していれば無条件 accept」に
/// 統一され、本関数は常に `true` を返す — containment を再検査
/// する経路はもう存在しない。旧
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout` は
/// `saturated_child_outside_parent_is_not_reset` に、旧
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// は
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// に、それぞれ改名・反転した (詳細は各 test の doc参照)。「この
/// invariant check 自体を維持すべきか」は別途明示的に deferred
/// とされた decision であり、この doc の時点では未解決。
///
/// `saturated_negative_margin_percentage_child_is_not_reset` (単一の
/// `margin-left: -1e9%` 宣言、nest 無し) と
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
/// (`width: 200%; margin-left: -100%` の深い nest chain、
/// `nested_percentage_wide_child_chain_is_not_reset` の負方向対) が
/// 実際の CSS パイプライン経由でこれを pin する。前者は特に、「`parent.size`
/// 自身も同じ axis で飽和していれば符号を見ずに re-validate をスキップ
/// する」という検討したが却下した別案を反証する最小 fixture でもある —
/// この fixture は `parent.size.width` が飽和していない (`100.0` のまま)
/// ので、判別軸は「parent も飽和しているか」ではなく「child 自身の符号」
/// でなければならないことを示す (この変更以降、この判別軸自体は意味を失った
/// が、fixture と regression pin としての価値は変わらない)。
/// `saturated_negative_location_is_not_reset` は同じ組み合わせを直接構築
/// した最小 synthetic case で孤立させて検査する
/// (`saturated_but_contained_layout_is_not_reset` と対になる、正方向
/// ケースの負方向対 — この変更以降はどちらも「飽和していれば無条件 accept」
/// という同じ結論の pin)。
fn child_within_parent_border_box(parent: &TaffyLayout, child: &TaffyLayout) -> bool {
    /// 1 axis 分の判定。`location` はその axis の `child.location.{x,y}`
    /// (`parent_size` は本体の式ではもう一切使わない — dead parameter。
    /// 削除せず「対応する `parent.size.{width,height}` を渡す」という
    /// 呼び出し規約の見た目だけ残しているのは、この関数・`axis_ok` 自体を
    /// 削除するかどうかを含めて「この invariant check を維持すべきか」が
    /// 別途明示的に deferred とされた follow-up だから — 将来その follow-up
    /// で `axis_ok` ごと削除される可能性があることを見越して、今
    /// signature を先回りして変える判断はしていない。詳細は上の doc
    /// 「符号を問わず無条件 accept になった理由」節。同じ理由で、直下の
    /// `if !taffy_magnitude_is_saturated(location) { return true; }` 分岐
    /// も実質的には常に `true` を返す既定文と等価な dead branch になって
    /// いる — こちらも `axis_ok` ごと削除されうる同じ deferred follow-up
    /// まで、あえて `true` 一本に畳んでいない)。
    fn axis_ok(location: f32, _parent_size: f32) -> bool {
        if !taffy_magnitude_is_saturated(location) {
            return true;
        }
        // 飽和していれば符号を問わず無条件 accept。
        // 負方向は先に確立していた — 本 decision は
        // その前例を正方向にも対称に拡張し、旧 `location <= parent_size`
        // 再検査 (正方向限定) を撤去した。containment を再検査する経路は
        // もう存在しない。
        true
    }
    axis_ok(child.location.x, parent.size.width) && axis_ok(child.location.y, parent.size.height)
}

/// [`ComputedLengthPercentage`] → [`taffy::LengthPercentage`] bridge
/// (padding / gap 用)。
///
/// `site` distinguishes the callers (`"padding"` / `"row-gap"` /
/// `"column-gap"`) in [`LayoutWarn::NonFiniteClamped`] events — same
/// `site`-parameter shape as
/// [`computed_length_percentage_or_auto_to_taffy_dimension`]
/// (`"width"`/`"height"`), generalized once a second caller
/// ([`bridge_gap`]) appeared.
///
/// # 網羅 match
///
/// 引数が **computed 層**の型になったため 2 arm で網羅する。以前あった
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` (font-relative unit を黙って
/// 0px に潰す fail-quiet) と `_ => length(0.0)` (non_exhaustive catch-all) は
/// **削除した** — `em` / `rem` / `pt` は cascade の phase 3 で px に絶対化済み
/// であり、computed 層に到達しない。
///
/// **ただし削除した arm は「単位」だけでなく「病的な f32 の値」も吸収していた**
/// (`Em(inf)` / `0.0 * inf` = NaN)。その分は [`sanitize_taffy`] が
/// backfill している — **guard を「不要な防御」と
/// 判断して外さないこと。**
///
/// [`ComputedLengthPercentage`] に `#[non_exhaustive]` が付いていないのは、
/// この網羅性を今得るための explicit trade である (将来 `Calc` variant が
/// 増えるときに coordinated breaking change を払う。`raikiri_style::resolve`
/// の module doc 参照)。**`_` arm を足して「forward-compat」にしてはならない** —
/// trade の得る側を捨てることになる。
///
/// # Percent policy
///
/// `Percent(p)` → `percent(sanitize_taffy(p / 100.0))` — CSS spec の authored
/// 0-100 を taffy の fraction 0.0-1.0 に変換し、[`sanitize_taffy`] で有限化する。
/// containing block に対する解決は **used value 層**
/// (CSS Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>) であり
/// taffy に委譲する — **guard が bound するのは fraction であって解決後の
/// used value ではない** ([`MAX_TAFFY_MAGNITUDE`] の射程節を参照)。
fn computed_length_percentage_to_taffy_length_percentage(
    len: ComputedLengthPercentage,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    match len {
        // site 1: `sanitize_taffy` で非有限を落とす。
        ComputedLengthPercentage::Px(v) => LengthPercentage::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentage::Percent(p) => {
            LengthPercentage::percent(sanitize_taffy(p / 100.0, site, diag))
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::Dimension`] bridge
/// (width / height 用)。
///
/// 網羅 match (`Px` / `Percent`) の分岐ロジック・Percent policy・非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ (参照先は
/// 2 arm)。本関数はそれに `Auto` arm が加わり合計 3 arm (catch-all なし)。
/// `Auto` → `Dimension::auto()` (f32 を持たないので guard 対象外)。
///
/// [`bridge_size`] から width / height 両方で consume される。
///
/// `site` distinguishes the two [`bridge_size`] callers (`"width"` /
/// `"height"`) in [`LayoutWarn::NonFiniteClamped`] events.
fn computed_length_percentage_or_auto_to_taffy_dimension(
    loa: ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> Dimension {
    match loa {
        // site 2。
        ComputedLengthPercentageOrAuto::Px(v) => Dimension::length(sanitize_taffy(v, site, diag)),
        ComputedLengthPercentageOrAuto::Percent(p) => {
            Dimension::percent(sanitize_taffy(p / 100.0, site, diag))
        }
        ComputedLengthPercentageOrAuto::Auto => Dimension::auto(),
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::LengthPercentageAuto`] bridge
/// (margin 用)。
///
/// 網羅 match / Percent policy / 非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ。`Auto` →
/// `LengthPercentageAuto::auto()` (CSS Box 3 §3.1 "margin auto = distribute
/// available space" を taffy に委譲、f32 を持たないので guard 対象外)。
fn computed_length_percentage_or_auto_to_taffy_length_percentage_auto(
    loa: ComputedLengthPercentageOrAuto,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentageAuto {
    match loa {
        // site 3。margin は負値が spec-valid なので
        // `sanitize_taffy` の対称 clamp が load-bearing。唯一の caller
        // (`bridge_margin`) 由来なので site label は固定。
        ComputedLengthPercentageOrAuto::Px(v) => {
            LengthPercentageAuto::length(sanitize_taffy(v, "margin", diag))
        }
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0, "margin", diag))
        }
        ComputedLengthPercentageOrAuto::Auto => LengthPercentageAuto::auto(),
    }
}

/// [`ComputedLength`] (px) → [`taffy::LengthPercentage`] bridge
/// (`border-*-width` 用)。
///
/// sibling 3 helper (`computed_length_percentage_to_taffy_length_percentage` /
/// `computed_length_percentage_or_auto_to_taffy_dimension` /
/// `computed_length_percentage_or_auto_to_taffy_length_percentage_auto`) と同じ
/// `<src>_to_taffy_<dst>` 命名 / 同じ cluster に置く。
///
/// `border-*-width` 専用に分けているのは、grammar (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) が `<percentage>` を含まないため
/// computed 層でも length しか来ないから (CSS Backgrounds 3 §3.3 "Line
/// Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。戻り値型は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同一
/// (`LengthPercentage` が taffy 側の最小共通型) だが、入力型が
/// [`ComputedLength`] なので percentage arm を
/// 持たない点が違う。非有限 guard ([`sanitize_taffy`]) は同じく通す。
fn computed_length_to_taffy_length_percentage(
    len: ComputedLength,
    diag: &mut Vec<LayoutWarn>,
) -> LengthPercentage {
    // site 4。唯一の caller (`bridge_border`) 由来
    // なので site label は固定。
    LengthPercentage::length(sanitize_taffy(len.px(), "border-width", diag))
}

/// 全 Text node を parley で pre-shape、結果を `Node.text_layout` に格納する。
///
/// 呼び出し側 (`layout_single_page`) は事前に全 `Node.text_layout = None` に
/// clear 済であることを前提とする (re-entrance safety)。
///
/// Font stack / size / weight / style / line-height は
/// `cascade.computed[idx]` (親から inherit 済) を消費。
/// `max_advance` は行折り返し境界で、通常 `page_box.width`。
///
/// # `line_height` は taffy の `compute_root_layout` より前でも正しく配線できる
///
/// 本関数は `layout_single_page` 内で taffy の `compute_root_layout` より
/// **前**に走る (下記 `text_align` 参照)。これは一般に「taffy が確定させる
/// 幅を必要とする property」には問題になるが、`line_height` はその種類の
/// property ではない — 依存方向が逆であり、むしろこの順序が**必要**:
///
/// - parley は line height を `RunMetrics.line_height` として shape 時点
///   (`builder.build(&text)` 内、`break_all_lines` より前) に計算する
///   (`parley-0.10.0/src/layout/data.rs` の `push_run`)。入力はフォント
///   metrics (ascent / descent / leading、shape 対象フォントから直接取得)
///   と `font_size` (本関数がすでに push した自要素の computed font-size)、
///   そして本関数が push する `StyleProperty::LineHeight` の値だけであり、
///   taffy が確定させる containing block 幅や利用可能領域には一切依存しない
///   (`parley-0.10.0/src/resolve/mod.rs` の対応 arm も `device pixel scale`
///   の乗算のみ)。
/// - 逆に、shape 済みの `Layout::height()` (line height を織り込み済み) は
///   `taffy_impl.rs` の `compute_child_layout` が leaf node の intrinsic
///   size として taffy に**渡す側**の入力になる。つまり line height は
///   taffy の出力を必要とするのではなく、taffy の入力を作る側に立つ —
///   `text_align` (taffy が確定させる幅を align の基準として必要とする、
///   下記参照) とは依存の向きが逆であり、本関数がここで走ることは
///   line height にとって「早すぎる」のではなくちょうど必要なタイミングである。
///
/// # 未消費の `ComputedValues` field
///
/// `cascade.computed[idx]` には他にも `direction` / `text_align` が乗って
/// いるが、本関数はいずれも読まない:
///
/// - `direction` — 配線先の API 自体が無い。`RangedBuilder` / `TreeBuilder`
///   は base direction を受け取る public API を公開しておらず、parley 内部の
///   bidi resolver は base level 引数に常に `None` を渡して呼ばれる (段落内の
///   文字列から Unicode Bidirectional Algorithm の P2/P3
///   first-strong-character heuristic で自動推定し、強い方向を持つ文字が
///   無ければ LTR に fallback — Unicode Standard Annex #9
///   <https://www.unicode.org/reports/tr9/>)。つまり `cv.direction` を明示的
///   に渡す先の API 自体が現状無い — LTR がハードコードされた default なの
///   ではない。
/// - `text_align` — 末尾の `layout.align(...)` 呼び出し自体は live だが
///   `Alignment::Start` に固定されており `cv.text_align` を読まない
///   (経路が無いのではなく、initial value に pin された stub)。単純な
///   enum mapping への置き換えでは不十分な点に注意: 本関数は
///   `layout_single_page` 内で taffy の `compute_root_layout` より **前**に
///   走るため、`align()` に渡せる幅は `max_advance` (= 通常
///   `page_box.width`) のみで、taffy が確定させる実際の containing block
///   幅ではない。`Start` は align 先の幅に依存しないため無害だが、
///   `Center` / `Right` / `End` / `Justify` を素朴に渡すと、ページ全体の
///   幅を基準にズレて align された glyph run を生む。加えて
///   `parley::Alignment::Start` / `End` は layout 内の bidi 解析結果から
///   physical 方向を解決するため、`direction` を配線せずに `text_align`
///   だけ配線しても `Start`/`End` は正しく解決されない — この 2 つは
///   独立した gap ではなく 1 セットとして扱う必要がある (line_height とは
///   異なり、taffy が確定させる幅を実際に必要とする依存方向のため、本関数
///   より後 — `compute_root_layout` 後 — の再設計を要する)。
///
/// # 失敗しない
///
/// 以前は `Result<(), LayoutError>` を返していた。唯一の `Err` 経路は
/// `cv.font_size` が specified 層の `Length` で `Px` 以外だった場合の
/// `LayoutError::Internal` だったが、`font_size` が [`ComputedLength`] (px) に
/// なって match 自体が消えたため到達不能になった。`pub(crate)` なので戻り値の
/// narrowing は crate 内で完結する (外部影響 0)。
///
/// [`ComputedLength`]: raikiri_style::ComputedLength
/// [`ComputedLineHeight`]: raikiri_style::ComputedLineHeight
pub(crate) fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) {
    // Cheap threshold: for tiny DOMs sequential is faster than rayon overhead.
    // Collect eligible Text nodes first to avoid borrowing `doc.nodes` mutably
    // across parallel tasks (which would require Sync on Document). Owned jobs
    // are Send and avoid sharing &Document across threads.
    struct Job {
        idx: usize,
        text: String,
        family_str: String,
        font_size_raw: f32,
        font_weight_raw: f32,
        font_style: StyleFontStyle,
        line_height_raw: ComputedLineHeight,
    }

    let mut jobs: Vec<Job> = Vec::with_capacity(doc.nodes.len() / 2);
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        let text: String = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(t) if !t.text_content.is_empty() => {
                t.text_content.as_str().to_string()
            }
            _ => continue,
        };
        let cv = &cascade.computed[idx];
        let family_str: String = cv
            .font_family
            .iter()
            .map(|a| a.0.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        jobs.push(Job {
            idx,
            text,
            family_str,
            font_size_raw: cv.font_size.px(),
            font_weight_raw: cv.font_weight,
            font_style: cv.font_style,
            line_height_raw: cv.line_height,
        });
    }

    if jobs.is_empty() {
        return;
    }

    // Copy original FontContext once; each rayon task clones from this base.
    // LayoutContext is cheap (clone returns new empty), so per-task new() is fine.
    // For very small job counts rayon overhead dominates; use sequential fallback.
    const PAR_THRESHOLD: usize = 32;
    if jobs.len() < PAR_THRESHOLD {
        for job in jobs {
            let mut warnings: Vec<LayoutWarn> = Vec::new();
            let font_size_px = sanitize_finite(
                job.font_size_raw,
                0.0,
                MAX_FONT_SIZE_PX,
                "font-size",
                &mut warnings,
            );
            let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
            let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
            let font_family = FontFamily::from(job.family_str.as_str());
            let mut builder = layout_cx.ranged_builder(fonts, &job.text, 1.0, true);
            builder.push_default(StyleProperty::FontFamily(font_family));
            builder.push_default(StyleProperty::FontSize(font_size_px));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
            builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                job.font_style,
            )));
            builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                line_height,
            )));
            let mut layout: Layout<()> = builder.build(&job.text);
            layout.break_all_lines(Some(max_advance));
            layout.align(Alignment::Start, AlignmentOptions::default());
            doc.layout_warnings.extend(warnings);
            if let Some(t) = doc.nodes[job.idx].data.as_text_mut() {
                t.text_layout = Some(layout);
            }
        }
        return;
    }

    let base_fonts: FontContext = fonts.clone();
    // Parallel shaping: chunked to amortize FontContext/LayoutContext setup.
    // Per-job `LayoutContext::new()` is expensive (ICU AnalysisDataSources etc.)
    // so we reuse one FontContext+LayoutContext per rayon chunk.
    // Chunk size 128 reduces clones to ~4/16 for 500/2000 jobs.
    let results: Vec<(usize, Layout<()>, Vec<LayoutWarn>)> = jobs
        .par_iter()
        .chunks(256)
        .flat_map(|chunk| {
            let mut fonts_thread = base_fonts.clone();
            let mut lcx = LayoutContext::<()>::new();
            let mut out = Vec::with_capacity(chunk.len());
            for job in chunk {
                let mut warnings: Vec<LayoutWarn> = Vec::new();
                let font_size_px = sanitize_finite(
                    job.font_size_raw,
                    0.0,
                    MAX_FONT_SIZE_PX,
                    "font-size",
                    &mut warnings,
                );
                let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
                let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
                let font_family = FontFamily::from(job.family_str.as_str());
                let mut builder = lcx.ranged_builder(&mut fonts_thread, &job.text, 1.0, true);
                builder.push_default(StyleProperty::FontFamily(font_family));
                builder.push_default(StyleProperty::FontSize(font_size_px));
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                    job.font_style,
                )));
                builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                    line_height,
                )));
                let mut layout: Layout<()> = builder.build(&job.text);
                layout.break_all_lines(Some(max_advance));
                layout.align(Alignment::Start, AlignmentOptions::default());
                out.push((job.idx, layout, warnings));
            }
            out
        })
        .collect();

    for (idx, layout, warnings) in results {
        doc.layout_warnings.extend(warnings);
        if let Some(t) = doc.nodes[idx].data.as_text_mut() {
            t.text_layout = Some(layout);
        }
    }
}

/// 単一 A4 (or 指定 PageBox) ページに Document を layout する。
///
/// # 変更 (in-place)
/// - Node.text_layout を全 `None` にクリア (re-entrance safety)
/// - `apply_computed_to_style` で computed → taffy::Style bridge (現時点では no-op)
/// - `preshape_text` で全 Text node を parley shape、Node.text_layout に格納
/// - `apply_page_box_to_body` で body.style.size = length(PageBox)
/// - `compute_root_layout` で taffy 計算、Node.unrounded_layout に書き込む
///
/// # Errors
/// - `LayoutError::Internal` — `<body>` element が見つからない (fragment
///   parse は現行実装では非対応) / taffy internal
///
///   parley shape (`preshape_text`) は **失敗しない**
///   (同関数の doc 参照)。
///
/// # Non-goals (現時点)
/// - 同じ Document で複数回呼ぶことは safe (text_layout を毎回 clear) だが、
///   incremental (差分だけ再走) は将来追加予定
/// - Consumer からの PageBox 上書きは将来の per-page PageBox 対応で扱う
/// - Fragment parse (no `<body>`) support は将来追加予定
/// # API 互換性
///
/// この signature は以前の 3-arg `(document, cascade, page_box)` から
/// 4-arg `(document, cascade, page_box, font_ctx)` に **意図的に breaking
/// change** された (choice β)。α (dual API: 既存 3-arg +
/// 新規 `_with_fonts`) との trade-off の末、raikiri-dom 内 caller が全て
/// in-repo (12 箇所 = production 1 + test 11) であり、内部 DI の explicit
/// 化と signature 統一の方が長期保守で優れると判断した。
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    mut font_ctx: FontContext,
) -> Result<(), LayoutError> {
    // observation-side entry で
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
    // Step 0b: layout_warnings re-entrance clear —
    // same rationale as the text_layout clear above: this Vec is populated
    // over the course of a pass (bridges below, then the taffy compute step
    // via `set_unrounded_layout`) and drained near the end of this function,
    // but an early `?` return (Step 3) would otherwise leave a previous call's
    // leftover entries for the next call to inherit.
    document.layout_warnings.clear();

    // Step 1: ComputedValues → taffy::Style bridge (現時点では no-op site)
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
    );

    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
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

    // Step 5b: 親子 geometry の意味的 invariant を検査し、破れている subtree
    // を決定的 fallback (ゼロ) に倒す。Step 5 の内部
    // (`set_unrounded_layout` 経由の `sanitize_taffy_layout`) が保証するのは
    // finiteness だけなので、その一段上のレイヤーとしてここに置く —
    // `enforce_layout_invariants`'s doc 参照。`document.layout_warnings` へ
    // 積む event は Step 1/2 と同じ buffer で、Step 6 がまとめて drain する。
    enforce_layout_invariants(document, body_id);

    // Step 6: replay buffered LayoutWarn events.
    //
    // `document.layout_warnings` accumulated events from this function's own
    // bridge calls (Step 1 / Step 2, via `&mut document.layout_warnings`
    // passed directly), from `<Document as
    // taffy::LayoutPartialTree>::set_unrounded_layout` (invoked internally
    // by `compute_root_layout` just above, via `self` — see
    // `Document::layout_warnings`'s doc for why that trait-fixed signature
    // can only reach an owned buffer, not a live observer), and from Step 5b
    // (`enforce_layout_invariants`) just above, which
    // pushes into the same buffer directly since it already holds `&mut
    // Document`.
    //
    // No external caller can supply an observer yet — `layout_single_page`'s
    // signature is a dom→paint boundary (`raikiri-paint` and `raikiri` both
    // call it directly) and adding a parameter, or a new `_with_observer`
    // sibling, is a decision for that wall rather than this task. `observer`
    // is therefore always `None` today, so this always falls back to the
    // same `eprintln!` shape `fonts.rs` uses when uncalled with an observer —
    // but the plumbing is real and ready for a future `_with_observer`
    // sibling to wire an observer through with no further refactor.
    let mut observer: LayoutWarnObserver<'_> = None;
    for event in document.layout_warnings.drain(..) {
        emit_layout_warn(&mut observer, event);
    }

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
        preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);

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
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);
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
        // display bridge active — DisplayValue → taffy::Display
        // mapping が正しく行われていることを確認する regression pin。
        //
        // bridge_margin が dispatch に加わったが
        // margin unspecified の element では initial `Sides::all(Length::Px(0.0))`
        // が cascade で入る → taffy `LengthPercentageAuto::length(0.0)` に translate、
        // これは `taffy::Style::default().margin` (all `Length(0.0)`) と一致するため
        // 既存 assertion は無変更で通ることを確認する pin にもなる。
        //
        // bridge_padding も dispatch に
        // 加わったが同様に padding unspecified の element では initial
        // `Sides::all(Length::Px(0.0))` → taffy `LengthPercentage::length(0.0)`
        // が入り、`taffy::Style::default().padding` と一致するため padding assertion
        // も無変更で通る pin。
        //
        // bridge_size (width) が dispatch に
        // 加わったが width unspecified の element は initial `LengthOrAuto::Auto`
        // → `Dimension::auto()` に translate、これは `taffy::Style::default().size`
        // (`Size::auto()`) の width と一致 (height は default 保持のまま追加予定)。
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
    fn establish_minimal_line_boxes_upgrades_qualifying_container_to_flex_row() {
        use raikiri_style::{build_rule_tree, cascade};

        // `build_rule_tree` deliberately excludes UA CSS (its own doc:
        // "UA CSS は含めない" — UA is injected via `raikiri_html::parse`
        // during real HTML parsing, which this hand-built-arena test
        // fixture bypasses). So every element's own display is declared
        // explicitly via inline style below, rather than relying on a
        // `p { display: block }` / `b { display: inline }` UA default
        // that would not actually be present in this fixture.
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A ");
        let b = doc.append_element(Some(p), "b", Style::default(), Some("display: inline"));
        let _b_text = doc.append_text(b, "B");
        let _c = doc.append_text(p, " C");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        assert_eq!(doc.nodes[p].style.display, Display::Flex);
        assert_eq!(doc.nodes[p].style.flex_direction, TaffyFlexDirection::Row);
        assert_eq!(doc.nodes[p].style.flex_wrap, TaffyFlexWrap::NoWrap);
        assert_eq!(
            doc.nodes[p].style.align_items,
            Some(TaffyAlignItems::FLEX_START)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT),
            "qualifying container must have IS_INLINE_ROOT set"
        );
        // Participating children get flex_grow/flex_shrink pinned to 0 so
        // they neither grow nor compress narrower than their own shaped
        // content (see this pass's doc, "flex-shrink hazard").
        for &c in &doc.nodes[p].children {
            assert_eq!(doc.nodes[c].style.flex_grow, 0.0);
            assert_eq!(doc.nodes[c].style.flex_shrink, 0.0);
        }

        // `b` is a plain `inline` element (not `block`/`inline-block`), so
        // it does not itself qualify regardless of its own child count —
        // covered separately by
        // `establish_minimal_line_boxes_excludes_plain_inline_container`.
        // Here it just needs to remain block-container-shaped (i.e. its
        // own single Text child is laid out exactly as before this pass).
        assert_eq!(doc.nodes[b].style.display, Display::Block);
        assert!(!doc.nodes[b].flags.contains(NodeFlags::IS_INLINE_ROOT));
    }

    #[test]
    fn establish_minimal_line_boxes_leaves_single_inline_child_container_on_block_path() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.display,
            Display::Block,
            "a single inline-level child is geometrically degenerate \
             (nothing to place beside it) — must be left on the plain \
             block path, not upgraded to a 1-item flex row"
        );
        assert!(!doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT));
    }

    #[test]
    fn establish_minimal_line_boxes_leaves_mixed_block_and_inline_content_untouched() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _text = doc.append_text(p, "A");
        let _div = doc.append_element(Some(p), "div", Style::default(), Some("display: block"));

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.display,
            Display::Block,
            "a block-level sibling among inline-level content must \
             disqualify minimal-line-box treatment (mixed content is out \
             of scope for this pass)"
        );
        assert!(!doc.nodes[p].flags.contains(NodeFlags::IS_INLINE_ROOT));
    }

    #[test]
    fn establish_minimal_line_boxes_hidden_sibling_does_not_count_toward_threshold() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");
        let _hidden = doc.append_element(Some(p), "span", Style::default(), Some("display:none"));

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // Only 1 *counted* inline-level child (the display:none sibling
        // doesn't count) — below the 2-child threshold.
        assert_eq!(doc.nodes[p].style.display, Display::Block);
    }

    #[test]
    fn establish_minimal_line_boxes_hidden_sibling_does_not_disqualify_when_threshold_met() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");
        let _hidden = doc.append_element(Some(p), "span", Style::default(), Some("display:none"));
        let _c = doc.append_text(p, "C");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.display,
            Display::Flex,
            "a display:none sibling must not block qualification once the \
             2 visible-inline-level threshold is otherwise met"
        );
    }

    #[test]
    fn qualifies_for_minimal_line_box_skips_not_in_document_sibling() {
        // Regression pin for the `if !doc.nodes[c].is_in_document() {
        // continue; }` guard in `qualifies_for_minimal_line_box`: a
        // Comment sibling is reachable in the raw arena tree but always
        // has `IS_IN_DOCUMENT` cleared by `mark_in_document_flags`
        // (`NodeData::Comment`'s doc) — it must be skipped entirely
        // (neither counted nor disqualifying) while the 2 real Text
        // children still meet the threshold.
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");
        let comment = doc.append_comment(Some(p), "not rendered");
        let _c = doc.append_text(p, "C");
        doc.mark_in_document_flags();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !doc.nodes[comment].is_in_document(),
            "fixture precondition: the comment sibling must actually be \
             not-in-document for this test to exercise the guard"
        );

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.display,
            Display::Flex,
            "a not-in-document sibling (e.g. a Comment) must be skipped \
             entirely by the is_in_document() guard — neither counted \
             nor disqualifying — while the 2 real Text children still \
             meet the threshold"
        );
    }

    #[test]
    fn establish_minimal_line_boxes_excludes_plain_inline_container() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        // 2 text children, but `b` itself must not qualify: a plain
        // `inline` node's content flows into its ancestor's line box, it
        // does not get its own (this module's doc, "does not qualify
        // here even with 2+ inline-level children").
        let b = doc.append_element(Some(body), "b", Style::default(), Some("display: inline"));
        let _x = doc.append_text(b, "X");
        let _y = doc.append_text(b, "Y");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        assert_eq!(doc.nodes[b].style.display, Display::Block);
        assert!(!doc.nodes[b].flags.contains(NodeFlags::IS_INLINE_ROOT));
    }

    #[test]
    fn establish_minimal_line_boxes_qualifies_inline_block_container() {
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let ib = doc.append_element(
            Some(body),
            "span",
            Style::default(),
            Some("display:inline-block"),
        );
        let _x = doc.append_text(ib, "X");
        let _y = doc.append_text(ib, "Y");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[ib].style.display,
            Display::Flex,
            "inline-block generates a block container for its own content, \
             same as block — it must qualify just like a block container"
        );
        assert!(doc.nodes[ib].flags.contains(NodeFlags::IS_INLINE_ROOT));
    }

    #[test]
    fn establish_minimal_line_boxes_lays_out_children_side_by_side_not_stacked() {
        // End-to-end pin (via the full `layout_single_page` pipeline, not
        // just `apply_computed_to_style` in isolation) that qualifying
        // inline-level siblings actually end up beside each other, not
        // independently stacked — the concrete geometry bug this pass
        // fixes.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let a = doc.append_text(p, "AAAA");
        let c = doc.append_text(p, "CCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[a].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.location.y - c_loc.location.y).abs() < 1e-3,
            "both text children must sit on the same line (same y), got \
             a.y={} c.y={}",
            a_loc.location.y,
            c_loc.location.y
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.x >= a_loc.location.x + a_loc.size.width - 1e-3,
            "second child must start at or after the first child's right \
             edge (side-by-side placement), got a.x={} a.w={} c.x={}",
            a_loc.location.x,
            a_loc.size.width,
            c_loc.location.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.x > 0.0,
            "second child must not sit at x=0 — that would mean it's \
             still being independently stacked below the first child \
             rather than placed beside it"
        );
    }

    #[test]
    fn establish_minimal_line_boxes_does_not_compress_children_narrower_than_shaped_text() {
        // Regression pin for the flex-shrink hazard documented on
        // `establish_minimal_line_boxes`: flex items default to
        // `flex-shrink: 1`, which under `flex_wrap: NoWrap` would (absent
        // this pass's override) compress each child's box narrower than
        // its own already-shaped glyph run once the combined content
        // exceeds the line's available width — the box would shrink but
        // the glyph run would not, so the rendered glyphs would overflow
        // the box and can overlap a neighboring child's glyphs.
        //
        // Two children, each a single unbroken run with no whitespace (so
        // `preshape_text`'s own per-node soft-wrap against the full page
        // width has no break opportunity and leaves each on one line,
        // regardless of length — see the `layout.len() == 1` precondition
        // below), together wide enough to exceed the page — real shrink
        // pressure, absent the override, would apply.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display: block; font-size: 72px"),
        );
        let a = doc.append_text(p, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        let c = doc.append_text(p, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let mut shaped_widths = Vec::new();
        for &id in &[a, c] {
            let layout = doc.nodes[id]
                .text_layout()
                .expect("text node must have a preshaped layout");
            // Fixture precondition: each string is one unbroken run with
            // no whitespace, so `preshape_text`'s own per-node soft-wrap
            // (against the full page width) has no break opportunity and
            // leaves it on a single line regardless of length — pin that
            // so a `shaped_width` below isn't silently the width of one
            // wrapped sub-line instead of the whole run.
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                layout.len(),
                1,
                "fixture precondition: each child's text must shape to \
                 exactly 1 line (no internal wrap) so its `width()` below \
                 reflects the whole run, not one wrapped sub-line"
            );
            shaped_widths.push(layout.width());
        }
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            shaped_widths[0] + shaped_widths[1] > PageBox::A4.width,
            "fixture precondition: the two children combined ({} + {}) \
             must exceed the page width ({}) so real shrink pressure \
             would apply absent the flex_shrink:0 override",
            shaped_widths[0],
            shaped_widths[1],
            PageBox::A4.width
        );

        for (&id, &shaped_width) in [a, c].iter().zip(shaped_widths.iter()) {
            let box_width = doc.nodes[id].unrounded_layout.size.width;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                box_width + 1e-3 >= shaped_width,
                "child box (width={box_width}) must not be compressed \
                 narrower than its own shaped glyph run (width={shaped_width})"
            );
        }
    }

    #[test]
    fn establish_minimal_line_boxes_resets_conflicting_flex_basis() {
        // Regression pin for the `flex_basis: auto` reset documented on
        // `establish_minimal_line_boxes`. `b` below carries an explicit
        // `flex-basis: 10px` far smaller than its own shaped content — a
        // declaration that was inert while `p` was a plain block
        // container but would become a live flex-item property (and a
        // box/glyph-desync hazard) once `p` qualifies here, absent this
        // reset.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display: block; font-size: 72px"),
        );
        let _a = doc.append_text(p, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        let b = doc.append_element(
            Some(p),
            "b",
            Style::default(),
            Some("display: inline; flex-basis: 10px"),
        );
        let c = doc.append_text(b, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[b].style.flex_basis,
            Dimension::auto(),
            "`b`'s explicit flex-basis:10px must be reset to auto once `p` \
             qualifies for minimal-line-box treatment — a non-auto \
             author flex-basis directly sets the box's main size \
             regardless of flex_shrink:0, so leaving it in place would \
             reopen the box/glyph desync hazard flex_shrink:0 exists to \
             prevent"
        );

        let shaped_width = doc.nodes[c]
            .text_layout()
            .expect("text node must have a preshaped layout")
            .width();
        let b_box_width = doc.nodes[b].unrounded_layout.size.width;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            b_box_width + 1e-3 >= shaped_width,
            "`b`'s box (width={b_box_width}) must not be compressed \
             below its shaped content (width={shaped_width}) now that \
             the conflicting flex-basis:10px is reset to auto"
        );
    }

    #[test]
    fn establish_minimal_line_boxes_resets_conflicting_justify_content_and_gap() {
        // Regression pin for the `justify_content: None` / `gap: 0`
        // resets documented on `establish_minimal_line_boxes`. `p` below
        // carries explicit `justify-content: space-between` and
        // `column-gap: 500px` — both inert while `p` was a plain block
        // container, both live main-axis-affecting flex-container
        // properties once `p` qualifies here, absent these resets. If
        // either leaked through, the two participating children would
        // end up far apart (`justify-content: space-between` alone would
        // push the second child to the far right edge, and a 500px gap
        // would separate them further still) instead of adjacent.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display: block; justify-content: space-between; column-gap: 500px"),
        );
        let a = doc.append_text(p, "AAAA");
        let c = doc.append_text(p, "CCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.justify_content, None,
            "`p`'s explicit justify-content:space-between must be reset \
             to taffy's default (None) once it qualifies for \
             minimal-line-box treatment — line boxes have no main-axis \
             item-redistribution semantics to preserve"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.gap,
            Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            },
            "`p`'s explicit column-gap:500px must be reset to 0 once it \
             qualifies for minimal-line-box treatment — line boxes have \
             no author-controllable gap semantics to preserve"
        );

        let a_loc = doc.nodes[a].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.x < a_loc.location.x + a_loc.size.width + 1.0,
            "the two children must still sit adjacent (within 1px) — \
             got a.x={} a.w={} c.x={}, i.e. neither the space-between \
             nor the 500px gap leaked through",
            a_loc.location.x,
            a_loc.size.width,
            c_loc.location.x
        );
    }

    #[test]
    fn establish_minimal_line_boxes_resets_conflicting_align_self() {
        // Regression pin for the `align_self: None` reset documented on
        // `establish_minimal_line_boxes`. `b` below carries an explicit
        // `align-self: flex-end` — inert while `p` was a plain block
        // container, a live cross-axis override once `p` qualifies here,
        // absent this reset (it would otherwise diverge from the
        // container's own `align_items: FlexStart`).
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");
        let b = doc.append_element(
            Some(p),
            "b",
            Style::default(),
            Some("display: inline; align-self: flex-end"),
        );
        let _c = doc.append_text(b, "B");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[b].style.align_self, None,
            "`b`'s explicit align-self:flex-end must be reset to None \
             (= auto) once `p` qualifies for minimal-line-box treatment, \
             so `b` falls back to `p`'s own align_items:FlexStart \
             instead of diverging from it"
        );
    }

    #[test]
    fn establish_minimal_line_boxes_br_switches_container_to_wrap_and_forces_full_basis() {
        // Unit-level pin for the "`<br>` forced break" mechanism documented
        // on `establish_minimal_line_boxes`: a qualifying container with a
        // participating `<br>` switches from `flex_wrap: NoWrap` to `Wrap`,
        // and only the `<br>` child (not its siblings) gets its flex_basis
        // forced to 100%.
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let a = doc.append_text(p, "A");
        let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let c = doc.append_text(p, "C");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        assert_eq!(doc.nodes[p].style.display, Display::Flex);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.flex_wrap,
            TaffyFlexWrap::Wrap,
            "a qualifying container with a participating <br> must enable \
             flex_wrap so taffy's own line-packing algorithm can place the \
             <br> (and anything after it) onto a new line"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[br].style.flex_basis,
            Dimension::percent(1.0),
            "<br> itself must get flex_basis:100% — the hypothetical main \
             size that never fits alongside a non-empty line, forcing it \
             (and everything after it) onto a new line"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[a].style.flex_basis,
            Dimension::auto(),
            "a plain sibling text node must keep the ordinary content-based \
             flex_basis — only <br> itself gets the 100% override"
        );
        assert_eq!(doc.nodes[c].style.flex_basis, Dimension::auto());
    }

    #[test]
    fn establish_minimal_line_boxes_display_none_br_does_not_switch_to_wrap() {
        // A `display:none` `<br>` generates no box at all (CSS Display 4
        // §2's "the element and its descendants generate no boxes"), so it
        // must not be treated as a forced break — otherwise a container
        // with no visually-effective `<br>` would still gain
        // `flex_wrap: Wrap`, silently enabling size-based wrapping
        // (`establish_minimal_line_boxes`'s Non-goals doc) for content that
        // never asked for it.
        use raikiri_style::{build_rule_tree, cascade};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let _a = doc.append_text(p, "A");
        let _br = doc.append_element(Some(p), "br", Style::default(), Some("display: none"));
        let _c = doc.append_text(p, "C");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        assert_eq!(doc.nodes[p].style.display, Display::Flex);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[p].style.flex_wrap,
            TaffyFlexWrap::NoWrap,
            "a display:none <br> must not count as a forced break"
        );
    }

    #[test]
    fn establish_minimal_line_boxes_br_forces_second_line_end_to_end() {
        // End-to-end pin (full `layout_single_page` pipeline, matching
        // `establish_minimal_line_boxes_lays_out_children_side_by_side_not_stacked`'s
        // style) for the concrete geometry `<br>` must produce: the content
        // after `<br>` lands on a lower line than the content before it,
        // and the `<br>` itself contributes no visible height — the line
        // it alone occupies (taffy's line-packing algorithm assigns it one
        // because its flex_basis:100% never fits next to prior content)
        // must have cross size 0, so it does not introduce a phantom blank
        // line between the two real ones.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let a = doc.append_text(p, "AAAA");
        let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let c = doc.append_text(p, "CCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[a].unrounded_layout;
        let br_loc = doc.nodes[br].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;
        let p_loc = doc.nodes[p].unrounded_layout;

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
            "the text after <br> must start at or below the first line's \
             bottom edge (forced break into a second line), got \
             a.y={} a.h={} c.y={}",
            a_loc.location.y,
            a_loc.size.height,
            c_loc.location.y
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            br_loc.size.height, 0.0,
            "<br> is a childless leaf with no text layout, so its own \
             measured cross size must be 0 — the load-bearing fact that \
             keeps the line it alone occupies from adding visible height"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (p_loc.size.height - (a_loc.size.height + c_loc.size.height)).abs() < 1e-3,
            "the container's total height must equal exactly the sum of \
             the two real lines' heights — no phantom third (blank) line \
             from the line <br> alone occupies, got p.h={} a.h={} c.h={}",
            p_loc.size.height,
            a_loc.size.height,
            c_loc.size.height
        );
    }

    #[test]
    fn establish_minimal_line_boxes_consecutive_br_adds_no_blank_line_either() {
        // Same mechanism as the leading-<br> Non-goal
        // (`establish_minimal_line_boxes_leading_br_does_not_add_leading_blank_line`),
        // checked for the *second* <br> in a `<br><br>` run instead of the
        // first: after the first <br> takes the whole of its own (now
        // empty) line, the second <br> is checked against a line with 0
        // remaining space — still its line's first item (the first <br>
        // already moved on), so the same "an empty line accepts its first
        // item" exception applies to it too, and it likewise measures
        // 0-height. Confirms the doc's claim explicitly, rather than
        // leaving it as an un-pinned assertion about a case distinct from
        // the leading-<br> one.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let a = doc.append_text(p, "AAAA");
        let br1 = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let br2 = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let b = doc.append_text(p, "BBBB");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[a].unrounded_layout;
        let br1_loc = doc.nodes[br1].unrounded_layout;
        let br2_loc = doc.nodes[br2].unrounded_layout;
        let b_loc = doc.nodes[b].unrounded_layout;
        let p_loc = doc.nodes[p].unrounded_layout;

        assert_eq!(br1_loc.size.height, 0.0);
        assert_eq!(br2_loc.size.height, 0.0);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            b_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
            "got a.y={} a.h={} b.y={}",
            a_loc.location.y,
            a_loc.size.height,
            b_loc.location.y
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (p_loc.size.height - (a_loc.size.height + b_loc.size.height)).abs() < 1e-3,
            "a consecutive <br><br> must not add a visible blank line \
             between AAAA and BBBB — got p.h={} a.h={} b.h={}",
            p_loc.size.height,
            a_loc.size.height,
            b_loc.size.height
        );
    }

    #[test]
    fn establish_minimal_line_boxes_br_full_basis_resolves_against_narrow_container_width() {
        // `<br>`'s flex_basis:100% (`establish_minimal_line_boxes`'s doc)
        // must resolve against the *qualifying container's own* used main
        // size, not its containing block's — otherwise a narrower
        // container (explicit `width`, rather than filling its parent)
        // would give <br> a too-wide box. Pins that by giving the
        // container an explicit width much narrower than the page and
        // checking <br>'s own box stays within it.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display: block; width: 100px"),
        );
        let a = doc.append_text(p, "AAAA");
        let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let c = doc.append_text(p, "CCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let p_loc = doc.nodes[p].unrounded_layout;
        let br_loc = doc.nodes[br].unrounded_layout;
        let a_loc = doc.nodes[a].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (p_loc.size.width - 100.0).abs() < 1e-3,
            "fixture precondition: explicit width:100px must actually take \
             effect, got p.w={}",
            p_loc.size.width
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            br_loc.size.width <= p_loc.size.width + 1e-3,
            "<br>'s flex_basis:100% must resolve against the qualifying \
             container's own (narrow) used width, not some wider \
             containing block — got br.w={} p.w={}",
            br_loc.size.width,
            p_loc.size.width
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
            "the forced break must still occur on a narrow container, got \
             a.y={} a.h={} c.y={}",
            a_loc.location.y,
            a_loc.size.height,
            c_loc.location.y
        );
    }

    #[test]
    fn establish_minimal_line_boxes_br_line_stays_zero_height_under_explicit_tall_container_height()
    {
        // Regression pin for a property-leak this pass must guard against:
        // `bridge_size` unconditionally copies an author `height` into
        // `style.size.height`, and `bridge_alignment` unconditionally
        // copies an author `align-content` into `style.align_content`
        // (`None` when unauthored). Once this container becomes a
        // multi-line flex container (this pass's "`<br>` forced break"),
        // an explicit `height` taller than the natural content height
        // leaves leftover cross space that taffy's own default
        // (`align_content: Stretch` when unset) would distribute across
        // ALL flex lines — including the zero-height line the `<br>`
        // alone occupies — growing it above 0 and inserting a visible gap
        // between the two real text lines. This container's own
        // `align_content` must be reset (mirroring the `align_items`
        // reset a few lines up) so that leftover space is not distributed
        // onto lines at all.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(
            Some(body),
            "p",
            Style::default(),
            Some("display: block; height: 200px"),
        );
        let a = doc.append_text(p, "AAAA");
        let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let c = doc.append_text(p, "CCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let p_loc = doc.nodes[p].unrounded_layout;
        let a_loc = doc.nodes[a].unrounded_layout;
        let br_loc = doc.nodes[br].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (p_loc.size.height - 200.0).abs() < 1e-3,
            "fixture precondition: explicit height:200px must actually \
             take effect (and exceed the two text lines' natural height), \
             got p.h={}",
            p_loc.size.height
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            br_loc.size.height, 0.0,
            "the <br>'s own line must stay at 0 height even when the \
             container's explicit height leaves leftover cross space — \
             that space must not stretch onto any line, got br.h={}",
            br_loc.size.height
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (c_loc.location.y - (a_loc.location.y + a_loc.size.height)).abs() < 1e-3,
            "no visible gap between the two real lines: the second \
             line's top must sit exactly at the first line's bottom \
             edge, got a.y={} a.h={} c.y={}",
            a_loc.location.y,
            a_loc.size.height,
            c_loc.location.y
        );
    }

    #[test]
    fn establish_minimal_line_boxes_leading_br_does_not_add_leading_blank_line() {
        // Documents (rather than merely asserting) the accepted Non-goal on
        // `establish_minimal_line_boxes`: a `<br>` with no preceding
        // participating content on its line is the *first* item taffy
        // considers for that line, so the flex line-packing exception
        // ("an already-empty line accepts its first item even if it
        // overflows") places it there rather than moving it — and since
        // `<br>` measures 0x0 (no children, no text layout), that first
        // line has no visible height. So a leading `<br>` here does not
        // reproduce the leading blank line real browsers render for it;
        // this pins the current (accepted) behavior instead.
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some("display: block"));
        let br = doc.append_element(Some(p), "br", Style::default(), None::<&str>);
        let a = doc.append_text(p, "AAAA");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let br_loc = doc.nodes[br].unrounded_layout;
        let a_loc = doc.nodes[a].unrounded_layout;
        let p_loc = doc.nodes[p].unrounded_layout;

        assert_eq!(br_loc.location.y, 0.0);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (p_loc.size.height - a_loc.size.height).abs() < 1e-3,
            "no leading blank line: container height must equal just the \
             text line's height, got p.h={} a.h={}",
            p_loc.size.height,
            a_loc.size.height
        );
    }

    #[test]
    fn establish_minimal_line_boxes_br_forces_second_line_on_inline_block_container() {
        // Same forced-break geometry as
        // `establish_minimal_line_boxes_br_forces_second_line_end_to_end`,
        // but on a qualifying `inline-block` container rather than `block`
        // — pins that the mechanism does not depend on which of the two
        // qualifying display values establishes the line box (this crate's
        // block layout gives both a definite available main size from
        // their own containing block; see `bridge_display`'s doc, neither
        // display value gets shrink-to-fit sizing here).
        use parley::FontContext;
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let ib = doc.append_element(
            Some(body),
            "span",
            Style::default(),
            Some("display:inline-block"),
        );
        let a = doc.append_text(ib, "AAAA");
        let br = doc.append_element(Some(ib), "br", Style::default(), None::<&str>);
        let c = doc.append_text(ib, "CCCCCCCCCCCCCCCCCCCC");

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[a].unrounded_layout;
        let br_loc = doc.nodes[br].unrounded_layout;
        let c_loc = doc.nodes[c].unrounded_layout;
        let ib_loc = doc.nodes[ib].unrounded_layout;

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            c_loc.location.y >= a_loc.location.y + a_loc.size.height - 1e-3,
            "got a.y={} a.h={} c.y={}",
            a_loc.location.y,
            a_loc.size.height,
            c_loc.location.y
        );
        assert_eq!(br_loc.size.height, 0.0);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (ib_loc.size.height - (a_loc.size.height + c_loc.size.height)).abs() < 1e-3,
            "got ib.h={} a.h={} c.h={}",
            ib_loc.size.height,
            a_loc.size.height,
            c_loc.size.height
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_margin_to_taffy() {
        // bridge_margin が
        // Sides<ComputedLengthPercentageOrAuto> を taffy::Rect<LengthPercentageAuto>
        // に translate することを確認する regression pin。bridge の 3 分岐
        // (Px / Percent / Auto) をそれぞれ 1 case で covering。
        //
        // Case 4 の `pt` は bridge の分岐ではなくなった
        // (cascade の phase 3 が px に絶対化する) が、end-to-end の期待値は
        // 変わらないので test は残す。
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
        //   **この変換は bridge ではなく cascade の phase 3
        //   (`raikiri_style::resolve_length_percentage_or_auto`) が行う**。
        //   bridge に届く時点で既に px。本 case は
        //   end-to-end の値を pin する。
        //   f32 bit-identical assert のため右辺を expression のまま書く
        //   (`13.333` literal は round-trip で drift する。この式は
        //   `resolve::pt_to_px` 本体と同じ `v * 4.0 / 3.0` の評価順を使う —
        //   f32 は結合則を満たさないため簡約すると bit が変わる。詳細は
        //   `resolve::pt_to_px` の doc 参照)。
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
        // bridge_padding が
        // Sides<ComputedLengthPercentage> を taffy::Rect<LengthPercentage> に
        // translate することを確認する regression pin。padding は margin と違い
        // `auto` を持たない (<length-percentage `[0,∞]`>) ため bridge は **2 arm**
        // (Px / Percent) で網羅する。
        //
        // Case 3 の `pt` は **bridge の分岐ではなくなった**
        // (cascade の phase 3 が px に絶対化する) が、end-to-end の期待値は
        // 変わらないので test は残す。
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
        //   **変換の所在は cascade の phase 3** (`resolve_length_percentage`) で
        //   bridge ではない。
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
        // bridge_size (width component)
        // が cv.width: ComputedLengthPercentageOrAuto を taffy::Style::size.width:
        // Dimension に translate することを pin する。bridge の 3 分岐
        // (Px / Percent / Auto) をそれぞれ 1 case で covering
        // (`pt` は cascade の phase 3 で px 化される)。
        //
        // Test 戦略: fixture は **非 body element** (この場合 `<p>`) を使う —
        // `<body>` は後段 `apply_page_box_to_body` で clobber されるため本 bridge
        // の効果は observable でない (別 test `apply_page_box_clobbers_body_width_from_bridge`
        // で clobber 挙動を pin)。inline style 経由なので raikiri-style の
        // parse_width path + ComputedLengthPercentageOrAuto encoding も同時に
        // regression pin。
        //
        // 本 test は width 軸に絞る — height 軸は sibling test
        // `apply_computed_to_style_bridges_height_to_taffy`
        // が同 fixture pattern で LengthOrAuto → Dimension bridge を pin する。
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

        // Case 2: `width: auto` → Dimension::auto()
        //   (ComputedLengthPercentageOrAuto::Auto arm)。
        assert_eq!(width_for("width: auto"), Dimension::auto());

        // Case 3: `width: 50%` → Dimension::percent(0.5)。CSS spec の authored
        //   0-100 → taffy fraction 0.0-1.0 の div-by-100 policy を pin。
        assert_eq!(width_for("width: 50%"), Dimension::percent(0.5));

        // Case 4: `width: 20pt` → Dimension::length(20 * 4/3) = length(26.666...)。
        //   CSS Values 4 §6.2 の `1pt = 4/3 px` (1pt=1/72in、1in=96px → 96/72=4/3)。
        //   **変換の所在は cascade の phase 3** で bridge ではない。
        //   f32 bit-identical assert のため右辺を expression で書く。
        assert_eq!(
            width_for("width: 20pt"),
            Dimension::length(20.0 * 4.0 / 3.0)
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_height_to_taffy() {
        // bridge_size の height 側
        // 拡張。cv.height: ComputedLengthPercentageOrAuto を
        // taffy::Style::size.height: Dimension に translate することを pin する。
        // sibling test `apply_computed_to_style_bridges_width_to_taffy` と
        // 対を成し、struct literal 化 (Size { width, height } の 1 発
        // assign) で height 側の 3 分岐 (Px / Percent / Auto) が意図通り
        // 書き込まれるか確認する。
        //
        // Test 戦略: fixture は **非 body element** (`<p>`) を使う — `<body>` は
        // 後段 `apply_page_box_to_body` で height も clobber されるため本 bridge
        // の効果は body 上で observable でない。inline style 経由で raikiri-style
        // の parse_height path + ComputedLengthPercentageOrAuto encoding も同時に
        // regression pin。
        //
        // Pt case は sibling width test が同じ
        // computed_length_percentage_or_auto_to_taffy_dimension policy を
        // pin しているため redundant (かつ pt → px 変換は
        // cascade の phase 3 の責務)。ここでは height
        // 特有の 3 arm (auto default 保持、`Px` 通路、`Percent` 通路) に絞る。
        use raikiri_style::{build_rule_tree, cascade};

        fn height_for(inline: &str) -> Dimension {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            // 非 body element (p) に inline を載せる。apply_page_box_to_body は
            // body だけを触るため、p の style.size は bridge 実行後そのまま観測可能。
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            apply_computed_to_style(&mut doc, &cr);
            doc.nodes[p].style.size.height
        }

        // Case 1: `height: 100px` → Dimension::length(100.0) (Px identity)。
        assert_eq!(height_for("height: 100px"), Dimension::length(100.0));

        // Case 2: `height: auto` → Dimension::auto()
        //   (ComputedLengthPercentageOrAuto::Auto arm)。
        //   CSS Sizing 3 §3.1.1 initial `height: auto` の identity round-trip pin。
        assert_eq!(height_for("height: auto"), Dimension::auto());

        // Case 3: `height: 50%` → Dimension::percent(0.5)。CSS spec の authored
        //   0-100 → taffy fraction 0.0-1.0 の div-by-100 policy を pin。
        assert_eq!(height_for("height: 50%"), Dimension::percent(0.5));
    }

    #[test]
    fn apply_computed_to_style_bridges_border_to_taffy() {
        // bridge_border が
        // Sides<ComputedBorder> を taffy::Rect<LengthPercentage> に translate
        // することを確認する regression pin。
        //
        // spec correctness gate: border-style が `none` / `hidden` の場合、
        // specified border-width にかかわらず width は 0 でなければならない。
        // **gate の所在は本 bridge ではなく上流の
        // `raikiri_style::resolve_border` (computed 層)** — CSS Backgrounds 3
        // §3.3 "Line Thickness: the border-width properties"
        // <https://www.w3.org/TR/css-backgrounds-3/#border-width> の propdef が
        // "Computed value: absolute length, snapped as a border width; zero if
        // the border style is none or hidden" と規定するため (TR 版 — ED は
        // CSSWG Issue 11494 で resolved-value 効果へ移動済)
        // (used 層から computed 層へ移動済で、bridge 側の
        // `used_border_width` helper は削除済)。§3.2 "Line Patterns: the
        // border-style properties"
        // <https://www.w3.org/TR/css-backgrounds-3/#border-style> の `none` も
        // "No border. Color and width are ignored (i.e., the border has width
        // 0)." と整合する。
        //
        // 本 test は依然 gating の **end-to-end** pin である (gate が上流に
        // 移っても `5px none red` の 5px が taffy に leak しないことを保証する
        // のが目的)。§ 番号と引用は spec の `data-level` / 本文実測に基づく —
        // 以前あった "§5.2 The used values of the corresponding border-*-width
        // become 0." は css-backgrounds-3 に存在しない文だったので差し替えた。
        //
        // Test 戦略: `border: <w> <s> <c>` 4-side shorthand と longhand の
        // 両方を使い、shorthand 展開 → per-side cascade → bridge_border の
        // pipeline を end-to-end で pin する (単一 side shorthand
        // `border-top: ...` は現時点で parser 未対応、
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

        // Case 2 (spec correctness): `border: 5px none red` shorthand
        //   → 4 side 全て width=5, style=None で cascade。§3.2 の `none` と
        //   §3.3 propdef (TR 版、逐語引用は冒頭 block) による style-gating で
        //   computed width = 0 → 4 side 全て length(0.0)。gating が壊れると 5.0
        //   が leak するので、この case が canary。
        assert_eq!(
            border_for("border: 5px none red"),
            Rect {
                top: LengthPercentage::length(0.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );

        // Case 3 (spec correctness): `border: 5px hidden red`
        //   shorthand → computed width = 0。直接の根拠は §3.3 propdef (TR 版) が
        //   `none` と並べて `hidden` を名指ししていること。§3.2 の `hidden` は
        //   "Same as none, but has different behavior in the border conflict
        //   resolution rules for border-collapsed tables `CSS2`." であり、`none`
        //   との差は border-collapsed table の conflict resolution だけ。
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
        //   3.0 px (property.rs `parse_border_width_side` 参照)。§3.3 は "The
        //   thin, medium, and thick keywords are equivalent to 1px, 3px, and
        //   5px, respectively." と**固定値を規定**する。CSS2.1 §8.5.1
        //   <https://www.w3.org/TR/CSS21/box.html#border-width-properties>
        //   の "The interpretation of the first three values depends on the
        //   user agent." から性格が変わっている点に注意。他 3 side は Case 4
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

    /// `ComputedBorder::width` / `::style` を `pub`
    /// field から `pub(crate)` + read-only accessor へ narrow した動機になった
    /// invariant の end-to-end pin。
    ///
    /// `border-top-width` だけを宣言し `border-top-style` を宣言しない
    /// (= 未宣言側の computed style は initial `none`、CSS Backgrounds 3
    /// §3.2) 素朴な入力で、declared width が bridge を通って taffy に **0** と
    /// して届くことを確認する。narrowing 前はこの gate を consumer が
    /// `ComputedValues::initial()` 等で得た `ComputedBorder` の
    /// `.style = BorderStyle::None` 直接書き換えで迂回でき、`.width` が非 0 の
    /// まま taffy に leak し得た (`used_border_width` bridge 削除後)。narrowing は
    /// `width` / `style` に限り crate 外
    /// からのその書き換え経路を塞ぐ — `color` は pub のまま、
    /// `cv.border.top = cv.border.left` のような side 単位の丸ごと代入も
    /// 引き続き可能で、いずれも本 invariant を破らない。
    ///
    /// **本 test は「narrowing 前の値で確認できる 1 入力が正しく gate される」
    /// ことの pin であり、「crate 外からこの invariant を破る経路が存在しない」
    /// ことを本 test 自身が総当たりで示すものではない。** ただし後者自体は
    /// 現状すでに **型の visibility 境界で構造的に防がれている** — `width` /
    /// `style` は `pub(crate)`、公開 API は値渡し read-only accessor (`width()` /
    /// `style()`) のみで setter / builder / ctor が無いため、crate 外の
    /// safe code がこの 2 field を書き換える経路はコンパイル時に存在しない。
    ///
    /// **未解決なのは別の軸 — regression 検知**: 将来誰かが `width` /
    /// `style` を `pub(crate)` から `pub` に戻す (= 上記の型保証そのものを
    /// 撤回する) 変更をしても、それを検知して落ちる test が現状無い。
    /// 他の 3 field (`Declaration::value` 等) には同じ形の regression を
    /// 検知する compile-fail harness がすでに追加され既に main に merge 済みだが、
    /// `ComputedBorder::width` / `::style` への横展開はまだ行われていない。
    #[test]
    fn border_width_alone_without_declared_style_reaches_taffy_as_zero() {
        use raikiri_style::{build_rule_tree, cascade};
        use taffy::{LengthPercentage, Rect};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            // `border-style` は一切宣言しない — 4 side とも computed style は
            // initial `none` (CSS Backgrounds 3 §3.2 "Inherited: no")。
            Some("border-top-width: 5px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);

        // border-style unset (initial `none`) must gate width to 0 all the way to taffy.
        assert_eq!(
            doc.nodes[body].style.border,
            Rect {
                top: LengthPercentage::length(0.0),
                right: LengthPercentage::length(0.0),
                bottom: LengthPercentage::length(0.0),
                left: LengthPercentage::length(0.0),
            }
        );
    }

    #[test]
    fn font_relative_lengths_reach_taffy_as_real_pixels() {
        // 以前の bridge は specified 層の `Length` を受けており、
        // `Length::Em(_) | Length::Rem(_) => length(0.0)` で font-relative unit
        // を **黙って 0px に潰していた** (fail-quiet)。cascade が phase 2 /
        // phase 3 で絶対化するようになったので、実 px が taffy に届く。
        //
        // この test は「0.0 に潰れる」regression の canary である — 期待値は
        // すべて font-size から計算した非ゼロ値。
        use raikiri_style::{build_rule_tree, cascade};
        use taffy::{Dimension, LengthPercentage, LengthPercentageAuto, Rect};

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("font-size: 20px"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            // font-size は inherit で 20px。em は自 node の 20px、rem は root の
            // 20px 基準。
            Some(
                "padding: 2em; margin: 1.5rem; width: 3em; \
                 border-top-width: 0.5em; border-top-style: solid",
            ),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        let style = &doc.nodes[body].style;

        // 2em × 20px = 40px (従来は 0.0)。
        assert_eq!(
            style.padding,
            Rect {
                top: LengthPercentage::length(40.0),
                right: LengthPercentage::length(40.0),
                bottom: LengthPercentage::length(40.0),
                left: LengthPercentage::length(40.0),
            }
        );
        // 1.5rem × 20px (root font-size) = 30px (従来は 0.0)。
        assert_eq!(
            style.margin,
            Rect {
                top: LengthPercentageAuto::length(30.0),
                right: LengthPercentageAuto::length(30.0),
                bottom: LengthPercentageAuto::length(30.0),
                left: LengthPercentageAuto::length(30.0),
            }
        );
        // 3em × 20px = 60px (従来は 0.0)。
        assert_eq!(style.size.width, Dimension::length(60.0));
        // 0.5em × 20px = 10px、style: solid なので gating も通り抜ける
        // (従来は 0.0)。
        assert_eq!(style.border.top, LengthPercentage::length(10.0));
    }

    #[test]
    fn apply_computed_to_style_bridges_box_sizing_to_taffy() {
        // bridge_box_sizing が
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
        // PageBox
        // 妥協の regression pin — `<body style="width: 100px">` に対して
        //   Step 1 (`apply_computed_to_style`) → bridge_size が body.style.size.width
        //       を length(100.0) に write
        //   Step 4 (`apply_page_box_to_body`) → PageBox.width で clobber
        // の順で走ると、最終 body.style.size.width は PageBox.width (author 値
        // ではない) になる。将来 @page per-page PageBox に refactor するまで
        // この clobber 挙動を意図的に保つ (現行実装での妥協) — silent regression 検出用。
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
            "apply_page_box_to_body must clobber author width with PageBox.width (現行実装での妥協)"
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
        // 相当 (parser の代わりに手動構築、raikiri-html 統合は将来 umbrella が担当)
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
        // layout_single_page 経由で display bridge が active
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
    fn layout_single_page_bridges_display_flex_to_taffy_flexbox() {
        // display bridge が Display::Flex を実際に taffy::compute_flexbox_layout
        // へ届けることを **geometry** で確認する — style.display の値を
        // asserting するだけでは bridge が繋がったことしか示さず、taffy_impl.rs
        // の compute_child_layout dispatch が実際に flex を起動していることの
        // 証明にはならない。CSS Flexbox Level 1 の initial value
        // (`flex-direction: row`、`flex-wrap: nowrap`) 通りなら、明示 width の
        // 2 child は主軸 (x) 方向に並び、交差軸 (y) は揃う。
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container =
            doc.append_element(Some(body), "div", Style::default(), Some("display:flex"));
        let child_a = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let child_b = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[flex_container].style.display,
            Display::Flex,
            "bridge_display must map DisplayValue::Flex to taffy::Display::Flex"
        );
        let a_loc = doc.nodes[child_a].unrounded_layout.location;
        let b_loc = doc.nodes[child_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.y - b_loc.y).abs() < 0.5,
            "row-direction flex items must share the same cross-axis (y) offset, got a.y={}, b.y={}",
            a_loc.y,
            b_loc.y
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
            "second flex item should sit 100px (first item's width) further along the main axis (x), got a.x={}, b.x={}",
            a_loc.x,
            b_loc.x
        );
    }

    #[test]
    fn bridge_flex_maps_every_flex_direction_and_flex_wrap_keyword() {
        // `bridge_flex`'s `flex_direction`/`flex_wrap` match arms — the
        // sibling `flex_direction_column_stacks_children_vertically` /
        // `gap_adds_space_between_flex_items` tests only exercise `Row`
        // (default) and `Column` end-to-end through real layout geometry;
        // this test pins the remaining keyword→taffy-constant mappings
        // directly, since geometry alone can't discriminate e.g.
        // `RowReverse` from `Row` without a multi-child fixture per
        // variant.
        for (direction, expected) in [
            (FlexDirectionValue::Row, TaffyFlexDirection::Row),
            (
                FlexDirectionValue::RowReverse,
                TaffyFlexDirection::RowReverse,
            ),
            (FlexDirectionValue::Column, TaffyFlexDirection::Column),
            (
                FlexDirectionValue::ColumnReverse,
                TaffyFlexDirection::ColumnReverse,
            ),
        ] {
            let mut cv = ComputedValues::initial();
            cv.flex_direction = direction;
            let mut style = Style::default();
            let mut diag = Vec::new();
            bridge_flex(&mut style, &cv, &mut diag);
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                style.flex_direction, expected,
                "flex-direction: {direction:?}"
            );
        }

        for (wrap, expected) in [
            (FlexWrapValue::NoWrap, TaffyFlexWrap::NoWrap),
            (FlexWrapValue::Wrap, TaffyFlexWrap::Wrap),
            (FlexWrapValue::WrapReverse, TaffyFlexWrap::WrapReverse),
        ] {
            let mut cv = ComputedValues::initial();
            cv.flex_wrap = wrap;
            let mut style = Style::default();
            let mut diag = Vec::new();
            bridge_flex(&mut style, &cv, &mut diag);
            assert_eq!(style.flex_wrap, expected, "flex-wrap: {wrap:?}");
        }
    }

    #[test]
    fn bridge_flex_absolutizes_flex_basis_px_and_percent() {
        // `bridge_flex`'s `ComputedFlexBasis::Px`/`Percent` arms — no
        // existing test sets an explicit `<length-percentage>` flex-basis
        // (`flex_grow_absorbs_free_space` below leaves it at the `auto`
        // initial value), so these two reachable arms had no direct
        // coverage.
        let mut cv = ComputedValues::initial();
        cv.flex_basis = ComputedFlexBasis::Px(40.0);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_flex(&mut style, &cv, &mut diag);
        assert_eq!(style.flex_basis, Dimension::length(40.0));

        // `ComputedFlexBasis::Percent` holds the authored number (`50.0`
        // for `50%`, not a `0.0..=1.0` fraction) — the bridge divides by
        // 100 before handing it to taffy, same convention as
        // `computed_length_percentage_or_auto_to_taffy_dimension`'s other
        // callers (`width`/`height`/`margin`).
        let mut cv = ComputedValues::initial();
        cv.flex_basis = ComputedFlexBasis::Percent(50.0);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_flex(&mut style, &cv, &mut diag);
        assert_eq!(style.flex_basis, Dimension::percent(0.5));
    }

    #[test]
    fn bridge_flex_maps_intrinsic_basis_keywords_to_taffy_dimensions() {
        // taffy 0.14 migration: `min-content` / `max-content` /
        // `fit-content` / `content` map to taffy's own intrinsic
        // `Dimension` variants (no `auto` collapse anymore).
        for (value, expected) in [
            (ComputedFlexBasis::MinContent, Dimension::min_content()),
            (ComputedFlexBasis::MaxContent, Dimension::max_content()),
            (ComputedFlexBasis::FitContent, Dimension::fit_content()),
            (ComputedFlexBasis::Content, Dimension::content()),
        ] {
            let mut cv = ComputedValues::initial();
            cv.flex_basis = value;
            let mut style = Style::default();
            let mut diag = Vec::new();
            bridge_flex(&mut style, &cv, &mut diag);
            assert_eq!(style.flex_basis, expected);
            assert!(diag.is_empty());
        }
    }

    #[test]
    fn content_alignment_to_taffy_maps_every_keyword() {
        // `content_alignment_to_taffy` backs both `justify-content` and
        // `align-content` (`bridge_alignment`) — pin every keyword→taffy
        // constant mapping directly, since only `Center`-ish geometry is
        // exercised end-to-end elsewhere.
        for (value, expected) in [
            (ContentAlignmentValue::Normal, None),
            (
                ContentAlignmentValue::Stretch,
                Some(TaffyAlignContent::STRETCH),
            ),
            (
                ContentAlignmentValue::SpaceBetween,
                Some(TaffyAlignContent::SPACE_BETWEEN),
            ),
            (
                ContentAlignmentValue::SpaceEvenly,
                Some(TaffyAlignContent::SPACE_EVENLY),
            ),
            (
                ContentAlignmentValue::SpaceAround,
                Some(TaffyAlignContent::SPACE_AROUND),
            ),
            (
                ContentAlignmentValue::Center,
                Some(TaffyAlignContent::CENTER),
            ),
            (ContentAlignmentValue::Start, Some(TaffyAlignContent::START)),
            (ContentAlignmentValue::End, Some(TaffyAlignContent::END)),
            (
                ContentAlignmentValue::FlexStart,
                Some(TaffyAlignContent::FLEX_START),
            ),
            (
                ContentAlignmentValue::FlexEnd,
                Some(TaffyAlignContent::FLEX_END),
            ),
        ] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                content_alignment_to_taffy(value),
                expected,
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn self_alignment_to_taffy_maps_every_keyword() {
        // `self_alignment_to_taffy` backs `align-items` and the
        // non-`auto` branch of `align-self` (`bridge_alignment`) — pin
        // every keyword→taffy constant mapping directly.
        for (value, expected) in [
            (SelfAlignmentValue::Normal, None),
            (SelfAlignmentValue::Stretch, Some(TaffyAlignItems::STRETCH)),
            (SelfAlignmentValue::Center, Some(TaffyAlignItems::CENTER)),
            (SelfAlignmentValue::Start, Some(TaffyAlignItems::START)),
            (SelfAlignmentValue::End, Some(TaffyAlignItems::END)),
            (
                SelfAlignmentValue::FlexStart,
                Some(TaffyAlignItems::FLEX_START),
            ),
            (SelfAlignmentValue::FlexEnd, Some(TaffyAlignItems::FLEX_END)),
            (
                SelfAlignmentValue::Baseline,
                Some(TaffyAlignItems::BASELINE),
            ),
        ] {
            assert_eq!(self_alignment_to_taffy(value), expected, "value: {value:?}");
        }
    }

    #[test]
    fn bridge_alignment_align_self_auto_maps_to_none() {
        // `bridge_alignment`'s `AlignSelfValue::Auto` arm — the `None`
        // mapping (CSS Box Alignment 3 §6.2: falls back to the parent's
        // `align-items`, delegated to taffy) has no other coverage.
        let mut cv = ComputedValues::initial();
        cv.align_self = AlignSelfValue::Auto;
        let mut style = Style::default();
        bridge_alignment(&mut style, &cv);
        assert_eq!(style.align_self, None);
    }

    #[test]
    fn bridge_alignment_align_self_value_delegates_to_self_alignment_to_taffy() {
        // `bridge_alignment`'s `AlignSelfValue::Value(v)` arm — distinct
        // from the sibling test above, which only pins the `Auto` arm.
        // `self_alignment_to_taffy` itself is already pinned directly by
        // `self_alignment_to_taffy_maps_every_keyword`, but that doesn't
        // exercise this match arm's delegation to it.
        let mut cv = ComputedValues::initial();
        cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Center);
        let mut style = Style::default();
        bridge_alignment(&mut style, &cv);
        assert_eq!(style.align_self, Some(TaffyAlignItems::CENTER));
    }

    #[test]
    fn bridge_alignment_align_self_normal_maps_to_stretch_not_none() {
        // Regression test (§8.3 review finding): `align-self: normal` must
        // NOT collapse to the same `None` mapping as `align-self: auto`.
        // `auto` computes to the parent's `align-items` value (CSS Box
        // Alignment 3 §8.3), which taffy's `align_self: None` already
        // implements by inheriting the container's `align_items`. But
        // `normal` independently behaves as `stretch` in flex layout
        // regardless of the parent's `align-items` — mapping it to `None`
        // would incorrectly make it inherit the parent's value like `auto`
        // does. See the end-to-end behavioral pin below.
        let mut cv = ComputedValues::initial();
        cv.align_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
        let mut style = Style::default();
        bridge_alignment(&mut style, &cv);
        assert_eq!(style.align_self, Some(TaffyAlignItems::STRETCH));
    }

    #[test]
    fn align_self_normal_stretches_even_when_parent_align_items_is_center() {
        // End-to-end pin of the §8.3 review finding: with the container's
        // `align-items: center`, a child with `align-self: auto` would
        // center (inheriting the parent's value per spec), but a child
        // with `align-self: normal` must independently stretch to fill the
        // cross axis — the two must NOT produce the same layout, even
        // though `bridge_alignment` maps both through `Option<AlignItems>`.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;height:100px;align-items:center"),
        );
        let normal_child = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:50px;align-self:normal"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let child_height = doc.nodes[normal_child].unrounded_layout.size.height;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (child_height - 100.0).abs() < 0.5,
            "align-self:normal must stretch to fill the 100px container cross-size regardless of the parent's align-items:center, got height={child_height}"
        );
    }

    #[test]
    fn flex_direction_column_stacks_children_vertically() {
        // `bridge_flex`'s `flex_direction` field must actually reach
        // taffy's `compute_flexbox_layout` — asserting `style.flex_direction
        // == Column` alone would only prove the assignment, not the wiring,
        // since reading the field back off the taffy `Style` cannot
        // distinguish "assigned" from "used by the layout algorithm". With
        // `flex-direction: column`, the main axis flips to the block (y)
        // axis: 2 children with an explicit height must land at the same
        // x offset with y offsets 20px (the first child's height) apart.
        //
        // Also carries `gap:10px 30px` (row-gap column-gap) to independently
        // pin `row-gap → taffy::Style::gap.height`, which the sibling
        // `gap_adds_space_between_flex_items` test cannot exercise — that
        // test's default row-direction container only puts `column-gap` on
        // the main axis. Here, `flex-direction: column` makes `row-gap` the
        // *main*-axis gap (`Size::main()` for a column container resolves to
        // `height`, per `bridge_gap`'s `width: column_gap, height: row_gap`
        // mapping), so the two children's y-offset becomes 20px (first
        // child's height) + 10px (row-gap) = 30px — discriminating a dropped
        // row-gap (would stay at 20px) or a transposed bridge (would become
        // 20px + 30px = 50px) from the correct wiring.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;flex-direction:column;gap:10px 30px"),
        );
        let child_a = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let child_b = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[child_a].unrounded_layout.location;
        let b_loc = doc.nodes[child_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.x - b_loc.x).abs() < 0.5,
            "column-direction flex items must share the same cross-axis (x) offset, got a.x={}, b.x={}",
            a_loc.x,
            b_loc.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.y - a_loc.y - 30.0).abs() < 0.5,
            "second flex item should sit 30px (20px first item's height + 10px row-gap) further along the main axis (y), got a.y={}, b.y={}",
            a_loc.y,
            b_loc.y
        );
    }

    #[test]
    fn flex_grow_absorbs_free_space() {
        // `bridge_flex`'s `flex_grow` field reaching taffy's flexible-length
        // resolution algorithm (§9.7 "Resolving Flexible Lengths") — a
        // growable child must widen past its own basis to consume the free
        // space in the container, while a non-growable sibling stays at its
        // basis.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;width:500px"),
        );
        let grower = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px;flex-grow:1"),
        );
        let fixed = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let grower_width = doc.nodes[grower].unrounded_layout.size.width;
        let fixed_width = doc.nodes[fixed].unrounded_layout.size.width;
        // Container is 500px, fixed sibling stays 100px, so the grower must
        // absorb the remaining 400px free space (500 - 100 = 400).
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (grower_width - 400.0).abs() < 0.5,
            "flex-grow:1 item should absorb the container's free space, got width={grower_width}"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (fixed_width - 100.0).abs() < 0.5,
            "non-growing sibling should stay at its flex-basis width, got width={fixed_width}"
        );
    }

    #[test]
    fn bridge_float_maps_every_float_and_clear_keyword() {
        // `bridge_float`'s enum match arms — direct pin, same rationale as
        // `bridge_flex_maps_every_flex_direction_and_flex_wrap_keyword`:
        // geometry-based tests below only exercise `Float::Left` /
        // `Clear::Left`, so this pins the remaining keyword→taffy-constant
        // mappings that a geometry fixture can't discriminate from each
        // other without a dedicated fixture per variant.
        for (float, expected) in [
            (FloatValue::None, TaffyFloat::None),
            (FloatValue::Left, TaffyFloat::Left),
            (FloatValue::Right, TaffyFloat::Right),
        ] {
            let mut cv = ComputedValues::initial();
            cv.float = float;
            let mut style = Style::default();
            bridge_float(&mut style, &cv);
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(style.float, expected, "float: {float:?}");
        }

        for (clear, expected) in [
            (ClearValue::None, TaffyClear::None),
            (ClearValue::Left, TaffyClear::Left),
            (ClearValue::Right, TaffyClear::Right),
            (ClearValue::Both, TaffyClear::Both),
        ] {
            let mut cv = ComputedValues::initial();
            cv.clear = clear;
            let mut style = Style::default();
            bridge_float(&mut style, &cv);
            assert_eq!(style.clear, expected, "clear: {clear:?}");
        }
    }

    #[test]
    fn floated_box_with_explicit_width_is_positioned_at_the_containing_block_edge() {
        // Real float layout (taffy's `float_layout` feature, wired via
        // `bridge_float` + `taffy_impl`'s `LayoutBlockContainer` impl) —
        // a `float:left` box with an explicit width must be sized to
        // that width (not stretch to the container's full width like a
        // normal block child would) and must be positioned flush against
        // the containing block's start edge (CSS2 §9.5.1
        // <https://www.w3.org/TR/CSS2/visuren.html#propdef-float>: "The
        // left outer edge of a left-floating box may not be to the left
        // of the left edge of its containing block").
        //
        // This does NOT exercise shrink-to-fit sizing (CSS2 §10.3.5
        // <https://www.w3.org/TR/CSS2/visudet.html#float-width>, which only
        // applies when 'width' computes to 'auto') — this fixture gives an
        // explicit width on purpose. Shrink-to-fit-width coverage is a
        // separate, currently-untested gap.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let container =
            doc.append_element(Some(body), "div", Style::default(), Some("width:200px"));
        let float_child = doc.append_element(
            Some(container),
            "div",
            Style::default(),
            Some("float:left;width:60px;height:40px"),
        );

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let loc = doc.nodes[float_child].unrounded_layout.location;
        let size = doc.nodes[float_child].unrounded_layout.size;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (size.width - 60.0).abs() < 0.5,
            "explicit width must still be honored, got width={}",
            size.width
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            loc.x.abs() < 0.5,
            "left float must sit flush against the containing block's left edge, got x={}",
            loc.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            loc.y.abs() < 0.5,
            "first child's float must sit flush against the containing block's top edge, got y={}",
            loc.y
        );
    }

    #[test]
    fn nested_normal_flow_descendant_clears_ancestor_level_float() {
        // `LayoutBlockContainer::compute_block_child_layout`'s override in
        // `taffy_impl` only matters when a float and content that reacts to
        // it are at *different* nesting depths — direct siblings share one
        // `compute_block_layout` invocation (and hence one taffy
        // `BlockFormattingContext`) regardless of the override. This
        // fixture puts the float and a `clear:left` box two levels apart
        // (`float_sibling` is `container`'s child, `probe` is `container`'s
        // grandchild via the intervening `wrapper`), so the assertion only
        // holds if `wrapper`'s own recursive layout call continues
        // `container`'s `BlockFormattingContext` rather than starting a
        // fresh, float-blind one for `wrapper`'s own children (CSS2 §9.5.2
        // <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>: "clear"
        // requires the box's top border edge be below any earlier float in
        // the same block formatting context — not just floats that are its
        // own direct siblings).
        //
        // `wrapper` is left with no explicit width so it stretch-fits to
        // `container`'s full inner width, matching the BFC root's width —
        // avoiding a taffy `block_layout` limitation (a same-BFC child
        // narrower than its BFC root can get float insets computed against
        // the root's width instead of its own) that is orthogonal to what
        // this test pins.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let container =
            doc.append_element(Some(body), "div", Style::default(), Some("width:200px"));
        let float_sibling = doc.append_element(
            Some(container),
            "div",
            Style::default(),
            Some("float:left;width:60px;height:40px"),
        );
        let wrapper = doc.append_element(Some(container), "div", Style::default(), None::<&str>);
        let probe = doc.append_element(
            Some(wrapper),
            "div",
            Style::default(),
            Some("clear:left;width:50px;height:10px"),
        );

        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let float_loc = doc.nodes[float_sibling].unrounded_layout.location;
        let float_size = doc.nodes[float_sibling].unrounded_layout.size;
        let wrapper_loc = doc.nodes[wrapper].unrounded_layout.location;
        let probe_loc = doc.nodes[probe].unrounded_layout.location;

        // `location` is parent-relative in taffy, so translate `probe`'s y
        // into `container`'s coordinate space by walking up one level.
        let probe_y_in_container = wrapper_loc.y + probe_loc.y;
        let float_bottom = float_loc.y + float_size.height;

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            probe_y_in_container + 0.5 >= float_bottom,
            "clear:left box two BFC levels below the float must sit at or below the float's bottom edge, got float_bottom={float_bottom}, probe_y_in_container={probe_y_in_container}"
        );
    }

    #[test]
    fn bridge_gap_collapses_normal_to_zero() {
        // `computed_gap_component_to_taffy`'s `Normal` arm (CSS Box
        // Alignment 3 §8.1's "normal" keyword has no taffy equivalent, so
        // this bridge collapses it to `0`) — a direct pin, since it's
        // plausible for this branch to appear covered only incidentally
        // through unrelated tests' default (gap-less) fixtures rather
        // than being pinned on its own.
        let cv = ComputedValues::initial();
        assert_eq!(cv.row_gap, ComputedLengthPercentageOrNormal::Normal);
        assert_eq!(cv.column_gap, ComputedLengthPercentageOrNormal::Normal);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_gap(&mut style, &cv, &mut diag);
        assert_eq!(
            style.gap,
            Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            }
        );
    }

    #[test]
    fn bridge_gap_absolutizes_percent_gap() {
        // `computed_gap_component_to_taffy`'s `Percent` arm — the `Px`
        // arm has incidental coverage through `gap_adds_space_between_flex_items`
        // below, but no existing test exercises `row-gap`/`column-gap` with
        // a `<percentage>` value. `ComputedLengthPercentageOrNormal::Percent`
        // holds the authored number (`25.0` for `25%`), and the bridge
        // divides by 100 before handing it to taffy, same convention as
        // `bridge_flex_absolutizes_flex_basis_px_and_percent` above.
        let mut cv = ComputedValues::initial();
        cv.row_gap = ComputedLengthPercentageOrNormal::Percent(25.0);
        cv.column_gap = ComputedLengthPercentageOrNormal::Percent(10.0);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_gap(&mut style, &cv, &mut diag);
        assert_eq!(
            style.gap,
            Size {
                width: LengthPercentage::percent(0.1),
                height: LengthPercentage::percent(0.25),
            }
        );
    }

    #[test]
    fn gap_adds_space_between_flex_items() {
        // `bridge_gap` maps the `gap` shorthand's 2 components onto taffy's
        // `Size<LengthPercentage>` as `Size { width: column_gap, height:
        // row_gap }`. The 2 components are deliberately asymmetric (10px
        // vs 30px) so that a transposed mapping (swapping row/column) would
        // fail this assertion instead of passing it unnoticed — in a
        // row-direction container (the default `flex-direction`), the
        // main axis (x) gap is `column-gap`, so the second item's x offset
        // must be the first item's width **plus 30px**, not 10px.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;gap:10px 30px"),
        );
        let child_a = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let child_b = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[child_a].unrounded_layout.location;
        let b_loc = doc.nodes[child_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.x - a_loc.x - 130.0).abs() < 0.5,
            "second flex item should sit 130px (100px width + 30px column-gap) further along the main axis (x), got a.x={}, b.x={}",
            a_loc.x,
            b_loc.x
        );
    }

    #[test]
    fn justify_content_flex_end_pushes_children_to_container_end() {
        // `bridge_alignment`'s `justify_content` field reaching taffy's
        // main-axis alignment (§9.5 "Main-Axis Alignment") — with
        // `justify-content: flex-end` in a 500px-wide row container, 2
        // 100px-wide children must land flush against the container's end
        // edge (x = 500 - 200 = 300 for the first child).
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let flex_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:flex;width:500px;justify-content:flex-end"),
        );
        let child_a = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let child_b = doc.append_element(
            Some(flex_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[child_a].unrounded_layout.location;
        let b_loc = doc.nodes[child_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.x - 300.0).abs() < 0.5,
            "first item should be pushed flush to the container's end edge (500 - 100 - 100 = 300), got a.x={}",
            a_loc.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.x - 400.0).abs() < 0.5,
            "second item should sit immediately after the first (300 + 100 = 400), got b.x={}",
            b_loc.x
        );
    }

    #[test]
    fn layout_single_page_bridges_display_grid_to_taffy_grid() {
        // display bridge が Display::Grid を taffy::compute_grid_layout へ
        // 届けることを確認する。raikiri-style は `grid-template-columns` 等の
        // grid-* property を未実装 (本 task の scope 外、"entry point を開く"
        // だけが scope) なので、implicit single-track grid の挙動は block と
        // 見分けがつきにくい — ここでは「bridge が Display::Grid を発火させ、
        // compute_grid_layout がクラッシュせず有限な box を返す」ことのみを
        // pin する。track-level の挙動 pin は grid-* property 実装時の
        // follow-up の責務。
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let grid_container =
            doc.append_element(Some(body), "div", Style::default(), Some("display:grid"));
        let child_a = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let child_b = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("width:100px;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[grid_container].style.display,
            Display::Grid,
            "bridge_display must map DisplayValue::Grid to taffy::Display::Grid"
        );
        for id in [child_a, child_b] {
            let size = doc.nodes[id].unrounded_layout.size;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                size.width.is_finite()
                    && size.height.is_finite()
                    && size.width >= 0.0
                    && size.height >= 0.0,
                "grid item layout must be finite and non-negative, got {:?}",
                size
            );
        }
    }

    #[test]
    fn layout_single_page_deterministic_across_10_runs() {
        // 10 回連続実行で byte-identical であることを acceptance 条件とする。
        // 同一マシン上の determinism を pin (cross-machine は将来 font
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
        // 10 回 total で 5 秒未満なら現行実装の per-call new() は許容
        // (10 連ラン determinism test が timeout しないため)
        assert!(
            elapsed.as_secs() < 5,
            "FontContext::new() too slow: 10x = {:?}",
            elapsed
        );
    }

    // ── 非有限 f32 guard ────────
    //
    // untrusted author CSS から +Inf / NaN が taffy / parley に到達しないことを
    // **5 site すべて**で pin する。reproducer は元の probe comment 由来。
    //
    // 期待値は「非有限でない」ではなく **clamp 後の具体値** で書く — NaN は
    // `NaN != NaN` なので `assert_ne!(x, ...NAN)` は無条件に pass してしまい
    // guard の有無を判別できない。

    /// cascade → `apply_computed_to_style` を通した後の対象 element の
    /// `taffy::Style` を返す。
    ///
    /// fixture は **非 body element** (`<p>`) — `<body>` は後段
    /// `apply_page_box_to_body` で size を clobber されるため。
    fn guarded_style_for(inline: &str) -> taffy::Style {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let p = doc.append_element(Some(body), "p", Style::default(), Some(inline));
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        apply_computed_to_style(&mut doc, &cr);
        doc.nodes[p].style.clone()
    }

    /// site 1 — `computed_length_percentage_to_taffy_length_percentage` (padding)。
    #[test]
    fn nonfinite_padding_is_clamped_before_taffy() {
        use taffy::LengthPercentage;

        // Reproducer A': `1e40px` は cssparser の f64→f32 変換で +Inf になり、
        // `parse_padding_side` の `v >= 0.0` を **通過する** (inf >= 0.0 は true)。
        assert_eq!(
            guarded_style_for("padding-top: 1e40px").padding.top,
            LengthPercentage::length(MAX_TAFFY_MAGNITUDE),
        );

        // Reproducer A: IEEE 754 `0.0 * inf = NaN` — em の乗算で NaN が生まれる。
        // かつては `Em(_) => length(0.0)` arm がこれを吸収していた。
        assert_eq!(
            guarded_style_for("font-size: 0px; padding-top: 1e40em")
                .padding
                .top,
            LengthPercentage::length(0.0),
            "NaN は clamp では潰れないので is_nan() → 0.0 で処理する",
        );

        // percentage 側 (`Percent` arm) も同じ guard を通す。
        // (`1e40%` は raikiri の `parse_percentage` が cssparser の unit_value
        //  1e38 を `* 100.0` して +Inf にする — 実測。)
        assert_eq!(
            guarded_style_for("padding-top: 1e40%").padding.top,
            LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
        );

        // **有限だが巨大**な値も clamp する。上の 3 case はすべて f32 で既に
        // 非有限 (`1e40` は f32 で +Inf) なので、実装を
        // `if v.is_finite() { v } else { ... }` に「簡素化」しても全部 pass して
        // しまう。`1e38%` は `Percent(1e38)` = **有限** (実測) で fraction は
        // 1e36 になるため、この 1 本だけがその簡素化を殺す。
        assert_eq!(
            guarded_style_for("padding-top: 1e38%").padding.top,
            LengthPercentage::percent(MAX_TAFFY_MAGNITUDE),
            "有限だが巨大な percentage も clamp する (is_finite() だけの実装への regression guard)",
        );
    }

    /// site 2 — `computed_length_percentage_or_auto_to_taffy_dimension` (width / height)。
    #[test]
    fn nonfinite_size_is_clamped_before_taffy() {
        use taffy::Dimension;
        let s = guarded_style_for("width: 1e40px; height: 1e40%");
        assert_eq!(s.size.width, Dimension::length(MAX_TAFFY_MAGNITUDE));
        assert_eq!(s.size.height, Dimension::percent(MAX_TAFFY_MAGNITUDE));

        // NaN 経路 (em × font-size 0)。
        let n = guarded_style_for("font-size: 0px; width: 1e40em");
        assert_eq!(n.size.width, Dimension::length(0.0));
    }

    /// site 3 — `computed_length_percentage_or_auto_to_taffy_length_percentage_auto` (margin)。
    ///
    /// margin は **負値が spec-valid** (CSS Box 3 §3.1) なので clamp は対称
    /// (`[-MAX, MAX]`) でなければならない。
    #[test]
    fn nonfinite_margin_is_clamped_symmetrically_before_taffy() {
        use taffy::LengthPercentageAuto;
        assert_eq!(
            guarded_style_for("margin-top: 1e40px").margin.top,
            LengthPercentageAuto::length(MAX_TAFFY_MAGNITUDE),
        );
        assert_eq!(
            guarded_style_for("margin-top: -1e40px").margin.top,
            LengthPercentageAuto::length(-MAX_TAFFY_MAGNITUDE),
            "負の margin は spec-valid なので -MAX 側に clamp する (0 に潰さない)",
        );
        assert_eq!(
            guarded_style_for("font-size: 0px; margin-top: 1e40em")
                .margin
                .top,
            LengthPercentageAuto::length(0.0),
        );
        // `Percent` の負値経路 (`parse_margin_side` は allow-negative なので
        // `-1e40%` が parse を通り `Percent(-inf)` になる — 実測)。
        // `Px` 側だけだと `Percent` arm から `sanitize_taffy` を外す変更が
        // test を素通りする。
        assert_eq!(
            guarded_style_for("margin-left: -1e40%").margin.left,
            LengthPercentageAuto::percent(-MAX_TAFFY_MAGNITUDE),
        );
    }

    /// site 4 — `computed_length_to_taffy_length_percentage` (border-width)。
    #[test]
    fn nonfinite_border_width_is_clamped_before_taffy() {
        use taffy::LengthPercentage;
        assert_eq!(
            guarded_style_for("border-top-width: 1e40px; border-top-style: solid")
                .border
                .top,
            LengthPercentage::length(MAX_TAFFY_MAGNITUDE),
        );
        assert_eq!(
            guarded_style_for("font-size: 0px; border-top-width: 1e40em; border-top-style: solid")
                .border
                .top,
            LengthPercentage::length(0.0),
        );
    }

    /// site 5 — `preshape_text` の `cv.font_size.px()` → parley
    /// `StyleProperty::FontSize`。
    ///
    /// 観測は shape 後の `Layout::height()` — font-size が非有限なら line metrics
    /// が汚染されて height も非有限になる。
    ///
    /// # guard を外すと fail ではなく **hang** する
    ///
    /// 実測 (`sanitize_finite` を恒等関数に差し替えて単独実行): site 1-4 は即座に
    /// assert 失敗するが、本 site は 25 秒経っても終了しない。機構は
    /// `parley-0.10.0/src/layout/line_break.rs` の `if next_x <= max_advance` が
    /// `next_x = inf` で恒偽になり、`while self.break_next().is_some() {}` が
    /// 前進しないこと (shaping 自体は完了しており spin するのは `break_all_lines`)。
    ///
    /// そのため本 test は **worker thread + `recv_timeout` で有界化**してある —
    /// guard が消えた場合に「CI job が 20 分で殺される」(infra flake と区別
    /// できず、同一 binary の後続 test の結果も失われる) ではなく
    /// **assert failure** として落ちる。
    #[test]
    fn nonfinite_font_size_is_clamped_before_parley() {
        // 親 / 子の inline style を分けて渡す — `font-size` の `em` は **親**の
        // computed font-size 基準 (CSS Values 4 §6.1.1) なので、NaN (`0 * inf`)
        // を作るには乗数 `font-size: 0px` が親側に載っている必要がある。
        // site 1-4 は乗数が同一 element に載るので 1 element で作れるが、
        // font-size だけは 2 element 要る。
        fn shaped_height(parent_inline: Option<&str>, child_inline: &str) -> f32 {
            use parley::{FontContext, LayoutContext};
            use raikiri_style::{build_rule_tree, cascade};

            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), parent_inline);
            let p = doc.append_element(Some(body), "p", Style::default(), Some(child_inline));
            let text = doc.append_text(p, "Hi");
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);
            doc.nodes[text].text_layout().unwrap().height()
        }

        /// guard 消失時の hang を **有界時間の失敗**に変える wrapper。
        ///
        /// 有界なのは **test** であって process ではない — timeout しても worker
        /// thread は spin したまま残る (parley に cancellation が無く、`break_all_lines`
        /// を中断する手段がないため)。test binary の終了時に process ごと落ちるので
        /// 実害は無いが、「有界化した」の射程はここまで。
        fn shaped_height_bounded(parent_inline: Option<&str>, child_inline: &str) -> f32 {
            use std::sync::mpsc::RecvTimeoutError;

            let parent = parent_inline.map(str::to_owned);
            let child = child_inline.to_owned();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(shaped_height(parent.as_deref(), &child));
            });
            // `Timeout` と `Disconnected` を混同しないこと — `shaped_height` は
            // 内部に `.expect("cascade Ok")` / `.unwrap()` を持つので、worker が
            // panic すると `tx` が drop されて **数 ms で** `Disconnected` が
            // 返る。これを「30 秒で終わらなかった」と報告すると cascade の
            // regression を guard 消失として調査させてしまい、本 wrapper の
            // 導入目的 (hang を通常の失敗と区別する) の裏返しになる。
            match rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(h) => h,
                Err(RecvTimeoutError::Timeout) => panic!(
                    "parley shaping が 30 秒で終わらなかった — font-size の非有限 \
                     guard (sanitize_finite) が外れると break_all_lines が spin \
                     する"
                ),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("worker thread が panic した (hang ではない、上の stderr を参照)")
                }
            }
        }

        // (a) +Inf font-size。`1e40px` は cssparser の f64→f32 で +Inf。
        let inf_px = shaped_height_bounded(None, "font-size: 1e40px");
        assert!(
            inf_px.is_finite(),
            "font-size +Inf (px 由来) が parley に届いた: {inf_px}"
        );

        // (b) +Inf font-size (em compounding 由来)。親は initial の 16px なので
        // `16.0 * inf = +Inf` — **NaN ではない**。
        let inf_em = shaped_height_bounded(None, "font-size: 1e40em");
        assert!(
            inf_em.is_finite(),
            "font-size +Inf (em 由来) が parley に届いた: {inf_em}"
        );

        // (c) **NaN** font-size — `0.0 * inf` (IEEE 754)。`font-size` の `em` は
        // **親**の computed font-size 基準 (CSS Values 4 §6.1.1) なので乗数
        // `font-size: 0px` は親側に載る。site 1-4 は乗数が同一 element に載るので
        // 1 element で作れるが、font-size だけは 2 element 要る。
        //
        // **`is_nan()` 分岐削除 mutation は本 case では死なない (実測)。** guard が生きている限り
        // parley が受け取るのは 0.0 であって NaN ではないので、**parley 側の
        // NaN 許容が変わってもここでは気づけない** (「上流の canary」ではない)。
        // `is_nan()` 分岐を殺す mutation を検出するのは site 1-4 の e2e 4 本と
        // `sanitize_finite_maps_nan_to_zero` の計 5 本 (mutation testing 実測)。
        //
        // それでも置く理由は 2 つ:
        //   1. NaN を作れる経路の一つ (親 `0px` × 子 `em`) が e2e で構築
        //      できることの pin。site 1-4 と違い 1 element では作れない。
        //   2. 「guard 消失 × 上流の NaN 許容変化」という複合 regression への
        //      保険 (単独ではどちらも他の test が拾う)。
        let nan = shaped_height_bounded(Some("font-size: 0px"), "font-size: 1e40em");
        assert!(nan.is_finite(), "font-size NaN が parley に届いた: {nan}");
        assert_eq!(
            nan, 0.0,
            "guard 後の font-size 0.0 に対する parley の height (上流変更の canary)",
        );
    }

    /// sites 7-8 — `preshape_text` の `cv.line_height` → parley
    /// `StyleProperty::LineHeight`。site 5 (`font-size`) と同じ「guard を
    /// 外すと fail ではなく hang する」site だが、機構は別
    /// (`sanitize_line_height` の doc参照 — `next_x <= max_advance` ではなく
    /// `running_line_height > line_max_height` が恒真になる)。
    ///
    /// 通常の `cascade()` だけで非有限値を作れる — `line-height: 1e40`
    /// (unitless number)、`line-height: 1e40px` (absolute length) はいずれも
    /// cssparser の f64→f32 変換で `+Inf` に saturate する (site 5 の
    /// `font-size: 1e40px` と同じ機構)。font-size と違い 2 element も
    /// bypass も要らない。
    #[test]
    fn nonfinite_line_height_is_clamped_before_parley() {
        fn shaped_height(inline_style: &str) -> f32 {
            use parley::{FontContext, LayoutContext};
            use raikiri_style::{build_rule_tree, cascade};

            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
            let text = doc.append_text(p, "Hi");
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).expect("cascade Ok");
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);
            doc.nodes[text].text_layout().unwrap().height()
        }

        /// guard 消失時の hang を有界時間の失敗に変える wrapper — site 5 の
        /// `shaped_height_bounded` と同じ構造 (doc参照)。
        fn shaped_height_bounded(inline_style: &str) -> f32 {
            use std::sync::mpsc::RecvTimeoutError;

            let owned = inline_style.to_owned();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(shaped_height(&owned));
            });
            // cov:ignore: every call site of this helper (both assertions
            // below) completes normally within its 30s bound — the Err arms
            // are diagnostics for failure modes (guard regression hang,
            // worker panic) this test's passing runs never hit, same as
            // `shape_raw_bounded`'s sibling match further down this file.
            match rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok(h) => h,
                Err(RecvTimeoutError::Timeout) => panic!(
                    "parley shaping が 30 秒で終わらなかった — line-height の非有限 \
                     guard (sanitize_line_height) が外れると BreakerState::add_line_height \
                     の max_height_exceeded 分岐が spin する"
                ),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("worker thread が panic した (hang ではない、上の stderr を参照)")
                }
            }
        }

        let number_inf = shaped_height_bounded("line-height: 1e40");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            number_inf.is_finite(),
            "line-height +Inf (unitless number 由来) が parley に届いた: {number_inf}"
        );

        let length_inf = shaped_height_bounded("line-height: 1e40px");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            length_inf.is_finite(),
            "line-height +Inf (absolute length 由来) が parley に届いた: {length_inf}"
        );
    }

    /// Shapes `"Hi"` via parley **directly**
    /// (bypassing `preshape_text` / `sanitize_finite` entirely, not just
    /// disabling them) with a raw `font_size`, bounded via worker-thread +
    /// `recv_timeout`. Shared by the two `#[test]` fns below it: one pins the
    /// (fast, cheap) "does not hang" cases, the other — `#[ignore]`d, see its
    /// own doc — pins the one case that does.
    fn shape_raw_bounded(font_size: f32, bound: std::time::Duration) -> Result<(), &'static str> {
        use std::sync::mpsc::RecvTimeoutError;

        fn shape_raw(font_size: f32) {
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            let mut builder = layout_cx.ranged_builder(&mut fonts, "Hi", 1.0, true);
            builder.push_default(StyleProperty::FontSize(font_size));
            let mut layout: Layout<()> = builder.build("Hi");
            // A4 width in px, matching `PageBox::A4.width` — the same
            // `max_advance` `preshape_text` would pass in production.
            layout.break_all_lines(Some(793.7008_f32));
        }

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(move || shape_raw(font_size));
            let _ = tx.send(result.is_ok());
        });
        // cov:ignore: every call site of this helper (both this file's
        // tests) completes normally within its bound — the Err arms are
        // diagnostics for failure modes (panic, timeout, worker-disconnect)
        // this module's tests don't hit.
        match rx.recv_timeout(bound) {
            Ok(true) => Ok(()),
            Ok(false) => Err("panicked"),
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
            Err(RecvTimeoutError::Disconnected) => Err("panicked"),
        }
    }

    /// Narrower half of a paired characterization — pins that `NaN`,
    /// `-Inf`, and a merely-huge finite `font_size` (`1e9`) **do not** hang
    /// parley's `break_all_lines`, at the same raw (guard-bypassing) call
    /// site the `#[ignore]`d `+Inf` test below uses. Cheap (each sub-case
    /// resolves in well under the 5s bound; no leaked spinning thread since
    /// none of them hang), so — unlike the `+Inf` case — this runs in every
    /// default `cargo test`.
    ///
    /// # Why this exists as assertions, not just prose
    ///
    /// The doc comment on `MAX_FONT_SIZE_PX` ("guard を外すと... 25 秒経っ
    /// ても終了しない") reads as "non-finite font-size ⇒ hang" in general —
    /// but that claim was written from a manual repro that only ever
    /// exercised `+Inf` (the first sub-case its guarded test tries) before
    /// hanging; it never got to see whether `NaN` or `-Inf` behave the same
    /// way. They do not: only `+Inf` hangs, via the specific mechanism
    /// documented on `MAX_FONT_SIZE_PX`
    /// (`parley-0.10.0/src/layout/line_break.rs`'s `if next_x <= max_advance`
    /// becoming permanently false once `next_x = +Inf`, so
    /// `while self.break_next().is_some() {}` never terminates — for `NaN`
    /// and `-Inf`, `next_x` does not end up stuck the same way). This test
    /// turns "narrower than the prose it formalizes" from an unverified
    /// assertion in a code comment into something a future `cargo test` run
    /// keeps honest.
    #[test]
    fn parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size() {
        for (label, font_size) in [
            ("NaN", f32::NAN),
            ("-Inf", f32::NEG_INFINITY),
            ("1e9 (finite, 3 decades past MAX_FONT_SIZE_PX)", 1e9_f32),
        ] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                shape_raw_bounded(font_size, std::time::Duration::from_secs(5)),
                Ok(()),
                "parley::Layout::break_all_lines(font_size = {label}) did not complete within 5s (bypassing raikiri's guard, same as the +Inf case) — this module's characterization that only +Inf hangs no longer holds for {label}; re-characterize rather than deleting this case"
            );
        }
    }

    /// `+Inf` half of the paired characterization — formalizes into an
    /// automated regression test the manual measurement recorded in
    /// `MAX_FONT_SIZE_PX`'s doc comment ("guard を外すと... 25 秒経っても
    /// 終了しない"): `font_size = +Inf` reaching parley directly (bypassing
    /// `preshape_text` / `sanitize_finite`, not just disabling them)
    /// reproducibly hangs `break_all_lines`. See
    /// `parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size`
    /// for why `NaN`/`-Inf`/huge-finite do *not* share this behavior (this is
    /// the one case that does, and it's the one this module's own
    /// repro — `1e40px`, `1e40em` compounding — actually produces).
    ///
    /// # Why `#[ignore]` (unlike every other test added alongside it)
    ///
    /// Every other characterization test in this pair resolves in
    /// well under a second because the sink under test either doesn't hang
    /// or fails fast. This one is different **in the passing case**: parley
    /// has no shaping-cancellation mechanism (documented on `MAX_FONT_SIZE_PX`
    /// and above), so confirming the hang costs the full `bound` below on
    /// every run, *and* the spawned worker thread is never joined — it spins
    /// at ~100% CPU on one core for the rest of this test binary's process
    /// lifetime, degrading every test that runs after it in the same binary.
    /// That's an acceptable one-time characterization cost but not a
    /// standing tax worth imposing on every `cargo test --workspace` from
    /// every future session — hence `#[ignore]`, matching this repo's
    /// existing convention for exactly this trade-off
    /// (`crates/raikiri/tests/hello_world_vrt.rs`'s doc comment). Run
    /// explicitly with:
    ///
    /// ```text
    /// cargo test -p raikiri-dom --lib \
    ///   layout::tests::parley_break_all_lines_hangs_on_raw_infinite_font_size_bypassing_the_guard \
    ///   -- --ignored
    /// ```
    // cov:ignore: this whole test body never runs under default `cargo
    // test` (it's `#[ignore]`d — a genuine ~10s hang + leaked thread, see
    // the doc comment above); it's exercised explicitly via `-- --ignored`
    // (verified separately to run and pass), which
    // llvm-cov's default `cargo test` invocation doesn't capture.
    #[test]
    #[ignore = "confirms a genuine ~10s hang + leaks a spinning worker thread for the rest \
                of the process; run explicitly, see doc comment"]
    fn parley_break_all_lines_hangs_on_raw_infinite_font_size_bypassing_the_guard() {
        assert_eq!(
            shape_raw_bounded(f32::INFINITY, std::time::Duration::from_secs(10)),
            Err("timeout"),
            "parley::Layout::break_all_lines(font_size = +Inf) did not hang within 10s \
             — the line_break.rs livelock this test pins no longer reproduces in parley 0.10.0; \
             re-characterize rather than deleting this test (and consider whether \
             raikiri-dom's own MAX_FONT_SIZE_PX guard is still load-bearing for this \
             specific sink if parley itself now handles it). If this instead reports \
             \"panicked\", the worker thread panicked rather than hanging — that's a \
             different (and likely worse, since panics propagate less predictably than \
             a bounded hang) finding, not a pass"
        );
    }

    // ── guard 関数そのものの unit test ───────────────────────────────────
    //
    // e2e test は site 5 が hang し得るうえ 1 本あたり FontContext 構築を伴う。
    // guard の算術は純関数なので直接叩く (数 ms、hang し得ない)。

    #[test]
    fn sanitize_finite_maps_nan_to_zero() {
        // `f32::clamp` は NaN を NaN のまま返すので、この分岐が無いと NaN が
        // 素通りする。
        let mut diag = Vec::new();
        assert_eq!(sanitize_finite(f32::NAN, -1.0, 1.0, "test", &mut diag), 0.0);
        assert_eq!(
            sanitize_finite(f32::NAN, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
            0.0
        );
        // 両方とも実際に clamp した (NaN != 0.0) ので、それぞれ 1 event ずつ
        // `LayoutWarn::NonFiniteClamped` が積まれる。
        assert_eq!(
            diag.len(),
            2,
            "clamp が発火した回数だけ event が積まれること"
        );
        for event in &diag {
            match event {
                LayoutWarn::NonFiniteClamped { site, raw, clamped } => {
                    assert_eq!(*site, "test");
                    assert!(raw.is_nan());
                    assert_eq!(*clamped, 0.0);
                }
                other => panic!("unexpected LayoutWarn variant: {other:?}"),
            }
        }
    }

    #[test]
    fn sanitize_finite_clamps_infinities_to_bounds() {
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_finite(f32::INFINITY, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
            MAX_FONT_SIZE_PX
        );
        assert_eq!(
            sanitize_finite(
                f32::NEG_INFINITY,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "test",
                &mut diag
            ),
            -MAX_TAFFY_MAGNITUDE
        );
        // 下限が 0.0 の site (font-size) では -Inf は 0.0 に落ちる。
        assert_eq!(
            sanitize_finite(f32::NEG_INFINITY, 0.0, MAX_FONT_SIZE_PX, "test", &mut diag),
            0.0
        );
        assert_eq!(diag.len(), 3, "3 回とも clamp が発火する (全て非有限入力)");
    }

    #[test]
    fn sanitize_taffy_clamps_out_of_range_finite_values() {
        // 有限でも範囲外なら寄せる (「有限化するだけ」ではない)。
        let mut diag = Vec::new();
        assert_eq!(sanitize_taffy(1e30, "test", &mut diag), MAX_TAFFY_MAGNITUDE);
        assert_eq!(
            sanitize_taffy(-1e30, "test", &mut diag),
            -MAX_TAFFY_MAGNITUDE
        );
        assert_eq!(diag.len(), 2);
    }

    #[test]
    fn sanitize_taffy_passes_through_in_range_values() {
        // 通常値は bit-identical に素通しする (VRT が pixel-exact である前提)。
        let mut diag = Vec::new();
        for v in [0.0_f32, 1.0, -1.0, 16.0, 793.7008, MAX_TAFFY_MAGNITUDE] {
            assert_eq!(
                sanitize_taffy(v, "test", &mut diag),
                v,
                "in-range value must pass through: {v}"
            );
        }
        // 範囲内 (clamp が実質 no-op) では何も積まない — per-node spam を
        // 避ける設計の pin (`sanitize_finite` の doc参照)。
        assert!(
            diag.is_empty(),
            "in-range value must not push a LayoutWarn: {diag:?}"
        );
    }

    // ── site 6: sanitize_font_weight ─────────────
    //
    // sanitize_finite / sanitize_taffy と同型の unit test。`ComputedValues`
    // が全 field `pub` であることに由来する非有限 font_weight (f32 格上げで
    // 型による排除ができなくなった) が
    // `parley::FontWeight::new` の直前で有限 + `[1,1000]` に収まることを
    // 直接検証する。

    #[test]
    fn sanitize_font_weight_maps_nan_to_normal_fallback() {
        // `f32::clamp` は NaN を NaN のまま返すので、この分岐が無いと NaN が
        // 素通りする。fallback は `0.0` ではなく `FALLBACK_FONT_WEIGHT`
        // (400.0、CSS Fonts 4 §2.2 "Font weight: the font-weight property"
        // <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> の
        // `normal` の computed value) — `sanitize_finite` の length 系 site
        // とは異なる fallback を選ぶ理由は `sanitize_font_weight` の doc 参照。
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_font_weight(f32::NAN, &mut diag),
            FALLBACK_FONT_WEIGHT
        );
        assert_eq!(diag.len(), 1, "clamp が発火したので 1 event 積まれること");
        match diag[0] {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => {
                assert_eq!(site, "font-weight");
                assert!(raw.is_nan());
                assert_eq!(clamped, FALLBACK_FONT_WEIGHT);
            }
            other => panic!("unexpected LayoutWarn variant: {other:?}"),
        }
    }

    #[test]
    fn sanitize_font_weight_clamps_infinities_to_bounds() {
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_font_weight(f32::INFINITY, &mut diag),
            MAX_FONT_WEIGHT
        );
        assert_eq!(
            sanitize_font_weight(f32::NEG_INFINITY, &mut diag),
            MIN_FONT_WEIGHT
        );
        assert_eq!(diag.len(), 2, "+Inf / -Inf とも clamp が発火する");
    }

    #[test]
    fn sanitize_font_weight_clamps_out_of_range_finite_values() {
        // 有限でも範囲外なら寄せる (「有限化するだけ」ではない) —
        // `sanitize_taffy_clamps_out_of_range_finite_values` の font-weight 版。
        let mut diag = Vec::new();
        assert_eq!(sanitize_font_weight(1e30, &mut diag), MAX_FONT_WEIGHT);
        assert_eq!(sanitize_font_weight(-1e30, &mut diag), MIN_FONT_WEIGHT);
        // `0.0` は length 系 site では有効な値だが font-weight の妥当域
        // `[1, 1000]` の外 — MIN_FONT_WEIGHT に寄る。
        assert_eq!(sanitize_font_weight(0.0, &mut diag), MIN_FONT_WEIGHT);
        assert_eq!(diag.len(), 3);
    }

    #[test]
    fn sanitize_font_weight_passes_through_in_range_values() {
        // 通常値 (fractional weight 含む) は
        // bit-identical に素通しする。
        let mut diag = Vec::new();
        for v in [
            MIN_FONT_WEIGHT,
            1.0,
            100.0,
            349.5,
            400.0,
            700.0,
            MAX_FONT_WEIGHT,
        ] {
            assert_eq!(
                sanitize_font_weight(v, &mut diag),
                v,
                "in-range value must pass through: {v}"
            );
        }
        assert!(
            diag.is_empty(),
            "in-range value must not push a LayoutWarn: {diag:?}"
        );
    }

    #[test]
    fn sanitize_line_height_passes_through_in_range_values() {
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Normal, &mut diag),
            ComputedLineHeight::Normal
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(0.0), &mut diag),
            ComputedLineHeight::Number(0.0)
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(1.5), &mut diag),
            ComputedLineHeight::Number(1.5)
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Length(ComputedLength(0.0)), &mut diag),
            ComputedLineHeight::Length(ComputedLength(0.0))
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Length(ComputedLength(32.0)), &mut diag),
            ComputedLineHeight::Length(ComputedLength(32.0))
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            diag.is_empty(),
            "in-range value must not push a LayoutWarn: {diag:?}"
        );
    }

    #[test]
    fn sanitize_line_height_clamps_non_finite_and_out_of_range_number() {
        // site 7: `ComputedLineHeight::Number` — grammar `<number [0,∞]>`
        // なので下限 0.0、上限 `MAX_LINE_HEIGHT_NUMBER`。NaN は他の length 系
        // site と同じ `sanitize_finite` の `0.0` fallback を継承する (font-weight
        // のような専用 fallback が要らない理由: line-height の unitless
        // number に `0` は grammar 上有効な値であり、font-weight の `400.0`
        // 事情 — `0.0` が妥当域外 — が line-height には無い)。
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(f32::NAN), &mut diag),
            ComputedLineHeight::Number(0.0)
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(f32::INFINITY), &mut diag),
            ComputedLineHeight::Number(MAX_LINE_HEIGHT_NUMBER)
        );
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(f32::NEG_INFINITY), &mut diag),
            ComputedLineHeight::Number(0.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Number(-5.0), &mut diag),
            ComputedLineHeight::Number(0.0),
            "negative multiplier is out of the [0,∞] grammar range and must clamp to 0.0"
        );
        assert_eq!(diag.len(), 4);
        for event in &diag {
            match event {
                LayoutWarn::NonFiniteClamped { site, .. } => {
                    assert_eq!(*site, "line-height (number)");
                }
                // cov:ignore: `diag` in this test only ever accumulates
                // `NonFiniteClamped` events pushed by `sanitize_line_height`
                // above — `LayoutWarn::Truncated` is pushed elsewhere
                // (the `layout_single_page` cap-limiting path), never by
                // this function, so this arm is unreachable with this
                // test's inputs; it exists only for the match's
                // exhaustiveness.
                other => panic!("unexpected LayoutWarn variant: {other:?}"),
            }
        }
    }

    #[test]
    fn sanitize_line_height_clamps_non_finite_and_out_of_range_length() {
        // site 8: `ComputedLineHeight::Length` — grammar
        // `<length-percentage [0,∞]>` の percentage は computed 層で既に
        // px へ絶対化済み (`ComputedLineHeight::Length` の doc参照) なので
        // ここでは px の妥当域だけを見る。
        let mut diag = Vec::new();
        assert_eq!(
            sanitize_line_height(
                ComputedLineHeight::Length(ComputedLength(f32::NAN)),
                &mut diag
            ),
            ComputedLineHeight::Length(ComputedLength(0.0))
        );
        assert_eq!(
            sanitize_line_height(
                ComputedLineHeight::Length(ComputedLength(f32::INFINITY)),
                &mut diag
            ),
            ComputedLineHeight::Length(ComputedLength(MAX_FONT_SIZE_PX))
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            sanitize_line_height(ComputedLineHeight::Length(ComputedLength(-10.0)), &mut diag),
            ComputedLineHeight::Length(ComputedLength(0.0)),
            "negative absolute line-height is out of the [0,∞] grammar range and must clamp to 0.0"
        );
        assert_eq!(diag.len(), 3);
        for event in &diag {
            match event {
                LayoutWarn::NonFiniteClamped { site, .. } => {
                    assert_eq!(*site, "line-height (length)");
                }
                // cov:ignore: same unreachable-exhaustiveness arm as the
                // sibling `Number` test above — `diag` here never
                // accumulates a `Truncated` event.
                other => panic!("unexpected LayoutWarn variant: {other:?}"),
            }
        }
    }

    #[test]
    fn font_style_to_parley_maps_normal_and_italic() {
        assert_eq!(
            font_style_to_parley(StyleFontStyle::Normal),
            FontStyle::Normal
        );
        assert_eq!(
            font_style_to_parley(StyleFontStyle::Italic),
            FontStyle::Italic
        );
    }

    #[test]
    fn line_height_to_parley_maps_all_three_variants() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            line_height_to_parley(ComputedLineHeight::Normal),
            LineHeight::MetricsRelative(1.0),
            "Normal must map to parley's own default (MetricsRelative(1.0))"
        );
        assert_eq!(
            line_height_to_parley(ComputedLineHeight::Number(1.5)),
            LineHeight::FontSizeRelative(1.5)
        );
        assert_eq!(
            line_height_to_parley(ComputedLineHeight::Length(ComputedLength(32.0))),
            LineHeight::Absolute(32.0)
        );
    }

    #[test]
    fn preshape_text_pushes_computed_font_style_into_parley_run_attrs() {
        // `preshape_text` が `cv.font_style` を実際に RangedBuilder へ push して
        // いることを、shape 済 `Run` の font-matching 属性から確認する。
        //
        // `GlyphRun::style()` (`parley::layout::Style<B>`) は brush /
        // underline / strikethrough / 非公開 line_height 等のみで
        // `font_style` field を持たないため使えない。代わりに
        // `Run::font_attrs()` (`&fontique::Attributes`, `pub style: FontStyle`
        // field を持つ) を使う — これは実際に選ばれた font file の属性では
        // なく、font matching に**渡された** CSS-requested attribute
        // そのもの (parley `shape` module が `RangedBuilder` へ push した
        // `StyleProperty::FontStyle` から直接組み立てる) なので、実行環境に
        // italic face を持つフォントがあるかどうかに関わらず決定的に検証できる。
        use parley::{FontContext, LayoutContext, PositionedLayoutItem};
        use raikiri_style::{build_rule_tree, cascade};

        fn shape_run_font_style(inline_style: &str) -> FontStyle {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
            let text = doc.append_text(p, "Hi");
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).unwrap();
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);
            let layout = doc.nodes[text].text_layout().unwrap();
            let line = layout.lines().next().expect("shaped text has one line");
            let item = line
                .items()
                .next()
                .expect("shaped line has at least one item");
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                // cov:ignore: only reached if shaping produces an InlineBox
                // instead of a GlyphRun — preshape_text (this file) never
                // pushes an inline box, matching the same premise
                // `crates/raikiri-paint/src/text.rs`'s glyph-draw walk
                // relies on ("InlineBox は現状生成されない" there), so
                // this is unreachable for plain text today.
                panic!("expected shaped text to produce a GlyphRun, got an InlineBox");
            };
            glyph_run.run().font_attrs().style
        }

        assert_eq!(shape_run_font_style("font-style:normal"), FontStyle::Normal);
        // cov:ignore: this multi-line assert_eq! is exempted as a whole
        // block by patch_coverage.py's bracket-depth scoping rule (the
        // marker's block starts at the open paren below and doesn't close
        // until the matching `);`, so every line in between — including
        // the always-executed call/comparison lines, not just the
        // panic-message continuation lines below — reports exempted). The
        // assertion itself is fully exercised on every run and would fail
        // on a wiring regression; only the panic-message string (only
        // "entered" on assertion failure, which doesn't happen while this
        // test passes) is the actual reason for the coverage-attribution
        // gap this marker works around.
        assert_eq!(
            shape_run_font_style("font-style:italic"),
            FontStyle::Italic,
            "font-style:italic must reach the shaped Run's font-matching attributes \
             (wiring regression: StyleProperty::FontStyle push missing or dropped)"
        );
    }

    #[test]
    fn preshape_text_pushes_computed_line_height_into_parley_run_metrics() {
        // `preshape_text` が `cv.line_height` を実際に RangedBuilder へ push
        // していることを、shape 済 `Run` の `RunMetrics::line_height` から
        // 確認する。`font_style` の兄弟 test と違い `Run::font_attrs()` では
        // 検証できない (`fontique::Attributes` に line-height 相当の field は
        // 無い) — 代わりに `Run::metrics()` (`&RunMetrics`, `pub line_height:
        // f32` field を持つ) を使う。`Number` / `Length` はいずれも font
        // metrics (ascent / descent / leading) に依存しない計算式
        // (`parley-0.10.0/src/layout/data.rs` の `push_run` 内 `match
        // style.line_height`) なので、実行環境のフォントに関わらず厳密な値で
        // 決定的に検証できる。
        use parley::{FontContext, LayoutContext, PositionedLayoutItem};
        use raikiri_style::{build_rule_tree, cascade};

        fn shaped_run_line_height(inline_style: &str) -> f32 {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let p = doc.append_element(Some(body), "p", Style::default(), Some(inline_style));
            let text = doc.append_text(p, "Hi");
            let rules = build_rule_tree(&doc);
            let cr = cascade(&doc, &rules).unwrap();
            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);
            let layout = doc.nodes[text].text_layout().unwrap();
            let line = layout.lines().next().expect("shaped text has one line");
            let item = line
                .items()
                .next()
                .expect("shaped line has at least one item");
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                // cov:ignore: preshape_text never produces an InlineBox for
                // plain text — see the same premise in the font-style
                // sibling test above.
                panic!("expected shaped text to produce a GlyphRun, got an InlineBox");
            };
            glyph_run.run().metrics().line_height
        }

        // `Number` (unitless multiplier) — `data.rs`'s `FontSizeRelative(value)
        // => value * font_size` is exact arithmetic on the pushed
        // `StyleProperty::FontSize` value (`font-size: 16px` here), so the
        // expected result is bit-computable, not just "taller than".
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            shaped_run_line_height("font-size: 16px; line-height: 3"),
            48.0,
            "line-height: 3 at font-size: 16px must reach the shaped Run's \
             line_height as exactly 3.0 * 16.0 (wiring regression: \
             StyleProperty::LineHeight push missing, dropped, or mismapped)"
        );

        // `Length` (absolute px) — `data.rs`'s `Absolute(value) => value` is a
        // pure passthrough, so the expected result is the declared px value
        // verbatim, independent of font-size.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            shaped_run_line_height("font-size: 16px; line-height: 50px"),
            50.0,
            "line-height: 50px must reach the shaped Run's line_height as \
             exactly 50.0 regardless of font-size (wiring regression: \
             StyleProperty::LineHeight push missing, dropped, or mismapped)"
        );

        // `Normal` (the property's initial value, and the case this module
        // used unconditionally before this wiring) must still resolve to a
        // positive, finite metrics-derived value — pins that leaving
        // line-height unset doesn't regress to 0 or a non-finite value.
        let normal = shaped_run_line_height("font-size: 16px");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal.is_finite() && normal > 0.0,
            "line-height: normal (default) must still produce a finite, \
             positive line_height: {normal}"
        );
    }

    #[test]
    fn preshape_text_sanitizes_non_finite_font_weight_bypassing_cascade() {
        // `ComputedValues` は全 field が `pub` なので、cascade を経由しない
        // 直接構築 (ここでは cascade() 後に該当 node の font_weight だけを
        // 上書きする形で再現) から非有限値が来る経路がある。この経路が
        // `preshape_text` を panic させないこと — sink 直前で
        // `sanitize_font_weight` が有限化すること — を確認する。
        //
        // `CascadeResult` / `ComputedValues` はどちらも `#[non_exhaustive]`
        // なので、raikiri-dom (外部 crate) からは struct literal で直接
        // construct できない。正当な `cascade()` 呼び出しで得た
        // `CascadeResult` の `pub computed: Vec<ComputedValues>` を後から
        // 上書きすることで、「cascade を経由しない値」を再現する — これは
        // `ComputedValues::font_weight` の doc が挙げる
        // `crate::page::cascade_page` の継承元 root 引数と同じ攻撃面
        // (呼び出し元が任意の `ComputedValues` を用意して渡せる) の縮図。
        //
        // `text_layout().is_some()` だけでは「panic しなかった」ことしか
        // 検証できない — 将来誰かが `preshape_text` から
        // `sanitize_font_weight` の呼び出しを誤って外しても (parley が
        // 非有限値を panic せず黒箱処理する場合)、それは検知できない。
        // そこで `doc.layout_warnings` (`sanitize_font_weight` が実際に
        // clamp した時だけ push する `LayoutWarn::NonFiniteClamped`
        // の蓄積先) を直接検査し、sink 直前に渡った raw 値と、そこから
        // 実際に有限化された値の両方を assert する — 配線が外れれば
        // site `"font-weight"` の event が一切積まれなくなるので、
        // その断線をここで検知できる。
        use parley::{FontContext, LayoutContext};
        use raikiri_style::{build_rule_tree, cascade};

        for (label, weight) in [
            ("NaN", f32::NAN),
            ("+Inf", f32::INFINITY),
            ("-Inf", f32::NEG_INFINITY),
            ("out-of-range finite (1e30)", 1e30_f32),
        ] {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
            let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
            let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
            let text = doc.append_text(p, "Hi");

            let rules = build_rule_tree(&doc);
            let mut cr = cascade(&doc, &rules).expect("cascade Ok");
            cr.computed[p].font_weight = weight;
            cr.computed[text].font_weight = weight;

            let mut fonts = FontContext::new();
            let mut layout_cx = LayoutContext::<()>::new();
            preshape_text(&mut doc, &cr, &mut fonts, &mut layout_cx, PageBox::A4.width);

            assert!(
                doc.nodes[text].text_layout().is_some(),
                "{label}: preshape_text must not panic and must still populate \
                 text_layout despite a non-finite font_weight bypassing cascade"
            );

            // 配線検証: font-size はこの test では触っていないので clamp は
            // 発火せず、"font-weight" site の event だけが (毎 case とも
            // clamp が実際に効くので) ちょうど 1 件積まれるはず。
            let font_weight_events: Vec<LayoutWarn> = doc
                .layout_warnings
                .iter()
                .copied()
                .filter(|w| {
                    matches!(
                        w,
                        LayoutWarn::NonFiniteClamped {
                            site: "font-weight",
                            ..
                        }
                    )
                })
                .collect();
            assert_eq!(
                font_weight_events.len(),
                1,
                "{label}: expected exactly one \"font-weight\" NonFiniteClamped \
                 event (only pushed when sanitize_font_weight actually ran and \
                 clamped) — 0 events means the sanitize_font_weight call was \
                 removed from preshape_text without this test noticing; \
                 all events: {:?}",
                doc.layout_warnings
            );
            let LayoutWarn::NonFiniteClamped { raw, clamped, .. } = font_weight_events[0] else {
                unreachable!("filtered for this variant above");
            };
            assert!(
                raw.is_nan() == weight.is_nan() && (raw.is_nan() || raw == weight),
                "{label}: the raw value sanitize_font_weight saw ({raw}) must be \
                 the exact font_weight this test set ({weight}), proving \
                 sanitize_font_weight is wired to cv.font_weight and not some \
                 unrelated value"
            );
            assert!(
                clamped.is_finite(),
                "{label}: the value handed to parley::FontWeight::new must be \
                 finite, got {clamped}"
            );
            assert!(
                (MIN_FONT_WEIGHT..=MAX_FONT_WEIGHT).contains(&clamped),
                "{label}: the value handed to parley::FontWeight::new must be \
                 within [{MIN_FONT_WEIGHT}, {MAX_FONT_WEIGHT}], got {clamped}"
            );
        }
    }

    // ── crate::diag 経由の generalized 診断 channel ──
    // fonts.rs の FontWarn observer pattern を汎用化した LayoutWarn 側の
    // 独自 unit test。fonts.rs の `observer_fires_*` test 群と対になる。

    /// `emit_layout_warn` は observer が `Some` ならそれを呼び、`eprintln!`
    /// はしない — fonts.rs の `emit_warn` と対称的な契約 (両方とも
    /// `crate::diag::emit_warn_via` を経由するので同じ振る舞いになるはず)。 // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    #[test]
    fn emit_layout_warn_calls_observer_when_some() {
        let mut collected: Vec<LayoutWarn> = Vec::new();
        let mut cb = |w: &LayoutWarn| collected.push(*w);
        let mut observer: LayoutWarnObserver<'_> = Some(&mut cb);
        emit_layout_warn(
            &mut observer,
            LayoutWarn::NonFiniteClamped {
                site: "test",
                raw: f32::NAN,
                clamped: 0.0,
            },
        );
        assert_eq!(collected.len(), 1);
        // float literal は pattern に書けない (`illegal_floating_point_literal_pattern`
        // は deny-by-default) ので variant/site だけ matches! で確認し、
        // `clamped` の値は別途 `if let` で束縛して assert する。
        assert!(matches!(
            collected[0],
            LayoutWarn::NonFiniteClamped { site: "test", .. }
        ));
        if let LayoutWarn::NonFiniteClamped { clamped, .. } = collected[0] {
            assert_eq!(clamped, 0.0);
        }
    }

    /// `observer == None` では代わりに `eprintln!` する — 呼び出しても panic
    /// しないことだけを確認する (stderr の内容は capture しない、fonts.rs の
    /// 対応する経路も同様に未検証)。
    #[test]
    fn emit_layout_warn_falls_back_to_eprintln_when_none() {
        let mut observer: LayoutWarnObserver<'_> = None;
        emit_layout_warn(&mut observer, LayoutWarn::Truncated { suppressed: 3 });
    }

    /// `push_layout_warn` は `LAYOUT_WARN_CAP` を超えた分を個別 event
    /// としてではなく単一の running `Truncated` counter に畳み込む —
    /// 「病的な入力で every field が毎回 clamp される」場合に buffer と
    /// 後段の eprintln! replay を有界にするための cap (doc 参照)。
    #[test]
    fn push_layout_warn_collapses_past_cap_into_truncated_counter() {
        let mut diag: Vec<LayoutWarn> = Vec::new();
        // cap ちょうどまでは real event。
        for _ in 0..LAYOUT_WARN_CAP {
            push_layout_warn(
                &mut diag,
                LayoutWarn::NonFiniteClamped {
                    site: "test",
                    raw: f32::NAN,
                    clamped: 0.0,
                },
            );
        }
        assert_eq!(diag.len(), LAYOUT_WARN_CAP);
        assert!(
            diag.iter()
                .all(|w| matches!(w, LayoutWarn::NonFiniteClamped { .. })),
            "cap 以内は real event のみのはず: {diag:?}"
        );

        // cap を超えた分は Vec を伸ばさず、末尾の Truncated counter に集約される。
        for _ in 0..5 {
            push_layout_warn(
                &mut diag,
                LayoutWarn::NonFiniteClamped {
                    site: "test",
                    raw: f32::INFINITY,
                    clamped: MAX_TAFFY_MAGNITUDE,
                },
            );
        }
        assert_eq!(
            diag.len(),
            LAYOUT_WARN_CAP + 1,
            "cap 超過分は Vec を伸ばさず Truncated に畳み込まれること: {diag:?}"
        );
        assert!(matches!(
            diag.last(),
            Some(LayoutWarn::Truncated { suppressed: 5 })
        ));
    }

    /// `LayoutWarn` の `Display` が両 variant で人間可読な文字列を出す
    /// ことの pin (`crate::diag::emit_warn_via` の `eprintln!` fallback が // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 実際に読める行になることの保証)。
    #[test]
    fn layout_warn_display_is_human_readable() {
        let clamped = LayoutWarn::NonFiniteClamped {
            site: "font-size",
            raw: f32::NAN,
            clamped: 0.0,
        };
        assert_eq!(
            clamped.to_string(),
            "font-size: clamped non-finite/out-of-range value NaN to 0"
        );
        let truncated = LayoutWarn::Truncated { suppressed: 7 };
        assert_eq!(
            truncated.to_string(),
            "7 additional layout clamp warning(s) suppressed (buffer cap reached)"
        );
    }

    /// clamp 定数が **doc が主張する帯の中にある**ことの pin。
    ///
    /// literal との `assert_eq!` は同語反復なので使わない — 定数を書き換えれば
    /// test も一緒に書き換わり、何も検出しない。doc が根拠として挙げた
    /// **関係式**を書く。
    #[test]
    fn clamp_limits_are_in_the_documented_range() {
        // taffy 幾何: CSSWG issue #4552 が報告する実装の LayoutUnit 上限帯
        // (1e7〜1e8 px) の中にあること。
        assert!(
            (1e7..=1e8).contains(&MAX_TAFFY_MAGNITUDE),
            "MAX_TAFFY_MAGNITUDE は CSSWG #4552 の 1e7..=1e8 px 帯に収まること: {MAX_TAFFY_MAGNITUDE}"
        );
        // doc はより強く「帯の**下端**を採る = 3 engine のいずれの上限より下」と
        // 主張している。最小は old-Edge の `2^31 / 100 ≈ 2.15e7 px`。
        assert!(
            MAX_TAFFY_MAGNITUDE <= (i32::MAX / 100) as f32,
            "MAX_TAFFY_MAGNITUDE は 3 engine の最小上限 (2^31/100 ≈ 2.15e7 px) 以下であること: {MAX_TAFFY_MAGNITUDE}"
        );
        // font-size: skrifa の 16.16 fixed 変換が saturate する
        // `i32::MAX / 64 ≈ 3.36e7` ppem より **1 桁以上**下 (doc の主張)。
        assert!(
            MAX_FONT_SIZE_PX * 10.0 < (i32::MAX / 64) as f32,
            "MAX_FONT_SIZE_PX は skrifa の saturation 点より 1 桁以上下であること: {MAX_FONT_SIZE_PX}"
        );
    }

    // ── 出力側 guard: nested percentage ──────────
    //
    // 入力側 guard (上の site 1-4) は bridge に入る f32 を有限化するが、
    // percentage は used value 層 (taffy) で containing block に対して解決され
    // nest ごとに複利するため、**出力**は非有限に戻りうる。以下はその出力側
    // guard (`sanitize_taffy_layout`) の pin。

    /// [`sanitize_taffy_layout`] が保証する invariant の述語 —
    /// [`taffy::Layout`] の全 f32 field が有限。
    ///
    /// paint が現に読む 4 field ではなく全 field を見る (guard 側と同じ理由)。
    ///
    /// `..` を使わず網羅 destructure するのも guard 側と同じ理由 — taffy が
    /// f32 field を増やしたときに guard 側 (網羅 literal) だけが compile error に
    /// なり、**述語側は黙って旧 field しか見ない**、という非対称を作らないため。
    fn layout_all_finite(l: &TaffyLayout) -> bool {
        fn size_ok(s: Size<f32>) -> bool {
            s.width.is_finite() && s.height.is_finite()
        }
        fn rect_ok(r: Rect<f32>) -> bool {
            r.left.is_finite() && r.right.is_finite() && r.top.is_finite() && r.bottom.is_finite()
        }
        let TaffyLayout {
            // `order` は u32 — guard 対象外 (`sanitize_taffy_layout` の doc)。
            order: _,
            location,
            size,
            scrollable_overflow_rect,
            scrollbar_size,
            border,
            padding,
            margin,
        } = l;
        location.x.is_finite()
            && location.y.is_finite()
            && size_ok(*size)
            && rect_ok(*scrollable_overflow_rect)
            && size_ok(*scrollbar_size)
            && rect_ok(*border)
            && rect_ok(*padding)
            && rect_ok(*margin)
    }

    /// `<html><body>` の下に `decl` を持つ `<div>` を `depth` 段 nest した
    /// document を [`layout_single_page`] に通し、**各段の**
    /// `unrounded_layout` を浅い順に返す。
    ///
    /// 起点は probe 材料の depth sweep harness だが、**depth ごとに document を作り直さない** —
    /// depth `N` の chain は 1..=`N` の各深さの node を既に含んでおり、
    /// probe が depth ごとに払っていた `FontContext::new()`
    /// (`font_context_new_cost_is_reasonable` が 10 回 5 秒未満を pin =
    /// 決して安くない) を depth 数だけ払う理由が無いため。
    fn nested_decl_layouts(decl: &str, depth: usize) -> Vec<TaffyLayout> {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let mut parent = body;
        let mut ids = Vec::with_capacity(depth);
        for _ in 0..depth {
            parent = doc.append_element(Some(parent), "div", Style::default(), Some(decl));
            ids.push(parent);
        }
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");
        ids.into_iter()
            .map(|i| doc.nodes[i].unrounded_layout)
            .collect()
    }

    /// 修正前は下記の depth で `unrounded_layout` が
    /// 非有限に戻っていた。probe 材料 RAWDATA.txt の depth sweep 実測では
    /// **base (guard 前) / head (入力側 guard 後) が完全に一致**していた =
    /// 入力側 guard では閉じない穴であることの証拠:
    ///
    /// | decl | probe harness | 本 harness (実測) |
    /// |---|---|---|
    /// | `width: 1e9%` | 6 | 6 |
    /// | `width: 100000%` | 12 | 12 |
    /// | `width: 10000%` | 18 | 18 |
    /// | `width: 1000%` | 36 | 36 |
    /// | `width: 200%` | 到達せず | 到達せず |
    /// | `padding-left: 1e9%` | 4 | 5 |
    /// | `padding-left: 100000%` | 8 | 9 |
    /// | `padding-left: 1000%` | 25 | 25 |
    /// | `padding-left: 200%` | 到達せず | 到達せず |
    ///
    /// (`padding-left` 系 2 行の ±1 は 2 harness の差に由来する。probe は
    /// depth ごとに document を作り直すので最深段が leaf になるが、本 harness
    /// は 1 本の chain を最深まで伸ばして各段を見るので同じ段が container に
    /// なる。**ただし機構は特定できていない** — この構造差が原因なら padding
    /// 系 3 行すべてがずれるはずだが `padding-left: 1000%` は 25/25 で一致
    /// する。数値自体は再現可能で、本 harness 列は `set_unrounded_layout` の
    /// `sanitize_taffy_layout` 呼び出しだけを外して実測した値である。
    /// `width` 系 4 行は完全一致。)
    ///
    /// 修正後はすべて「到達せず」になる。
    ///
    /// **検査幅 45 は表の sweep 範囲に揃えた値であって、保証の上限ではない。**
    /// 本 test が pin するのは「この 9 declaration を深さ 45 まで見た範囲で
    /// 保存値が全 field 有限」という**検査した点**だけである。深さ非依存性
    /// そのものは test からは出てこない — 根拠は
    /// `sanitize_taffy_layout` が taffy から arena への唯一の書き込み経路に
    /// 置かれているという **choke point の構造的議論**の側にある。
    /// `nested_percentage_output_stays_finite_far_past_the_sweep` も
    /// 「sweep よりかなり深い一例」を足すだけで、全称的な深さ非依存性を
    /// pin するものではない。したがってこの 45 を「安全な上限」として
    /// 下げないこと (下げてよい根拠は test ではなく構造の側にある)。
    #[test]
    fn nested_percentage_output_is_finite_through_probe_sweep_depth() {
        const SWEEP_DEPTH: usize = 45;
        for decl in [
            "width: 1e9%",
            "width: 100000%",
            "width: 10000%",
            "width: 1000%",
            "width: 200%",
            "padding-left: 1e9%",
            "padding-left: 100000%",
            "padding-left: 1000%",
            "padding-left: 200%",
        ] {
            let layouts = nested_decl_layouts(decl, SWEEP_DEPTH);
            assert_eq!(layouts.len(), SWEEP_DEPTH);
            if let Some((i, bad)) = layouts
                .iter()
                .enumerate()
                .find(|(_, l)| !layout_all_finite(l))
            {
                panic!(
                    "decl {decl:?}: nest depth {} の unrounded_layout に非有限 f32 が残っている: {bad:?}",
                    i + 1
                );
            }
        }
    }

    /// **深さ 96 でも保存値が有限**であることの pin。
    ///
    /// `nested_percentage_output_is_finite_through_probe_sweep_depth` は上
    /// の表に揃えた深さ 45 までしか見ないので、修正前に最も浅く破れた
    /// `padding-left: 1e9%` (probe harness で depth 4 / 本 harness で depth 5)
    /// を、その sweep 幅の 2 倍超で追加の 1 点として見る。
    ///
    /// **本 test は深さ非依存性を pin しない** — 有限深さの test が示せるのは
    /// 常に「検査した深さでは有限」までである。深さ非依存性の根拠は
    /// `sanitize_taffy_layout` が taffy から arena への唯一の書き込み経路に
    /// 置かれているという **choke point の構造的議論**であって、本 test では
    /// ない。本 test はその構造的議論に対する sanity check の位置づけ。
    ///
    /// **本 test は (a) / (b) 案を排除しない** (できない) — 入力側 fraction
    /// bound `F` に対する破綻深さ `35.6 / log10(F)`
    /// (`MAX_TAFFY_MAGNITUDE` の doc の表) は深さ 96 では `F >= 2.35` しか
    /// 捕まえられず、`width: 200%` を温存する最小の `F = 2.0` は `D = 118` で
    /// **本 test を通ってしまう**。有限深さの test は原理的に (a) を排除できない。
    /// (a) 却下の根拠は「深さ非依存には `F <= 1` が要り、それが `width: 200%` を
    /// 殺す」という `MAX_TAFFY_MAGNITUDE` の doc の議論であって、本 test ではない。
    #[test]
    fn nested_percentage_output_stays_finite_far_past_the_sweep() {
        const DEEP: usize = 96;
        let layouts = nested_decl_layouts("padding-left: 1e9%", DEEP);
        assert_eq!(layouts.len(), DEEP);
        for (i, l) in layouts.iter().enumerate() {
            assert!(
                layout_all_finite(l),
                "nest depth {} で非有限に戻った: {l:?}",
                i + 1
            );
        }
    }

    /// `sanitize_taffy_layout` の field 単位の挙動 (上の 2 test は「有限で
    /// ある」までしか見ないので、どの値に落ちるかはこちらで pin する)。
    #[test]
    fn sanitize_taffy_layout_clamps_every_f32_field() {
        let poisoned = TaffyLayout {
            order: 7,
            location: Point {
                x: f32::INFINITY,
                y: f32::NEG_INFINITY,
            },
            size: Size {
                width: f32::NAN,
                height: 1e30,
            },
            scrollable_overflow_rect: Rect {
                left: 3.0,
                top: 4.0,
                right: -1e30,
                bottom: f32::NAN,
            },
            scrollbar_size: Size {
                width: f32::INFINITY,
                height: 12.0,
            },
            border: Rect {
                left: f32::NAN,
                right: f32::INFINITY,
                top: f32::NEG_INFINITY,
                bottom: 1.0,
            },
            padding: Rect {
                left: 1e30,
                right: -1e30,
                top: f32::NAN,
                bottom: 2.0,
            },
            margin: Rect {
                left: f32::NEG_INFINITY,
                right: f32::INFINITY,
                top: -3.0,
                bottom: f32::NAN,
            },
        };
        let mut diag = Vec::new();
        let s = sanitize_taffy_layout(&poisoned, &mut diag);

        // `order` は u32 なので guard 対象外 — 素通しすること。
        assert_eq!(s.order, 7, "order は clamp 対象ではない");

        // 16 field が非有限/範囲外 (下の個別 assert が数える対象と一致): location
        // 2 + size 2 + scrollable_overflow_rect 2 + scrollbar_size 1 + border 3 + padding 3
        // + margin 3。範囲内の 4 field (scrollbar_size.height / border.bottom /
        // padding.bottom / margin.top) は積まれない (per-node spam を
        // 避ける設計)。
        assert_eq!(
            diag.len(),
            16,
            "clamp が実際に発火した field の数だけ LayoutWarn が積まれること: {diag:?}"
        );
        assert!(
            diag.iter().all(|w| matches!(
                w,
                LayoutWarn::NonFiniteClamped { site, .. }
                    if site.starts_with("layout.")
            )),
            "sanitize_taffy_layout 由来の event は全て layout.* site label を持つこと: {diag:?}"
        );

        assert_eq!(s.location.x, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.location.y, -MAX_TAFFY_MAGNITUDE);
        // NaN は clamp では潰れないので `is_nan()` → 0.0 (sanitize_finite)。
        assert_eq!(s.size.width, 0.0);
        assert_eq!(s.size.height, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.scrollable_overflow_rect.right, -MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.scrollable_overflow_rect.bottom, 0.0);
        assert_eq!(s.scrollbar_size.width, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.scrollbar_size.height, 12.0, "範囲内の値は素通し");
        assert_eq!(s.border.left, 0.0);
        assert_eq!(s.border.right, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.border.top, -MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.border.bottom, 1.0);
        assert_eq!(s.padding.left, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.padding.right, -MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.padding.top, 0.0);
        assert_eq!(s.padding.bottom, 2.0);
        assert_eq!(s.margin.left, -MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.margin.right, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.margin.top, -3.0);
        assert_eq!(s.margin.bottom, 0.0);

        // `[-MAX, MAX]` に収まる Layout の 1 例が bit 単位で不変であることの
        // pin。「通常 layout への影響ゼロ」を示すものではない — 影響が無いのは
        // 帯の内側に収まる場合だけで、外に出る入力 (`width: 200%` × 14 段 nest
        // など) では値が動く (`MAX_TAFFY_MAGNITUDE` の「clamp が実際に効く帯」節)。
        let benign = TaffyLayout {
            order: 3,
            location: Point { x: 10.0, y: -20.5 },
            size: Size {
                width: 793.7008,
                height: 1122.52,
            },
            scrollable_overflow_rect: Rect {
                left: 0.0,
                top: 0.0,
                right: 100.0,
                bottom: 200.0,
            },
            scrollbar_size: Size {
                width: 0.0,
                height: 0.0,
            },
            border: Rect {
                left: 1.0,
                right: 2.0,
                top: 3.0,
                bottom: 4.0,
            },
            padding: Rect {
                left: 5.0,
                right: 6.0,
                top: 7.0,
                bottom: 8.0,
            },
            margin: Rect {
                left: -9.0,
                right: 10.0,
                top: 11.0,
                bottom: 12.0,
            },
        };
        let mut diag = Vec::new();
        assert_eq!(sanitize_taffy_layout(&benign, &mut diag), benign);
        assert!(
            diag.is_empty(),
            "全 field が range 内なので LayoutWarn は積まれないこと: {diag:?}"
        );
    }

    // ── 意味的 invariant fallback ─────────────────
    //
    // 上の `sanitize_taffy_layout_*` test 群は「全 field が有限」までしか
    // 見ない (前段の scope)。以下は `enforce_layout_invariants` が扱う
    // 「field は有限だが親子関係が意味的に壊れている」層の pin。
    // `enforce_layout_invariants` の doc の 2 つの probe (border-box
    // padding overflow / 負 margin overflow) の数値もここで正式な
    // assertion に昇格させている。

    /// invariant 1 (content box 非負) の直接 pin。field 単位では
    /// `sanitize_taffy_layout` を素通りする値 (`size` も `padding` もどちらも
    /// `[-MAX, MAX]` 内) で `content_box_width() < 0.0` を作り、
    /// `enforce_layout_invariants` が (a) その node、(b) その **subtree 内の
    /// child** の両方をゼロ化すること、(c) `LayoutWarn` を積むことを確認する。
    /// (b) が無いと「親だけゼロ化して子は壊れた親を基準にした古い値のまま」
    /// という中途半端な状態になり、invariant 2 を新たに破ってしまう。
    #[test]
    fn content_box_violation_resets_subtree_to_zero_layout() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        // size.width (10.0) より padding.left+right (40.0) の方が大きい ⇒
        // content_box_width() = 10.0 - 40.0 = -30.0 < 0.0。size / padding
        // どちらも個別には `[-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE]` 内
        // なので `sanitize_taffy_layout` の field 単位 clamp はこれを止めない
        // — これが実際に起こりうることの直接的な再現。
        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 3,
            location: Point::ZERO,
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect {
                left: 20.0,
                right: 20.0,
                top: 0.0,
                bottom: 0.0,
            },
            margin: Rect::zero(),
        };
        // child 自体は (壊れた parent を無視すれば) 何の問題も無い layout —
        // subtree 全体がゼロ化されることを確認するための材料。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point { x: 1.0, y: 1.0 },
            size: Size {
                width: 2.0,
                height: 2.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        enforce_layout_invariants(&mut doc, parent);

        assert_eq!(
            doc.nodes[parent].unrounded_layout,
            TaffyLayout::with_order(3),
            "content box が負の node は order だけ残してゼロ化されること"
        );
        assert_eq!(
            doc.nodes[child].unrounded_layout,
            TaffyLayout::with_order(1),
            "破れた parent の subtree にいる child も (order だけ残して) \
             ゼロ化されること — 壊れた親を基準にした古い座標を残さない"
        );
        assert!(
            doc.layout_warnings.iter().any(|w| matches!(
                w,
                LayoutWarn::GeometryInvariantViolated {
                    invariant: "content_box_non_negative",
                    subtree_count: 1,
                }
            )),
            "content box invariant 違反が LayoutWarn として記録されること: {:?}",
            doc.layout_warnings
        );
    }

    /// **この変更で挙動が反転した直接 pin (旧名
    /// `saturated_child_outside_parent_resets_subtree_to_zero_layout`)**。
    /// `child_within_parent_border_box` の gate (「その axis 自身の
    /// `child.location` が飽和している」) を満たし、かつ旧実装なら
    /// containment 違反として reset されていたはずの、直接構築した
    /// maximally-非-contained な値 (`location.x == MAX_TAFFY_MAGNITUDE`
    /// に対し `parent.size.width == 100.0`) を使う。この変更で `axis_ok` が
    /// 符号を問わず無条件 `true` になったため、この fixture は — 実際には
    /// 明らかに parent border box の外にあるにもかかわらず — もう reset
    /// されない。「fixture を直接構築しても、もはやこの invariant を
    /// 破らせることはできない」ことを示す regression pin として残す
    /// (`child_within_parent_border_box` の doc「符号を問わず無条件
    /// accept になった理由」節、および将来「この check 自体を維持すべきか」
    /// を判断する follow-up (別途明示的に deferred とされた問題)
    /// が「現在の関数は実際に何をするか」を確認する材料として使うことを
    /// 想定している)。
    #[test]
    fn saturated_child_outside_parent_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
        let grandchild = doc.append_element(Some(child), "div", Style::default(), None::<&str>);

        // parent は正常 (100x100, 飽和していない)。
        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // child.location.x がちょうど飽和境界 (MAX_TAFFY_MAGNITUDE) —
        // parent (100x100) には到底収まらない。この変更以降、
        // この「明らかに収まっていない」事実はもう reset の理由にならない。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 2,
            location: Point {
                x: MAX_TAFFY_MAGNITUDE,
                y: 0.0,
            },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // grandchild は child を基準にした normal な値 — reset されて
        // いないことを subtree 全体で確認する材料 (下記 assert 参照)。
        doc.nodes[grandchild].unrounded_layout = TaffyLayout {
            order: 5,
            location: Point { x: 1.0, y: 1.0 },
            size: Size {
                width: 1.0,
                height: 1.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        let grandchild_before = doc.nodes[grandchild].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[parent].unrounded_layout.size,
            Size {
                width: 100.0,
                height: 100.0
            },
            "parent 自身は invariant を破っていないので手を付けないこと"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "child.location.x が飽和境界にちょうど達し、かつ実際に parent border box の外にあっても、現在の実装では符号を問わず無条件 accept なので reset されないこと"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[grandchild].unrounded_layout, grandchild_before,
            "child が reset されていない以上、その下の grandchild も一切変更されないこと"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// invariant 2 の gate が **無条件ではない**ことの pin — CSS が普通に
    /// 許す overflow (小さい parent + 負 margin で右/下/左にはみ出す child)
    /// を `layout_single_page` のフルパイプラインで実際に layout し、
    /// `enforce_layout_invariants` がそれを誤って fallback しないことを
    /// 確認する。数値は `enforce_layout_invariants` の doc に記録した
    /// 実測値と同じ (probe で先に確認済み)。
    ///
    /// これが無いと「invariant 2 を無条件チェックにしてしまう」regression
    /// (spec 違反 — legitimate な overflow layout を壊す) を検出できない。
    #[test]
    fn legitimate_negative_margin_overflow_is_not_reset() {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("width: 50px; height: 50px;"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("width: 200px; height: 200px; margin-left: -30px;"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let parent_layout = doc.nodes[parent].unrounded_layout;
        let child_layout = doc.nodes[child].unrounded_layout;
        assert_eq!(
            parent_layout.size,
            Size {
                width: 50.0,
                height: 50.0
            },
            "parent の正常な layout は不変であること"
        );
        assert_eq!(
            child_layout.location,
            Point { x: -30.0, y: 0.0 },
            "負 margin による legitimate overflow の location はゼロ化されないこと \
             (parent の border box に収まらないが、これは正しい CSS layout)"
        );
        assert_eq!(
            child_layout.size,
            Size {
                width: 200.0,
                height: 200.0
            },
            "負 margin による legitimate overflow の size はゼロ化されないこと"
        );
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "飽和していない legitimate overflow は invariant 2 の gate を \
             通らないので GeometryInvariantViolated は 1 件も積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// gate (`child.location` 自身が飽和) が真でも child が実際には parent
    /// に収まっている (境界ちょうど) ケースでは fallback しないことの pin。
    ///
    /// **この変更が入る前の history**: この test はもともと旧
    /// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
    /// (飽和 かつ containment 違反 → reset、この変更で
    /// `saturated_child_outside_parent_is_not_reset` に改名・反転) と
    /// 対にして、「gate 単独ではなく『gate かつ containment 違反』という
    /// conjunction を検査している」ことを示す pin だった。この変更で
    /// `axis_ok` が符号を問わず無条件 `true` になったため、この
    /// conjunction はもう成立しない — containment が実際にどうであっても
    /// (境界ちょうどで収まっていても、明らかに外れていても) reset は
    /// 起きない。本 test の assert 自体は (この fixture がたまたま
    /// 「収まっている」ケースだったため) 引き続き通るが、それは
    /// 「containment を検査して pass した」からではなく「そもそも
    /// containment を見ていない」から — その事実を示す対の regression pin
    /// は `saturated_child_outside_parent_is_not_reset` を参照。
    /// 実際の nested percentage chain を使った同種の pin は
    /// `nested_percentage_wide_child_chain_is_not_reset` を参照
    /// (extent ではなく origin だけを見る現行の `child_within_parent_border_box`
    /// を選んだ直接の理由になった regression)。
    #[test]
    fn saturated_but_contained_layout_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        // parent の size もちょうど飽和境界 — child の location がそこに
        // ぴったり収まる (境界値、`<=` で ok) ケースを作る。
        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: MAX_TAFFY_MAGNITUDE,
                height: MAX_TAFFY_MAGNITUDE,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // child.location 自身がちょうど飽和境界 — gate
        // (`child_within_parent_border_box` の axis 単位 gate) を発火させる。
        // parent.size と同じ値なので `<=` で境界ちょうど「収まっている」。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point {
                x: MAX_TAFFY_MAGNITUDE,
                y: MAX_TAFFY_MAGNITUDE,
            },
            size: Size {
                width: 0.0,
                height: 0.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        assert_eq!(
            doc.nodes[parent].unrounded_layout.size,
            Size {
                width: MAX_TAFFY_MAGNITUDE,
                height: MAX_TAFFY_MAGNITUDE
            },
            "飽和した parent 自身は content box invariant を破っていないので \
             手を付けないこと"
        );
        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "gate (child.location 自身の飽和) が真の child はゼロ化されないこと \
             — 現在の実装では、この fixture がたまたま境界ちょうど \
             で収まっているかどうかは無関係 (containment はもう見ていない)"
        );
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// **この変更で挙動が反転した pin (旧名
    /// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`,
    /// 旧主張「`y` 軸の検出力が保たれていること」)**。
    ///
    /// この変更が入る前は、この fixture (`y` 軸が飽和かつ実際に parent に
    /// 収まっていない = 真の violation、`x` 軸は飽和していない legitimate
    /// な負 margin `-30`) は `y` 軸の検出力を示す pin として意味があった —
    /// `y` 軸の再検査だけで reset の理由が説明でき、`x` 軸の legitimate な
    /// 負値は無視されることを示せた。この変更で `axis_ok` が符号を問わず
    /// 無条件 `true` になったため、`y` 軸は飽和しているだけでもう
    /// containment を再検査しない —「収まっていない」という事実自体が
    /// reset の理由になり得なくなった。
    ///
    /// 本 test は現在、`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
    /// とほぼ同じ主張 (どちらの axis も reset の理由にならない) の近縁 pin
    /// になっている。唯一の違いは `y` 軸の値 — こちらは `y` が **実際には
    /// parent に収まっていない** (`MAX_TAFFY_MAGNITUDE > 100.0`) のに対し、
    /// あちらは境界ちょうどで収まっている。両方とも reset されないことで、
    /// 「収まっているかどうか」が結果に一切影響しなくなったことを示す。
    #[test]
    fn saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // y 軸: child.location.y がちょうど飽和境界かつ実際に parent
        // (height=100.0) に収まっていない。x 軸: 通常の負 margin (`-30`)
        // による legitimate overflow — 飽和していない。この変更
        // 以降、どちらの軸も reset の理由にならない。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point {
                x: -30.0,
                y: MAX_TAFFY_MAGNITUDE,
            },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "y 軸 (飽和かつ実際には parent に収まっていない) も x 軸 (legitimate な負 margin、飽和していない) もどちらも現在の実装では reset の理由にならないので、child は一切変更されないこと"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// **直前の指摘への直接回帰 pin**
    /// — 直前の
    /// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
    /// (この変更が入る前の旧名
    /// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
    /// は当時「`y` 軸だけでも reset の説明がつく」ため、`x` 軸を実装が正しく
    /// 無視しているかどうかを実際には区別できない、と指摘された
    /// (旧 axis-mixing 実装でも当時の axis 単位実装でも同じ「reset される」
    /// という結果になってしまうため)。
    ///
    /// 本 test は区別できる fixture を使う: `y` 軸を「飽和境界にちょうど
    /// 達しているが、それでも parent に収まっている」(= 真の violation では
    /// ない) 値にし、`x` 軸には legitimate な負 margin (`-30`、飽和して
    /// いない) を与える。
    ///
    /// - **現行実装 (符号を問わず無条件 accept)**:
    ///   `x` 軸は飽和していないので無条件 ok。`y` 軸は飽和しているが、
    ///   containment を再検査せずやはり無条件 ok。→ **reset されない**。
    /// - **この変更が入る直前の実装 (axis 単位・符号で分岐、正方向だけ `<=` を
    ///   再検査)**: `x` 軸は無条件 ok、`y` 軸は飽和かつ正なので再検査するが
    ///   実際に parent に収まっているので ok。→ 同じく reset されない
    ///   (この test はこの変更の前後で結果が変わらない — 変わったのは
    ///   `saturated_child_outside_parent_is_not_reset` や
    ///   `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
    ///   のように実際に containment が破れているケース)。
    /// - **旧 axis-mixing 実装 (`parent.size` / `child.size` /
    ///   `child.location` のいずれかが飽和していれば `x` / `y` 両方を
    ///   無条件チェック)**: `y` の飽和で gate が開き、`x >= 0.0` の
    ///   チェックに `-30.0` が失敗する → **誤って reset される**。
    ///
    /// すなわち本 test が pass することは「`x` 軸の legitimate な負 margin
    /// が reset の原因になっていない」ことの直接証拠であり、旧 axis-mixing
    /// 実装への退行があれば本 test 単体で fail する。
    #[test]
    fn saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        // parent.size.height もちょうど飽和境界 — child.location.y
        // (同じく飽和境界) がそこにぴったり収まる (`<=`、境界ちょうど) ように
        // するため。width は通常値 (x 軸は飽和させないので関係ない)。
        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: 100.0,
                height: MAX_TAFFY_MAGNITUDE,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // x 軸: legitimate な負 margin (`-30`)、飽和していない — 無条件で
        // ok 扱いされるべき軸。y 軸: 飽和境界ちょうどだが、parent.size.height
        // と等しいので実際には収まっている (真の violation ではない) —
        // 「飽和している」だけでは reset の理由にならないことも同時に示す。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point {
                x: -30.0,
                y: MAX_TAFFY_MAGNITUDE,
            },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "x 軸 (legitimate な負 margin、飽和していない) も y 軸 (飽和\
             しているが実際には parent に収まっている) もどちらも reset の \
             理由にならないので、child は一切変更されないこと — これが \
             旧 axis-mixing 実装との直接の分岐点 (本 test の doc 参照)"
        );
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// **直前の指摘への直接回帰 pin (その 2)**
    /// — 指摘された元の scenario そのもの: 同じ subtree の**無関係な
    /// 別の場所** (ここでは同じ `parent` 自身) の `size` が飽和している状況で、
    /// **その child 自身は何も飽和していない**のに legitimate な負 margin
    /// (`location.x = -30`) を持つ。旧版の gate
    /// (`parent.size` / `child.size` / `child.location` のいずれか 1 つでも
    /// 飽和していれば検査する) はこれを誤って reset していた —
    /// `child_within_parent_border_box` の doc「gate を axis 単位・
    /// `child.location` 自身に限定する理由」節の 1. で説明した field 単位の
    /// 問題を直接再現する。
    #[test]
    fn saturated_parent_size_with_legitimate_negative_margin_child_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        // parent.size が飽和 — 無関係な原因 (例えば別の subtree で起きた
        // percentage 複利) を想定した、この child とは関係の無い飽和。
        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: MAX_TAFFY_MAGNITUDE,
                height: MAX_TAFFY_MAGNITUDE,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // child 自身は完全に正常 — location / size とも飽和していない、
        // 通常の負 margin による legitimate overflow
        // (`legitimate_negative_margin_overflow_is_not_reset` と同じ形)。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point { x: -30.0, y: 0.0 },
            size: Size {
                width: 200.0,
                height: 200.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "child 自身は何も飽和していないので、無関係な parent.size の飽和を \
             理由に legitimate な負 margin を reset してはならない — これが \
             以前指摘された finding そのもの"
        );
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// Regression pin for a false positive found while implementing this
    /// task: `width: 200%` nested `DEPTH`-ish levels deep is
    /// **legitimate** CSS (each level is, by design, twice its parent — the
    /// child's origin never moves off `(0, 0)`), yet an earlier version of
    /// `child_within_parent_border_box` checked *extent*
    /// (`location + size <= parent.size`) rather than just `location`, so it
    /// flagged the transition depth where one level's saturated `size` first
    /// exceeded its still-unsaturated parent's `size` — even though nothing
    /// about the relationship (child = 2x parent) had changed, only the
    /// absolute magnitude crossed `MAX_TAFFY_MAGNITUDE`. That cascaded
    /// through `zero_layout_subtree` and silently collapsed most of a
    /// legitimate deep chain to zero-size boxes.
    ///
    /// `saturated_but_contained_layout_is_not_reset` pins the equivalent
    /// minimal synthetic case (its doc records how a later change
    /// changed what that pin actually demonstrates); this test pins the
    /// same "not reset" outcome against the exact real-world shape that
    /// first surfaced the bug, so a future edit that reintroduces an
    /// extent-based check (or anything else that treats "child bigger than
    /// parent" as evidence of corruption) fails loudly here rather than
    /// only in the synthetic test.
    #[test]
    fn nested_percentage_wide_child_chain_is_not_reset() {
        const DEPTH: usize = 45;
        let layouts = nested_decl_layouts("width: 200%", DEPTH);
        assert_eq!(layouts.len(), DEPTH);

        let saturated_count = layouts
            .iter()
            .filter(|l| taffy_magnitude_is_saturated(l.size.width))
            .count();
        assert!(
            saturated_count > 0,
            "this test's premise (some depth saturates `size.width` in this \
             sweep range) no longer holds — re-verify against \
             MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this \
             test: {layouts:?}"
        );
        for (i, l) in layouts.iter().enumerate() {
            assert_ne!(
                *l,
                TaffyLayout::with_order(l.order),
                "nest depth {} was reset to a zero layout — a legitimate \
                 \"child is 2x parent at every depth\" declaration must \
                 survive `enforce_layout_invariants` even past the depth \
                 where `size` saturates, since the child's `location` never \
                 leaves the parent's border box: {l:?}",
                i + 1
            );
        }
    }

    /// Minimal synthetic pin for the new negative-
    /// saturation branch of `child_within_parent_border_box`, isolated
    /// from any real CSS pipeline (mirrors how
    /// `saturated_but_contained_layout_is_not_reset` pins the positive-
    /// saturation pass path). `child.location.x` is placed exactly at
    /// `-MAX_TAFFY_MAGNITUDE` against an ordinary, unsaturated
    /// `parent.size` — under the pre-fix predicate
    /// (`child.location >= 0.0 && child.location <= parent.size`) this is
    /// **unreachable as a pass**: no negative value ever satisfies `>= 0.0`,
    /// so the old code reset this unconditionally regardless of
    /// `parent.size`. This test pins that the sign-based branch introduced
    /// later (see `child_within_parent_border_box`'s
    /// doc, "符号を問わず無条件 accept になった理由") treats
    /// saturated-negative as unconditionally ok, the same way
    /// unsaturated-negative already was — and, since a later change,
    /// the same way saturated-positive now is too.
    #[test]
    fn saturated_negative_location_is_not_reset() {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
        let child = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);

        doc.nodes[parent].unrounded_layout = TaffyLayout {
            order: 0,
            location: Point::ZERO,
            size: Size {
                width: 100.0,
                height: 100.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };
        // child.location.x はちょうど飽和境界の**負**側 — parent.size は
        // 飽和していない通常値 (100.0)。旧式 (`>= 0.0 && <= parent.size`) は
        // 負の値に対しては恒等的に false だったので、この fixture は
        // 旧実装では必ず reset される (`>= 0.0` を満たす負数は存在しない)。
        doc.nodes[child].unrounded_layout = TaffyLayout {
            order: 1,
            location: Point {
                x: -MAX_TAFFY_MAGNITUDE,
                y: 0.0,
            },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
            scrollable_overflow_rect: Rect::ZERO,
            scrollbar_size: Size::zero(),
            border: Rect::zero(),
            padding: Rect::zero(),
            margin: Rect::zero(),
        };

        let child_before = doc.nodes[child].unrounded_layout;
        enforce_layout_invariants(&mut doc, parent);

        // これ以降の assert メッセージはあえて 1 物理行で書く (この file の
        // 他 test の慣習である backslash 継続の複数行ではない) —
        // `scripts/lib/patch_coverage.py` の `code_only()` は行ごとに
        // string 状態をリセットするため、backslash 継続行の途中に (地の文
        // としての) `)`/`]`/`}` があると `cov:ignore` の block scope 計算が
        // そこで途切れ、後続行が exempt されず patch coverage が誤って
        // FAIL する (実際に踏んだ経験がある)。1 行に畳むのは
        // その回避策であり、単なる style の揺れではない。
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            doc.nodes[child].unrounded_layout, child_before,
            "child.location.x が飽和境界にちょうど達していても、負である限り MAX_TAFFY_MAGNITUDE の doc が言う implementation-specific limit に達しただけで破綻の証拠にはならない — reset されないこと"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// Real CSS pipeline regression pin, and the
    /// **discriminating fixture** between the sign-based fix and a
    /// considered-and-rejected alternative ("skip re-validation whenever
    /// `parent.size` on that axis is also saturated, regardless of sign").
    ///
    /// A *single* `margin-left: -1e9%` declaration (no nesting) on a child
    /// of a plain `width: 100px` parent is enough: `sanitize_taffy` clamps
    /// the *fraction* (`-1e9%` → `-1e7` after `/100.0`), taffy then resolves
    /// that fraction against the parent's 100px containing block
    /// (`-1e7 * 100 = -1e9`), and `sanitize_taffy_layout` clamps the
    /// resulting raw `location.x` to exactly `-MAX_TAFFY_MAGNITUDE` on
    /// write. `parent.size.width` stays `100.0` — nowhere near saturated.
    ///
    /// The rejected "parent-also-saturated" alternative would still reset
    /// this case (parent isn't saturated on this axis), so it does not
    /// close this gap. Only a rule keyed on the
    /// *child's own sign* (this fix) accepts it, which is why this test is
    /// pinned independently of
    /// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
    /// below (that one's parent *does* happen to be saturated too, so on
    /// its own it could not rule out the rejected alternative).
    #[test]
    fn saturated_negative_margin_percentage_child_is_not_reset() {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("width: 100px; height: 100px;"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("width: 10px; height: 10px; margin-left: -1e9%;"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let parent_layout = doc.nodes[parent].unrounded_layout;
        let child_layout = doc.nodes[child].unrounded_layout;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parent_layout.size,
            Size {
                width: 100.0,
                height: 100.0
            },
            "this test's premise (parent stays unsaturated) no longer holds — re-verify before trusting the rest of this test"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            taffy_magnitude_is_saturated(child_layout.location.x) && child_layout.location.x < 0.0,
            "this test's premise (margin-left: -1e9% saturates child.location.x negative) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {child_layout:?}"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_ne!(
            child_layout,
            TaffyLayout::with_order(child_layout.order),
            "a single extreme-but-spec-valid negative percentage margin (no nesting needed) must not reset the subtree merely because it saturates the location — the parent here is not saturated, so a 'skip when parent is also saturated' rule would not have fixed this: {child_layout:?}"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// **Positive-direction analog of
    /// `saturated_negative_margin_percentage_child_is_not_reset`**, real
    /// CSS pipeline regression pin for the decision to extend
    /// the negative-side unconditional-accept treatment symmetrically to
    /// the positive side.
    ///
    /// A *single* `margin-left: 1e9%` declaration (no nesting) on a child of
    /// a plain `width: 100px` parent is enough: `sanitize_taffy` clamps the
    /// *fraction* (`1e9%` → `1e7` after `/100.0`), taffy then resolves that
    /// fraction against the parent's 100px containing block
    /// (`1e7 * 100 = 1e9`), and `sanitize_taffy_layout` clamps the resulting
    /// raw `location.x` to exactly `MAX_TAFFY_MAGNITUDE` on write.
    /// `parent.size.width` stays `100.0` — nowhere near saturated. Before
    /// this fix, `axis_ok` re-checked `location.x <=
    /// parent.size.width` for saturated positive locations
    /// (`1e7 <= 100.0` → false) and reset the whole child subtree to a zero
    /// layout, discarding spec-legal content — this is the exact repro that
    /// motivated the decision (independently reproduced
    /// in a throwaway worktree before the decision was recorded). This test
    /// pins that the fix actually closes the gap through the real
    /// cascade+taffy pipeline, not just at the unit level
    /// (`saturated_but_contained_layout_is_not_reset` and
    /// `saturated_child_outside_parent_is_not_reset` build `TaffyLayout`
    /// literals directly and don't exercise cascade/taffy at all).
    #[test]
    fn saturated_positive_margin_percentage_child_is_not_reset() {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("width: 100px; height: 100px;"),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("width: 10px; height: 10px; margin-left: 1e9%;"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");
        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let parent_layout = doc.nodes[parent].unrounded_layout;
        let child_layout = doc.nodes[child].unrounded_layout;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parent_layout.size,
            Size {
                width: 100.0,
                height: 100.0
            },
            "this test's premise (parent stays unsaturated) no longer holds — re-verify before trusting the rest of this test"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            taffy_magnitude_is_saturated(child_layout.location.x) && child_layout.location.x > 0.0,
            "this test's premise (margin-left: 1e9% saturates child.location.x positive) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {child_layout:?}"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_ne!(
            child_layout,
            TaffyLayout::with_order(child_layout.order),
            "a single extreme-but-spec-valid positive percentage margin (no nesting needed) must not reset the subtree merely because it saturates the location — this fix extends the negative-side unconditional-accept treatment symmetrically to this positive case: {child_layout:?}"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            doc.layout_warnings
                .iter()
                .all(|w| !matches!(w, LayoutWarn::GeometryInvariantViolated { .. })),
            "reset が起きていないので GeometryInvariantViolated は積まれないこと: {:?}",
            doc.layout_warnings
        );
    }

    /// Negative-direction analog of
    /// `nested_percentage_wide_child_chain_is_not_reset`. `width: 200%`
    /// alone never moves the child's origin off `(0, 0)` (that test's own
    /// premise), so it can't exercise invariant 2's location check at all.
    /// Adding `margin-left: -100%` at every depth does: since each level's
    /// own width is `2x` its containing block, and `margin-left: -100%`
    /// resolves against that same (growing) containing block, the relation
    /// `child.location.x == -parent.size.width` holds at *every* depth —
    /// legitimate, consistent, and unrelated to corruption — yet it pushes
    /// `location.x` negative fast enough to saturate several levels before
    /// `size.width` does (verified empirically: depth 15 saturates
    /// `location.x` here, one level after `size.width` alone saturates at
    /// depth 14 for plain `width: 200%`). Before this fix, every depth past
    /// the first saturated one collapsed to a zero layout.
    #[test]
    fn deep_nested_negative_percentage_margin_saturating_location_is_not_reset() {
        const DEPTH: usize = 45;
        let layouts = nested_decl_layouts("width: 200%; margin-left: -100%", DEPTH);
        assert_eq!(layouts.len(), DEPTH);

        let saturated_negative_count = layouts
            .iter()
            .filter(|l| taffy_magnitude_is_saturated(l.location.x) && l.location.x < 0.0)
            .count();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            saturated_negative_count > 0,
            "this test's premise (some depth saturates location.x negative in this sweep range) no longer holds — re-verify against MAX_TAFFY_MAGNITUDE's doc before trusting the rest of this test: {layouts:?}"
        );
        for (i, l) in layouts.iter().enumerate() {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_ne!(
                *l,
                TaffyLayout::with_order(l.order),
                "nest depth {} was reset to a zero layout — a legitimate \
                 \"child sits exactly one containing-block-width to the \
                 left of its parent at every depth\" declaration must \
                 survive `enforce_layout_invariants` even past the depth \
                 where `location.x` saturates negative: {l:?}",
                i + 1
            );
        }
    }

    // ── bridge_grid (CSS Grid Layout Module Level 1) ────────────────────

    use raikiri_style::property::{GridTemplateAreaEntry, GridTemplateAreas};
    use raikiri_style::{ComputedGridTrackList, ComputedGridTrackRepeat};

    #[test]
    fn bridge_grid_maps_grid_auto_flow_keywords() {
        for (flow, expected) in [
            (GridAutoFlowValue::Row, TaffyGridAutoFlow::Row),
            (GridAutoFlowValue::Column, TaffyGridAutoFlow::Column),
            (GridAutoFlowValue::RowDense, TaffyGridAutoFlow::RowDense),
            (
                GridAutoFlowValue::ColumnDense,
                TaffyGridAutoFlow::ColumnDense,
            ),
        ] {
            let mut cv = ComputedValues::initial();
            cv.grid_auto_flow = flow;
            let mut style = Style::default();
            let mut diag = Vec::new();
            bridge_grid(&mut style, &cv, &mut diag);
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(style.grid_auto_flow, expected, "grid-auto-flow: {flow:?}");
        }
    }

    #[test]
    fn bridge_grid_maps_every_grid_line_placement_variant() {
        for (value, expected) in [
            (GridLineValue::Auto, GridPlacement::Auto),
            (GridLineValue::Line(-2), taffy_style_helpers::line(-2i16)),
            (
                GridLineValue::Named("col".into()),
                GridPlacement::NamedLine("col".to_string(), 1),
            ),
            (
                GridLineValue::NamedLine("col".into(), 3),
                GridPlacement::NamedLine("col".to_string(), 3),
            ),
            (GridLineValue::Span(4), GridPlacement::Span(4)),
            (
                GridLineValue::SpanNamed("col".into(), 2),
                GridPlacement::NamedSpan("col".to_string(), 2),
            ),
        ] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                grid_line_value_to_taffy_placement(&value),
                expected,
                "grid-line: {value:?}"
            );
        }
    }

    #[test]
    fn bridge_grid_maps_grid_row_and_grid_column_from_the_4_longhands() {
        let mut cv = ComputedValues::initial();
        cv.grid_row_start = GridLineValue::Line(2);
        cv.grid_row_end = GridLineValue::Span(3);
        cv.grid_column_start = GridLineValue::Named("main".into());
        cv.grid_column_end = GridLineValue::Auto;
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        assert_eq!(
            style.grid_row,
            TaffyLine {
                start: taffy_style_helpers::line(2i16),
                end: GridPlacement::Span(3),
            }
        );
        assert_eq!(
            style.grid_column,
            TaffyLine {
                start: GridPlacement::NamedLine("main".to_string(), 1),
                end: GridPlacement::Auto,
            }
        );
    }

    #[test]
    fn bridge_grid_maps_track_list_with_repeat_minmax_fr_and_named_lines() {
        // `grid-template-columns: [a] 100px repeat(2, [b] minmax(0, 1fr))`
        // — exercises a bare fixed track, a `repeat()` with `minmax()`/`fr`
        // inside, and named lines both outside and inside the `repeat()`.
        let mut cv = ComputedValues::initial();
        cv.grid_template_columns =
            ComputedGridTemplateTracks::List(std::sync::Arc::new(ComputedGridTrackList {
                line_names: vec![vec!["a".into()], vec![], vec![]],
                components: vec![
                    ComputedGridTrackListComponent::Size(ComputedGridTrackSize::Breadth(
                        ComputedGridTrackBreadth::Px(100.0),
                    )),
                    ComputedGridTrackListComponent::Repeat(ComputedGridTrackRepeat {
                        count: GridRepeatCount::Count(2),
                        line_names: vec![vec!["b".into()], vec![]],
                        tracks: vec![ComputedGridTrackSize::MinMax(
                            ComputedGridTrackBreadth::Px(0.0),
                            ComputedGridTrackBreadth::Flex(1.0),
                        )],
                    }),
                ],
            }));
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);

        assert_eq!(style.grid_template_columns.len(), 2);
        assert_eq!(
            style.grid_template_columns[0],
            GridTemplateComponent::Single(TrackSizingFunction {
                min: MinTrackSizingFunction::length(100.0),
                max: MaxTrackSizingFunction::length(100.0),
            })
        );
        assert_eq!(
            style.grid_template_columns[1],
            GridTemplateComponent::Repeat(GridTemplateRepetition {
                count: TaffyRepetitionCount::Count(2),
                tracks: vec![TrackSizingFunction {
                    min: MinTrackSizingFunction::length(0.0),
                    max: MaxTrackSizingFunction::fr(1.0),
                }],
                line_names: vec![vec!["b".to_string()], vec![]],
            })
        );
        assert_eq!(
            style.grid_template_column_names,
            vec![vec!["a".to_string()], vec![], vec![]]
        );
    }

    #[test]
    fn bridge_grid_maps_grid_template_areas() {
        let mut cv = ComputedValues::initial();
        cv.grid_template_areas =
            GridTemplateAreasValue::Areas(std::sync::Arc::new(GridTemplateAreas {
                row_strings: vec!["header".into()],
                areas: vec![GridTemplateAreaEntry {
                    name: "header".into(),
                    row_start: 1,
                    row_end: 2,
                    column_start: 1,
                    column_end: 3,
                }],
                row_count: 1,
                column_count: 2,
            }));
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        assert_eq!(
            style.grid_template_areas,
            Some(taffy::style::GridTemplateAreas {
                areas: vec![TaffyGridTemplateArea {
                    name: "header".to_string(),
                    row_start: 1,
                    row_end: 2,
                    column_start: 1,
                    column_end: 3,
                }],
                row_count: 1,
                column_count: 2,
            })
        );
    }

    #[test]
    fn bridge_grid_maps_grid_auto_columns_and_rows_track_sizes() {
        let mut cv = ComputedValues::initial();
        cv.grid_auto_columns = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::Px(50.0),
        )]);
        cv.grid_auto_rows = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::MinContent,
        )]);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        assert_eq!(
            style.grid_auto_columns,
            vec![TrackSizingFunction {
                min: MinTrackSizingFunction::length(50.0),
                max: MaxTrackSizingFunction::length(50.0),
            }]
        );
        assert_eq!(
            style.grid_auto_rows,
            vec![TrackSizingFunction {
                min: MinTrackSizingFunction::min_content(),
                max: MaxTrackSizingFunction::min_content(),
            }]
        );
    }

    #[test]
    fn bridge_grid_maps_grid_auto_columns_percent_and_max_content_breadth() {
        // Sibling of `bridge_grid_maps_grid_auto_columns_and_rows_track_sizes`
        // above, which only exercises `Px`/`MinContent` — `Percent` and
        // `MaxContent` are the 2 `ComputedGridTrackBreadth` variants that
        // test left unreached in both `grid_track_breadth_to_taffy_min` and
        // `grid_track_breadth_to_taffy_max` (`Flex`/`Auto` are covered
        // elsewhere by the repeat/minmax and named-line tests).
        let mut cv = ComputedValues::initial();
        cv.grid_auto_columns = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::Percent(25.0),
        )]);
        cv.grid_auto_rows = std::sync::Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::MaxContent,
        )]);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        assert_eq!(
            style.grid_auto_columns,
            vec![TrackSizingFunction {
                min: MinTrackSizingFunction::percent(0.25),
                max: MaxTrackSizingFunction::percent(0.25),
            }]
        );
        assert_eq!(
            style.grid_auto_rows,
            vec![TrackSizingFunction {
                min: MinTrackSizingFunction::max_content(),
                max: MaxTrackSizingFunction::max_content(),
            }]
        );
    }

    #[test]
    fn bridge_grid_maps_grid_auto_columns_fit_content() {
        // `fit-content(<length-percentage>)` (CSS Grid 1 §7.2.1) — no other
        // `bridge_grid` test constructs `ComputedGridTrackSize::FitContent`,
        // so `grid_track_size_to_taffy`'s `FitContent` arm (both the px and
        // percentage forms) was previously unreached by any test.
        let mut cv = ComputedValues::initial();
        cv.grid_auto_columns = std::sync::Arc::new(vec![
            ComputedGridTrackSize::FitContent(ComputedLengthPercentage::Px(80.0)),
            ComputedGridTrackSize::FitContent(ComputedLengthPercentage::Percent(40.0)),
        ]);
        let mut style = Style::default();
        let mut diag = Vec::new();
        bridge_grid(&mut style, &cv, &mut diag);
        assert_eq!(
            style.grid_auto_columns,
            vec![
                TrackSizingFunction {
                    min: MinTrackSizingFunction::auto(),
                    max: MaxTrackSizingFunction::fit_content_px(80.0),
                },
                TrackSizingFunction {
                    min: MinTrackSizingFunction::auto(),
                    max: MaxTrackSizingFunction::fit_content_percent(0.4),
                },
            ]
        );
    }

    #[test]
    fn grid_repeat_count_to_taffy_maps_auto_fill_and_auto_fit() {
        // `repeat(auto-fill, ...)` / `repeat(auto-fit, ...)` (CSS Grid 1
        // §7.2.3.2) — the other `bridge_grid` track-list tests only
        // exercise `GridRepeatCount::Count(_)`, so the two auto-repeat
        // variants had no direct coverage.
        assert_eq!(
            grid_repeat_count_to_taffy(GridRepeatCount::AutoFill),
            TaffyRepetitionCount::AutoFill
        );
        assert_eq!(
            grid_repeat_count_to_taffy(GridRepeatCount::AutoFit),
            TaffyRepetitionCount::AutoFit
        );
    }

    #[test]
    fn bridge_alignment_maps_justify_items_and_justify_self() {
        let mut cv = ComputedValues::initial();
        cv.justify_items = SelfAlignmentValue::Center;
        cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::End);
        let mut style = Style::default();
        bridge_alignment(&mut style, &cv);
        assert_eq!(style.justify_items, Some(TaffyAlignItems::CENTER));
        assert_eq!(style.justify_self, Some(TaffyAlignItems::END));

        // `justify-self: normal` behaves as `stretch` (CSS Box Alignment 3
        // §8.3), not as `None` (which would mean "inherit justify-items")
        // — same special-case as `align-self: normal`
        // (`self_alignment_or_auto_to_taffy` doc).
        let mut cv = ComputedValues::initial();
        cv.justify_self = AlignSelfValue::Value(SelfAlignmentValue::Normal);
        let mut style = Style::default();
        bridge_alignment(&mut style, &cv);
        assert_eq!(style.justify_self, Some(TaffyAlignItems::STRETCH));
    }

    #[test]
    fn grid_template_columns_actually_sizes_columns_through_taffy_grid_algorithm() {
        // `bridge_grid`'s `grid_template_columns` field must actually reach
        // taffy's `compute_grid_layout` — asserting the `taffy::Style`
        // field alone would only prove the assignment, not the wiring
        // (same rationale as `flex_direction_column_stacks_children_vertically`
        // above). A 2-column `100px 200px` grid with one child explicitly
        // placed in each column must position the second child 100px to
        // the right of the first.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let grid_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-columns:100px 200px"),
        );
        let cell_a = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("grid-column:1;grid-row:1;height:20px"),
        );
        let cell_b = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("grid-column:2;grid-row:1;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[cell_a].unrounded_layout.location;
        let b_loc = doc.nodes[cell_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
            "second grid cell should sit 100px (first column's width) to \
             the right of the first, got a.x={}, b.x={}",
            a_loc.x,
            b_loc.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.y - b_loc.y).abs() < 0.5,
            "both cells share row 1, so they must share the same y \
             offset, got a.y={}, b.y={}",
            a_loc.y,
            b_loc.y
        );
    }

    #[test]
    fn grid_auto_flow_column_places_implicit_items_column_wise_through_taffy() {
        // `bridge_grid`'s `grid_auto_flow` field reaching taffy's
        // auto-placement algorithm — with `grid-auto-flow: column` and no
        // explicit placement, 2 children must be auto-placed into
        // successive *columns* of the same row (not successive rows, the
        // `row` default), which the 100px `grid-auto-columns` track makes
        // observable as a 100px x-offset between them.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let grid_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-auto-flow:column;grid-auto-columns:100px"),
        );
        let cell_a = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("height:20px"),
        );
        let cell_b = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let a_loc = doc.nodes[cell_a].unrounded_layout.location;
        let b_loc = doc.nodes[cell_b].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (b_loc.x - a_loc.x - 100.0).abs() < 0.5,
            "column-flow auto-placement should put the second item 100px \
             (grid-auto-columns) to the right of the first, got a.x={}, b.x={}",
            a_loc.x,
            b_loc.x
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (a_loc.y - b_loc.y).abs() < 0.5,
            "column-flow auto-placement should keep both items on row 1, \
             got a.y={}, b.y={}",
            a_loc.y,
            b_loc.y
        );
    }

    #[test]
    fn grid_template_columns_named_line_after_repeat_resolves_to_the_correct_taffy_grid_line() {
        // Regression test for the interleaving contract between
        // raikiri-style's `GridTrackList::line_names` (one entry per
        // `<line-names>?` production, i.e. `components.len() + 1` entries —
        // CSS Grid 1 §7.2.1 `<track-list>` grammar) and taffy's
        // `NamedLineResolver`, which advances an internal line counter once
        // per name-list entry and additionally steps across an unrolled
        // `repeat()`'s own tracks when it consumes one. A child placed with
        // `grid-column-start: z`, where `z` is named right after a
        // `repeat(2, [b] 50px)` block, must resolve to the grid line
        // following the 100px + 50px + 50px tracks that precede it — this
        // can only be told apart from an off-by-one in that bookkeeping by
        // checking where taffy's own grid algorithm actually places the
        // child, not by asserting on the bridged `taffy::Style` value.
        use raikiri_style::{build_rule_tree, cascade};
        use raikiri_traits::PageBox;

        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        let grid_container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-columns:[a] 100px repeat(2, [b] 50px) [z] 100px"),
        );
        let cell_first = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("grid-column:1;grid-row:1;height:20px"),
        );
        let cell_z = doc.append_element(
            Some(grid_container),
            "div",
            Style::default(),
            Some("grid-column:z;grid-row:1;height:20px"),
        );
        let rules = build_rule_tree(&doc);
        let cr = cascade(&doc, &rules).expect("cascade Ok");

        layout_single_page(&mut doc, &cr, PageBox::A4, FontContext::new()).expect("layout Ok");

        let first_loc = doc.nodes[cell_first].unrounded_layout.location;
        let z_loc = doc.nodes[cell_z].unrounded_layout.location;
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (z_loc.x - first_loc.x - 200.0).abs() < 0.5,
            "line `z` follows a 100px track and a repeat(2, 50px) block \
             (100px total), so it should sit 200px to the right of line 1, \
             got first.x={}, z.x={}",
            first_loc.x,
            z_loc.x
        );
    }
}
