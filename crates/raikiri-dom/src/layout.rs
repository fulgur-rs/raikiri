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
use raikiri_style::property::{BoxSizing as StyleBoxSizing, DisplayValue};
use raikiri_style::{
    CascadeResult, ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
    ComputedValues,
};
use raikiri_traits::{LayoutError, PageBox};
use taffy::{
    AvailableSpace, BoxSizing as TaffyBoxSizing, Dimension, Display, Layout as TaffyLayout,
    LengthPercentage, LengthPercentageAuto, NodeId as TaffyNodeId, Point, Rect, Size,
    compute_root_layout,
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
/// - [`bridge_margin`] — `Sides<ComputedLengthPercentageOrAuto>` → [`taffy::Rect<LengthPercentageAuto>`]
///   (raikiri-spike-j5rz)
/// - [`bridge_padding`] — `Sides<ComputedLengthPercentage>` → [`taffy::Rect<LengthPercentage>`]
///   (raikiri-spike-jbu0)
/// - [`bridge_size`] — [`ComputedLengthPercentageOrAuto`] `cv.width` / `cv.height` →
///   [`taffy::Style::size`] (`Size<Dimension>`)。Wave 2 (raikiri-spike-ggig) が
///   width 側、Wave 3 (raikiri-spike-01up) が height 側を追記し struct literal
///   1 発 assign に refactor。
/// - [`bridge_border`] — `Sides<ComputedBorder>` → [`taffy::Rect<LengthPercentage>`]
///   (raikiri-spike-q0uc Wave 2)。**border-style gating は本 bridge ではなく上流の
///   `raikiri_style::resolve_border` (computed 層) が持つ** — CSS Backgrounds 3
///   §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>、
///   bd raikiri-spike-zls8 で移動
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
///
/// (raikiri-spike-j5rz Sprint 18 Wave 1)
fn bridge_margin(style: &mut taffy::Style, cv: &ComputedValues) {
    let m = cv.margin;
    style.margin = Rect {
        top: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.top),
        right: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.right),
        bottom: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.bottom),
        left: computed_length_percentage_or_auto_to_taffy_length_percentage_auto(m.left),
    };
}

/// [`ComputedValues::padding`] (`Sides<ComputedLengthPercentage>`) → [`taffy::Style::padding`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// CSS Box 3 §6.1 <https://www.w3.org/TR/css-box-3/#padding-physical> の
/// physical padding 4 side (top / right / bottom / left) を taffy `Rect` に
/// **field 名 mapping** で write する (positional constructor は使わない —
/// `Sides` の field 順 `top,right,bottom,left` と `Rect` の field 順
/// `left,right,top,bottom` が異なるため silent transpose を防ぐ)。margin と
/// の差は value type: padding は `<length-percentage [0,∞]>` (auto なし、
/// non-negative は raikiri-style parse-time enforce) のため
/// [`computed_length_percentage_to_taffy_length_percentage`] を使う。
///
/// Length policy は [`computed_length_percentage_to_taffy_length_percentage`] を参照。
///
/// (raikiri-spike-jbu0 Sprint 18 Wave 2)
fn bridge_padding(style: &mut taffy::Style, cv: &ComputedValues) {
    let p = cv.padding;
    style.padding = Rect {
        top: computed_length_percentage_to_taffy_length_percentage(p.top),
        right: computed_length_percentage_to_taffy_length_percentage(p.right),
        bottom: computed_length_percentage_to_taffy_length_percentage(p.bottom),
        left: computed_length_percentage_to_taffy_length_percentage(p.left),
    };
}

/// [`ComputedValues::border`] (`Sides<ComputedBorder>`) → [`taffy::Style::border`]
/// (`Rect<LengthPercentage>`) bridge。
///
/// # style-gating は **上流** で済んでいる (bd raikiri-spike-zls8)
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
/// 一方だけ直す drift の温床になる)。Sprint 18 の `used_border_width` helper は
/// この理由で削除した。end-to-end の gating pin は本 file の
/// `apply_computed_to_style_bridges_border_to_taffy` が引き続き持つ。
///
/// **この「上流で済んでいる」は element 経路 (per-node cascade) の話である** —
/// `@page` 経路の `PageCascadeResult::declarations` には gate が無く非 gating の
/// border-width が出る (raikiri-style 側 `resolve_border` doc の caveat 参照)。
/// 本 bridge が読むのは per-node [`ComputedValues`] なので影響しないが、将来
/// page-margin box の layout を本 bridge に通すなら再確認が必要。
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
/// (raikiri-spike-q0uc Sprint 18 Wave 2、raikiri-spike-zls8 で computed 層へ移行)
fn bridge_border(style: &mut taffy::Style, cv: &ComputedValues) {
    let b = cv.border;
    style.border = Rect {
        top: computed_length_to_taffy_length_percentage(b.top.width),
        right: computed_length_to_taffy_length_percentage(b.right.width),
        bottom: computed_length_to_taffy_length_percentage(b.bottom.width),
        left: computed_length_to_taffy_length_percentage(b.left.width),
    };
}

/// [`ComputedValues::width`] / [`ComputedValues::height`] (`ComputedLengthPercentageOrAuto`) →
/// [`taffy::Style::size`] (`Size<Dimension>`) bridge (CSS Sizing 3 §3.1.1
/// "Preferred Size Properties"
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>)。
///
/// Wave 2 (raikiri-spike-ggig) が width 側を landing、Wave 3
/// (raikiri-spike-01up) が height 側を追記して両 preferred size 軸を full-bridge
/// にした。両 field を同時に書き込むため struct literal
/// (`style.size = Size { width, height }`) で 1 発 assign する — partial write
/// scaffold は Wave 3 で不要になった。
///
/// Length policy は [`computed_length_percentage_or_auto_to_taffy_dimension`] を参照。
///
/// # PageBox 妥協 (M1)
///
/// `<body>` element の `style.size` は本 bridge の後、[`apply_page_box_to_body`]
/// で PageBox の値 (width / height 両方) に上書きされる (layout.rs Step 1 →
/// Step 4)。したがって `<body style="width: 100px; height: 200px">` の author
/// 値は本 helper で一度 taffy に write されるが、Step 4 で PageBox 値に
/// clobber される — M1 期間中の意図された挙動 (M4 で @page cascade + per-page
/// PageBox に refactor 予定)。width 側 clobber の author→PageBox 上書き経路は
/// test `apply_page_box_clobbers_body_width_from_bridge` が pin する。height
/// 側は [`apply_page_box_to_body`] が `style.size = Size { width, height }` の
/// struct literal で **field を分岐なく一括代入する** ため、width と同じ
/// clobber 経路を通る (両 field は同一 statement で書かれる)。同 helper の
/// PageBox output pin は test `apply_page_box_to_body_sets_body_style_size_to_page_dimensions`
/// が担う (author→PageBox の bridge→clobber 連鎖 test は width 側で十分、
/// 冗長化を避け height 側は structural 保証に留める)。
fn bridge_size(style: &mut taffy::Style, cv: &ComputedValues) {
    // Wave 3 完了後: width + height 両方を同時に書くので struct literal を採用。
    // (Wave 2 の field-assign scaffold は Wave 3 マージまでの一時形態で、
    //  もはや保つ必要がない — default 保持は cv 側で `Auto` を返せば自然に達成。)
    style.size = Size {
        width: computed_length_percentage_or_auto_to_taffy_dimension(cv.width),
        height: computed_length_percentage_or_auto_to_taffy_dimension(cv.height),
    };
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

// ---------------------------------------------------------------------------
// 非有限 f32 の guard (bd raikiri-spike-2ui0、PMO 判断 2026-07-27)
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
/// これらは bd raikiri-spike-zls8 (decision raikiri-spike-082k Phase 2) が
/// 絶対化を cascade に入れるまで、`layout.rs` の
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
/// # 入力側 bound の射程と、出力側 guard による決着 (bd raikiri-spike-r8ew)
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
/// 浅い。padding は `location` / `content_size` の累積にも寄与するため。)
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
/// ±Inf / NaN を taffy の内部演算に入れない役割が残っており (bd
/// raikiri-spike-2ui0 の site 1-4 test が pin)、出力側 clamp は「arena に
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
/// (PMO 判断 2026-07-27 §4「site ごとに target context が違うので一律に
/// しない」)。font-size の target context は parley → skrifa の glyph scaler
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
/// bd raikiri-spike-2ui0 の security lens は下流 sink の帰結を「PLAUSIBLE、
/// 未 characterize」としていたが、本 guard の実装時に実測した:
/// `sanitize_finite` を恒等関数に差し替えて
/// `nonfinite_font_size_is_clamped_before_parley` を単独実行すると
/// **25 秒経っても終了しない**。すなわち非有限 font-size は parley の shaping を
/// 有界時間で終わらせない。
///
/// 対して site 1-4 の taffy 側 test は **本 test 入力では**即座に assert 失敗する
/// (値が壊れるだけ)。これは「taffy は非有限で hang しない」という一般命題では
/// ない — 測ったのは 5 本の入力だけである。taffy 内部の used value に対する
/// 挙動は bd raikiri-spike-r8ew / 下流 sink の characterize 課題を参照。
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
/// panic しない (`LayoutError` も返さない) — PMO 判断 2026-07-27 §5 のとおり
/// **clamp して続行**する。
///
/// # なぜ warn しないのか (silent clamp)
///
/// `log` / `tracing` は workspace に依存が無い (`grep` → 0 hit) が、**それが
/// 理由ではない** — 同一 crate の `fonts.rs` に dep 追加ゼロの診断機構が既に
/// ある (`FontWarn` enum + `FontWarnObserver = Option<&mut dyn FnMut(&FontWarn)>`
/// + `emit_warn`、bd raikiri-spike-1uq)。
///
/// 採らない理由は **observer を本 site まで通すと公開 signature に波及する**
/// から: `sanitize_*` は `bridge_*` → `apply_computed_to_style` →
/// `layout_single_page` の奥にあり、observer を渡すにはこの chain の signature を
/// 変えるか、per-node で裸の `eprintln!` を撒いて spam するかの二択になる。
/// `fonts.rs` 型の観測機構を本 site に導入するのは別 task の判断
/// (coordinator が起票予定、`wall/build` 壁予兆として declare 済)。
///
/// **残余リスク (declare)**: silent なので、将来 absolutize 側に本物の算術 bug
/// (例: 単位換算ミスで `1e9px`) が入ると、本 guard が 1e7 に吸収して
/// **「それらしい layout」として描画されてしまう** — NaN や破綻として可視化
/// されない。これは decision 082k が削除した fail-quiet arm
/// (`Em(_) => length(0.0)`) と**同じ class の残余リスク**であり、本 guard は
/// 値の病理を可用性と引き換えに隠している。
fn sanitize_finite(v: f32, min: f32, max: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(min, max) }
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
fn sanitize_taffy(v: f32) -> f32 {
    sanitize_finite(v, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
}

/// taffy が resolve した [`taffy::Layout`] の全 f32 field を
/// [`sanitize_taffy`] に通す **出力側** guard (bd raikiri-spike-r8ew)。
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
/// 現在 `padding` / `border` / `content_size` / `scrollbar_size` を読む
/// consumer は無い (grep 実測) が、将来 paint がこれらを使うときは
/// 非負性を仮定しないこと。
pub(crate) fn sanitize_taffy_layout(layout: &TaffyLayout) -> TaffyLayout {
    fn size(s: Size<f32>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width),
            height: sanitize_taffy(s.height),
        }
    }
    fn rect(r: Rect<f32>) -> Rect<f32> {
        Rect {
            left: sanitize_taffy(r.left),
            right: sanitize_taffy(r.right),
            top: sanitize_taffy(r.top),
            bottom: sanitize_taffy(r.bottom),
        }
    }
    TaffyLayout {
        order: layout.order,
        location: Point {
            x: sanitize_taffy(layout.location.x),
            y: sanitize_taffy(layout.location.y),
        },
        size: size(layout.size),
        content_size: size(layout.content_size),
        scrollbar_size: size(layout.scrollbar_size),
        border: rect(layout.border),
        padding: rect(layout.padding),
        margin: rect(layout.margin),
    }
}

/// [`ComputedLengthPercentage`] → [`taffy::LengthPercentage`] bridge
/// (padding 用)。
///
/// # 網羅 match (bd raikiri-spike-zls8 / decision raikiri-spike-082k)
///
/// 引数が **computed 層**の型になったため 2 arm で網羅する。Sprint 18 の
/// `Length::Em(_) | Length::Rem(_) => length(0.0)` (font-relative unit を黙って
/// 0px に潰す fail-quiet) と `_ => length(0.0)` (non_exhaustive catch-all) は
/// **削除した** — `em` / `rem` / `pt` は cascade の phase 3 で px に絶対化済み
/// であり、computed 層に到達しない。
///
/// **ただし削除した arm は「単位」だけでなく「病的な f32 の値」も吸収していた**
/// (`Em(inf)` / `0.0 * inf` = NaN)。その分は [`sanitize_taffy`] が
/// backfill している (bd raikiri-spike-2ui0) — **guard を「不要な防御」と
/// 判断して外さないこと。**
///
/// [`ComputedLengthPercentage`] に `#[non_exhaustive]` が付いていないのは、
/// この網羅性を今得るための explicit trade である (Epic 5 で `Calc` variant が
/// 増えるときに coordinated breaking change を払う。`raikiri_style::resolve`
/// の module doc 参照)。**`_` arm を足して「forward-compat」にしてはならない** —
/// trade の得る側を捨てることになる。
///
/// # Percent policy
///
/// `Percent(p)` → `percent(sanitize_taffy(p / 100.0))` — CSS spec の authored
/// 0-100 を taffy の fraction 0.0-1.0 に変換し、[`sanitize_taffy`] で有限化する
/// (bd raikiri-spike-2ui0)。containing block に対する解決は **used value 層**
/// (CSS Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>) であり
/// taffy に委譲する — **guard が bound するのは fraction であって解決後の
/// used value ではない** ([`MAX_TAFFY_MAGNITUDE`] の射程節を参照)。
fn computed_length_percentage_to_taffy_length_percentage(
    len: ComputedLengthPercentage,
) -> LengthPercentage {
    match len {
        // site 1 (bd raikiri-spike-2ui0): `sanitize_taffy` で非有限を落とす。
        ComputedLengthPercentage::Px(v) => LengthPercentage::length(sanitize_taffy(v)),
        ComputedLengthPercentage::Percent(p) => {
            LengthPercentage::percent(sanitize_taffy(p / 100.0))
        }
    }
}

/// [`ComputedLengthPercentageOrAuto`] → [`taffy::Dimension`] bridge
/// (width / height 用)。
///
/// 網羅 match / Percent policy / 非有限 guard は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同じ (3 arm、catch-all なし)。
/// `Auto` → `Dimension::auto()` (f32 を持たないので guard 対象外)。
///
/// [`bridge_size`] から width (raikiri-spike-ggig Wave 2) / height
/// (raikiri-spike-01up Wave 3) 両方で consume される。
fn computed_length_percentage_or_auto_to_taffy_dimension(
    loa: ComputedLengthPercentageOrAuto,
) -> Dimension {
    match loa {
        // site 2 (bd raikiri-spike-2ui0)。
        ComputedLengthPercentageOrAuto::Px(v) => Dimension::length(sanitize_taffy(v)),
        ComputedLengthPercentageOrAuto::Percent(p) => Dimension::percent(sanitize_taffy(p / 100.0)),
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
) -> LengthPercentageAuto {
    match loa {
        // site 3 (bd raikiri-spike-2ui0)。margin は負値が spec-valid なので
        // `sanitize_taffy` の対称 clamp が load-bearing。
        ComputedLengthPercentageOrAuto::Px(v) => LengthPercentageAuto::length(sanitize_taffy(v)),
        ComputedLengthPercentageOrAuto::Percent(p) => {
            LengthPercentageAuto::percent(sanitize_taffy(p / 100.0))
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
/// `<src>_to_taffy_<dst>` 命名 / 同じ cluster に置く (37n sibling convention)。
///
/// `border-*-width` 専用に分けているのは、grammar (`<line-width>` =
/// `<length [0,∞]> | thin | medium | thick`) が `<percentage>` を含まないため
/// computed 層でも length しか来ないから (CSS Backgrounds 3 §3.3 "Line
/// Thickness: the border-width properties"
/// <https://www.w3.org/TR/css-backgrounds-3/#border-width>)。戻り値型は
/// [`computed_length_percentage_to_taffy_length_percentage`] と同一
/// (`LengthPercentage` が taffy 側の最小共通型) だが、入力型が
/// [`ComputedLength`] なので percentage arm を
/// 持たない点が違う。非有限 guard ([`sanitize_taffy`]) は同じく通す
/// (bd raikiri-spike-2ui0)。
fn computed_length_to_taffy_length_percentage(len: ComputedLength) -> LengthPercentage {
    // site 4 (bd raikiri-spike-2ui0)。
    LengthPercentage::length(sanitize_taffy(len.px()))
}

/// 全 Text node を parley で pre-shape、結果を `Node.text_layout` に格納する。
///
/// 呼び出し側 (`layout_single_page`) は事前に全 `Node.text_layout = None` に
/// clear 済であることを前提とする (re-entrance safety)。
///
/// Font stack / size / weight は `cascade.computed[idx]` (親から inherit 済) を消費。
/// `max_advance` は行折り返し境界で、通常 `page_box.width`。
///
/// # 失敗しない (bd raikiri-spike-zls8)
///
/// 以前は `Result<(), LayoutError>` を返していた。唯一の `Err` 経路は
/// `cv.font_size` が specified 層の `Length` で `Px` 以外だった場合の
/// `LayoutError::Internal` だったが、`font_size` が [`ComputedLength`] (px) に
/// なって match 自体が消えたため到達不能になった。`pub(crate)` なので戻り値の
/// narrowing は crate 内で完結する (外部影響 0)。
///
/// [`ComputedLength`]: raikiri_style::ComputedLength
pub(crate) fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) {
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

        // bd raikiri-spike-zls8: `cv.font_size` は computed 層の
        // `ComputedLength` (px) になったので match も fallback も要らない。
        // Sprint 18 までは specified 層の `Length` を受けていたため
        // `LayoutError::Internal` を返す wildcard arm があったが、`em` / `rem` /
        // `pt` は cascade の phase 2 で絶対化されるようになり到達しない。
        //
        // site 5 (bd raikiri-spike-2ui0): ただし**値**は非有限になり得るので
        // parley に渡す直前で有限化する。下限 0.0 は grammar
        // `<length-percentage [0,∞]>` (CSS Fonts 4 §2.5) に一致。
        let font_size_px = sanitize_finite(cv.font_size.px(), 0.0, MAX_FONT_SIZE_PX);

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
///   parse は M1 非対応) / taffy internal
///
///   parley shape (`preshape_text`) は bd raikiri-spike-zls8 以降 **失敗しない**
///   (同関数の doc 参照)。
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
    );

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
        // Sides<ComputedLengthPercentageOrAuto> を taffy::Rect<LengthPercentageAuto>
        // に translate することを確認する regression pin。bridge の 3 分岐
        // (Px / Percent / Auto) をそれぞれ 1 case で covering。
        //
        // bd raikiri-spike-zls8: Case 4 の `pt` は bridge の分岐ではなくなった
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
        //   (`raikiri_style::resolve_length_percentage_or_auto`) が行う**
        //   (bd raikiri-spike-zls8)。bridge に届く時点で既に px。本 case は
        //   end-to-end の値を pin する。
        //   f32 bit-identical assert のため右辺を expression のまま書く
        //   (`13.333` literal は round-trip で drift する。上流も同じ
        //   `v * 4.0 / 3.0` の評価順を使う — `resolve::pt_to_px` の doc 参照)。
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
        // Sides<ComputedLengthPercentage> を taffy::Rect<LengthPercentage> に
        // translate することを確認する regression pin。padding は margin と違い
        // `auto` を持たない (<length-percentage [0,∞]>) ため bridge は **2 arm**
        // (Px / Percent) で網羅する。
        //
        // bd raikiri-spike-zls8: Case 3 の `pt` は **bridge の分岐ではなくなった**
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
        //   bridge ではない (bd raikiri-spike-zls8)。
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
        // が cv.width: ComputedLengthPercentageOrAuto を taffy::Style::size.width:
        // Dimension に translate することを pin する。bridge の 3 分岐
        // (Px / Percent / Auto) をそれぞれ 1 case で covering
        // (`pt` は cascade の phase 3 で px 化される — raikiri-spike-zls8)。
        //
        // Test 戦略: fixture は **非 body element** (この場合 `<p>`) を使う —
        // `<body>` は後段 `apply_page_box_to_body` で clobber されるため本 bridge
        // の効果は observable でない (別 test `apply_page_box_clobbers_body_width_from_bridge`
        // で clobber 挙動を pin)。inline style 経由なので raikiri-style の
        // parse_width path + ComputedLengthPercentageOrAuto encoding も同時に
        // regression pin。
        //
        // 本 test は width 軸に絞る — height 軸は sibling test
        // `apply_computed_to_style_bridges_height_to_taffy` (Wave 3、raikiri-spike-01up)
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
        //   **変換の所在は cascade の phase 3** で bridge ではない
        //   (bd raikiri-spike-zls8)。
        //   f32 bit-identical assert のため右辺を expression で書く。
        assert_eq!(
            width_for("width: 20pt"),
            Dimension::length(20.0 * 4.0 / 3.0)
        );
    }

    #[test]
    fn apply_computed_to_style_bridges_height_to_taffy() {
        // raikiri-spike-01up (Sprint 18 dom-4 Wave 3): bridge_size の height 側
        // 拡張。cv.height: ComputedLengthPercentageOrAuto を
        // taffy::Style::size.height: Dimension に translate することを pin する。
        // Wave 2 sibling test `apply_computed_to_style_bridges_width_to_taffy` と
        // 対を成し、Wave 3 の struct literal 化 (Size { width, height } の 1 発
        // assign) で height 側の 3 分岐 (Px / Percent / Auto) が意図通り
        // 書き込まれるか確認する (型名は bd raikiri-spike-zls8 で更新)。
        //
        // Test 戦略: fixture は **非 body element** (`<p>`) を使う — `<body>` は
        // 後段 `apply_page_box_to_body` で height も clobber されるため本 bridge
        // の効果は body 上で observable でない。inline style 経由で raikiri-style
        // の parse_height path + ComputedLengthPercentageOrAuto encoding も同時に
        // regression pin。
        //
        // Pt case は sibling width test が同じ
        // computed_length_percentage_or_auto_to_taffy_dimension policy を
        // pin しているため redundant (かつ pt → px 変換は bd
        // raikiri-spike-zls8 以降 cascade の phase 3 の責務)。ここでは height
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
        // raikiri-spike-q0uc (Sprint 18 dom-4 Wave 2): bridge_border が
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
        // the border style is none or hidden" と規定するため
        // (bd raikiri-spike-zls8 で used 層から computed 層へ移動、bridge 側の
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
        // become 0." は css-backgrounds-3 に存在しない文だったので差し替えた
        // (bd raikiri-spike-zls8 §8.2 spec lens SPEC-7)。
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
    fn font_relative_lengths_reach_taffy_as_real_pixels() {
        // bd raikiri-spike-zls8 (decision raikiri-spike-082k Phase 2)。
        //
        // Sprint 18 の bridge は specified 層の `Length` を受けており、
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

    // ── 非有限 f32 guard (bd raikiri-spike-2ui0、PMO 判断 2026-07-27) ────────
    //
    // untrusted author CSS から +Inf / NaN が taffy / parley に到達しないことを
    // **5 site すべて**で pin する。reproducer は 2ui0 の probe comment 由来。
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
        // zls8 以前は `Em(_) => length(0.0)` arm がこれを吸収していた。
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
                     する (bd raikiri-spike-2ui0)"
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

    // ── guard 関数そのものの unit test ───────────────────────────────────
    //
    // e2e test は site 5 が hang し得るうえ 1 本あたり FontContext 構築を伴う。
    // guard の算術は純関数なので直接叩く (数 ms、hang し得ない)。

    #[test]
    fn sanitize_finite_maps_nan_to_zero() {
        // `f32::clamp` は NaN を NaN のまま返すので、この分岐が無いと NaN が
        // 素通りする。
        assert_eq!(sanitize_finite(f32::NAN, -1.0, 1.0), 0.0);
        assert_eq!(sanitize_finite(f32::NAN, 0.0, MAX_FONT_SIZE_PX), 0.0);
    }

    #[test]
    fn sanitize_finite_clamps_infinities_to_bounds() {
        assert_eq!(
            sanitize_finite(f32::INFINITY, 0.0, MAX_FONT_SIZE_PX),
            MAX_FONT_SIZE_PX
        );
        assert_eq!(
            sanitize_finite(f32::NEG_INFINITY, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE),
            -MAX_TAFFY_MAGNITUDE
        );
        // 下限が 0.0 の site (font-size) では -Inf は 0.0 に落ちる。
        assert_eq!(
            sanitize_finite(f32::NEG_INFINITY, 0.0, MAX_FONT_SIZE_PX),
            0.0
        );
    }

    #[test]
    fn sanitize_taffy_clamps_out_of_range_finite_values() {
        // 有限でも範囲外なら寄せる (「有限化するだけ」ではない)。
        assert_eq!(sanitize_taffy(1e30), MAX_TAFFY_MAGNITUDE);
        assert_eq!(sanitize_taffy(-1e30), -MAX_TAFFY_MAGNITUDE);
    }

    #[test]
    fn sanitize_taffy_passes_through_in_range_values() {
        // 通常値は bit-identical に素通しする (VRT が pixel-exact である前提)。
        for v in [0.0_f32, 1.0, -1.0, 16.0, 793.7008, MAX_TAFFY_MAGNITUDE] {
            assert_eq!(
                sanitize_taffy(v),
                v,
                "in-range value must pass through: {v}"
            );
        }
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

    // ── 出力側 guard: nested percentage (bd raikiri-spike-r8ew) ──────────
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
            content_size,
            scrollbar_size,
            border,
            padding,
            margin,
        } = l;
        location.x.is_finite()
            && location.y.is_finite()
            && size_ok(*size)
            && size_ok(*content_size)
            && size_ok(*scrollbar_size)
            && rect_ok(*border)
            && rect_ok(*padding)
            && rect_ok(*margin)
    }

    /// `<html><body>` の下に `decl` を持つ `<div>` を `depth` 段 nest した
    /// document を [`layout_single_page`] に通し、**各段の**
    /// `unrounded_layout` を浅い順に返す。
    ///
    /// 起点は probe 材料の depth sweep harness
    /// (`docs/superpowers/specs/2026-07-28-r8ew-probe-materials/probe2.rs`、
    /// zls8 perf lens iter2) だが、**depth ごとに document を作り直さない** —
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

    /// 修正前 (bd raikiri-spike-r8ew) は下記の depth で `unrounded_layout` が
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
    /// [`sanitize_taffy_layout`] 呼び出しだけを外して実測した値である。
    /// `width` 系 4 行は完全一致。)
    ///
    /// 修正後はすべて「到達せず」になる。
    ///
    /// **検査幅 45 は表の sweep 範囲に揃えた値であって、保証の上限ではない。**
    /// 本 test が pin するのは「この 9 declaration を深さ 45 まで見た範囲で
    /// 保存値が全 field 有限」という**検査した点**だけである。深さ非依存性
    /// そのものは test からは出てこない — 根拠は
    /// [`sanitize_taffy_layout`] が taffy から arena への唯一の書き込み経路に
    /// 置かれているという **choke point の構造的議論**の側にある。
    /// [`nested_percentage_output_stays_finite_far_past_the_sweep`] も
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
    /// [`nested_percentage_output_is_finite_through_probe_sweep_depth`] は bd
    /// の表に揃えた深さ 45 までしか見ないので、修正前に最も浅く破れた
    /// `padding-left: 1e9%` (probe harness で depth 4 / 本 harness で depth 5)
    /// を、その sweep 幅の 2 倍超で追加の 1 点として見る。
    ///
    /// **本 test は深さ非依存性を pin しない** — 有限深さの test が示せるのは
    /// 常に「検査した深さでは有限」までである。深さ非依存性の根拠は
    /// [`sanitize_taffy_layout`] が taffy から arena への唯一の書き込み経路に
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

    /// [`sanitize_taffy_layout`] の field 単位の挙動 (上の 2 test は「有限で
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
            content_size: Size {
                width: -1e30,
                height: f32::NAN,
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
        let s = sanitize_taffy_layout(&poisoned);

        // `order` は u32 なので guard 対象外 — 素通しすること。
        assert_eq!(s.order, 7, "order は clamp 対象ではない");

        assert_eq!(s.location.x, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.location.y, -MAX_TAFFY_MAGNITUDE);
        // NaN は clamp では潰れないので `is_nan()` → 0.0 (sanitize_finite)。
        assert_eq!(s.size.width, 0.0);
        assert_eq!(s.size.height, MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.content_size.width, -MAX_TAFFY_MAGNITUDE);
        assert_eq!(s.content_size.height, 0.0);
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
            content_size: Size {
                width: 100.0,
                height: 200.0,
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
        assert_eq!(sanitize_taffy_layout(&benign), benign);
    }
}
